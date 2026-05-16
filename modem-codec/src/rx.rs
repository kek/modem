use modem_core::frame::{decode_frame, FrameError, FRAME_LEN};
use modem_core::phy::Phy;
use modem_core::preamble::SYNC_WORD;
use sha2::{Digest, Sha256};

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
}

impl<P: Phy> Receiver<P> {
    pub fn new(phy: P) -> Self {
        let payload_samples = phy.frame_data_samples(SYNC_WORD.len() + FRAME_LEN);
        Self {
            phy,
            state: State::Searching,
            buffer: Vec::with_capacity(payload_samples * 4),
            assembled: Vec::new(),
            seq: 0,
            payload_samples,
        }
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
                let template = self.phy.preamble();
                let tn = template.len();
                if self.buffer.len() < tn + self.payload_samples {
                    return false;
                }
                // Only search the early part; if no preamble found there,
                // slide.
                let search_end = self.buffer.len() - self.payload_samples;
                let slice = &self.buffer[..search_end + tn];
                if let Some((off, score)) = self.phy.detect_preamble(slice) {
                    // Threshold 0.3 — at the verified pure-noise ceiling
                    // (see preamble.rs::rejects_pure_noise, which asserts <0.3),
                    // well below clean-channel scores (~0.9+). Real-world
                    // over-the-air scores land ~0.3-0.4 due to speaker/mic FR
                    // and sample-rate drift. The sync-word check after preamble
                    // is the secondary filter against false positives.
                    if score > 0.3 {
                        // Drop everything up to and including the preamble.
                        self.buffer.drain(..off + tn);
                        self.state = State::AfterPreamble;
                        return true;
                    }
                }
                // No preamble found; drop oldest samples to make progress.
                let drop = tn;
                if self.buffer.len() > drop {
                    self.buffer.drain(..drop);
                }
                false
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

                // Sync word is best-effort: log mismatch but still try the frame.
                if sync_part != SYNC_WORD {
                    let diffs = sync_part.iter().zip(SYNC_WORD.iter()).filter(|(a,b)| a != b).count();
                    if diffs > 1 {
                        events.push(FrameEvent::FrameDropped { seq: self.seq, reason: format!("bad sync word ({} mismatches)", diffs) });
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
    use modem_core::phy::FskPhy;

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
