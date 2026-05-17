package se.karleklund.modem.viz

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp

@Composable
fun FrameTimeline(timeline: List<FrameMark>, modifier: Modifier = Modifier) {
    val listState = rememberLazyListState()
    LaunchedEffect(timeline.size) {
        if (timeline.isNotEmpty()) {
            listState.animateScrollToItem(timeline.size - 1)
        }
    }
    var expandedSeq by remember { mutableStateOf<UInt?>(null) }
    Column {
        LazyRow(
            state = listState,
            horizontalArrangement = Arrangement.spacedBy(6.dp),
            contentPadding = PaddingValues(horizontal = 4.dp),
            modifier = modifier,
        ) {
            items(timeline) { m ->
                val bg = when (m.status) {
                    FrameStatus.Ok -> Color(0xFF4CAF50)
                    FrameStatus.Dropped -> Color(0xFFE53935)
                }
                Text(
                    text = "${m.seq}",
                    color = Color.White,
                    style = MaterialTheme.typography.labelSmall,
                    modifier = Modifier
                        .clip(RoundedCornerShape(6.dp))
                        .background(bg)
                        .clickable(enabled = m.status == FrameStatus.Dropped) {
                            expandedSeq = if (expandedSeq == m.seq) null else m.seq
                        }
                        .padding(horizontal = 8.dp, vertical = 4.dp),
                )
            }
        }
        val drop = timeline.firstOrNull { it.seq == expandedSeq }
        if (drop?.reason != null) {
            Text(
                text = "frame ${drop.seq}: ${drop.reason}",
                style = MaterialTheme.typography.bodySmall,
                color = Color(0xFFE53935),
                modifier = Modifier.padding(top = 4.dp),
            )
        }
    }
}
