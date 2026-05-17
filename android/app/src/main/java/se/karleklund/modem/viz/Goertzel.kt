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
