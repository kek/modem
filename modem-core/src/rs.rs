//! Reed–Solomon RS(255,223): protects 223 bytes of data with 32 parity bytes,
//! corrects up to 16 byte errors per codeword.

use reed_solomon::{Encoder, Decoder};

pub const RS_DATA: usize = 223;
pub const RS_PARITY: usize = 32;
pub const RS_TOTAL: usize = 255;

/// Encode exactly RS_DATA bytes of data into a RS_TOTAL-byte codeword
/// (data followed by 32 parity bytes).
pub fn rs_encode(data: &[u8; RS_DATA]) -> [u8; RS_TOTAL] {
    let enc = Encoder::new(RS_PARITY);
    let buf = enc.encode(data);
    let mut out = [0u8; RS_TOTAL];
    out.copy_from_slice(&buf[..]);
    out
}

#[derive(Debug, thiserror::Error)]
pub enum RsError {
    #[error("too many errors to correct: {0}")]
    Uncorrectable(String),
}

/// Decode a 255-byte codeword, correcting up to 16 byte errors.
/// Returns the original 223 data bytes.
pub fn rs_decode(codeword: &[u8; RS_TOTAL]) -> Result<[u8; RS_DATA], RsError> {
    let dec = Decoder::new(RS_PARITY);
    let recovered = dec
        .correct(codeword, None)
        .map_err(|e| RsError::Uncorrectable(format!("{e:?}")))?;
    let mut out = [0u8; RS_DATA];
    out.copy_from_slice(recovered.data());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_data() -> [u8; RS_DATA] {
        let mut d = [0u8; RS_DATA];
        for (i, b) in d.iter_mut().enumerate() { *b = (i as u8).wrapping_mul(37).wrapping_add(11); }
        d
    }

    #[test]
    fn clean_roundtrip() {
        let data = sample_data();
        let cw = rs_encode(&data);
        let back = rs_decode(&cw).unwrap();
        assert_eq!(back, data);
    }

    #[test]
    fn corrects_16_byte_errors() {
        let data = sample_data();
        let mut cw = rs_encode(&data);
        // Flip 16 arbitrary bytes
        for &i in &[3, 17, 42, 60, 77, 100, 128, 150, 170, 190, 200, 210, 220, 230, 240, 250] {
            cw[i] ^= 0xFF;
        }
        let back = rs_decode(&cw).unwrap();
        assert_eq!(back, data, "RS must correct up to 16 byte errors");
    }

    #[test]
    fn fails_above_16_byte_errors() {
        let data = sample_data();
        let mut cw = rs_encode(&data);
        for i in 0..17 {
            cw[i * 10] ^= 0xAA;
        }
        assert!(rs_decode(&cw).is_err(), "17 errors must be uncorrectable");
    }
}
