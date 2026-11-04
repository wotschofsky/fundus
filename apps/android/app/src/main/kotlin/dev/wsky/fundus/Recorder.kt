package dev.wsky.fundus

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.hardware.usb.UsbManager
import android.net.Uri
import android.util.Log
import androidx.core.content.edit
import android.provider.DocumentsContract
import android.provider.DocumentsContract.Document
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.serialization.Serializable
import java.io.File
import java.time.ZoneId

/**
 * Connected DJI mics (the Mac's RecorderSources). The case shows up as a USB drive that Android keeps
 * hidden from apps (only SD cards are listed as storage volumes), so the app reads it through the
 * system's folder picker: the person picks the drive once, and Android remembers that choice.
 */
class Recorder(private val context: Context) {
    data class Volume(val uuid: String, val name: String)

    data class State(
        /** Drives with DJI recordings this app may read. */
        val recorders: List<Volume> = emptyList(),
        /** A mic is plugged in and no allowed drive shows its recordings: never allowed, or the wrong folder was picked. */
        val pending: Boolean = false,
        val sessions: List<RecordingSession> = emptyList(),
        val scanning: Boolean = false,
    ) {
        val connected get() = recorders.isNotEmpty() || pending
    }

    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    /** The mic window is open (the Mac's floating panel). */
    val prompt = MutableStateFlow(false)

    private val prefs = context.getSharedPreferences("recorder", Context.MODE_PRIVATE)

    /** The zone the recorder's clock runs in (the Mac's "Recorder clock" setting): DJI file names carry no zone. */
    val clock = MutableStateFlow(prefs.getString(CLOCK, null)?.let { runCatching { ZoneId.of(it) }.getOrNull() } ?: ZoneId.systemDefault())
    val clockChosen get() = prefs.contains(CLOCK)

    suspend fun setClock(zone: ZoneId) {
        prefs.edit { putString(CLOCK, zone.id) }
        clock.value = zone
        scan()
    }

    private val usb = context.getSystemService(UsbManager::class.java)
    private val scanning = Mutex()
    @Serializable private data class Header(val format: AudioFormat, val frames: Long)

    // ponytail: grows by ~150 bytes per recording ever seen; prune if it ever matters.
    /** Header reads by document and size, kept across launches: reading ~200 files over USB is what makes a scan slow. */
    private val headerFile = File(context.filesDir, "mic-headers.json")
    private val headers: MutableMap<String, Header> by lazy {
        runCatching { VaultJson.decodeFromString<Map<String, Header>>(headerFile.readText()) }.getOrDefault(emptyMap()).toMutableMap()
    }
    private var headersChanged = false

    suspend fun scan(): State = scanning.withLock {
        _state.value = _state.value.copy(scanning = true)
        var result = withContext(Dispatchers.IO) { read() }
        // The drive mounts, and lists its files, a few seconds after the USB device appears: an allowed mic
        // is not asked for again meanwhile.
        var tries = 0
        while (result.waiting && tries++ < 10) {
            delay(1_000)
            result = withContext(Dispatchers.IO) { read() }
        }
        _state.value = result.state
        result.state
    }

    private class Read(val state: State, val waiting: Boolean)

    private fun read(): Read {
        val mics = usb.deviceList.values.count { it.vendorId == DJI_VENDOR }
        val trees = context.contentResolver.persistedUriPermissions
            .filter { it.isReadPermission && it.uri.authority == STORAGE }.map { it.uri }
            .filter { drive(DocumentsContract.getTreeDocumentId(it)) != null }
        val recorders = mutableListOf<Volume>()
        val recordings = mutableListOf<SourceRecording>()
        val zone = clock.value
        // Counted by drive: two grants on one mic (its root and a folder) are still one mic.
        val readable = HashSet<String>()
        for (tree in trees) {
            val root = DocumentsContract.getTreeDocumentId(tree)
            val drive = drive(root) ?: continue
            // A drive that is not plugged in does not answer.
            val name = runCatching {
                context.contentResolver.query(DocumentsContract.buildDocumentUriUsingTree(tree, root), arrayOf(Document.COLUMN_DISPLAY_NAME), null, null, null)
                    ?.use { if (it.moveToFirst()) it.getString(0) else null }
            }.getOrNull() ?: continue
            readable += drive
            val found = files(tree).mapNotNull { inspect(it, zone) }
            if (found.isEmpty()) Log.w(TAG, "no readable DJI recordings in $tree")
            if (found.isNotEmpty()) {
                if (recorders.none { it.uuid == drive }) recorders += Volume(drive, name)
                recordings += found
            }
        }
        if (headersChanged) runCatching { headerFile.writeText(VaultJson.encodeToString(headers.toMap())) }.onSuccess { headersChanged = false }
        val drives = trees.mapNotNullTo(HashSet()) { drive(DocumentsContract.getTreeDocumentId(it)) }
        // Asking again is the way back from a wrong pick (a folder on the mic, an SD card), so any plugged-in mic
        // without recordings to show is asked for; only an allowed drive still mounting is waited for first.
        val pending = mics > recorders.size
        // The same file seen through two grants on one drive is one recording.
        val unique = recordings.distinctBy { DocumentsContract.getDocumentId(Uri.parse(it.uri)) }
        return Read(State(recorders, pending, SessionGrouper.group(unique)), waiting = pending && drives.size > readable.size)
    }

