package dev.wsky.fundus

import androidx.browser.customtabs.CustomTabsIntent
import androidx.compose.animation.core.animateDpAsState
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.windowInsetsTopHeight
import androidx.compose.material3.SearchBar
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.semantics.isTraversalGroup
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.traversalIndex
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.Logout
import androidx.compose.material.icons.automirrored.filled.OpenInNew
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.ErrorOutline
import androidx.compose.material.icons.outlined.MoveToInbox
import androidx.compose.material.icons.filled.Search
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemColors
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.ListItemShapes
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SearchBarDefaults
import androidx.compose.material3.SegmentedListItem
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.Surface
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.SwipeToDismissBox
import androidx.compose.material3.SwipeToDismissBoxValue
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.material3.pulltorefresh.PullToRefreshDefaults
import androidx.compose.material3.pulltorefresh.rememberPullToRefreshState
import androidx.compose.material3.rememberSwipeToDismissBoxState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.core.net.toUri
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.repeatOnLifecycle
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.filter
import kotlinx.coroutines.launch
import java.net.URI

/** The collapsed search bar's height below the status bar, including its margins. */
@OptIn(ExperimentalMaterial3Api::class)
private val SearchBarHeight = SearchBarDefaults.InputFieldHeight + 16.dp

/** The vault's recordings, newest first, in one rounded group per day. */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun LibraryScreen(
    library: Library,
    vault: Vault,
    snackbar: SnackbarHostState,
    onOpen: (String, Long?) -> Unit,
    onTrash: (Entry) -> Unit,
) {
    val state by library.state.collectAsStateWithLifecycle()
    val server by vault.server.collectAsStateWithLifecycle()
    val app = LocalContext.current.fundus
    val recorder by app.recorder.state.collectAsStateWithLifecycle()
    val upload by app.uploads.queue.collectAsStateWithLifecycle()
    val scope = rememberCoroutineScope()
    var refreshing by remember { mutableStateOf(false) }
    var vaultSheet by remember { mutableStateOf(false) }
    var confirmDisconnect by remember { mutableStateOf(false) }
    val list = rememberLazyListState()
    val days = remember(state.entries) { state.entries.groupBy { it.date.atZone(Format.zone).toLocalDate() } }

    var query by rememberSaveable { mutableStateOf("") }
    var expanded by rememberSaveable { mutableStateOf(false) }
    val margin by animateDpAsState(if (expanded) 0.dp else 16.dp, label = "search margin")
    fun closeSearch() {
        expanded = false
        query = ""
    }

    // Like the Mac: every few seconds while the vault is working, otherwise once a minute;
    // only while the app is on screen.
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    LaunchedEffect(lifecycle) {
        lifecycle.repeatOnLifecycle(Lifecycle.State.STARTED) {
            while (true) {
                library.refresh()
                delay(if (library.state.value.isProcessing) 5_000 else 60_000)
            }
        }
    }

    Scaffold(snackbarHost = { SnackbarHost(snackbar) }, contentWindowInsets = WindowInsets(0)) { padding ->
        Box(Modifier.fillMaxSize().padding(padding).semantics { isTraversalGroup = true }) {
            Column(Modifier.fillMaxSize()) {
                // Room for the search bar, which stays pinned above the list and grows over it.
                Spacer(Modifier.windowInsetsTopHeight(WindowInsets.statusBars))
                Spacer(Modifier.height(SearchBarHeight))
                val pull = rememberPullToRefreshState()
                PullToRefreshBox(
                    isRefreshing = refreshing,
                    onRefresh = {
                        scope.launch {
                            refreshing = true
                            library.refresh()
                            refreshing = false
                        }
                    },
                    state = pull,
                    modifier = Modifier.fillMaxSize(),
                    indicator = { PullToRefreshDefaults.LoadingIndicator(state = pull, isRefreshing = refreshing, modifier = Modifier.align(Alignment.TopCenter)) },
                ) {
                    LazyColumn(
                        state = list,
                        modifier = Modifier.fillMaxSize(),
                        contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 16.dp + WindowInsets.navigationBars.asPaddingValues().calculateBottomPadding()),
                        verticalArrangement = Arrangement.spacedBy(ListItemDefaults.SegmentedGap),
                    ) {
                        if (state.entries.isEmpty()) {
                            item { EmptyLibrary(state, Modifier.fillParentMaxSize()) { scope.launch { library.refresh() } } }
                        }
                        days.forEach { (day, entries) ->
                            // The day stays at the top while its recordings scroll under it.
                            stickyHeader(key = "day-$day") {
                                GroupHeader(Format.dayHeader(day), Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.surface))
                            }
                            itemsIndexed(entries, key = { _, entry -> entry.id }) { index, entry ->
                                val shapes = ListItemDefaults.segmentedShapes(index, entries.size)
                                SwipeToTrash(shape = shapes.shape, onTrash = { onTrash(entry) }, modifier = Modifier.animateItem()) {
                                    EntryRow(entry, shapes, onClick = { onOpen(entry.id, null) })
                                }
                            }
                        }
                        if (state.hasMore) {
                            item(key = "more") {
                                LaunchedEffect(Unit) { library.loadMore() }
                                Box(Modifier.fillMaxWidth().padding(16.dp), contentAlignment = Alignment.Center) { LoadingIndicator() }
                            }
                        }
                    }
                }
            }

            // ponytail: the classic search bar is deprecated, but its in-place growth is the motion we
            // want; the newer contained bar opens as a separate full-screen layer instead.
            @Suppress("DEPRECATION")
            SearchBar(
                inputField = {
                    @Suppress("DEPRECATION")
                    SearchBarDefaults.InputField(
                        query = query,
                        onQueryChange = { query = it },
                        onSearch = {},
                        expanded = expanded,
                        onExpandedChange = { if (it) expanded = true else closeSearch() },
                        placeholder = { Text("Search recordings") },
                        leadingIcon = {
                            if (expanded) {
                                IconButton(onClick = ::closeSearch) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "Back") }
                            } else {
                                Icon(Icons.Default.Search, contentDescription = null)
                            }
                        },
                        trailingIcon = {
                            when {
                                expanded && query.isNotEmpty() -> IconButton(onClick = { query = "" }) { Icon(Icons.Default.Close, "Clear") }
                                // Like the account picture in Google's apps: the vault this phone is connected to.
                                !expanded -> Row(verticalAlignment = Alignment.CenterVertically) {
                                    // A mic plugged in before the app opened, or an upload not finished: its window, on request.
                                    if (recorder.connected || upload.unfinished) IconButton(onClick = { app.recorder.prompt.value = true }) { Icon(Icons.Outlined.MoveToInbox, "Import from DJI Mic") }
                                    IconButton(onClick = { vaultSheet = true }) { VaultAvatar(server) }
                                }
                            }
                        },
                    )
                },
                expanded = expanded,
                onExpandedChange = { if (it) expanded = true else closeSearch() },
                modifier = Modifier.align(Alignment.TopCenter).padding(horizontal = margin).semantics { traversalIndex = -1f },
            ) {
                SearchResults(library, query) { id, at ->
                    closeSearch()
                    onOpen(id, at)
                }
            }
        }
    }

    if (vaultSheet) {
        VaultSheet(server, onDismiss = { vaultSheet = false }, onDisconnect = {
            vaultSheet = false
            confirmDisconnect = true
        })
    }
    if (confirmDisconnect) {
        AlertDialog(
            onDismissRequest = { confirmDisconnect = false },
            title = { Text("Disconnect this phone?") },
            text = { Text("Fundus forgets the connection to ${host(server)}. Your recordings stay in the vault.") },
            confirmButton = {
                TextButton(onClick = {
                    confirmDisconnect = false
                    scope.launch { vault.signOut() }
                }) { Text("Disconnect") }
            },
            dismissButton = { TextButton(onClick = { confirmDisconnect = false }) { Text("Cancel") } },
        )
    }
}

