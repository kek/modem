//! End-to-end through encode→modulate→channel→demodulate→decode for one frame.

use modem_core::channel::{awgn, drop_window, multipath};
use modem_core::frame::{decode_frame, encode_frame, FrameHeader, FRAME_LEN, MAX_PAYLOAD};
use modem_core::phy::{FskPhy, Phy};

fn header_of(len: usize) -> FrameHeader {
    FrameHeader { first: true, last: true, ultrasonic: false, payload_len: len as u16 }
}

fn payload() -> Vec<u8> {
    (0..MAX_PAYLOAD).map(|i| ((i * 31) & 0xFF) as u8).collect()
}

#[test]
fn clean_channel_audible() {
    let phy = FskPhy::audible();
    let p = payload();
    let frame = encode_frame(header_of(p.len()), &p).unwrap();
    let samples = phy.modulate_bytes(&frame);
    let back = phy.demodulate_bytes(&samples, FRAME_LEN);
    let (_, decoded) = decode_frame(&back).unwrap();
    assert_eq!(decoded, p);
}

#[test]
fn awgn_12db_audible_recovers() {
    let phy = FskPhy::audible();
    let p = payload();
    let frame = encode_frame(header_of(p.len()), &p).unwrap();
    let mut samples = phy.modulate_bytes(&frame);
    awgn(&mut samples, 12.0, 0xC0FFEE);
    let back = phy.demodulate_bytes(&samples, FRAME_LEN);
    let (_, decoded) = decode_frame(&back).unwrap();
    assert_eq!(decoded, p);
}

#[test]
fn single_dropout_recovers_via_rs() {
    let phy = FskPhy::audible();
    let p = payload();
    let frame = encode_frame(header_of(p.len()), &p).unwrap();
    let mut samples = phy.modulate_bytes(&frame);
    // 100 ms dropout aligned to symbol boundary (10 symbols ≈ <4 bytes)
    let symbol_samples = modem_core::fsk::FskConfig::audible().symbol_samples;
    let start = 100 * symbol_samples; // ~1s into the frame
    drop_window(&mut samples, start, 10 * symbol_samples);
    let back = phy.demodulate_bytes(&samples, FRAME_LEN);
    let (_, decoded) = decode_frame(&back).expect("100ms dropout must recover via RS");
    assert_eq!(decoded, p);
}

#[test]
fn tolerates_freq_offset_audible() {
    let phy = FskPhy::audible();
    let p = payload();
    let frame = encode_frame(header_of(p.len()), &p).unwrap();
    let samples = phy.modulate_bytes(&frame);
    // ±20 Hz clock skew at 48 kHz (~400 ppm — well above the worst-case
    // consumer crystal). Pad with silence so the demodulator always has the
    // symbol-aligned window it expects after the resample shortened the buffer.
    let cfg_samples = modem_core::fsk::FskConfig::audible().symbol_samples;
    let needed = samples.len();
    for hz in [-20.0f32, 20.0] {
        let mut skewed = modem_core::channel::freq_offset(&samples, hz, 48_000);
        if skewed.len() < needed {
            skewed.resize(needed, 0.0);
        }
        // Round to symbol boundary, take exactly `needed` samples (= 691 symbols).
        let n = (skewed.len() / cfg_samples) * cfg_samples;
        let back = phy.demodulate_bytes(&skewed[..n], FRAME_LEN);
        let (_, decoded) = decode_frame(&back)
            .unwrap_or_else(|e| panic!("freq_offset {hz} Hz must be tolerated, got: {e:?}"));
        assert_eq!(decoded, p);
    }
}

/// Where clock-skew tolerance actually runs out, and that halving the symbol
/// rate does not move it.
///
/// `docs/android-smoke-test.md` states, as part of costing the drop to
/// 25 sym/s, that ±30 Hz decodes at both rates and ±40 Hz fails at both. This
/// pins that claim rather than leaving it as prose the suite does not defend.
///
/// It is not obvious that the two rates should agree: a fixed *Hz* offset slips
/// twice as many samples over a 25 sym/s frame, because the frame occupies
/// twice the air time. But the symbol window doubles with it, so the slip
/// measured as a fraction of a symbol — which is what the demodulator's fixed
/// grid actually cares about — is identical, and so is the breaking point.
#[test]
fn freq_offset_tolerance_is_the_same_at_both_symbol_rates() {
    let p = payload();
    let frame = encode_frame(header_of(p.len()), &p).unwrap();

    // Ordered, so the first entry that fails is the boundary.
    const TOLERATED_HZ: [f32; 2] = [20.0, 30.0];
    const BEYOND_HZ: f32 = 40.0;

    for rate in [50u32, 25] {
        let phy = FskPhy::audible_at(rate, Default::default());
        let cfg_samples = modem_core::fsk::FskConfig::audible_at(rate, Default::default()).symbol_samples;
        let samples = phy.modulate_bytes(&frame);
        let needed = samples.len();

        // Pad with silence so the demodulator always has the symbol-aligned
        // window it expects after the resample shortened the buffer.
        let decode_at = |hz: f32| {
            let mut skewed = modem_core::channel::freq_offset(&samples, hz, 48_000);
            if skewed.len() < needed {
                skewed.resize(needed, 0.0);
            }
            let n = (skewed.len() / cfg_samples) * cfg_samples;
            let back = phy.demodulate_bytes(&skewed[..n], FRAME_LEN);
            decode_frame(&back).map(|(_, decoded)| decoded)
        };

        for hz in TOLERATED_HZ {
            for signed in [-hz, hz] {
                let decoded = decode_at(signed).unwrap_or_else(|e| {
                    panic!("{rate} sym/s must tolerate {signed} Hz skew, got: {e:?}")
                });
                assert_eq!(decoded, p, "{rate} sym/s at {signed} Hz decoded wrongly");
            }
        }
        for signed in [-BEYOND_HZ, BEYOND_HZ] {
            assert!(
                decode_at(signed).is_err(),
                "{rate} sym/s unexpectedly survived {signed} Hz skew — tolerance \
                 improved, so the figure quoted in docs/android-smoke-test.md is stale"
            );
        }
    }
}

#[test]
fn small_multipath_recovers() {
    let phy = FskPhy::audible();
    let p = payload();
    let frame = encode_frame(header_of(p.len()), &p).unwrap();
    let mut samples = phy.modulate_bytes(&frame);
    multipath(&mut samples, 240 /* 5 ms */, 0.3);
    let back = phy.demodulate_bytes(&samples, FRAME_LEN);
    let (_, decoded) = decode_frame(&back).unwrap();
    assert_eq!(decoded, p);
}
