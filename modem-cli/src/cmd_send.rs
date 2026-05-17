use crate::{make_phy, Profile};
use modem_codec::tx::Transmitter;
use modem_core::fsk::DspVariants;
use std::fs;
use std::io::Read;
use std::path::PathBuf;

pub fn run(profile: Profile, variants: DspVariants, input: Option<PathBuf>) -> anyhow::Result<()> {
    let bytes = match input {
        Some(p) => fs::read(&p)?,
        None => {
            let mut buf = Vec::new();
            std::io::stdin().read_to_end(&mut buf)?;
            buf
        }
    };
    let phy = make_phy(profile, variants);
    let ultrasonic = matches!(profile, Profile::Ultrasonic);
    let tx = Transmitter::new(phy, ultrasonic);
    let samples = tx.encode(&bytes);
    eprintln!("→ {} bytes, {} samples ({:.1}s) over speaker",
        bytes.len(), samples.len(), samples.len() as f32 / 48_000.0);
    modem_audio::output::play_samples(samples)?;
    eprintln!("✓ done");
    Ok(())
}
