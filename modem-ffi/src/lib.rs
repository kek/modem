//! UniFFI surface around modem-codec.

use std::sync::Mutex;

use modem_codec::rx::{FrameEvent as CoreFrameEvent, Receiver as CoreReceiver};
use modem_codec::tx::Transmitter as CoreTransmitter;
use modem_core::fsk::{DspVariants as CoreDspVariants, DEFAULT_SYMBOL_RATE};
use modem_core::phy::FskPhy;

uniffi::setup_scaffolding!();

/// Symbols per second the rate-less constructors use, so callers can offer a
/// rate control without hard-coding 50 on the other side of the binding.
#[uniffi::export]
pub fn default_symbol_rate() -> u32 {
    DEFAULT_SYMBOL_RATE
}

#[derive(uniffi::Enum)]
pub enum Profile {
    Audible,
    Ultrasonic,
}

/// DSP variant toggles exposed to the Android UI. All default to OFF
/// (legacy trunk behaviour). See modem_core::fsk::DspVariants for what
/// each one does.
#[derive(uniffi::Record, Default, Clone, Copy)]
pub struct DspVariants {
    pub pulse_shape: bool,
    pub matched_filter: bool,
    pub timing_recovery: bool,
}

impl From<DspVariants> for CoreDspVariants {
    fn from(v: DspVariants) -> Self {
        CoreDspVariants {
            pulse_shape: v.pulse_shape,
            matched_filter: v.matched_filter,
            timing_recovery: v.timing_recovery,
        }
    }
}

/// Build the PHY for `profile` at `symbol_rate`.
///
/// `symbol_rate` must divide 48 kHz exactly; `modem_core` panics otherwise
/// rather than run at a rate it would then misreport, and this surface does not
/// soften that — a rate that cannot exist is a caller bug, not a channel
/// condition. Both ends of a link must be built at the same rate: it is not
/// part of the frame format, so nothing on the wire announces it.
fn phy_for(profile: &Profile, symbol_rate: u32, variants: DspVariants) -> FskPhy {
    let v: CoreDspVariants = variants.into();
    match profile {
        Profile::Audible => FskPhy::audible_at(symbol_rate, v),
        Profile::Ultrasonic => FskPhy::ultrasonic_at(symbol_rate, v),
    }
}

#[derive(uniffi::Object)]
pub struct FfiTransmitter {
    inner: CoreTransmitter<FskPhy>,
}

#[uniffi::export]
impl FfiTransmitter {
    /// Transmit at the default symbol rate.
    #[uniffi::constructor]
    pub fn new(profile: Profile, variants: DspVariants) -> std::sync::Arc<Self> {
        Self::new_at(profile, DEFAULT_SYMBOL_RATE, variants)
    }

    /// Transmit at an explicit symbol rate, in symbols/second. Must divide
    /// 48000. Halving it to 25 buys margin against room reverberation at half
    /// the bitrate — see docs/android-smoke-test.md — and the receiver has to
    /// be told the same rate out of band.
    #[uniffi::constructor]
    pub fn new_at(profile: Profile, symbol_rate: u32, variants: DspVariants) -> std::sync::Arc<Self> {
        let ultrasonic = matches!(profile, Profile::Ultrasonic);
        std::sync::Arc::new(Self {
            inner: CoreTransmitter::new(phy_for(&profile, symbol_rate, variants), ultrasonic),
        })
    }

    /// Symbols per second this transmitter actually modulates at, read back
    /// off the PHY rather than from a remembered argument.
    pub fn symbol_rate(&self) -> u32 {
        self.inner.phy().config().symbol_rate()
    }

    /// Encode payload bytes into 48 kHz mono f32 audio samples.
    pub fn encode(&self, payload: Vec<u8>) -> Vec<f32> {
        self.inner.encode(&payload)
    }
}

#[derive(uniffi::Enum)]
pub enum FfiFrameEvent {
    FrameOk { seq: u32, bytes: Vec<u8> },
    FrameDropped { seq: u32, reason: String },
    StreamComplete { bytes: Vec<u8>, sha256_ok: bool },
}

fn convert(e: CoreFrameEvent) -> FfiFrameEvent {
    match e {
        CoreFrameEvent::FrameOk { seq, bytes } => FfiFrameEvent::FrameOk { seq, bytes },
        CoreFrameEvent::FrameDropped { seq, reason } => FfiFrameEvent::FrameDropped { seq, reason },
        CoreFrameEvent::StreamComplete { bytes, sha256_ok } => {
            FfiFrameEvent::StreamComplete { bytes, sha256_ok }
        }
    }
}

#[derive(uniffi::Object)]
pub struct FfiReceiver {
    inner: Mutex<CoreReceiver<FskPhy>>,
}

#[uniffi::export]
impl FfiReceiver {
    /// Receive at the default symbol rate.
    #[uniffi::constructor]
    pub fn new(profile: Profile, variants: DspVariants) -> std::sync::Arc<Self> {
        Self::new_at(profile, DEFAULT_SYMBOL_RATE, variants)
    }

    /// Receive at an explicit symbol rate, in symbols/second. Must match the
    /// rate the sender used, or the frame body demodulates to noise: the
    /// preamble is a fixed 80 ms chirp and locks at any rate, so a mismatch
    /// looks like a frame failure after a clean lock, not like silence.
    #[uniffi::constructor]
    pub fn new_at(profile: Profile, symbol_rate: u32, variants: DspVariants) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            inner: Mutex::new(CoreReceiver::new(phy_for(&profile, symbol_rate, variants))),
        })
    }

    /// Symbols per second this receiver actually demodulates at, read back off
    /// the PHY rather than from a remembered argument.
    pub fn symbol_rate(&self) -> u32 {
        let g = self.inner.lock().expect("FfiReceiver poisoned");
        g.phy().config().symbol_rate()
    }

    /// Push f32 samples (48 kHz mono). Returns any events produced.
    pub fn push_samples(&self, samples: Vec<f32>) -> Vec<FfiFrameEvent> {
        let mut g = self.inner.lock().expect("FfiReceiver poisoned");
        g.push_samples(&samples).into_iter().map(convert).collect()
    }
}
