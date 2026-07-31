use modem_core::frame::{decode_frame, FrameError, FRAME_LEN};
use modem_core::phy::Phy;
use modem_core::preamble::{SCAN_STRIDE, SYNC_WORD};
use sha2::{Digest, Sha256};

/// How far back a single preamble scan may reach, in seconds of audio.
///
/// The scan takes the *strongest* correlation in the window it is given. That
/// is the right rule for choosing between two arrivals of the same chirp — a
/// smeared reflection and the direct path — and the wrong rule for choosing
/// between two different transmissions. Unbounded, which is what this receiver
/// used to be (it searched whatever happened to be buffered: 126 s of offsets
/// when a ten-frame file is pushed in one call), it also means a distant strong
/// peak beats a nearer acceptable one, so the receiver could walk past a frame
/// it could have decoded in order to lock on a louder one later in the
/// recording. Nobody chose that.
///
/// One second, because:
///
/// * it is more than twelve times the 80 ms chirp, so a candidate is still
///   compared against everything that could plausibly be *the same chirp*
///   arriving by another path. Sound covers 343 m in a second; a reflection a
///   second late has taken a detour no room provides.
/// * it is far below the frame period (13.9 s at 50 sym/s), so two consecutive
///   frames' preambles can never compete: the earlier is locked and decoded
///   before the later is ever scored.
/// * it fixes the cost of one scan at 12 000 stride-4 windows however much
///   audio is buffered, and — because successive scans partition the timeline —
///   it removes the re-scan of the whole buffer that used to happen after every
///   frame. That, not the inner loop, is where `stream_roundtrip`'s 45 minutes
///   went.
/// * it moves no lock on the six real captures: nothing before the chirp scores
///   above the 0.25 accept threshold in any of them (the first window over
///   threshold is 76 samples before the peak on capture 001 and within 12 on the
///   other five — see docs/android-smoke-test.md).
///
/// What the bound gives up: a chirp that starts late in a long buffer is still
/// *found* — scans sweep forward one window at a time and retire each, so every
/// offset is still examined — but a later, stronger chirp can no longer override
/// an above-threshold one more than a second before it. When that earlier one is
/// a false positive the receiver now spends the following frame's samples on
/// garbage, which the sync word and RS+CRC reject, instead of skipping it. That
/// is the deliberate trade: bounded, predictable work and a lock on the first
/// acceptable arrival, against an unbounded comparison that preferred whatever
/// was loudest in the last two minutes.
pub const LOOK_BACK_SECONDS: usize = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameEvent {
    FrameOk { seq: u32, bytes: Vec<u8> },
    FrameDropped { seq: u32, reason: String },
    StreamComplete { bytes: Vec<u8>, sha256_ok: bool },
}

enum State {
    Searching,
    AfterPreamble,
}

pub struct Receiver<P: Phy> {
    phy: P,
    state: State,
    buffer: Vec<f32>,
    assembled: Vec<u8>,
    seq: u32,
    /// Number of samples needed for sync(2 B) + frame(FRAME_LEN bytes).
    payload_samples: usize,
    /// Length of the preamble chirp in samples. Cached: it is a fixed 80 ms
    /// waveform, and `Phy::preamble` hands out a fresh `Vec` every call, which
    /// `step` used to clone once per scan for nothing but its length.
    preamble_samples: usize,
    /// `LOOK_BACK_SECONDS` converted to samples for this PHY's rate.
    look_back: usize,
}

impl<P: Phy> Receiver<P> {
    pub fn new(phy: P) -> Self {
        let payload_samples = phy.frame_data_samples(SYNC_WORD.len() + FRAME_LEN);
        let preamble_samples = phy.preamble().len();
        let look_back = phy.sample_rate() as usize * LOOK_BACK_SECONDS;
        Self {
            phy,
            state: State::Searching,
            buffer: Vec::with_capacity(payload_samples * 4),
            assembled: Vec::new(),
            seq: 0,
            payload_samples,
            preamble_samples,
            look_back,
        }
    }

