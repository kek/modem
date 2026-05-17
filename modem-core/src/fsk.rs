//! Multi-tone FSK modulator / demodulator.

/// Number of tones (must be a power of two).
pub const N_TONES: usize = 8;
pub const BITS_PER_SYMBOL: usize = 3;

/// Fixed sample rate everywhere.
pub const SAMPLE_RATE: u32 = 48_000;

/// Runtime-selectable DSP variants. All default to OFF, preserving legacy
/// trunk behaviour. Each flag is documented in `docs/android-smoke-test.md`
/// under "Suggested follow-up DSP work".
///
/// The fields are independent; any subset can be enabled together. The
/// `modem rank` CLI subcommand sweeps all 8 combinations across a capture
/// corpus to identify which combination wins over a real channel.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DspVariants {
    /// TX-side raised-cosine envelope on each symbol (`ramp_samples`
    /// computed as 10% of `symbol_samples` when enabled). Softens the
    /// speaker drive at every frequency transition.
    pub pulse_shape: bool,
    /// RX-side Tukey-windowed matched filter (α=0.5) instead of
    /// rectangular Goertzel. De-emphasizes corrupted symbol edges.
    pub matched_filter: bool,
    /// RX-side early-late timing tracker. Continuously nudges the symbol
    /// window offset to compensate for clock drift between TX and RX.
    pub timing_recovery: bool,
}

impl DspVariants {
    pub const BASELINE: Self = Self { pulse_shape: false, matched_filter: false, timing_recovery: false };
    pub const ALL: Self = Self { pulse_shape: true, matched_filter: true, timing_recovery: true };

