# Modem visualization UI — design

Status: draft (v1 scope)
Date: 2026-05-17

## Goal

Give the Android app and CLI a real-time visual sense of what the modem is
doing during send and receive. The current UI just shows status text and a
result — you can't tell from the screen whether the device is actually
hearing the FSK tones, where it is in the protocol, or why frames are being
dropped.

## Non-goals

- No DSP changes. Visualization is pure observability — it must not change
  decode behaviour or timing.
- No new FFI events in v1. We reuse the existing `FfiFrameEvent` stream and
  the audio buffers we already have in hand on the Kotlin/Rust side.
- No spectrogram/FFT in v1 (see "Future work").

## What we have to work with

From `modem-core`/`modem-codec`:
- `FrameEvent::FrameOk { seq, bytes }`, `FrameDropped { seq, reason }`,
  `StreamComplete { bytes, sha256_ok }`.
- 8-FSK with 8 known tone frequencies per profile (2000–3400 Hz audible,
  17500–19250 Hz ultrasonic) and `symbol_samples = 960` at 48 kHz (20 ms).
- Preamble template available via `phy.preamble()`; `detect_preamble` returns
  `(offset, score)` — but the current FFI doesn't expose it.

On the Android side:
- `AudioCapture.channel` already streams `FloatArray` chunks of ~50 ms.
- `ModemViewModel` already holds the encoded TX `samples` and playback timing.

On the CLI side:
- Rust has direct access to the audio chunks and the receiver state; can add a
  ratatui TUI alongside the existing plain output.

## Approach (chosen: A)

**Goertzel tone bars + frame timeline.** For each incoming chunk (or each
20 ms slice of an outgoing send), compute a Goertzel filter at each of the 8
tone frequencies. Display the 8 magnitudes as a vertical bar meter. Below
that, a horizontal "frame timeline" shows past frames as colored chips —
green for `FrameOk`, red for `FrameDropped` (with the drop reason on tap /
expand). A pulsing "searching…" indicator appears while no frame has arrived
yet. TX gets a progress bar that fills with elapsed playback time, with the
same tone bars animated from the outgoing samples so you see the FSK pattern
play out.

Why this approach over a full FFT spectrogram (option B):
- Goertzel at 8 known frequencies is ~50× cheaper than a 512-pt FFT per
  chunk; trivial battery cost on phone, trivial CPU in the TUI.
- It mirrors the demod's own world view: 8 tone energies per symbol is
  literally what the FSK decoder uses to pick a symbol.
- No FFT dependency on either side (no Apache Commons Math on Android, no
  `rustfft` pull-in for the CLI).
- The "is the modem hearing the right thing?" question reduces to "are the
  right tone bars lighting up?" — directly readable.

Future work (v2) can add a proper spectrogram waterfall on top of this; the
data plumbing is the same shape.

## Components

### Shared signal-processing primitive

`Goertzel` — a small block that, given an array of target frequencies and a
sample rate, computes per-tone magnitudes for a buffer of f32 samples.

- Implemented in `modem-core` as `goertzel_bank(samples, freqs, sample_rate)
  -> Vec<f32>`. Pure function, no state. ~30 LOC.
- Exposed through `modem-ffi` as `FfiSpectrum::compute(samples, freqs)` so
  Kotlin can call it instead of reimplementing Goertzel.
  - Alternative: reimplement in Kotlin to avoid an extra FFI round trip per
    UI tick (~20/s). Decision: reimplement in Kotlin. Goertzel is 8 lines;
    crossing the FFI boundary 20×/sec is unnecessary overhead. Both sides
    will have a tiny implementation against the same well-known formula.

This is the only DSP-adjacent code that gets duplicated; everything else
stays in modem-core.

### Android (Compose)

New file: `android/app/src/main/java/se/karleklund/modem/viz/`

- `Goertzel.kt` — `fun goertzelBank(samples: FloatArray, freqs: FloatArray,
  sampleRate: Int): FloatArray`. ~20 LOC.
