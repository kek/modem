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
    /// 8 distinct frequencies, in Hz.
    pub tone_freqs: [f32; N_TONES],
}

impl FskConfig {
    /// Audible profile: 8-FSK, 100 sym/s, 500 Hz spacing, 2.0 – 5.5 kHz.
    pub fn audible() -> Self {
        let mut tones = [0f32; N_TONES];
        for i in 0..N_TONES {
            tones[i] = 2000.0 + (i as f32) * 500.0; // 2000..5500
        }
        Self {
            sample_rate: SAMPLE_RATE,
            symbol_samples: SAMPLE_RATE as usize / 100, // 10 ms
            tone_freqs: tones,
        }
    }

    /// Ultrasonic profile: 8-FSK, 50 sym/s, 250 Hz spacing, 17.5 – 19.25 kHz.
    pub fn ultrasonic() -> Self {
        let mut tones = [0f32; N_TONES];
        for i in 0..N_TONES {
            tones[i] = 17_500.0 + (i as f32) * 250.0;
        }
        Self {
            sample_rate: SAMPLE_RATE,
            symbol_samples: SAMPLE_RATE as usize / 50, // 20 ms
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
pub fn modulate(cfg: &FskConfig, symbols: &[u8]) -> Vec<f32> {
    let mut out = Vec::with_capacity(symbols.len() * cfg.symbol_samples);
    let dt = 1.0 / cfg.sample_rate as f32;
    // Continuous phase across symbols to avoid clicks.
    let mut phase = 0f32;
    for &s in symbols {
        debug_assert!((s as usize) < N_TONES);
        let f = cfg.tone_freqs[s as usize];
        let dphase = 2.0 * std::f32::consts::PI * f * dt;
        for _ in 0..cfg.symbol_samples {
            out.push(phase.sin() * 0.6); // leave headroom
            phase += dphase;
            if phase > std::f32::consts::TAU {
                phase -= std::f32::consts::TAU;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audible_geometry() {
        let c = FskConfig::audible();
        assert_eq!(c.symbol_samples, 480);
        assert_eq!(c.tone_freqs[0], 2000.0);
        assert_eq!(c.tone_freqs[7], 5500.0);
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