    /// Short tag like "" for baseline, "p+m" for pulse-shape + matched-filter, etc.
    /// Used by the rank harness to label result lines.
    pub fn tag(&self) -> String {
        let mut parts: Vec<&str> = Vec::new();
        if self.pulse_shape { parts.push("p"); }
        if self.matched_filter { parts.push("m"); }
        if self.timing_recovery { parts.push("t"); }
        if parts.is_empty() { "baseline".into() } else { parts.join("+") }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FskConfig {
    pub sample_rate: u32,
    pub symbol_samples: usize,
    /// 8 distinct frequencies, in Hz.
    pub tone_freqs: [f32; N_TONES],
    /// DSP variant toggles. Defaults to all-off (legacy behaviour).
    pub variants: DspVariants,
}

impl FskConfig {
    /// Audible profile: 8-FSK, 50 sym/s, 200 Hz spacing, 2.0 – 3.4 kHz.
    ///
    /// Tones live in the flattest region of typical built-in laptop / phone
    /// speaker+mic response. Outside this band, response drops 6+ dB on many
    /// devices, causing Goertzel to confuse tones.
    ///
    /// 20 ms symbols (960 samples @ 48 kHz) give Goertzel ≈50 Hz bin
    /// resolution, so adjacent tones at 200 Hz spacing are 4 bins apart —
    /// strongly selective even under speaker/mic FR variation. 10 ms symbols
    /// (the previous setting) gave only 2 bins of separation, which led to
    /// bit errors in real over-the-air decode (Mac↔Android).
    ///
    /// Throughput: ~150 bps raw.
    pub fn audible() -> Self {
        Self::audible_with(DspVariants::default())
    }

    pub fn audible_with(variants: DspVariants) -> Self {
        let mut tones = [0f32; N_TONES];
        for i in 0..N_TONES {
            tones[i] = 2000.0 + (i as f32) * 200.0; // 2000..3400
        }
        Self {
            sample_rate: SAMPLE_RATE,
            symbol_samples: SAMPLE_RATE as usize / 50, // 20 ms
            tone_freqs: tones,
            variants,
        }
    }

    /// Ultrasonic profile: 8-FSK, 50 sym/s, 250 Hz spacing, 17.5 – 19.25 kHz.
    pub fn ultrasonic() -> Self {
        Self::ultrasonic_with(DspVariants::default())
    }

    pub fn ultrasonic_with(variants: DspVariants) -> Self {
        let mut tones = [0f32; N_TONES];
        for i in 0..N_TONES {
            tones[i] = 17_500.0 + (i as f32) * 250.0;
        }
        Self {
            sample_rate: SAMPLE_RATE,
            symbol_samples: SAMPLE_RATE as usize / 50, // 20 ms
            tone_freqs: tones,
            variants,
        }
    }
}

/// Convert bytes to a sequence of 3-bit symbols (MSB first within each byte).
pub fn bytes_to_symbols(bytes: &[u8]) -> Vec<u8> {
    let total_bits = bytes.len() * 8;
    // Pad to a multiple of BITS_PER_SYMBOL with zero bits.
    let n_syms = (total_bits + BITS_PER_SYMBOL - 1) / BITS_PER_SYMBOL;
    let mut syms = Vec::with_capacity(n_syms);
    let mut bit_idx = 0;
    for _ in 0..n_syms {
        let mut sym: u8 = 0;
        for _ in 0..BITS_PER_SYMBOL {
            let byte_i = bit_idx / 8;
            let bit_i = 7 - (bit_idx % 8); // MSB first
            let bit = if byte_i < bytes.len() {
                (bytes[byte_i] >> bit_i) & 1
            } else {
                0 // pad
            };
            sym = (sym << 1) | bit;
            bit_idx += 1;
        }
        syms.push(sym);
    }
    syms
}

/// Inverse of bytes_to_symbols. `n_bytes` is the expected output length;
/// extra symbols (padding) are dropped.
pub fn symbols_to_bytes(syms: &[u8], n_bytes: usize) -> Vec<u8> {
    let mut out = vec![0u8; n_bytes];
    let mut bit_idx = 0;
    for &s in syms {
        for k in 0..BITS_PER_SYMBOL {
            let bit = (s >> (BITS_PER_SYMBOL - 1 - k)) & 1;
            let byte_i = bit_idx / 8;
            let bit_i = 7 - (bit_idx % 8);
            if byte_i < n_bytes {
                out[byte_i] |= bit << bit_i;
            }
            bit_idx += 1;
        }
    }
    out
}

/// Modulate a symbol stream into audio samples.
///
/// Phase is always continuous across symbols (CPFSK). If
/// `cfg.variants.pulse_shape` is set, each symbol is additionally weighted
/// by a Tukey envelope with raised-cosine ramps over 10% of the symbol on
/// each edge — so the carrier amplitude eases through frequency
/// transitions instead of switching abruptly.
pub fn modulate(cfg: &FskConfig, symbols: &[u8]) -> Vec<f32> {
    let n = cfg.symbol_samples;
    let r = if cfg.variants.pulse_shape { n / 10 } else { 0 };
    let mut out = Vec::with_capacity(symbols.len() * n);
    let dt = 1.0 / cfg.sample_rate as f32;
    let mut phase = 0f32;
    for &s in symbols {
        debug_assert!((s as usize) < N_TONES);
        let f = cfg.tone_freqs[s as usize];
        let dphase = 2.0 * std::f32::consts::PI * f * dt;
        for k in 0..n {
            let env = tukey_edge_envelope(k, n, r);
            out.push(phase.sin() * 0.6 * env);
            phase += dphase;
            if phase > std::f32::consts::TAU {
                phase -= std::f32::consts::TAU;
            }
        }
    }
    out
}

/// Envelope at sample `k` of an `n`-sample symbol, with raised-cosine
/// ramps of length `r` at each edge and flat 1.0 in the middle.
/// `r == 0` is rectangular.
fn tukey_edge_envelope(k: usize, n: usize, r: usize) -> f32 {
    if r == 0 {
        return 1.0;
    }
    if k < r {
        0.5 * (1.0 - (std::f32::consts::PI * k as f32 / r as f32).cos())
    } else if k >= n - r {
        let m = n - 1 - k;
        0.5 * (1.0 - (std::f32::consts::PI * m as f32 / r as f32).cos())
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audible_geometry() {
        let c = FskConfig::audible();
        assert_eq!(c.symbol_samples, 960);
        assert_eq!(c.tone_freqs[0], 2000.0);
        assert_eq!(c.tone_freqs[7], 3400.0);
    }

    #[test]
    fn ultrasonic_geometry() {
        let c = FskConfig::ultrasonic();
        assert_eq!(c.symbol_samples, 960);
        assert_eq!(c.tone_freqs[0], 17_500.0);
        assert_eq!(c.tone_freqs[7], 19_250.0);
    }

    #[test]
    fn nyquist_safe() {
        let nyq = SAMPLE_RATE as f32 / 2.0;
        for c in [FskConfig::audible(), FskConfig::ultrasonic()] {
            for &f in &c.tone_freqs {
                assert!(f < nyq, "tone {f} exceeds Nyquist {nyq}");
            }
        }
    }
}

#[cfg(test)]
mod modulator_tests {
    use super::*;

    #[test]
    fn symbol_roundtrip_byte_aligned() {
        // 3 bytes = 24 bits = 8 symbols exactly.
        let bytes = vec![0xDE, 0xAD, 0xBE];
        let syms = bytes_to_symbols(&bytes);
        assert_eq!(syms.len(), 8);
        let back = symbols_to_bytes(&syms, bytes.len());
        assert_eq!(back, bytes);
    }

    #[test]
    fn symbol_roundtrip_unaligned() {
        let bytes = vec![0x01, 0x02]; // 16 bits → 6 symbols (18 bits, padded)
        let syms = bytes_to_symbols(&bytes);
        assert_eq!(syms.len(), 6);
        let back = symbols_to_bytes(&syms, bytes.len());
        assert_eq!(back, bytes);
    }

    #[test]
    fn modulator_output_length() {
        let cfg = FskConfig::audible();
        let syms = vec![0u8; 10];
        let samples = modulate(&cfg, &syms);
        assert_eq!(samples.len(), 10 * cfg.symbol_samples);
    }

    #[test]
    fn modulator_amplitude_bounded() {
        let cfg = FskConfig::audible();
        let syms = vec![3, 5, 7, 1];
        let samples = modulate(&cfg, &syms);
        for s in samples {
            assert!(s.abs() <= 0.61, "sample out of headroom: {s}");
        }
    }

    #[test]
    fn goertzel_bank_picks_the_right_tone() {
        // Pure 2200 Hz sine, 20 ms at 48 kHz = 960 samples.
        let cfg = FskConfig::audible();
        let n = cfg.symbol_samples;
        let f0 = cfg.tone_freqs[1]; // 2200 Hz
        let mut samples = vec![0f32; n];
        for k in 0..n {
            let t = k as f32 / cfg.sample_rate as f32;
            samples[k] = (2.0 * std::f32::consts::PI * f0 * t).sin();
        }
        let mags = goertzel_bank(&samples, &cfg.tone_freqs, cfg.sample_rate);
        assert_eq!(mags.len(), N_TONES);
        // The 2200 Hz bin must dominate by >10× over every other bin.
        let target = mags[1];
        for (i, &m) in mags.iter().enumerate() {
            if i == 1 { continue; }
            assert!(target > 10.0 * m, "tone {} not dominant: {:?}", i, mags);
        }
    }
}

/// Goertzel single-bin magnitude squared for frequency `f` in `samples`.
fn goertzel_mag2(samples: &[f32], f: f32, sample_rate: u32) -> f32 {
    let n = samples.len() as f32;
    let k = (0.5 + n * f / sample_rate as f32).floor();
    let w = 2.0 * std::f32::consts::PI * k / n;
    let coeff = 2.0 * w.cos();
    let (mut s0, mut s1, mut s2) = (0f32, 0f32, 0f32);
    for &x in samples {
        s0 = x + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    s1 * s1 + s2 * s2 - coeff * s1 * s2
}

/// Compute Goertzel magnitude² for each frequency in `freqs` against the
/// given buffer of f32 samples. Returns one magnitude per frequency, in the
/// same order. Used by visualization layers that want per-tone energy
/// without running the full demod state machine.
pub fn goertzel_bank(samples: &[f32], freqs: &[f32], sample_rate: u32) -> Vec<f32> {
    freqs.iter().map(|&f| goertzel_mag2(samples, f, sample_rate)).collect()
}

/// Full-length Tukey window of length `n` with `alpha/2` raised-cosine
/// fraction on each side (`alpha = 0.5` → 25% taper per edge, 50% flat).
fn tukey_window(n: usize, alpha: f32) -> Vec<f32> {
    let r = ((alpha * n as f32 / 2.0) as usize).min(n / 2);
    let mut w = vec![1f32; n];
    if r == 0 {
        return w;
    }
    for k in 0..r {
        let v = 0.5 * (1.0 - (std::f32::consts::PI * k as f32 / r as f32).cos());
        w[k] = v;
        w[n - 1 - k] = v;
    }
    w
}

/// Magnitude² of inner product of `samples` with a windowed complex
/// exponential at `f`. Equivalent to Goertzel when `window` is all-1.0.
fn matched_filter_mag2(samples: &[f32], window: &[f32], f: f32, sample_rate: u32) -> f32 {
    debug_assert_eq!(samples.len(), window.len());
    let dt = 1.0 / sample_rate as f32;
    let omega = 2.0 * std::f32::consts::PI * f * dt;
    let mut c = 0f32;
    let mut s = 0f32;
    for (k, (&x, &w)) in samples.iter().zip(window.iter()).enumerate() {
        let phi = omega * k as f32;
        let xw = x * w;
        c += xw * phi.cos();
        s += xw * phi.sin();
    }
    c * c + s * s
}

/// Tone-energy detector at a given offset. Dispatches to Goertzel
/// (rectangular) or matched filter (Tukey) based on the supplied window.
fn tone_energy(
    samples: &[f32],
    start: usize,
    n: usize,
    window: &Option<Vec<f32>>,
    f: f32,
    sample_rate: u32,
) -> f32 {
    let win = &samples[start..start + n];
    match window {
        Some(w) => matched_filter_mag2(win, w, f, sample_rate),
        None => goertzel_mag2(win, f, sample_rate),
    }
}

/// Demodulate audio samples (assumed symbol-aligned at sample 0) into a
/// symbol stream. `samples.len()` must be a multiple of `cfg.symbol_samples`.
///
/// Behaviour depends on `cfg.variants`:
///   - `matched_filter`: per-tone detector uses a Tukey (α=0.5) window
///     instead of a rectangular Goertzel.
///   - `timing_recovery`: an early-late gate adjusts the symbol-window
///     offset by ±1 sample whenever the chosen tone has asymmetric energy
///     around the current centre, clamped to ±N/8.
pub fn demodulate(cfg: &FskConfig, samples: &[f32]) -> Vec<u8> {
    let n = cfg.symbol_samples;
    assert_eq!(samples.len() % n, 0, "demodulate: not symbol-aligned");
    let n_syms = samples.len() / n;
    let buf_len = samples.len() as i32;
    let n_i = n as i32;
    let step = (n_i / 16).max(2);
    let max_offset = n_i / 8;

    let window = if cfg.variants.matched_filter {
        Some(tukey_window(n, 0.5))
    } else {
        None
    };

    let mut offset: i32 = 0;
    let mut out = Vec::with_capacity(n_syms);

    for i in 0..n_syms {
        let nominal = (i as i32) * n_i;
        let center = if cfg.variants.timing_recovery {
            (nominal + offset).clamp(0, buf_len - n_i) as usize
        } else {
            nominal as usize
        };

        let mut best_idx = 0usize;
        let mut best_mag = f32::NEG_INFINITY;
        for (t, &f) in cfg.tone_freqs.iter().enumerate() {
            let m = tone_energy(samples, center, n, &window, f, cfg.sample_rate);
            if m > best_mag {
                best_mag = m;
                best_idx = t;
            }
        }
        out.push(best_idx as u8);

        if cfg.variants.timing_recovery {
            let early_start = nominal + offset - step;
            let late_start = nominal + offset + step;
            if early_start >= 0 && late_start + n_i <= buf_len {
                let f_best = cfg.tone_freqs[best_idx];
                let early = tone_energy(
                    samples, early_start as usize, n, &window, f_best, cfg.sample_rate,
                );
                let late = tone_energy(
                    samples, late_start as usize, n, &window, f_best, cfg.sample_rate,
                );
                let threshold = 0.1 * best_mag;
                if late > early + threshold && offset < max_offset {
                    offset += 1;
                } else if early > late + threshold && offset > -max_offset {
                    offset -= 1;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod demod_tests {
    use super::*;

    #[test]
    fn clean_roundtrip_audible() {
        let cfg = FskConfig::audible();
        let bytes = b"acoustic modem".to_vec();
        let syms = bytes_to_symbols(&bytes);
        let samples = modulate(&cfg, &syms);
        let back_syms = demodulate(&cfg, &samples);
        assert_eq!(back_syms, syms);
        let back = symbols_to_bytes(&back_syms, bytes.len());
        assert_eq!(back, bytes);
    }

    #[test]
    fn clean_roundtrip_ultrasonic() {
        let cfg = FskConfig::ultrasonic();
        let bytes = b"hi".to_vec();
        let syms = bytes_to_symbols(&bytes);
        let samples = modulate(&cfg, &syms);
        let back = demodulate(&cfg, &samples);
        assert_eq!(back, syms);
    }

    #[test]
    fn resilient_to_small_white_noise() {
        let cfg = FskConfig::audible();
        let bytes = b"noise test 12345".to_vec();
        let syms = bytes_to_symbols(&bytes);
        let mut samples = modulate(&cfg, &syms);
        // Add deterministic "noise" at ~-20 dB peak
        for (i, s) in samples.iter_mut().enumerate() {
            let n = ((i * 7919) % 7) as f32 / 7.0 - 0.5;
            *s += n * 0.06;
        }
        let back = demodulate(&cfg, &samples);
        assert_eq!(back, syms, "demodulator failed under mild noise");
    }

    /// Clean-channel roundtrip must work with every variant combination.
    /// This is the regression net: any future demodulator change that
    /// breaks one of the 8 cells here will fail this test.
    #[test]
    fn all_variant_combinations_roundtrip_clean() {
        let bytes = b"variant matrix".to_vec();
        for ps in [false, true] {
            for mf in [false, true] {
                for tr in [false, true] {
                    let variants = DspVariants { pulse_shape: ps, matched_filter: mf, timing_recovery: tr };
                    let cfg = FskConfig::audible_with(variants);
                    let syms = bytes_to_symbols(&bytes);
                    let samples = modulate(&cfg, &syms);
                    let back_syms = demodulate(&cfg, &samples);
                    assert_eq!(back_syms, syms, "roundtrip failed for {}", variants.tag());
                }
            }
        }
    }
}
