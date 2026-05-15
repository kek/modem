use std::process::Command;

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
