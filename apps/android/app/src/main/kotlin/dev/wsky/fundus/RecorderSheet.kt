package dev.wsky.fundus

import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Download
import androidx.compose.material.icons.outlined.CollectionsBookmark
import androidx.compose.material.icons.outlined.Schedule
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.FilledTonalIconButton
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearWavyProgressIndicator
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.SegmentedListItem
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId

/**
 * What opens when a DJI mic is plugged in (the Mac's RecorderPrompt): its recordings, newest first,
 * each with its own Import button.
 */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun RecorderSheet(app: FundusApp, onDismiss: () -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val state by app.recorder.state.collectAsStateWithLifecycle()
    val queue by app.uploads.queue.collectAsStateWithLifecycle()
    val history by app.uploads.history.collectAsStateWithLifecycle()
    val waiting by app.uploads.waiting.collectAsStateWithLifecycle()
    val server by app.vault.server.collectAsStateWithLifecycle()
    val records = remember(history, queue, server) { app.uploads.records() }

    // Everything on the mic, newest first, imported ones too.
    val listed = remember(state.sessions) { state.sessions.sortedByDescending { it.start } }
    val proposed = remember(state.sessions, records) { Imports.proposed(state.sessions, records) }
    val showsMic = listed.map { it.transmitter }.toSet().size > 1

    val clock by app.recorder.clock.collectAsStateWithLifecycle()
    // The first time, take the clock the vault's newest recording was imported with (the Mac's setting); it is
    // then kept, like a pick in the footer, until another one is picked.
    val newestZone = app.library.state.collectAsStateWithLifecycle().value.entries.firstOrNull()?.timezone
    LaunchedEffect(newestZone) {
        if (newestZone != null && !app.recorder.clockChosen) runCatching { ZoneId.of(newestZone) }.getOrNull()?.let { app.recorder.setClock(it) }
    }
    // A newly plugged-in mic's newest session goes on top; without this the list keeps showing older rows.
    val list = rememberLazyListState()
    LaunchedEffect(listed.firstOrNull()?.id) { list.scrollToItem(0) }

    var scanned by remember { mutableStateOf(false) }
    LaunchedEffect(Unit) {
        app.recorder.scan()
        scanned = true
        // Like the Mac: every few seconds while something is in flight, otherwise once a minute.
        while (true) {
            app.uploads.refreshHistory()
            val busy = app.uploads.queue.value.isTransferring || app.uploads.records().any { it.stage !in setOf("complete", "failed", "paused") }
            delay(if (busy) 4_000 else 60_000)
        }
    }
    // Unplugged: nothing left to show, unless an upload is still unfinished (its Resume and Discard live here).
    LaunchedEffect(scanned, state.connected, state.scanning, queue.unfinished) {
        if (scanned && !state.scanning && !state.connected && !queue.unfinished) onDismiss()
    }

    val notifications = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) {}
    fun askForNotifications() {
        if (Build.VERSION.SDK_INT >= 33 && context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            notifications.launch(Manifest.permission.POST_NOTIFICATIONS)
        }
    }
    val access = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
        result.data?.data?.let { scope.launch { app.recorder.allow(it) } }
        // So uploads can show their progress outside the app.
        askForNotifications()
    }

    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)) {
        Column(Modifier.padding(horizontal = 24.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(16.dp)) {
                Surface(shape = CircleShape, color = Color(0xFF2D2F36), modifier = Modifier.size(48.dp)) {
                    Icon(painterResource(R.drawable.ic_launcher_foreground), contentDescription = null, tint = Color.White)
                }
                Column {
                    Text(if (state.connected) "DJI Mic connected" else "DJI Mic", style = MaterialTheme.typography.titleLarge)
                    val subtitle = when {
                        state.recorders.isEmpty() && state.scanning -> "Looking for recordings…"
                        !state.connected -> "Unplugged"
                        state.recorders.isEmpty() -> "Fundus needs your permission to read it"
                        proposed.isEmpty() -> "Everything is imported"
                        else -> Recorder.newRecordings(proposed)
                    }
                    Text(subtitle, style = MaterialTheme.typography.bodyMedium.merge(TextStyle(fontFeatureSettings = "tnum")), color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }

            if (state.pending) {
                Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Text("Allow Fundus to read the mic", style = MaterialTheme.typography.titleSmall)
                    Text(
                        "Android asks once. Open the menu, choose the mic’s USB drive and tap “Use this folder”. Nothing on it is changed.",
                        style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                        Button(onClick = { access.launch(app.recorder.accessIntent()) }) { Text("Allow") }
                    }
                }
            }

            if (listed.isNotEmpty()) {
                LazyColumn(Modifier.heightIn(max = 420.dp), state = list, verticalArrangement = Arrangement.spacedBy(ListItemDefaults.SegmentedGap)) {
                    itemsIndexed(listed, key = { _, session -> session.id }) { index, session ->
                        val record = Imports.record(session, records)
                        val partly = record == null && Imports.partlyImported(session, records)
                        SegmentedListItem(
                            shapes = ListItemDefaults.segmentedShapes(index, listed.size),
                            colors = cardColors(),
                            supportingContent = {
                                Text(
                                    listOfNotNull(day(session.start), Format.duration(session.duration), session.transmitter.takeIf { showsMic }, "partly imported".takeIf { partly }).joinToString(" · "),
                                    color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis,
                                )
                            },
                            trailingContent = {
                                Status(record, queued = session.id in waiting.map { it.id }, queue = queue, canImport = server != null) {
                                    askForNotifications()
                                    app.uploads.importNext(session)
                                }
                            },
                        ) {
                            Text(
                                "${Format.time(session.start)}–${Format.time(session.end)}",
                                style = MaterialTheme.typography.titleMedium.merge(TextStyle(fontFeatureSettings = "tnum")),
                                color = if (record?.stage == "complete") MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.onSurface,
                            )
                        }
                    }
                }
            }

            if (queue.files.isNotEmpty() && queue.state != "verified") Transfer(queue, waiting.size, app.uploads)

            Row(verticalAlignment = Alignment.CenterVertically) {
                val caption = MaterialTheme.typography.labelMedium
                val muted = MaterialTheme.colorScheme.onSurfaceVariant
                Icon(Icons.Outlined.CollectionsBookmark, contentDescription = null, Modifier.size(16.dp), tint = muted)
                Text(" " + (server?.let(::host) ?: "Connect a vault"), style = caption, color = muted, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f, fill = false))
                Spacer(Modifier.weight(1f).width(8.dp))
                RecorderClock(clock, onPick = { zone -> scope.launch { app.recorder.setClock(zone) } })
            }
            Spacer(Modifier.navigationBarsPadding())
        }
    }
}

