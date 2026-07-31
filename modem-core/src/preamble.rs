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

/// Coarse scan stride, in samples. A stride of 4 costs a quarter of the work of
/// a full scan, but it cannot *see* the peak of a chirp correlation: the main
/// lobe of a 1.5-6 kHz chirp's autocorrelation is only ~10 samples wide and its
/// fine structure oscillates at the ~3.75 kHz centre frequency, so the best
/// stride-4 grid point can sit up to `SCAN_STRIDE - 1` samples off true
/// alignment and score well below the real peak. Hence `refine_peak`.
///
/// Public because the grid has a *phase*, and a caller that slides a window
/// across a stream has to keep it: `detect_preamble` strides from index 0 of
/// whatever slice it is handed, so a caller that advances by something other
/// than a multiple of `SCAN_STRIDE` re-phases the grid and scores a different
/// set of offsets. Measured on the six captures in `captures/`: advancing by
/// `bound + 1` instead of a multiple of the stride moved the lock on three of
/// the six (capture 001: 116460 at 0.2947 becomes 116385 at 0.2697 — a peak
/// traded for a shoulder, and a third of the margin over the 0.25 accept
/// threshold given away for nothing). `Receiver::step` advances in whole
/// strides for that reason.
pub const SCAN_STRIDE: usize = 4;

/// Normalised correlation of `template` against `haystack[i..i + template.len()]`.
/// Amplitude-invariant: a quiet but well-shaped copy scores as high as a loud one.
///
/// The loop zips two slices rather than indexing `haystack[i + k]`, which is
/// the same arithmetic in the same order — so every score is bit-identical to
/// the indexed version — with one bounds check instead of `template.len()` of
/// them. That matters because the tests run this in a debug build, where the
/// per-element check is a large fraction of the cost of the whole scan.
fn score_at(haystack: &[f32], template: &[f32], i: usize, template_energy: f32) -> f32 {
    let mut corr = 0f32;
    let mut win_energy = 0f32;
    for (x, t) in haystack[i..i + template.len()].iter().zip(template.iter()) {
        corr += x * t;
        win_energy += x * x;
    }
    let denom = (win_energy.sqrt() * template_energy).max(1e-9);
    corr / denom
}

/// Slide the template across `haystack` and return (best_offset, peak_score).
/// score is normalised energy-of-correlation.
///
/// This searches for the *maximum* over the whole of `haystack` and only then
/// returns; it never commits to the first window that happens to look good.
/// `Receiver::step` applies its accept threshold to what comes back, so the
/// threshold is tested against the peak, not against the leading edge of the
/// correlation ramp. `locks_on_the_peak_not_the_first_crossing` pins that.
pub fn detect_preamble(haystack: &[f32], template: &[f32]) -> Option<(usize, f32)> {
    if haystack.len() < template.len() {
        return None;
    }
    let tn = template.len();
    let template_energy: f32 = template.iter().map(|x| x * x).sum::<f32>().sqrt();
    let last = haystack.len() - tn;
    let mut best: Option<(usize, f32)> = None;
    for i in (0..=last).step_by(SCAN_STRIDE) {
        let score = score_at(haystack, template, i, template_energy);
        if best.map_or(true, |(_, b)| score > b) {
            best = Some((i, score));
        }
    }
    let (coarse_off, coarse_score) = best?;
    Some(refine_peak(
        haystack,
        template,
        template_energy,
        coarse_off,
        coarse_score,
        last,
    ))
}

