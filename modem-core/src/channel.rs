//! Test-only channel impairments. NOT compiled outside cfg(test) for end users,
//! but exposed unconditionally so integration tests in modem-codec can also use them.

/// Additive white Gaussian noise (Marsaglia polar method, deterministic via seed).
pub fn awgn(samples: &mut [f32], snr_db: f32, seed: u64) {
    let sig_power: f32 = samples.iter().map(|x| x * x).sum::<f32>() / samples.len() as f32;
    let snr_linear = 10f32.powf(snr_db / 10.0);
    let noise_power = sig_power / snr_linear;
    let sigma = noise_power.sqrt();
    let mut s = seed;
    for x in samples.iter_mut() {
        // xorshift64 → uniform → Box-Muller (just sin form, deterministic enough)
        s ^= s << 13; s ^= s >> 7; s ^= s << 17;
        let u1 = ((s as u32) as f32 / u32::MAX as f32).max(1e-9);
        s ^= s << 13; s ^= s >> 7; s ^= s << 17;
        let u2 = (s as u32) as f32 / u32::MAX as f32;
        let z = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f32::consts::PI * u2).cos();
        *x += z * sigma;
    }
}

/// Simulate a sample-rate skew between TX and RX of `hz` Hz at `sample_rate`.
/// This is the realistic model of clock drift between two consumer devices —
/// each symbol is stretched or compressed by the ratio `(sr + hz) / sr`.
pub fn freq_offset(samples: &[f32], hz: f32, sample_rate: u32) -> Vec<f32> {
    let ratio = (sample_rate as f32 + hz) / sample_rate as f32;
    let out_len = (samples.len() as f32 / ratio) as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src = i as f32 * ratio;
        let i0 = src.floor() as usize;
        let i1 = (i0 + 1).min(samples.len() - 1);
        let frac = src - i0 as f32;
        out.push(samples[i0] * (1.0 - frac) + samples[i1] * frac);
    }
    out
}

/// One-tap multipath: y[n] = x[n] + gain * x[n - delay_samples]
pub fn multipath(samples: &mut [f32], delay_samples: usize, gain: f32) {
    let orig: Vec<f32> = samples.to_vec();
    for n in delay_samples..samples.len() {
        samples[n] += gain * orig[n - delay_samples];
    }
}

/// Hard-clip to ±threshold.
pub fn clip(samples: &mut [f32], threshold: f32) {
    for x in samples.iter_mut() {
        *x = x.clamp(-threshold, threshold);
    }
}

/// Zero out a window of samples (simulates a dropout / cough / packet loss).
pub fn drop_window(samples: &mut [f32], start: usize, len: usize) {
    let end = (start + len).min(samples.len());
    for x in &mut samples[start..end] {
        *x = 0.0;
    }
}

/// A small phone speaker driven near its excursion limit.
///
/// Two stages, in the order a real driver applies them:
///
/// 1. **Cone energy storage.** The moving mass does not stop when the drive
///    signal changes frequency; a fraction of the previous ~milliseconds of
///    drive is still radiating. Modelled as a short echo (`ring_delay_samples`
///    / `ring_gain`), which is also a fair stand-in for the near-field
///    reflection off the phone body. The consequence that matters for FSK:
///    for the first `ring_delay_samples` of every symbol, the *previous*
///    symbol's tone is still present, so two tones coexist.
///
/// 2. **Excursion-limited compression.** `tanh` soft-clip, normalised so the
///    loudest sample sits at the limit. Memoryless and odd, so a single tone
///    at `f` only produces `3f`, `5f`… — out of our 2.0–3.4 kHz band, simply
///    lost. But wherever stage 1 left two tones overlapping, the same cubic
///    term produces third-order intermodulation at `2f1 − f2`, which for
///    200 Hz-spaced tones lands squarely *back inside* the band, on other
///    tones' Goertzel bins.
///
/// That is the failure `docs/android-smoke-test.md` describes: energy leaking
/// out of band into harmonics and back, concentrated at symbol edges — not
/// adjacent-tone confusion within the band. It is also why the three remedies
/// listed there are the right ones: pulse shaping removes the abrupt
/// transition that feeds stage 1, and a matched filter de-weights the symbol
/// edges where the intermodulation lives.
#[derive(Debug, Clone, Copy)]
pub struct SpeakerDistortion {
    /// How long the cone keeps radiating the previous drive, in samples.
    pub ring_delay_samples: usize,
    /// Amplitude of that stored energy relative to the direct path.
    pub ring_gain: f32,
    /// `tanh` drive. 0 is linear; larger is harder compression. See
    /// `total_harmonic_distortion` to convert a drive into a THD figure.
    pub drive: f32,
}

