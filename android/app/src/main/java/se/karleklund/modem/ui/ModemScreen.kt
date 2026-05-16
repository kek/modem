package se.karleklund.modem.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.Button
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import se.karleklund.modem.ModemViewModel
import se.karleklund.modem.UiState
import uniffi.modem_ffi.Profile

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ModemScreen(vm: ModemViewModel) {
    val state by vm.state.collectAsStateWithLifecycle()
    val profile by vm.profile.collectAsStateWithLifecycle()
    var text by rememberSaveable { mutableStateOf("hello from Android") }
    var showHex by remember { mutableStateOf(false) }

    Scaffold(topBar = { CenterAlignedTopAppBar(title = { Text("Modem") }) }) { pad ->
        Column(
            modifier = Modifier.padding(pad).padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            // Profile toggle
            val profiles = Profile.entries
            SingleChoiceSegmentedButtonRow(modifier = Modifier.fillMaxWidth()) {
                profiles.forEachIndexed { idx, p ->
                    SegmentedButton(
                        selected = profile == p,
                        onClick = { vm.setProfile(p) },
                        shape = SegmentedButtonDefaults.itemShape(idx, profiles.size),
                    ) {
                        Text(p.name.lowercase().replaceFirstChar { it.uppercase() })
                    }
                }
            }

            OutlinedTextField(
                value = text,
                onValueChange = { text = it },
                label = { Text("Message") },
                modifier = Modifier.fillMaxWidth(),
                minLines = 3,
            )

            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(onClick = { vm.send(text) }, modifier = Modifier.weight(1f)) { Text("Send") }
                val s = state
                if (s is UiState.Receiving) {
                    OutlinedButton(onClick = { vm.stopReceive() }, modifier = Modifier.weight(1f)) { Text("Stop") }
                } else {
                    OutlinedButton(onClick = { vm.startReceive() }, modifier = Modifier.weight(1f)) { Text("Receive") }
                }
            }

            // Status / result
            when (val s = state) {
                UiState.Idle -> Text("Ready")
                is UiState.Sending -> Text("Sending ${s.bytes} B (${"%.1f".format(s.seconds)} s)…")
                is UiState.Receiving -> Text("Listening… ${s.framesOk} frame(s) ok")
                is UiState.Error -> Text("Error: ${s.message}")
                is UiState.Result -> ResultView(s.bytes, s.sha256Ok, showHex, onToggle = { showHex = !showHex })
            }
        }
    }
}

@Composable
private fun ResultView(bytes: ByteArray, sha256Ok: Boolean, showHex: Boolean, onToggle: () -> Unit) {
    // Auto-detect once per `bytes` reference; toggle only flips view mode.
    val printable = remember(bytes) { isPrintable(bytes) }
    val rendered = remember(bytes, showHex, printable) {
        if (showHex || !printable) hexDump(bytes) else String(bytes, Charsets.UTF_8)
    }
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text("${bytes.size} bytes · sha256 ${if (sha256Ok) "ok" else "MISMATCH"}")
        Surface(tonalElevation = 4.dp, modifier = Modifier.fillMaxWidth()) {
            BasicTextField(
                value = rendered,
                onValueChange = {},
                readOnly = true,
                modifier = Modifier.padding(8.dp).fillMaxWidth(),
            )
        }
        TextButton(onClick = onToggle) { Text(if (showHex) "View as text" else "View as hex") }
    }
}

private fun isPrintable(b: ByteArray): Boolean {
    return try {
        val s = String(b, Charsets.UTF_8)
        s.all { c -> !c.isISOControl() || c == '\t' || c == '\n' || c == '\r' }
    } catch (_: Throwable) {
        false
    }
}

private fun hexDump(b: ByteArray): String = buildString {
    var i = 0
    while (i < b.size) {
        val row = b.copyOfRange(i, minOf(i + 16, b.size))
        append("%08x  ".format(i))
        append(row.joinToString(" ") { "%02x".format(it) }.padEnd(48))
        append("  ")
        append(row.joinToString("") { byte ->
            val c = byte.toInt() and 0xFF
            if (c in 0x20..0x7E) c.toChar().toString() else "."
        })
        append('\n')
        i += 16
    }
}
