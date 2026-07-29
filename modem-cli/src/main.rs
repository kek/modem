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
use modem_core::fsk::{DspVariants, DEFAULT_SYMBOL_RATE};
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
        #[command(flatten)]
        phy: PhyArgs,
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
        #[command(flatten)]
        phy: PhyArgs,
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
        #[command(flatten)]
        phy: PhyArgs,
        input: PathBuf,
        output: PathBuf,
    },
    /// Offline: decode a WAV file back to bytes.
    RxWav {
        #[command(flatten)]
        phy: PhyArgs,
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
        #[command(flatten)]
        phy: PhyArgs,
    },
}

/// Which PHY a command should build. Every subcommand that touches the modem
/// takes this same block, so `--symbol-rate` means the same thing to live audio
/// as it does to the offline WAV pair.
#[derive(Args, Clone, Copy)]
pub struct PhyArgs {
    #[arg(long, value_enum, default_value_t = Profile::Audible)]
    pub profile: Profile,
    /// Symbols per second. Must divide 48000. Halving to 25 buys margin
    /// against room reverberation at half the bitrate — see
    /// docs/android-smoke-test.md. The rate is not part of the frame format,
    /// so both ends must be told the same one; a receiver at the wrong rate
    /// still locks onto the preamble and then decodes noise.
    #[arg(long, default_value_t = DEFAULT_SYMBOL_RATE)]
    pub symbol_rate: u32,
    #[command(flatten)]
    pub variants: VariantArgs,
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
        Cmd::Send { phy, input, tui, plain } => {
            let want_tui = tui || (!plain && std::io::stdout().is_terminal());
            if want_tui {
                let mut r = TuiReporter::new_tx(phy.profile)?;
                cmd_send::run(phy.into(), input, &mut r)
            } else {
                let mut r = PlainReporter;
                cmd_send::run(phy.into(), input, &mut r)
            }
        }
        Cmd::Recv { phy, output, hex, tui, plain } => {
            let want_tui = tui || (!plain && std::io::stdout().is_terminal());
            if want_tui {
                let mut r = TuiReporter::new_rx(phy.profile)?;
                cmd_recv::run(phy.into(), output, hex, &mut r)
            } else {
                let mut r = PlainReporter;
                cmd_recv::run(phy.into(), output, hex, &mut r)
            }
        }
        Cmd::TxWav { phy, input, output } => cmd_tx_wav::run(phy.into(), input, output),
        Cmd::RxWav { phy, input, output, hex } => cmd_rx_wav::run(phy.into(), input, output, hex),
        Cmd::Rank { corpus } => cmd_rank::run(corpus),
        Cmd::Chat { phy } => cmd_chat::run(phy.into()),
    }
}

/// Everything needed to build the PHY a command runs on, travelling as one
/// value. There is deliberately no rate-less constructor: a command cannot be
/// handed a symbol rate and then quietly build its PHY at the default, which
/// is the bug that kept live audio pinned to 50 sym/s.
#[derive(Clone, Copy)]
pub struct PhySpec {
    pub profile: Profile,
    /// Symbols per second. Must divide 48 kHz exactly; `modem_core` panics
    /// otherwise rather than misreport the rate it runs at.
    pub symbol_rate: u32,
    pub variants: DspVariants,
}

impl From<PhyArgs> for PhySpec {
    fn from(a: PhyArgs) -> Self {
        PhySpec {
            profile: a.profile,
            symbol_rate: a.symbol_rate,
            variants: a.variants.into(),
        }
    }
}

impl PhySpec {
    pub fn phy(&self) -> modem_core::phy::FskPhy {
        match self.profile {
            Profile::Audible => {
                modem_core::phy::FskPhy::audible_at(self.symbol_rate, self.variants)
            }
            Profile::Ultrasonic => {
                modem_core::phy::FskPhy::ultrasonic_at(self.symbol_rate, self.variants)
            }
        }
    }

    /// The config of the PHY this spec builds — tone frequencies, sample rate,
    /// symbol length. Read off a real PHY so it cannot drift from `phy()`.
    pub fn config(&self) -> modem_core::fsk::FskConfig {
        *self.phy().config()
    }

    pub fn ultrasonic(&self) -> bool {
        matches!(self.profile, Profile::Ultrasonic)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec_from(args: &[&str]) -> PhySpec {
        let cli = Cli::try_parse_from(args).expect("argv should parse");
        match cli.cmd {
            Cmd::Send { phy, .. }
            | Cmd::Recv { phy, .. }
            | Cmd::TxWav { phy, .. }
            | Cmd::RxWav { phy, .. }
            | Cmd::Chat { phy } => phy.into(),
            Cmd::Rank { .. } => panic!("rank builds a PHY per capture, not from argv"),
        }
    }

    /// One argv per subcommand that reaches the modem, with the positional
    /// arguments each one requires.
    fn argv_for(cmd: &str) -> Vec<&str> {
        match cmd {
            "tx-wav" => vec!["modem", "tx-wav", "in.bin", "out.wav"],
            "rx-wav" => vec!["modem", "rx-wav", "in.wav"],
            _ => vec!["modem", cmd],
        }
    }

    const PHY_COMMANDS: [&str; 5] = ["send", "recv", "chat", "tx-wav", "rx-wav"];

    /// `--symbol-rate` must reach the PHY on every command, live audio
    /// included. Asserted through the built PHY's symbol length, not through
    /// the parsed field: a rate that is accepted and then ignored would leave
    /// the field set to 25 and the symbols 960 samples long.
    #[test]
    fn every_command_passes_the_symbol_rate_to_the_phy() {
        for cmd in PHY_COMMANDS {
            let mut argv = argv_for(cmd);
            argv.extend(["--symbol-rate", "25"]);
            let spec = spec_from(&argv);
            assert_eq!(spec.symbol_rate, 25, "{cmd}");
            assert_eq!(spec.config().symbol_rate(), 25, "{cmd} built the wrong PHY");
            assert_eq!(spec.config().symbol_samples, 1920, "{cmd}");
        }
    }

    #[test]
    fn every_command_defaults_to_the_default_symbol_rate() {
        for cmd in PHY_COMMANDS {
            let spec = spec_from(&argv_for(cmd));
            assert_eq!(spec.symbol_rate, DEFAULT_SYMBOL_RATE, "{cmd}");
            assert_eq!(spec.config().symbol_rate(), DEFAULT_SYMBOL_RATE, "{cmd}");
            assert_eq!(spec.config().symbol_samples, 960, "{cmd}");
        }
    }

    /// The rate is a timing change only: it must not disturb the profile's
    /// tones, and it must compose with the DSP variant flags.
    #[test]
    fn the_rate_is_orthogonal_to_profile_and_variants() {
        let slow = spec_from(&[
            "modem", "chat", "--profile", "ultrasonic", "--symbol-rate", "25",
            "--pulse-shape", "--matched-filter",
        ]);
        assert!(slow.ultrasonic());
        assert_eq!(slow.config().symbol_rate(), 25);
        assert_eq!(slow.config().tone_freqs[0], 17_500.0);
        assert!(slow.variants.pulse_shape && slow.variants.matched_filter);
        assert!(!slow.variants.timing_recovery);
    }
}