    /// The PHY this receiver demodulates with. See `Transmitter::phy`.
    pub fn phy(&self) -> &P {
        &self.phy
    }

    pub fn push_samples(&mut self, samples: &[f32]) -> Vec<FrameEvent> {
        self.buffer.extend_from_slice(samples);
        let mut events = Vec::new();
        loop {
            let progressed = self.step(&mut events);
            if !progressed { break; }
        }
        // Trim buffer to bound memory: keep at most ~4 frames worth
        let max_keep = self.payload_samples * 4;
        if self.buffer.len() > max_keep {
            let drop = self.buffer.len() - max_keep;
            self.buffer.drain(..drop);
        }
        events
    }

    fn step(&mut self, events: &mut Vec<FrameEvent>) -> bool {
        match self.state {
            State::Searching => {
                let tn = self.preamble_samples;
                if self.buffer.len() < tn + self.payload_samples {
                    return false;
                }
                // The highest offset whose whole frame is already buffered,
                // then the look-back bound applied to it. `bounded_end` is
                // inclusive: this scan examines offsets 0..=bounded_end and
                // nothing else.
                let search_end = self.buffer.len() - self.payload_samples;
                let bounded_end = search_end.min(self.look_back);
                let slice = &self.buffer[..bounded_end + tn];
                if let Some((off, score)) = self.phy.detect_preamble(slice) {
                    // Threshold 0.25 — near the verified pure-noise ceiling
                    // (preamble.rs::rejects_pure_noise asserts <0.3 for a
                    // specific synthetic case; real ambient noise scores
                    // lower). Clean-channel scores are >0.9. Real over-the-air
                    // through a Pixel speaker into a MacBook mic peaks at
                    // 0.295-0.651, measured over the six captures ranked in
                    // docs/android-smoke-test.md.
                    //
                    // The threshold is applied to the *peak*, not to the
                    // leading edge of the correlation ramp: detect_preamble
                    // searches all of `slice` for the maximum and only then
                    // returns — `slice` is now the bounded look-back window
                    // rather than the whole buffer (see LOOK_BACK_SECONDS), so
                    // this is the peak of the window, and the window is where
                    // the earliest acceptable candidate lives. `slice` always
                    // extends `payload_samples`
                    // (13.9 s at 50 sym/s) behind the newest sample, so every
                    // candidate offset is examined with the whole chirp and its
                    // neighbourhood already buffered. A comment here used to
                    // claim this accepted the first window over threshold; it
                    // does not, and never did — measured on all six captures at
                    // whole-file, 4800- and 1024-sample chunking, the lock lands
                    // exactly on the peak. It still does with the bound in
                    // place, because in those recordings nothing ahead of the
                    // chirp clears 0.25, so the chirp's own peak is the peak of
                    // the window it lands in. The sync word + RS+CRC catch false
                    // positives that get past this.
                    if score > 0.25 {
                        // Drop everything up to and including the preamble.
                        self.buffer.drain(..off + tn);
                        self.state = State::AfterPreamble;
                        return true;
                    }
                }
                // Nothing acceptable in this window. Retire the offsets just
                // examined and report progress, so the caller's loop scans the
                // next window instead of waiting for more audio. This is
                // lossless: offset `i` needs samples `i..i + tn`, so once every
                // offset up to `bounded_end` has been scored and rejected, no
                // future window can start below it.
                //
                // Retire a whole number of `SCAN_STRIDE` steps, not
                // `bounded_end + 1` samples. `detect_preamble` strides from
                // index 0 of the slice it is given, so the coarse grid's phase
                // is set by where the buffer starts: advance by a non-multiple
                // and every later window scores a *different* set of offsets
                // than an unbounded scan of the same audio would have. Aligned,
                // successive windows scan exactly the grid the unbounded scan
                // scanned, so the bound changes only which candidates compete —
                // not which offsets exist. Measured: unaligned, three of the six
                // real captures lock somewhere other than their peak; aligned,
                // all six lock on the same offset and score as before, to four
                // decimals.
                //
                // The last grid point of this window is re-scored as the first
                // of the next (`retire == bounded_end` when the bound bites).
                // That is one duplicated correlation per window, and it is what
                // keeps `refine_peak`'s ±3 reach around every grid point intact
                // across the seam.
                let retire = ((bounded_end + 1) / SCAN_STRIDE) * SCAN_STRIDE;
                if retire == 0 {
                    // Fewer than one grid step left to retire: wait for more
                    // audio rather than lose the grid's phase.
                    return false;
                }
                self.buffer.drain(..retire);
                true
            }
            State::AfterPreamble => {
                if self.buffer.len() < self.payload_samples {
                    return false;
                }
                let payload_slice: Vec<f32> = self.buffer.drain(..self.payload_samples).collect();
                self.state = State::Searching;

                let raw = self.phy.demodulate_bytes(&payload_slice, SYNC_WORD.len() + FRAME_LEN);
                if raw.len() < SYNC_WORD.len() + FRAME_LEN {
                    events.push(FrameEvent::FrameDropped { seq: self.seq, reason: "short demod".into() });
                    self.seq += 1;
                    return true;
                }
                let (sync_part, frame_part) = raw.split_at(SYNC_WORD.len());

                // Sync word: count *bit* mismatches across the 16-bit sync.
                // Allow up to 4 (75% bits correct still strongly indicates a
                // real preamble; pure noise scores ~8). RS+CRC catches the
                // rest if this lets a false preamble through.
                if sync_part != SYNC_WORD {
                    let bit_diffs: u32 = sync_part
                        .iter()
                        .zip(SYNC_WORD.iter())
                        .map(|(a, b)| (a ^ b).count_ones())
                        .sum();
                    if bit_diffs > 4 {
                        events.push(FrameEvent::FrameDropped { seq: self.seq, reason: format!("bad sync word ({} bit diffs)", bit_diffs) });
                        self.seq += 1;
                        return true;
                    }
                }

                match decode_frame(frame_part) {
                    Ok((header, payload)) => {
                        if header.first {
                            self.assembled.clear();
                            self.seq = 0;
                        }
                        self.assembled.extend_from_slice(&payload);
                        events.push(FrameEvent::FrameOk { seq: self.seq, bytes: payload });
                        if header.last {
                            // Last 32 bytes of assembled are the SHA-256.
                            if self.assembled.len() >= 32 {
                                let (body, sha_tail) = self.assembled.split_at(self.assembled.len() - 32);
                                let computed = Sha256::digest(body);
                                let ok = computed.as_slice() == sha_tail;
                                events.push(FrameEvent::StreamComplete { bytes: body.to_vec(), sha256_ok: ok });
                            } else {
                                events.push(FrameEvent::StreamComplete { bytes: vec![], sha256_ok: false });
                            }
                            self.assembled.clear();
                            self.seq = 0;
                            return true;
                        }
                        self.seq += 1;
                    }
                    Err(FrameError::RsUncorrectable) => {
                        events.push(FrameEvent::FrameDropped { seq: self.seq, reason: "RS uncorrectable".into() });
                        self.seq += 1;
                    }
                    Err(e) => {
                        events.push(FrameEvent::FrameDropped { seq: self.seq, reason: format!("{e}") });
                        self.seq += 1;
                    }
                }
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tx::Transmitter;
    use modem_core::phy::{FskPhy, Phy};

    /// Deterministic pseudo-noise in [-amp, amp]. No RNG, for the reason
    /// `preamble.rs` gives: a detector test that only fails on some seeds is
    /// worse than no test.
    fn pseudo_noise(n: usize, amp: f32) -> Vec<f32> {
        (0..n)
            .map(|i| ((((i * 7919) % 17) as f32 / 17.0) - 0.5) * 2.0 * amp)
            .collect()
    }

    /// A decoy: the chirp, smeared into noise. Scores above the 0.25 accept
    /// threshold, well below a clean transmission.
    fn plant_decoy(buf: &mut [f32], at: usize, template: &[f32]) {
        for (k, t) in template.iter().enumerate() {
            buf[at + k] += t * 0.55;
        }
    }

    /// Plant a real transmission on a near-silent background.
    fn plant_signal(buf: &mut [f32], at: usize, signal: &[f32]) {
        for (k, x) in signal.iter().enumerate() {
            buf[at + k] = buf[at + k] * 0.02 + x;
        }
    }

    /// The look-back bound (`LOOK_BACK_SECONDS`). A weaker but acceptable
    /// preamble further back must win over a stronger one that is beyond the
    /// bound — the receiver locks on the first acceptable arrival instead of
    /// scanning on for whatever is loudest in the whole buffer.
    ///
    /// The two halves of the test are the same buffer with one number changed,
    /// the gap between decoy and real transmission:
    ///
    ///   * gap beyond the bound  → the decoy wins (this is the bound)
    ///   * gap inside the bound  → the stronger real chirp wins (this is the
    ///     peak-picking `preamble.rs` guarantees, unchanged inside the window)
    #[test]
    fn does_not_look_back_past_the_bound() {
        let phy = FskPhy::audible();
        let bound = phy.sample_rate() as usize * LOOK_BACK_SECONDS;
        let template = phy.preamble();
        let tx = Transmitter::new(FskPhy::audible(), false);
        let payload = b"the later, louder transmission";
        let signal = tx.encode(payload);
        let decoy_at = 2_000usize;

        // --- beyond the bound: 1.5 s of gap, bound is 1 s ---
        let far = decoy_at + bound + bound / 2;
        let mut buf = pseudo_noise(far + signal.len() + 1_000, 0.30);
        plant_decoy(&mut buf, decoy_at, &template);
        plant_signal(&mut buf, far, &signal);

        // Unbounded, the real chirp is the global peak — that is what the old
        // receiver locked on, and what makes this test about the bound rather
        // than about peak-picking.
        let (peak_off, peak_score) = phy.detect_preamble(&buf).expect("global peak");
        assert!(
            (peak_off as i64 - far as i64).abs() <= 3,
            "the real transmission should be the global peak, got {peak_off} (want {far})"
        );
        let (decoy_off, decoy_score) = phy
            .detect_preamble(&buf[..far])
            .expect("decoy detectable on its own");
        assert!(
            (decoy_off as i64 - decoy_at as i64).abs() <= 3,
            "decoy should be found near {decoy_at}, got {decoy_off}"
        );
        assert!(
            decoy_score > 0.25 && decoy_score < peak_score,
            "decoy {decoy_score} must clear the 0.25 threshold and stay under \
             the real peak {peak_score} for this test to mean anything"
        );

        let mut rx = Receiver::new(FskPhy::audible());
        let events = rx.push_samples(&buf);
        assert!(
            matches!(events.first(), Some(FrameEvent::FrameDropped { .. })),
            "the scan must not reach {far} from a window that starts at 0 \
             (bound {bound}); expected the decoy at {decoy_at} to be locked on \
             and its garbage frame dropped, got {events:?}"
        );
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, FrameEvent::StreamComplete { sha256_ok: true, .. })),
            "the real transmission is consumed as the decoy's payload; it \
             cannot also decode: {events:?}"
        );

        // --- inside the bound: 0.2 s of gap, both candidates in one window ---
        let near = decoy_at + bound / 5;
        let mut buf = pseudo_noise(near + signal.len() + 1_000, 0.30);
        plant_decoy(&mut buf, decoy_at, &template);
        plant_signal(&mut buf, near, &signal);

        let mut rx = Receiver::new(FskPhy::audible());
        let events = rx.push_samples(&buf);
        assert!(
            matches!(events.first(), Some(FrameEvent::FrameOk { seq: 0, .. })),
            "within one look-back window the stronger chirp still wins, so the \
             decoy at {decoy_at} must lose to the transmission at {near}: {events:?}"
        );
        let complete = events
            .iter()
            .find_map(|e| match e {
                FrameEvent::StreamComplete { bytes, sha256_ok: true } => Some(bytes.clone()),
                _ => None,
            })
            .expect("no StreamComplete event");
        assert_eq!(complete.as_slice(), payload.as_slice());
    }

