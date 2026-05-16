# Mac ↔ Android smoke test

## What works today

- **macOS↔macOS via [BlackHole 2ch](https://existential.audio/blackhole/) virtual loopback:** fully working for both `audible` and `ultrasonic` profiles. See `docs/smoke-test.md`.
- **WAV file roundtrip on either platform:** `modem tx-wav` produces a 48 kHz mono float WAV; `modem rx-wav` decodes it byte-identically. Pure DSP path, no audio hardware required.
- **Mac → Android over-the-air (audible):** ✅ decoded byte-identically with SHA-256 verification, with the phone mic ~20–30 cm from a MacBook speaker at ~75% volume in a quiet room.
- **Mac → Android over-the-air (ultrasonic):** ✅ decoded byte-identically. Inaudible to humans, no annoying warble.
- **Android app:** Pixel 8 Pro / Android 16, builds and installs, captures via `AudioSource.UNPROCESSED`, plays via `AudioTrack` on the FAST low-latency path bypassing Android's Dynamics Processing Effect.

## What does not work yet — Android → Mac

The Mac receiver locks onto the preamble (correlation score ~0.28–0.35) and starts decoding the frame body, but the demodulated sync word always has 1–4 bit errors and the rest of the frame exceeds RS's 16-byte recovery budget (RsUncorrectable). The same is true on both audible and ultrasonic profiles.

Root cause: the Pixel 8 Pro's small built-in speaker produces enough harmonic distortion at our tone frequencies that the demodulated symbols pick up neighbour-tone energy. The Mac's built-in mic + macOS audio input processing doesn't help.

Evidence that the path itself is OK:
- Phone media volume at 25/25, AudioTrack via `AUDIO_OUTPUT_FLAG_FAST` (verified in logcat), no Dynamics Processing Effect engaged.
- Sox capturing from Mac mic during phone send shows continuous RMS ~0.005–0.012 across the full 14 s transmission (was bursty ~0.001 with Dynamics Processing).
- Preamble detector triggers — so the chirp survives the channel.
- It's the frame body bytes that come out with too many bit errors.

## Profile choice

Use `--profile audible` for Mac→Android when you want the audible R2-D2 warble for demo theatre.
Use `--profile ultrasonic` when you want it silent. Both work for the *one* direction. ~150 bps either way.

## Reproduction procedure

Hardware: Android device with USB debugging, recognized by `adb devices`. Quiet room.

```bash
# Build & install
./scripts/build-android.sh
cd android && ./gradlew installDebug && cd ..

# Grant mic permission without UI tap
adb shell pm grant se.karleklund.modem android.permission.RECORD_AUDIO

# Launch the app
adb shell am start -n se.karleklund.modem/.MainActivity

# Tap Receive on the phone (or via adb).
# Mac speaker volume to ~70%.
# Phone mic 20–30 cm from Mac speaker.

echo "hello from the Mac" | ./target/release/modem send
# … wait ~16 s (14 s transmission + decode time).
# Phone screen displays "19 bytes · sha256 ok / hello from the Mac".
```

## Suggested follow-up DSP work for Android → Mac

Listed in increasing complexity:

1. **Pulse-shaping filter on TX.** Apply a raised-cosine or Gaussian envelope to each symbol so adjacent symbols share less spectral energy. Should suppress speaker-side intermodulation distortion. ~50 lines of Rust.

2. **Matched-filter detection instead of Goertzel.** Convolve the received signal with each tone's expected symbol shape (raised cosine of correct frequency) and pick the maximum. More robust to harmonic distortion than per-bin energy.

3. **Symbol-timing recovery loop.** Currently we lock at preamble correlation and assume perfect timing afterwards. Real receivers do early-late gate or Mueller-Müller continuous adjustment. Helps with clock drift between TX and RX devices.

4. **OFDM PHY** — spec's eventual upgrade. Cyclic prefix absorbs multipath naturally; per-subcarrier QAM with pilot tones gives equalization for free.

## Why we don't simply lower tone count or symbol rate further

We already cut from 8-FSK @ 100 sym/s @ 500 Hz spacing in 2.0–5.5 kHz to 8-FSK @ 50 sym/s @ 200 Hz spacing in 2.0–3.4 kHz. The next obvious step (4-FSK @ 50 sym/s @ 500 Hz spacing) would halve throughput again to ~75 bps and probably still hit the harmonic-distortion issue, because the problem is energy *leaking out* of our band into harmonics and back, not adjacent-tone confusion within the band.

## Known traps

- macOS AEC blocks single-Mac speaker→built-in-mic loopback. Use BlackHole (`docs/smoke-test.md`).
- Android's standard AudioSource applies AEC by default; the app uses AudioSource.UNPROCESSED to bypass it.
- USAGE_MEDIA + CONTENT_TYPE_SONIFICATION triggers Pixel's Dynamics Processing Effect, which compresses our FSK signal into bursts. AudioPlayer uses USAGE_UNKNOWN + CONTENT_TYPE_UNKNOWN + PERFORMANCE_MODE_LOW_LATENCY to dodge it.
