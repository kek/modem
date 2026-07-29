# Mac ↔ Android smoke test

## What works today

- **macOS↔macOS via [BlackHole 2ch](https://existential.audio/blackhole/) virtual loopback:** fully working for both `audible` and `ultrasonic` profiles. See `docs/smoke-test.md`.
- **WAV file roundtrip on either platform:** `modem tx-wav` produces a 48 kHz mono float WAV; `modem rx-wav` decodes it byte-identically. Pure DSP path, no audio hardware required.
- **Mac → Android over-the-air (audible):** ✅ decoded byte-identically with SHA-256 verification, with the phone mic ~20–30 cm from a MacBook speaker at ~75% volume in a quiet room.
- **Mac → Android over-the-air (ultrasonic):** ✅ decoded byte-identically. Inaudible to humans, no annoying warble.
- **Android app:** Pixel 8 Pro / Android 16, builds and installs, captures via `AudioSource.UNPROCESSED`, plays via `AudioTrack` on the FAST low-latency path bypassing Android's Dynamics Processing Effect. The app shows an inline visualization (8-tone Goertzel bar meter + frame-event chip strip + watchdog readout) on every Send and Receive, mirroring the CLI's ratatui dashboard.

## What does not work yet — Android → Mac

The Mac receiver locks onto the preamble (correlation score ~0.28–0.35) and starts decoding the frame body, but the demodulated sync word always has 1–4 bit errors and the rest of the frame exceeds RS's 16-byte recovery budget (RsUncorrectable). The same is true on both audible and ultrasonic profiles.

Original hypothesis: the Pixel 8 Pro's small built-in speaker produces enough harmonic distortion at our tone frequencies that the demodulated symbols pick up neighbour-tone energy. The Mac's built-in mic + macOS audio input processing doesn't help.

**That hypothesis does not survive simulation — see "What the DSP tuning pass actually bought" below.** Speaker distortion on its own, modelled well past what a real micro-speaker does, costs zero bit errors. What does reproduce this exact symptom is multi-symbol inter-symbol interference from ordinary room reverberation.

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

## What the DSP tuning pass actually bought

This section used to be a wishlist of four suggested DSP remedies. Three of them
— **pulse-shaping on TX**, **matched-filter detection instead of Goertzel**, and
a **symbol-timing recovery loop** — have since been built, as runtime toggles in
`modem-core/src/fsk.rs` (`DspVariants { pulse_shape, matched_filter,
timing_recovery }`, tagged `p`, `m`, `t`), with `modem rank` sweeping all eight
combinations. **The answer is: they help, but they do not close this
direction.** The fourth, an OFDM PHY, is still the real fix. Evidence below.

### The stated root cause is not the cause

Speaker harmonic distortion alone does not break the link — not at the Pixel's
~10% THD, and not at 35% THD either, which is getting on for four times that.
Nor does the speaker's ragged modal response, a −1 dB near-field echo,
18 dB of tilt across the band, or wideband noise down to −12 dB SNR. Each was
simulated and each cost **zero** byte errors.

The reason is arithmetic. A memoryless odd nonlinearity driven by a *single*
tone at `f` radiates only `3f`, `5f`, … — all outside 2.0–3.4 kHz, and simply
not seen by the tone detector. FSK sends one tone at a time, so there is no
second tone to intermodulate with and fold energy back into the band. And a
960-sample Goertzel bin is 50 Hz wide against 24 kHz of noise bandwidth, i.e.
~27 dB of processing gain, on top of RS's 16-byte budget. There is a great deal
of margin here.

What *does* reproduce the observed symptom — preamble locks, sync word marginal,
frame body past the RS budget — is **inter-symbol interference spanning many
symbols**: ordinary room reverberation. At 20 ms per symbol, a 300 ms tail
smears every symbol across the next ~15. That is modelled in
`modem-core/src/channel.rs` as `AndroidToMac::PIXEL_8_PRO_AT_30CM` (micro-speaker
modal response → cone storage + excursion clipping → room reverb).

The tell is in `modem-codec/tests/android_to_mac.rs`: an all-`0xFF` payload,
which modulates to one unbroken tone with no symbol transitions, sails through
the very room that destroys every mixed-symbol payload — under plain trunk
`baseline`, with no remedies at all. Nothing is wrong with the tones. The
problem is what neighbouring symbols do to each other.

### How much the remedies help

Ranking a simulated corpus spanning the failure threshold (24 captures: 4
payloads × 3 room levels × 2 transmit modes):

```text
SUMMARY variant=baseline ok=14/24 no_preamble=8 rs_fail=2
SUMMARY variant=m ok=16/24 no_preamble=7 rs_fail=1
SUMMARY variant=m+t ok=18/24 no_preamble=6
SUMMARY variant=p ok=14/24 no_preamble=8 rs_fail=2
SUMMARY variant=p+m ok=16/24 no_preamble=7 rs_fail=1
SUMMARY variant=p+m+t ok=18/24 no_preamble=6
SUMMARY variant=p+t ok=14/24 no_preamble=8 rs_fail=2
SUMMARY variant=t ok=14/24 no_preamble=8 rs_fail=2
```

58% → 75%. Real, repeatable, and nowhere near enough to call the direction
working. Split by transmit mode (12 captures each):

| TX mode | `baseline` | best RX-side variant |
|---------|-----------|----------------------|
| trunk (rectangular symbols) | 7/12 | 9/12 (`m+t`, `p+m+t`) |
| pulse-shaped symbols | 7/12 | 9/12 (`m`, `m+t`, `p+m`, `p+m+t`) |

Two things to read off that table. Pulse shaping at TX lets the matched filter
reach the ceiling without needing timing recovery — but it does not *raise* the
ceiling. And `p` at RX is provably a no-op on a replayed capture (`p` scores
identically to `baseline`, `p+m` to `m`, `p+m+t` to `m+t`), exactly as
`docs/capture-corpus.md` warns: pulse shaping is baked in at transmit time.

**Best combination: `p+m+t`** — pulse-shaped transmit plus a Tukey-windowed
matched filter plus early-late timing recovery. `m` carries most of the weight;
`t` adds a little; `p` matters only if the *sender* enables it.

### Why they cannot fix it

None of the three is an equalizer. They all make a single symbol cleaner — a
gentler envelope, a window that de-weights corrupted edges, a nudged sampling
instant. None of them can undo interference arriving from fifteen symbols back.
So they shift the threshold a little and then fall off a cliff together:
`modem-codec/tests/android_to_mac.rs::no_variant_survives_a_more_reverberant_room`
pins the point where raising reverberation by one notch takes all eight
combinations to zero simultaneously.

Remedy 4 remains the real answer:

**OFDM PHY** — spec's eventual upgrade. Cyclic prefix absorbs multipath
naturally; per-subcarrier QAM with pilot tones gives equalization for free.
That is the one item on the original list that addresses inter-symbol
interference rather than symbol shape, and it is now the recommended next step
for this direction.

### What is evidence and what is simulation

Be clear about this, because the two are not interchangeable:

- **Over the air (real):** everything in "What works today", and the
  Android→Mac failure itself. Observed on a Pixel 8 Pro and a MacBook.
- **Simulated (not over the air):** every result in this section. There are no
  real Android→Mac captures in `captures/` on this machine, so nothing here was
  ranked against a genuine recording. The corpus is synthesised by
  `cargo run --release -p modem-codec --example gen_sim_corpus -- captures/simulated`,
  which pushes generated transmissions through the modelled channel.

The channel model is physically parameterised, not fitted to make a variant look
good — the speaker preset is pinned to a measured ~9.5% THD, in the 5–15% band
micro-speakers are quoted at, and asserted by
`speaker_preset_thd_is_physically_plausible`. But it is still a model, and it
does not match the real capture in every respect. In particular it gets the
preamble wrong: at the nominal room the modelled chirp still correlates at
~0.87, and at the `live` tier it falls under the 0.25 detection threshold
entirely (hence the `no_preamble` counts above). The real Android→Mac captures
sat at 0.28–0.35 — degraded but locking — which the model never reproduces.
Something in the real path attacks the chirp harder than reverberation does,
and that part is still unexplained.

**So: the conclusion that `p+m+t` is the best combination has not been confirmed
against a real Android→Mac recording.** Doing so is the obvious next step, and
needs nothing more than a phone, a quiet room, and `docs/capture-corpus.md`.
A capture that reproduces the 0.28–0.35 preamble score would also settle what
the model is missing.

### Reproducing all of the above

```bash
cargo test                       # includes modem-codec/tests/android_to_mac.rs
cargo run --release -p modem-codec --example gen_sim_corpus -- captures/simulated
cargo build --release && ./target/release/modem rank captures/simulated
```

## Why we don't simply lower tone count or symbol rate further

We already cut from 8-FSK @ 100 sym/s @ 500 Hz spacing in 2.0–5.5 kHz to 8-FSK @ 50 sym/s @ 200 Hz spacing in 2.0–3.4 kHz. The next obvious step (4-FSK @ 50 sym/s @ 500 Hz spacing) would halve throughput again to ~75 bps and probably still hit the harmonic-distortion issue, because the problem is energy *leaking out* of our band into harmonics and back, not adjacent-tone confusion within the band.

**Correction, from the simulation work above:** the conclusion still holds, but not for that reason. Widening tone spacing doesn't help because the failure isn't tone confusion of *any* kind — it's inter-symbol interference, and every tone suffers it equally. Note the direction this points, though: a *lower* symbol rate is the one cheap knob that does attack ISI, since it buys guard time relative to the reverb tail. Halving to 25 sym/s costs half the throughput and is worth measuring before committing to OFDM.

## Known traps

- macOS AEC blocks single-Mac speaker→built-in-mic loopback. Use BlackHole (`docs/smoke-test.md`).
- Android's standard AudioSource applies AEC by default; the app uses AudioSource.UNPROCESSED to bypass it.
- USAGE_MEDIA + CONTENT_TYPE_SONIFICATION triggers Pixel's Dynamics Processing Effect, which compresses our FSK signal into bursts. AudioPlayer uses USAGE_UNKNOWN + CONTENT_TYPE_UNKNOWN + PERFORMANCE_MODE_LOW_LATENCY to dodge it.
