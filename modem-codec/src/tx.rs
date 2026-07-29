use modem_core::frame::{encode_frame, FrameHeader, MAX_PAYLOAD};
use modem_core::phy::Phy;
use modem_core::preamble::SYNC_WORD;
use sha2::{Digest, Sha256};

pub struct Transmitter<P: Phy> {
    phy: P,
    ultrasonic: bool,
}

impl<P: Phy> Transmitter<P> {
    pub fn new(phy: P, ultrasonic: bool) -> Self {
        Self { phy, ultrasonic }
    }

    /// The PHY this transmitter modulates with. A caller that wants to report
    /// what is actually going on the wire — symbol rate, tone frequencies —
    /// should ask the PHY rather than repeat what it passed in.
    pub fn phy(&self) -> &P {
        &self.phy
    }

    /// Encode `payload` into a complete audio sample buffer (preamble + sync + frame, repeated).
    pub fn encode(&self, payload: &[u8]) -> Vec<f32> {
        // Append SHA-256 to last frame's data.
        let sha = Sha256::digest(payload);
        let mut full = Vec::with_capacity(payload.len() + 32);
        full.extend_from_slice(payload);
        full.extend_from_slice(&sha);

        // Per-frame app payload max: 219 normally, but the *last* frame must fit
        // the trailing 32-byte sha. We treat it as if everything is one stream
        // up to 219 bytes per frame, and the sha is just part of the last
        // frame's bytes.
        let chunks: Vec<&[u8]> = full.chunks(MAX_PAYLOAD).collect();
        let n_frames = chunks.len();

        let mut out: Vec<f32> = Vec::new();
        let preamble = self.phy.preamble();
        for (i, chunk) in chunks.iter().enumerate() {
            // Each frame: preamble | sync_word_modulated | frame_bytes_modulated
            let header = FrameHeader {
                first: i == 0,
                last: i == n_frames - 1,
                ultrasonic: self.ultrasonic,
                payload_len: chunk.len() as u16,
            };
            let frame_bytes = encode_frame(header, chunk).expect("payload size checked above");

            // IMPORTANT: modulate sync + frame as ONE byte buffer.
            // bytes_to_symbols packs bits across byte boundaries; splitting into
            // two modulate calls would introduce padding bits between them that
            // the receiver's single demodulate window would not expect.
            let mut combined = Vec::with_capacity(SYNC_WORD.len() + frame_bytes.len());
            combined.extend_from_slice(&SYNC_WORD);
            combined.extend_from_slice(&frame_bytes);

            out.extend_from_slice(&preamble);
            out.extend_from_slice(&self.phy.modulate_bytes(&combined));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use modem_core::phy::FskPhy;

    #[test]
    fn encodes_to_nonempty_samples() {
        let tx = Transmitter::new(FskPhy::audible(), false);
        let samples = tx.encode(b"hi");
        assert!(samples.len() > 1000, "got {}", samples.len());
    }

    #[test]
    fn multi_frame_payload_splits() {
        let tx = Transmitter::new(FskPhy::audible(), false);
        // 500 bytes of payload + 32 bytes sha = 532 → 3 frames (219+219+94)
        let payload = vec![0xAA; 500];
        let samples = tx.encode(&payload);
        // 3 preambles → at minimum 3 * 80 ms = 240 ms of preamble alone = 11520 samples
        assert!(samples.len() > 11_500);
    }
}
