# modem

An acoustic modem. Sends bytes through the air as audio (or through a WAV file) and decodes them back. Pure software — uses whatever speaker and microphone are attached.

**Try it in your browser:** <https://kek.github.io/modem/> — the same Rust DSP compiled to WebAssembly, driving Web Audio for capture + playback. Works on Chrome/Firefox/recent Safari. Pick a profile, type something, hit Send — or hit Start listening on a second device to receive.

Two profiles:
- **audible** — 8-FSK in 2.0–3.4 kHz, sounds like an R2-D2 warble. ~150 bps.
- **ultrasonic** — same scheme shifted above ~17 kHz. Silent to most humans, range limited by speaker rolloff.

## Workspace layout

| Crate | Role |
|-------|------|
| `modem-core` | Framing, CRC, Reed–Solomon, FSK modulator/demodulator, preamble |
| `modem-codec` | High-level `Transmitter` / `Receiver` that turn bytes ↔ samples |
| `modem-audio` | CPAL bindings for live mic input and speaker output |
| `modem-cli` | `modem` binary — `send`, `recv`, `tx-wav`, `rx-wav` |
| `modem-ffi` | UniFFI bindings so Android can drive the same DSP code |
| `uniffi-bindgen` | Workspace-pinned `uniffi-bindgen` binary used by the Android build |

The `android/` directory holds a Compose app (`se.karleklund.modem`) that wraps `modem-ffi` via JNA, using `AudioSource.UNPROCESSED` for capture and a low-latency `AudioTrack` for playback.

## Build

```bash
cargo build --release
```

The CLI lives at `target/release/modem`.

## Quick start (single Mac, WAV roundtrip)

```bash
echo "hello modem" | ./target/release/modem tx-wav out.wav
./target/release/modem rx-wav out.wav
```

## Live visualization

`modem send` and `modem recv` show a ratatui dashboard (tone bars, frame
timeline, progress) when stdout is a TTY. Pass `--plain` to force the
historical line-by-line output (used by scripts and CI), or `--tui` to
force the dashboard when piping.

The Android app shows the same kind of dashboard inline on every Send and
Receive — tone bars driven by Goertzel filters at the 8 FSK tone
frequencies, a green/red chip strip for frames, a pulsing "listening"
indicator while no preamble has been found, and a 10 s watchdog readout
that triggers once the first frame arrives.

## Over the air

See [`docs/smoke-test.md`](docs/smoke-test.md) for Mac↔Mac (needs [BlackHole](https://existential.audio/blackhole/) — macOS AEC blocks single-device speaker→mic loopback).

See [`docs/android-smoke-test.md`](docs/android-smoke-test.md) for Mac↔Android. Mac→Android works on both profiles. Android→Mac is still flaky — the Pixel 8 Pro's small speaker produces enough harmonic distortion that the demodulated frame body usually exceeds Reed–Solomon's recovery budget. Open follow-ups (pulse shaping, matched filtering, timing recovery) are listed in that doc.

## Android build

```bash
./scripts/install-android-prereqs.sh   # one-time: cargo-ndk + NDK toolchain
./scripts/build-android.sh             # cross-compile + uniffi-bindgen
cd android && ./gradlew installDebug
```

## License

MIT