/** Rounded list groups on the white page: a light grey card, darker in dark mode. */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun cardColors(): ListItemColors = ListItemDefaults.segmentedColors(containerColor = MaterialTheme.colorScheme.surfaceContainerHigh)

@Composable
fun GroupHeader(title: String, modifier: Modifier = Modifier) {
    Text(
        title,
        modifier = modifier.padding(start = 16.dp, end = 16.dp, top = 20.dp, bottom = 8.dp),
        style = MaterialTheme.typography.titleSmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
}

/**
 * Recordings whose title, summary or labels match right away; then passages inside transcripts
 * from the vault's own search (keywords and meaning), once typing pauses.
 */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun SearchResults(library: Library, query: String, onOpen: (String, Long?) -> Unit) {
    val state by library.state.collectAsStateWithLifecycle()
    val recordings = remember(query, state.entries) { library.matching(query).take(5) }
    var hits by remember { mutableStateOf<List<SearchHit>?>(null) }
    var failure by remember { mutableStateOf<String?>(null) }
    val searchable = query.trim().length >= 2

    LaunchedEffect(query) {
        hits = null
        failure = null
        if (!searchable) return@LaunchedEffect
        delay(400) // each search may embed the query on the vault
        try {
            hits = library.search(query)
        } catch (error: CancellationException) {
            throw error
        } catch (error: Exception) {
            failure = error.describe()
            hits = emptyList()
        }
    }

    // The page's own background, so result cards stand out exactly as they do in the list.
    LazyColumn(
        Modifier.fillMaxSize().background(MaterialTheme.colorScheme.surface).imePadding(),
        contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 16.dp),
        verticalArrangement = Arrangement.spacedBy(ListItemDefaults.SegmentedGap),
    ) {
        if (query.isBlank()) {
            item { Hint("Search titles, summaries and everything that was said.") }
            return@LazyColumn
        }
        if (recordings.isNotEmpty()) {
            item("recordings") { GroupHeader("Recordings") }
            itemsIndexed(recordings, key = { _, entry -> entry.id }) { index, entry ->
                EntryRow(entry, ListItemDefaults.segmentedShapes(index, recordings.size), onClick = { onOpen(entry.id, null) }, showDay = true)
            }
        }
        when {
            searchable && hits == null -> item("searching") {
                Box(Modifier.fillMaxWidth().padding(24.dp), contentAlignment = Alignment.Center) { LoadingIndicator() }
            }
            !hits.isNullOrEmpty() -> passages(hits.orEmpty(), query, state.entries, onOpen)
            failure != null -> item { Hint(failure.orEmpty()) }
            recordings.isEmpty() && hits != null -> item { Hint("Nothing matches “${query.trim()}”.") }
        }
    }
}

