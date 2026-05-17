use crate::output_format::{is_printable, print_hex};
use crate::{make_phy, Profile};
use hound::WavReader;
use modem_codec::rx::{FrameEvent, Receiver};
use modem_core::fsk::DspVariants;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

pub fn run(profile: Profile, variants: DspVariants, input: PathBuf, output: Option<PathBuf>, hex: bool) -> anyhow::Result<()> {
    let mut reader = WavReader::open(&input)?;
    let samples: Vec<f32> = match reader.spec().sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().filter_map(|s| s.ok()).collect(),
        hound::SampleFormat::Int => reader.samples::<i32>()
            .filter_map(|s| s.ok())
            .map(|s| s as f32 / i32::MAX as f32)
            .collect(),
    };

    let phy = make_phy(profile, variants);
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