impl SpeakerDistortion {
    /// A phone micro-speaker at full media volume: 5 ms of cone storage at
    /// −9 dB, and a drive measuring ~9.5% THD on a single in-band tone
    /// (asserted in `modem-codec/tests/android_to_mac.rs`). Micro-speakers at
    /// full output are commonly quoted in the 5–15% THD range, so this sits
    /// mid-range — loud, but not a caricature.
    pub const PIXEL_AT_FULL_VOLUME: Self =
        Self { ring_delay_samples: 240, ring_gain: 0.35, drive: 1.25 };
}

/// Apply `SpeakerDistortion` to a buffer, returning the radiated signal.
pub fn speaker_distortion(samples: &[f32], p: SpeakerDistortion) -> Vec<f32> {
    let mut driven = samples.to_vec();
    if p.ring_gain != 0.0 && p.ring_delay_samples > 0 {
        multipath(&mut driven, p.ring_delay_samples, p.ring_gain);
    }
    if p.drive <= 0.0 {
        return driven;
    }
    let peak = driven.iter().fold(0f32, |a, &x| a.max(x.abs()));
    if peak < 1e-9 {
        return driven;
    }
    let denom = p.drive.tanh();
    driven
        .iter()
        .map(|&x| (p.drive * x / peak).tanh() / denom * peak)
        .collect()
}

/// One RBJ-cookbook peaking-EQ biquad. Minimum-phase, so a high-`q` section
/// carries a large group-delay swing around `f0` as well as a gain bump —
/// which is the honest way to model a resonance: it colours *and* smears.
fn peaking_biquad(samples: &[f32], f0: f32, q: f32, gain_db: f32, sample_rate: u32) -> Vec<f32> {
    let a = 10f32.powf(gain_db / 40.0);
    let w0 = 2.0 * std::f32::consts::PI * f0 / sample_rate as f32;
    let alpha = w0.sin() / (2.0 * q);
    let cw = w0.cos();
    let (b0, b1, b2) = (1.0 + alpha * a, -2.0 * cw, 1.0 - alpha * a);
    let (a0, a1, a2) = (1.0 + alpha / a, -2.0 * cw, 1.0 - alpha / a);
    let (b0, b1, b2, a1, a2) = (b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0);
    let mut out = Vec::with_capacity(samples.len());
    let (mut x1, mut x2, mut y1, mut y2) = (0f32, 0f32, 0f32, 0f32);
    for &x in samples {
        let y = b0 * x + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2;
        out.push(y);
        x2 = x1;
        x1 = x;
        y2 = y1;
        y1 = y;
    }
    out
}

/// Frequency response of a phone micro-speaker: a fundamental resonance
/// around 900 Hz plus cone-breakup modes higher up. `strength` scales every
/// mode's dB gain (1.0 = the nominal device, 0.0 = a flat ideal speaker).
///
/// This is the "response drops 6+ dB on many devices" that
/// `fsk.rs::FskConfig::audible` already warns about, made concrete.
pub fn micro_speaker_response(samples: &[f32], strength: f32, sample_rate: u32) -> Vec<f32> {
    // (centre Hz, Q, gain dB) — fundamental, two in-band breakup modes, one above.
    const MODES: [(f32, f32, f32); 4] =
        [(900.0, 3.0, 10.0), (2350.0, 9.0, 7.0), (3050.0, 12.0, -9.0), (5200.0, 6.0, 5.0)];
    let mut v = samples.to_vec();
    for (f0, q, gain_db) in MODES {
        v = peaking_biquad(&v, f0, q, gain_db * strength, sample_rate);
    }
    v
}

