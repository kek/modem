package se.karleklund.modem

import android.Manifest
import android.content.pm.PackageManager
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.viewModels
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import se.karleklund.modem.ui.ModemScreen

class MainActivity : ComponentActivity() {
    private val vm: ModemViewModel by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            MaterialTheme {
                var granted by remember {
                    mutableStateOf(
                        ContextCompat.checkSelfPermission(this, Manifest.permission.RECORD_AUDIO)
                            == PackageManager.PERMISSION_GRANTED
                    )
                }
                var denied by remember { mutableStateOf(false) }
                val launcher = androidx.activity.compose.rememberLauncherForActivityResult(
                    ActivityResultContracts.RequestPermission()
                ) { result ->
                    granted = result
                    denied = !result
                }
                androidx.compose.runtime.LaunchedEffect(Unit) {
                    if (!granted) launcher.launch(Manifest.permission.RECORD_AUDIO)
                }
                if (granted) {
                    ModemScreen(vm)
                } else {
                    Box(Modifier.fillMaxSize().padding(24.dp), contentAlignment = Alignment.Center) {
                        Text(
                            if (denied)
                                "Microphone permission denied. Grant RECORD_AUDIO in Settings to use Receive."
                            else
                                "Requesting microphone permission…"
                        )
                    }
                }
            }
        }
    }
}
