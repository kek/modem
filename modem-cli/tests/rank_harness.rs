//! Smoke test for `modem rank`. Generates a synthetic clean WAV with the
//! tx-wav command, writes a tiny manifest, runs rank, and verifies that
//! the baseline variant reports `ok` (since the synthetic capture has no
//! channel impairments). Confirms the harness's plumbing end-to-end.

use std::process::Command;

const VARIANTS: [&str; 8] = ["baseline", "t", "m", "m+t", "p", "p+t", "p+m", "p+m+t"];

/// Encode `payload` to a WAV at `symbol_rate`, index it in a manifest, rank it,
/// and return the harness's stdout.
fn rank_one(symbol_rate: u32, manifest_rate_line: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let corpus = dir.path().join("captures");
    std::fs::create_dir(&corpus).unwrap();

    let payload = b"hello rank harness";
    let in_path = corpus.join("in.bin");
    let wav_path = corpus.join("synth-001.wav");
    std::fs::write(&in_path, payload).unwrap();

    let bin = env!("CARGO_BIN_EXE_modem");

    let s1 = Command::new(bin)
        .args([
            "tx-wav",
            "--symbol-rate",
            &symbol_rate.to_string(),
            in_path.to_str().unwrap(),
            wav_path.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(s1.success(), "tx-wav failed");

    let manifest = format!(
        "[[capture]]\nfile = \"synth-001.wav\"\nexpected = \"hello rank harness\"\n\
         direction = \"synthetic\"\nprofile = \"audible\"\n{manifest_rate_line}"
    );
    std::fs::write(corpus.join("manifest.toml"), manifest).unwrap();

    let out = Command::new(bin)
        .args(["rank", corpus.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success(), "rank failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

fn assert_all_variants_ok(stdout: &str, rate: u32) {
    for v in VARIANTS {
        for needle in [
            format!("rate={rate} variant={v} outcome=ok"),
            format!("SUMMARY rate={rate} variant={v} ok=1/1"),
        ] {
            assert!(stdout.contains(&needle), "expected `{needle}` in rank output:\n{stdout}");
        }
    }
}

#[test]
fn rank_reports_ok_for_clean_synthetic_capture() {
    // No `symbol_rate` in the manifest — the harness must fall back to 50.
    assert_all_variants_ok(&rank_one(50, ""), 50);
}

/// The manifest's `symbol_rate` has to reach the decoder, or a 25 sym/s
/// recording would be replayed at 50 and score zero everywhere.
#[test]
fn rank_honours_the_manifests_symbol_rate() {
    assert_all_variants_ok(&rank_one(25, "symbol_rate = 25\n"), 25);
}

/// …and the converse, which is what makes the field load-bearing rather than
/// decorative: rank a 25 sym/s recording as if it were 50 and nothing decodes.
#[test]
fn rank_at_the_wrong_symbol_rate_decodes_nothing() {
    let stdout = rank_one(25, "symbol_rate = 50\n");
    for v in VARIANTS {
        let needle = format!("SUMMARY rate=50 variant={v} ok=0/1");
        assert!(
            stdout.contains(&needle),
            "expected `{needle}` — a rate mismatch must not decode:\n{stdout}"
        );
    }
}
