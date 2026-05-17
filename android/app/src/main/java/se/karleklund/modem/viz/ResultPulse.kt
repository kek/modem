package se.karleklund.modem.viz

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp

@Composable
fun ResultPulse(ok: Boolean, content: @Composable () -> Unit) {
    val alpha = remember { Animatable(1f) }
    LaunchedEffect(ok) {
        alpha.snapTo(1f)
        alpha.animateTo(0f, animationSpec = tween(1500))
    }
    val color = if (ok) Color(0xFF4CAF50) else Color(0xFFE53935)
    Box(
        modifier = Modifier
            .background(color.copy(alpha = alpha.value * 0.25f))
            .padding(4.dp),
    ) {
        content()
    }
}
