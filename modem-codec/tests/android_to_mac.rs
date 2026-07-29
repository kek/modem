//! Regression for the Android→Mac direction documented in
//! `docs/android-smoke-test.md`.
//!
//! That document recorded Android→Mac as broken over the air and listed three
//! DSP remedies as the suggested fix. These tests turn the channel into
//! something reproducible without hardware, so the question "do those remedies
//! actually fix it?" has a permanent, checkable answer.
//!
//! The answer they encode is: **no, not really.** The tuning pass buys a
//! sliver of extra margin and nothing more. The tests below pin down all three
//! parts of that finding:
//!
//!   1. `speaker_distortion_alone_does_not_break_the_link` — the root cause the
//!      document names (speaker harmonic distortion) does not by itself cause
//!      a single bit error, even at absurd drive.
//!   2. `android_to_mac_defeats_baseline` / `all_variants_recover_where_baseline_fails`
//!      — what does break the link is multi-symbol ISI from room
//!      reverberation, and right at the failure threshold the variants do
//!      recover it.
//!   3. `no_variant_survives_a_more_reverberant_room` — move the room only
//!      slightly past that threshold and all eight combinations fail together.
//!
//! The `halving_*` tests at the end answer the follow-up question that finding
//! left open: does dropping to 25 sym/s, the one cheap knob that attacks ISI
//! directly, buy back the direction? **In simulation, yes** — and at exactly
//! half the bitrate, which those tests pin too.

use modem_codec::rx::{FrameEvent, Receiver};
use modem_codec::tx::Transmitter;
use modem_core::channel::{
    android_to_mac, speaker_distortion, total_harmonic_distortion, AndroidToMac, SpeakerDistortion,
};
use modem_core::fsk::{DspVariants, FskConfig, DEFAULT_SYMBOL_RATE, SAMPLE_RATE};
use modem_core::phy::{FskPhy, Phy};

/// The candidate rate this file's `halving_*` tests measure: half of today's.
const HALVED_SYMBOL_RATE: u32 = 25;

/// The payload that sits closest to the failure threshold, and so is the one
/// the pinned single-case tests use.
const MARGINAL: &[u8] = b"the quick brown fox jumps over the lazy dog 0123456789";

fn corpus() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("short", b"hello from the Pixel 8 Pro".to_vec()),
        ("marginal", MARGINAL.to_vec()),
        ("take-seven", b"Android to Mac, audible profile, take seven.".to_vec()),
        ("binary180", (0..180u32).map(|i| (i.wrapping_mul(37) & 0xFF) as u8).collect()),
        ("binary200", (0..200u32).map(|i| (i.wrapping_mul(91).wrapping_add(7) & 0xFF) as u8).collect()),
        ("ascii120", (0..120u32).map(|i| b'a' + (i % 26) as u8).collect()),
    ]
}

/// The three payloads nearest the failure threshold. Sweeping all eight
/// variants across the full corpus costs about a minute of debug-mode CPU for
/// no extra signal, so the exhaustive tests use this subset.
fn hard_corpus() -> Vec<(&'static str, Vec<u8>)> {
    corpus()
        .into_iter()
        .filter(|(n, _)| matches!(*n, "short" | "marginal" | "binary180"))
        .collect()
}

fn all_variants() -> Vec<DspVariants> {
    let mut out = Vec::with_capacity(8);
    for pulse_shape in [false, true] {
        for matched_filter in [false, true] {
            for timing_recovery in [false, true] {
                out.push(DspVariants { pulse_shape, matched_filter, timing_recovery });
            }
        }
    }
    out
}

/// Transmit `payload` at `symbol_rate` with `variants` enabled, push it through
/// `channel`, and receive it at the same rate with the same variants. True iff
/// the stream came back intact.
fn survives_at(
    symbol_rate: u32,
    variants: DspVariants,
    payload: &[u8],
    channel: AndroidToMac,
) -> bool {
    let tx = Transmitter::new(FskPhy::audible_at(symbol_rate, variants), false);
    // Lead-in silence, as any real capture has.
    let mut air = vec![0f32; 4800];
    air.extend_from_slice(&tx.encode(payload));
    let heard = android_to_mac(&air, channel, SAMPLE_RATE);

    let mut rx = Receiver::new(FskPhy::audible_at(symbol_rate, variants));
    rx.push_samples(&heard).iter().any(|e| {
        matches!(e, FrameEvent::StreamComplete { bytes, sha256_ok: true } if bytes == payload)
    })
}

