# modem · web POC

**Live demo:** <https://kek.github.io/modem/> (deployed from `trunk` by `.github/workflows/deploy-pages.yml`).

A browser page that uses the same `modem-core` / `modem-codec` crates as the
CLI and Android app, compiled to WebAssembly via `wasm-bindgen`. JavaScript
owns audio I/O through the Web Audio API; the wasm module owns the DSP.

## Build

```bash
# Compiles modem-wasm and runs wasm-bindgen. First run installs
# wasm-bindgen-cli and the wasm32-unknown-unknown target.
./scripts/build-wasm.sh
```

This writes `web/pkg/modem_wasm.js` + `web/pkg/modem_wasm_bg.wasm`.

## Serve

Any static file server works; ES modules need an `http://` origin
(`file://` won't load the worklet). Two easy options:

```bash
python3 -m http.server -d web 8000
# then open http://localhost:8000
```

```bash
npx http-server web
```

## Use

- Pick a profile (`audible` or `ultrasonic`).
- Type something, press Send → encoder produces 48 kHz samples, played
  through `AudioBufferSourceNode`. The tone bars animate from the encoded
  buffer while playback proceeds.
- Click `Start listening` → page asks for microphone permission. Captured
  samples are routed through an `AudioWorkletNode` (realtime thread,
  ~50 ms chunks) into the wasm `Receiver`. Decoded frames and complete
  messages appear in the log.

## Notes

- The modem hardcodes 48 kHz. The page constructs
  `AudioContext({ sampleRate: 48000 })`; if the device refuses, you'll see
  a warning in the log and decode may misalign. Most desktops, Chrome on
  Android, and recent Safari honour the request.
- A user gesture (button click) is required to start audio under browser
  autoplay policies.
- The worklet captures with `echoCancellation: false`, `autoGainControl:
  false`, `noiseSuppression: false` — DSP needs the raw signal.
- The worklet is NOT connected to `ctx.destination`; doing so would loop
  the mic back to the speakers.
- For a single-machine loopback (mic hears its own speaker), macOS AEC at
  the OS level still applies, so you need either a virtual loopback (e.g.
  BlackHole) or two devices in the same room.
