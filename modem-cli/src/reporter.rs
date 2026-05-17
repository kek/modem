//! Pluggable reporter for the live `recv`/`send` commands. The default
//! implementation prints plain text (the historical behaviour, used by
//! scripts and tests). The TUI implementation in `tui.rs` renders a live
//! ratatui dashboard instead.

use modem_codec::rx::FrameEvent;

/// Receive-side events the runner emits.
pub enum RxEvent<'a> {
    Listening,
    Chunk(&'a [f32]),
    Frame(&'a FrameEvent),
    /// Seconds remaining on the no-progress watchdog (0 = giving up).
    Watchdog { seconds_left: u32 },
}

/// Send-side events the runner emits.
pub enum TxEvent<'a> {
    Starting { bytes: usize, samples: usize, seconds: f32 },
    Chunk(&'a [f32]),
    Done,
}

pub trait Reporter {
    fn on_rx(&mut self, _e: RxEvent<'_>) {}
    fn on_tx(&mut self, _e: TxEvent<'_>) {}

    /// Returns a sink that can be told elapsed playback time, when the
    /// reporter supports it (only the TUI variant does).
    fn as_tx_progress_sink(&mut self) -> Option<&mut dyn TxProgressSink> { None }

    /// Did the user ask to quit? Plain reporters always return false (the
    /// process exits via Ctrl-C). The TUI reporter polls the keyboard.
    fn should_quit(&mut self) -> bool { false }
}

pub trait TxProgressSink {
    fn set_tx_progress(&mut self, elapsed_sec: f32);
}

/// Drop-in for the historical eprintln-based output.
pub struct PlainReporter;

impl Reporter for PlainReporter {
    fn on_rx(&mut self, e: RxEvent<'_>) {
        match e {
            RxEvent::Listening => eprintln!("listening… (Ctrl-C to stop)"),
            RxEvent::Chunk(_) => {}
            RxEvent::Frame(FrameEvent::FrameOk { seq, bytes }) => {
                eprintln!("  frame {seq} ok ({} bytes)", bytes.len());
            }
            RxEvent::Frame(FrameEvent::FrameDropped { seq, reason }) => {
                eprintln!("  frame {seq} dropped: {reason}");
            }
            RxEvent::Frame(FrameEvent::StreamComplete { bytes, sha256_ok }) => {
                eprintln!(
                    "✓ {} bytes received, sha256 {}",
                    bytes.len(),
                    if *sha256_ok { "ok" } else { "MISMATCH" },
                );
            }
            RxEvent::Watchdog { seconds_left } => {
                if seconds_left == 0 {
                    eprintln!("⚠ no progress for 10s, giving up");
                }
            }
        }
    }
    fn on_tx(&mut self, e: TxEvent<'_>) {
        match e {
            TxEvent::Starting { bytes, samples, seconds } => {
                eprintln!(
                    "→ {} bytes, {} samples ({:.1}s) over speaker",
                    bytes, samples, seconds,
                );
            }
            TxEvent::Chunk(_) => {}
            TxEvent::Done => eprintln!("✓ done"),
        }
    }
}