/// `survives_at` at today's shipping rate.
fn survives(variants: DspVariants, payload: &[u8], channel: AndroidToMac) -> bool {
    survives_at(DEFAULT_SYMBOL_RATE, variants, payload, channel)
}

fn survivors(variants: DspVariants, channel: AndroidToMac) -> usize {
    corpus().iter().filter(|(_, p)| survives(variants, p, channel)).count()
}

fn hard_survivors_at(symbol_rate: u32, variants: DspVariants, channel: AndroidToMac) -> usize {
    hard_corpus().iter().filter(|(_, p)| survives_at(symbol_rate, variants, p, channel)).count()
}

fn hard_survivors(variants: DspVariants, channel: AndroidToMac) -> usize {
    hard_survivors_at(DEFAULT_SYMBOL_RATE, variants, channel)
}

/// The `PIXEL_AT_FULL_VOLUME` preset must stay in the range real micro-speakers
/// are quoted at. If someone retunes the drive, this is the guard rail that
/// says the model stopped describing a plausible speaker.
#[test]
fn speaker_preset_thd_is_physically_plausible() {
    let n = SAMPLE_RATE as usize / 5;
    let tone: Vec<f32> = (0..n)
        .map(|k| (2.0 * std::f32::consts::PI * 2600.0 * k as f32 / SAMPLE_RATE as f32).sin() * 0.6)
        .collect();
    // THD is a property of the memoryless stage, so probe it with ring off.
    let radiated = speaker_distortion(
        &tone,
        SpeakerDistortion { ring_gain: 0.0, ..SpeakerDistortion::PIXEL_AT_FULL_VOLUME },
    );
    let thd = total_harmonic_distortion(&radiated, 2600.0, SAMPLE_RATE, 9);
    assert!(
        (0.05..=0.15).contains(&thd),
        "preset THD {:.1}% is outside the 5-15% a micro-speaker is quoted at",
        thd * 100.0
    );
}

/// The root cause `docs/android-smoke-test.md` names — speaker harmonic
/// distortion putting neighbour-tone energy into the demodulator — is not
/// sufficient to break the link, and not even close.
///
/// A memoryless odd nonlinearity on a single tone at `f` radiates only `3f`,
/// `5f`, …, all of which land outside the 2.0-3.4 kHz band and are simply not
/// seen by the tone detector. So even at a drive far past any real speaker,
/// with the speaker's ragged modal response on top, every payload decodes.
#[test]
fn speaker_distortion_alone_does_not_break_the_link() {
    let anechoic = AndroidToMac {
        // ~35% THD — getting on for four times the preset's 9.5%, and well
        // past anything a real micro-speaker does.
        speaker: SpeakerDistortion { drive: 6.0, ..SpeakerDistortion::PIXEL_AT_FULL_VOLUME },
        reverb_wet: 0.0,
        ..AndroidToMac::PIXEL_8_PRO_AT_30CM
    };
    assert_eq!(
        hard_survivors(DspVariants::BASELINE, anechoic),
        hard_corpus().len(),
        "speaker distortion alone should not cost a single frame"
    );
}

/// The regression itself: over the documented channel, the trunk decoder fails.
///
/// This is what `docs/android-smoke-test.md` observed over the air, now
/// reproducible on any machine with no hardware.
#[test]
fn android_to_mac_defeats_baseline() {
    assert!(
        !survives(DspVariants::BASELINE, MARGINAL, AndroidToMac::PIXEL_8_PRO_AT_30CM),
        "BASELINE was expected to fail over the Android→Mac channel; if this \
         now passes, the decoder improved and the doc's conclusion needs redoing"
    );
}

/// …and with all three remedies enabled, the same transmission over the same
/// channel comes back byte-identically.
#[test]
fn all_variants_recover_where_baseline_fails() {
    assert!(
        survives(DspVariants::ALL, MARGINAL, AndroidToMac::PIXEL_8_PRO_AT_30CM),
        "p+m+t was expected to recover the transmission BASELINE loses"
    );
}