/** Matching passages, one rounded group per recording in the vault's ranking order. */
private fun LazyListScope.passages(hits: List<SearchHit>, query: String, entries: List<Entry>, onOpen: (String, Long?) -> Unit) {
    item("passages") { GroupHeader("In transcripts") }
    hits.groupBy { it.entryId }.forEach { (entryId, found) ->
        val entry = entries.firstOrNull { it.id == entryId }
        val count = found.size + 1
        item(key = "hit-recording-$entryId") {
            RecordingHeader(found.first(), entry, ListItemDefaults.segmentedShapes(0, count)) { onOpen(entryId, null) }
        }
        itemsIndexed(found, key = { _, hit -> "hit-${hit.id}" }) { index, hit ->
            PassageRow(hit, query, ListItemDefaults.segmentedShapes(index + 1, count)) { onOpen(entryId, hit.startMs) }
        }
        item(key = "hit-gap-$entryId") { Spacer(Modifier.height(10.dp)) }
    }
}

/** The top of a recording's group: which recording, and when. */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun RecordingHeader(hit: SearchHit, entry: Entry?, shapes: ListItemShapes, onClick: () -> Unit) {
    val title = entry?.displayTitle ?: hit.title.trim().ifEmpty { "Untitled recording" }
    val start = entry?.recorded ?: hit.recordedAt?.let(::instant)
    SegmentedListItem(
        onClick = onClick,
        shapes = shapes,
        colors = ListItemDefaults.segmentedColors(containerColor = MaterialTheme.colorScheme.surfaceContainerHighest),
        supportingContent = start?.let { { Text("${Format.day(it)} · ${Format.time(it)}") } },
    ) {
        Text(title, style = MaterialTheme.typography.titleMedium, maxLines = 2, overflow = TextOverflow.Ellipsis)
    }
}

