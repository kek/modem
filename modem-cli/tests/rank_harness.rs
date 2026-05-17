//! Smoke test for `modem rank`. Generates a synthetic clean WAV with the
//! tx-wav command, writes a tiny manifest, runs rank, and verifies that
//! the baseline variant reports `ok` (since the synthetic capture has no
//! channel impairments). Confirms the harness's plumbing end-to-end.

use std::process::Command;

#[test]
fn rank_reports_ok_for_clean_synthetic_capture() {
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
            in_path.to_str().unwrap(),
            wav_path.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(s1.success(), "tx-wav failed");

    let manifest = r#"
[[capture]]
file = "synth-001.wav"
expected = "hello rank harness"
direction = "synthetic"
profile = "audible"
"#;
    std::fs::write(corpus.join("manifest.toml"), manifest).unwrap();

    let out = Command::new(bin)
        .args(["rank", corpus.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success(), "rank failed: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8(out.stdout).unwrap();

    // Every variant should decode the clean synthetic capture as `ok`.
    let expected_variants = [
        "baseline", "t", "m", "m+t", "p", "p+t", "p+m", "p+m+t",
    ];
    for v in expected_variants {
        let needle = format!("variant={v} outcome=ok");
        assert!(
            stdout.contains(&needle),
            "expected `{needle}` in rank output:\n{stdout}"
        );
    }

    // Summary lines: every variant should be 1/1.
    for v in expected_variants {
        let needle = format!("SUMMARY variant={v} ok=1/1");
        assert!(
            stdout.contains(&needle),
            "expected `{needle}` in rank output:\n{stdout}"
        );
    }
}