    private data class FileOnMic(val uri: Uri, val name: String, val size: Long)

    /** Bounded, read-only walk like the Mac's: three folders deep, hidden files skipped, DJI names only. */
    private fun files(tree: Uri): List<FileOnMic> {
        val found = mutableListOf<FileOnMic>()
        fun walk(id: String, depth: Int) {
            val children = DocumentsContract.buildChildDocumentsUriUsingTree(tree, id)
            val columns = arrayOf(Document.COLUMN_DOCUMENT_ID, Document.COLUMN_DISPLAY_NAME, Document.COLUMN_MIME_TYPE, Document.COLUMN_SIZE)
            runCatching { context.contentResolver.query(children, columns, null, null, null) }.onFailure { Log.w(TAG, "cannot list $children", it) }.getOrNull()?.use { rows ->
                while (rows.moveToNext()) {
                    val child = rows.getString(0)
                    val name = rows.getString(1) ?: continue
                    if (name.startsWith(".")) continue
                    if (rows.getString(2) == Document.MIME_TYPE_DIR) {
                        if (depth < 3) walk(child, depth + 1)
                    } else if (DJIFilename.parse(name) != null) {
                        found += FileOnMic(DocumentsContract.buildDocumentUriUsingTree(tree, child), name, rows.getLong(3))
                    }
                }
            }
        }
        walk(DocumentsContract.getTreeDocumentId(tree), 1)
        return found
    }

    /**
     * A file whose header cannot be read (the recorder lost power mid-file) is left out of this scan, as on
     * the Mac; only good headers are kept, so a read cut short by unplugging is tried again next time.
     */
    private fun inspect(document: FileOnMic, zone: ZoneId): SourceRecording? {
        val name = DJIFilename.parse(document.name) ?: return null
        val key = "${document.uri}|${document.size}"
        val header = headers[key] ?: runCatching {
            val (format, frames) = context.contentResolver.openInputStream(document.uri)!!.buffered(65_536).use { WavHeader.read(it, document.size) }
            Header(format, frames).also { headers[key] = it; headersChanged = true }
        }.onFailure { Log.w(TAG, "skipped ${document.name}", it) }.getOrNull() ?: return null
        return SourceRecording(document.uri.toString(), document.name, name.transmitter, name.variant, name.sequence,
            name.startInstant(zone), header.frames, document.size, header.format)
    }

    /** The person picked the drive in the system's folder picker: keep reading it after restarts. */
    suspend fun allow(tree: Uri) {
        context.contentResolver.takePersistableUriPermission(tree, Intent.FLAG_GRANT_READ_URI_PERMISSION)
        scan()
    }

    /** The system's folder picker; the person picks the mic's drive in its menu. */
    fun accessIntent() = Intent(Intent.ACTION_OPEN_DOCUMENT_TREE)

    /** A mic was plugged in: always open the window, and read the mic again (the window may already be open). */
    suspend fun attached() {
        prompt.value = true
        scan()
    }

    /** Unplugged: close the window, unless it still holds an upload that has not finished (its Resume and Discard). */
    suspend fun detached(uploads: Uploads) {
        if (!scan().connected && !uploads.queue.value.unfinished) prompt.value = false
    }

    companion object {
        /** The removable drive a folder grant is on; null for the phone's own storage, which never holds a mic. */
        fun drive(treeDocumentId: String): String? = treeDocumentId.substringBefore(':').takeIf { it !in setOf("primary", "home") }

        private const val TAG = "Recorder"
        private const val CLOCK = "clock"
        /** What DJI mics ship with; the Mac offers it by name. */
        val FACTORY_CLOCK: ZoneId = ZoneId.of("Etc/GMT-8")
        const val OPEN_PROMPT = "dev.wsky.fundus.RECORDER"
        private const val STORAGE = "com.android.externalstorage.documents"
        // ponytail: the USB chip vendor (Actions) of the DJI Mic case; matches res/xml/dji_mic.xml.
        const val DJI_VENDOR = 0x10D6

        /** `3 new recordings · 2h 09m`. */
        fun newRecordings(sessions: List<RecordingSession>) =
            "${if (sessions.size == 1) "1 new recording" else "${sessions.size} new recordings"} · ${Format.duration(sessions.sumOf { it.duration })}"
    }
}

/** The mic was unplugged: close its window, unless an upload in it has not finished. */
class RecorderReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val app = context.fundus
        val pending = goAsync()
        app.scope.launch {
            try { app.recorder.detached(app.uploads) } finally { pending.finish() }
        }
    }
}