/** One matching passage: a time chip and the words around the match, with the searched words in bold. */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun PassageRow(hit: SearchHit, query: String, shapes: ListItemShapes, onClick: () -> Unit) {
    SegmentedListItem(
        onClick = onClick,
        shapes = shapes,
        colors = cardColors(),
        verticalAlignment = Alignment.Top,
        leadingContent = {
            // Same width for every time, so the excerpts line up.
            Surface(shape = CircleShape, color = MaterialTheme.colorScheme.surfaceContainerHighest, contentColor = MaterialTheme.colorScheme.onSurface) {
                Text(
                    Format.position(hit.startMs / 1000.0),
                    Modifier.widthIn(min = 56.dp).padding(horizontal = 8.dp, vertical = 4.dp),
                    style = MaterialTheme.typography.labelMedium.merge(TextStyle(fontFeatureSettings = "tnum")),
                    textAlign = TextAlign.Center,
                )
            }
        },
    ) {
        Text(
            emphasized(hit.excerpt.trim(), query),
            style = MaterialTheme.typography.bodyMedium,
            maxLines = 4,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

@Composable
private fun Hint(text: String) {
    Text(
        text,
        modifier = Modifier.fillMaxWidth().padding(vertical = 24.dp, horizontal = 8.dp),
        style = MaterialTheme.typography.bodyLarge,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
}

/** The excerpt with the searched words in bold, as on the web. */
private fun emphasized(text: String, query: String): AnnotatedString {
    val words = query.split(Regex("\\s+")).map { it.trim('"', '\'', '.', ',') }.filter { it.length >= 2 }
    return buildAnnotatedString {
        append(text)
        for (word in words) {
            var at = text.indexOf(word, ignoreCase = true)
            while (at >= 0) {
                addStyle(SpanStyle(fontWeight = FontWeight.SemiBold), at, at + word.length)
                at = text.indexOf(word, at + word.length, ignoreCase = true)
            }
        }
    }
}

/** One recording: its title, when, and how long (or what the vault is doing). */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun EntryRow(
    entry: Entry,
    shapes: ListItemShapes,
    onClick: () -> Unit,
    showDay: Boolean = false,
    colors: ListItemColors = cardColors(),
) {
    val failed = entry.stage == "failed"
    val untitled = entry.displayTitle == entry.dateLine
    // Under a day heading an untitled recording is known by its time alone.
    val title = entry.recorded?.takeIf { untitled && !showDay }?.let { Format.time(it) } ?: entry.displayTitle
    val time = entry.recorded?.takeUnless { untitled }?.let { if (showDay) entry.dateLine else Format.time(it) }
    val line = if (entry.stage == "complete") time else listOfNotNull(time, entry.status).joinToString(" · ")
    SegmentedListItem(
        onClick = onClick,
        shapes = shapes,
        colors = colors,
        supportingContent = line?.let {
            { Text(it, color = if (failed) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis) }
        },
        // Lengths line up on the right, so the list reads as title, time, length.
        trailingContent = {
            when {
                entry.isProcessing -> LoadingIndicator(Modifier.size(32.dp))
                failed -> Icon(Icons.Default.ErrorOutline, "Processing stopped", tint = MaterialTheme.colorScheme.error)
                else -> Text(
                    Format.duration(entry.durationSeconds),
                    style = MaterialTheme.typography.labelLarge.merge(TextStyle(fontFeatureSettings = "tnum")),
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        },
    ) {
        Text(
            title,
            style = MaterialTheme.typography.titleMedium,
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

/** Swipe a row towards the start to move the recording to the trash. */
@Composable
private fun SwipeToTrash(shape: Shape, onTrash: () -> Unit, modifier: Modifier = Modifier, content: @Composable () -> Unit) {
    val state = rememberSwipeToDismissBoxState()
    val haptics = LocalHapticFeedback.current
    LaunchedEffect(state) {
        // The list keeps a row's swipe by recording id, so a restored recording (Undo) would come
        // back already swiped away and be trashed again. It comes back in place instead.
        if (state.currentValue != SwipeToDismissBoxValue.Settled) state.snapTo(SwipeToDismissBoxValue.Settled)
        snapshotFlow { state.currentValue }.filter { it == SwipeToDismissBoxValue.EndToStart }.collect { onTrash() }
    }
    // A tick as the swipe crosses the point where letting go trashes the recording.
    LaunchedEffect(state) {
        snapshotFlow { state.targetValue }.filter { it == SwipeToDismissBoxValue.EndToStart }
            .collect { haptics.performHapticFeedback(HapticFeedbackType.GestureThresholdActivate) }
    }
    SwipeToDismissBox(
        state = state,
        modifier = modifier.clip(shape),
        enableDismissFromStartToEnd = false,
        backgroundContent = {
            val active = state.dismissDirection == SwipeToDismissBoxValue.EndToStart
            Box(
                Modifier.fillMaxSize()
                    .background(if (active) MaterialTheme.colorScheme.errorContainer else Color.Transparent)
                    .padding(horizontal = 24.dp),
                contentAlignment = Alignment.CenterEnd,
            ) {
                if (active) Icon(Icons.Default.Delete, "Move to trash", tint = MaterialTheme.colorScheme.onErrorContainer)
            }
        },
    ) { content() }
}

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun EmptyLibrary(state: Library.State, modifier: Modifier, onRetry: () -> Unit) {
    Column(
        modifier.padding(32.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp, Alignment.CenterVertically),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        when {
            state.error != null -> {
                Text("Could not load recordings", style = MaterialTheme.typography.titleMedium)
                Text(state.error, textAlign = TextAlign.Center, color = MaterialTheme.colorScheme.onSurfaceVariant)
                Button(onClick = onRetry) { Text("Try again") }
            }
            !state.loaded -> LoadingIndicator()
            else -> {
                Text("No recordings yet", style = MaterialTheme.typography.titleMedium)
                Text(
                    "Recordings you upload to your vault appear here.",
                    textAlign = TextAlign.Center,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}

/** The vault's initial in a circle, where Google's apps show the account picture. */
@Composable
private fun VaultAvatar(server: String?, size: Dp = 32.dp) {
    Box(Modifier.size(size).clip(CircleShape).background(MaterialTheme.colorScheme.primary), contentAlignment = Alignment.Center) {
        Text(
            host(server).firstOrNull()?.uppercase() ?: "F",
            style = if (size > 40.dp) MaterialTheme.typography.headlineSmall else MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.onPrimary,
        )
    }
}

/** Which vault this phone is connected to, with the web library and disconnecting. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun VaultSheet(server: String?, onDismiss: () -> Unit, onDisconnect: () -> Unit) {
    val context = LocalContext.current
    ModalBottomSheet(onDismissRequest = onDismiss) {
        Column(Modifier.fillMaxWidth().padding(bottom = 16.dp), horizontalAlignment = Alignment.CenterHorizontally) {
            VaultAvatar(server, size = 64.dp)
            Text(host(server), Modifier.padding(top = 12.dp), style = MaterialTheme.typography.titleLarge)
            Text("Connected with full access", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        HorizontalDivider()
        ListItem(
            onClick = {
                server?.let { CustomTabsIntent.Builder().build().launchUrl(context, it.toUri()) }
                onDismiss()
            },
            leadingContent = { Icon(Icons.AutoMirrored.Filled.OpenInNew, contentDescription = null) },
            colors = ListItemDefaults.colors(containerColor = Color.Transparent),
        ) { Text("Open web library") }
        ListItem(
            onClick = onDisconnect,
            leadingContent = { Icon(Icons.AutoMirrored.Filled.Logout, contentDescription = null) },
            colors = ListItemDefaults.colors(containerColor = Color.Transparent),
        ) { Text("Disconnect this phone") }
        Spacer(Modifier.navigationBarsPadding().height(16.dp))
    }
}

/** `fundus.example.com` for `https://fundus.example.com`. */
fun host(server: String?): String = server?.let { runCatching { URI(it).host }.getOrNull() } ?: server.orEmpty()
