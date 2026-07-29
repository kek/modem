use std::process::Command;

/// Encode `payload` to a WAV and decode it back, passing `extra_tx` / `extra_rx`
/// to the two subcommands. Returns the decoded bytes, or None if `rx-wav`
/// refused to produce a stream.
fn roundtrip(payload: &[u8], extra_tx: &[&str], extra_rx: &[&str]) -> Option<Vec<u8>> {
    let dir = tempfile::tempdir().unwrap();
    let in_path = dir.path().join("in.bin");
    let wav_path = dir.path().join("out.wav");
    let out_path = dir.path().join("decoded.bin");
    std::fs::write(&in_path, payload).unwrap();

    let bin = env!("CARGO_BIN_EXE_modem");
    let mut tx = Command::new(bin);
    tx.arg("tx-wav").args(extra_tx);
    let s1 = tx
        .args([in_path.to_str().unwrap(), wav_path.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(s1.success(), "tx-wav {extra_tx:?} failed");

    let mut rx = Command::new(bin);
    rx.arg("rx-wav").args(extra_rx);
    let s2 = rx
        .args([wav_path.to_str().unwrap(), "-o", out_path.to_str().unwrap()])
        .status()
        .unwrap();
    if !s2.success() {
        return None;
    }
    Some(std::fs::read(&out_path).unwrap())
}

/// The live/CLI path at half the symbol rate, end to end through the real
/// binary: `tx-wav --symbol-rate 25` must produce a recording that only
/// `rx-wav --symbol-rate 25` can read. The mismatch half matters as much as
/// the match — it is what fails if `--symbol-rate` is parsed and then dropped.
#[test]
fn cli_wav_roundtrip_at_25_sym_per_second() {
    let payload = b"twenty five symbols per second".to_vec();
    assert_eq!(
        roundtrip(&payload, &["--symbol-rate", "25"], &["--symbol-rate", "25"]).as_deref(),
        Some(payload.as_slice()),
    );
}

#[test]
fn cli_wav_decoded_at_the_default_rate_does_not_read_a_25_sym_per_second_wav() {
    let payload = b"twenty five symbols per second".to_vec();
    assert_eq!(
        roundtrip(&payload, &["--symbol-rate", "25"], &[]),
        None,
        "a 25 sym/s recording must not decode at the default rate"
    );
}

#[test]
fn cli_wav_roundtrip_ultrasonic_at_25_sym_per_second() {
    let payload = b"silent and slow".to_vec();
    let args = ["--profile", "ultrasonic", "--symbol-rate", "25"];
    assert_eq!(
        roundtrip(&payload, &args, &args).as_deref(),
        Some(payload.as_slice()),
    );
}

#[test]
fn cli_wav_roundtrip_audible() {
    let dir = tempfile::tempdir().unwrap();
    let in_path = dir.path().join("in.bin");
    let wav_path = dir.path().join("out.wav");
    let out_path = dir.path().join("decoded.bin");

    let payload: Vec<u8> = (0..400).map(|i| (i & 0xFF) as u8).collect();
    std::fs::write(&in_path, &payload).unwrap();

    let bin = env!("CARGO_BIN_EXE_modem");
    let s1 = Command::new(bin)
        .args(["tx-wav", in_path.to_str().unwrap(), wav_path.to_str().unwrap()])
        .status().unwrap();
    assert!(s1.success());

    let s2 = Command::new(bin)
        .args(["rx-wav", wav_path.to_str().unwrap(), "-o", out_path.to_str().unwrap()])
        .status().unwrap();
    assert!(s2.success());

    let back = std::fs::read(&out_path).unwrap();
    assert_eq!(back, payload);
}
