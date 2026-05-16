package se.karleklund.modem.audio

import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioTrack

object AudioPlayer {
    const val SAMPLE_RATE_HZ = 48_000

    /**
     * Play the given f32 mono samples through the default speaker.
     * Blocks until playback finishes (plus a 500 ms tail to avoid truncation).
     *
     * Audio attributes deliberately bypass system "dynamics processing":
     * USAGE_MEDIA + CONTENT_TYPE_SONIFICATION causes Android (at least on
     * Pixel) to apply a Dynamics Processing Effect on the output stream,
     * which compresses/limits our signal and breaks the FSK envelope.
     * USAGE_VOICE_COMMUNICATION + CONTENT_TYPE_SPEECH still goes through
     * voice-call processing (AEC, AGC) -- also bad.
     * USAGE_UNKNOWN + CONTENT_TYPE_UNKNOWN + PERFORMANCE_MODE_LOW_LATENCY
     * is the closest to a raw passthrough path. Verified by logcat: no
     * EffectConversionHelperAidl "Dynamics Processing Effect" sets up.
     */
    fun play(samples: FloatArray) {
        val minBuf = AudioTrack.getMinBufferSize(
            SAMPLE_RATE_HZ,
            AudioFormat.CHANNEL_OUT_MONO,
            AudioFormat.ENCODING_PCM_FLOAT,
        )
        // Keep buffer modest; AudioTrack streams further data on demand.
        val bufSize = maxOf(minBuf, 8 * 1024)
        val track = AudioTrack.Builder()
            .setAudioAttributes(
                AudioAttributes.Builder()
                    .setUsage(AudioAttributes.USAGE_UNKNOWN)
                    .setContentType(AudioAttributes.CONTENT_TYPE_UNKNOWN)
                    .build()
            )
            .setAudioFormat(
                AudioFormat.Builder()
                    .setSampleRate(SAMPLE_RATE_HZ)
                    .setChannelMask(AudioFormat.CHANNEL_OUT_MONO)
                    .setEncoding(AudioFormat.ENCODING_PCM_FLOAT)
                    .build()
            )
            .setBufferSizeInBytes(bufSize)
            .setTransferMode(AudioTrack.MODE_STREAM)
            .setPerformanceMode(AudioTrack.PERFORMANCE_MODE_LOW_LATENCY)
            .build()

        track.play()
        try {
            track.write(samples, 0, samples.size, AudioTrack.WRITE_BLOCKING)
            // Drain
            Thread.sleep(500)
        } finally {
            track.stop()
            track.release()
        }
    }
}
