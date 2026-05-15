# Acoustic modem — design

**Date:** 2026-05-15
**Status:** Design approved, ready for implementation plan
**Inspiration:** [Quiet](https://github.com/quiet/org.quietmodem.Quiet)

## Goal

Build a cross-platform proof-of-concept that transmits arbitrary bytes between two devices using only their speakers and microphones. Primary devices: macOS desktop and Android phone. Combined motivation: party trick, learning, and validating feasibility for an AirDrop/croc-style "send to whoever is nearby, no network" use case.

## Scope

In scope:
- Arbitrary byte payloads (text is just bytes), up to a few KB per transmission for the PoC.
- Two interchangeable audio profiles: audible (~300 bps) and ultrasonic (~150 bps).
- One-way, half-duplex, one-shot transmissions. No acks, no retries.
- macOS CLI and a minimal Android app.

Out of scope (explicitly deferred):
- OFDM modulation. The PHY layer is designed swappable; OFDM is future work.
- ARQ (acknowledgements / retransmits). Noted as a future strategy.
- Fountain codes. Noted as a future strategy.
- Linux and Windows desktop. macOS only for the PoC.
- File pickers, share-sheet integration, Downloads writes on Android. Send is text-only via a text field; receive shows text or hex.

## Architecture

Six Rust workspace members, plus a Kotlin Android app:

| Crate           | Role                                                                                                       | Notable deps                                  |
|-----------------|------------------------------------------------------------------------------------------------------------|-----------------------------------------------|
| `modem-core`    | Pure DSP + framing. `Phy` trait, `FskPhy`, framer, RS, CRC. No I/O, no hot-path allocations. `no_std + alloc`. | `rustfft`, `reed-solomon-erasure`, `crc32fast` |
| `modem-codec`   | High-level `Transmitter` / `Receiver` state machines on top of `modem-core`. Synchronous, push-based.       | `modem-core`                                  |
| `modem-audio`   | Desktop audio glue, wires `cpal` streams to the codec.                                                      | `modem-codec`, `cpal`                         |
| `modem-cli`     | macOS binary: `modem send` / `modem recv` / `modem tx-wav` / `modem rx-wav`.                                | `modem-audio`, `clap`                         |
| `modem-ffi`     | UniFFI surface for Android. Kotlin owns audio I/O, samples flow through FFI.                                | `modem-codec`, `uniffi`                       |
| `android-app/`  | Kotlin + Compose. One screen, no file pickers.                                                              | `libmodem_ffi.so`                             |

**Principles:**
- All non-trivial logic lives in `modem-core` / `modem-codec`. Audio I/O is the only platform-specific concern.
- `Phy` trait isolates modulation. Adding OFDM later means a new `OfdmPhy` impl and zero changes elsewhere.
- No threads or async in the core. Callers (cpal callbacks, `AudioRecord.read()`) push sample buffers in and pull events out. This makes unit testing trivial and FFI clean.

## PHY layer — multi-tone FSK

Sample rate: **48 kHz** everywhere (native on macOS CoreAudio and Android `AudioTrack`/`AudioRecord`).

Two profiles share one `FskPhy` impl, differing only in `FskConfig`:

| Profile     | Modulation | Symbol rate | Tone spacing | Band            | Raw rate |
|-------------|------------|-------------|--------------|-----------------|----------|
| Audible     | 8-FSK      | 100 sym/s   | 500 Hz       | 2.0 – 5.5 kHz   | ~300 bps |
| Ultrasonic  | 8-FSK      | 50 sym/s    | 250 Hz       | 17.5 – 19.25 kHz | ~150 bps |

**Modulation:** the byte stream is chunked into 3-bit symbols; each symbol selects one of 8 tones, transmitted as a sine of the symbol duration. No phase coherence assumed across symbols.

**Demodulation:** sliding STFT, window = symbol duration. For each symbol slot, take FFT, pick the bin with maximum magnitude across the 8 tone bins. Non-coherent.

**Synchronization:** each frame begins with an ~80 ms linear up-chirp (e.g. 1.5 → 6 kHz). Receiver cross-correlates incoming samples against the known chirp; the correlation peak gives sample-level start of frame. A 2-byte sync word right after the chirp confirms it was real and absorbs sub-symbol timing slop.

**Why not faster than 300 bps?** Acoustic channels through air sit around 10–15 dB SNR with significant multipath (room reverb smears symbols), no carrier phase coherence, and uneven speaker/mic frequency response. Long symbols are required to outlast reverb (~10 ms vs. the ~1.6 ms of a V.22bis modem on a wire). FSK gives only log₂(N) bits per symbol vs. QAM's 4–9. OFDM with pilot tones and a cyclic prefix can reclaim much of this — hence "future work".

## Frame format

```
[ preamble | sync | header | payload | RS parity | CRC32 ]
   ~80 ms    2 B    4 B     ≤219 B    32 B         4 B
                   \___ RS(255,223) data ___/
```

Header (4 B) + payload (≤219 B) together fill one RS(255,223) data block (223 B). RS parity (32 B) protects all of it. CRC32 sits outside RS as a final integrity check on the decoded result.

| Field      | Size      | Notes                                                                                          |
|------------|-----------|------------------------------------------------------------------------------------------------|
| Preamble   | ~80 ms    | Linear up-chirp 1.5→6 kHz. Distinctive; speech rarely false-triggers.                         |
| Sync word  | 2 B       | Fixed bytes after the chirp; confirms a real preamble.                                         |
| Header     | 4 B       | version (1 B), flags (1 B: first-frame bit, last-frame bit, profile bit, reserved), payload length (2 B). Header is included under the same RS(255,223) block as the payload, so RS protects it. |
| Payload    | up to 219 B | Application bytes. Header + payload together fill the 223-byte RS data block. The last frame's payload ends with a 32-byte SHA-256, so its effective application payload is up to **187 B**.   |
| RS parity  | 32 B      | Reed–Solomon over header+payload. Corrects up to 16 byte errors per frame. ~14% overhead.     |
| CRC32      | 4 B       | Final integrity check on the decoded payload.                                                  |

**Stream semantics for payloads > 219 B:** the byte stream is split into a sequence of self-contained frames. Each frame re-syncs from its own preamble, so corruption of one frame does not desync the rest. The header's flags indicate first/last frame; the last frame's application payload ends with a SHA-256 of the full byte stream for end-to-end integrity.

## Rust API (sketch)

```rust
pub trait Phy {
    fn sample_rate(&self) -> u32;
    fn modulate(&self, bits: &[u8], out: &mut Vec<f32>);
    fn demodulate(&mut self, samples: &[f32]) -> Vec<u8>;
}

pub struct Transmitter<P: Phy> { /* … */ }
impl<P: Phy> Transmitter<P> {
    pub fn new(phy: P) -> Self;             // profile lives in the Phy impl's FskConfig
    pub fn encode(&mut self, payload: &[u8]) -> Vec<f32>;
}

pub struct Receiver<P: Phy> { /* … */ }
impl<P: Phy> Receiver<P> {
    pub fn new(phy: P) -> Self;
    pub fn push_samples(&mut self, samples: &[f32]) -> Vec<FrameEvent>;
}

pub enum FrameEvent {
    FrameOk { seq: u32, bytes: Vec<u8> },                       // per-frame, for live status
    FrameDropped { seq: u32, reason: DropReason },
    StreamComplete { bytes: Vec<u8>, sha256_ok: bool },         // last frame seen; full payload assembled
}
```

## CLI UX (macOS)

```text
$ echo "hello world" | modem send
$ modem recv
$ modem send --profile ultrasonic < secret.bin
$ modem recv -o out.bin
$ modem recv --hex
$ modem tx-wav in.bin out.wav     # offline encode
$ modem rx-wav captured.wav        # offline decode
```

- Stdin/stdout by default — pipeable into other tools.
- `--profile audible|ultrasonic`, `--device <name>`.
- `tx-wav`/`rx-wav` enable CI tests without sound hardware.

## Android UX

One Compose screen. No file pickers, no share-sheet, no `Downloads` writes.

- Profile toggle (audible / ultrasonic).
- A text field and a **Send** button. UTF-8 bytes of the field content are fed straight to the codec.
- A **Receive** button toggles listening. When a transmission completes, the result is displayed.
- **Printable detection:** result is shown as text iff all bytes are valid UTF-8 with no control characters except `\t \n \r`. Otherwise, the view switches to a 16-byte-per-row hex view with an ASCII gutter. A manual toggle flips between modes either way.
- "Copy" sends the current view (text or hex) to the clipboard.
- A foreground service runs during transmit/receive so screen-off doesn't interrupt audio.

## Testing strategy

Five layers, ordered by cost:

1. **Unit** — `modem-core` round-trips (encode → decode → assert equal) for each `FskConfig` and edge payload sizes around the per-frame boundary: 1 B, 186 B, 187 B (last-frame max), 188 B, 218 B, 219 B (single-frame max), 220 B (forces 2 frames), 1 KB.
2. **Property** — `proptest!` over arbitrary `Vec<u8>` up to 8 KB; catches off-by-ones in framing, RS chunk boundaries, last-frame handling.
3. **Channel simulation** — between encode and decode, inject realistic impairments and assert recovery thresholds:
   - `awgn(snr_db)` — at SNR ≥ 12 dB, audible profile must recover full payload.
   - `freq_offset(hz)` — ±20 Hz between TX and RX must be tolerated.
   - `multipath(delays_ms, gains)` — typical-room reverb must not break decoding.
   - `clip(threshold)` — loud-speaker clipping recovery.
   - `drop_window(start, len)` — a single 200 ms dropout per frame must still recover via RS.
4. **End-to-end via WAV** — `modem tx-wav` then `modem rx-wav`; assert output bytes match input. Exercises the full stack except real audio I/O. Runs in CI on macOS.
5. **Manual real-audio matrix** — MacBook↔MacBook, MacBook↔Android (both profiles), Android↔MacBook. Adversarial: TV on, music playing, cough mid-transmission. Optionally maintain a `recordings/` directory of real-mic captures replayed in CI via `rx-wav` for regression protection.

**Explicitly skipped:** Android instrumented tests for audio I/O (slow, fragile, low ROI for a PoC).

## Definition of done

1. Layers 1–4 of the test suite pass on macOS in CI.
2. One successful MacBook ↔ Android round-trip per profile, recorded on video.

## Risks and open questions

- **Sample-rate drift between devices.** Two consumer-grade clocks at 48 kHz drift by 10s of ppm — that's tens of Hz of effective frequency offset over a long transmission. Tone spacing of 250–500 Hz absorbs this comfortably; the channel-sim `freq_offset` test enforces it.
- **Android speaker/mic ultrasonic response varies wildly by device.** Pixel and Samsung flagships are generally OK to ~20 kHz; cheaper devices may roll off earlier. The ultrasonic profile is "best effort" — if it doesn't work on a given phone, audible always will.
- **macOS microphone permission UX.** The CLI will need to request mic access on first run; Terminal must be granted Microphone in System Settings → Privacy.
- **No back-channel means transmitter never knows if RX succeeded.** Acceptable for PoC. ARQ is a future strategy.

## Future work (deliberately out of scope)

- OFDM PHY (target: ~7 kbps audible) — drop-in `OfdmPhy` impl.
- ARQ with acoustic acks (full reliability at the cost of half-duplex turn-taking).
- Fountain / Raptor codes (start anytime, stop anytime).
- Linux and Windows desktop support.
- iOS app.
- File transfer UX on Android (share-sheet, file picker).
