//! Framing: header + payload + RS(255,223) parity + CRC32.

use crate::crc::crc32;
use crate::rs::{rs_decode, rs_encode, RS_DATA, RS_PARITY};

pub const HEADER_LEN: usize = 4;
pub const MAX_PAYLOAD: usize = RS_DATA - HEADER_LEN; // 219
pub const FRAME_LEN: usize = RS_DATA + RS_PARITY + 4; // 255 + 4 CRC = 259 bytes

pub const VERSION: u8 = 0x01;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    pub first: bool,
    pub last: bool,
    pub ultrasonic: bool,
    pub payload_len: u16,
}

impl FrameHeader {
    pub fn flags_byte(&self) -> u8 {
        (self.first as u8)
            | ((self.last as u8) << 1)
            | ((self.ultrasonic as u8) << 2)
    }
    pub fn from_bytes(b: &[u8; HEADER_LEN]) -> Result<Self, FrameError> {
        if b[0] != VERSION {
            return Err(FrameError::BadVersion(b[0]));
        }
        let flags = b[1];
        // Bit 0=first, 1=last, 2=ultrasonic; bits 3-7 are reserved and must be zero.
        if flags & !0b111 != 0 {
            return Err(FrameError::ReservedFlagsSet(flags));
        }
        let payload_len = u16::from_be_bytes([b[2], b[3]]);
        if payload_len as usize > MAX_PAYLOAD {
            return Err(FrameError::PayloadTooLong(payload_len));
        }
        Ok(Self {
            first: flags & 0b001 != 0,
            last:  flags & 0b010 != 0,
            ultrasonic: flags & 0b100 != 0,
            payload_len,
        })
    }
    pub fn to_bytes(&self) -> [u8; HEADER_LEN] {
        let mut h = [0u8; HEADER_LEN];
        h[0] = VERSION;
        h[1] = self.flags_byte();
        let len_be = self.payload_len.to_be_bytes();
        h[2] = len_be[0];
        h[3] = len_be[1];
        h
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("payload too long: {0} > {MAX_PAYLOAD}")]
    PayloadTooLong(u16),
    #[error("header says payload_len={header_says} but got {got} bytes")]
    HeaderPayloadLenMismatch { header_says: u16, got: usize },
    #[error("bad version byte: 0x{0:02x}")]
    BadVersion(u8),
    #[error("reserved flag bits set: 0b{0:08b}")]
    ReservedFlagsSet(u8),
    #[error("frame length wrong: got {0}, expected {FRAME_LEN}")]
    BadLength(usize),
    #[error("RS uncorrectable")]
    RsUncorrectable,
    #[error("CRC mismatch")]
    CrcMismatch,
}

/// Encode a frame into FRAME_LEN bytes.
pub fn encode_frame(header: FrameHeader, payload: &[u8]) -> Result<[u8; FRAME_LEN], FrameError> {
    if payload.len() != header.payload_len as usize {
        return Err(FrameError::HeaderPayloadLenMismatch {
            header_says: header.payload_len,
            got: payload.len(),
        });
    }
    if payload.len() > MAX_PAYLOAD {
        return Err(FrameError::PayloadTooLong(payload.len() as u16));
    }

    // Build the 223-byte RS data block: header + payload, right-padded with zeros.
    let mut data = [0u8; RS_DATA];
    data[..HEADER_LEN].copy_from_slice(&header.to_bytes());
    data[HEADER_LEN..HEADER_LEN + payload.len()].copy_from_slice(payload);

    let codeword = rs_encode(&data);

    let mut out = [0u8; FRAME_LEN];
    out[..codeword.len()].copy_from_slice(&codeword);

    // CRC over header + payload (the *application-visible* data, not the padding).
    let crc = crc32(&data[..HEADER_LEN + payload.len()]);
    out[codeword.len()..].copy_from_slice(&crc.to_be_bytes());
    Ok(out)
}

