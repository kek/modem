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

**Best candidate fix so far: halve the symbol rate to 25 sym/s.** In simulation that clears the whole corpus where the DSP tuning pass reached 75%, at the cost of half the bitrate (150 → 75 bps). Not yet confirmed over the air — see "What halving the symbol rate to 25 sym/s buys" below.

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
SUMMARY rate=50 variant=baseline ok=14/24 crc_fail=8 rs_fail=2
SUMMARY rate=50 variant=m ok=16/24 crc_fail=7 rs_fail=1
SUMMARY rate=50 variant=m+t ok=18/24 crc_fail=6
SUMMARY rate=50 variant=p ok=14/24 crc_fail=8 rs_fail=2
SUMMARY rate=50 variant=p+m ok=16/24 crc_fail=7 rs_fail=1
SUMMARY rate=50 variant=p+m+t ok=18/24 crc_fail=6
SUMMARY rate=50 variant=p+t ok=14/24 crc_fail=8 rs_fail=2
SUMMARY rate=50 variant=t ok=14/24 crc_fail=8 rs_fail=2
```

(Those failures were originally reported as `no_preamble`, which was a
mislabel in the harness — see "A correction to the harness" below. The `ok`
counts are unchanged.)

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

Remedy 4 remains the only one that addresses the mechanism:

**OFDM PHY** — spec's eventual upgrade. Cyclic prefix absorbs multipath
naturally; per-subcarrier QAM with pilot tones gives equalization for free.
That is the one item on the original list that addresses inter-symbol
interference rather than symbol shape.

But it is a large undertaking, and there was one cheap knob left that attacks
ISI without any of that machinery: **halve the symbol rate**. That has now been
measured, and it beats the entire tuning pass — see the next section. OFDM is
still the right eventual answer; it is no longer the only thing left to try.

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
preamble wrong, and in the *optimistic* direction: the modelled chirp correlates
at ~0.86 at the nominal room and still ~0.85 at the `live` tier — it never
struggles at all. The real Android→Mac captures sat at 0.28–0.35 — degraded but
locking — which the model never reproduces. Something in the real path attacks
the chirp far harder than reverberation does, and that part is still unexplained.

(An earlier version of this paragraph said the modelled chirp falls under the
0.25 threshold at the `live` tier, and attributed the harness's `no_preamble`
counts to that. Both halves were wrong; see "A correction to the harness,
found on the way" below.)

**So: the conclusion that `p+m+t` is the best combination has not been confirmed
against a real Android→Mac recording.** Doing so is the obvious next step, and
needs nothing more than a phone, a quiet room, and `docs/capture-corpus.md`.
A capture that reproduces the 0.28–0.35 preamble score would also settle what
the model is missing.

### Reproducing all of the above

```bash
cargo test --release             # includes modem-codec/tests/android_to_mac.rs
cargo run --release -p modem-codec --example gen_sim_corpus -- captures/simulated
cargo build --release && ./target/release/modem rank captures/simulated
```

Use `--release`. The `stream_roundtrip` proptest takes tens of minutes in debug.

## What halving the symbol rate to 25 sym/s buys

> **Every number in this section is simulated.** No Android→Mac transmission at
> 25 sym/s has been recorded or decoded over the air, on this machine or any
> other. The rate is now expressible everywhere on the Rust side — live CLI
> audio and the UniFFI surface included — but expressible is not recorded, and
> the Android app still asks for the default. What follows is evidence from a
> channel model, and that model is known not to match the real captures on the
> preamble. Read it as "the strongest candidate found so far", never as
> "Android→Mac works now".

The section above named exactly one cheap knob still unmeasured: drop the
symbol rate. It has now been measured. **In simulation it is the first remedy
that clears the corpus outright** — the DSP tuning pass topped out at 75%, and
this reaches 100%. It also costs half the bitrate, which is the whole of the
catch.

### The measurement

Same channel, same speaker preset, same corpus, same eight variants — only the
symbol rate changed. 24 simulated captures each (4 payloads × 3 room levels ×
2 transmit modes), generated and ranked as:

```bash
cargo run --release -p modem-codec --example gen_sim_corpus -- captures/simulated 50
cargo run --release -p modem-codec --example gen_sim_corpus -- captures/simulated-25 25
./target/release/modem rank captures/simulated
./target/release/modem rank captures/simulated-25
```

```text
SUMMARY rate=50 variant=baseline ok=14/24 crc_fail=8 rs_fail=2
SUMMARY rate=50 variant=m        ok=16/24 crc_fail=7 rs_fail=1
SUMMARY rate=50 variant=m+t      ok=18/24 crc_fail=6
SUMMARY rate=50 variant=p        ok=14/24 crc_fail=8 rs_fail=2
SUMMARY rate=50 variant=p+m      ok=16/24 crc_fail=7 rs_fail=1
SUMMARY rate=50 variant=p+m+t    ok=18/24 crc_fail=6
SUMMARY rate=50 variant=p+t      ok=14/24 crc_fail=8 rs_fail=2
SUMMARY rate=50 variant=t        ok=14/24 crc_fail=8 rs_fail=2

