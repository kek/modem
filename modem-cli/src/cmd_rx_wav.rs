use crate::{make_phy, Profile};
use hound::WavReader;
use modem_codec::rx::{FrameEvent, Receiver};
use std::fs;
use std::io::Write;
use std::path::PathBuf;

pub fn run(profile: Profile, input: PathBuf, output: Option<PathBuf>, hex: bool) -> anyhow::Result<()> {
    let mut reader = WavReader::open(&input)?;
    let samples: Vec<f32> = match reader.spec().sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().filter_map(|s| s.ok()).collect(),
        hound::SampleFormat::Int => reader.samples::<i32>()
            .filter_map(|s| s.ok())
            .map(|s| s as f32 / i32::MAX as f32)
            .collect(),
    };

    let phy = make_phy(profile);
    let mut rx = Receiver::new(phy);
    let events = rx.push_samples(&samples);

    let bytes: Vec<u8> = events.into_iter().find_map(|e| match e {
        FrameEvent::StreamComplete { bytes, .. } => Some(bytes),
        _ => None,
    }).ok_or_else(|| anyhow::anyhow!("no complete stream received"))?;

    if let Some(p) = output {
        fs::write(&p, &bytes)?;
        eprintln!("wrote {} bytes to {}", bytes.len(), p.display());
    } else if hex || !is_printable(&bytes) {
        print_hex(&bytes);
    } else {
        std::io::stdout().write_all(&bytes)?;
    }
    Ok(())
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
