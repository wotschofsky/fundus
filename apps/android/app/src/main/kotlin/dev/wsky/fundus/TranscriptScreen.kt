package dev.wsky.fundus

import android.content.Intent
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.AutoAwesome
import androidx.compose.material.icons.filled.Block
import androidx.compose.material.icons.filled.Forward10
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.Pause
import androidx.compose.material.icons.filled.PersonAdd
import androidx.compose.material.icons.filled.PersonRemove
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.Replay10
import androidx.compose.material.icons.filled.Share
import androidx.compose.material.icons.outlined.Edit
import androidx.compose.material.icons.outlined.Info
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.LinearWavyProgressIndicator
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.SegmentedListItem
import androidx.compose.material3.SheetValue
import androidx.compose.material3.SliderState
import androidx.compose.material3.rememberBottomSheetState
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Slider
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** One recording: summary, speakers, transcript and playback. */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun TranscriptScreen(
    model: TranscriptModel,
    library: Library,
    snackbar: SnackbarHostState,
    onBack: () -> Unit,
    onTrash: (Entry) -> Unit,
) {
    val state by model.state.collectAsStateWithLifecycle()
    val playback by model.playback.collectAsStateWithLifecycle()
    val people = library.state.collectAsStateWithLifecycle().value.people
    val entry = state.entry
    val list = rememberLazyListState()
    val scrollBehavior = TopAppBarDefaults.pinnedScrollBehavior()
    val pastTitle by remember { derivedStateOf { list.firstVisibleItemIndex > 0 } }
    var speakerSheet by rememberSaveable { mutableStateOf<String?>(null) }

    LaunchedEffect(model) { model.messages.collect { snackbar.showSnackbar(it) } }

    Scaffold(
        modifier = Modifier.nestedScroll(scrollBehavior.nestedScrollConnection),
        topBar = {
            TopAppBar(
                title = {
                    AnimatedVisibility(pastTitle && entry != null, enter = fadeIn(), exit = fadeOut()) {
                        Text(entry?.displayTitle.orEmpty(), maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                },
                navigationIcon = { IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "Back") } },
                actions = { if (entry != null) TranscriptActions(entry, model, hasRows = state.rows.isNotEmpty(), onTrash = { onTrash(entry) }) },
                scrollBehavior = scrollBehavior,
            )
        },
        bottomBar = {
            // The player arrives with the recording's details: it rises into place rather than popping in.
            val motion = MaterialTheme.motionScheme
            AnimatedVisibility(
                visible = playback.available,
                enter = slideInVertically(motion.defaultSpatialSpec()) { it } + fadeIn(motion.defaultEffectsSpec()),
                exit = fadeOut(motion.fastEffectsSpec()),
            ) {
                PlayerBar(playback, onToggle = model::togglePlay, onSeek = model::seek, onSkip = model::skip)
            }
        },
        snackbarHost = { SnackbarHost(snackbar) },
    ) { padding ->
        when {
            state.loadError != null -> Column(
                Modifier.fillMaxSize().padding(padding).padding(32.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp, Alignment.CenterVertically),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                Text("Transcript unavailable", style = MaterialTheme.typography.titleMedium)
                Text(state.loadError.orEmpty(), textAlign = TextAlign.Center, color = MaterialTheme.colorScheme.onSurfaceVariant)
                val scope = rememberCoroutineScope()
                Button(onClick = { scope.launch { model.refresh() } }) { Text("Try again") }
            }
            entry == null -> Box(Modifier.fillMaxSize().padding(padding), contentAlignment = Alignment.Center) { DelayedLoadingIndicator() }
            else -> TranscriptContent(entry, state, model, list, padding, onSpeaker = { speakerSheet = it })
        }
    }

    speakerSheet?.let { label ->
        SpeakerSheet(
            label = label,
            state = state,
            people = people,
            onDismiss = { speakerSheet = null },
            onAssign = { model.setIdentity(it, label) },
            onCreate = { model.addPerson(it, label) },
            onClear = { locked -> model.setIdentity(null, label, locked) },
        )
    }
}

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun TranscriptContent(
    entry: Entry,
    state: TranscriptModel.State,
    model: TranscriptModel,
    list: androidx.compose.foundation.lazy.LazyListState,
    padding: androidx.compose.foundation.layout.PaddingValues,
    onSpeaker: (String) -> Unit,
) {
    val current by model.currentRow.collectAsStateWithLifecycle()
    val playable = model.playback.collectAsStateWithLifecycle().value.available
    val clockWidth = if (entry.recorded == null || Format.hour24) 64.dp else 88.dp
    val haptics = LocalHapticFeedback.current

    // Opened from a search result: show that passage once the transcript is in.
    var revealed by rememberSaveable { mutableStateOf(model.openAt == null) }
    LaunchedEffect(state.rows.isNotEmpty()) {
        val at = model.openAt ?: return@LaunchedEffect
        if (revealed || state.rows.isEmpty()) return@LaunchedEffect
        val row = state.rows.indexOfFirst { it.id == model.rowAt(at) }
        if (row >= 0) list.scrollToItem(list.layoutInfo.totalItemsCount - 1 - state.rows.size + row)
        revealed = true
    }

    // While playing, keep the spoken turn in view, unless the person is scrolling themselves.
    LaunchedEffect(current) {
        val id = current ?: return@LaunchedEffect
        if (!model.playback.value.playing || list.isScrollInProgress) return@LaunchedEffect
        val info = list.layoutInfo
        val shown = info.visibleItemsInfo.firstOrNull { it.key == id }
        if (shown == null || shown.offset < 0 || shown.offset + shown.size > info.viewportEndOffset - info.afterContentPadding) {
            val row = state.rows.indexOfFirst { it.id == id }
            // Rows are the last items before the closing spacer.
            if (row >= 0) list.animateScrollToItem(info.totalItemsCount - 1 - state.rows.size + row)
        }
    }

    LazyColumn(state = list, modifier = Modifier.fillMaxSize(), contentPadding = padding) {
        item("header") {
            Column(Modifier.padding(start = 16.dp, end = 16.dp, top = 4.dp, bottom = 8.dp)) {
                Text(entry.displayTitle, style = MaterialTheme.typography.headlineSmall)
                Spacer(Modifier.height(4.dp))
                Text(
                    "${entry.dateLine} · ${Format.duration(entry.durationSeconds)}",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                if (entry.labels.isNotEmpty()) {
                    FlowRow(Modifier.padding(top = 12.dp), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        entry.labels.forEach { LabelChip(it) }
                    }
                }
            }
        }
        if (entry.isProcessing || entry.canResume) {
            item("status") { Status(entry, onResume = { model.perform(Library.Action.Resume) }) }
        }
        items(entry.warnings) { Notice(it) }
        if (!entry.summary.isNullOrBlank()) {
            item("summary") {
                Surface(
                    Modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, top = 16.dp),
                    shape = MaterialTheme.shapes.extraLarge,
                    color = MaterialTheme.colorScheme.surfaceContainerHigh,
                ) {
                    Column(Modifier.padding(20.dp)) {
                        Text("Summary", style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        Text(entry.summary, Modifier.padding(top = 8.dp), style = MaterialTheme.typography.bodyLarge)
                    }
                }
            }
        }
        if (state.speakers.isNotEmpty()) {
            item("speakers") { Section("Speakers") }
            itemsIndexed(state.speakers, key = { _, speaker -> "speaker-${speaker.label}" }) { index, speaker ->
                SegmentedListItem(
                    onClick = { onSpeaker(speaker.label) },
                    shapes = ListItemDefaults.segmentedShapes(index, state.speakers.size),
                    colors = cardColors(),
                    enabled = !state.updatingSpeaker,
                    modifier = Modifier.padding(horizontal = 16.dp).padding(bottom = ListItemDefaults.SegmentedGap),
                    supportingContent = speaker.explanations.firstOrNull()?.let { reason -> { Text(reason, maxLines = 2, overflow = TextOverflow.Ellipsis) } }
                        ?: if (speaker.keptUnidentified) { { Text("Kept unidentified") } } else null,
                    trailingContent = { Icon(Icons.Outlined.Edit, "Change") },
                ) { Text(state.name(speaker.label)) }
            }
        }
        item("transcript") { Section("Transcript") }
        if (!state.rowsLoaded) {
            item("rows-loading") { Box(Modifier.fillMaxWidth().padding(24.dp), contentAlignment = Alignment.Center) { DelayedLoadingIndicator() } }
        } else if (state.rows.isEmpty()) {
            item("no-rows") {
                Text(
                    if (entry.isProcessing) "The transcript appears here when it is ready." else "No speech was found.",
                    Modifier.padding(horizontal = 16.dp),
                    style = MaterialTheme.typography.bodyLarge,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
        itemsIndexed(state.rows, key = { _, row -> row.id }) { index, row ->
            // The name only where the speaker changes, as in the web transcript.
            val name = if (index == 0 || state.rows[index - 1].speaker != row.speaker) state.name(row.speaker) else null
            TurnRow(
                row = row,
                name = name,
                current = row.id == current,
                clockWidth = clockWidth,
                onClick = if (playable) {
                    {
                        haptics.performHapticFeedback(HapticFeedbackType.ContextClick)
                        model.seek(row.startMs, play = true)
                    }
                } else null,
            )
        }
        item("end") { Spacer(Modifier.height(24.dp)) }
    }
}

@Composable
private fun Section(title: String) {
    Text(
        title,
        modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 24.dp, bottom = 8.dp),
        style = MaterialTheme.typography.titleSmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
}

@Composable
private fun LabelChip(label: String) {
    Surface(shape = RoundedCornerShape(8.dp), border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant)) {
        Text(label, Modifier.padding(horizontal = 10.dp, vertical = 4.dp), style = MaterialTheme.typography.labelMedium)
    }
}

/** What the vault is doing, or why it stopped, with Resume once it has. */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun Status(entry: Entry, onResume: () -> Unit) {
    Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp)) {
        if (entry.isProcessing) {
            Text(entry.status, style = MaterialTheme.typography.bodyMedium)
            Spacer(Modifier.height(8.dp))
            LinearWavyProgressIndicator(Modifier.fillMaxWidth())
        } else {
            val failed = entry.stage == "failed"
            Text(
                if (failed) entry.failure ?: entry.status else entry.status,
                style = MaterialTheme.typography.bodyMedium,
                color = if (failed) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurface,
            )
            OutlinedButton(onClick = onResume, modifier = Modifier.padding(top = 8.dp)) { Text("Resume processing") }
        }
    }
}

@Composable
private fun Notice(text: String) {
    Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 6.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        Icon(Icons.Outlined.Info, contentDescription = null, modifier = Modifier.size(20.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
        Text(text, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

/** One turn: clock time beside the text, the speaker's name where the speaker changes. Tap to play from here. */
@Composable
private fun TurnRow(row: TranscriptRow, name: String?, current: Boolean, clockWidth: androidx.compose.ui.unit.Dp, onClick: (() -> Unit)?) {
    val tabular = TextStyle(fontFeatureSettings = "tnum")
    Row(
        Modifier.fillMaxWidth()
            .padding(horizontal = 8.dp, vertical = 1.dp)
            .padding(top = if (name != null) 12.dp else 0.dp)
            .clip(RoundedCornerShape(12.dp))
            .background(if (current) MaterialTheme.colorScheme.surfaceContainerHigh else Color.Transparent)
            .then(if (onClick != null) Modifier.clickable(onClick = onClick) else Modifier)
            .padding(horizontal = 8.dp, vertical = 6.dp),
    ) {
        Text(
            row.clock,
            modifier = Modifier.width(clockWidth).padding(top = 3.dp),
            style = MaterialTheme.typography.labelMedium.merge(tabular),
            color = if (current) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Column {
            if (name != null) Text(name, style = MaterialTheme.typography.titleSmall, modifier = Modifier.padding(bottom = 2.dp))
            Text(row.text, style = MaterialTheme.typography.bodyLarge)
        }
    }
}

/** Share and the recording's processing actions. */
@Composable
private fun TranscriptActions(entry: Entry, model: TranscriptModel, hasRows: Boolean, onTrash: () -> Unit) {
    val context = LocalContext.current
    var menu by remember { mutableStateOf(false) }
    if (hasRows) {
        IconButton(onClick = {
            // ponytail: shared as text; a very long transcript could exceed the share size limit and needs a file then.
            val send = Intent(Intent.ACTION_SEND).setType("text/plain")
                .putExtra(Intent.EXTRA_SUBJECT, entry.displayTitle)
                .putExtra(Intent.EXTRA_TEXT, model.markdown())
            context.startActivity(Intent.createChooser(send, null))
        }) { Icon(Icons.Default.Share, "Share transcript") }
    }
    Box {
        IconButton(onClick = { menu = true }) { Icon(Icons.Default.MoreVert, "More") }
        DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
            fun run(action: Library.Action) {
                menu = false
                model.perform(action)
            }
            when {
                entry.isProcessing -> DropdownMenuItem(
                    text = { Text("Pause after this step") },
                    onClick = { run(Library.Action.Pause) },
                    enabled = !entry.pauseRequested,
                )
                entry.canResume -> DropdownMenuItem(text = { Text("Resume processing") }, onClick = { run(Library.Action.Resume) })
                else -> {
                    DropdownMenuItem(text = { Text("Reanalyze speakers") }, onClick = { run(Library.Action.ReanalyzeSpeakers) })
                    DropdownMenuItem(text = { Text("Regenerate summary") }, onClick = { run(Library.Action.RegenerateSummary) })
                }
            }
            HorizontalDivider()
            DropdownMenuItem(
                text = { Text("Move to trash") },
                onClick = {
                    menu = false
                    onTrash()
                },
            )
        }
    }
}

/** Playback for the recording: a floating card with the scrubber and expressive controls. */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun PlayerBar(playback: TranscriptModel.Playback, onToggle: () -> Unit, onSeek: (Long, Boolean) -> Unit, onSkip: (Long) -> Unit) {
    val duration = playback.durationMs.coerceAtLeast(1)
    val slider = remember { SliderState() }
    var dragging by remember { mutableStateOf(false) }
    LaunchedEffect(playback.positionMs, duration, dragging) {
        if (!dragging) slider.value = (playback.positionMs.toFloat() / duration).coerceIn(0f, 1f)
    }
    val tabular = MaterialTheme.typography.labelMedium.merge(TextStyle(fontFeatureSettings = "tnum"))
    Surface(
        Modifier.navigationBarsPadding().padding(horizontal = 12.dp, vertical = 8.dp),
        shape = MaterialTheme.shapes.extraLarge,
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
        shadowElevation = 3.dp,
    ) {
        Column(Modifier.padding(start = 16.dp, end = 16.dp, top = 4.dp, bottom = 12.dp)) {
            Slider(
                state = slider,
                onValueChange = {
                    dragging = true
                    slider.value = it
                },
                onValueChangeFinished = {
                    onSeek((slider.value * duration).toLong(), false)
                    dragging = false
                },
            )
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(Format.position(slider.value * duration / 1000.0), style = tabular, modifier = Modifier.width(56.dp))
                Spacer(Modifier.weight(1f))
                IconButton(onClick = { onSkip(-10_000) }, shapes = IconButtonDefaults.shapes()) { Icon(Icons.Default.Replay10, "Back 10 seconds") }
                // Morphs from round to squarish while pressed, like the system media controls.
                FilledIconButton(
                    onClick = onToggle,
                    shapes = IconButtonDefaults.shapes(),
                    modifier = Modifier.padding(horizontal = 12.dp).size(64.dp),
                ) {
                    Icon(
                        if (playback.playing) Icons.Default.Pause else Icons.Default.PlayArrow,
                        if (playback.playing) "Pause" else "Play",
                        Modifier.size(32.dp),
                    )
                }
                IconButton(onClick = { onSkip(10_000) }, shapes = IconButtonDefaults.shapes()) { Icon(Icons.Default.Forward10, "Forward 10 seconds") }
                Spacer(Modifier.weight(1f))
                Text(Format.position(duration / 1000.0), style = tabular, modifier = Modifier.width(56.dp), textAlign = TextAlign.End)
            }
        }
    }
}

/** Who a speaker is: a saved person, a new one, or nobody. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun SpeakerSheet(
    label: String,
    state: TranscriptModel.State,
    people: List<Person>,
    onDismiss: () -> Unit,
    onAssign: (String) -> Unit,
    onCreate: (String) -> Unit,
    onClear: (locked: Boolean) -> Unit,
) {
    val sheet = rememberBottomSheetState(
        initialValue = SheetValue.Hidden,
        enabledValues = if (people.size < 6) setOf(SheetValue.Hidden, SheetValue.Expanded) else setOf(SheetValue.Hidden, SheetValue.PartiallyExpanded, SheetValue.Expanded),
    )
    val scope = rememberCoroutineScope()
    var naming by remember { mutableStateOf(false) }
    val speaker = state.speaker(label)
    fun done(change: () -> Unit) {
        change()
        scope.launch { sheet.hide() }.invokeOnCompletion { onDismiss() }
    }

    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = sheet) {
        LazyColumn(contentPadding = androidx.compose.foundation.layout.PaddingValues(bottom = 16.dp)) {
            item {
                Column(Modifier.padding(start = 24.dp, end = 24.dp, bottom = 12.dp)) {
                    Text(state.name(label), style = MaterialTheme.typography.headlineSmall)
                    speaker?.explanations?.forEach {
                        Text(it, Modifier.padding(top = 4.dp), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
            }
            items(people, key = { it.id }) { person ->
                ListItem(
                    onClick = { done { onAssign(person.id) } },
                    leadingContent = { RadioButton(selected = speaker?.personId == person.id, onClick = null) },
                    colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                ) { Text(person.name) }
            }
            item {
                ListItem(
                    onClick = { naming = true },
                    leadingContent = { Icon(Icons.Default.PersonAdd, contentDescription = null) },
                    colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                ) { Text("New person") }
                HorizontalDivider(Modifier.padding(vertical = 8.dp))
            }
            if (speaker?.isIdentified == true) {
                item {
                    ListItem(
                        onClick = { done { onClear(false) } },
                        supportingContent = { Text("Fundus may recognize this speaker again.") },
                        leadingContent = { Icon(Icons.Default.PersonRemove, contentDescription = null) },
                        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                    ) { Text("Remove identity") }
                }
            }
            item {
                val kept = speaker?.keptUnidentified == true
                ListItem(
                    onClick = { done { onClear(!kept) } },
                    supportingContent = { Text(if (kept) "Let Fundus name this speaker by voice or introduction." else "Fundus will not name this speaker.") },
                    leadingContent = { Icon(if (kept) Icons.Default.AutoAwesome else Icons.Default.Block, contentDescription = null) },
                    colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                ) { Text(if (kept) "Recognize automatically" else "Keep unidentified") }
            }
        }
    }

    if (naming) {
        NewPersonDialog(onDismiss = { naming = false }, onAdd = { name ->
            naming = false
            done { onCreate(name) }
        })
    }
}

@Composable
private fun NewPersonDialog(onDismiss: () -> Unit, onAdd: (String) -> Unit) {
    var name by rememberSaveable { mutableStateOf("") }
    val focus = remember { FocusRequester() }
    LaunchedEffect(Unit) { focus.requestFocus() }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("New person") },
        text = {
            Column {
                Text("Their voice in this recording is used to recognize them in future recordings.")
                OutlinedTextField(
                    value = name,
                    onValueChange = { name = it },
                    modifier = Modifier.fillMaxWidth().padding(top = 16.dp).focusRequester(focus),
                    label = { Text("Name") },
                    singleLine = true,
                    keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Words, imeAction = ImeAction.Done),
                    keyboardActions = KeyboardActions(onDone = { if (name.isNotBlank()) onAdd(name) }),
                )
            }
        },
        confirmButton = { TextButton(onClick = { onAdd(name) }, enabled = name.isNotBlank()) { Text("Add") } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

/** A loading indicator that appears only if loading takes longer than a moment, so fast loads don't flash. */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun DelayedLoadingIndicator() {
    var shown by remember { mutableStateOf(false) }
    LaunchedEffect(Unit) {
        delay(400)
        shown = true
    }
    if (shown) LoadingIndicator()
}
