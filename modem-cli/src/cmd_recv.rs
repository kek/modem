use crate::output_format::{is_printable, print_hex};
use crate::reporter::{Reporter, RxEvent};
use crate::{make_phy, Profile};
use modem_audio::input::open_mic;
use modem_codec::rx::{FrameEvent, Receiver};
use modem_core::fsk::DspVariants;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub fn run(
    profile: Profile,
    variants: DspVariants,
    output: Option<PathBuf>,
    hex: bool,
    reporter: &mut dyn Reporter,
) -> anyhow::Result<()> {
    let phy = make_phy(profile, variants);
    let mut rx = Receiver::new(phy);
    let mic = open_mic()?;

    reporter.on_rx(RxEvent::Listening);

    let mut last_progress = Instant::now();
    let mut started = false;
    loop {
        let chunk = match mic.rx.recv_timeout(Duration::from_millis(500)) {
            Ok(c) => c,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if started && last_progress.elapsed() > Duration::from_secs(10) {
                    eprintln!("⚠ no progress for 10s, giving up");
                    return Ok(());
                }
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        reporter.on_rx(RxEvent::Chunk(&chunk));
        let events = rx.push_samples(&chunk);
        for ev in events {
            reporter.on_rx(RxEvent::Frame(&ev));
            match ev {
                FrameEvent::FrameOk { .. } => {
                    started = true;
                    last_progress = Instant::now();
                }
                FrameEvent::FrameDropped { .. } => {
                    started = true;
                    last_progress = Instant::now();
                }
                FrameEvent::StreamComplete { bytes, sha256_ok: _ } => {
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