/// Re-search the `SCAN_STRIDE - 1` samples either side of the coarse winner at
/// full resolution — exactly, and only, the offsets the coarse grid skipped.
///
/// The window is deliberately tiny and symmetric. Peak-picking needs a bound:
/// an unbounded "keep looking for something better" search would trade an early
/// lock for a late one, and a late lock is the worse failure, because every
/// symbol boundary downstream is measured from the returned offset. ±3 samples
/// is 0.3% of a 960-sample symbol either way, so this can only sharpen the
/// alignment the coarse scan already chose; it cannot move the lock to a
/// different candidate.
fn refine_peak(
    haystack: &[f32],
    template: &[f32],
    template_energy: f32,
    coarse_off: usize,
    coarse_score: f32,
    last: usize,
) -> (usize, f32) {
    let lo = coarse_off.saturating_sub(SCAN_STRIDE - 1);
    let hi = (coarse_off + SCAN_STRIDE - 1).min(last);
    let mut best = (coarse_off, coarse_score);
    for i in lo..=hi {
        if i == coarse_off {
            continue;
        }
        let score = score_at(haystack, template, i, template_energy);
        if score > best.1 {
            best = (i, score);
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

    /// Deterministic pseudo-noise in [-amp, amp]. No RNG: a sync test that
    /// only fails on some seeds is worse than no test.
    fn pseudo_noise(n: usize, amp: f32) -> Vec<f32> {
        (0..n)
            .map(|i| ((((i * 7919) % 17) as f32 / 17.0) - 0.5) * 2.0 * amp)
            .collect()
    }

    /// The behaviour this whole module exists to guarantee: when the
    /// correlation rises above the accept threshold and then peaks *higher*
    /// later, the lock lands on the peak.
    ///
    /// Synthetic on purpose. The six real captures in `captures/` are evidence
    /// about the channel, not a fixture — this must fail on a first-crossing
    /// detector on a machine that has never seen them.
    ///
    /// Two candidates, both over the 0.25 threshold `Receiver::step` uses:
    /// a smeared one first (the chirp buried in noise, as a distant or
    /// reflected copy arrives), a clean one later. A detector that returns the
    /// first window over threshold returns the smeared one.
    #[test]
    fn locks_on_the_peak_not_the_first_crossing() {
        let template = preamble_samples();
        let tn = template.len();
        let early = 2_000usize;
        // Deliberately off the coarse grid, so the peak is only reachable by
        // the fine refine as well.
        let late = 9_001usize;

        let mut haystack = pseudo_noise(late + tn + 2_000, 0.30);
        // Smeared early copy: correct shape, drowned in noise. Note the score
        // is amplitude-invariant, so it is the noise, not the level, that
        // holds this candidate down.
        for k in 0..tn {
            haystack[early + k] += template[k] * 0.55;
        }
        // Clean late copy, on a quiet background.
        for k in 0..tn {
            haystack[late + k] = haystack[late + k] * 0.02 + template[k];
        }

        // What a first-crossing detector would have committed to: restrict the
        // haystack to the early candidate and read off its score.
        let (early_off, early_score) =
            detect_preamble(&haystack[..late], &template).expect("early candidate detectable");
        assert!(
            (early_off as i64 - early as i64).abs() <= 4,
            "early candidate should be found near {early}, got {early_off}"
        );
        assert!(
            early_score > 0.25,
            "the early candidate must clear Receiver::step's 0.25 threshold for \
             this test to mean anything, got {early_score}"
        );

        let (off, score) = detect_preamble(&haystack, &template).expect("peak detectable");
        assert_eq!(
            off, late,
            "locked on the first window over threshold ({early_score:.3} at {early_off}) \
             instead of searching on to the peak ({score:.3} at {off}); expected {late}"
        );
        assert!(
            score > early_score,
            "peak score {score} should beat the first crossing {early_score}"
        );
    }

    /// The coarse scan strides over samples, so on its own it cannot land on
    /// the peak of a correlation whose main lobe is ~10 samples wide. Pin the
    /// bounded refine that fixes that, and pin that it is an improvement over
    /// the grid point it started from.
    #[test]
    fn refines_the_peak_past_the_coarse_grid() {
        let template = preamble_samples();
        let tn = template.len();
        let true_off = 4_003usize; // 3 samples off the stride-4 grid
        let mut haystack = pseudo_noise(true_off + tn + 4_000, 0.01);
        for k in 0..tn {
            haystack[true_off + k] += template[k];
        }

        let (off, score) = detect_preamble(&haystack, &template).expect("detectable");
        assert_eq!(off, true_off, "refine should reach the exact peak");

        let te: f32 = template.iter().map(|x| x * x).sum::<f32>().sqrt();
        for grid in [true_off - 3, true_off + 1] {
            let grid_score = score_at(&haystack, &template, grid, te);
            assert!(
                score > grid_score,
                "peak at {true_off} ({score}) should beat coarse grid point {grid} ({grid_score})"
            );
        }
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
