package dev.wsky.fundus

import androidx.browser.customtabs.CustomTabsIntent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.core.net.toUri
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.launch

/** First run: the vault's address, then approval in the browser. */
@Composable
fun ConnectScreen(vault: Vault) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val signIn by vault.signIn.collectAsStateWithLifecycle()
    var address by rememberSaveable { mutableStateOf(vault.lastAddress) }

    fun connect() {
        scope.launch {
            runCatching { vault.authorizationUrl(address) }.onSuccess { url ->
                CustomTabsIntent.Builder().setShowTitle(true).build().launchUrl(context, url.toUri())
            }
        }
    }

    Surface(Modifier.fillMaxSize()) {
        Column(Modifier.safeDrawingPadding().imePadding()) {
            Column(
                Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(horizontal = 24.dp, vertical = 32.dp),
            ) {
                Icon(
                    painterResource(R.drawable.ic_launcher_foreground),
                    contentDescription = null,
                    modifier = Modifier.size(88.dp).padding(bottom = 8.dp),
                    tint = MaterialTheme.colorScheme.onSurface,
                )
                Text("Connect to your vault", style = MaterialTheme.typography.headlineMedium)
                Spacer(Modifier.height(8.dp))
                Text(
                    "Fundus opens your browser so you can approve this phone. The connection is saved on this device only.",
                    style = MaterialTheme.typography.bodyLarge,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Spacer(Modifier.height(32.dp))
                OutlinedTextField(
                    value = address,
                    onValueChange = { address = it },
                    modifier = Modifier.fillMaxWidth(),
                    label = { Text("Server address") },
                    placeholder = { Text("vault.example.com") },
                    singleLine = true,
                    isError = signIn.error != null,
                    supportingText = signIn.error?.let { error -> { Text(error) } },
                    keyboardOptions = KeyboardOptions(
                        capitalization = KeyboardCapitalization.None,
                        autoCorrectEnabled = false,
                        keyboardType = KeyboardType.Uri,
                        imeAction = ImeAction.Go,
                    ),
                    keyboardActions = KeyboardActions(onGo = { if (address.isNotBlank()) connect() }),
                )
                if (signIn.waiting) {
                    Row(Modifier.padding(top = 16.dp), verticalAlignment = Alignment.CenterVertically) {
                        CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
                        Text(
                            "Approve the connection in your browser.",
                            modifier = Modifier.padding(start = 12.dp),
                            style = MaterialTheme.typography.bodyMedium,
                        )
                    }
                }
            }
            Row(
                Modifier.fillMaxWidth().padding(horizontal = 24.dp, vertical = 16.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.End),
            ) {
                if (signIn.waiting) TextButton(onClick = vault::cancelSignIn) { Text("Cancel") }
                Button(onClick = ::connect, enabled = address.isNotBlank()) {
                    Text(if (signIn.waiting) "Open browser again" else "Connect")
                }
            }
        }
    }
}
