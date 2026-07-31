//! Measure, on real captures, the two numbers the look-back bound rests on:
//! where the receiver actually locks, and how long a chirp keeps correlating
//! after it has arrived.
//!
//! ```bash
//! cargo run --release -p modem-codec --example preamble_probe -- captures
//! ```
//!
//! Emits, per capture:
//!
//! * `LOCK` — the offset and score `Receiver` locked on, in *recording*
//!   coordinates (sample index from the start of the WAV). This is the gate the
//!   preamble work in `docs/android-smoke-test.md` is stated against: the rank
//!   table is not, because rank survived a bug that moved three of six locks
//!   onto correlation shoulders.
//! * `PEAK` — the unbounded stride-4 scan's global winner, for comparison.
//! * `AHEAD` — the strongest correlation anywhere before the lock's cluster.
//!   The bound can only move a lock if something here clears 0.25.
//! * `CLUSTER` — the full-resolution extent of the above-threshold offsets
//!   around the peak: how far ahead of the peak this chirp first becomes
//!   acceptable, and how far past it it stays acceptable.
//! * `TAIL` — the reverb tail: the strongest correlation at each delay band
//!   after the peak, i.e. how late a reflected copy of the chirp still
//!   correlates in the room this was recorded in. This is what sets the bound:
//!   the bound exists so that a smeared early arrival is still compared against
//!   the same chirp arriving by a later path, so it has to outlast the tail.
//! * `BOUNDARY` — how close the cluster sits to a look-back window seam. A seam
//!   falling inside a cluster is the one way a smaller bound could move a lock.
//!
//! Read-only: opens WAVs, writes nothing.

use hound::WavReader;
use modem_codec::rx::{FrameEvent, Receiver, LOOK_BACK_MILLIS};
use modem_core::phy::{FskPhy, Phy};
use modem_core::preamble::SCAN_STRIDE;
use std::path::PathBuf;

/// `Receiver::step`'s accept threshold. Duplicated deliberately: the probe
/// reports distance from *this* number, and reading it from rx would hide a
/// change to it behind an unchanged-looking report.
const ACCEPT: f32 = 0.25;

fn read_wav(path: &std::path::Path) -> anyhow::Result<Vec<f32>> {
    let mut reader = WavReader::open(path)?;
    let spec = reader.spec();
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().filter_map(|s| s.ok()).collect(),
        hound::SampleFormat::Int => reader
            .samples::<i32>()
            .filter_map(|s| s.ok())
            .map(|s| s as f32 / i32::MAX as f32)
            .collect(),
    };
    // Mono-ise by taking the first channel.
    if spec.channels > 1 {
        let ch = spec.channels as usize;
        return Ok(samples.iter().step_by(ch).copied().collect());
    }
    Ok(samples)
}

/// Score exactly one offset, through the public detector: handed a slice of
/// exactly one template length, `detect_preamble` has one grid point to score
/// and returns it. Full resolution, and the same arithmetic the receiver uses.
fn score_one(phy: &FskPhy, buf: &[f32], off: usize, tn: usize) -> f32 {
    phy.detect_preamble(&buf[off..off + tn]).map(|(_, s)| s).unwrap_or(0.0)
}

