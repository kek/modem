//! PHY trait + multi-tone FSK implementation.

use crate::fsk::{demodulate, modulate, bytes_to_symbols, symbols_to_bytes, FskConfig, BITS_PER_SYMBOL};
use crate::preamble::{preamble_samples, detect_preamble};

pub trait Phy {
    fn sample_rate(&self) -> u32;
    fn preamble(&self) -> Vec<f32>;
    fn detect_preamble(&self, samples: &[f32]) -> Option<(usize, f32)>;
    /// Modulate a frame's raw bytes (`FRAME_LEN` bytes) into samples.
    fn modulate_bytes(&self, bytes: &[u8]) -> Vec<f32>;
    /// Demodulate `n_bytes` worth of samples (symbol-aligned).
    fn demodulate_bytes(&self, samples: &[f32], n_bytes: usize) -> Vec<u8>;
    /// Samples per frame's data section (for the receiver's window math).
    fn frame_data_samples(&self, n_bytes: usize) -> usize;
}

pub struct FskPhy {
    cfg: FskConfig,
    preamble: Vec<f32>,
}

impl FskPhy {
    pub fn audible() -> Self {
        Self { cfg: FskConfig::audible(), preamble: preamble_samples() }
    }
    pub fn ultrasonic() -> Self {
        Self { cfg: FskConfig::ultrasonic(), preamble: preamble_samples() }
    }
    pub fn config(&self) -> &FskConfig { &self.cfg }
}

impl Phy for FskPhy {
    fn sample_rate(&self) -> u32 { self.cfg.sample_rate }
    fn preamble(&self) -> Vec<f32> { self.preamble.clone() }
    fn detect_preamble(&self, samples: &[f32]) -> Option<(usize, f32)> {
        detect_preamble(samples, &self.preamble)
    }
    fn modulate_bytes(&self, bytes: &[u8]) -> Vec<f32> {
        let syms = bytes_to_symbols(bytes);
        modulate(&self.cfg, &syms)
    }
    fn demodulate_bytes(&self, samples: &[f32], n_bytes: usize) -> Vec<u8> {
        let n_syms = (n_bytes * 8 + BITS_PER_SYMBOL - 1) / BITS_PER_SYMBOL;
        let needed = n_syms * self.cfg.symbol_samples;
        let win = &samples[..needed.min(samples.len())];
        let syms = demodulate(&self.cfg, win);
        symbols_to_bytes(&syms, n_bytes)
    }
    fn frame_data_samples(&self, n_bytes: usize) -> usize {
        let n_syms = (n_bytes * 8 + BITS_PER_SYMBOL - 1) / BITS_PER_SYMBOL;
        n_syms * self.cfg.symbol_samples
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_via_trait_object() {
        let phy: Box<dyn Phy> = Box::new(FskPhy::audible());
        let bytes = vec![0x11, 0x22, 0x33, 0x44, 0x55];
        let samples = phy.modulate_bytes(&bytes);
        let back = phy.demodulate_bytes(&samples, bytes.len());
        assert_eq!(back, bytes);
    }
}
