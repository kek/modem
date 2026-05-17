//! Sweep DSP variant combinations across a capture corpus and emit a
//! structured outcome line per (capture × variant). Intended to be
//! consumed by an agent — no human-eyeballing.
//!
//! Corpus layout:
//!
//! ```text
//! captures/
//!   manifest.toml
//!   foo.wav
//!   bar.wav
//! ```
//!
//! Manifest schema:
//!
//! ```toml
//! [[capture]]
//! file = "foo.wav"
//! expected = "hello from android"     # OR expected_hex = "deadbeef"
//! direction = "android-to-mac"         # free-form label
//! profile = "audible"                  # "audible" | "ultrasonic"
//! ```
//!
//! Output (one line per cell):
//! ```text
//! capture=foo.wav variant=baseline outcome=no_preamble
//! capture=foo.wav variant=p+m outcome=ok
//! ```
//!
//! Followed by per-variant aggregates:
//! ```text
//! SUMMARY variant=baseline ok=0/20 no_preamble=8 sync_fail=4 rs_fail=8 sha_mismatch=0
//! ```

use crate::Profile;
use hound::WavReader;
use modem_codec::rx::{FrameEvent, Receiver};
use modem_core::fsk::DspVariants;
use modem_core::phy::FskPhy;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
struct Manifest {
    capture: Vec<CaptureEntry>,
}

#[derive(Deserialize)]
struct CaptureEntry {
    file: String,
    expected: Option<String>,
    expected_hex: Option<String>,
    #[serde(default)]
    direction: String,
    profile: String,
}

impl CaptureEntry {
    fn expected_bytes(&self) -> anyhow::Result<Vec<u8>> {
        if let Some(s) = &self.expected {
            return Ok(s.as_bytes().to_vec());
        }
        if let Some(h) = &self.expected_hex {
            return hex_decode(h);
        }
        anyhow::bail!("capture {} has neither expected nor expected_hex", self.file)
    }
    fn profile(&self) -> anyhow::Result<Profile> {
        match self.profile.as_str() {
            "audible" => Ok(Profile::Audible),
            "ultrasonic" => Ok(Profile::Ultrasonic),
            other => anyhow::bail!("unknown profile {other}"),
        }
    }
}

fn hex_decode(s: &str) -> anyhow::Result<Vec<u8>> {
    let cleaned: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if cleaned.len() % 2 != 0 {
        anyhow::bail!("odd-length hex string");
    }
    let mut out = Vec::with_capacity(cleaned.len() / 2);
    for i in (0..cleaned.len()).step_by(2) {
        out.push(u8::from_str_radix(&cleaned[i..i + 2], 16)?);
    }
    Ok(out)
}

fn load_wav(path: &Path) -> anyhow::Result<Vec<f32>> {
    let mut reader = WavReader::open(path)?;
    let samples: Vec<f32> = match reader.spec().sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().filter_map(|s| s.ok()).collect(),
        hound::SampleFormat::Int => reader
            .samples::<i32>()
            .filter_map(|s| s.ok())
            .map(|s| s as f32 / i32::MAX as f32)
            .collect(),
    };
    Ok(samples)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Ok,
    NoPreamble,
    SyncFail,
    RsFail,
    ShaMismatch,
    WrongBytes,
}

impl Outcome {
    fn tag(&self) -> &'static str {
        match self {
            Outcome::Ok => "ok",
            Outcome::NoPreamble => "no_preamble",
            Outcome::SyncFail => "sync_fail",
            Outcome::RsFail => "rs_fail",
            Outcome::ShaMismatch => "sha_mismatch",
            Outcome::WrongBytes => "wrong_bytes",
        }
    }
}

fn classify(events: &[FrameEvent], expected: &[u8]) -> Outcome {
    let mut saw_drop_sync = false;
    let mut saw_drop_rs = false;
    let mut saw_any_frame = false;
    for ev in events {
        match ev {
            FrameEvent::FrameOk { .. } => saw_any_frame = true,
            FrameEvent::FrameDropped { reason, .. } => {
                saw_any_frame = true;
                if reason.contains("sync") {
                    saw_drop_sync = true;
                } else if reason.contains("RS") || reason.contains("Rs") {
                    saw_drop_rs = true;
                }
            }
            FrameEvent::StreamComplete { bytes, sha256_ok } => {
                if !*sha256_ok {
                    return Outcome::ShaMismatch;
                }
                if bytes == expected {
                    return Outcome::Ok;
                }
                return Outcome::WrongBytes;
            }
        }
    }
    if !saw_any_frame {
        Outcome::NoPreamble
    } else if saw_drop_rs {
        Outcome::RsFail
    } else if saw_drop_sync {
        Outcome::SyncFail
    } else {
        Outcome::NoPreamble
    }
}

fn make_phy(profile: Profile, variants: DspVariants) -> FskPhy {
    match profile {
        Profile::Audible => FskPhy::audible_with(variants),
        Profile::Ultrasonic => FskPhy::ultrasonic_with(variants),
    }
}

fn all_variants() -> Vec<DspVariants> {
    let mut out = Vec::with_capacity(8);
    for ps in [false, true] {
        for mf in [false, true] {
            for tr in [false, true] {
                out.push(DspVariants { pulse_shape: ps, matched_filter: mf, timing_recovery: tr });
            }
        }
    }
    out
}

pub fn run(corpus: PathBuf) -> anyhow::Result<()> {
    let manifest_path = corpus.join("manifest.toml");
    let manifest_text = fs::read_to_string(&manifest_path)
        .map_err(|e| anyhow::anyhow!("read {}: {}", manifest_path.display(), e))?;
    let manifest: Manifest = toml::from_str(&manifest_text)?;

    // tally[variant.tag()] = (ok_count, total_count, per-outcome counts)
    let mut tally: BTreeMap<String, BTreeMap<&'static str, usize>> = BTreeMap::new();

    for entry in &manifest.capture {
        let wav_path = corpus.join(&entry.file);
        let samples = load_wav(&wav_path)
            .map_err(|e| anyhow::anyhow!("load {}: {}", wav_path.display(), e))?;
        let expected = entry.expected_bytes()?;
        let profile = entry.profile()?;
        for variants in all_variants() {
            let phy = make_phy(profile, variants);
            let mut rx = Receiver::new(phy);
            let events = rx.push_samples(&samples);
            let outcome = classify(&events, &expected);
            println!(
                "capture={} direction={} variant={} outcome={}",
                entry.file,
                if entry.direction.is_empty() { "?" } else { entry.direction.as_str() },
                variants.tag(),
                outcome.tag(),
            );
            let bucket = tally.entry(variants.tag()).or_default();
            *bucket.entry(outcome.tag()).or_insert(0) += 1;
            *bucket.entry("__total").or_insert(0) += 1;
        }
    }

    println!();
    for (variant, counts) in &tally {
        let total = counts.get("__total").copied().unwrap_or(0);
        let ok = counts.get("ok").copied().unwrap_or(0);
        let mut detail = String::new();
        for (k, v) in counts {
            if k.starts_with("__") || *k == "ok" { continue; }
            detail.push_str(&format!(" {k}={v}"));
        }
        println!("SUMMARY variant={variant} ok={ok}/{total}{detail}");
    }

    Ok(())
}
