# Modem Visualization UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add live visualization (tone bars + frame timeline + TX progress) to the Android Compose app and the Rust CLI, so the user can see what the modem is hearing/sending instead of just status text.

**Architecture:** Pure observability layer — no DSP changes, no new FFI events. Both apps run Goertzel filters at the 8 known FSK tone frequencies on the audio buffers they already have, and surface the existing `FrameOk`/`FrameDropped`/`StreamComplete` event stream as a scrolling timeline of chips. Android uses Compose; the CLI uses ratatui + crossterm behind a TTY check (with `--plain` to keep current text output for scripts).

**Tech Stack:** Rust (modem-core, modem-cli with new ratatui + crossterm deps), Kotlin/Compose (modem Android app, existing kotlinx-coroutines, no new third-party deps).

**Spec:** `docs/superpowers/specs/2026-05-17-modem-visualization-design.md`

---

## File Structure

### Rust (shared + CLI)
- Modify `modem-core/src/fsk.rs` — expose `goertzel_bank` publicly (lift the existing private `goertzel_mag2`).
- Create `modem-cli/src/reporter.rs` — `Reporter` trait + `PlainReporter` impl.
- Create `modem-cli/src/tui.rs` — `TuiReporter` (ratatui) implementation.
- Modify `modem-cli/src/cmd_recv.rs` — drive output through `Reporter`.
- Modify `modem-cli/src/cmd_send.rs` — drive output through `Reporter`.
- Modify `modem-cli/src/main.rs` — add `--tui`/`--plain` flags, TTY autodetect.
- Modify `modem-cli/Cargo.toml` — add `ratatui` and `crossterm`.

### Android (Compose)
- Create `android/app/src/main/java/se/karleklund/modem/viz/Goertzel.kt` — Kotlin port.
- Create `android/app/src/main/java/se/karleklund/modem/viz/VizEngine.kt` — state machine + `VizState`.
- Create `android/app/src/main/java/se/karleklund/modem/viz/ToneBars.kt` — Compose component.
- Create `android/app/src/main/java/se/karleklund/modem/viz/FrameTimeline.kt` — Compose component.
- Create `android/app/src/main/java/se/karleklund/modem/viz/ListeningIndicator.kt` — Compose component.
- Create `android/app/src/main/java/se/karleklund/modem/viz/TxProgress.kt` — Compose component.
- Create `android/app/src/main/java/se/karleklund/modem/viz/ResultPulse.kt` — success/failure flash.
- Create `android/app/src/test/java/se/karleklund/modem/viz/GoertzelTest.kt` — unit test.
- Create `android/app/src/test/java/se/karleklund/modem/viz/VizEngineTest.kt` — unit test.
- Modify `android/app/src/main/java/se/karleklund/modem/ModemViewModel.kt` — own a `VizEngine`, feed it chunks/events.
- Modify `android/app/src/main/java/se/karleklund/modem/ui/ModemScreen.kt` — host new components per state.
- Modify `android/app/build.gradle.kts` — add JUnit test deps if missing.

### Repo notes
- Commits go through `jj` (this is a Jujutsu repo). Each task ends with `jj desc -m "…" && jj new` rather than `git commit`.

---

## Phase 1: Shared groundwork (Rust)

### Task 1: Expose `goertzel_bank` from `modem-core`

A small public helper that the CLI's `TuiReporter` can call to compute per-tone magnitudes for a buffer of samples. Lift from the existing private `goertzel_mag2`. Android does NOT use this (it reimplements Goertzel in Kotlin to avoid an FFI round trip per UI tick), but Rust callers should share one implementation.

**Files:**
- Modify: `modem-core/src/fsk.rs` (around lines 269–282)
- Test: `modem-core/src/fsk.rs` (new `#[test]` in the existing `mod tests`)

- [ ] **Step 1: Write the failing test**

Add to the existing `#[cfg(test)] mod tests` block at the bottom of `modem-core/src/fsk.rs`:

```rust
    #[test]
    fn goertzel_bank_picks_the_right_tone() {
        // Pure 2200 Hz sine, 20 ms at 48 kHz = 960 samples.
        let cfg = FskConfig::audible();
        let n = cfg.symbol_samples;
        let f0 = cfg.tone_freqs[1]; // 2200 Hz
        let mut samples = vec![0f32; n];
        for k in 0..n {
            let t = k as f32 / cfg.sample_rate as f32;
            samples[k] = (2.0 * std::f32::consts::PI * f0 * t).sin();
        }
        let mags = goertzel_bank(&samples, &cfg.tone_freqs, cfg.sample_rate);
        assert_eq!(mags.len(), N_TONES);
        // The 2200 Hz bin must dominate by >10× over every other bin.
        let target = mags[1];
        for (i, &m) in mags.iter().enumerate() {
            if i == 1 { continue; }
            assert!(target > 10.0 * m, "tone {} not dominant: {:?}", i, mags);
        }
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p modem-core goertzel_bank_picks_the_right_tone -- --nocapture`
Expected: FAIL with `cannot find function 'goertzel_bank' in this scope`.

- [ ] **Step 3: Add the public function**

In `modem-core/src/fsk.rs`, immediately after the existing private `fn goertzel_mag2` (around line 282), add:

```rust
/// Compute Goertzel magnitude² for each frequency in `freqs` against the
/// given buffer of f32 samples. Returns one magnitude per frequency, in the
/// same order. Used by visualization layers that want per-tone energy
/// without running the full demod state machine.
pub fn goertzel_bank(samples: &[f32], freqs: &[f32], sample_rate: u32) -> Vec<f32> {
    freqs.iter().map(|&f| goertzel_mag2(samples, f, sample_rate)).collect()
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p modem-core goertzel_bank_picks_the_right_tone -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Run the full test suite to confirm no regressions**

Run: `cargo test --workspace`
Expected: all existing tests pass.

- [ ] **Step 6: Commit**

```bash
jj desc -m "Expose goertzel_bank from modem-core for visualization callers"
jj new
```

---

## Phase 2: Android — viz primitives

### Task 2: Kotlin Goertzel port + unit test

Implement Goertzel in Kotlin so the Android UI can compute per-tone energies on each incoming/outgoing chunk without crossing the FFI boundary. The reference fixture matches the Rust test from Task 1.

**Files:**
- Create: `android/app/src/main/java/se/karleklund/modem/viz/Goertzel.kt`
- Create: `android/app/src/test/java/se/karleklund/modem/viz/GoertzelTest.kt`
- Modify: `android/app/build.gradle.kts` (add JUnit test deps if absent)

- [ ] **Step 1: Ensure JVM test deps are wired**

Check `android/app/build.gradle.kts`. If the `dependencies { }` block does not already include `testImplementation("junit:junit:4.13.2")`, append:

```kotlin
    testImplementation("junit:junit:4.13.2")
```

inside the `dependencies { }` block (after the existing `debugImplementation` line).

- [ ] **Step 2: Write the failing test**

Create `android/app/src/test/java/se/karleklund/modem/viz/GoertzelTest.kt`:

```kotlin
package se.karleklund.modem.viz

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.math.PI
import kotlin.math.sin

