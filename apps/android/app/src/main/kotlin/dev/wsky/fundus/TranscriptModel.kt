package dev.wsky.fundus

import android.content.ComponentName
import androidx.core.content.ContextCompat
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.PlaybackException
import androidx.media3.common.MediaMetadata
import androidx.media3.common.Player
import androidx.media3.session.MediaController
import androidx.media3.session.SessionToken
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** One transcript row, with its clock time worked out once so scrolling stays cheap. */
data class TranscriptRow(val id: String, val speaker: String, val startMs: Long, val clock: String, val text: String)

/** A recording opened from the vault: transcript, speakers, playback and edits (the Mac's `TranscriptModel`). */
/** `openAt` is a position to open at, e.g. from a search result. */
class TranscriptModel(private val app: FundusApp, val entryId: String, val openAt: Long? = null) : ViewModel() {
    private val vault = app.vault
    private val library = app.library

    data class State(
        val entry: Entry? = null,
        val rows: List<TranscriptRow> = emptyList(),
        val loadError: String? = null,
        val updatingSpeaker: Boolean = false,
    ) {
        val speakers get() = entry?.speakers.orEmpty()
        fun speaker(label: String) = speakers.firstOrNull { it.label == label }
        fun name(label: String) = speakers.nameOf(label)
    }

    data class Playback(val available: Boolean = false, val playing: Boolean = false, val positionMs: Long = 0, val durationMs: Long = 0)

    // Empty until the details and transcript are both read, so the screen opens in one piece.
    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    /** Kept apart from `state` so playback ticks redraw only the player and the spoken row. */
    private val _playback = MutableStateFlow(Playback())
    val playback: StateFlow<Playback> = _playback.asStateFlow()
    private val _currentRow = MutableStateFlow<String?>(null)
    val currentRow: StateFlow<String?> = _currentRow.asStateFlow()

    private val _messages = Channel<String>(Channel.BUFFERED)
    /** One-off messages for a snackbar. */
    val messages = _messages.receiveAsFlow()

    /** The vault's key for the segments `rows` came from (hex); sent back, unchanged ones are not resent. */
    private var rowsKey: String? = null
    private var watcher: Job? = null
    private var ticker: Job? = null
    private var player: MediaController? = null
    private var connecting = false
    private var cleared = false

    init { load() }

    /** Also "Try again": a recording still processing is followed from there, as on opening. */
    fun load() {
        // People are read beside the recording, not after it.
        viewModelScope.launch { library.refreshPeople() }
        viewModelScope.launch {
            refresh()
            watch()
        }
    }

    // MARK: Loading

    suspend fun refresh() {
        try {
            // The recording and its transcript in one read: neither waits for the other.
            val held = rowsKey?.let { "?transcript_key=$it" }.orEmpty()
            val text = vault.request("/entries/${segment(entryId)}$held")
            val detail = decode<Entry>(text)
            val transcript = decode<TranscriptPart>(text)
            // Refreshes overlap (the 3 s poll, an edit's, "Try again"): an older answer that a newer one overtook is dropped.
            if (!transcript.fits(rowsKey)) return
            // Left out when unchanged. The key covers the recording's time too, which moves every clock time.
            val rows = transcript.segments?.let { rows(detail, it) }
            rowsKey = transcript.transcriptKey
            // Shown together: the recording appears once its transcript is in, not before.
            _state.update { it.copy(entry = detail, rows = rows ?: it.rows, loadError = null) }
            if (detail.audioAvailable && player == null) startPlayer(detail)
            if (rows != null && openAt != null && _currentRow.value == null) _currentRow.value = rowAt(openAt)
        } catch (error: CancellationException) {
            throw error
        } catch (error: Exception) {
            if (_state.value.entry == null) _state.update { it.copy(loadError = error.describe()) }
            else _messages.trySend(error.describe())
        }
    }

    /** While the vault is still working on the recording, follow its progress. */
    private fun watch() {
        if (watcher?.isActive == true) return
        watcher = viewModelScope.launch {
            while (_state.value.entry?.isProcessing == true) {
                delay(3_000)
                refresh()
            }
        }
    }

    private suspend fun rows(entry: Entry, segments: List<Segment>): List<TranscriptRow> {
        return withContext(Dispatchers.Default) {
            val timeline = SessionTimeline.of(entry)
            val dated = entry.recorded != null
            segments.map {
                val clock = if (dated) Format.time(timeline.at(it.startMs), seconds = true) else Format.position(it.startMs / 1000.0)
                TranscriptRow(it.id, it.speaker, it.startMs, clock, it.text)
            }
        }
    }

    // MARK: Playback

