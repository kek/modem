//! The symbol rate must survive the trip through the UniFFI surface.
//!
//! These are deliberately not "the parameter exists and compiles" tests. A
//! rate that is accepted and then dropped on the floor is exactly the bug
//! this surface had — `FfiTransmitter::new` called the rate-less
//! `FskPhy::audible_with`, so the Android app could only ever transmit at 50
//! sym/s — so every assertion here looks at an observable consequence: how
//! long the encoded buffer is, and whether a receiver built at the other rate
//! can decode it at all.

use modem_core::frame::{FRAME_LEN, MAX_PAYLOAD};
use modem_core::fsk::{DspVariants as CoreDspVariants, DEFAULT_SYMBOL_RATE};
use modem_core::phy::{FskPhy, Phy};
use modem_core::preamble::SYNC_WORD;
use modem_ffi::{default_symbol_rate, DspVariants, FfiFrameEvent, FfiReceiver, FfiTransmitter, Profile};

/// The candidate rate from docs/android-smoke-test.md.
const HALF: u32 = 25;

const SHA_LEN: usize = 32;

fn baseline() -> DspVariants {
    DspVariants::default()
}

/// What `Transmitter::encode` must produce for `payload_len` bytes at
/// `symbol_rate`: one fixed-length chirp plus one modulated sync+frame block
/// per frame. Derived from the PHY rather than hard-coded, so the expectation
/// tracks the frame format instead of duplicating it.
fn expected_samples(payload_len: usize, symbol_rate: u32) -> usize {
    let phy = FskPhy::audible_at(symbol_rate, CoreDspVariants::BASELINE);
    let n_frames = (payload_len + SHA_LEN + MAX_PAYLOAD - 1) / MAX_PAYLOAD;
    let preamble = <FskPhy as Phy>::preamble(&phy).len();
    n_frames * (preamble + phy.frame_data_samples(SYNC_WORD.len() + FRAME_LEN))
}

fn stream_complete(events: &[FfiFrameEvent]) -> Option<(Vec<u8>, bool)> {
    events.iter().find_map(|e| match e {
        FfiFrameEvent::StreamComplete { bytes, sha256_ok } => Some((bytes.clone(), *sha256_ok)),
        _ => None,
    })
}

fn count_stream_complete(events: &[FfiFrameEvent]) -> usize {
    events
        .iter()
        .filter(|e| matches!(e, FfiFrameEvent::StreamComplete { .. }))
        .count()
}

#[test]
fn the_transmitters_rate_changes_the_air_time_it_produces() {
    let payload = b"hello from the Pixel 8 Pro".to_vec();

    let fast = FfiTransmitter::new(Profile::Audible, baseline());
    let slow = FfiTransmitter::new_at(Profile::Audible, HALF, baseline());

    assert_eq!(fast.symbol_rate(), DEFAULT_SYMBOL_RATE);
    assert_eq!(slow.symbol_rate(), HALF);

    let at_50 = fast.encode(payload.clone());
    let at_25 = slow.encode(payload.clone());

    assert_eq!(at_50.len(), expected_samples(payload.len(), DEFAULT_SYMBOL_RATE));
    assert_eq!(at_25.len(), expected_samples(payload.len(), HALF));

    // The data section doubles; the 80 ms chirp does not scale with the rate,
    // so the buffers differ by exactly one frame's worth of symbols.
    let preamble = <FskPhy as Phy>::preamble(&FskPhy::audible()).len();
    assert_eq!(at_25.len() - preamble, 2 * (at_50.len() - preamble));
}

