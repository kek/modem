use crate::reporter::{Reporter, TxEvent};
use crate::{make_phy, Profile};
use modem_codec::tx::Transmitter;
use modem_core::fsk::DspVariants;
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

pub fn run(
    profile: Profile,
    variants: DspVariants,
    input: Option<PathBuf>,
    reporter: &mut dyn Reporter,
) -> anyhow::Result<()> {
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
    let total_sec = samples.len() as f32 / 48_000.0;
    reporter.on_tx(TxEvent::Starting {
        bytes: bytes.len(),
        samples: samples.len(),
        seconds: total_sec,
    });

    // Hand the samples to a background playback thread; tick the reporter
    // from this (main) thread at 50 ms cadence so the TUI stays interactive
    // while AudioTrack/CPAL blocks.
    let samples_for_play = samples.clone();
    let (done_tx, done_rx) = mpsc::channel::<anyhow::Result<()>>();
    let play_handle = thread::spawn(move || {
        let r = modem_audio::output::play_samples(samples_for_play)
            .map_err(anyhow::Error::from);
        let _ = done_tx.send(r);
    });

    let start = Instant::now();
    let chunk_samples = 2400; // 50 ms at 48 kHz
    let mut i = 0usize;
    let mut play_result: Option<anyhow::Result<()>> = None;
    loop {
        if let Ok(r) = done_rx.try_recv() {
            play_result = Some(r);
            break;
        }
        let end = (i + chunk_samples).min(samples.len());
        if end > i {
            reporter.on_tx(TxEvent::Chunk(&samples[i..end]));
            i = end;
        }
        if let Some(t) = reporter.as_tx_progress_sink() {
            t.set_tx_progress(start.elapsed().as_secs_f32().min(total_sec));
        }
        thread::sleep(Duration::from_millis(50));
    }
    let _ = play_handle.join();
    if let Some(r) = play_result {
        r?;
    }
    reporter.on_tx(TxEvent::Done);
    Ok(())
}
