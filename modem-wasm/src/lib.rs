//! WebAssembly surface around `modem-codec`.
//!
//! Pure DSP layer for a browser-side modem: takes bytes → samples (TX) and
//! samples → events (RX). Audio I/O and visualization live in JavaScript.

use modem_codec::rx::{FrameEvent as CoreEvent, Receiver as CoreReceiver};
use modem_codec::tx::Transmitter as CoreTransmitter;
use modem_core::fsk::{goertzel_bank, DspVariants, SAMPLE_RATE};
use modem_core::phy::FskPhy;
use serde::Serialize;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(start)]
pub fn _start() {
    console_error_panic_hook::set_once();
}

/// Audio sample rate the modem operates at. Construct your `AudioContext`
/// with this value or the receiver will misdecode.
#[wasm_bindgen(js_name = sampleRate)]
pub fn sample_rate() -> u32 {
    SAMPLE_RATE
}

fn make_phy(profile: &str) -> Result<FskPhy, JsValue> {
    let v = DspVariants::default();
    match profile {
        "audible" => Ok(FskPhy::audible_with(v)),
        "ultrasonic" => Ok(FskPhy::ultrasonic_with(v)),
        other => Err(JsValue::from_str(&format!("unknown profile: {other}"))),
    }
}

/// Return the 8 FSK tone frequencies (Hz) for the given profile. Useful for
/// labelling tone bars in the UI.
#[wasm_bindgen(js_name = toneFreqs)]
pub fn tone_freqs(profile: &str) -> Result<Vec<f32>, JsValue> {
    let phy = make_phy(profile)?;
    Ok(phy.config().tone_freqs.to_vec())
}

/// Compute Goertzel magnitude² at each tone frequency for the given chunk.
/// Stateless; safe to call per audio frame for live visualization.
#[wasm_bindgen]
pub fn goertzel(samples: &[f32], profile: &str) -> Result<Vec<f32>, JsValue> {
    let phy = make_phy(profile)?;
    let cfg = phy.config();
    Ok(goertzel_bank(samples, &cfg.tone_freqs, cfg.sample_rate))
}

#[wasm_bindgen]
pub struct WasmTransmitter {
    inner: CoreTransmitter<FskPhy>,
}

#[wasm_bindgen]
impl WasmTransmitter {
    #[wasm_bindgen(constructor)]
    pub fn new(profile: &str) -> Result<WasmTransmitter, JsValue> {
        let phy = make_phy(profile)?;
        let ultrasonic = profile == "ultrasonic";
        Ok(Self {
            inner: CoreTransmitter::new(phy, ultrasonic),
        })
    }

    /// Encode `payload` into 48 kHz mono f32 samples.
    pub fn encode(&self, payload: &[u8]) -> Vec<f32> {
        self.inner.encode(payload)
    }
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum SerEvent {
    FrameOk { seq: u32, payload_bytes: usize },
    FrameDropped { seq: u32, reason: String },
    StreamComplete { bytes: Vec<u8>, sha256_ok: bool },
}

#[wasm_bindgen]
pub struct WasmReceiver {
    inner: CoreReceiver<FskPhy>,
}

#[wasm_bindgen]
impl WasmReceiver {
    #[wasm_bindgen(constructor)]
    pub fn new(profile: &str) -> Result<WasmReceiver, JsValue> {
        let phy = make_phy(profile)?;
        Ok(Self {
            inner: CoreReceiver::new(phy),
        })
    }

    /// Push captured samples. Returns a JS array of event objects with shape
    ///   { kind: 'frame_ok', seq: 0, payload_bytes: 219 }
    ///   { kind: 'frame_dropped', seq: 1, reason: 'RS uncorrectable' }
    ///   { kind: 'stream_complete', bytes: Uint8Array, sha256_ok: true }
    #[wasm_bindgen(js_name = pushSamples)]
    pub fn push_samples(&mut self, samples: &[f32]) -> Result<JsValue, JsValue> {
        let events: Vec<SerEvent> = self
            .inner
            .push_samples(samples)
            .into_iter()
            .map(|e| match e {
                CoreEvent::FrameOk { seq, bytes } => SerEvent::FrameOk {
                    seq,
                    payload_bytes: bytes.len(),
                },
                CoreEvent::FrameDropped { seq, reason } => SerEvent::FrameDropped { seq, reason },
                CoreEvent::StreamComplete { bytes, sha256_ok } => SerEvent::StreamComplete {
                    bytes,
                    sha256_ok,
                },
            })
            .collect();
        serde_wasm_bindgen::to_value(&events).map_err(|e| JsValue::from_str(&e.to_string()))
    }
}
