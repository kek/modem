//! Shared printable-bytes detection and hex dump.

use std::io::Write;

pub fn is_printable(b: &[u8]) -> bool {
    std::str::from_utf8(b).is_ok_and(|s| {
        s.chars().all(|c| !c.is_control() || c == '\t' || c == '\n' || c == '\r')
    })
}

pub fn print_hex(b: &[u8]) {
    for (i, chunk) in b.chunks(16).enumerate() {
        let hex: String = chunk.iter().map(|x| format!("{x:02x} ")).collect();
        let ascii: String = chunk
            .iter()
            .map(|&x| if x.is_ascii_graphic() || x == b' ' { x as char } else { '.' })
            .collect();
        println!("{:08x}  {:<48}  {}", i * 16, hex, ascii);
    }
}

pub fn write_bytes_to_stdout(b: &[u8]) -> anyhow::Result<()> {
    std::io::stdout().write_all(b)?;
    Ok(())
}
