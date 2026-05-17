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
