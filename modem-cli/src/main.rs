mod cmd_chat;
mod cmd_rx_wav;
mod cmd_send;
mod cmd_recv;
mod cmd_tx_wav;
mod cmd_rank;
mod output_format;
mod reporter;
mod tui;

use clap::{Args, Parser, Subcommand, ValueEnum};
use modem_core::fsk::DspVariants;
use std::io::IsTerminal;
use std::path::PathBuf;

use crate::reporter::PlainReporter;
use crate::tui::TuiReporter;

#[derive(Parser)]
#[command(name = "modem", version, about = "Acoustic modem CLI")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Transmit live over speaker.
    Send {
        #[arg(long, value_enum, default_value_t = Profile::Audible)]
        profile: Profile,
        #[command(flatten)]
        variants: VariantArgs,
        /// Force the ratatui dashboard even when stdout isn't a TTY.
        #[arg(long, conflicts_with = "plain")]
        tui: bool,
        /// Force the plain text reporter (skip the dashboard).
        #[arg(long, conflicts_with = "tui")]
        plain: bool,
        /// Input file; if omitted, reads stdin.
        input: Option<PathBuf>,
    },
    /// Receive live from microphone.
    Recv {
        #[arg(long, value_enum, default_value_t = Profile::Audible)]
        profile: Profile,
        #[command(flatten)]
        variants: VariantArgs,
        /// Output file; if omitted, writes to stdout.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Print as hex dump instead of raw bytes.
        #[arg(long)]
        hex: bool,
        /// Force the ratatui dashboard even when stdout isn't a TTY.
        #[arg(long, conflicts_with = "plain")]
        tui: bool,
        /// Force the plain text reporter (skip the dashboard).
        #[arg(long, conflicts_with = "tui")]
        plain: bool,
    },
    /// Offline: encode bytes to a WAV file.
    TxWav {
        #[arg(long, value_enum, default_value_t = Profile::Audible)]
        profile: Profile,
        #[command(flatten)]
        variants: VariantArgs,
        input: PathBuf,
        output: PathBuf,
    },
    /// Offline: decode a WAV file back to bytes.
    RxWav {
        #[arg(long, value_enum, default_value_t = Profile::Audible)]
        profile: Profile,
        #[command(flatten)]
        variants: VariantArgs,
        input: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long)]
        hex: bool,
    },
    /// Sweep DSP variant combinations across a capture corpus and emit
    /// one structured outcome line per (capture, variant). Intended to be
    /// driven by an agent: pipe to grep/jq, no human-eyeballing.
    Rank {
        /// Directory containing capture WAVs and a `manifest.toml`
        /// describing what each capture should decode to.
        corpus: PathBuf,
    },
    /// Interactive: type a line, press Enter, transmit it; repeat.
    Chat {
        #[arg(long, value_enum, default_value_t = Profile::Audible)]
        profile: Profile,
        #[command(flatten)]
        variants: VariantArgs,
    },
}

/// DSP variant toggles. All default to OFF (trunk baseline). The `rank`
/// subcommand ignores these and sweeps all 8 combinations internally.
#[derive(Args, Clone, Copy, Default)]
pub struct VariantArgs {
    /// TX-side raised-cosine pulse shaping.
    #[arg(long)]
    pub pulse_shape: bool,
    /// RX-side Tukey-windowed matched filter.
    #[arg(long)]
    pub matched_filter: bool,
    /// RX-side early-late symbol-timing recovery.
    #[arg(long)]
    pub timing_recovery: bool,
}

impl From<VariantArgs> for DspVariants {
    fn from(a: VariantArgs) -> Self {
        DspVariants {
            pulse_shape: a.pulse_shape,
            matched_filter: a.matched_filter,
            timing_recovery: a.timing_recovery,
        }
    }
}

#[derive(Copy, Clone, ValueEnum)]
pub enum Profile { Audible, Ultrasonic }

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Send { profile, variants, input, tui, plain } => {
            let want_tui = tui || (!plain && std::io::stdout().is_terminal());
            if want_tui {
                let mut r = TuiReporter::new_tx(profile)?;
                cmd_send::run(profile, variants.into(), input, &mut r)
            } else {
                let mut r = PlainReporter;
                cmd_send::run(profile, variants.into(), input, &mut r)
            }
        }
        Cmd::Recv { profile, variants, output, hex, tui, plain } => {
            let want_tui = tui || (!plain && std::io::stdout().is_terminal());
            if want_tui {
                let mut r = TuiReporter::new_rx(profile)?;
                cmd_recv::run(profile, variants.into(), output, hex, &mut r)
            } else {
                let mut r = PlainReporter;
                cmd_recv::run(profile, variants.into(), output, hex, &mut r)
            }
        }
        Cmd::TxWav { profile, variants, input, output } => cmd_tx_wav::run(profile, variants.into(), input, output),
        Cmd::RxWav { profile, variants, input, output, hex } => cmd_rx_wav::run(profile, variants.into(), input, output, hex),
        Cmd::Rank { corpus } => cmd_rank::run(corpus),
        Cmd::Chat { profile, variants } => cmd_chat::run(profile, variants.into()),
    }
}

pub fn make_phy(profile: Profile, variants: DspVariants) -> modem_core::phy::FskPhy {
    match profile {
        Profile::Audible => modem_core::phy::FskPhy::audible_with(variants),
        Profile::Ultrasonic => modem_core::phy::FskPhy::ultrasonic_with(variants),
    }
}
