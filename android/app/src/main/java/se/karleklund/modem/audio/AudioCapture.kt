package se.karleklund.modem.audio

import android.Manifest
import android.annotation.SuppressLint
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import android.util.Log
import androidx.annotation.RequiresPermission
import java.io.DataOutputStream
import java.io.File
import java.io.FileOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder
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

        // Diagnostic: dump all captured samples to /sdcard/Download/mic-<ts>.wav
        val wavFile = File("/sdcard/Download/mic-${System.currentTimeMillis()}.wav")
        val wavOut = DataOutputStream(FileOutputStream(wavFile))
        writeWavHeader(wavOut, SAMPLE_RATE_HZ)
        Log.i("ModemAudioCapture", "writing WAV to ${wavFile.absolutePath}")

        scope.launch {
            val chunk = FloatArray(2400) // 50 ms at 48 kHz
            var chunkIdx = 0
            while (isActive) {
                val n = rec.read(chunk, 0, chunk.size, AudioRecord.READ_BLOCKING)
                if (n > 0) {
                    val out = if (n == chunk.size) chunk.copyOf() else chunk.copyOf(n)
                    // Append raw samples to WAV (little-endian float32)
                    val bb = ByteBuffer.allocate(n * 4).order(ByteOrder.LITTLE_ENDIAN)
                    for (i in 0 until n) bb.putFloat(out[i])
                    wavOut.write(bb.array())
                    // Diagnostic: log RMS + peak every ~500 ms so we can verify the mic is hot
                    if (chunkIdx % 10 == 0) {
                        var sumSq = 0.0
                        var peak = 0f
                        for (i in 0 until n) {
                            val v = out[i]
                            sumSq += (v * v).toDouble()
                            if (kotlin.math.abs(v) > peak) peak = kotlin.math.abs(v)
                        }
                        val rms = kotlin.math.sqrt(sumSq / n)
                        Log.i("ModemAudioCapture", "chunk=%d n=%d rms=%.4f peak=%.4f".format(chunkIdx, n, rms, peak))
                    }
                    chunkIdx++
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

    private fun writeWavHeader(out: DataOutputStream, sampleRate: Int) {
        val bitsPerSample = 32
        val channels = 1
        val byteRate = sampleRate * channels * bitsPerSample / 8
        val blockAlign = channels * bitsPerSample / 8
        val audioFormat = 3 // IEEE float
        val header = ByteBuffer.allocate(44).order(ByteOrder.LITTLE_ENDIAN)
        header.put("RIFF".toByteArray())
        header.putInt(0x7FFFFFFE)
        header.put("WAVE".toByteArray())
        header.put("fmt ".toByteArray())
        header.putInt(16)
        header.putShort(audioFormat.toShort())
        header.putShort(channels.toShort())
        header.putInt(sampleRate)
        header.putInt(byteRate)
        header.putShort(blockAlign.toShort())
        header.putShort(bitsPerSample.toShort())
        header.put("data".toByteArray())
        header.putInt(0x7FFFFFFE)
        out.write(header.array())
        out.flush()
    }
}