/// The mirror of the test below: the *receiver's* rate has to reach its PHY
/// too. A receiver asked for 25 sym/s must fail on a default-rate buffer —
/// if `new_at` were ignored on the receive side, that buffer would decode
/// cleanly and this test would be the only thing to notice.
#[test]
fn the_receivers_rate_changes_the_window_it_demodulates() {
    let fast = FfiReceiver::new(Profile::Audible, baseline());
    let slow = FfiReceiver::new_at(Profile::Audible, HALF, baseline());

    assert_eq!(fast.symbol_rate(), DEFAULT_SYMBOL_RATE);
    assert_eq!(slow.symbol_rate(), HALF);

    let payload = b"fifty symbols per second".to_vec();
    let at_50 = FfiTransmitter::new(Profile::Audible, baseline()).encode(payload.clone());
    assert_eq!(at_50.len(), expected_samples(payload.len(), DEFAULT_SYMBOL_RATE));

    assert_eq!(
        stream_complete(&fast.push_samples(at_50.clone())),
        Some((payload, true)),
        "a receiver at the default rate must decode a default-rate transmission"
    );
    assert_eq!(
        count_stream_complete(&slow.push_samples(at_50)),
        0,
        "a receiver at {HALF} sym/s must not decode a {DEFAULT_SYMBOL_RATE} sym/s transmission"
    );
}

/// The end the commission cares about: a rate chosen through the FFI reaches
/// the PHY on both sides, and only the matching rate decodes. If the rate were
/// accepted and ignored, the "matching" receiver here would be a 50 sym/s one
/// looking at a 50 sym/s buffer and both halves of this test would pass for
/// the wrong reason — hence the length assertions above pinning the buffer to
/// 25 sym/s first.
#[test]
fn only_a_receiver_at_the_matching_rate_decodes_a_25_sym_per_second_transmission() {
    let payload = b"twenty five symbols per second, over the air".to_vec();
    let tx = FfiTransmitter::new_at(Profile::Audible, HALF, baseline());
    let samples = tx.encode(payload.clone());
    assert_eq!(samples.len(), expected_samples(payload.len(), HALF));

    let matching = FfiReceiver::new_at(Profile::Audible, HALF, baseline());
    let events = matching.push_samples(samples.clone());
    assert_eq!(
        stream_complete(&events),
        Some((payload.clone(), true)),
        "a receiver at 25 sym/s must decode a 25 sym/s transmission"
    );

    let mismatched = FfiReceiver::new(Profile::Audible, baseline());
    let events = mismatched.push_samples(samples);
    assert_eq!(
        count_stream_complete(&events),
        0,
        "a receiver at the default {DEFAULT_SYMBOL_RATE} sym/s must not decode a {HALF} sym/s transmission"
    );
}

#[test]
fn the_rate_less_constructors_still_mean_the_default_rate() {
    assert_eq!(default_symbol_rate(), DEFAULT_SYMBOL_RATE);

    let payload = b"unchanged".to_vec();
    let implicit = FfiTransmitter::new(Profile::Audible, baseline());
    let explicit = FfiTransmitter::new_at(Profile::Audible, default_symbol_rate(), baseline());
    assert_eq!(implicit.encode(payload.clone()), explicit.encode(payload));

    for profile in [Profile::Audible, Profile::Ultrasonic] {
        assert_eq!(
            FfiTransmitter::new(profile, baseline()).symbol_rate(),
            DEFAULT_SYMBOL_RATE
        );
    }
    assert_eq!(
        FfiReceiver::new(Profile::Ultrasonic, baseline()).symbol_rate(),
        DEFAULT_SYMBOL_RATE
    );
}

#[test]
fn the_ultrasonic_profile_takes_the_rate_too() {
    let slow = FfiTransmitter::new_at(Profile::Ultrasonic, HALF, baseline());
    assert_eq!(slow.symbol_rate(), HALF);

    let payload = b"silent and slow".to_vec();
    let samples = slow.encode(payload.clone());
    assert_eq!(samples.len(), expected_samples(payload.len(), HALF));

    let rx = FfiReceiver::new_at(Profile::Ultrasonic, HALF, baseline());
    assert_eq!(
        stream_complete(&rx.push_samples(samples)),
        Some((payload, true))
    );
}

/// The FFI does not soften `symbol_samples_for`'s contract: a rate that does
/// not divide 48 kHz exactly is refused rather than silently misreported. It
/// surfaces to Kotlin as an unexpected-exception panic, which is the honest
/// outcome for a caller asking for a rate that cannot exist.
#[test]
#[should_panic(expected = "does not divide")]
fn a_rate_that_does_not_divide_the_sample_rate_is_refused_through_the_ffi() {
    FfiTransmitter::new_at(Profile::Audible, 7, baseline());
}