/// Decode a frame's bytes back to (header, payload). Returns the longest sensible payload.
pub fn decode_frame(bytes: &[u8]) -> Result<(FrameHeader, Vec<u8>), FrameError> {
    if bytes.len() != FRAME_LEN {
        return Err(FrameError::BadLength(bytes.len()));
    }
    let mut cw = [0u8; RS_DATA + RS_PARITY];
    cw.copy_from_slice(&bytes[..RS_DATA + RS_PARITY]);

    let data = rs_decode(&cw).map_err(|_| FrameError::RsUncorrectable)?;

    let header_bytes: [u8; HEADER_LEN] = data[..HEADER_LEN]
        .try_into()
        .expect("FRAME_LEN was already checked above");
    let header = FrameHeader::from_bytes(&header_bytes)?;
    let payload_end = HEADER_LEN + header.payload_len as usize;
    let payload = data[HEADER_LEN..payload_end].to_vec();

    // Verify CRC
    let crc_bytes: [u8; 4] = bytes[RS_DATA + RS_PARITY..]
        .try_into()
        .expect("FRAME_LEN was already checked above");
    let expected = u32::from_be_bytes(crc_bytes);
    let actual = crc32(&data[..payload_end]);
    if expected != actual {
        return Err(FrameError::CrcMismatch);
    }

    Ok((header, payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(first: bool, last: bool, len: usize) -> FrameHeader {
        FrameHeader { first, last, ultrasonic: false, payload_len: len as u16 }
    }

    #[test]
    fn empty_payload_roundtrip() {
        let header = h(true, true, 0);
        let bytes = encode_frame(header, &[]).unwrap();
        let (h2, p) = decode_frame(&bytes).unwrap();
        assert_eq!(h2, header);
        assert_eq!(p, Vec::<u8>::new());
    }

    #[test]
    fn max_payload_roundtrip() {
        let payload: Vec<u8> = (0..MAX_PAYLOAD).map(|i| i as u8).collect();
        let header = h(true, true, MAX_PAYLOAD);
        let bytes = encode_frame(header, &payload).unwrap();
        let (h2, p) = decode_frame(&bytes).unwrap();
        assert_eq!(h2, header);
        assert_eq!(p, payload);
    }

    #[test]
    fn corrects_byte_errors() {
        let payload: Vec<u8> = b"hello, world! this is a test payload.".to_vec();
        let header = h(true, true, payload.len());
        let mut bytes = encode_frame(header, &payload).unwrap();
        // Corrupt 10 bytes scattered across the frame
        for i in [5, 22, 50, 80, 110, 140, 170, 200, 230, 240] {
            bytes[i] ^= 0x55;
        }
        let (_, p) = decode_frame(&bytes).unwrap();
        assert_eq!(p, payload);
    }

    #[test]
    fn rejects_oversize_payload() {
        let payload = vec![0u8; MAX_PAYLOAD + 1];
        let header = h(true, true, payload.len());
        assert!(encode_frame(header, &payload).is_err());
    }

    #[test]
    fn rs_failure_reported_as_rs_not_crc() {
        // 17+ errors → RS gives up. We treat as RsUncorrectable, not CRC.
        let payload = vec![0xAA; 100];
        let header = h(true, true, payload.len());
        let mut bytes = encode_frame(header, &payload).unwrap();
        for i in 0..20 {
            bytes[i] ^= 0xFF;
        }
        assert!(matches!(decode_frame(&bytes), Err(FrameError::RsUncorrectable)));
    }

    #[test]
    fn rejects_reserved_flag_bits() {
        let mut bytes = encode_frame(
            FrameHeader { first: true, last: true, ultrasonic: false, payload_len: 4 },
            b"abcd",
        ).unwrap();
        // Forge a flag bit
        bytes[1] |= 0b1000;
        // Now CRC and RS won't match… RS may auto-correct it. To exercise the
        // header check cleanly, decode the (mutated) header bytes directly:
        let header_bytes: [u8; HEADER_LEN] = [VERSION, 0b0000_1011, 0, 4];
        assert!(matches!(FrameHeader::from_bytes(&header_bytes), Err(FrameError::ReservedFlagsSet(_))));
    }
}
