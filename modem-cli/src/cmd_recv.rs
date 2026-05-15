use crate::{make_phy, Profile};
use modem_audio::input::open_mic;
use modem_codec::rx::{FrameEvent, Receiver};
use std::fs;
use std::io::Write;
use std::path::PathBuf;

pub fn run(profile: Profile, output: Option<PathBuf>, hex: bool) -> anyhow::Result<()> {
    let phy = make_phy(profile);
    let mut rx = Receiver::new(phy);
    let mic = open_mic()?;

    eprintln!("listening… (Ctrl-C to stop)");

    loop {
        let chunk = mic.rx.recv()?;
        let events = rx.push_samples(&chunk);
        for ev in events {
            match ev {
                FrameEvent::FrameOk { seq, bytes } => {
                    eprintln!("  frame {seq} ok ({} bytes)", bytes.len());
                }
                FrameEvent::FrameDropped { seq, reason } => {
                    eprintln!("  frame {seq} dropped: {reason}");
                }
                FrameEvent::StreamComplete { bytes, sha256_ok } => {
                    eprintln!("✓ {} bytes received, sha256 {}", bytes.len(),
                        if sha256_ok { "ok" } else { "MISMATCH" });
                    if let Some(p) = &output {
                        fs::write(p, &bytes)?;
                        eprintln!("  wrote {}", p.display());
                    } else if hex || !is_printable(&bytes) {
                        print_hex(&bytes);
                    } else {
                        std::io::stdout().write_all(&bytes)?;
                        std::io::stdout().write_all(b"\n")?;
                    }
                    return Ok(());
                }
            }
        }
    }
}

fn is_printable(b: &[u8]) -> bool {
    std::str::from_utf8(b).map_or(false, |s| s.chars().all(|c|
        !c.is_control() || c == '\t' || c == '\n' || c == '\r'
    ))
}

fn print_hex(b: &[u8]) {
    for (i, chunk) in b.chunks(16).enumerate() {
        let hex: String = chunk.iter().map(|x| format!("{:02x} ", x)).collect();
        let ascii: String = chunk.iter()
            .map(|&x| if x.is_ascii_graphic() || x == b' ' { x as char } else { '.' })
            .collect();
        println!("{:08x}  {:<48}  {}", i * 16, hex, ascii);
    }
}