    /** Connects to PlaybackService, which owns the player so it keeps playing in the background. */
    private fun startPlayer(entry: Entry) {
        if (connecting) return
        connecting = true
        val token = SessionToken(app, ComponentName(app, PlaybackService::class.java))
        val future = MediaController.Builder(app, token).buildAsync()
        future.addListener({
            val player = runCatching { future.get() }.getOrElse {
                _messages.trySend("Playback is unavailable: ${it.describe()}")
                return@addListener
            }
            if (cleared) return@addListener player.release()
            player.addListener(object : Player.Listener {
                override fun onEvents(player: Player, events: Player.Events) = tick()
                override fun onPlayerError(error: PlaybackException) {
                    _messages.trySend("Could not play the recording. ${error.cause?.describe() ?: ""}".trim())
                }
            })
            val item = MediaItem.Builder()
                .setMediaId(vault.url("/entries/${segment(entryId)}/audio"))
                .setMediaMetadata(MediaMetadata.Builder().setTitle(entry.displayTitle).setArtist(entry.dateLine).build())
                .build()
            player.setMediaItem(item, openAt ?: 0)
            player.prepare()
            this.player = player
            tick()
        }, ContextCompat.getMainExecutor(app))
    }

    private fun tick() {
        val player = player ?: return
        val playing = player.playWhenReady && player.playbackState != Player.STATE_ENDED
        val duration = player.duration.takeIf { it != C.TIME_UNSET } ?: _state.value.entry?.durationMs ?: 0
        _playback.value = Playback(true, playing, player.currentPosition, duration)
        val position = player.currentPosition
        _currentRow.value = if (position > 0 || playing) rowAt(position) else null
        if (playing && ticker?.isActive != true) {
            ticker = viewModelScope.launch {
                while (player.playWhenReady) {
                    delay(200)
                    tick()
                }
            }
        }
    }

    /** Rows are ordered by start time: the last one that has started. */
    fun rowAt(ms: Long): String? {
        val rows = _state.value.rows
        var low = 0
        var high = rows.size
        while (low < high) {
            val mid = (low + high) / 2
            if (rows[mid].startMs <= ms) low = mid + 1 else high = mid
        }
        return rows.getOrNull(low - 1)?.id
    }

    fun togglePlay() {
        val player = player ?: return
        if (player.playWhenReady && player.playbackState != Player.STATE_ENDED) return player.pause()
        if (player.playbackState == Player.STATE_ENDED) player.seekTo(0)
        player.play()
    }

    fun pause() = player?.pause()

    fun seek(ms: Long, play: Boolean = false) {
        val player = player ?: return
        player.seekTo(ms.coerceIn(0, _playback.value.durationMs.coerceAtLeast(0)))
        if (play) player.play()
        tick()
    }

    fun skip(by: Long) = seek((player?.currentPosition ?: 0) + by)

    /** Leaving the recording ends its playback; the service stops once nothing is playing. */
    override fun onCleared() {
        cleared = true
        player?.run {
            stop()
            clearMediaItems()
            release()
        }
        player = null
    }

    // MARK: Edits

    fun perform(action: Library.Action) {
        viewModelScope.launch {
            try {
                library.perform(action, entryId)
                refresh()
                watch()
            } catch (error: CancellationException) {
                throw error
            } catch (error: Exception) {
                _messages.trySend(error.describe())
            }
        }
    }

    /** `personId` names the speaker; null keeps it unidentified (`locked`) or hands it back to recognition. */
    fun setIdentity(personId: String?, label: String, locked: Boolean = true) {
        viewModelScope.launch { identify(label) { library.assign(personId, label, entryId, locked) } }
    }

    fun addPerson(name: String, label: String) {
        viewModelScope.launch {
            identify(label) {
                val person = library.addPerson(name)
                library.assign(person.id, label, entryId)
            }
        }
    }

    private suspend fun identify(label: String, change: suspend () -> Unit) {
        _state.update { it.copy(updatingSpeaker = true) }
        try {
            change()
            refresh()
        } catch (error: CancellationException) {
            throw error
        } catch (error: Exception) {
            _messages.trySend("Could not change ${_state.value.name(label)}: ${error.describe()}")
        } finally {
            _state.update { it.copy(updatingSpeaker = false) }
        }
    }

    fun markdown(): String {
        val state = _state.value
        val entry = state.entry ?: return ""
        fun escape(value: String) = value.replace("\\", "\\\\").replace("*", "\\*").replace("[", "\\[").replace("]", "\\]")
        val lines = mutableListOf("# ${entry.displayTitle}", "", "- Recorded: ${entry.dateLine}", "- Duration: ${Format.duration(entry.durationSeconds)}")
        if (entry.labels.isNotEmpty()) lines += "- Labels: ${entry.labels.joinToString(", ")}"
        if (!entry.summary.isNullOrEmpty()) lines += listOf("", "## Summary", "", entry.summary)
        lines += listOf("", "## Transcript", "")
        for (row in state.rows) lines += listOf("[${row.clock}] **${escape(state.name(row.speaker))}**", row.text, "")
        return lines.joinToString("\n")
    }
}