/// Across the whole corpus the improvement is real but small: enabling the
/// remedies recovers strictly more payloads than trunk, and pulse-shaping plus
/// matched-filtering is where essentially all of that gain comes from.
#[test]
fn variants_improve_on_baseline_but_only_marginally() {
    let total = corpus().len();
    let base = survivors(DspVariants::BASELINE, AndroidToMac::PIXEL_8_PRO_AT_30CM);
    let all = survivors(DspVariants::ALL, AndroidToMac::PIXEL_8_PRO_AT_30CM);
    assert!(
        all > base,
        "expected the tuning pass to recover more than trunk, got {all}/{total} vs {base}/{total}"
    );
    // The gain is a sliver, not a fix. If this ever fails because `all` pulled
    // far ahead, something genuinely better happened — go update the doc.
    assert!(
        all - base <= 2,
        "tuning pass recovered {} more payloads than trunk; that is a real fix, \
         not the marginal gain the doc records",
        all - base
    );
}

/// The honest limit. Nudge the room past the threshold and every one of the
/// eight combinations fails together.
///
/// None of the three remedies is an equalizer: at 20 ms per symbol a 300 ms
/// tail spreads each symbol over the next ~15, and no amount of shaping the
/// symbol or re-windowing the detector undoes interference that arrives from
/// fifteen symbols back. That is why the document's fourth suggestion — OFDM
/// with a cyclic prefix — is the one that actually addresses this.
#[test]
fn no_variant_survives_a_more_reverberant_room() {
    let livelier = AndroidToMac { reverb_wet: 0.25, ..AndroidToMac::PIXEL_8_PRO_AT_30CM };
    for v in all_variants() {
        assert_eq!(
            hard_survivors(v, livelier),
            0,
            "variant {} unexpectedly recovered something at reverb_wet=0.25",
            v.tag()
        );
    }
}

/// Corroborates that the mechanism really is inter-symbol interference and
/// not, say, the frequency response or the distortion.
///
/// An all-`0xFF` payload modulates to one unbroken tone. There are no symbol
/// transitions, so the reverberant tail of every symbol carries exactly the
/// tone the detector already wants. That payload survives the same room that
/// destroys all six mixed-symbol payloads — under plain trunk `BASELINE`, with
/// no remedies at all.
#[test]
fn a_transition_free_payload_survives_the_room_that_kills_the_others() {
    let livelier = AndroidToMac { reverb_wet: 0.25, ..AndroidToMac::PIXEL_8_PRO_AT_30CM };
    assert!(
        survives(DspVariants::BASELINE, &[0xFF; 150], livelier),
        "a constant-tone payload has no ISI to suffer from and should survive"
    );
}

// ---------------------------------------------------------------------------
// 25 sym/s — the one cheap remedy that attacks ISI rather than symbol shape.
// ---------------------------------------------------------------------------

/// The headline: at the documented 30 cm geometry, halving the symbol rate
/// recovers the transmission that plain trunk loses — with *no* DSP remedies
/// enabled at all.
///
/// `android_to_mac_defeats_baseline` above pins that `MARGINAL` at 50 sym/s
/// under `BASELINE` fails. The only thing changed here is the symbol rate.
#[test]
fn halving_the_symbol_rate_recovers_what_baseline_loses() {
    assert!(
        survives_at(
            HALVED_SYMBOL_RATE,
            DspVariants::BASELINE,
            MARGINAL,
            AndroidToMac::PIXEL_8_PRO_AT_30CM
        ),
        "25 sym/s was expected to recover, with no remedies, the transmission \
         that 50 sym/s BASELINE loses over the same channel"
    );
}

/// And it does what none of the three remedies could: it moves the cliff.
///
/// `no_variant_survives_a_more_reverberant_room` pins that at `reverb_wet =
/// 0.25` all eight combinations go to zero together at 50 sym/s, because none
/// of them is an equalizer. Halving the symbol rate is not an equalizer
/// either, but it does not need to be — it halves how many symbols the
/// reverberant tail can reach into, and that is enough to put survivors back
/// on the board in the room that was previously a wipeout.
#[test]
fn halving_the_symbol_rate_moves_the_cliff() {
    let livelier = AndroidToMac { reverb_wet: 0.25, ..AndroidToMac::PIXEL_8_PRO_AT_30CM };
    let at_50 = hard_survivors(DspVariants::ALL, livelier);
    let at_25 = hard_survivors_at(HALVED_SYMBOL_RATE, DspVariants::ALL, livelier);
    assert_eq!(at_50, 0, "the 50 sym/s wipeout is the premise of this test");
    assert!(
        at_25 > at_50,
        "expected 25 sym/s to recover something at reverb_wet=0.25 where every \
         50 sym/s variant scores zero; got {at_25}/{} vs {at_50}/{}",
        hard_corpus().len(),
        hard_corpus().len()
    );
}

