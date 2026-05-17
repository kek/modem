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
