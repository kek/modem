/// CRC-32 (IEEE 802.3 polynomial), as used in PNG, Ethernet, zlib.
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut h = crc32fast::Hasher::new();
    h.update(bytes);
    h.finalize()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors() {
        // Standard test vectors for CRC-32/ISO-HDLC
        assert_eq!(crc32(b""), 0x00000000);
        assert_eq!(crc32(b"123456789"), 0xCBF43926);
        assert_eq!(crc32(b"hello world"), 0x0D4A1185);
    }
}
