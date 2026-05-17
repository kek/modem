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
)

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
    private var hasSeenFrame: Boolean = false

    fun reset() {
        lastProgressMs = 0L
        hasSeenFrame = false
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
        hasSeenFrame = true
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
        if (s.searching || !hasSeenFrame) return
        val elapsed = clockMs() - lastProgressMs
        val left = max(0L, WATCHDOG_TOTAL_MS - elapsed) / 1000L
        _state.update { it.copy(watchdogSecondsLeft = left.toInt()) }
    }
}
