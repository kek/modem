package se.karleklund.modem.audio

import android.Manifest
import android.annotation.SuppressLint
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import androidx.annotation.RequiresPermission
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.runBlocking

class AudioCapture {
    companion object {
        const val SAMPLE_RATE_HZ = 48_000
    }

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private var record: AudioRecord? = null

    /** Emits FloatArray chunks as they arrive. Closed when [stop] is called. */
    val channel: Channel<FloatArray> = Channel(capacity = 64)

    @SuppressLint("MissingPermission")
    @RequiresPermission(Manifest.permission.RECORD_AUDIO)
    fun start() {
        val minBuf = AudioRecord.getMinBufferSize(
            SAMPLE_RATE_HZ,
            AudioFormat.CHANNEL_IN_MONO,
            AudioFormat.ENCODING_PCM_FLOAT,
        )
        val bufSize = maxOf(minBuf, SAMPLE_RATE_HZ * Float.SIZE_BYTES / 4) // ~250 ms
        val rec = AudioRecord.Builder()
            .setAudioSource(MediaRecorder.AudioSource.UNPROCESSED)
            .setAudioFormat(
                AudioFormat.Builder()
                    .setSampleRate(SAMPLE_RATE_HZ)
                    .setChannelMask(AudioFormat.CHANNEL_IN_MONO)
                    .setEncoding(AudioFormat.ENCODING_PCM_FLOAT)
                    .build()
            )
            .setBufferSizeInBytes(bufSize)
            .build()
        record = rec
        rec.startRecording()

        scope.launch {
            val chunk = FloatArray(2400) // 50 ms at 48 kHz
            while (isActive) {
                val n = rec.read(chunk, 0, chunk.size, AudioRecord.READ_BLOCKING)
                if (n > 0) {
                    val out = if (n == chunk.size) chunk.copyOf() else chunk.copyOf(n)
                    val r = channel.trySend(out)
                    if (r.isFailure) {
                        // Channel full — drop oldest by receiving and ignoring.
                        channel.tryReceive()
                        channel.trySend(out)
                    }
                }
            }
        }
    }

    fun stop() {
        record?.stop()
        record?.release()
        record = null
        scope.cancel()
        channel.close()
    }
}
