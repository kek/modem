# Mac ↔ Android smoke test

## What works today

- macOS↔macOS via [BlackHole 2ch](https://existential.audio/blackhole/) virtual loopback: **fully working** for both `audible` and `ultrasonic` profiles. See `docs/smoke-test.md`.
- WAV file roundtrip on either platform: `modem tx-wav` produces a 48 kHz mono float WAV; `modem rx-wav` decodes it back. Pure DSP path, no audio hardware required.
- Android app builds, installs, and runs on Pixel 8 Pro / Android 16 with `AudioSource.UNPROCESSED` capture and `AudioTrack` playback at 48 kHz mono `f32`.
- Preamble detection on real over-the-air signal (Mac speaker → 20–30 cm air → Pixel mic) **does work** at the lowered correlation threshold of 0.3 (commit `df4dc227`).

## What does not work yet — Mac → Android over-the-air decode

The Android receiver picks up the preamble (verified: cross-correlation score ~0.34 over real audio), but the FSK demodulator confuses tones inside the frame body. The captured WAV from the Pixel mic shows the modem signal at clear amplitude (RMS ~0.005, peak ~0.02), but `modem rx-wav` on the captured file fails with `Err(RsUncorrectable)` after sync-word bit errors.

Concrete evidence: with a transmission of `"hello from the Mac"`, the demodulated sync word came out `[6e 81]` vs expected `[7e 81]` — i.e. one symbol decoded as tone 3 (3500 Hz) instead of tone 7 (5500 Hz). Tone confusion at the high end of the audible band suggests non-flat frequency response of the speaker/mic chain.

The same captured WAV decodes cleanly when fed back through `modem rx-wav` after a +30 dB amplification, but only when the threshold is lowered to 0.2; at the post-amplification realistic scores of ~0.4 we still get sync-word bit errors. **So it's not a level-only problem** — Goertzel's tone classification is failing under acoustic distortion that synthetic AWGN channel impairments don't reproduce.

## Reproduction procedure (when you want to retry)

Hardware required:
- An Android device with USB debugging enabled, recognized by `adb devices`.
- Quiet room.

```bash
# Build & install
./scripts/build-android.sh
cd android && ./gradlew installDebug && cd ..

# Grant mic permission without UI tap
adb shell pm grant se.karleklund.modem android.permission.RECORD_AUDIO

# Launch app
adb shell am start -n se.karleklund.modem/.MainActivity

# Tap Receive in the app (manually or via adb shell input tap <x> <y>).
# The status line should change to "Listening… 0 frame(s) ok".

# Set Mac speaker to ~70% volume
osascript -e "set volume output volume 70"

# Hold the phone 20–30 cm from the laptop speaker
echo "hello from the Mac" | ./target/release/modem send

# Watch the phone screen — expect "Listening… 1 frame(s) ok" then the decoded text.
# Today: expect "Error: no progress for 10 s" — the watchdog fires because the
# preamble is detected but the frame body decode fails.
```

To inspect what the phone mic captured:
```bash
# Pull the latest WAV (if you've re-enabled the dev capture, see git history)
adb shell ls /sdcard/Download/ | grep mic   # not enabled in current build
# OR: use the dev-mode `tinycap` if available on your device, or QuickTime via AirDrop.

# Run rx-wav offline against the captured signal (works ~50% of the time after sox cleanup):
sox /tmp/captured.wav /tmp/clean.wav gain 25
./target/release/modem rx-wav /tmp/clean.wav
```

## Suggested follow-up DSP work (rough ordering by effort)

1. **Tighten tone spacing into a flatter-response band.** Move audible tones from 2.0–5.5 kHz to e.g. 2.0–3.5 kHz with 8 tones spaced 200 Hz. Cuts throughput from ~300 to ~250 bps, but most consumer speakers/mics are dramatically flatter in this band. Likely the highest ROI single change.

2. **Drop from 8-FSK to 4-FSK.** Halves throughput to ~150 bps but doubles per-tone spacing (1 kHz), making tone confusion much less likely. Combined with #1 = ~125 bps but should decode reliably.

3. **Implement symbol-timing recovery.** Currently the receiver uses the preamble correlation peak position (resolution: 4 samples) as the absolute symbol-boundary anchor. A real receiver would do early-late gate or Mueller-Müller timing recovery. This addresses sample-rate clock drift between TX and RX devices that the synthetic `freq_offset` channel impairment doesn't fully model.

4. **Channel equalization.** Use the chirp preamble as a probe to estimate the channel impulse response, then invert. Expensive but eliminates speaker/mic FR effects.

5. **OFDM PHY.** The spec's planned upgrade. ~10× throughput at the cost of a real DSP effort.

## Why the Mac↔Mac BlackHole test still validates the design

BlackHole is a digital loopback — no DAC, no speaker, no air, no mic, no ADC. It validates: framer, RS, CRC, modulator, demodulator, preamble correlation, receiver state machine, multi-frame assembly, SHA-256 end-to-end check, the CLI plumbing, and (separately) the UniFFI/Kotlin/AudioTrack/AudioRecord plumbing on Android. The single thing it doesn't validate is the acoustic channel — which is exactly the open problem above.
