use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::mpsc;

/// Play `samples` over the default output device at 48 kHz mono.
/// Blocks until all samples have been delivered to the stream callback.
pub fn play_samples(samples: Vec<f32>) -> Result<()> {
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or_else(|| anyhow!("no default output device"))?;

    let mut configs: Vec<_> = device.supported_output_configs()?.collect();
    // Prefer 48 kHz mono f32.
    configs.sort_by_key(|c| {
        let r = c.max_sample_rate().0;
        (r as i64 - 48_000).abs()
    });
    let supported = configs
        .into_iter()
        .find(|c| c.sample_format() == cpal::SampleFormat::F32)
        .ok_or_else(|| anyhow!("no f32 output config"))?
        .with_sample_rate(cpal::SampleRate(48_000));

    let channels = supported.channels() as usize;
    let config: cpal::StreamConfig = supported.into();

    let mut idx = 0usize;
    let total = samples.len();
    let (done_tx, done_rx) = mpsc::channel::<()>();
    let done_tx = std::sync::Mutex::new(Some(done_tx));

    let stream = device.build_output_stream(
        &config,
        move |out: &mut [f32], _| {
            for frame in out.chunks_mut(channels) {
                let v = if idx < total { samples[idx] } else { 0.0 };
                for ch in frame { *ch = v; }
                if idx < total { idx += 1; }
            }
            if idx >= total {
                if let Some(tx) = done_tx.lock().unwrap().take() {
                    let _ = tx.send(());
                }
            }
        },
        |e| eprintln!("output stream error: {e}"),
        None,
    )?;

    stream.play()?;
    // Wait until the callback has consumed all samples, then pad ~200ms.
    let _ = done_rx.recv();
    std::thread::sleep(std::time::Duration::from_millis(200));
    Ok(())
}
