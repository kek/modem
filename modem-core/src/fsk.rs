//! Multi-tone FSK modulator / demodulator.

/// Number of tones (must be a power of two).
pub const N_TONES: usize = 8;
pub const BITS_PER_SYMBOL: usize = 3;

/// Fixed sample rate everywhere.
pub const SAMPLE_RATE: u32 = 48_000;

#[derive(Debug, Clone, Copy)]
pub struct FskConfig {
    pub sample_rate: u32,
    pub symbol_samples: usize,
    /// Length (in samples) of the raised-cosine ramp at each symbol edge.
    /// 0 = legacy rectangular envelope. Set ≈10% of `symbol_samples` to
    /// soften the speaker drive at frequency transitions.
    pub ramp_samples: usize,
    /// 8 distinct frequencies, in Hz.
    pub tone_freqs: [f32; N_TONES],
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
        let mut tones = [0f32; N_TONES];
        for i in 0..N_TONES {
            tones[i] = 2000.0 + (i as f32) * 200.0; // 2000..3400
        }
        let symbol_samples = SAMPLE_RATE as usize / 50; // 20 ms
        Self {
            sample_rate: SAMPLE_RATE,
            symbol_samples,
            // 2 ms raised-cosine edges (~10% of symbol). Softens the speaker
            // drive at every frequency change, which on small phone speakers
            // suppresses the transient nonlinearity that leaks energy into
            // neighbour tones at the receiver. See docs/android-smoke-test.md.
            ramp_samples: symbol_samples / 10,
            tone_freqs: tones,
        }
    }

    /// Ultrasonic profile: 8-FSK, 50 sym/s, 250 Hz spacing, 17.5 – 19.25 kHz.
    pub fn ultrasonic() -> Self {
        let mut tones = [0f32; N_TONES];
        for i in 0..N_TONES {
            tones[i] = 17_500.0 + (i as f32) * 250.0;
        }
        let symbol_samples = SAMPLE_RATE as usize / 50; // 20 ms
        Self {
            sample_rate: SAMPLE_RATE,
            symbol_samples,
            ramp_samples: symbol_samples / 10,
            tone_freqs: tones,
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
/// Phase is continuous across symbols (CPFSK). On top of that, each symbol is
/// multiplied by a Tukey (raised-cosine-tapered) envelope so the carrier
/// amplitude eases through every frequency transition instead of switching
/// abruptly. This significantly reduces the transient broadband splatter that
/// small phone speakers produce when slammed with instantaneous frequency
/// jumps.
pub fn modulate(cfg: &FskConfig, symbols: &[u8]) -> Vec<f32> {
    let n = cfg.symbol_samples;
    let r = cfg.ramp_samples.min(n / 2);
    let mut out = Vec::with_capacity(symbols.len() * n);
    let dt = 1.0 / cfg.sample_rate as f32;
    let mut phase = 0f32;
    for &s in symbols {
        debug_assert!((s as usize) < N_TONES);
        let f = cfg.tone_freqs[s as usize];
        let dphase = 2.0 * std::f32::consts::PI * f * dt;
        for k in 0..n {
            let env = tukey_envelope(k, n, r);
            out.push(phase.sin() * 0.6 * env); // 0.6 leaves headroom
            phase += dphase;
            if phase > std::f32::consts::TAU {
                phase -= std::f32::consts::TAU;
            }
        }
    }
    out
}

/// Tukey (cosine-tapered) window sample at position `k` of an `n`-sample
/// symbol, with `r`-sample raised-cosine ramps on each edge. r == 0 is
/// rectangular (legacy behaviour).
fn tukey_envelope(k: usize, n: usize, r: usize) -> f32 {
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

/// Demodulate audio samples (assumed symbol-aligned) into a symbol stream.
/// `samples.len()` must be a multiple of `cfg.symbol_samples`.
pub fn demodulate(cfg: &FskConfig, samples: &[f32]) -> Vec<u8> {
    let n = cfg.symbol_samples;
    assert_eq!(samples.len() % n, 0, "demodulate: not symbol-aligned");
    let n_syms = samples.len() / n;
    let mut out = Vec::with_capacity(n_syms);
    for i in 0..n_syms {
        let win = &samples[i * n..(i + 1) * n];
        let mut best_idx = 0usize;
        let mut best_mag = f32::NEG_INFINITY;
        for (t, &f) in cfg.tone_freqs.iter().enumerate() {
            let m = goertzel_mag2(win, f, cfg.sample_rate);
            if m > best_mag {
                best_mag = m;
                best_idx = t;
            }
        }
        out.push(best_idx as u8);
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
}