- `VizEngine.kt` — eats f32 chunks and emits `VizState` as a `StateFlow`.
  Fields:
    - `tonesDb: FloatArray` — per-tone magnitude in dB, smoothed with a short
      EMA so bars don't strobe.
    - `rms: Float` — chunk RMS, used for the input-level indicator.
    - `timeline: List<FrameMark>` — append-only list of frame events
      (bounded to ~64 entries); a `FrameMark` holds seq, status (Ok/Dropped),
      reason, and a wall-clock timestamp for spacing.
    - `searching: Boolean` — true while no frame has arrived since
      `startReceive`. Drives the pulse animation.
    - `watchdogSecondsLeft: Int?` — counts down from 10 once we've started
      seeing frames.
  Inputs: `pushChunk(samples: FloatArray)` (called from the receive loop),
  `pushEvent(FfiFrameEvent)`, `reset()`.

- `ToneBars.kt` (Compose) — Canvas-drawn vertical bars, one per tone. Tone
  centres labeled below in Hz. Bar fill height = current `tonesDb`, clamped.
  Color: monochrome with brightness scaling by energy. Smooth 60 Hz redraw
  driven by the `VizState` flow.
- `FrameTimeline.kt` (Compose) — horizontal `LazyRow` of chips (latest on
  right). Each chip shows seq + a colored dot. Tapping a dropped chip
  reveals the drop reason inline. Auto-scrolls to the latest entry.
- `ListeningIndicator.kt` (Compose) — animated pulse (a circle scaling
  1.0 → 1.4 with alpha fading) shown while `searching == true`.
- `TxProgress.kt` (Compose) — linear progress bar driven by elapsed time vs
  total TX duration (we already know both). Also runs tone bars off the
  current 20 ms slice of the outgoing samples so the visualization animates
  in sync with the actual sound coming out of the speaker.

`ModemScreen.kt` is updated to host these components inside the existing
state-aware layout:

- During `Idle` and `Result`: tone bars greyed, frame timeline shows
  history of the most recent receive (or empty), result chip pulses
  green/red briefly on transition.
- During `Sending`: TX progress bar + tone bars animated from outgoing
  samples; a small chip "→ 32 bytes · 2.1 s".
- During `Receiving`: tone bars from mic, frame timeline appending, status
  header "listening 4.2 s · 3 ok · 1 dropped", watchdog countdown when
  applicable.
- During `Error`: red banner with the message; timeline preserved so the
  user can see what happened just before.

`ModemViewModel` changes:
- Hold a `VizEngine` instance with the same lifetime as the screen.
- In the receive loop, call `viz.pushChunk(chunk)` for every chunk before /
  alongside `rx.pushSamples(chunk)`, and `viz.pushEvent(e)` for each event.