/** The footer's clock; tapping it offers the Mac's two common choices. */
@Composable
private fun RecorderClock(clock: ZoneId, onPick: (ZoneId) -> Unit) {
    var open by remember { mutableStateOf(false) }
    val muted = MaterialTheme.colorScheme.onSurfaceVariant
    Box {
        Row(Modifier.clickable { open = true }.padding(vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            Icon(Icons.Outlined.Schedule, contentDescription = null, Modifier.size(16.dp), tint = muted)
            Text(" Recorder clock ${Format.utcOffset(clock)}", style = MaterialTheme.typography.labelMedium, color = muted, maxLines = 1)
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            val phone = ZoneId.systemDefault()
            listOf("Same as this phone (${Format.utcOffset(phone)})" to phone, "DJI factory setting (UTC+8)" to Recorder.FACTORY_CLOCK).forEach { (label, zone) ->
                DropdownMenuItem(
                    text = { Text(label) },
                    trailingIcon = { if (zone.rules == clock.rules) Icon(Icons.Default.Check, contentDescription = "Selected") },
                    onClick = { open = false; onPick(zone) },
                )
            }
        }
    }
}

/** Where the Import button was, from the moment it is pressed. */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun Status(record: ImportRecord?, queued: Boolean, queue: QueueState, canImport: Boolean, onImport: () -> Unit) {
    val muted = MaterialTheme.colorScheme.onSurfaceVariant
    when {
        record?.stage == "complete" -> Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            Icon(Icons.Default.CheckCircle, contentDescription = null, Modifier.size(18.dp), tint = muted)
            Text("Imported", style = MaterialTheme.typography.labelLarge, color = muted)
        }
        record?.importId != null && queue.state == "verifying" -> Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            LoadingIndicator(Modifier.size(24.dp))
            Text("Checking", style = MaterialTheme.typography.labelLarge, color = muted)
        }
        record != null && record.stage != "uploading" -> Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            if (record.stage !in setOf("failed", "paused")) LoadingIndicator(Modifier.size(24.dp))
            Text(Entry.describe(record.stage, record.activeStage, record.pauseRequested), style = MaterialTheme.typography.labelLarge, color = muted)
        }
        record != null || queued -> {
            // Empty while it waits its turn.
            val progress = if (record?.importId != null) Uploads.progress(queue).toFloat() else 0f
            Box(Modifier.size(40.dp), contentAlignment = Alignment.Center) {
                CircularProgressIndicator(progress = { progress }, Modifier.size(22.dp), strokeWidth = 2.dp)
            }
        }
        else -> FilledTonalIconButton(onClick = onImport, enabled = canImport) { Icon(Icons.Default.Download, "Import") }
    }
}

/** The running upload in one line, with Pause, Resume or Discard (the Mac's TransferSummary). */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun Transfer(queue: QueueState, waiting: Int, uploads: Uploads) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                Uploads.headline(queue, waiting),
                Modifier.weight(1f),
                style = MaterialTheme.typography.bodyMedium,
                color = if (queue.state == "failed") MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurface,
                maxLines = 3,
            )
            if (queue.isTransferring) {
                TextButton(onClick = uploads::pause) { Text("Pause") }
            } else {
                TextButton(onClick = uploads::discard) { Text("Discard") }
                TextButton(onClick = uploads::resume) { Text("Resume") }
            }
        }
        // Everything is sent; how long the vault takes to check it is unknown.
        if (queue.state == "verifying") LinearWavyProgressIndicator(Modifier.fillMaxWidth())
        else if (queue.isTransferring) LinearWavyProgressIndicator(progress = { Uploads.progress(queue).toFloat() }, Modifier.fillMaxWidth())
    }
}

/** `Today`, `Yesterday`, or `Oct 24`. */
private fun day(instant: Instant): String {
    val date = instant.atZone(Format.zone).toLocalDate()
    val today = LocalDate.now(Format.zone)
    return when (date) {
        today -> "Today"
        today.minusDays(1) -> "Yesterday"
        else -> Format.day(instant)
    }
}