SUMMARY rate=25 variant=baseline ok=21/24 crc_fail=3
SUMMARY rate=25 variant=m        ok=24/24
SUMMARY rate=25 variant=m+t      ok=24/24
SUMMARY rate=25 variant=p        ok=21/24 crc_fail=3
SUMMARY rate=25 variant=p+m      ok=24/24
SUMMARY rate=25 variant=p+m+t    ok=24/24
SUMMARY rate=25 variant=p+t      ok=22/24 crc_fail=2
SUMMARY rate=25 variant=t        ok=22/24 crc_fail=2
```

Read the two blocks against each other:

| | 50 sym/s | 25 sym/s |
|---|---|---|
| `baseline`, no remedies | 14/24 (58%) | **21/24 (88%)** |
| best variant | 18/24 (75%), `m+t` / `p+m+t` | **24/24 (100%)**, any variant with `m` |

Halving the rate is worth more than the entire DSP tuning pass — 58% → 88% with
no remedies at all, against 58% → 75% for the best combination at 50 sym/s. And
with the matched filter on top, the simulated corpus is clean.

It also does the thing none of the three remedies could: **it moves the cliff.**
`no_variant_survives_a_more_reverberant_room` pins that at `reverb_wet = 0.25`
all eight combinations go to zero together, because none of them is an
equalizer. 25 sym/s is not an equalizer either, but it halves how many symbols
the reverberant tail can reach into, and that is enough to put survivors back on
the board in the room that was a wipeout — pinned by
`halving_the_symbol_rate_moves_the_cliff`.

### The bill

Exactly half the bitrate. This is not a free win:

| | 50 sym/s | 25 sym/s |
|---|---|---|
| symbol duration | 20 ms (960 samples) | 40 ms (1920 samples) |
| raw rate | 150 bps | **75 bps** |
| air time, one 259-byte frame | 14.00 s | **27.92 s** |
| goodput, 54-byte payload | 30.9 bps | **15.5 bps** |

A single frame carries at most 219 payload bytes inside 259 bytes of RS+CRC, so
short messages are dominated by frame overhead at either rate — "hello from the
Pixel 8 Pro" takes 14 s today and would take 28 s. That is the trade on the
table: a link that works at half the speed, versus a faster one that doesn't.
`halving_the_symbol_rate_halves_the_bitrate` pins the cost so it cannot quietly
be forgotten.

Clock-skew tolerance is unaffected, which is worth stating because one might
expect longer symbols to be more fragile: ±30 Hz at 48 kHz (~625 ppm) decodes at
both rates and ±40 Hz fails at both. A fixed *Hz* offset does slip twice as many
samples over a 25 sym/s frame, since the frame occupies twice the air time — but
the symbol window doubles with it, so the slip as a fraction of a symbol is
identical, and so is the breaking point. Pinned by
`modem-core/tests/channel_recovery.rs::freq_offset_tolerance_is_the_same_at_both_symbol_rates`,
which asserts both the ±30 Hz pass and the ±40 Hz failure at each rate.

### What it does not buy

- **It is not a cure, only margin.** Push the room to `reverb_wet = 0.35` and
  25 sym/s dies as thoroughly as 50 did, across all eight variants —
  `halving_the_symbol_rate_is_not_a_cure_for_a_wet_room`. Slower symbols buy
  guard time against ISI; they do not equalize it. OFDM with a cyclic prefix is
  still the remedy that addresses the mechanism rather than the margin.
- **It cannot touch the preamble.** The chirp is a fixed 80 ms waveform that
  knows nothing about symbols, and it correlates *identically* at both rates
  over the same recording — verified in
  `symbol_rate_does_not_change_the_preamble`, not merely argued. That matters
  because the preamble is precisely where this model does not match reality (see
  below), so the one unexplained part of the real failure is exactly the part
  this change cannot help.

### A correction to the harness, found on the way

The `no_preamble` counts in the block above used to read `no_preamble=8` rather
than `crc_fail=8`, and the previous section's explanation for them — that the
modelled chirp falls under the 0.25 detection threshold at the `live` tier — was
**wrong**. Probing those exact captures shows the chirp locking at 0.85, at both
symbol rates. Every one of those cells was in fact a `FrameDropped { reason:
"CRC mismatch" }`, which `cmd_rank.rs::classify` had no bucket for and swept
into `no_preamble`.

`crc_fail` and `frame_fail` outcomes now exist, and `no_preamble` means what it
says: no frame events at all. The `ok` counts are unchanged — this corrects how
the failures were labelled, not how many there were. It also sharpens the story,
because it means *every* simulated failure at either rate is a frame-body
failure after a successful preamble lock. Which is exactly the real
over-the-air symptom.

### How much of this is variable

The rate is a `FskConfig` / `FskPhy` parameter (`audible_at`, `ultrasonic_at`,
default `DEFAULT_SYMBOL_RATE = 50`), and it must divide 48 kHz exactly or
construction panics rather than silently running at a rate it does not report.
It is *not* part of the frame format — preamble, sync word and framing are
byte-identical at any rate — but both ends have to agree on it out of band,
exactly like the profile does. So:

- `gen_sim_corpus` takes the rate as its second argument.
- `modem rank` reads a per-capture `symbol_rate` in `manifest.toml`
  (defaults to 50), because the rate is baked into a recording the same way the
  `p` flag is. Replaying a 25 sym/s capture at 50 decodes nothing, pinned by
  `rank_at_the_wrong_symbol_rate_decodes_nothing`.
- Every `modem` subcommand that drives the modem takes `--symbol-rate`: the
  offline pair `tx-wav` / `rx-wav` and the live paths `send`, `recv`, `chat`.
  They share one `PhyArgs` block, so there is no rate-less way to build a PHY
  left in the CLI.
- `modem-ffi` exposes it as `FfiTransmitter.newAt` / `FfiReceiver.newAt`
  (symbols/second; `defaultSymbolRate()` returns 50, so a caller need not
  hard-code it), and both objects report the rate their PHY *actually* runs at
  through `symbolRate`, read off the PHY rather than the argument. Pinned by
  `modem-ffi/tests/symbol_rate.rs`, which decodes a 25 sym/s transmission
  through the surface and shows a default-rate receiver failing on the same
  buffer.
- **Still 50 in practice: the Android app.** `ModemViewModel` calls the
  rate-less `FfiTransmitter(profile, variants)` / `FfiReceiver(profile,
  variants)` constructors, so the phone transmits at 50 until the app offers the
  rate. That — plus a Pixel and a quiet room — is what a real Android→Mac
  capture at 25 sym/s now needs; the Rust side is no longer the blocker.

### So is Android→Mac fixed?

**No — it is worth a real capture, which is a different claim.** Everything in
this section is simulated, on a model whose preamble behaviour is known not to
match the real captures (~0.86 modelled versus 0.28–0.35 observed). A clean
24/24 in simulation is the strongest signal any remedy has produced for this
direction, and it is the reason to go and record, not a substitute for having
recorded. The honest status of Android→Mac remains: broken over the air, with
one promising and cheap candidate fix now identified and costed.

## Why we don't simply lower tone count further

We already cut from 8-FSK @ 100 sym/s @ 500 Hz spacing in 2.0–5.5 kHz to 8-FSK @ 50 sym/s @ 200 Hz spacing in 2.0–3.4 kHz. The next obvious step (4-FSK @ 50 sym/s @ 500 Hz spacing) would halve throughput again to ~75 bps and probably still hit the harmonic-distortion issue, because the problem is energy *leaking out* of our band into harmonics and back, not adjacent-tone confusion within the band.

**Correction, from the simulation work above:** the conclusion still holds for the tone count, but not for that reason. Widening tone spacing doesn't help because the failure isn't tone confusion of *any* kind — it's inter-symbol interference, and every tone suffers it equally. Cutting the *symbol rate*, on the other hand, does attack ISI — and it turned out to be the cheapest real gain found so far. See the section above.

## Known traps

- macOS AEC blocks single-Mac speaker→built-in-mic loopback. Use BlackHole (`docs/smoke-test.md`).
- Android's standard AudioSource applies AEC by default; the app uses AudioSource.UNPROCESSED to bypass it.
- USAGE_MEDIA + CONTENT_TYPE_SONIFICATION triggers Pixel's Dynamics Processing Effect, which compresses our FSK signal into bursts. AudioPlayer uses USAGE_UNKNOWN + CONTENT_TYPE_UNKNOWN + PERFORMANCE_MODE_LOW_LATENCY to dodge it.
