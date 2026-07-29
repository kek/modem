use crate::{make_phy_at, Profile};
use hound::{SampleFormat, WavSpec, WavWriter};
use modem_codec::tx::Transmitter;
use modem_core::fsk::DspVariants;
use std::fs;
use std::path::PathBuf;

pub fn run(
    profile: Profile,
    symbol_rate: u32,
    variants: DspVariants,
    input: PathBuf,
    output: PathBuf,
) -> anyhow::Result<()> {
    let bytes = fs::read(&input)?;
    let phy = make_phy_at(profile, symbol_rate, variants);
    let ultrasonic = matches!(profile, Profile::Ultrasonic);
    let tx = Transmitter::new(phy, ultrasonic);
    let samples = tx.encode(&bytes);

    let spec = WavSpec {
        channels: 1,
        sample_rate: 48_000,
        bits_per_sample: 32,
        sample_format: SampleFormat::Float,
    };
    let mut w = WavWriter::create(&output, spec)?;
    let n = samples.len();
    for s in samples { w.write_sample(s)?; }
    w.finalize()?;
    eprintln!("wrote {} samples ({:.1}s) to {}", n, n as f32 / 48_000.0, output.display());
    Ok(())
}
