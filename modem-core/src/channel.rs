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

/// Apply a constant frequency offset by multiplying with a complex exponential
/// (here we cheat: small offsets just shift each sample's phase linearly).
pub fn freq_offset(samples: &mut [f32], hz: f32, sample_rate: u32) {
    let dphi = 2.0 * std::f32::consts::PI * hz / sample_rate as f32;
    let mut phi = 0f32;
    for x in samples.iter_mut() {
        *x *= phi.cos();
        phi += dphi;
    }
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

    #[test]
    fn drop_window_zeros_correct_range() {
        let mut s = vec![1f32; 100];
        drop_window(&mut s, 20, 10);
        assert!(s[20..30].iter().all(|&x| x == 0.0));
        assert!(s[0..20].iter().all(|&x| x == 1.0));
        assert!(s[30..].iter().all(|&x| x == 1.0));
    }
}
