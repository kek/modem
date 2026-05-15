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
