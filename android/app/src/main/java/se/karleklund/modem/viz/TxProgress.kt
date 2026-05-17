package se.karleklund.modem.viz

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

@Composable
fun TxProgress(
    elapsedSec: Float,
    totalSec: Float,
    bytes: Int,
    modifier: Modifier = Modifier,
) {
    val frac = if (totalSec > 0f) (elapsedSec / totalSec).coerceIn(0f, 1f) else 0f
    Column(modifier = modifier) {
        Text("→ $bytes B · ${"%.1f".format(elapsedSec)} / ${"%.1f".format(totalSec)} s")
        LinearProgressIndicator(
            progress = { frac },
            modifier = Modifier.fillMaxWidth().padding(top = 4.dp),
        )
    }
}
