use modem_core::frame::{decode_frame, FrameError, FRAME_LEN};
use modem_core::phy::Phy;
use modem_core::preamble::{SCAN_STRIDE, SYNC_WORD};
use sha2::{Digest, Sha256};

/// How far back a single preamble scan may reach, in milliseconds of audio.
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
/// 250 ms, because:
///
/// * it is three times the 80 ms chirp, so a candidate is still compared
///   against everything that could plausibly be *the same chirp* arriving by
///   another path: sound covers 86 m in 250 ms, and a reflection that late has
///   taken a detour no room provides.
/// * the six real captures say the reverb tail needs nothing like that much.
///   Measured at full resolution with
///   `cargo run --release -p modem-codec --example preamble_probe -- captures`:
///   the band of offsets around each chirp that clears the 0.25 accept
///   threshold is 29–91 samples wide (0.6–1.9 ms), and the latest offset in it
///   is 19 samples — 0.40 ms — past the peak. Past that band nothing in any of
///   the six re-correlates above **0.166** at any delay out to a full second.
///   The strongest delayed correlation, band by band across the six: 5–10 ms
///   0.085–0.137, 10–20 ms 0.069–0.102, 20–40 ms 0.037–0.089, 40–80 ms
///   0.093–0.112, 80–160 ms 0.097–0.121, 160–320 ms 0.109–0.141, 320–640 ms
///   0.125–0.145, 640 ms–1 s 0.101–0.166. Reverb is indistinguishable from the
///   floor by 5 ms; the slow *rise* at the long end is the payload's own weak
///   correlation against the chirp template, not the room. So in these rooms a
///   delayed arrival of the same chirp never becomes a candidate at all, and
///   250 ms is already two orders of magnitude more look-back than the corpus
///   can justify needing.
/// * it is far below the frame period (13.9 s at 50 sym/s), so two consecutive
///   frames' preambles can never compete: the earlier is locked and decoded
///   before the later is ever scored.
/// * it fixes the cost of one scan at 3 000 stride-4 windows however much audio
///   is buffered, and — because successive scans partition the timeline — it
///   removes the re-scan of the whole buffer that used to happen after every
///   frame. That, not the inner loop, is where `stream_roundtrip`'s 45 minutes
///   went, and the window length is what is left of it: the scan searches its
///   whole window for a maximum even when the chirp sits at offset 0, which it
///   does for every frame after the first, so one scan costs one window
///   regardless of where the preamble is.
/// * it moves no lock on the six real captures: nothing before the chirp scores
///   above the 0.25 accept threshold in any of them (the strongest correlation
///   anywhere ahead of the chirp is 0.0764 on capture 003, 0.0144–0.0377 on the
///   other five), and all six return the same offset *and* the same score as
///   the 1 s bound and as an unbounded scan — 001 116460/0.2947, 002
///   92113/0.5794, 003 91147/0.5918, 004 94573/0.6510, 005 96620/0.6130, 006
///   93845/0.5914. Capture 001 is the one to watch: 0.2947 is only 0.045 clear
///   of the threshold.
///
/// One thing this bound does *not* guarantee structurally, at 250 ms or at the
/// 1 s it replaced. Successive windows partition the timeline, so there are
/// seams, and a seam that falls strictly inside a chirp's above-threshold band
/// splits it: the earlier window sees only the band's leading edge, which on
/// capture 001 clears 0.25 on its own (0.2593 at 116384), and locks on a
/// shoulder instead of the peak. 12 000 divides 48 000, so every seam of the
/// old bound is still a seam — shortening the window cannot fix a split, only
/// add places one could happen, and it adds four times as many. Measured: none
/// of the six bands contains a seam at either bound, and the closest approach
/// is capture 005, whose band begins 609 samples (12.7 ms) after the seam at
/// 96 000 — a seam both bounds share. With bands ≤91 samples and seams every
/// 12 000, the residual exposure is under 1% per capture. Removing it needs
/// windows that overlap by a chirp length rather than abut, which costs a third
/// of what this bound just bought and is a separate change.
///
/// What the bound gives up: a chirp that starts late in a long buffer is still
/// *found* — scans sweep forward one window at a time and retire each, so every
/// offset is still examined — but a later, stronger chirp can no longer override
/// an above-threshold one more than `LOOK_BACK_MILLIS` before it. When that one is
/// a false positive the receiver now spends the following frame's samples on
/// garbage, which the sync word and RS+CRC reject, instead of skipping it. That
/// is the deliberate trade: bounded, predictable work and a lock on the first
/// acceptable arrival, against an unbounded comparison that preferred whatever
/// was loudest in the last two minutes.
///
/// Expressed in milliseconds rather than seconds only so this can be a quarter
/// of a second; `sample_rate * LOOK_BACK_MILLIS / 1_000` is exact at every rate
/// the PHYs use.
pub const LOOK_BACK_MILLIS: usize = 250;

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
    /// `LOOK_BACK_MILLIS` converted to samples for this PHY's rate.
    look_back: usize,
    /// Every sample ever dropped from the front of `buffer`, summed: the
    /// absolute index, in the stream the caller pushed, of `buffer[0]`.
    consumed: usize,
    /// One entry per preamble lock, `(offset, score)`, with the offset in the
    /// caller's stream coordinates rather than the buffer's.
    ///
    /// Bookkeeping, not behaviour — nothing in `step` reads it. It exists
    /// because the preamble gate for this receiver is *the per-capture lock
    /// offset and score* (`docs/android-smoke-test.md`), and until this existed
    /// there was no way to read either one out: the offset `detect_preamble`
    /// returns is relative to a slice whose start has already moved, and the
    /// score is discarded the moment it clears the threshold. The rank table is
    /// not a substitute — it stayed byte-identical through a bug that moved
    /// three of six locks onto correlation shoulders.
    locks: Vec<(usize, f32)>,
}

