use modem_codec::rx::{FrameEvent, Receiver};
use modem_codec::tx::Transmitter;
use modem_core::phy::FskPhy;
use proptest::prelude::*;

fn run_roundtrip(payload: Vec<u8>) {
    let tx = Transmitter::new(FskPhy::audible(), false);
    let samples = tx.encode(&payload);

    let mut rx = Receiver::new(FskPhy::audible());
    let events = rx.push_samples(&samples);

    let got = events.iter().find_map(|e| match e {
        FrameEvent::StreamComplete { bytes, sha256_ok: true } => Some(bytes.clone()),
        _ => None,
    });
    assert_eq!(got.as_deref(), Some(payload.as_slice()));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn arbitrary_bytes_clean_channel(payload in proptest::collection::vec(any::<u8>(), 0..2048)) {
        run_roundtrip(payload);
    }
}

#[test]
fn boundary_lengths() {
    // Sizes that hit chunk boundaries: 0, 1, 219 (exactly 1 chunk before sha),
    // 220 (forces 2 chunks), 437, 438, 1024.
    for &n in &[0usize, 1, 100, 186, 187, 188, 218, 219, 220, 437, 438, 1024] {
        let payload: Vec<u8> = (0..n).map(|i| (i & 0xFF) as u8).collect();
        run_roundtrip(payload);
    }
}