/// The bill. Halving the symbol rate halves the bitrate — 150 bps raw becomes
/// 75 bps — and doubles the time a transmission occupies the air. This is not
/// a free win, and any decision to ship it is a throughput trade.
#[test]
fn halving_the_symbol_rate_halves_the_bitrate() {
    let fast = FskConfig::audible_at(DEFAULT_SYMBOL_RATE, DspVariants::BASELINE);
    let slow = FskConfig::audible_at(HALVED_SYMBOL_RATE, DspVariants::BASELINE);
    assert_eq!(fast.symbol_rate(), 50);
    assert_eq!(slow.symbol_rate(), 25);
    assert_eq!(slow.symbol_samples, 2 * fast.symbol_samples);

    // …and that shows up as air time on a real transmission, not just in the
    // config. The preamble is a fixed 80 ms and does not scale, so the ratio
    // lands just under 2×.
    let air = |rate: u32| {
        Transmitter::new(FskPhy::audible_at(rate, DspVariants::BASELINE), false)
            .encode(MARGINAL)
            .len() as f32
            / SAMPLE_RATE as f32
    };
    let (fast_s, slow_s) = (air(DEFAULT_SYMBOL_RATE), air(HALVED_SYMBOL_RATE));
    let ratio = slow_s / fast_s;
    assert!(
        (1.98..=2.0).contains(&ratio),
        "expected ~2x air time for half the symbol rate, got {fast_s:.2}s -> {slow_s:.2}s ({ratio:.3}x)"
    );
}

/// The limit of the cheap fix, pinned so nobody reads the tests above as
/// "Android→Mac is solved". Push the room well past the documented geometry
/// and 25 sym/s dies just as thoroughly as 50 did. Slower symbols buy margin
/// against ISI; they do not equalize it away. OFDM with a cyclic prefix
/// remains the remedy that actually addresses the mechanism.
#[test]
fn halving_the_symbol_rate_is_not_a_cure_for_a_wet_room() {
    let wet = AndroidToMac { reverb_wet: 0.35, ..AndroidToMac::PIXEL_8_PRO_AT_30CM };
    for v in all_variants() {
        assert_eq!(
            hard_survivors_at(HALVED_SYMBOL_RATE, v, wet),
            0,
            "variant {} at 25 sym/s unexpectedly recovered something at reverb_wet=0.35",
            v.tag()
        );
    }
}

/// Symbol rate cannot touch the preamble, and that matters for how far these
/// results can be trusted.
///
/// The chirp is a fixed 80 ms waveform that knows nothing about symbols, so it
/// correlates identically at either rate — verified here on the actual
/// channel, not argued from the source. The real Android→Mac captures scored
/// 0.28-0.35 on the preamble where this model scores ~0.86, and whatever
/// attacks the real chirp that hard is therefore *untouched* by this change.
/// A simulated pass at 25 sym/s is a reason to go and capture, not a fix.
#[test]
fn symbol_rate_does_not_change_the_preamble() {
    let tx = Transmitter::new(FskPhy::audible_with(DspVariants::BASELINE), false);
    let mut air = vec![0f32; 4800];
    air.extend_from_slice(&tx.encode(MARGINAL));
    let heard = android_to_mac(&air, AndroidToMac::PIXEL_8_PRO_AT_30CM, SAMPLE_RATE);
    let window = &heard[..24_000];

    let fast = FskPhy::audible_at(DEFAULT_SYMBOL_RATE, DspVariants::BASELINE)
        .detect_preamble(window)
        .expect("preamble detectable");
    let slow = FskPhy::audible_at(HALVED_SYMBOL_RATE, DspVariants::BASELINE)
        .detect_preamble(window)
        .expect("preamble detectable");
    assert_eq!(fast, slow, "symbol rate must not affect preamble detection");
    assert!(
        fast.1 > 0.8,
        "the model's chirp survives this room easily (score {:.3}) — far better \
         than the 0.28-0.35 the real captures showed. That gap is unexplained \
         and no symbol-rate change can close it.",
        fast.1
    );
}