/// Room reverberation: twelve exponentially-decaying taps at mutually prime
/// delays from 7 ms to 127 ms, scaled so a tap at `rt60_ms` sits 60 dB down.
/// `wet` is the reverberant field's amplitude relative to the direct sound.
///
/// This is the impairment that actually matters for this modem. At 20 ms per
/// symbol, a 300 ms tail smears each symbol across the following ~15 — which
/// is inter-symbol interference far beyond what a per-symbol detector can
/// undo, no matter how the symbol is shaped or windowed.
pub fn reverb(samples: &[f32], rt60_ms: f32, wet: f32, sample_rate: u32) -> Vec<f32> {
    if rt60_ms <= 0.0 || wet <= 0.0 {
        return samples.to_vec();
    }
    const TAPS_MS: [f32; 12] =
        [7.0, 11.0, 13.0, 19.0, 23.0, 31.0, 41.0, 53.0, 67.0, 83.0, 101.0, 127.0];
    let mut out = samples.to_vec();
    for t_ms in TAPS_MS {
        let d = (t_ms / 1000.0 * sample_rate as f32) as usize;
        if d >= samples.len() {
            continue;
        }
        let g = wet * 10f32.powf(-3.0 * t_ms / rt60_ms);
        for n in d..samples.len() {
            out[n] += g * samples[n - d];
        }
    }
    out
}

/// The complete Android→Mac acoustic path: phone speaker, then room, then
/// the Mac's microphone.
#[derive(Debug, Clone, Copy)]
pub struct AndroidToMac {
    /// Scales `micro_speaker_response`.
    pub modal_strength: f32,
    /// Cone storage + excursion limiting at the driver.
    pub speaker: SpeakerDistortion,
    /// Room decay time.
    pub rt60_ms: f32,
    /// Reverberant amplitude relative to the direct path at the mic.
    pub reverb_wet: f32,
}

impl AndroidToMac {
    /// The setup `docs/android-smoke-test.md` describes: a Pixel 8 Pro at full
    /// media volume, ~30 cm from a MacBook's built-in mic, in an ordinary
    /// quiet room.
    ///
    /// `rt60_ms = 300` is a typical furnished room. `reverb_wet = 0.14` follows
    /// from the geometry: with a critical distance of roughly 1 m in such a
    /// room, a mic at 0.3 m sits well inside the direct field, so reverberant
    /// energy arrives at about `(0.3/1.0)² ≈ 0.09` of the direct energy —
    /// order 0.1–0.15 in amplitude.
    pub const PIXEL_8_PRO_AT_30CM: Self = Self {
        modal_strength: 1.0,
        speaker: SpeakerDistortion::PIXEL_AT_FULL_VOLUME,
        rt60_ms: 300.0,
        reverb_wet: 0.14,
    };
}

/// Push `samples` through the phone speaker and the room, in that order:
/// the driver colours and distorts what it radiates, and only then does the
/// room smear it.
pub fn android_to_mac(samples: &[f32], p: AndroidToMac, sample_rate: u32) -> Vec<f32> {
    let radiated = micro_speaker_response(samples, p.modal_strength, sample_rate);
    let radiated = speaker_distortion(&radiated, p.speaker);
    reverb(&radiated, p.rt60_ms, p.reverb_wet, sample_rate)
}

