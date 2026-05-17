package se.karleklund.modem.viz

import androidx.compose.animation.core.InfiniteRepeatableSpec
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp

@Composable
fun ListeningIndicator(modifier: Modifier = Modifier) {
    val t = rememberInfiniteTransition(label = "listening")
    val scale by t.animateFloat(
        initialValue = 1.0f,
        targetValue = 1.4f,
        animationSpec = InfiniteRepeatableSpec(
            animation = tween(1000),
            repeatMode = RepeatMode.Reverse,
        ),
        label = "listening-scale",
    )
    val alpha by t.animateFloat(
        initialValue = 1.0f,
        targetValue = 0.2f,
        animationSpec = InfiniteRepeatableSpec(
            animation = tween(1000),
            repeatMode = RepeatMode.Reverse,
        ),
        label = "listening-alpha",
    )
    Row(verticalAlignment = Alignment.CenterVertically, modifier = modifier) {
        Canvas(modifier = Modifier.size(16.dp)) {
            drawCircle(
                color = Color(0xFF1E88E5).copy(alpha = alpha),
                radius = size.minDimension / 2f * scale,
            )
        }
        Text("listening for preamble…", modifier = Modifier)
    }
}
