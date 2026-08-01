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
/// The seam this bound creates is no longer a hazard, and closing it cost
/// nothing measurable — not the third of this saving the note here used to
/// price it at, and not by the remedy that note named, which turned out not to
/// work at all. Successive windows
/// still partition the timeline, and a seam strictly inside a chirp's
/// above-threshold band still splits it — but `defer_to_next_window` refuses to
/// lock on a candidate within one chirp length of the trailing edge of a
/// look-back window, retiring up to it and rescanning instead, so the band and
/// its peak are examined together. The reach is a chirp length because past
/// `tn` of shift the template and the chirp do not overlap at all, which makes
/// the guarantee independent of these six recordings rather than a property of
/// their 29–91-sample bands. What it does *not* cover, deliberately, is a
/// trailing edge that is the end of buffered audio: see that method.
///
/// It was latent when it was closed, not live, and that is worth recording. No
/// seam falls inside any of the six bands at this bound or the 1 s it replaced;
/// the closest approach is capture 005, whose band begins 609 samples (12.7 ms)
/// after the seam at 96 000 — a seam both bounds share — and with bands ≤91
/// samples and seams every 12 000 the exposure was under 1% per capture. The
/// deferral nevertheless fires on three of the six, because a lock within 3 840
/// samples of a seam is deferred whether or not its band actually crosses one:
/// 001 (3 540 from the seam at 120 000), 004 (1 427 from 96 000) and 006
/// (2 155 from 96 000) each cost one rescan and re-lock on the same offset and
/// the same score. That is the price, and it is why the shape of the fix is a
/// conditional rescan rather than an overlap: a window that finds nothing
/// acceptable retires whole, as before, so the common case pays nothing.
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

    /// Whether an accepted candidate at `off` should be *deferred* to the next
    /// window instead of locked on, and if so how many samples to retire.
    /// `Some(retire)` means: drop `retire` samples and scan again, so that the
    /// candidate's whole correlation band — and therefore its true peak — is
    /// inside one window. `None` means lock.
    ///
    /// Successive windows partition the timeline, so they have seams, and a seam
    /// strictly inside a chirp's above-threshold band splits it: the earlier
    /// window sees only the band's leading edge. That is only harmful when the
    /// leading edge clears the accept threshold on its own, because then the
    /// receiver locks on a shoulder and every symbol boundary downstream is
    /// measured from an offset that is not where the chirp starts. When nothing
    /// in a window clears the threshold, retiring the whole window is lossless —
    /// the next window contains the peak and locks on it correctly. So the fix is
    /// not to overlap every window; it is to defer exactly the locks that could
    /// be split-band shoulders, which costs a partial rescan only when a
    /// candidate clears the threshold near an edge.
    ///
    /// **The reach is one chirp length, and that is what makes this structural.**
    /// Shift the template more than `tn` from the chirp and the two do not
    /// overlap at all, so a band cannot start more than `tn` before its peak.
    /// A candidate at `off` that is the leading edge of a band whose peak `p`
    /// lies beyond the window therefore satisfies both `p <= off + tn` and
    /// `p > bounded_end`, so `bounded_end - off < tn`. Contrapositive: a
    /// candidate `tn` or more before the trailing edge cannot be a shoulder of an
    /// out-of-window peak — if it were, the peak would be inside this window, and
    /// the peak outscores the shoulder, so the peak is what the scan would have
    /// returned. The guarantee holds for any recording, not just the six in
    /// `captures/`, whose bands happen to be 29–91 samples wide.
    ///
    /// Two conditions, and the second is the reason this terminates.
    ///
    /// * `bounded_end - off < tn` — the candidate is within reach of the edge.
    /// * `off + tn <= search_end` — the furthest position the peak could occupy
    ///   is already scannable, i.e. a whole frame is buffered behind it. This
    ///   confines the deferral to look-back seams: if `bounded_end == search_end`
    ///   (the trailing edge is the end of buffered audio) the two conditions
    ///   contradict each other, so it never fires there. See below.
    ///
    /// Termination. Because it only fires when `bounded_end` is the look-back
    /// bound, `off > look_back - tn`, and `look_back` (250 ms) exceeds `tn`
    /// (80 ms) at every sample rate, so `retire >= look_back - tn > 0`: every
    /// deferral retires at least 8 160 samples at 48 kHz — two thirds of a full
    /// window advance — and the scan cannot stand still. Nor can the same
    /// candidate be deferred twice: the rescanned window reaches at least `tn`
    /// past it, which is the first condition failing.
    ///
    /// `retire` rounds **down** to a whole `SCAN_STRIDE`, for the reason `step`'s
    /// retire comment and `SCAN_STRIDE`'s own give: an advance that is not a
    /// stride multiple re-phases the coarse grid onto a different set of offsets,
    /// which is the bug that moved three of the six captures' locks onto
    /// shoulders. Rounding down keeps the candidate inside the next window.
    ///
    /// **Why not overlapping windows, which is what the note above used to
    /// promise.** "Windows that overlap by a chirp length instead of abutting"
    /// turns out to mean two different changes, and both were tried against
    /// `a_seam_inside_the_band_does_not_cost_the_peak`.
    ///
    /// * *Advance less* — retire `bounded_end + 1 - tn` instead of
    ///   `bounded_end + 1`, so successive windows re-scan the last chirp length.
    ///   **This does not fix anything, and the test proves it.** The premature
    ///   lock happens in the window that first sees the leading edge; overlap
    ///   only means a *later* window would have seen the whole band, and that
    ///   window is never reached, because the receiver has already locked and
    ///   moved on. The lock decision, not the window geometry, is what has to
    ///   change.
    /// * *Look ahead* — keep the advance at `bounded_end + 1` but hand
    ///   `detect_preamble` a slice reaching `tn` further, so every window sees a
    ///   chirp length past its own trailing edge. This does work. It also widens
    ///   the comparison range every scan uses from 250 ms to 330 ms, which is the
    ///   bound `LOOK_BACK_MILLIS` was deliberately chosen to be, and it pays 32%
    ///   more correlation on every window whether or not anything is near an
    ///   edge.
    ///
    /// So the deferral is the cheaper of the two that work, and it leaves the
    /// bound alone. Cheaper in *correlations*, which is the figure worth quoting:
    /// looking ahead scans 3 961 offsets per window instead of 3 001, forever,
    /// while the deferral scans one extra partial window per candidate that clears
    /// the threshold near a seam and nothing at all otherwise — the whole
    /// workspace suite defers twice, once in the test above and once in
    /// `back_to_back_transmissions_decode_cleanly`, which passes either way. The
    /// wall clock cannot resolve any of it on the machine that measured it:
    /// `cargo test --test stream_roundtrip` gave 21.49 s on trunk, 21.17 s
    /// advancing less, 21.36 s looking ahead — and 14.16 s then 20.02 s on two
    /// consecutive runs of the *same* deferral binary. The "about a third of what
    /// this bought" that the note on `LOOK_BACK_MILLIS` priced this work at was
    /// arithmetic rather than a measurement, and it was arithmetic about the
    /// variant that does not work.
    ///
    /// **What this deliberately does not cover.** When the window's trailing edge
    /// is the end of buffered audio rather than a look-back seam, a candidate
    /// near it can still be a split-band shoulder — but there the peak is not in
    /// the next window, it is *not yet buffered*, so deferring means waiting for
    /// up to a chirp length more audio. That trades a latent misalignment for a
    /// live cost: 80 ms of added latency for every near-edge candidate on a
    /// stream, and on a whole-file push — where no more audio is coming and there
    /// is no flush — a dropped frame whenever the preamble sits within `tn` of
    /// the last scannable offset. Not worth it. The residual exposure is the last
    /// `tn` offsets of whatever has been pushed so far, and it shrinks to nothing
    /// as the buffer fills.
    fn defer_to_next_window(
        &self,
        off: usize,
        bounded_end: usize,
        search_end: usize,
    ) -> Option<usize> {
        let tn = self.preamble_samples;
        debug_assert!(
            off <= bounded_end,
            "the scan slice ends at bounded_end + tn, so no offset past \
             bounded_end can be returned: off={off} bounded_end={bounded_end}"
        );
        if bounded_end - off >= tn || off + tn > search_end {
            return None;
        }
        let retire = (off / SCAN_STRIDE) * SCAN_STRIDE;
        debug_assert!(
            retire > 0,
            "a deferral must make progress: off={off} bounded_end={bounded_end} \
             search_end={search_end} look_back={} tn={tn}",
            self.look_back
        );
        (retire > 0).then_some(retire)
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
                    // the window it lands in. That last clause used to be a fact
                    // about these six recordings; `defer_to_next_window` below
                    // makes it hold whatever is recorded, by refusing to lock
                    // where a band could be split across the window's trailing
                    // edge. The sync word + RS+CRC catch false
                    // positives that get past this.
                    if score > 0.25 {
                        // Before locking: is this candidate close enough to the
                        // window's trailing edge that it could be the *leading
                        // edge* of a band whose peak is on the other side of a
                        // seam? If so, retire up to it and rescan rather than
                        // lock, so the next window holds the whole band. See
                        // `defer_to_next_window`.
                        if let Some(retire) = self.defer_to_next_window(off, bounded_end, search_end)
                        {
                            self.drop_front(retire);
                            return true;
                        }
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

    /// The chirp, smeared into whatever is already in `buf`, at an arbitrary
    /// amplitude. Amplitude is not what holds a smeared arrival's score down —
    /// the correlation is amplitude-invariant — the surrounding noise is; this
    /// is the knob for how much of the window's energy is chirp-shaped.
    fn plant_smeared(buf: &mut [f32], at: usize, template: &[f32], amp: f32) {
        for (k, t) in template.iter().enumerate() {
            buf[at + k] += t * amp;
        }
    }

    /// A decoy: the chirp, smeared into noise. Scores above the 0.25 accept
    /// threshold, well below a clean transmission.
    fn plant_decoy(buf: &mut [f32], at: usize, template: &[f32]) {
        plant_smeared(buf, at, template, 0.55);
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

    /// A window seam falling strictly inside an above-threshold correlation band
    /// must not cost the peak.
    ///
    /// Synthetic on purpose, and it has to be: no seam falls inside any of the
    /// six real captures' bands, so the corpus cannot fail this test and cannot
    /// prove it either. What the corpus does supply is the *shape* — capture
    /// 001's band runs from 90 samples ahead of its peak to 1 past it, and its
    /// leading edge clears the 0.25 accept threshold on its own (0.2593 at
    /// 116384 against 0.2947 at 116460). This reproduces that shape across a
    /// seam: a weak smeared arrival before the seam, the stronger one after it.
    ///
    /// With windows that abut, the earlier window sees only the leading edge,
    /// finds it acceptable, and locks — every symbol boundary of the frame is
    /// then measured from an offset that is not where the chirp starts. Measured:
    /// before `defer_to_next_window` existed this locked at 11 972 on a score of
    /// 0.3673, 88 samples before the chirp, with the 0.8646 peak at 12 060 one
    /// window away.
    ///
    /// The buffer is deliberately long enough that a whole frame sits behind
    /// every offset either window scores, so the first window's trailing edge is
    /// the look-back seam and not the end of the audio. Those are different
    /// hazards with different answers — see `defer_to_next_window`.
    #[test]
    fn a_seam_inside_the_band_does_not_cost_the_peak() {
        let phy = FskPhy::audible();
        let bound = phy.sample_rate() as usize * LOOK_BACK_MILLIS / 1_000;
        let template = phy.preamble();
        let tn = template.len();
        // One frame's worth of audio, so the receiver has a whole frame buffered
        // behind every offset it scores and the trailing edge of the first
        // window is the look-back seam rather than the end of the buffer.
        let frame_samples = Transmitter::new(FskPhy::audible(), false).encode(b"one frame").len();

        // Straddle the seam: the peak past it, the leading edge before it, both
        // on the stride-4 grid so this test is about the seam and not about
        // `refine_peak`'s reach.
        let peak_at = bound + 60;
        let shoulder_at = bound - 28;
        assert!(shoulder_at < bound && bound < peak_at, "the seam must split the band");

        let mut buf = pseudo_noise(peak_at + tn + frame_samples + 2_000, 0.30);
        plant_smeared(&mut buf, shoulder_at, &template, 0.42);
        plant_smeared(&mut buf, peak_at, &template, 1.00);

        // What the first window can see: exactly the offsets 0..=bound, which is
        // the slice `step` hands to `detect_preamble`. For this test to mean
        // anything that leading edge must clear the accept threshold on its own
        // and still lose to the peak.
        let (edge_off, edge_score) = phy
            .detect_preamble(&buf[..bound + tn])
            .expect("leading edge detectable inside the first window");
        let (peak_off, peak_score) = phy.detect_preamble(&buf).expect("peak detectable");
        assert!(
            edge_off <= bound,
            "the first window cannot see past {bound}, got {edge_off}"
        );
        assert!(
            edge_score > 0.25,
            "the sub-seam leading edge must clear the 0.25 accept threshold for \
             this test to bite, got {edge_score} at {edge_off}"
        );
        assert!(
            (peak_off as i64 - peak_at as i64).abs() <= 3 && peak_score > edge_score,
            "the peak must be the later arrival at {peak_at} and beat the edge \
             {edge_score:.4}, got {peak_score:.4} at {peak_off}"
        );

        let mut rx = Receiver::new(FskPhy::audible());
        rx.push_samples(&buf);
        assert_eq!(rx.locks().len(), 1, "one arrival, one lock: {:?}", rx.locks());
        let (off, score) = rx.locks()[0];
        assert!(
            (off as i64 - peak_at as i64).abs() <= 3,
            "locked on the band's leading edge at {off} (score {score:.4}) because \
             the seam at {bound} split the band, instead of the peak at {peak_at} \
             ({peak_score:.4}); windows that abut lose the peak whenever a band \
             crosses a seam"
        );
        assert!(
            score > edge_score,
            "the lock's score {score:.4} should be the peak's, not the edge's \
             {edge_score:.4}"
        );
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