class GoertzelTest {
    @Test
    fun bankPicksTheRightTone() {
        val sampleRate = 48_000
        val n = sampleRate / 50 // 20 ms, matches symbol_samples
        val tones = FloatArray(8) { 2000f + it * 200f }
        val f0 = tones[1] // 2200 Hz
        val samples = FloatArray(n) { k ->
            sin(2.0 * PI * f0 * k / sampleRate).toFloat()
        }
        val mags = goertzelBank(samples, tones, sampleRate)
        assertEquals(8, mags.size)
        val target = mags[1]
        for (i in mags.indices) {
            if (i == 1) continue
            assertTrue("tone $i not dominant: ${mags.toList()}", target > 10f * mags[i])
        }
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cd android && ./gradlew :app:testDebugUnitTest --tests se.karleklund.modem.viz.GoertzelTest`
Expected: FAIL — unresolved reference `goertzelBank`.

- [ ] **Step 4: Implement Goertzel**

Create `android/app/src/main/java/se/karleklund/modem/viz/Goertzel.kt`:

```kotlin
package se.karleklund.modem.viz

import kotlin.math.PI
import kotlin.math.cos
import kotlin.math.floor

/**
 * Goertzel single-bin magnitude² for one frequency in `samples`. Matches the
 * reference impl in `modem-core::fsk::goertzel_mag2`.
 */
fun goertzelMag2(samples: FloatArray, f: Float, sampleRate: Int): Float {
    val n = samples.size.toFloat()
    val k = floor(0.5f + n * f / sampleRate.toFloat())
    val w = 2f * PI.toFloat() * k / n
    val coeff = 2f * cos(w)
    var s1 = 0f
    var s2 = 0f
    for (x in samples) {
        val s0 = x + coeff * s1 - s2
        s2 = s1
        s1 = s0
    }
    return s1 * s1 + s2 * s2 - coeff * s1 * s2
}

/**
 * Compute Goertzel magnitude² for each frequency in `freqs` against the given
 * buffer. Returns one magnitude per frequency, in the same order.
 */
fun goertzelBank(samples: FloatArray, freqs: FloatArray, sampleRate: Int): FloatArray =
    FloatArray(freqs.size) { goertzelMag2(samples, freqs[it], sampleRate) }
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cd android && ./gradlew :app:testDebugUnitTest --tests se.karleklund.modem.viz.GoertzelTest`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
jj desc -m "Add Kotlin Goertzel port for Android visualization"
jj new
```

---

### Task 3: `VizEngine` + unit test

State container that the receive/send loops feed audio chunks and events into. Exposes a `StateFlow<VizState>` for the UI to render. Tracks:

- `tonesDb: FloatArray(8)` — smoothed per-tone energy in dB.
- `rms: Float` — input level for the chunk.
- `timeline: List<FrameMark>` — bounded ring of past `FrameOk`/`FrameDropped` events.
- `searching: Boolean` — true until any frame event has been seen since the last reset.
- `watchdogSecondsLeft: Int?` — 10 → 0 once `searching` has flipped false; null while still searching or after watchdog reset.
- `totalOk: Int`, `totalDropped: Int` — running counts.

**Files:**
- Create: `android/app/src/main/java/se/karleklund/modem/viz/VizEngine.kt`
- Create: `android/app/src/test/java/se/karleklund/modem/viz/VizEngineTest.kt`

- [ ] **Step 1: Write the failing test**

Create `android/app/src/test/java/se/karleklund/modem/viz/VizEngineTest.kt`:

```kotlin
package se.karleklund.modem.viz

import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.modem_ffi.FfiFrameEvent

class VizEngineTest {
    private val audibleTones = FloatArray(8) { 2000f + it * 200f }

    @Test
    fun startsInSearchingStateWithNoTimeline() = runTest {
        val eng = VizEngine(audibleTones, sampleRate = 48_000, clockMs = { 0L })
        val s = eng.state.value
        assertTrue(s.searching)
        assertEquals(0, s.timeline.size)
        assertEquals(0, s.totalOk)
        assertEquals(0, s.totalDropped)
        assertNull(s.watchdogSecondsLeft)
    }

    @Test
    fun frameOkLeavesSearchingAndAdvancesCounters() = runTest {
        val eng = VizEngine(audibleTones, sampleRate = 48_000, clockMs = { 0L })
        eng.pushEvent(FfiFrameEvent.FrameOk(seq = 0u, bytes = byteArrayOf(1, 2, 3)))
        val s = eng.state.value
        assertFalse(s.searching)
        assertEquals(1, s.timeline.size)
        assertEquals(FrameStatus.Ok, s.timeline.last().status)
        assertEquals(1, s.totalOk)
    }

    @Test
    fun frameDroppedRecordsReason() = runTest {
        val eng = VizEngine(audibleTones, sampleRate = 48_000, clockMs = { 0L })
        eng.pushEvent(FfiFrameEvent.FrameDropped(seq = 2u, reason = "RS uncorrectable"))
        val s = eng.state.value
        assertEquals(FrameStatus.Dropped, s.timeline.last().status)
        assertEquals("RS uncorrectable", s.timeline.last().reason)
        assertEquals(1, s.totalDropped)
    }

    @Test
    fun resetClearsTimelineAndReturnsToSearching() = runTest {
        val eng = VizEngine(audibleTones, sampleRate = 48_000, clockMs = { 0L })
        eng.pushEvent(FfiFrameEvent.FrameOk(seq = 0u, bytes = byteArrayOf()))
        eng.reset()
        val s = eng.state.value
        assertTrue(s.searching)
        assertEquals(0, s.timeline.size)
        assertEquals(0, s.totalOk)
        assertEquals(0, s.totalDropped)
    }

    @Test
    fun timelineIsBoundedTo64Entries() = runTest {
        val eng = VizEngine(audibleTones, sampleRate = 48_000, clockMs = { 0L })
        repeat(100) {
            eng.pushEvent(FfiFrameEvent.FrameOk(seq = it.toUInt(), bytes = byteArrayOf()))
        }
        assertEquals(64, eng.state.value.timeline.size)
        // Oldest should have been evicted: last entry has seq 99
        assertEquals(99u, eng.state.value.timeline.last().seq)
    }

    @Test
    fun watchdogCountsDownFromTenAfterFirstFrame() = runTest {
        var t = 0L
        val eng = VizEngine(audibleTones, sampleRate = 48_000, clockMs = { t })
        eng.pushEvent(FfiFrameEvent.FrameOk(seq = 0u, bytes = byteArrayOf()))
        assertEquals(10, eng.state.value.watchdogSecondsLeft)
        t = 3_000L
        eng.tickWatchdog()
        assertEquals(7, eng.state.value.watchdogSecondsLeft)
        t = 11_000L
        eng.tickWatchdog()
        assertEquals(0, eng.state.value.watchdogSecondsLeft)
    }

    @Test
    fun pushChunkUpdatesRmsAndTones() = runTest {
        val sr = 48_000
        val n = sr / 50
        val chunk = FloatArray(n) { k ->
            kotlin.math.sin(2.0 * kotlin.math.PI * audibleTones[3] * k / sr).toFloat()
        }
        val eng = VizEngine(audibleTones, sampleRate = sr, clockMs = { 0L })
        eng.pushChunk(chunk)
        val s = eng.state.value
        assertTrue(s.rms > 0.5f)
        // Tone index 3 dominates after a single chunk.
        val target = s.tonesDb[3]
        for (i in s.tonesDb.indices) {
            if (i == 3) continue
            assertTrue("tone $i not dominated", target > s.tonesDb[i] + 5f)
        }
    }
}
```

Append to `android/app/build.gradle.kts` `dependencies` block if missing:

```kotlin
    testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.8.1")
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd android && ./gradlew :app:testDebugUnitTest --tests se.karleklund.modem.viz.VizEngineTest`
Expected: FAIL — `VizEngine`, `FrameStatus`, `FrameMark` unresolved.

- [ ] **Step 3: Implement `VizEngine`**

Create `android/app/src/main/java/se/karleklund/modem/viz/VizEngine.kt`:

```kotlin
package se.karleklund.modem.viz

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlin.math.log10
import kotlin.math.max
import kotlin.math.sqrt
import uniffi.modem_ffi.FfiFrameEvent

enum class FrameStatus { Ok, Dropped }

data class FrameMark(
    val seq: UInt,
    val status: FrameStatus,
    val reason: String?,
    val tMs: Long,
)

data class VizState(
    val tonesDb: FloatArray = FloatArray(8),
    val rms: Float = 0f,
    val timeline: List<FrameMark> = emptyList(),
    val searching: Boolean = true,
    val watchdogSecondsLeft: Int? = null,
    val totalOk: Int = 0,
    val totalDropped: Int = 0,
) {
    // Equals/hashCode generated by data class is correct for our purposes;
    // arrays are compared by reference, but we only emit fresh arrays on each
    // pushChunk so reference inequality correctly signals "new frame to draw."
}

private const val MAX_TIMELINE = 64
private const val WATCHDOG_TOTAL_MS = 10_000L
private const val TONE_EMA_ALPHA = 0.4f
private const val DB_FLOOR = -80f

class VizEngine(
    private val toneFreqs: FloatArray,
    private val sampleRate: Int,
    private val clockMs: () -> Long = { System.currentTimeMillis() },
) {
    private val _state = MutableStateFlow(VizState())
    val state: StateFlow<VizState> = _state.asStateFlow()

    private var lastProgressMs: Long = 0L

    fun reset() {
        lastProgressMs = 0L
        _state.value = VizState()
    }

    fun pushChunk(samples: FloatArray) {
        if (samples.isEmpty()) return
        val mags = goertzelBank(samples, toneFreqs, sampleRate)
        // Normalize by chunk length so different chunk sizes are comparable;
        // convert mag² to dB.
        val norm = (samples.size * samples.size).toFloat().coerceAtLeast(1f)
        val newTones = FloatArray(mags.size) { i ->
            val v = mags[i] / norm
            val db = if (v > 0f) 10f * log10(v) else DB_FLOOR
            // EMA smoothing.
            val prev = _state.value.tonesDb.getOrElse(i) { DB_FLOOR }
            prev + TONE_EMA_ALPHA * (db - prev)
        }
        var sumSq = 0.0
        for (x in samples) sumSq += (x * x).toDouble()
        val rms = sqrt(sumSq / samples.size).toFloat()
        _state.update { it.copy(tonesDb = newTones, rms = rms) }
    }

    fun pushEvent(e: FfiFrameEvent) {
        when (e) {
            is FfiFrameEvent.FrameOk -> recordFrame(e.seq, FrameStatus.Ok, reason = null)
            is FfiFrameEvent.FrameDropped -> recordFrame(e.seq, FrameStatus.Dropped, e.reason)
            is FfiFrameEvent.StreamComplete -> { /* terminal; UI shows result */ }
        }
    }

    private fun recordFrame(seq: UInt, status: FrameStatus, reason: String?) {
        val now = clockMs()
        lastProgressMs = now
        _state.update { s ->
            val mark = FrameMark(seq, status, reason, now)
            val trimmed = if (s.timeline.size >= MAX_TIMELINE) {
                s.timeline.drop(s.timeline.size - MAX_TIMELINE + 1) + mark
            } else {
                s.timeline + mark
            }
            s.copy(
                timeline = trimmed,
                searching = false,
                totalOk = if (status == FrameStatus.Ok) s.totalOk + 1 else s.totalOk,
                totalDropped = if (status == FrameStatus.Dropped) s.totalDropped + 1 else s.totalDropped,
                watchdogSecondsLeft = max(0, ((WATCHDOG_TOTAL_MS) / 1000L).toInt()),
            )
        }
    }

    /** Called by the screen (~1 Hz) to refresh the watchdog countdown. */
    fun tickWatchdog() {
        val s = _state.value
        if (s.searching || lastProgressMs == 0L) return
        val elapsed = clockMs() - lastProgressMs
        val left = max(0L, WATCHDOG_TOTAL_MS - elapsed) / 1000L
        _state.update { it.copy(watchdogSecondsLeft = left.toInt()) }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd android && ./gradlew :app:testDebugUnitTest --tests se.karleklund.modem.viz.VizEngineTest`
Expected: PASS (all 7 tests).

- [ ] **Step 5: Commit**

```bash
jj desc -m "Add VizEngine — tone bars + frame timeline state for Android"
jj new
```

---

### Task 4: `ToneBars` Compose component

Vertical bar chart, one bar per tone, height driven by `tonesDb` (mapped from `[-80, 0] dB` to `[0, 1]`). Renders with a `Canvas`, redraws on state change.

**Files:**
- Create: `android/app/src/main/java/se/karleklund/modem/viz/ToneBars.kt`

- [ ] **Step 1: Create the component**

Create `android/app/src/main/java/se/karleklund/modem/viz/ToneBars.kt`:

```kotlin
package se.karleklund.modem.viz

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp

/**
 * 8 vertical bars (one per FSK tone), height in [0,1] scaled from dB.
 * `tonesDb` and `freqs` must have matching length.
 */
@Composable
fun ToneBars(
    tonesDb: FloatArray,
    freqs: FloatArray,
    modifier: Modifier = Modifier,
) {
    Column(modifier = modifier) {
        Canvas(modifier = Modifier.fillMaxWidth().height(120.dp)) {
            val n = tonesDb.size.coerceAtLeast(1)
            val barW = size.width / (n * 2f) // half-width gap between bars
            val maxH = size.height
            for (i in 0 until n) {
                val h01 = ((tonesDb[i] - DB_MIN) / (DB_MAX - DB_MIN)).coerceIn(0f, 1f)
                val h = h01 * maxH
                val x = (i * 2f + 0.5f) * barW
                drawRect(
                    color = Color(0xFF4CAF50).copy(alpha = 0.3f + 0.7f * h01),
                    topLeft = Offset(x, maxH - h),
                    size = Size(barW, h),
                )
            }
        }
        Row(modifier = Modifier.fillMaxWidth().padding(top = 4.dp)) {
            for (i in freqs.indices) {
                Text(
                    text = "%.1fk".format(freqs[i] / 1000f),
                    style = MaterialTheme.typography.labelSmall,
                    modifier = Modifier.weight(1f),
                )
            }
        }
    }
}

private const val DB_MIN = -80f
private const val DB_MAX = 0f
```

- [ ] **Step 2: Verify the module compiles**

Run: `cd android && ./gradlew :app:compileDebugKotlin`
Expected: BUILD SUCCESSFUL.

- [ ] **Step 3: Commit**

```bash
jj desc -m "Add ToneBars Compose component"
jj new
```

---

### Task 5: `FrameTimeline` Compose component

Horizontal scrollable chip strip showing past `FrameMark` entries. Latest on the right; auto-scrolls so newest is visible. Tapping a dropped chip expands its reason inline.

**Files:**
- Create: `android/app/src/main/java/se/karleklund/modem/viz/FrameTimeline.kt`

- [ ] **Step 1: Create the component**

Create `android/app/src/main/java/se/karleklund/modem/viz/FrameTimeline.kt`:

```kotlin
package se.karleklund.modem.viz

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.clickable
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp

@Composable
fun FrameTimeline(timeline: List<FrameMark>, modifier: Modifier = Modifier) {
    val listState = rememberLazyListState()
    LaunchedEffect(timeline.size) {
        if (timeline.isNotEmpty()) {
            listState.animateScrollToItem(timeline.size - 1)
        }
    }
    var expandedSeq by remember { mutableStateOf<UInt?>(null) }
    Column {
        LazyRow(
            state = listState,
            horizontalArrangement = Arrangement.spacedBy(6.dp),
            contentPadding = PaddingValues(horizontal = 4.dp),
            modifier = modifier,
        ) {
            items(timeline.size) { idx ->
                val m = timeline[idx]
                val bg = when (m.status) {
                    FrameStatus.Ok -> Color(0xFF4CAF50)
                    FrameStatus.Dropped -> Color(0xFFE53935)
                }
                val labelColor = Color.White
                Text(
                    text = "${m.seq}",
                    color = labelColor,
                    style = MaterialTheme.typography.labelSmall,
                    modifier = Modifier
                        .clip(RoundedCornerShape(6.dp))
                        .background(bg)
                        .clickable(enabled = m.status == FrameStatus.Dropped) {
                            expandedSeq = if (expandedSeq == m.seq) null else m.seq
                        }
                        .padding(horizontal = 8.dp, vertical = 4.dp),
                )
            }
        }
        val drop = timeline.firstOrNull { it.seq == expandedSeq }
        if (drop?.reason != null) {
            Text(
                text = "frame ${drop.seq}: ${drop.reason}",
                style = MaterialTheme.typography.bodySmall,
                color = Color(0xFFE53935),
                modifier = Modifier.padding(top = 4.dp),
            )
        }
    }
}

// LazyRow `items` extension import:
private fun androidx.compose.foundation.lazy.LazyListScope.items(
    count: Int,
    itemContent: @Composable (Int) -> Unit,
) {
    items(count = count, key = null, contentType = { null }) { itemContent(it) }
}
```

Note: the helper `items` shim is to avoid pulling in `androidx.compose.foundation.lazy.items` — keeps imports tight. If the editor flags it, instead import `androidx.compose.foundation.lazy.items` directly and remove the shim.

- [ ] **Step 2: Verify the module compiles**

Run: `cd android && ./gradlew :app:compileDebugKotlin`
Expected: BUILD SUCCESSFUL. If the `items` helper fails to resolve, replace the shim with a direct `import androidx.compose.foundation.lazy.items` and use `items(timeline) { m -> … }` instead of the indexed form.

- [ ] **Step 3: Commit**

```bash
jj desc -m "Add FrameTimeline Compose component"
jj new
```

---

### Task 6: `ListeningIndicator` Compose component

Pulsing circle shown while `searching == true`. Uses `InfiniteTransition` to scale 1.0 → 1.4 and fade alpha 1.0 → 0.

**Files:**
- Create: `android/app/src/main/java/se/karleklund/modem/viz/ListeningIndicator.kt`

- [ ] **Step 1: Create the component**

Create `android/app/src/main/java/se/karleklund/modem/viz/ListeningIndicator.kt`:

```kotlin
package se.karleklund.modem.viz

import androidx.compose.animation.core.InfiniteRepeatableSpec
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp

@Composable
fun ListeningIndicator(modifier: Modifier = Modifier) {
    val t = rememberInfiniteTransition(label = "listening")
    val scale by t.animateFloat(
        initialValue = 1.0f,
        targetValue = 1.4f,
        animationSpec = InfiniteRepeatableSpec(
            animation = tween(1000),
            repeatMode = RepeatMode.Reverse,
        ),
        label = "listening-scale",
    )
    val alpha by t.animateFloat(
        initialValue = 1.0f,
        targetValue = 0.2f,
        animationSpec = InfiniteRepeatableSpec(
            animation = tween(1000),
            repeatMode = RepeatMode.Reverse,
        ),
        label = "listening-alpha",
    )
    Row(verticalAlignment = Alignment.CenterVertically, modifier = modifier) {
        Canvas(modifier = Modifier.size(16.dp)) {
            drawCircle(
                color = Color(0xFF1E88E5).copy(alpha = alpha),
                radius = size.minDimension / 2f * scale,
            )
        }
        Text("listening for preamble…", modifier = Modifier)
    }
}
```

- [ ] **Step 2: Verify the module compiles**

Run: `cd android && ./gradlew :app:compileDebugKotlin`
Expected: BUILD SUCCESSFUL.

- [ ] **Step 3: Commit**

```bash
jj desc -m "Add ListeningIndicator Compose component"
jj new
```

---

### Task 7: `TxProgress` Compose component

Linear progress bar that fills based on `elapsed / total`. Caller passes total samples (or seconds) and a `StateFlow<Float>` of elapsed seconds (which ModemViewModel will drive from the playback-feeder coroutine in Task 9).

**Files:**
- Create: `android/app/src/main/java/se/karleklund/modem/viz/TxProgress.kt`

- [ ] **Step 1: Create the component**

Create `android/app/src/main/java/se/karleklund/modem/viz/TxProgress.kt`:

```kotlin
package se.karleklund.modem.viz

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

@Composable
fun TxProgress(
    elapsedSec: Float,
    totalSec: Float,
    bytes: Int,
    modifier: Modifier = Modifier,
) {
    val frac = if (totalSec > 0f) (elapsedSec / totalSec).coerceIn(0f, 1f) else 0f
    Column(modifier = modifier) {
        Text("→ $bytes B · ${"%.1f".format(elapsedSec)} / ${"%.1f".format(totalSec)} s")
        LinearProgressIndicator(
            progress = { frac },
            modifier = Modifier.fillMaxWidth().padding(top = 4.dp),
        )
    }
}
```

- [ ] **Step 2: Verify the module compiles**

Run: `cd android && ./gradlew :app:compileDebugKotlin`
Expected: BUILD SUCCESSFUL.

- [ ] **Step 3: Commit**

```bash
jj desc -m "Add TxProgress Compose component"
jj new
```

---

### Task 8: `ResultPulse` flash animation

Brief green pulse on `sha256_ok == true`, red shake on mismatch. Wraps a child `@Composable`. Used to wrap the final `ResultView` already in `ModemScreen.kt`.

**Files:**
- Create: `android/app/src/main/java/se/karleklund/modem/viz/ResultPulse.kt`

- [ ] **Step 1: Create the component**

Create `android/app/src/main/java/se/karleklund/modem/viz/ResultPulse.kt`:

```kotlin
package se.karleklund.modem.viz

import androidx.compose.animation.core.Animatable
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp

@Composable
fun ResultPulse(ok: Boolean, content: @Composable () -> Unit) {
    val alpha = remember { Animatable(1f) }
    LaunchedEffect(ok) {
        alpha.snapTo(1f)
        alpha.animateTo(0f, animationSpec = androidx.compose.animation.core.tween(1500))
    }
    val color = if (ok) Color(0xFF4CAF50) else Color(0xFFE53935)
    androidx.compose.foundation.layout.Box(
        modifier = Modifier
            .background(color.copy(alpha = alpha.value * 0.25f))
            .padding(4.dp),
    ) {
        content()
    }
}
```

- [ ] **Step 2: Verify the module compiles**

Run: `cd android && ./gradlew :app:compileDebugKotlin`
Expected: BUILD SUCCESSFUL.

- [ ] **Step 3: Commit**

```bash
jj desc -m "Add ResultPulse Compose component"
jj new
```

---

### Task 9: Wire `VizEngine` into `ModemViewModel`

Own a `VizEngine` instance, feed it audio chunks from both the receive loop and a new send-side feeder coroutine, and expose its `state` plus a `txElapsedSec` flow.

**Files:**
- Modify: `android/app/src/main/java/se/karleklund/modem/ModemViewModel.kt`

- [ ] **Step 1: Add `VizEngine`, send-feeder, and elapsed-time flow**

Open `android/app/src/main/java/se/karleklund/modem/ModemViewModel.kt`. Apply the changes below.

After the `private val _variants` block (around line 41), add:

```kotlin
    private val toneFreqs = audibleToneFreqs() // updated when profile changes
    val viz = VizEngine(toneFreqs, sampleRate = 48_000)

    private val _txElapsedSec = MutableStateFlow(0f)
    val txElapsedSec: StateFlow<Float> = _txElapsedSec.asStateFlow()

    private fun audibleToneFreqs(): FloatArray = FloatArray(8) { 2000f + it * 200f }
    private fun ultrasonicToneFreqs(): FloatArray = FloatArray(8) { 17_500f + it * 250f }

    private fun toneFreqsFor(p: Profile): FloatArray = when (p) {
        Profile.AUDIBLE -> audibleToneFreqs()
        Profile.ULTRASONIC -> ultrasonicToneFreqs()
    }
```

Add new imports at the top:

```kotlin
import se.karleklund.modem.viz.VizEngine
```

Modify `setProfile`:

```kotlin
    fun setProfile(p: Profile) {
        if (_profile.value != p) {
            stopReceive()
        }
        _profile.value = p
        viz.reset()
        // Rebuild a fresh VizEngine if the profile changed bands.
        // (Simplest: reset + a profile-aware setter.)
        viz.setToneFreqs(toneFreqsFor(p))
    }
```

In `VizEngine.kt` (from Task 3), add at the bottom of the class:

```kotlin
    fun setToneFreqs(freqs: FloatArray) {
        // Reset is independent — callers usually call reset() too.
        // We just swap the freq array used by the next pushChunk.
        toneFreqsRef = freqs
    }
```

And change the constructor field from `private val toneFreqs: FloatArray` to:

```kotlin
class VizEngine(
    toneFreqs: FloatArray,
    private val sampleRate: Int,
    private val clockMs: () -> Long = { System.currentTimeMillis() },
) {
    private var toneFreqsRef: FloatArray = toneFreqs
```

And inside `pushChunk`, replace `goertzelBank(samples, toneFreqs, sampleRate)` with `goertzelBank(samples, toneFreqsRef, sampleRate)`.

Modify `send` in `ModemViewModel.kt`:

```kotlin
    fun send(text: String) {
        if (sendJob?.isActive == true) return
        stopReceive()
        viz.reset()
        viz.setToneFreqs(toneFreqsFor(_profile.value))
        sendJob = viewModelScope.launch {
            try {
                val bytes = text.toByteArray(Charsets.UTF_8)
                val tx = FfiTransmitter(_profile.value, _variants.value)
                val samplesList = tx.encode(bytes)
                val samples = FloatArray(samplesList.size).also { arr ->
                    for (i in samplesList.indices) arr[i] = samplesList[i]
                }
                val totalSec = samples.size / 48_000f
                _state.value = UiState.Sending(bytes.size, totalSec)
                _txElapsedSec.value = 0f
                // Feed viz from a coroutine that walks the buffer at wall-clock
                // rate, so the tone bars animate in sync with playback.
                val feeder = launch {
                    val chunk = 2400 // 50 ms
                    val start = System.currentTimeMillis()
                    var i = 0
                    while (isActive && i < samples.size) {
                        val end = (i + chunk).coerceAtMost(samples.size)
                        viz.pushChunk(samples.copyOfRange(i, end))
                        i = end
                        _txElapsedSec.value = (System.currentTimeMillis() - start) / 1000f
                        kotlinx.coroutines.delay(50L)
                    }
                }
                withContext(Dispatchers.IO) { AudioPlayer.play(samples) }
                feeder.cancel()
                _txElapsedSec.value = totalSec
                _state.value = UiState.Idle
            } catch (t: Throwable) {
                _state.value = UiState.Error(t.message ?: t.toString())
            }
        }
    }
```

Modify `startReceive` so it feeds `viz`:

```kotlin
    fun startReceive() {
        if (receiveJob?.isActive == true) return
        viz.reset()
        viz.setToneFreqs(toneFreqsFor(_profile.value))
        val rx = FfiReceiver(_profile.value, _variants.value)
        val cap = AudioCapture().also { capture = it; it.start() }
        _state.value = UiState.Receiving(framesOk = 0)
        receiveJob = viewModelScope.launch(Dispatchers.IO) {
            var framesOk = 0
            var lastProgress = System.currentTimeMillis()
            var started = false
            try {
                while (isActive) {
                    val chunk = withTimeoutOrNull(500L) { cap.channel.receive() } ?: run {
                        viz.tickWatchdog()
                        null
                    }
                    if (chunk != null) {
                        viz.pushChunk(chunk)
                        val events = rx.pushSamples(chunk.toList())
                        for (e in events) {
                            viz.pushEvent(e)
                            when (e) {
                                is FfiFrameEvent.FrameOk -> {
                                    framesOk++
                                    started = true
                                    lastProgress = System.currentTimeMillis()
                                    _state.value = UiState.Receiving(framesOk)
                                }
                                is FfiFrameEvent.FrameDropped -> {
                                    started = true
                                    lastProgress = System.currentTimeMillis()
                                }
                                is FfiFrameEvent.StreamComplete -> {
                                    _state.value = UiState.Result(e.bytes, e.sha256Ok)
                                    stopReceiveInternal()
                                    return@launch
                                }
                            }
                        }
                    }
                    if (started && System.currentTimeMillis() - lastProgress > 10_000) {
                        _state.value = UiState.Error("no progress for 10 s")
                        stopReceiveInternal()
                        return@launch
                    }
                }
            } catch (t: Throwable) {
                _state.value = UiState.Error(t.message ?: t.toString())
                stopReceiveInternal()
            } finally {
                rx.close()
            }
        }
    }
```

- [ ] **Step 2: Update VizEngineTest for the new constructor**

Tests pass through unchanged: the constructor signature is the same (named-arg call sites in tests still match `VizEngine(audibleTones, sampleRate = 48_000, clockMs = { 0L })`). No test edits needed.

- [ ] **Step 3: Verify build**

Run: `cd android && ./gradlew :app:assembleDebug :app:testDebugUnitTest`
Expected: BUILD SUCCESSFUL and all unit tests pass.

- [ ] **Step 4: Commit**

```bash
jj desc -m "Wire VizEngine into ModemViewModel for send and receive"
jj new
```

---

### Task 10: Update `ModemScreen` to show new viz components

**Files:**
- Modify: `android/app/src/main/java/se/karleklund/modem/ui/ModemScreen.kt`

- [ ] **Step 1: Add imports and viz block**

Open `android/app/src/main/java/se/karleklund/modem/ui/ModemScreen.kt`. Add imports:

```kotlin
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.height
import se.karleklund.modem.viz.FrameTimeline
import se.karleklund.modem.viz.ListeningIndicator
import se.karleklund.modem.viz.ResultPulse
import se.karleklund.modem.viz.ToneBars
import se.karleklund.modem.viz.TxProgress
```

Inside the existing `Column` (right after the existing `Row` of profile and DSP chips, before the `OutlinedTextField`), add:

```kotlin
            val vizState by vm.viz.state.collectAsStateWithLifecycle()
            val txElapsed by vm.txElapsedSec.collectAsStateWithLifecycle()
            val tones = remember(profile) {
                when (profile) {
                    Profile.AUDIBLE -> FloatArray(8) { 2000f + it * 200f }
                    Profile.ULTRASONIC -> FloatArray(8) { 17_500f + it * 250f }
                }
            }
            ToneBars(tonesDb = vizState.tonesDb, freqs = tones, modifier = Modifier.fillMaxWidth())
            if (vizState.timeline.isNotEmpty()) {
                FrameTimeline(timeline = vizState.timeline, modifier = Modifier.fillMaxWidth())
            }
            val s = state
            if (s is UiState.Receiving && vizState.searching) {
                ListeningIndicator()
            }
            if (s is UiState.Receiving) {
                val wd = vizState.watchdogSecondsLeft
                Text(
                    "listening · ${vizState.totalOk} ok · ${vizState.totalDropped} dropped" +
                            (wd?.let { " · watchdog ${it}s" } ?: ""),
                )
            }
            if (s is UiState.Sending) {
                TxProgress(elapsedSec = txElapsed, totalSec = s.seconds, bytes = s.bytes)
            }
            Spacer(Modifier.height(4.dp))
```

Replace the existing `is UiState.Result -> ResultView(...)` line with:

```kotlin
                is UiState.Result -> ResultPulse(ok = s.sha256Ok) {
                    ResultView(s.bytes, s.sha256Ok, showHex, onToggle = { showHex = !showHex })
                }
```

Add a `LaunchedEffect` near the top of `ModemScreen` to tick the watchdog while receiving:

```kotlin
    androidx.compose.runtime.LaunchedEffect(state) {
        if (state is UiState.Receiving) {
            while (true) {
                kotlinx.coroutines.delay(1000L)
                vm.viz.tickWatchdog()
            }
        }
    }
```

- [ ] **Step 2: Verify build**

Run: `cd android && ./gradlew :app:assembleDebug`
Expected: BUILD SUCCESSFUL.

- [ ] **Step 3: Manual smoke test — installDebug + visual check**

The user runs these themselves:

```bash
./scripts/build-android.sh
cd android && ./gradlew installDebug
```

Then: open the app, hit Receive — confirm the tone bars are alive and a "listening" pulse appears. Hit Send on another device (or play one of `captures/` over a speaker) — confirm at least one green chip lands on the timeline. Hit Send from this device — confirm the TX progress bar fills and tone bars animate.

If the visualization looks acceptable, continue. Otherwise file specific feedback as new tasks before moving on.

- [ ] **Step 4: Commit**

```bash
jj desc -m "Show ToneBars, FrameTimeline, TxProgress, ResultPulse in ModemScreen"
jj new
```

---

## Phase 3: CLI — TUI

### Task 11: `Reporter` trait + `PlainReporter` impl

Refactor `cmd_recv` and `cmd_send` to emit events through a `Reporter` trait so we can swap a TUI implementation in next. Keep current eprintln output intact via `PlainReporter`.

**Files:**
- Create: `modem-cli/src/reporter.rs`
- Modify: `modem-cli/src/cmd_recv.rs`
- Modify: `modem-cli/src/cmd_send.rs`
- Modify: `modem-cli/src/main.rs` (declare `mod reporter;`)

- [ ] **Step 1: Create the trait**

Create `modem-cli/src/reporter.rs`:

```rust
//! Pluggable reporter for the live `recv`/`send` commands. The default
//! implementation prints plain text (the historical behaviour, used by
//! scripts and tests). The TUI implementation in `tui.rs` renders a live
//! ratatui dashboard instead.

use modem_codec::rx::FrameEvent;

/// Receive-side events the runner emits.
pub enum RxEvent<'a> {
    Listening,
    Chunk(&'a [f32]),
    Frame(&'a FrameEvent),
}

/// Send-side events the runner emits.
pub enum TxEvent<'a> {
    Starting { bytes: usize, samples: usize, seconds: f32 },
    Chunk(&'a [f32]),
    Done,
}

pub trait Reporter {
    fn on_rx(&mut self, _e: RxEvent<'_>) {}
    fn on_tx(&mut self, _e: TxEvent<'_>) {}
}

/// Drop-in for the historical eprintln-based output.
pub struct PlainReporter;

impl Reporter for PlainReporter {
    fn on_rx(&mut self, e: RxEvent<'_>) {
        match e {
            RxEvent::Listening => eprintln!("listening… (Ctrl-C to stop)"),
            RxEvent::Chunk(_) => {}
            RxEvent::Frame(FrameEvent::FrameOk { seq, bytes }) => {
                eprintln!("  frame {seq} ok ({} bytes)", bytes.len());
            }
            RxEvent::Frame(FrameEvent::FrameDropped { seq, reason }) => {
                eprintln!("  frame {seq} dropped: {reason}");
            }
            RxEvent::Frame(FrameEvent::StreamComplete { bytes, sha256_ok }) => {
                eprintln!(
                    "✓ {} bytes received, sha256 {}",
                    bytes.len(),
                    if *sha256_ok { "ok" } else { "MISMATCH" },
                );
            }
        }
    }
    fn on_tx(&mut self, e: TxEvent<'_>) {
        match e {
            TxEvent::Starting { bytes, samples, seconds } => {
                eprintln!(
                    "→ {} bytes, {} samples ({:.1}s) over speaker",
                    bytes, samples, seconds,
                );
            }
            TxEvent::Chunk(_) => {}
            TxEvent::Done => eprintln!("✓ done"),
        }
    }
}
```

- [ ] **Step 2: Wire `mod reporter;` into `main.rs`**

Add to the top of `modem-cli/src/main.rs`, alongside the other `mod` declarations:

```rust
mod reporter;
```

- [ ] **Step 3: Route `cmd_recv` through the reporter**

Open `modem-cli/src/cmd_recv.rs`. Change the signature and body to:

```rust
use crate::output_format::{is_printable, print_hex};
use crate::reporter::{Reporter, RxEvent};
use crate::{make_phy, Profile};
use modem_audio::input::open_mic;
use modem_codec::rx::{FrameEvent, Receiver};
use modem_core::fsk::DspVariants;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub fn run(
    profile: Profile,
    variants: DspVariants,
    output: Option<PathBuf>,
    hex: bool,
    reporter: &mut dyn Reporter,
) -> anyhow::Result<()> {
    let phy = make_phy(profile, variants);
    let mut rx = Receiver::new(phy);
    let mic = open_mic()?;

    reporter.on_rx(RxEvent::Listening);

    let mut last_progress = Instant::now();
    let mut started = false;
    loop {
        let chunk = match mic.rx.recv_timeout(Duration::from_millis(500)) {
            Ok(c) => c,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if started && last_progress.elapsed() > Duration::from_secs(10) {
                    eprintln!("⚠ no progress for 10s, giving up");
                    return Ok(());
                }
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        reporter.on_rx(RxEvent::Chunk(&chunk));
        let events = rx.push_samples(&chunk);
        for ev in events {
            reporter.on_rx(RxEvent::Frame(&ev));
            match ev {
                FrameEvent::FrameOk { .. } => {
                    started = true;
                    last_progress = Instant::now();
                }
                FrameEvent::FrameDropped { .. } => {
                    started = true;
                    last_progress = Instant::now();
                }
                FrameEvent::StreamComplete { bytes, sha256_ok: _ } => {
                    if let Some(p) = &output {
                        fs::write(p, &bytes)?;
                        eprintln!("  wrote {}", p.display());
                    } else if hex || !is_printable(&bytes) {
                        print_hex(&bytes);
                    } else {
                        std::io::stdout().write_all(&bytes)?;
                        std::io::stdout().write_all(b"\n")?;
                    }
                    return Ok(());
                }
            }
        }
    }
}
```

- [ ] **Step 4: Route `cmd_send` through the reporter**

Open `modem-cli/src/cmd_send.rs`. Change to:

```rust
use crate::reporter::{Reporter, TxEvent};
use crate::{make_phy, Profile};
use modem_codec::tx::Transmitter;
use modem_core::fsk::DspVariants;
use std::fs;
use std::io::Read;
use std::path::PathBuf;

pub fn run(
    profile: Profile,
    variants: DspVariants,
    input: Option<PathBuf>,
    reporter: &mut dyn Reporter,
) -> anyhow::Result<()> {
    let bytes = match input {
        Some(p) => fs::read(&p)?,
        None => {
            let mut buf = Vec::new();
            std::io::stdin().read_to_end(&mut buf)?;
            buf
        }
    };
    let phy = make_phy(profile, variants);
    let ultrasonic = matches!(profile, Profile::Ultrasonic);
    let tx = Transmitter::new(phy, ultrasonic);
    let samples = tx.encode(&bytes);
    reporter.on_tx(TxEvent::Starting {
        bytes: bytes.len(),
        samples: samples.len(),
        seconds: samples.len() as f32 / 48_000.0,
    });
    // Note: the reporter doesn't get per-chunk TxEvent::Chunk events in
    // PlainReporter mode (the audio writer is opaque); the TUI reporter
    // drives that itself via a wall-clock feeder. See tui.rs.
    modem_audio::output::play_samples(samples)?;
    reporter.on_tx(TxEvent::Done);
    Ok(())
}
```

- [ ] **Step 5: Update `main.rs` call sites**

In `modem-cli/src/main.rs`, change the `Cmd::Send` and `Cmd::Recv` match arms to:

```rust
        Cmd::Send { profile, variants, input } => {
            let mut r = reporter::PlainReporter;
            cmd_send::run(profile, variants.into(), input, &mut r)
        }
        Cmd::Recv { profile, variants, output, hex } => {
            let mut r = reporter::PlainReporter;
            cmd_recv::run(profile, variants.into(), output, hex, &mut r)
        }
```

- [ ] **Step 6: Verify build + existing tests**

Run: `cargo build -p modem-cli && cargo test --workspace`
Expected: BUILD SUCCESSFUL and all existing tests pass (no test changes needed; the existing test files don't touch `cmd_recv`/`cmd_send`).

- [ ] **Step 7: Commit**

```bash
jj desc -m "Introduce Reporter trait + PlainReporter; route cmd_recv/send through it"
jj new
```

---

### Task 12: Add ratatui + crossterm deps

**Files:**
- Modify: `modem-cli/Cargo.toml`

- [ ] **Step 1: Add dependencies**

Open `modem-cli/Cargo.toml`. In the `[dependencies]` block, append:

```toml
ratatui   = "0.28"
crossterm = "0.28"
```

- [ ] **Step 2: Verify dependency resolution**

Run: `cargo build -p modem-cli`
Expected: BUILD SUCCESSFUL (new crates fetched).

- [ ] **Step 3: Commit**

```bash
jj desc -m "Add ratatui + crossterm deps for modem-cli TUI"
jj new
```

---

### Task 13: `TuiReporter` skeleton — alt-screen setup/teardown

Set up an alternate-screen terminal, draw an empty frame, accept Ctrl-C to exit cleanly, and tear down on Drop. No content yet.

**Files:**
- Create: `modem-cli/src/tui.rs`
- Modify: `modem-cli/src/main.rs` (declare `mod tui;`)

- [ ] **Step 1: Wire `mod tui;` into main**

Add to the top of `modem-cli/src/main.rs`, alongside other `mod` declarations:

```rust
mod tui;
```

- [ ] **Step 2: Create skeleton TuiReporter**

Create `modem-cli/src/tui.rs`:

```rust
//! ratatui-based live reporter for `modem recv` / `modem send`.
//!
//! Activated when stdout is a TTY (and `--plain` isn't passed). Renders a
//! tone-bar meter, frame timeline, and status line. On drop, restores the
//! terminal to normal mode.

use std::io::{stdout, Stdout};

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    Terminal,
};

use crate::reporter::{Reporter, RxEvent, TxEvent};

pub struct TuiReporter {
    term: Terminal<CrosstermBackend<Stdout>>,
    // Mode-specific render state lives in Task 14+.
}

impl TuiReporter {
    pub fn new() -> std::io::Result<Self> {
        enable_raw_mode()?;
        execute!(stdout(), EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout());
        let term = Terminal::new(backend)?;
        Ok(Self { term })
    }

    /// Poll the keyboard non-blockingly. Returns true if the user requested exit.
    pub fn should_quit(&mut self) -> bool {
        if event::poll(std::time::Duration::from_millis(0)).unwrap_or(false) {
            if let Ok(Event::Key(k)) = event::read() {
                if k.kind == KeyEventKind::Press {
                    return matches!(k.code, KeyCode::Char('q') | KeyCode::Esc);
                }
            }
        }
        false
    }
}

impl Drop for TuiReporter {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(stdout(), LeaveAlternateScreen);
    }
}

impl Reporter for TuiReporter {
    fn on_rx(&mut self, _e: RxEvent<'_>) {
        // Filled in by Tasks 14+.
    }
    fn on_tx(&mut self, _e: TxEvent<'_>) {
        // Filled in by Task 17.
    }
}
```

- [ ] **Step 3: Verify build**

Run: `cargo build -p modem-cli`
Expected: BUILD SUCCESSFUL.

- [ ] **Step 4: Commit**

```bash
jj desc -m "Add TuiReporter skeleton with raw-mode/alt-screen lifecycle"
jj new
```

---

### Task 14: TuiReporter — render tone bars + frame timeline for `recv`

Hold receive state (per-tone EMA in dB, ring of `FrameMark`s, totals) and draw one frame on each `on_rx` call.

**Files:**
- Modify: `modem-cli/src/tui.rs`

- [ ] **Step 1: Add render state + draw**

Replace the contents of `modem-cli/src/tui.rs` with:

```rust
//! ratatui-based live reporter for `modem recv` / `modem send`.

use std::io::{stdout, Stdout};
use std::time::Instant;

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Terminal,
};

use modem_codec::rx::FrameEvent;
use modem_core::fsk::{goertzel_bank, FskConfig};

use crate::reporter::{Reporter, RxEvent, TxEvent};
use crate::Profile;

const TONE_EMA_ALPHA: f32 = 0.4;
const DB_FLOOR: f32 = -80.0;
const DB_CEIL: f32 = 0.0;
const MAX_TIMELINE: usize = 64;

#[derive(Clone, Copy)]
enum FrameStatus { Ok, Dropped }

struct FrameMark {
    seq: u32,
    status: FrameStatus,
    reason: Option<String>,
}

enum Mode {
    Rx {
        tones_db: Vec<f32>,
        timeline: Vec<FrameMark>,
        total_ok: usize,
        total_dropped: usize,
        rms: f32,
        last_progress: Option<Instant>,
        listening_start: Instant,
    },
    Tx {
        bytes: usize,
        total_sec: f32,
        elapsed_sec: f32,
        tones_db: Vec<f32>,
    },
}

pub struct TuiReporter {
    term: Terminal<CrosstermBackend<Stdout>>,
    mode: Mode,
    freqs: Vec<f32>,
    sample_rate: u32,
}

impl TuiReporter {
    pub fn new_rx(profile: Profile) -> std::io::Result<Self> {
        enable_raw_mode()?;
        execute!(stdout(), EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout());
        let term = Terminal::new(backend)?;
        let cfg = match profile {
            Profile::Audible => FskConfig::audible(),
            Profile::Ultrasonic => FskConfig::ultrasonic(),
        };
        Ok(Self {
            term,
            mode: Mode::Rx {
                tones_db: vec![DB_FLOOR; cfg.tone_freqs.len()],
                timeline: Vec::new(),
                total_ok: 0,
                total_dropped: 0,
                rms: 0.0,
                last_progress: None,
                listening_start: Instant::now(),
            },
            freqs: cfg.tone_freqs.to_vec(),
            sample_rate: cfg.sample_rate,
        })
    }

    pub fn new_tx(profile: Profile) -> std::io::Result<Self> {
        enable_raw_mode()?;
        execute!(stdout(), EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout());
        let term = Terminal::new(backend)?;
        let cfg = match profile {
            Profile::Audible => FskConfig::audible(),
            Profile::Ultrasonic => FskConfig::ultrasonic(),
        };
        Ok(Self {
            term,
            mode: Mode::Tx {
                bytes: 0,
                total_sec: 0.0,
                elapsed_sec: 0.0,
                tones_db: vec![DB_FLOOR; cfg.tone_freqs.len()],
            },
            freqs: cfg.tone_freqs.to_vec(),
            sample_rate: cfg.sample_rate,
        })
    }

    pub fn should_quit(&mut self) -> bool {
        if event::poll(std::time::Duration::from_millis(0)).unwrap_or(false) {
            if let Ok(Event::Key(k)) = event::read() {
                if k.kind == KeyEventKind::Press {
                    return matches!(k.code, KeyCode::Char('q') | KeyCode::Esc);
                }
            }
        }
        false
    }

    fn draw_rx(&mut self) {
        let Mode::Rx { tones_db, timeline, total_ok, total_dropped, rms, listening_start, .. } = &self.mode else { return };
        let freqs = &self.freqs;
        let tones = tones_db.clone();
        let timeline = timeline.clone();
        let total_ok = *total_ok;
        let total_dropped = *total_dropped;
        let rms = *rms;
        let elapsed = listening_start.elapsed().as_secs_f32();
        let _ = self.term.draw(|f| {
            let layout = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(8),  // tone bars block
                    Constraint::Length(4),  // frame timeline
                    Constraint::Length(3),  // status
                ])
                .split(f.area());

            // Tone bars (block characters)
            let bars: String = tones.iter().zip(freqs.iter()).map(|(db, hz)| {
                let level = ((*db - DB_FLOOR) / (DB_CEIL - DB_FLOOR)).clamp(0.0, 1.0);
                let bar = blocks_for(level);
                format!("{}  {:>4.1}k\n", bar, hz / 1000.0)
            }).collect::<String>();
            let bars_p = Paragraph::new(bars).block(
                Block::default().borders(Borders::ALL).title(format!(" tones · rms {:.3} ", rms))
            );
            f.render_widget(bars_p, layout[0]);

            // Frame timeline
            let chips: Vec<Span> = timeline.iter().flat_map(|m| {
                let color = match m.status { FrameStatus::Ok => Color::Green, FrameStatus::Dropped => Color::Red };
                let label = match m.status { FrameStatus::Ok => format!("[ok {}]", m.seq), FrameStatus::Dropped => format!("[DROP {}]", m.seq) };
                vec![Span::styled(label, Style::default().fg(color)), Span::raw(" ")]
            }).collect();
            let tl = Paragraph::new(Line::from(chips)).block(
                Block::default().borders(Borders::ALL).title(" frames ")
            );
            f.render_widget(tl, layout[1]);

            // Status line
            let status = format!(
                " listening {:.1} s · {} ok · {} dropped · q to quit ",
                elapsed, total_ok, total_dropped
            );
            f.render_widget(Paragraph::new(status), layout[2]);
        });
    }

    fn draw_tx(&mut self) {
        let Mode::Tx { bytes, total_sec, elapsed_sec, tones_db } = &self.mode else { return };
        let bytes = *bytes;
        let total_sec = *total_sec;
        let elapsed_sec = *elapsed_sec;
        let tones = tones_db.clone();
        let freqs = self.freqs.clone();
        let _ = self.term.draw(|f| {
            let layout = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(3), Constraint::Length(8)])
                .split(f.area());
            let frac = if total_sec > 0.0 { elapsed_sec / total_sec } else { 0.0 };
            let pct = (frac * 100.0).round() as u32;
            let width = 40;
            let filled = ((width as f32) * frac.clamp(0.0, 1.0)) as usize;
            let bar = format!("[{}{}] {}% · {} B · {:.1}/{:.1} s",
                "█".repeat(filled),
                "░".repeat(width - filled),
                pct, bytes, elapsed_sec, total_sec,
            );
            f.render_widget(
                Paragraph::new(bar).block(Block::default().borders(Borders::ALL).title(" sending ")),
                layout[0],
            );
            let bars: String = tones.iter().zip(freqs.iter()).map(|(db, hz)| {
                let level = ((*db - DB_FLOOR) / (DB_CEIL - DB_FLOOR)).clamp(0.0, 1.0);
                format!("{}  {:>4.1}k\n", blocks_for(level), hz / 1000.0)
            }).collect();
            f.render_widget(
                Paragraph::new(bars).block(Block::default().borders(Borders::ALL).title(" tones ")),
                layout[1],
            );
        });
    }

    pub fn set_tx_progress(&mut self, elapsed_sec: f32) {
        if let Mode::Tx { elapsed_sec: e, .. } = &mut self.mode {
            *e = elapsed_sec;
        }
    }

    fn update_tones(&mut self, samples: &[f32]) {
        let mags = goertzel_bank(samples, &self.freqs, self.sample_rate);
        let norm = (samples.len().max(1) as f32).powi(2);
        let buf = match &mut self.mode {
            Mode::Rx { tones_db, rms, .. } => {
                let mut sum_sq = 0.0f64;
                for x in samples { sum_sq += (*x as f64) * (*x as f64); }
                *rms = (sum_sq / samples.len().max(1) as f64).sqrt() as f32;
                tones_db
            }
            Mode::Tx { tones_db, .. } => tones_db,
        };
        for (i, m) in mags.iter().enumerate() {
            let v = m / norm;
            let db = if v > 0.0 { 10.0 * v.log10() } else { DB_FLOOR };
            buf[i] = buf[i] + TONE_EMA_ALPHA * (db - buf[i]);
        }
    }
}

fn blocks_for(level: f32) -> String {
    let chars = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let width = 24;
    let n = ((level * width as f32).round() as usize).min(width);
    let last = if n == 0 { 0 } else { ((level * chars.len() as f32) as usize).min(chars.len() - 1) };
    let mut s = String::with_capacity(width);
    for i in 0..width {
        if i < n.saturating_sub(1) {
            s.push('█');
        } else if i == n.saturating_sub(1) {
            s.push(chars[last]);
        } else {
            s.push(' ');
        }
    }
    s
}

impl Drop for TuiReporter {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(stdout(), LeaveAlternateScreen);
    }
}

impl Reporter for TuiReporter {
    fn on_rx(&mut self, e: RxEvent<'_>) {
        match e {
            RxEvent::Listening => { self.draw_rx(); }
            RxEvent::Chunk(samples) => {
                self.update_tones(samples);
                self.draw_rx();
            }
            RxEvent::Frame(ev) => {
                if let Mode::Rx { timeline, total_ok, total_dropped, last_progress, .. } = &mut self.mode {
                    match ev {
                        FrameEvent::FrameOk { seq, .. } => {
                            if timeline.len() >= MAX_TIMELINE { timeline.remove(0); }
                            timeline.push(FrameMark { seq: *seq, status: FrameStatus::Ok, reason: None });
                            *total_ok += 1;
                            *last_progress = Some(Instant::now());
                        }
                        FrameEvent::FrameDropped { seq, reason } => {
                            if timeline.len() >= MAX_TIMELINE { timeline.remove(0); }
                            timeline.push(FrameMark { seq: *seq, status: FrameStatus::Dropped, reason: Some(reason.clone()) });
                            *total_dropped += 1;
                            *last_progress = Some(Instant::now());
                        }
                        FrameEvent::StreamComplete { .. } => {}
                    }
                }
                self.draw_rx();
            }
        }
    }
    fn on_tx(&mut self, e: TxEvent<'_>) {
        match e {
            TxEvent::Starting { bytes, samples: _, seconds } => {
                if let Mode::Tx { bytes: b, total_sec: t, .. } = &mut self.mode {
                    *b = bytes;
                    *t = seconds;
                }
                self.draw_tx();
            }
            TxEvent::Chunk(samples) => {
                self.update_tones(samples);
                self.draw_tx();
            }
            TxEvent::Done => { self.draw_tx(); }
        }
    }
}
```

Note about Rust ownership: the `draw_rx` / `draw_tx` methods clone the relevant slices before passing into `term.draw` to avoid simultaneous `&self` + `&mut self` borrows. This is fine — the data is small.

- [ ] **Step 2: Verify it builds**

Run: `cargo build -p modem-cli`
Expected: BUILD SUCCESSFUL.

- [ ] **Step 3: Smoke test the import path**

Run: `cargo check -p modem-cli`
Expected: no warnings beyond pre-existing ones.

- [ ] **Step 4: Commit**

```bash
jj desc -m "Implement ratatui TuiReporter: tone bars, frame timeline, TX progress"
jj new
```

---

### Task 15: TTY autodetect + `--tui`/`--plain` flags

Wire the TUI in. Default: TUI on a TTY, plain otherwise. `--plain` forces plain; `--tui` forces TUI.

**Files:**
- Modify: `modem-cli/src/main.rs`

- [ ] **Step 1: Add CLI flags + dispatch**

Open `modem-cli/src/main.rs`. Change `Cmd::Send` and `Cmd::Recv` variants to include flags:

```rust
    Send {
        #[arg(long, value_enum, default_value_t = Profile::Audible)]
        profile: Profile,
        #[command(flatten)]
        variants: VariantArgs,
        #[arg(long, conflicts_with = "plain")]
        tui: bool,
        #[arg(long, conflicts_with = "tui")]
        plain: bool,
        input: Option<PathBuf>,
    },
    Recv {
        #[arg(long, value_enum, default_value_t = Profile::Audible)]
        profile: Profile,
        #[command(flatten)]
        variants: VariantArgs,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long)]
        hex: bool,
        #[arg(long, conflicts_with = "plain")]
        tui: bool,
        #[arg(long, conflicts_with = "tui")]
        plain: bool,
    },
```

Add at the top with the other imports:

```rust
use std::io::IsTerminal;

use crate::reporter::{PlainReporter, Reporter};
use crate::tui::TuiReporter;
```

Replace the call-site arms in `fn main`:

```rust
        Cmd::Send { profile, variants, input, tui, plain } => {
            let want_tui = tui || (!plain && std::io::stdout().is_terminal());
            if want_tui {
                let mut r = TuiReporter::new_tx(profile)?;
                // Feeder: walk samples at wall-clock to animate the TUI.
                // Implemented inside cmd_send via the reporter's TxEvent::Chunk
                // pipeline. For now we set elapsed via a sidecar thread
                // managed by the reporter; see set_tx_progress.
                cmd_send::run(profile, variants.into(), input, &mut r)
            } else {
                let mut r = PlainReporter;
                cmd_send::run(profile, variants.into(), input, &mut r)
            }
        }
        Cmd::Recv { profile, variants, output, hex, tui, plain } => {
            let want_tui = tui || (!plain && std::io::stdout().is_terminal());
            if want_tui {
                let mut r = TuiReporter::new_rx(profile)?;
                cmd_recv::run(profile, variants.into(), output, hex, &mut r)
            } else {
                let mut r = PlainReporter;
                cmd_recv::run(profile, variants.into(), output, hex, &mut r)
            }
        }
```

- [ ] **Step 2: Verify build**

Run: `cargo build -p modem-cli`
Expected: BUILD SUCCESSFUL.

- [ ] **Step 3: Verify CLI help shows the new flags**

Run: `cargo run -p modem-cli --quiet -- recv --help`
Expected: output includes `--tui` and `--plain`.

- [ ] **Step 4: Verify the existing CLI flow still works in plain mode**

Run:
```bash
echo "hello modem" | cargo run -p modem-cli --quiet -- tx-wav /tmp/m.wav
cargo run -p modem-cli --quiet -- rx-wav /tmp/m.wav
```
Expected: prints `hello modem`. (The `tx-wav`/`rx-wav` arms didn't change; this is just a smoke check that the rework didn't regress wav roundtrip.)

- [ ] **Step 5: Commit**

```bash
jj desc -m "Auto-select Tui or Plain reporter for modem recv/send"
jj new
```

---

### Task 16: TX feeder thread that pushes `TxEvent::Chunk` at wall-clock rate

Right now `cmd_send::run` blocks inside `play_samples`; the TUI tone bars sit static until `Done`. Spawn a side thread (or std::thread) that walks the encoded samples in 50 ms slices and calls `reporter.on_tx(TxEvent::Chunk(&slice))` at wall-clock cadence, then joins on play completion.

Important: the `Reporter` trait is non-`Send` by default and a single reporter instance is in the main thread. Use a thread-local cadence model that buffers the samples in a `Vec`, owned by the reporter, and have the reporter expose a "tick" method called from a small loop in `cmd_send::run` between `play_samples` start and finish — but `play_samples` is blocking. Workaround: kick off the audio playback in a background thread and tick the reporter from the main thread.

**Files:**
- Modify: `modem-cli/src/cmd_send.rs`
- Modify: `modem-cli/src/tui.rs` (already has `set_tx_progress` from Task 14)

- [ ] **Step 1: Update `cmd_send::run` to drive ticks from main**

Replace the body of `modem-cli/src/cmd_send.rs` with:

```rust
use crate::reporter::{Reporter, TxEvent};
use crate::{make_phy, Profile};
use modem_codec::tx::Transmitter;
use modem_core::fsk::DspVariants;
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

pub fn run(
    profile: Profile,
    variants: DspVariants,
    input: Option<PathBuf>,
    reporter: &mut dyn Reporter,
) -> anyhow::Result<()> {
    let bytes = match input {
        Some(p) => fs::read(&p)?,
        None => {
            let mut buf = Vec::new();
            std::io::stdin().read_to_end(&mut buf)?;
            buf
        }
    };
    let phy = make_phy(profile, variants);
    let ultrasonic = matches!(profile, Profile::Ultrasonic);
    let tx = Transmitter::new(phy, ultrasonic);
    let samples = tx.encode(&bytes);
    let total_sec = samples.len() as f32 / 48_000.0;
    reporter.on_tx(TxEvent::Starting {
        bytes: bytes.len(),
        samples: samples.len(),
        seconds: total_sec,
    });

    // Hand the samples to a background playback thread; tick the reporter
    // from this (main) thread at 50 ms cadence so the TUI stays interactive.
    let samples_for_play = samples.clone();
    let (done_tx, done_rx) = mpsc::channel::<anyhow::Result<()>>();
    let play_handle = thread::spawn(move || {
        let r = modem_audio::output::play_samples(samples_for_play)
            .map_err(anyhow::Error::from);
        let _ = done_tx.send(r);
    });

    let start = Instant::now();
    let chunk_samples = 2400; // 50 ms
    let mut i = 0usize;
    loop {
        if let Ok(r) = done_rx.try_recv() {
            r?;
            break;
        }
        let end = (i + chunk_samples).min(samples.len());
        if end > i {
            reporter.on_tx(TxEvent::Chunk(&samples[i..end]));
            i = end;
        }
        // Inform the reporter of elapsed for the progress bar.
        if let Some(t) = reporter.as_tx_progress_sink() {
            t.set_tx_progress(start.elapsed().as_secs_f32().min(total_sec));
        }
        thread::sleep(Duration::from_millis(50));
    }
    let _ = play_handle.join();
    reporter.on_tx(TxEvent::Done);
    Ok(())
}
```

- [ ] **Step 2: Add a small accessor on Reporter for the progress sink**

In `modem-cli/src/reporter.rs`, append to the trait:

```rust
    /// Returns a sink that can be told elapsed playback time, when the
    /// reporter supports it (only the TUI variant does). Default: None.
    fn as_tx_progress_sink(&mut self) -> Option<&mut dyn TxProgressSink> { None }
}

pub trait TxProgressSink {
    fn set_tx_progress(&mut self, elapsed_sec: f32);
}
```

(Move the trait close brace to after this addition. The existing `impl Reporter for PlainReporter` doesn't need to override — the default returns `None`.)

In `modem-cli/src/tui.rs`, add at the bottom of the file:

```rust
impl crate::reporter::TxProgressSink for TuiReporter {
    fn set_tx_progress(&mut self, elapsed_sec: f32) {
        TuiReporter::set_tx_progress(self, elapsed_sec);
    }
}
```

And in the existing `impl Reporter for TuiReporter`, add:

```rust
    fn as_tx_progress_sink(&mut self) -> Option<&mut dyn crate::reporter::TxProgressSink> {
        Some(self)
    }
```

- [ ] **Step 3: Verify build**

Run: `cargo build -p modem-cli`
Expected: BUILD SUCCESSFUL.

- [ ] **Step 4: Manual TUI smoke test**

Run:
```bash
echo "hello tui" | cargo run --release -p modem-cli -- send
```
Expected: the alt-screen draws, the progress bar fills smoothly over ~1.5 s, tone bars animate, then the screen exits cleanly back to your prompt.

```bash
cargo run --release -p modem-cli -- recv
```
Expected: alt-screen draws with empty tone bars + "listening 0.0 s". Hit `q` — it exits cleanly.

- [ ] **Step 5: Commit**

```bash
jj desc -m "Drive TX TUI via background playback thread + main-thread ticks"
jj new
```

---

### Task 17: Per-frame status, watchdog, and `--plain` regression check

The `recv` watchdog (10 s no-progress → exit) currently lives only in `cmd_recv::run` as plain `eprintln!("⚠ no progress…")`. Push it through the reporter so the TUI can show the countdown.

**Files:**
- Modify: `modem-cli/src/reporter.rs`
- Modify: `modem-cli/src/cmd_recv.rs`
- Modify: `modem-cli/src/tui.rs`

- [ ] **Step 1: Add `RxEvent::Watchdog`**

In `modem-cli/src/reporter.rs`, extend `RxEvent`:

```rust
pub enum RxEvent<'a> {
    Listening,
    Chunk(&'a [f32]),
    Frame(&'a FrameEvent),
    /// Seconds remaining on the no-progress watchdog (0 = giving up).
    Watchdog { seconds_left: u32 },
}
```

In `impl Reporter for PlainReporter`, handle it:

```rust
            RxEvent::Watchdog { seconds_left } => {
                if seconds_left == 0 {
                    eprintln!("⚠ no progress for 10s, giving up");
                }
            }
```

- [ ] **Step 2: Emit watchdog events from `cmd_recv`**

In `modem-cli/src/cmd_recv.rs`, replace the timeout branch:

```rust
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if started {
                    let left = 10u32.saturating_sub(last_progress.elapsed().as_secs() as u32);
                    reporter.on_rx(RxEvent::Watchdog { seconds_left: left });
                    if last_progress.elapsed() > Duration::from_secs(10) {
                        return Ok(());
                    }
                }
                continue;
            }
```

- [ ] **Step 3: Render watchdog in the TUI**

In `modem-cli/src/tui.rs`, extend `Mode::Rx` with a `watchdog_secs: Option<u32>` field:

```rust
    Rx {
        tones_db: Vec<f32>,
        timeline: Vec<FrameMark>,
        total_ok: usize,
        total_dropped: usize,
        rms: f32,
        last_progress: Option<Instant>,
        listening_start: Instant,
        watchdog_secs: Option<u32>,
    },
```

Initialize `watchdog_secs: None` in `new_rx`.

In `impl Reporter for TuiReporter`, handle the new event:

```rust
            RxEvent::Watchdog { seconds_left } => {
                if let Mode::Rx { watchdog_secs, .. } = &mut self.mode {
                    *watchdog_secs = Some(seconds_left);
                }
                self.draw_rx();
            }
```

In `draw_rx`, change the status format string to include the watchdog when present:

```rust
            let wd = match self.mode { Mode::Rx { watchdog_secs, .. } => watchdog_secs, _ => None };
            let wd_str = wd.map(|s| format!(" · watchdog {}s", s)).unwrap_or_default();
            let status = format!(
                " listening {:.1} s · {} ok · {} dropped{} · q to quit ",
                elapsed, total_ok, total_dropped, wd_str
            );
```

(Adjust the destructure at the top of `draw_rx` to include `watchdog_secs` — keep it `_` since it's read via the match below.)

- [ ] **Step 4: Verify build + cargo test**

Run: `cargo build -p modem-cli && cargo test --workspace`
Expected: BUILD SUCCESSFUL and all existing tests pass.

- [ ] **Step 5: Plain-mode regression check**

Run:
```bash
echo "hello again" | cargo run -p modem-cli --quiet -- tx-wav /tmp/m.wav
cargo run -p modem-cli --quiet -- rx-wav /tmp/m.wav --plain 2>/dev/null
```

Wait — `rx-wav` is a separate command and doesn't go through the new reporter path. We're regression-checking the plain output for `recv`/`send`. Since those need real audio devices and aren't run unattended, do this check instead:

```bash
cargo run -p modem-cli --quiet -- recv --plain --help
```
Expected: the flag is accepted (returns the same help text — no error). And:

```bash
echo test | cargo run -p modem-cli --quiet -- send --plain --help
```
Expected: similar.

Plain-mode behavioural check happens during real over-the-air smoke tests (see `docs/android-smoke-test.md`); that's outside the scope of automated verification here.

- [ ] **Step 6: Commit**

```bash
jj desc -m "Plumb RxEvent::Watchdog through Reporter + TUI"
jj new
```

---

### Task 18: README + smoke-test doc updates

Add a short note to `README.md` about the new `--plain`/`--tui` flags and the Android viz, and update `docs/android-smoke-test.md` and `docs/smoke-test.md` so the documented workflow mentions the visualization.

**Files:**
- Modify: `README.md`
- Modify: `docs/smoke-test.md`
- Modify: `docs/android-smoke-test.md`

- [ ] **Step 1: README addition**

In `README.md`, after the existing "Quick start" section (around line 36), add a new section:

```markdown
## Live visualization

`modem send` and `modem recv` show a ratatui dashboard (tone bars + frame
timeline + progress) when stdout is a TTY. Pass `--plain` to force the
historical text-only output (used by scripts and CI), or `--tui` to force the
dashboard even when piping.

The Android app shows the same kind of dashboard inline on every Send and
Receive — tone bars during playback/capture, a green/red chip strip for
frames, a pulsing "listening" indicator, and a 10 s watchdog readout.
```

- [ ] **Step 2: Smoke-test doc updates**

In `docs/smoke-test.md`, find the existing `modem recv` invocation and add an aside:

```markdown
> The CLI auto-enables a live TUI when stdout is a TTY. Pass `--plain` to
> revert to the line-by-line text output (matches the historical behaviour).
```

In `docs/android-smoke-test.md`, add similar text noting the new viz components are visible during the smoke test.

- [ ] **Step 3: Commit**

```bash
jj desc -m "Document visualization UI in README and smoke-test docs"
jj new
```

---

## Self-Review

**Spec coverage check:**

| Spec section | Task(s) |
|---|---|
| Goertzel primitive (Rust + Kotlin) | Task 1 (Rust), Task 2 (Kotlin) |
| VizEngine state machine | Task 3 |
| ToneBars Compose | Task 4 |
| FrameTimeline Compose | Task 5 |
| ListeningIndicator Compose | Task 6 |
| TxProgress Compose | Task 7 |
| ResultPulse success/failure flash | Task 8 |
| ViewModel wiring (TX + RX feed) | Task 9 |
| ModemScreen layout | Task 10 |
| CLI Reporter trait + Plain impl | Task 11 |
| ratatui/crossterm deps | Task 12 |
| TuiReporter skeleton | Task 13 |
| TuiReporter rendering | Task 14 |
| TTY autodetect + flags | Task 15 |
| TX feeder for live TUI animation | Task 16 |
| Watchdog plumbing | Task 17 |
| Doc updates | Task 18 |

All spec sections have at least one task. Goertzel test cross-check between Rust and Kotlin is covered by both Task 1 and Task 2 using the same fixture (pure 2200 Hz sine over 20 ms at 48 kHz, expecting bin 1 dominance).

**Placeholder scan:** No "TBD", no "implement later", no "add appropriate handling". Every code block is complete. Drop-reason rendering is shown in FrameTimeline as an inline expand on tap.

**Type consistency:** `FrameStatus` enum used identically across `VizEngine`, `FrameTimeline`, `TuiReporter`. `tonesDb` (Float in Kotlin, `Vec<f32>` in Rust) consistent shape. `FfiFrameEvent` variants match what `modem-ffi/src/lib.rs` already exports.

**Scope check:** Single coherent feature (visualization), single plan, suitable for one implementation cycle. ~18 tasks but each is 2–5 minutes of focused work.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-05-17-modem-visualization.md`. Two execution options:

1. **Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration.

2. **Inline Execution** — Execute tasks in this session using executing-plans, batch execution with checkpoints.

Which approach?