impl<P: Phy> Receiver<P> {
    pub fn new(phy: P) -> Self {
        let payload_samples = phy.frame_data_samples(SYNC_WORD.len() + FRAME_LEN);
        let preamble_samples = phy.preamble().len();
        let look_back = phy.sample_rate() as usize * LOOK_BACK_MILLIS / 1_000;
        Self {
            phy,
            state: State::Searching,
            buffer: Vec::with_capacity(payload_samples * 4),
            assembled: Vec::new(),
            seq: 0,
            payload_samples,
            preamble_samples,
            look_back,
            consumed: 0,
            locks: Vec::new(),
        }
    }

    /// The PHY this receiver demodulates with. See `Transmitter::phy`.
    pub fn phy(&self) -> &P {
        &self.phy
    }

    /// Every preamble this receiver has locked on, in order, as
    /// `(offset, score)` — the offset counted from the first sample ever pushed,
    /// so it can be compared against a figure measured on a whole recording.
    /// See the `locks` field for why this is worth reading.
    pub fn locks(&self) -> &[(usize, f32)] {
        &self.locks
    }

    /// Drop `n` samples from the front of the buffer, keeping `consumed` — and
    /// so the meaning of every offset in `locks` — correct. Every removal from
    /// the front goes through here.
    fn drop_front(&mut self, n: usize) {
        self.buffer.drain(..n);
        self.consumed += n;
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
            self.drop_front(drop);
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
                    // rather than the whole buffer (see LOOK_BACK_MILLIS), so
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
                        self.locks.push((self.consumed + off, score));
                        // Drop everything up to and including the preamble.
                        self.drop_front(off + tn);
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
                self.drop_front(retire);
                true
            }
            State::AfterPreamble => {
                if self.buffer.len() < self.payload_samples {
                    return false;
                }
                let payload_slice: Vec<f32> = self.buffer.drain(..self.payload_samples).collect();
                self.consumed += self.payload_samples;
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

    /// The look-back bound (`LOOK_BACK_MILLIS`). A weaker but acceptable
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
        let bound = phy.sample_rate() as usize * LOOK_BACK_MILLIS / 1_000;
        let template = phy.preamble();
        let tx = Transmitter::new(FskPhy::audible(), false);
        let payload = b"the later, louder transmission";
        let signal = tx.encode(payload);
        let decoy_at = 2_000usize;

        // --- beyond the bound: a gap of one and a half bounds ---
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

        // --- inside the bound: a fifth of one, both candidates in one window ---
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

    /// `locks()` reports in the caller's coordinates, not the buffer's, and the
    /// answer does not depend on how the stream was chunked.
    ///
    /// This is the readout the preamble gate is stated against — per-capture
    /// lock offset and score — so it has to survive the two things that move the
    /// buffer under it: retiring a rejected window, and draining a frame. The
    /// offset here is deliberately off the stride-4 grid, so a lock reported
    /// from an unrefined coarse winner would be three samples out.
    #[test]
    fn reports_lock_offsets_in_stream_coordinates() {
        let tx = Transmitter::new(FskPhy::audible(), false);
        let signal = tx.encode(b"where did that preamble start?");
        let lead = 5_001usize;

        let mut buf = pseudo_noise(lead, 0.02);
        buf.extend_from_slice(&signal);

        let mut whole = Receiver::new(FskPhy::audible());
        whole.push_samples(&buf);
        assert_eq!(whole.locks().len(), 1, "one frame, one lock: {:?}", whole.locks());
        let (off, score) = whole.locks()[0];
        assert_eq!(off, lead, "lock should be reported at the chirp's true start");
        assert!(score > 0.9, "clean signal should score near 1, got {score}");

        let mut chunked = Receiver::new(FskPhy::audible());
        for chunk in buf.chunks(4_800) {
            chunked.push_samples(chunk);
        }
        assert_eq!(
            chunked.locks(),
            whole.locks(),
            "chunking the same audio must not move the reported lock"
        );
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
