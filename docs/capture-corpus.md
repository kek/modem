# Capture corpus — over-the-air ranking harness

`modem rank <dir>` sweeps the eight DSP variant combinations across a directory
of WAV captures and emits one structured outcome line per `(capture, variant)`,
followed by per-variant aggregates. Designed to be driven by an agent — no
human eyeballing required.

```text
SUMMARY rate=50 variant=baseline ok=0/20 no_preamble=8 sync_fail=4 rs_fail=8
SUMMARY rate=50 variant=p+m+t    ok=15/20 rs_fail=3 sync_fail=2
```

Aggregates are keyed by `(symbol rate, variant)`, because a corpus may mix
symbol rates and averaging across them would mean nothing.

## What lives where

| Path | Tracked? | Purpose |
|------|----------|---------|
| `captures/` (entire directory) | **gitignored** | Per-machine working area. Each developer captures their own WAVs. |
| `captures/*.wav` | gitignored | Real over-the-air recordings (typically 4–10 MB each). |
| `captures/manifest.toml` | gitignored | Pairs each WAV with the payload it should decode to. Local because the manifest only makes sense alongside the WAVs it references. |
| `docs/capture-corpus.md` (this file) | tracked | Schema, recipe, and the `modem rank` contract. |

`captures/` is in `.gitignore` because real capture sessions accumulate
hundreds of MB and the WAVs are not portable across machines (different
mic, different room, different speaker). Don't try to commit them. Each
developer maintains a personal corpus and runs `modem rank` against it.

## Recording a capture

You'll typically be capturing one device's transmission with another
device's microphone. From the receiving end (e.g. macOS):

```bash
# Record 22 s of mic audio at 48 kHz mono float.
rec -c 1 -r 48000 -b 32 -e floating-point captures/android-to-mac-007.wav trim 0 22 &

# Within ~2 s of starting the recording, trigger the sender:
#   - tap the Send button on the Android app, OR
#   - on the Mac: echo "your payload here" | ./target/release/modem send
```

Trim the result if there's excessive silence at the start or end — the
preamble detector works fine with leading silence, but anything longer
than ~5 s is just wasted CPU.

**Record long enough for one whole frame, with margin.** A frame is 261 bytes
over the air (2 sync + 259), i.e. 696 symbols, so its air time scales with the
symbol rate: **14.0 s at 50 sym/s, 27.9 s at 25 sym/s.** The `trim 0 22` above
is right for 50 and would truncate a 25 sym/s frame — use `trim 0 32` there.
This matters more than it looks, because `Receiver::step` will not start
searching for a preamble until it holds `preamble + payload_samples`, so a
capture shorter than one frame produces `no_preamble` for every variant. **A
too-short recording is indistinguishable in the rank output from a signal that
never arrived.** If a whole corpus reads `no_preamble=N/N`, check the durations
and the `symbol_rate` field before suspecting the DSP.

## Manifest schema

`captures/manifest.toml` is the index of your corpus. Append an entry per
WAV you want included in ranking:

```toml
[[capture]]
file = "android-to-mac-007.wav"
expected = "hello from Android"      # plain UTF-8 payload
direction = "android-to-mac"          # free-form label, surfaced in output
profile = "audible"                   # "audible" | "ultrasonic"
symbol_rate = 50                      # optional; symbols/second, defaults to 50

[[capture]]
file = "weird-binary.wav"
expected_hex = "deadbeef00ff"         # use instead of `expected` for arbitrary bytes
direction = "mac-to-android"
profile = "ultrasonic"
```

`symbol_rate` is not something the harness can sweep — the sender fixed it when
it put the signal in the air, exactly like the profile and the `p` flag. Decode
a 25 sym/s recording at 50 and you get nothing at all, so an omitted field
silently means "this was recorded at 50". Get it right, or every cell reads as a
failure of the DSP rather than of the manifest.

Empty manifest (`capture = []`) is valid — `modem rank` will just emit no
per-cell lines and no summaries.

## Running

```bash
cargo build --release
./target/release/modem rank captures/
```

Pipe the output through any standard text-processing tool:

```bash
# Top variants by ok rate
./target/release/modem rank captures/ | grep '^SUMMARY' | sort -t= -k3 -rn

# Which captures still fail under the best-so-far variant
./target/release/modem rank captures/ | awk '$4 == "variant=p+m+t" && $5 !~ /=ok$/'
```

## Outcome categories

Each `(capture, variant)` cell produces exactly one outcome:

| Outcome | Meaning |
|---------|---------|
| `ok` | StreamComplete event, SHA-256 matched, decoded bytes equal `expected` |
| `no_preamble` | No frame events at all — the chirp was never found. Signal too quiet, wrong `symbol_rate`, or the wrong file |
| `sync_fail` | Preamble found but sync word had too many bit errors |
| `rs_fail` | Sync passed but frame body exceeded Reed–Solomon's correction budget |
| `crc_fail` | RS reported success but the CRC32 disagreed — RS "corrected" past its budget into a plausible-looking wrong frame |
| `frame_fail` | A frame event happened, but the stream never completed: some other framing error, or the final frame never arrived |
| `sha_mismatch` | Frame decoded, but final SHA-256 didn't match the trailer |
| `wrong_bytes` | Frame decoded, SHA matched, but decoded bytes didn't equal `expected` |

The distinction between `no_preamble` and the rest is load-bearing for
Android→Mac, where the real symptom is a preamble that locks and a body that
does not decode. `crc_fail` and `frame_fail` exist because those cells used to
fall through to `no_preamble`, which made frame-body failures look like signal
failures — see `docs/android-smoke-test.md`.

## Variant tags in the output

| Tag | TX side | RX side |
|-----|---------|---------|
| `baseline` | rectangular envelope | Goertzel, no timing tracking |
| `p` | raised-cosine envelope | (no RX change — `p` is a TX-only flag) |
| `m` | (no TX change) | Tukey-windowed matched filter |
| `t` | (no TX change) | early-late symbol-timing tracker |
| `p+m`, `m+t`, `p+m+t` | combinations | combinations |

Caveat: when the rank harness replays a pre-recorded WAV, the `p` flag is
a no-op because the TX already happened on the sending device. Only `m`
and `t` actually change RX behaviour for replayed WAVs. To test pulse
shaping for real you need a fresh capture from a sender that had
`pulse_shape=true` enabled at transmit time (Android app filter chip, or
`modem send --pulse-shape` on the Mac).

The same applies, more sharply, to `symbol_rate`: it is baked in at transmit
time and the harness can only be *told* what it was. Every `modem` subcommand
now accepts `--symbol-rate`, live audio (`send`, `recv`, `chat`) included, and
`modem-ffi` exposes it as `FfiTransmitter.newAt` / `FfiReceiver.newAt`. So a
Mac-side 25 sym/s capture needs nothing more than the flag on both ends. The
Android app is the one sender still pinned to 50: it calls the rate-less
constructors, so a 25 sym/s capture *from the phone* waits on the app offering
the rate.
