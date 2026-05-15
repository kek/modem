use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::mpsc::{self, Receiver};

pub struct Mic {
    pub rx: Receiver<Vec<f32>>,
    _stream: cpal::Stream,
}

/// Open the default input device at 48 kHz, mono, f32, delivering buffers
/// of samples to the returned receiver. Drop `Mic` to stop.
pub fn open_mic() -> Result<Mic> {
    let host = cpal::default_host();
    let device = host.default_input_device().ok_or_else(|| anyhow!("no default input device"))?;
    let mut configs: Vec<_> = device.supported_input_configs()?.collect();
    configs.sort_by_key(|c| {
        let r = c.max_sample_rate().0;
        (r as i64 - 48_000).abs()
    });
    let supported = configs.into_iter()
        .find(|c| c.sample_format() == cpal::SampleFormat::F32)
        .ok_or_else(|| anyhow!("no f32 input config"))?
        .with_sample_rate(cpal::SampleRate(48_000));
    let channels = supported.channels() as usize;
    let config: cpal::StreamConfig = supported.into();

    let (tx, rx) = mpsc::channel::<Vec<f32>>();
    let stream = device.build_input_stream(
        &config,
        move |samples: &[f32], _| {
            // Down-mix to mono by averaging channels.
            let mono: Vec<f32> = samples.chunks(channels)
                .map(|c| c.iter().sum::<f32>() / channels as f32)
                .collect();
            let _ = tx.send(mono);
        },
        |e| eprintln!("input stream error: {e}"),
        None,
    )?;
    stream.play()?;
    Ok(Mic { rx, _stream: stream })
}