- In the send loop, after computing `samples`, kick off a coroutine that
  iterates over the buffer in 20 ms slices at wall-clock rate and feeds
  `viz.pushChunk(slice)`. This decouples viz timing from `AudioTrack`'s
  blocking writes (we don't have a per-sample callback from AudioTrack).
- Reset the engine on `startReceive` / `send`.

### CLI (Rust)

Add a new module `modem-cli/src/tui.rs` that hosts a ratatui-based UI used
by `recv` and `send` when stdout is a TTY (`atty` check) or when
`--tui` is explicitly passed. Add `--plain` to force the existing text
output (needed for integration tests and scripts).

Dependencies: `ratatui` + `crossterm`. Both are stable and small enough to
take on.

Layout (recv):
```
┌ modem recv · audible ─────────────────────────────┐
│  level  ▁▂▄▅▇█▅▃▂▁  rms 0.07                      │
│                                                   │
│  ▁ ▂ █ ▃ ▂ ▁ ▁ ▂    ← tone bars (8 tones)         │
│  2.0k 2.2 2.4 2.6 2.8 3.0 3.2 3.4 kHz             │
│                                                   │
│  frames:  [ok 0] [ok 1] [DROP 2: RS] [ok 3] …     │
│                                                   │
│  listening 4.2 s · 3 ok · 1 dropped · watchdog 7  │
└───────────────────────────────────────────────────┘
```

Layout (send):
```
┌ modem send · audible · 32 B → 2.1 s ──────────────┐
│  ███████████████░░░░░░░░░░░░░░░░░  47%            │
│  ▁ ▂ █ ▃ ▂ ▁ ▁ ▂    (tones playing now)           │
└───────────────────────────────────────────────────┘
```

Implementation notes:
- The receive loop in `cmd_recv.rs` and send in `cmd_send.rs` move their
  output behind a small `Reporter` trait with two impls: `PlainReporter`
  (current behaviour) and `TuiReporter` (ratatui). This keeps the DSP and
  control flow untouched.
- Tone bars in the TUI use `▁▂▃▄▅▆▇█` block characters. Per-tone EMA
  smoothing the same as Android.
- On end (StreamComplete or Ctrl-C) the TUI cleans up and prints the same
  one-line summary the plain mode prints, so log capture still works.

## Data flow

```
TX:
  bytes -> Transmitter.encode -> samples
  samples -> AudioPlayer.play(samples)            (Android)
  samples -> 20ms slices @ wall-clock -> VizEngine.pushChunk
              ↓
            tone bars + TX progress bar render at 60 Hz

RX:
  mic -> AudioCapture.channel -> FloatArray chunk (50ms)
                                  |
                  +---------------+---------------+
                  ↓                               ↓
        rx.pushSamples(chunk)          viz.pushChunk(chunk)
                  ↓                               ↓
        FfiFrameEvent stream  -------> viz.pushEvent(e)
                                                  ↓
                                       tone bars + frame timeline render
```

## Testing

- `Goertzel`: unit tests in both Rust and Kotlin verifying that a pure sine
  at one of the target frequencies dominates and the others stay near zero.
  Cross-check Rust and Kotlin outputs on a small fixture for parity.
- `VizEngine`: unit tests for state transitions — searching → frame-arrived
  → watchdog countdown → reset.
- Compose previews / screenshot tests for `ToneBars` and `FrameTimeline`
  with seeded `VizState` fixtures (no actual audio needed).
- CLI: a snapshot test of the `Reporter` trait with the `PlainReporter`
  impl ensures we haven't regressed the existing text output (existing CLI
  integration tests already cover this if they only check stdout in
  `--plain` mode). `TuiReporter` doesn't get a snapshot test (terminal
  output is awkward to snapshot reliably); instead add a smoke test that
  drives it through a sequence of events and asserts it doesn't panic.

## Risks / open questions

- **TX-side animation drift.** Driving the viz off wall-clock slices isn't
  perfectly aligned with what's coming out of the speaker (AudioTrack has
  its own buffering). Acceptable for v1 because (a) the latency is small
  and (b) the user is watching their own outgoing signal, not trying to
  decode it. If it looks bad in practice, switch to driving viz off the
  AudioTrack playback head position (Android exposes
  `getPlaybackHeadPosition`).
- **Battery / CPU on Android.** Goertzel × 8 tones × 50 ms chunks = ~5
  multiplies per sample, negligible. Compose redraws at 60 Hz are the
  bigger concern; we already redraw on state changes, so this is in line
  with normal Compose usage.
- **CLI deps.** ratatui + crossterm add ~150 KB to the release binary.
  Acceptable for a developer tool; flagged here so it's not a surprise.
- **Preamble-score exposure.** A live "preamble score" plot would be a
  great debugging aid (especially for the flaky Android→Mac path) but
  requires a new FFI surface. Deferred to v2 along with the spectrogram.

## Out of scope for v1

- Spectrogram waterfall (option B above).
- Live preamble-score readout.
- Persistent recording UI (the WAV dump already lives in `AudioCapture`).
- A "replay this capture" UI feeding from `captures/`.

## Migration / rollout

- Behind no flag on Android — it's just additional UI.
- On the CLI, default to TUI when stdout is a TTY, plain otherwise. Add
  `--tui` / `--plain` overrides. Existing scripts and CI keep working
  because they're not TTYs.
