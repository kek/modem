//! Frame preamble: linear up-chirp + sync word.

use crate::fsk::SAMPLE_RATE;

/// Sync word that follows the chirp (one symbol's worth of recognizable bytes).
pub const SYNC_WORD: [u8; 2] = [0x7E, 0x81];

pub fn preamble_samples() -> Vec<f32> {
    // 80 ms chirp from 1.5 kHz to 6 kHz
    chirp(1_500.0, 6_000.0, 0.080)
}

fn chirp(f0: f32, f1: f32, duration_s: f32) -> Vec<f32> {
    let n = (SAMPLE_RATE as f32 * duration_s) as usize;
    let mut out = Vec::with_capacity(n);
    let dt = 1.0 / SAMPLE_RATE as f32;
    let k = (f1 - f0) / duration_s; // Hz per second
    let mut t = 0f32;
    let mut phase = 0f32;
    for _ in 0..n {
        let f = f0 + k * t;
        phase += 2.0 * std::f32::consts::PI * f * dt;
        out.push(phase.sin() * 0.6);
        t += dt;
    }
    out
}

/// Slide the template across `haystack` and return (best_offset, peak_score).
/// score is normalised energy-of-correlation.
pub fn detect_preamble(haystack: &[f32], template: &[f32]) -> Option<(usize, f32)> {
    if haystack.len() < template.len() {
        return None;
    }
    let tn = template.len();
    let template_energy: f32 = template.iter().map(|x| x * x).sum::<f32>().sqrt();
    let mut best: Option<(usize, f32)> = None;
    // Stride 4 samples for speed (~12 kHz effective scan rate); fine enough for ~80ms preamble.
    for i in (0..=haystack.len() - tn).step_by(4) {
        let mut corr = 0f32;
        let mut win_energy = 0f32;
        for k in 0..tn {
            let x = haystack[i + k];
            corr += x * template[k];
            win_energy += x * x;
        }
        let denom = (win_energy.sqrt() * template_energy).max(1e-9);
        let score = corr / denom;
        if best.map_or(true, |(_, b)| score > b) {
            best = Some((i, score));
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chirp_has_expected_length() {
        let p = preamble_samples();
        assert_eq!(p.len(), (SAMPLE_RATE as f32 * 0.080) as usize);
    }

    #[test]
    fn detects_preamble_at_known_offset() {
        let template = preamble_samples();
        let mut haystack = vec![0f32; 2000];
        haystack.extend(template.iter().copied());
        haystack.extend(std::iter::repeat(0f32).take(2000));
        let (off, score) = detect_preamble(&haystack, &template).unwrap();
        // Stride is 4, so allow ±3-sample tolerance.
        assert!((off as i64 - 2000).abs() <= 4, "off={off}");
        assert!(score > 0.9, "score={score}");
    }

    #[test]
    fn rejects_pure_noise() {
        let template = preamble_samples();
        // Deterministic "noise"
        let haystack: Vec<f32> = (0..10000).map(|i| (((i * 7919) % 17) as f32 / 17.0 - 0.5) * 0.2).collect();
        let (_, score) = detect_preamble(&haystack, &template).unwrap();
        assert!(score < 0.3, "noise should not produce high correlation, got {score}");
    }
}
