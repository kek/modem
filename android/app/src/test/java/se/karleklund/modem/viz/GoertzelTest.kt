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
