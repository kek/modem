//! UniFFI surface around modem-codec.

use std::sync::Mutex;

use modem_codec::rx::{FrameEvent as CoreFrameEvent, Receiver as CoreReceiver};
use modem_codec::tx::Transmitter as CoreTransmitter;
use modem_core::phy::FskPhy;

uniffi::setup_scaffolding!();

#[derive(uniffi::Enum)]
pub enum Profile {
    Audible,
    Ultrasonic,
}

fn phy_for(profile: &Profile) -> FskPhy {
    match profile {
        Profile::Audible => FskPhy::audible(),
        Profile::Ultrasonic => FskPhy::ultrasonic(),
    }
}

#[derive(uniffi::Object)]
pub struct FfiTransmitter {
    inner: CoreTransmitter<FskPhy>,
}

#[uniffi::export]
impl FfiTransmitter {
    #[uniffi::constructor]
    pub fn new(profile: Profile) -> std::sync::Arc<Self> {
        let ultrasonic = matches!(profile, Profile::Ultrasonic);
        std::sync::Arc::new(Self {
            inner: CoreTransmitter::new(phy_for(&profile), ultrasonic),
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
    pub fn new(profile: Profile) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            inner: Mutex::new(CoreReceiver::new(phy_for(&profile))),
        })
    }

    /// Push f32 samples (48 kHz mono). Returns any events produced.
    pub fn push_samples(&self, samples: Vec<f32>) -> Vec<FfiFrameEvent> {
        let mut g = self.inner.lock().expect("FfiReceiver poisoned");
        g.push_samples(&samples).into_iter().map(convert).collect()
    }
}
