# Mac ↔ Android smoke test

## What works today

- **macOS↔macOS via [BlackHole 2ch](https://existential.audio/blackhole/) virtual loopback:** fully working for both `audible` and `ultrasonic` profiles. See `docs/smoke-test.md`.
- **WAV file roundtrip on either platform:** `modem tx-wav` produces a 48 kHz mono float WAV; `modem rx-wav` decodes it byte-identically. Pure DSP path, no audio hardware required.
- **Mac → Android over-the-air (audible):** ✅ decoded byte-identically with SHA-256 verification, with the phone mic ~20–30 cm from a MacBook speaker at ~75% volume in a quiet room.
- **Mac → Android over-the-air (ultrasonic):** ✅ decoded byte-identically. Inaudible to humans, no annoying warble.
- **Android → Mac over-the-air (audible):** ⚠️ **intermittent, but it does work.** 3 of the 6 real captures on this machine decode byte-identically with SHA-256 verification and *no DSP remedies at all*; 4 of 6 with the matched filter. Measured over the air, not modelled — see "What the six real captures say".
- **Android app:** Pixel 8 Pro / Android 16, builds and installs, captures via `AudioSource.UNPROCESSED`, plays via `AudioTrack` on the FAST low-latency path bypassing Android's Dynamics Processing Effect. The app shows an inline visualization (8-tone Goertzel bar meter + frame-event chip strip + watchdog readout) on every Send and Receive, mirroring the CLI's ratatui dashboard.

## Android → Mac — intermittent, not broken

> **Corrected 2026-07-30, by ranking the six real captures that were sitting in
> `captures/` unanalysed.** This section used to read "What does not work yet —
> Android → Mac", and to state that the demodulated sync word *always* had 1–4
> bit errors and the frame *always* exceeded RS's budget. Neither claim survives
> contact with the recordings:
>
> - 3 of the 6 real captures decode **byte-identically, SHA-256 verified, under
>   plain `baseline`** — no remedies at all. 4 of 6 with the matched filter.
> - **Not one of the 48 `(capture, variant)` cells failed on the sync word.**
> - The dominant real failure is `crc_fail` — RS reporting success and the CRC32
>   disagreeing — not `rs_fail`.
> - The real preamble correlates at **0.295–0.651** over these six, not
>   0.28–0.35. (0.295–0.629 as first published; the top of the range was itself
>   understated by the coarse correlation scan — see "The preamble gap is real
>   but half the size we said".)
>
> Numbers, command and output: "What the six real captures say" below. The
> honest status of Android→Mac is **an unreliable link, around half to two
> thirds of frames**, not a dead one. On six captures, so the rate itself is
> soft. Everything below about inter-symbol interference still stands as the
> explanation for the frames that *do* fail — it is now an explanation of a
> failure rate rather than of a total failure.

The Mac receiver locks onto the preamble and starts decoding the frame body; on
the captures that fail, the body exceeds what Reed–Solomon can put right. The
same was reported on both audible and ultrasonic profiles (only `audible` is
represented in the real corpus).

Original hypothesis: the Pixel 8 Pro's small built-in speaker produces enough harmonic distortion at our tone frequencies that the demodulated symbols pick up neighbour-tone energy. The Mac's built-in mic + macOS audio input processing doesn't help.

**That hypothesis does not survive simulation — see "What the DSP tuning pass actually bought" below.** Speaker distortion on its own, modelled well past what a real micro-speaker does, costs zero bit errors. What does reproduce this exact symptom is multi-symbol inter-symbol interference from ordinary room reverberation.

**Best candidate fix so far: halve the symbol rate to 25 sym/s.** In simulation that clears the whole corpus where the DSP tuning pass reached 75%, at the cost of half the bitrate (150 → 75 bps). Not yet confirmed over the air — see "What halving the symbol rate to 25 sym/s buys" below. **And it cannot be confirmed against the captures we have**: the rate is baked into the air signal, so those six recordings can only ever be decoded at the 50 sym/s they were sent at. See "Why 25 sym/s cannot be tested against these captures".

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

## What the six real captures say

**This is the only section on this page whose numbers came out of a microphone.**
Everything under "What the DSP tuning pass actually bought" and "What halving the
symbol rate to 25 sym/s buys" is simulated. This is not.

Six real Android→Mac recordings have been sitting in `captures/` since
2026-05-17, from the session that produced the original "does not work"
verdict. They are gitignored (`docs/capture-corpus.md` explains why) and they
had **never been run through `modem rank`** — even though the harness was
written the same morning, authored at 12:01 and merged at 15:26, so it existed
before the first of these recordings at 12:48 and was simply never pointed at
them. They were ranked for the first time on 2026-07-30. The result contradicts
what this page had been claiming for two months.

### The corpus

All six: Pixel 8 Pro transmitting, MacBook built-in mic recording, `audible`
profile, 22.0 s at 48 kHz mono f32 (4.2 MB each), payload `hello from Android`,
50 sym/s. `captures/manifest.toml` carries no `symbol_rate` field, which means
50 — correct for these, since they predate the rate being adjustable at all.

| Capture | TX side | Preamble peak | `baseline` | `+m` |
|---|---|---|---|---|
| `android-to-mac-001.wav` | rectangular, vol 22/25 | 0.295 | `rs_fail` | `rs_fail` |
| `android-to-mac-002-tx-pulse.wav` | pulse-shaped, vol 25/25 | 0.388 | `crc_fail` | **`ok`** |
| `android-to-mac-003-tx-base-vol25.wav` | rectangular, vol 25/25 | 0.451 | **`ok`** | **`ok`** |
| `android-to-mac-004-tx-base-vol25.wav` | rectangular, vol 25/25 | 0.629 | **`ok`** | **`ok`** |
| `android-to-mac-005-tx-pulse-vol25.wav` | pulse-shaped, vol 25/25 | 0.613 | `crc_fail` | `crc_fail` |
| `android-to-mac-006-tx-pulse-vol25.wav` | pulse-shaped, vol 25/25 | 0.498 | **`ok`** | **`ok`** |

`ok` means what `docs/capture-corpus.md` says it means: `StreamComplete`,
SHA-256 matched, decoded bytes equal to `hello from Android`. Confirmed outside
the harness too — `modem rx-wav captures/android-to-mac-003-tx-base-vol25.wav`
prints `hello from Android` on its own, and 002 does the same with
`--matched-filter`.

### The measurement

```bash
cargo build --release
./target/release/modem rank captures/
```

(Ranked on 2026-07-30 against a read-only copy of the six, to keep the
irreplaceable originals out of reach of the harness. Same files, verified by
SHA-256 before and after.)

```text
SUMMARY rate=50 variant=baseline ok=3/6 crc_fail=2 rs_fail=1
SUMMARY rate=50 variant=m        ok=4/6 crc_fail=1 rs_fail=1
SUMMARY rate=50 variant=m+t      ok=4/6 crc_fail=1 rs_fail=1
SUMMARY rate=50 variant=p        ok=3/6 crc_fail=2 rs_fail=1
SUMMARY rate=50 variant=p+m      ok=4/6 crc_fail=1 rs_fail=1
SUMMARY rate=50 variant=p+m+t    ok=4/6 crc_fail=1 rs_fail=1
SUMMARY rate=50 variant=p+t      ok=3/6 crc_fail=2 rs_fail=1
SUMMARY rate=50 variant=t        ok=3/6 crc_fail=2 rs_fail=1
```

### What that overturns

**Android→Mac is not a dead direction.** Half the real captures decode with no
remedies whatsoever. The claim this page made — preamble locks, sync word
*always* wrong by 1–4 bits, body *always* past the RS budget — was never true of
these files.

Nor can a later fix explain it away. `modem-core` and `modem-codec` have changed
exactly twice on the demodulation path since 2026-05-17, and neither change can
turn a failure into a decode: one added `goertzel_bank`, a visualization helper
that the receiver does not call, and the other is yesterday's symbol-rate
parameterization, which at rate 50 is a pure refactor — `SAMPLE_RATE / 50`
became `symbol_samples_for(50)`, the same 960 samples. **These captures would
have decoded on the day they were made.** The original verdict came from live
microphone sessions, and the recordings made alongside it were never checked
against it.

**No cell failed on the sync word.** Zero `sync_fail` and zero `no_preamble`
across all 48 cells. Every real failure is a frame-body failure after a good
preamble lock and a good sync word — which is exactly what the simulated corpus
also shows, and it is the one part of the old story that the real data confirms.

**`m` is the only remedy that does anything, and it is worth one capture.**
`t` changes not a single cell on any of the six. `p` is a no-op at RX exactly as
`docs/capture-corpus.md` predicts (`p` ≡ `baseline`, `p+m` ≡ `m`, `p+m+t` ≡
`m+t`, cell for cell). So on real data `p+m+t` is indistinguishable from plain
`m`, and the "best combination: `p+m+t`" conclusion reduces to "turn the matched
filter on".

**A strong preamble does not predict a decodable body.** Capture 005 has the
second-best chirp correlation of the six (0.613) and never decodes under any
variant; capture 003 locks at 0.451 and decodes under all eight. Whatever kills
005's body is not attacking the chirp.

### What it does *not* overturn

**The channel model's aggregate hit rate is closer to reality than expected.**
Real `baseline` is 3/6 (50%) against 14/24 (58%) simulated, and real best is 4/6
(67%) against 18/24 (75%) simulated. The failure *mix* lines up too: both are
dominated by `crc_fail`, with a minority of `rs_fail` and no preamble or sync
failures at all. With n=6 that agreement should not be leant on, but the model is
not the wild optimist this page implied — outside the preamble.

**The preamble gap is real but half the size we said.** Measured peak
correlation over the six is 0.295–0.651 (mean ≈ 0.55) against ~0.86 modelled.
The 0.28–0.35 figure this page has quoted throughout came from the bottom of
that range. Reproduced with `modem_core::preamble::detect_preamble` over each
WAV.

> **Corrected again 2026-07-30.** This paragraph used to add: "Part of the
> discrepancy is that the streaming receiver accepts the *first* window over
> 0.25 rather than the peak, so what it locks on can score below the true peak —
> 0.259 versus 0.295 on capture 001, a 76-sample-early lock." **The receiver
> does not do that.** `detect_preamble` scans the entire searchable window for
> the maximum and returns only that; `Receiver::step` then applies the 0.25
> threshold to the peak. The 0.259/0.295/76-samples figures are real numbers —
> 0.2593 at offset 116384 is genuinely the first stride-4 window over 0.25 on
> capture 001, and the peak is genuinely 0.2947 at 116460 — but they were
> misattributed: the receiver locks on 116460, not 116384. Verified on all six
> captures at whole-file, 4800-sample and 1024-sample chunking: the lock offset
> and score equal the peak offset and score in every case. It cannot differ,
> structurally — `Receiver::step` will not search an offset until
> `payload_samples` (13.9 s at 50 sym/s) of audio *after* it is buffered, so no
> candidate is ever judged on a partially-arrived correlation ramp.
>
> Forcing the misbehaviour costs nothing either. Chunking the feed to cut
> exactly at each capture's first-crossing offset (so the peak is not yet
> searchable) makes the receiver lock 76/8/12/0/0/0 samples early on captures
> 001–006 — and every one of the six decodes to the same outcome as before
> (`rs_fail`, `crc_fail`, `ok`, `ok`, `crc_fail`, `ok`). A first-crossing lock of
> that size is not a mechanism in the preamble gap, and fixing one would not
> have bought a single cell.
>
> What *was* mechanical, and is now fixed: the scan strides 4 samples, and a
> 1.5–6 kHz chirp's correlation peak is ~10 samples wide with fine structure at
> the ~3.75 kHz centre frequency, so the best stride-4 grid point can be up to 3
> samples off true alignment **and score far below the real peak**. Refining the
> coarse winner over ±3 samples at full resolution moves four of the six
> captures onto a better offset and raises the measured range from 0.295–0.629
> (mean 0.48) to 0.295–0.651 (mean 0.55): 001 0.2947 (unchanged), 002 0.3877 →
> 0.5794, 003 0.4514 → 0.5918, 004 0.6290 → 0.6510, 005 0.6130 (unchanged), 006
> 0.4984 → 0.5914. So roughly a fifth of the gap this page has been calling
> mysterious was the measurement, not the channel. **It changes no decode
> outcome:** the rank output over the six captures is cell-for-cell identical
> before and after (`baseline` 3/6, `m` 4/6), which is expected — 3 samples is
> 0.3% of a 960-sample symbol, far inside what the demodulator tolerates.

> **Added 2026-07-31: the scan is now bounded, and that is a behaviour change.**
> `Receiver::step` used to scan every stride-4 offset in whatever was buffered —
> `buffer.len() - payload_samples` offsets, which is 126 s of audio when a
> ten-frame file is pushed in one call, re-run whole after every frame. It now
> scans at most `LOOK_BACK_MILLIS` (then 1 s, 12 000 windows; now 250 ms — see
> the note below) per scan and slides,
> retiring the offsets it has rejected, so a stream is swept once instead of
> once per frame. Measured on this corpus, the lock does not move: all six
> captures return the same offset *and* the same score as the unbounded scan
> (001 116460/0.2947, 002 92113/0.5794, 003 91147/0.5918, 004 94573/0.6510,
> 005 96620/0.6130, 006 93845/0.5914), and the rank output is byte-identical.
> The reason it cannot move here is measurable: the strongest correlation
> anywhere *ahead* of the chirp is 0.0764 (capture 003; the other five are
> 0.0144–0.0377), against a 0.25 accept threshold, so no earlier window can win.
> What the bound gives up is the ability of a later, stronger chirp to override
> an acceptable one more than a second earlier — deliberately, because at 13.9 s
> per frame the unbounded comparison could skip a decodable frame for a louder
> one further down the recording.
>
> One trap worth recording, because it is the same class of bug as the stride-4
> one above: the slide must advance by a **multiple of the stride**.
> `detect_preamble` strides from index 0 of the slice it is handed, so an
> unaligned advance re-phases the grid and scores a different set of offsets.
> Advancing by `bound + 1` instead moved three of the six locks off their peaks —
> 001 116460/0.2947 → 116385/0.2697, 003 91147/0.5918 → 91136/0.3532, 005
> 96620/0.6130 → 96637/0.4360 — which the rank output survived (same eight
> outcomes) but which throws away a third of capture 001's margin over the
> threshold for nothing.

> **Added 2026-07-31, later: the bound is 250 ms, and the corpus is what says
> so.** The window length is the whole remaining cost of a scan, because the scan
> searches all of it for a maximum even when the chirp sits at offset 0 — which
> it does for every frame after the first. Shortening it from 1 s to 250 ms
> divides that cost by four and moves nothing: **all six captures lock on the
> same offset and the same score, and produce the same decode outcome** (001
> 116460/0.2947 `rs_fail`, 002 92113/0.5794 `crc_fail`, 003 91147/0.5918 `ok`,
> 004 94573/0.6510 `ok`, 005 96620/0.6130 `crc_fail`, 006 93845/0.5914 `ok`).
> Capture 001 keeps its whole 0.0447 of margin over the 0.25 threshold.
>
> **The evidence for 250 ms, measured rather than assumed.** The bound exists so
> that a smeared early arrival of a chirp is still compared against the same
> chirp arriving by a later path, so the question it answers is: *in these rooms,
> what is the latest-arriving energy that still correlates?* Reproduce with
>
> ```bash
> cargo run --release -p modem-codec --example preamble_probe -- captures
> ```
>
> which scans each capture at full resolution and reports, per capture, the lock
> (through `Receiver::locks()`, in recording coordinates), the unbounded peak,
> the strongest correlation ahead of the chirp, the extent of the above-threshold
> band, and the strongest delayed correlation in each delay band after the peak.
> Over the six:
>
> | delay after the peak | strongest correlation across the six |
> |---|---|
> | inside the band (0–0.40 ms) | 0.2947–0.6510 — the arrival itself |
> | 5–10 ms | 0.085–0.137 |
> | 10–20 ms | 0.069–0.102 |
> | 20–40 ms | 0.037–0.089 |
> | 40–80 ms | 0.093–0.112 |
> | 80–160 ms | 0.097–0.121 |
> | 160–320 ms | 0.109–0.141 |
> | 320–640 ms | 0.125–0.145 |
> | 640 ms–1 s | 0.101–0.166 |
>
> The above-threshold band around each chirp is **29–91 samples wide (0.6–1.9
> ms)** and its last offset is at most 19 samples (0.40 ms) past the peak. Past
> that band, nothing re-correlates above **0.166** at any delay out to a full
> second, against a 0.25 accept threshold. Reverb is indistinguishable from the
> floor by 5 ms; the slow rise at the long end is the payload's own weak
> correlation against the chirp template, not the room. So no delayed arrival in
> this corpus ever becomes a candidate, and 250 ms — three chirp lengths, 86 m of
> extra path — is far more look-back than these recordings can justify needing.
>
> **What is still only empirical, at 250 ms and at 1 s alike** — closed on
> 2026-08-01, see the note below, including why the remedy named here is not the
> one that worked. Successive
> windows abut, so there are seams, and a seam falling strictly inside an
> above-threshold band splits it: the earlier window sees only the band's leading
> edge, which on capture 001 clears the threshold on its own (0.2593 at 116384),
> and locks on a shoulder — the same degradation the unaligned-advance trap above
> produced. 12 000 divides 48 000, so every seam of the old bound is still a
> seam; a shorter window cannot fix a split, only add four times as many places
> one could occur. Measured: **no seam falls inside any of the six bands at
> either bound**, and the closest approach is capture 005, whose band starts 609
> samples (12.7 ms) after the seam at 96 000 — a seam both bounds share. With
> bands ≤91 samples and seams every 12 000 the residual exposure is under 1% per
> capture. Making it structural means windows that overlap by a chirp length
> instead of abutting, which costs about a third of what this change bought;
> that is a separate piece of work, not a hidden rider on this one.

> **Added 2026-08-01: the seam is structural now, and this corpus cannot see the
> difference.** `Receiver::step` no longer locks on an above-threshold candidate
> that sits within one chirp length (3 840 samples) of the trailing edge of a
> look-back window. It retires up to that candidate — rounded *down* to a whole
> `SCAN_STRIDE`, for the reason the unaligned-advance trap above gives — and
> scans again, so the whole band and its peak are weighed in one window. The
> reach is a chirp length because past `tn` of shift the template and the chirp
> do not overlap at all, so no band can start more than a chirp length before its
> peak; that is why the guarantee does not depend on these six recordings, whose
> bands are only 29–91 samples wide.
>
> **The gate cannot prove the fix, and saying so is the point.** No seam falls
> inside any of the six bands, so all six locks are *expected* to be
> bit-identical before and after, and identical locks only prove nothing
> regressed. They are, at full resolution and with the same scores: 001
> 116460/0.2947, 002 92113/0.5794, 003 91147/0.5918, 004 94573/0.6510, 005
> 96620/0.6130, 006 93845/0.5914, decode outcomes unchanged
> (`rs_fail`/`crc_fail`/`ok`/`ok`/`crc_fail`/`ok`). Three of the six do exercise
> the new path — 001, 004 and 006 lock 3 540, 1 427 and 2 155 samples before a
> seam, all inside a chirp length, so each is deferred once and re-locks on the
> same offset and the same score. That is the deferral being lossless on real
> audio, which is the most this corpus can say.
>
> **What proves it is a synthetic bite test**,
> `a_seam_inside_the_band_does_not_cost_the_peak` in `modem-codec/src/rx.rs`,
> which reproduces capture 001's band
> shape across a seam: a weak smeared arrival at 11 972, a stronger one at
> 12 060, the seam at 12 000 between them, a whole frame buffered behind both so
> the trailing edge is the look-back bound and not the end of the audio. On trunk
> the first window sees only the leading edge, finds 0.3673 there, and locks —
> 88 samples before the chirp, with the 0.8646 peak one window away. With the
> deferral it locks on the peak.
>
> **Two variants of "windows that overlap by a chirp length" were tried, and the
> one the note above named does not work.** Retiring `bounded_end + 1 - tn` so
> successive windows re-scan the last chirp length leaves the bite test failing:
> the premature lock happens in the window that first sees the leading edge, and
> the later window that would have seen the whole band is never reached, because
> the receiver has already locked. Handing the detector a slice that reaches `tn`
> *past* the trailing edge does work, but it widens every comparison from 250 ms
> to 330 ms — the bound chosen deliberately one note above — and pays for it on
> every window whether or not anything is near an edge.
>
> **Count the correlations rather than the seconds; the seconds on this machine
> cannot resolve it.** Looking ahead scans 3 961 stride-4 offsets per window
> instead of 3 001, a permanent +32%. The deferral scans one extra partial window
> per candidate that clears the threshold within a chirp length of a seam, and
> nothing otherwise: **the whole workspace suite defers twice**, once in the bite
> test that exists to force it and once in
> `back_to_back_transmissions_decode_cleanly`, which passes unchanged. The six
> captures defer three times, all lossless. Timings, for the record and with
> their noise: `cargo test --workspace` 47.4 s / 42.9 s on trunk against 46.1 s /
> 42.3 s with the deferral; `cargo test --test stream_roundtrip` 21.49 s trunk,
> 21.17 s retiring less, 21.36 s looking ahead, 14.16 s and 20.02 s on two
> consecutive runs of the *same* deferral binary. That last pair is the point —
> this machine had other work on it, and a 40% spread on identical code means no
> timing here can price a 32% change in correlation work. The "about a third of
> what this bought" the note above priced the fix at was never measured either.
>
> **One hazard is left standing on purpose.** When a window's trailing edge is
> the end of buffered audio rather than a look-back seam, a candidate near it can
> still be a split-band shoulder. Deferring there would not rescan, it would
> *wait*: up to 80 ms of added latency per near-edge candidate on a live stream,
> and on a whole-file push — no more audio coming, no flush — a dropped frame
> whenever the preamble sits within a chirp length of the last scannable offset.
> A live loss to close a latent risk is the wrong trade. The exposure is the last
> 3 840 offsets of whatever has been pushed, and it closes as the buffer fills.

## Why 25 sym/s cannot be tested against these captures

Ranking the real corpus "at both rates" is not a thing that can be done, and a
table claiming it would be worthless. Two independent reasons, both measured:

**1. The rate is baked into the air.** These recordings contain a 50 sym/s
signal. A 25 sym/s receiver reading them is integrating over 1920-sample windows
that each straddle two transmitted symbols — it is not decoding a slow
transmission, it is decoding the wrong thing. Pinned in the abstract by
`rank_at_the_wrong_symbol_rate_decodes_nothing`, and directly on real data:

```bash
./target/release/modem rx-wav --symbol-rate 25 captures/android-to-mac-003-tx-base-vol25.wav
# Error: no complete stream received      # decodes fine at the default 50
```

**2. The recordings are too short to hold a 25 sym/s frame at all.** One frame
is 261 bytes over the air (2 sync + 259), i.e. 696 symbols; at 1920 samples per
symbol that is 1,336,320 samples, plus 3,840 for the chirp — **27.92 s**. These
captures are 22.0 s (1,056,003 samples). `Receiver::step` will not even begin
searching for a preamble until the buffer holds `preamble + payload_samples`, so
a 25 sym/s receiver on a 22 s file returns before looking at anything. That is
why relabelling the manifest `symbol_rate = 25` produces this, and why it says
nothing about 25 sym/s:

```text
SUMMARY rate=25 variant=baseline ok=0/6 no_preamble=6      # ... and the same for all 8 variants
```

`no_preamble=6` there is a file-length artifact, not a DSP result. It is a
useful trap to know about: **a capture shorter than one frame reads as a signal
failure.**

**So the simulated 25 sym/s result stands entirely unconfirmed, and this corpus
cannot confirm it.** What it needs is a new over-the-air recording, which needs
three things: the Android app to offer the rate (`ModemViewModel` still calls the
rate-less `FfiTransmitter` / `FfiReceiver` constructors), a phone and a quiet
room, and **a recording at least ~32 s long** — the 22 s in the
`docs/capture-corpus.md` recipe would truncate a 25 sym/s frame.

One thing the real data does say about the *case* for 25 sym/s: the argument for
it is unchanged in mechanism and weaker in urgency. Unchanged, because every
real failure is a frame-body failure after a clean lock — ISI is what 25 sym/s
attacks, and the real corpus is consistent with ISI being the limiter. Weaker,
because the trade is no longer "a link that works at half speed versus one that
doesn't". It is "an unreliable link at 150 bps versus an unmeasured one at
75 bps". A second thing worth measuring before spending a recording session on
the rate: **more captures at 50**, because a 3-of-6 success rate on six files is
barely a number.

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
- **Simulated (not over the air):** every result in this section. It was ranked
  only against a synthesised corpus. (This used to read "there are no real
  Android→Mac captures in `captures/` on this machine" — **that was false when
  written.** Six had been there since 2026-05-17; they were simply never ranked.
  They have been now: see "What the six real captures say", which supersedes this
  bullet.) The corpus is synthesised by
  `cargo run --release -p modem-codec --example gen_sim_corpus -- captures/simulated`,
  which pushes generated transmissions through the modelled channel.

The channel model is physically parameterised, not fitted to make a variant look
good — the speaker preset is pinned to a measured ~9.5% THD, in the 5–15% band
micro-speakers are quoted at, and asserted by
`speaker_preset_thd_is_physically_plausible`. But it is still a model, and it
does not match the real capture in every respect. In particular it gets the
preamble wrong, and in the *optimistic* direction: the modelled chirp correlates
at ~0.86 at the nominal room and still ~0.85 at the `live` tier — it never
struggles at all. The six real Android→Mac captures peak at **0.295–0.651**
(mean ≈ 0.55) — degraded but locking — which the model never reproduces.
Something in the real path attacks the chirp harder than reverberation does, and
that part is still unexplained. (This paragraph used to quote the real range as
0.28–0.35, i.e. only the bottom of it, and then 0.295–0.629, which was measured
on a stride-4 scan too coarse to see the peak of a chirp correlation; measured
values per capture are in "What the six real captures say". The gap is genuine
and roughly half as wide as first stated.)

(An earlier version of this paragraph said the modelled chirp falls under the
0.25 threshold at the `live` tier, and attributed the harness's `no_preamble`
counts to that. Both halves were wrong; see "A correction to the harness,
found on the way" below.)

**The conclusion that `p+m+t` is the best combination has now been checked
against the real recordings, and it does not hold in the form stated.** On the
six real captures `t` changes no cell at all and `p` is a no-op at RX, so
`p+m+t` is cell-for-cell identical to plain `m` — the matched filter is the
whole of the gain, and it is worth one capture out of six (3/6 → 4/6). See "What
the six real captures say". The simulated ranking is not wrong about `m`
carrying the weight; it overstated what `t` adds.

### Reproducing all of the above

```bash
cargo test --release             # includes modem-codec/tests/android_to_mac.rs
cargo run --release -p modem-codec --example gen_sim_corpus -- captures/simulated
cargo build --release && ./target/release/modem rank captures/simulated
```

Use `--release`. In debug the `stream_roundtrip` proptest used to take tens of
minutes (2706 s when the preamble scan was unbounded, of a ~47-minute suite);
with the bounded scan it is about 50 s, and the whole workspace suite about 80 s.

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

**No, and 25 sym/s is not what fixed the part of it that works.** Everything in
this section is simulated, on a model whose preamble behaviour is known not to
match the real captures (~0.86 modelled versus 0.295–0.629 measured). A clean
24/24 in simulation is the strongest signal any remedy has produced for this
direction, and it is the reason to go and record, not a substitute for having
recorded.

Two separate updates to the honest status, both from the real captures:

- Android→Mac is **intermittent rather than broken** at the rate we already ship:
  3/6 real captures at `baseline`, 4/6 with the matched filter. That has nothing
  to do with 25 sym/s and was true before any of this work.
- 25 sym/s **cannot be tested against the corpus on disk**, for two measured
  reasons — see "Why 25 sym/s cannot be tested against these captures". It
  remains a simulation-only result, and confirming it needs a fresh recording
  from a phone that can transmit at 25 sym/s, at least ~32 s long.

## Why we don't simply lower tone count further

We already cut from 8-FSK @ 100 sym/s @ 500 Hz spacing in 2.0–5.5 kHz to 8-FSK @ 50 sym/s @ 200 Hz spacing in 2.0–3.4 kHz. The next obvious step (4-FSK @ 50 sym/s @ 500 Hz spacing) would halve throughput again to ~75 bps and probably still hit the harmonic-distortion issue, because the problem is energy *leaking out* of our band into harmonics and back, not adjacent-tone confusion within the band.

**Correction, from the simulation work above:** the conclusion still holds for the tone count, but not for that reason. Widening tone spacing doesn't help because the failure isn't tone confusion of *any* kind — it's inter-symbol interference, and every tone suffers it equally. Cutting the *symbol rate*, on the other hand, does attack ISI — and it turned out to be the cheapest real gain found so far. See the section above.

## Known traps

- macOS AEC blocks single-Mac speaker→built-in-mic loopback. Use BlackHole (`docs/smoke-test.md`).
- Android's standard AudioSource applies AEC by default; the app uses AudioSource.UNPROCESSED to bypass it.
- USAGE_MEDIA + CONTENT_TYPE_SONIFICATION triggers Pixel's Dynamics Processing Effect, which compresses our FSK signal into bursts. AudioPlayer uses USAGE_UNKNOWN + CONTENT_TYPE_UNKNOWN + PERFORMANCE_MODE_LOW_LATENCY to dodge it.