/// Total harmonic distortion of `samples` for a fundamental at `f0`, as a
/// fraction (0.11 = 11%): `sqrt(Σ harmonics²) / fundamental`, over harmonics
/// 2..=`n_harmonics` that fall below Nyquist.
///
/// Used to state the strength of a `SpeakerDistortion` in a unit that can be
/// compared against a real speaker's datasheet, instead of an opaque `drive`.
pub fn total_harmonic_distortion(
    samples: &[f32],
    f0: f32,
    sample_rate: u32,
    n_harmonics: usize,
) -> f32 {
    let amp = |f: f32| -> f32 {
        let omega = 2.0 * std::f32::consts::PI * f / sample_rate as f32;
        let (mut c, mut s) = (0f64, 0f64);
        for (k, &x) in samples.iter().enumerate() {
            let phi = (omega * k as f32) as f64;
            c += x as f64 * phi.cos();
            s += x as f64 * phi.sin();
        }
        ((c * c + s * s).sqrt() / samples.len() as f64) as f32
    };
    let fundamental = amp(f0).max(1e-12);
    let nyquist = sample_rate as f32 / 2.0;
    let harmonic_power: f32 = (2..=n_harmonics)
        .map(|h| f0 * h as f32)
        .filter(|f| *f < nyquist)
        .map(|f| amp(f).powi(2))
        .sum();
    harmonic_power.sqrt() / fundamental
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn awgn_reduces_snr_predictably() {
        let mut samples: Vec<f32> = (0..4800).map(|i| (i as f32 / 48.0).sin() * 0.5).collect();
        let before_rms: f32 = (samples.iter().map(|x| x * x).sum::<f32>() / samples.len() as f32).sqrt();
        awgn(&mut samples, 6.0, 42);
        let after_rms: f32 = (samples.iter().map(|x| x * x).sum::<f32>() / samples.len() as f32).sqrt();
        assert!(after_rms > before_rms, "noise should raise RMS");
    }

    fn tone(f: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|k| (2.0 * std::f32::consts::PI * f * k as f32 / 48_000.0).sin() * 0.6)
            .collect()
    }

    #[test]
    fn speaker_distortion_generates_harmonics_but_stays_in_level() {
        let clean = tone(2600.0, 9600);
        let driven = speaker_distortion(&clean, SpeakerDistortion::PIXEL_AT_FULL_VOLUME);
        assert!(
            total_harmonic_distortion(&clean, 2600.0, 48_000, 9) < 0.001,
            "a pure sine should measure ~0% THD"
        );
        assert!(
            total_harmonic_distortion(&driven, 2600.0, 48_000, 9) > 0.05,
            "the speaker model should measure meaningful THD"
        );
        let peak = driven.iter().fold(0f32, |a, &x| a.max(x.abs()));
        assert!(peak <= 1.0, "soft clipping must not run away, got peak {peak}");
    }

    #[test]
    fn micro_speaker_response_colours_the_band() {
        // Two of the audible profile's own tones, one sitting on the +7 dB
        // breakup mode at 2350 Hz and one on the −9 dB dip at 3050 Hz, must
        // come back at clearly different levels. That per-tone spread is the
        // "response drops 6+ dB on many devices" that `fsk.rs` warns about.
        let rms = |s: &[f32]| (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt();
        // Compare the settled halves, past the filters' startup transient.
        let boosted = micro_speaker_response(&tone(2400.0, 9600), 1.0, 48_000);
        let cut = micro_speaker_response(&tone(3000.0, 9600), 1.0, 48_000);
        let spread_db = 20.0 * (rms(&boosted[4800..]) / rms(&cut[4800..])).log10();
        assert!(spread_db > 6.0, "tone-to-tone spread only {spread_db:.1} dB");
    }

    #[test]
    fn reverb_extends_the_signal_and_leaves_the_onset_alone() {
        let mut burst = vec![0f32; 48_000];
        burst[..4800].copy_from_slice(&tone(2600.0, 4800));
        let wet = reverb(&burst, 300.0, 0.5, 48_000);
        // Nothing arrives before the direct sound.
        assert_eq!(wet[..7 * 48], burst[..7 * 48]);
        // Energy persists after the burst ends, where there was silence.
        let tail: f32 = wet[4800..14400].iter().map(|x| x.abs()).sum();
        assert!(tail > 1.0, "reverb should leave a tail, got {tail}");
    }

    #[test]
    fn reverb_is_a_no_op_when_dry() {
        let s = tone(2600.0, 4800);
        assert_eq!(reverb(&s, 300.0, 0.0, 48_000), s);
        assert_eq!(reverb(&s, 0.0, 0.5, 48_000), s);
    }

    #[test]
    fn drop_window_zeros_correct_range() {
        let mut s = vec![1f32; 100];
        drop_window(&mut s, 20, 10);
        assert!(s[20..30].iter().all(|&x| x == 0.0));
        assert!(s[0..20].iter().all(|&x| x == 1.0));
        assert!(s[30..].iter().all(|&x| x == 1.0));
    }
}
