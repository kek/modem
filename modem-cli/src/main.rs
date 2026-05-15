mod cmd_rx_wav;
mod cmd_send;
mod cmd_recv;
mod cmd_tx_wav;
mod output_format;

use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

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
        /// Input file; if omitted, reads stdin.
        input: Option<PathBuf>,
    },
    /// Receive live from microphone.
    Recv {
        #[arg(long, value_enum, default_value_t = Profile::Audible)]
        profile: Profile,
        /// Output file; if omitted, writes to stdout.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Print as hex dump instead of raw bytes.
        #[arg(long)]
        hex: bool,
    },
    /// Offline: encode bytes to a WAV file.
    TxWav {
        #[arg(long, value_enum, default_value_t = Profile::Audible)]
        profile: Profile,
        input: PathBuf,
        output: PathBuf,
    },
    /// Offline: decode a WAV file back to bytes.
    RxWav {
        #[arg(long, value_enum, default_value_t = Profile::Audible)]
        profile: Profile,
        input: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long)]
        hex: bool,
    },
}

#[derive(Copy, Clone, ValueEnum)]
pub enum Profile { Audible, Ultrasonic }

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Send { profile, input } => cmd_send::run(profile, input),
        Cmd::Recv { profile, output, hex } => cmd_recv::run(profile, output, hex),
        Cmd::TxWav { profile, input, output } => cmd_tx_wav::run(profile, input, output),
        Cmd::RxWav { profile, input, output, hex } => cmd_rx_wav::run(profile, input, output, hex),
    }
}

pub fn make_phy(profile: Profile) -> modem_core::phy::FskPhy {
    match profile {
        Profile::Audible => modem_core::phy::FskPhy::audible(),
        Profile::Ultrasonic => modem_core::phy::FskPhy::ultrasonic(),
    }
}