    #[test]
    fn single_frame_roundtrip() {
        let tx = Transmitter::new(FskPhy::audible(), false);
        let payload = b"hello, acoustic world!";
        let samples = tx.encode(payload);

        let mut rx = Receiver::new(FskPhy::audible());
        let events = rx.push_samples(&samples);

        // Expect at least one FrameOk and a StreamComplete with sha256_ok.
        let complete = events.iter().find_map(|e| match e {
            FrameEvent::StreamComplete { bytes, sha256_ok } => Some((bytes.clone(), *sha256_ok)),
            _ => None,
        }).expect("no StreamComplete event");
        assert!(complete.1, "sha256 should match");
        assert_eq!(complete.0, payload);
    }

    #[test]
    fn multi_frame_roundtrip() {
        let tx = Transmitter::new(FskPhy::audible(), false);
        let payload: Vec<u8> = (0..500).map(|i| (i & 0xFF) as u8).collect();
        let samples = tx.encode(&payload);

        let mut rx = Receiver::new(FskPhy::audible());
        let events = rx.push_samples(&samples);

        let complete = events.iter().find_map(|e| match e {
            FrameEvent::StreamComplete { bytes, sha256_ok } => Some((bytes.clone(), *sha256_ok)),
            _ => None,
        }).expect("no StreamComplete event");
        assert!(complete.1);
        assert_eq!(complete.0, payload);
    }

