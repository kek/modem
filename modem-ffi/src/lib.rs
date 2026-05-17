//! UniFFI surface around modem-codec.

use std::sync::Mutex;

use modem_codec::rx::{FrameEvent as CoreFrameEvent, Receiver as CoreReceiver};
use modem_codec::tx::Transmitter as CoreTransmitter;
use modem_core::fsk::DspVariants as CoreDspVariants;
use modem_core::phy::FskPhy;

uniffi::setup_scaffolding!();

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

fn phy_for(profile: &Profile, variants: DspVariants) -> FskPhy {
    let v: CoreDspVariants = variants.into();
    match profile {
        Profile::Audible => FskPhy::audible_with(v),
        Profile::Ultrasonic => FskPhy::ultrasonic_with(v),
    }
}

#[derive(uniffi::Object)]
pub struct FfiTransmitter {
    inner: CoreTransmitter<FskPhy>,
}

#[uniffi::export]
impl FfiTransmitter {
    #[uniffi::constructor]
    pub fn new(profile: Profile, variants: DspVariants) -> std::sync::Arc<Self> {
        let ultrasonic = matches!(profile, Profile::Ultrasonic);
        std::sync::Arc::new(Self {
            inner: CoreTransmitter::new(phy_for(&profile, variants), ultrasonic),
        })
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
    #[uniffi::constructor]
    pub fn new(profile: Profile, variants: DspVariants) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            inner: Mutex::new(CoreReceiver::new(phy_for(&profile, variants))),
        })
    }

    /// Push f32 samples (48 kHz mono). Returns any events produced.
    pub fn push_samples(&self, samples: Vec<f32>) -> Vec<FfiFrameEvent> {
        let mut g = self.inner.lock().expect("FfiReceiver poisoned");
        g.push_samples(&samples).into_iter().map(convert).collect()
    }
}
