use crate::reporter::{PlainReporter, Reporter, TxEvent};
use crate::{make_phy, Profile};
use modem_codec::tx::Transmitter;
use modem_core::fsk::DspVariants;
use std::io::{self, BufRead, Write};

pub fn run(profile: Profile, variants: DspVariants) -> anyhow::Result<()> {
    let phy = make_phy(profile, variants);
    let ultrasonic = matches!(profile, Profile::Ultrasonic);
    let tx = Transmitter::new(phy, ultrasonic);

    let profile_tag = if ultrasonic { "ultrasonic" } else { "audible" };
    eprintln!("interactive chat on {profile_tag} · empty line or Ctrl-D to exit");

    let stdin = io::stdin();
    let mut stdin = stdin.lock();
    let mut reporter = PlainReporter;
    let mut buf = String::new();
    loop {
        eprint!("> ");
        io::stderr().flush()?;
        buf.clear();
        let n = stdin.read_line(&mut buf)?;
        if n == 0 {
            // EOF (Ctrl-D).
            eprintln!();
            return Ok(());
        }
        let line = buf.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            return Ok(());
        }
        let bytes = line.as_bytes();
        let samples = tx.encode(bytes);
        let seconds = samples.len() as f32 / 48_000.0;
        reporter.on_tx(TxEvent::Starting {
            bytes: bytes.len(),
            samples: samples.len(),
            seconds,
        });
        modem_audio::output::play_samples(samples)?;
        reporter.on_tx(TxEvent::Done);
    }
}
