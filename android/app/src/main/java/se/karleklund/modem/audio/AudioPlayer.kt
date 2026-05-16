package se.karleklund.modem.audio

import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioTrack

object AudioPlayer {
    const val SAMPLE_RATE_HZ = 48_000

    /**
     * Play the given f32 mono samples through the default speaker.
     * Blocks until playback finishes (plus a 500 ms tail to avoid CoreAudio-style truncation).
     */
    fun play(samples: FloatArray) {
        val minBuf = AudioTrack.getMinBufferSize(
            SAMPLE_RATE_HZ,
            AudioFormat.CHANNEL_OUT_MONO,
            AudioFormat.ENCODING_PCM_FLOAT,
        )
        val bufSize = maxOf(minBuf, samples.size * Float.SIZE_BYTES)
        val track = AudioTrack.Builder()
            .setAudioAttributes(
                AudioAttributes.Builder()
                    .setUsage(AudioAttributes.USAGE_MEDIA)
                    .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
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
