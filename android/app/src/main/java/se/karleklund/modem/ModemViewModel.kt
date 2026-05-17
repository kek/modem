package se.karleklund.modem

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import se.karleklund.modem.audio.AudioCapture
import se.karleklund.modem.audio.AudioPlayer
import uniffi.modem_ffi.DspVariants
import uniffi.modem_ffi.FfiFrameEvent
import uniffi.modem_ffi.FfiReceiver
import uniffi.modem_ffi.FfiTransmitter
import uniffi.modem_ffi.Profile

sealed interface UiState {
    data object Idle : UiState
    data class Sending(val bytes: Int, val seconds: Float) : UiState
    data class Receiving(val framesOk: Int) : UiState
    data class Result(val bytes: ByteArray, val sha256Ok: Boolean) : UiState
    data class Error(val message: String) : UiState
}

class ModemViewModel : ViewModel() {
    private val _state = MutableStateFlow<UiState>(UiState.Idle)
    val state: StateFlow<UiState> = _state.asStateFlow()

    private val _profile = MutableStateFlow(Profile.AUDIBLE)
    val profile: StateFlow<Profile> = _profile.asStateFlow()

    private val _variants = MutableStateFlow(
        DspVariants(pulseShape = false, matchedFilter = false, timingRecovery = false)
    )
    val variants: StateFlow<DspVariants> = _variants.asStateFlow()

    private var receiveJob: Job? = null
    private var sendJob: Job? = null
    private var capture: AudioCapture? = null

    fun setProfile(p: Profile) {
        // If a receive is running with a different profile, stop it so the user
        // sees Idle and can re-start with the new profile. Send is unaffected.
        if (_profile.value != p) {
            stopReceive()
        }
        _profile.value = p
    }

    fun setVariants(v: DspVariants) {
        // Stop in-flight receive so the new flags actually apply on next start.
        if (_variants.value != v) {
            stopReceive()
        }
        _variants.value = v
    }

    fun send(text: String) {
        if (sendJob?.isActive == true) return
        // Stop receive if active — playing audio while listening on the same
        // device is meaningless and may confuse the user.
        stopReceive()
        sendJob = viewModelScope.launch {
            try {
                val bytes = text.toByteArray(Charsets.UTF_8)
                val tx = FfiTransmitter(_profile.value, _variants.value)
                val samplesList = tx.encode(bytes)
                val samples = FloatArray(samplesList.size).also { arr ->
                    for (i in samplesList.indices) arr[i] = samplesList[i]
                }
                _state.value = UiState.Sending(bytes.size, samples.size / 48_000f)
                withContext(Dispatchers.IO) { AudioPlayer.play(samples) }
                _state.value = UiState.Idle
            } catch (t: Throwable) {
                _state.value = UiState.Error(t.message ?: t.toString())
            }
        }
    }

    fun startReceive() {
        if (receiveJob?.isActive == true) return
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
                        // No samples this tick; still check watchdog below.
                        null
                    }
                    if (chunk != null) {
                        val events = rx.pushSamples(chunk.toList())
                        for (e in events) when (e) {
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

    fun stopReceive() {
        receiveJob?.cancel()
        receiveJob = null
        stopReceiveInternal()
        // If we were in a Receiving state, return to Idle (preserve Result/Error).
        if (_state.value is UiState.Receiving) {
            _state.value = UiState.Idle
        }
    }

    private fun stopReceiveInternal() {
        capture?.stop()
        capture = null
    }

    override fun onCleared() {
        receiveJob?.cancel()
        sendJob?.cancel()
        stopReceiveInternal()
    }
}