fn main() -> anyhow::Result<()> {
    let dir: PathBuf = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "captures".into())
        .into();
    let mut wavs: Vec<PathBuf> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map_or(false, |e| e == "wav"))
        .collect();
    wavs.sort();

    let phy = FskPhy::audible();
    let tn = phy.preamble().len();
    let rate = phy.sample_rate() as usize;
    let bound = rate * LOOK_BACK_MILLIS / 1_000;
    let retire = ((bound + 1) / SCAN_STRIDE) * SCAN_STRIDE;

    println!(
        "# LOOK_BACK_MILLIS={LOOK_BACK_MILLIS} bound={bound} retire={retire} \
         stride={SCAN_STRIDE} template={tn} accept={ACCEPT}"
    );

    for path in &wavs {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let buf = read_wav(path)?;
        println!("\n=== {name} ({} samples, {:.2} s)", buf.len(), buf.len() as f32 / rate as f32);

        // --- what the receiver locks on, in recording coordinates ---
        let mut rx = Receiver::new(FskPhy::audible());
        let events = rx.push_samples(&buf);
        let outcome = events
            .iter()
            .map(|e| match e {
                FrameEvent::FrameOk { seq, .. } => format!("ok:{seq}"),
                FrameEvent::FrameDropped { seq, reason } => format!("dropped:{seq}:{reason}"),
                FrameEvent::StreamComplete { sha256_ok, .. } => format!("complete:sha_ok={sha256_ok}"),
            })
            .collect::<Vec<_>>()
            .join(",");
        let locks = rx.locks().to_vec();
        match locks.first() {
            Some(&(off, score)) => println!(
                "LOCK   off={off} score={score:.4} margin_over_accept={:+.4}",
                score - ACCEPT
            ),
            None => println!("LOCK   none"),
        }
        println!("LOCKS  {locks:?}");
        println!("EVENTS {outcome}");

        // --- the unbounded scan's global winner, for comparison ---
        let (peak_off, peak_score) = match phy.detect_preamble(&buf) {
            Some(p) => p,
            None => {
                println!("PEAK   none");
                continue;
            }
        };
        println!("PEAK   off={peak_off} score={peak_score:.4}");

        // --- full-resolution cluster around the peak ---
        // Widest plausible reach: one look-back second either side, clamped.
        let lo = peak_off.saturating_sub(rate);
        let hi = (peak_off + 2 * rate).min(buf.len() - tn);
        let mut first_over: Option<usize> = None;
        let mut last_over: Option<usize> = None;
        let mut dense_peak = (peak_off, 0f32);
        // Delay bands after the peak, in ms, for the reverb tail.
        let bands_ms: [(usize, usize); 9] = [
            (0, 5),
            (5, 10),
            (10, 20),
            (20, 40),
            (40, 80),
            (80, 160),
            (160, 320),
            (320, 640),
            (640, 1000),
        ];
        let mut band_max = [0f32; 9];
        let mut band_arg = [0usize; 9];
        for off in lo..=hi {
            let s = score_one(&phy, &buf, off, tn);
            if s > ACCEPT {
                first_over = first_over.or(Some(off));
                last_over = Some(off);
            }
            if s > dense_peak.1 {
                dense_peak = (off, s);
            }
            if off > peak_off {
                let d_ms = (off - peak_off) * 1_000 / rate;
                for (b, &(from, to)) in bands_ms.iter().enumerate() {
                    if d_ms >= from && d_ms < to && s > band_max[b] {
                        band_max[b] = s;
                        band_arg[b] = off - peak_off;
                    }
                }
            }
        }
        println!(
            "DENSE  peak_off={} peak_score={:.4} (stride-4 scan said {peak_off}/{peak_score:.4})",
            dense_peak.0, dense_peak.1
        );
        match (first_over, last_over) {
            (Some(f), Some(l)) => println!(
                "CLUSTER first_over_accept={f} last_over_accept={l} width={} \
                 ahead_of_peak={} past_peak={} ({:.2} ms .. {:.2} ms)",
                l - f,
                peak_off as i64 - f as i64,
                l as i64 - peak_off as i64,
                (peak_off - f.min(peak_off)) as f32 * 1_000.0 / rate as f32,
                (l.max(peak_off) - peak_off) as f32 * 1_000.0 / rate as f32,
            ),
            _ => println!("CLUSTER nothing over {ACCEPT} in [{lo},{hi}]"),
        }
        print!("TAIL  ");
        for (b, &(from, to)) in bands_ms.iter().enumerate() {
            print!(" {from}-{to}ms:{:.4}@+{}", band_max[b], band_arg[b]);
        }
        println!();

        // --- what is ahead of the cluster, i.e. can the bound move the lock ---
        let cluster_start = first_over.unwrap_or(peak_off);
        if cluster_start > tn {
            let (ahead_off, ahead_score) = phy
                .detect_preamble(&buf[..cluster_start])
                .unwrap_or((0, 0.0));
            println!(
                "AHEAD  best_before_cluster off={ahead_off} score={ahead_score:.4} \
                 (clears accept: {})",
                ahead_score > ACCEPT
            );
        }

        // --- window-seam margin: the one hazard a smaller bound multiplies ---
        if let (Some(f), Some(l)) = (first_over, last_over) {
            let seams_inside: Vec<usize> = (((f / retire) * retire)..=(l + retire))
                .step_by(retire)
                .filter(|s| *s > f && *s <= l)
                .collect();
            let prev_seam = (f / retire) * retire;
            let next_seam = prev_seam + retire;
            println!(
                "BOUNDARY cluster=[{f},{l}] seams_inside={seams_inside:?} \
                 nearest_seams=({prev_seam},{next_seam}) gap_to_next={}",
                next_seam as i64 - l as i64
            );
        }
    }
    Ok(())
}