    #[test]
    fn back_to_back_transmissions_decode_cleanly() {
        let tx = Transmitter::new(FskPhy::audible(), false);
        let mut rx = Receiver::new(FskPhy::audible());

        // Multi-frame first transmission: feed all frames except the last.
        // The receiver will collect ok frames into `assembled` but never see
        // StreamComplete, leaving stale bytes that, without the fix, get
        // prepended to the next transmission.
        let big: Vec<u8> = (0..500).map(|i| (i & 0xFF) as u8).collect();
        let samples = tx.encode(&big);
        // 500 bytes + 32 sha = 532 bytes → 3 frames. Drop the final frame.
        // Each frame occupies the same number of samples in the audio stream.
        let per_frame = samples.len() / 3;
        let _ = rx.push_samples(&samples[..per_frame * 2]);
        // Silence between transmissions so the receiver settles.
        let _ = rx.push_samples(&vec![0f32; 48_000]);

        let good = tx.encode(b"second message");
        let events = rx.push_samples(&good);
        let complete = events.iter().find_map(|e| match e {
            FrameEvent::StreamComplete { bytes, sha256_ok: true } => Some(bytes.clone()),
            _ => None,
        });
        assert_eq!(complete.as_deref(), Some(&b"second message"[..]));
    }

    #[test]
    fn finds_preamble_with_silence_padding() {
        let tx = Transmitter::new(FskPhy::audible(), false);
        let samples = tx.encode(b"abc");
        let mut padded = vec![0f32; 4800];
        padded.extend_from_slice(&samples);
        padded.extend_from_slice(&[0f32; 4800]);

        let mut rx = Receiver::new(FskPhy::audible());
        let events = rx.push_samples(&padded);
        assert!(events.iter().any(|e| matches!(e, FrameEvent::StreamComplete { sha256_ok: true, .. })));
    }
}
