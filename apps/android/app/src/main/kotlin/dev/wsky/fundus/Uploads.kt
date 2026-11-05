package dev.wsky.fundus

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.Uri
import android.os.IBinder
import android.util.AtomicFile
import androidx.annotation.Keep
import androidx.core.app.NotificationCompat
import androidx.core.content.edit
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.FlowPreview
import kotlinx.coroutines.Job
import kotlinx.coroutines.MainScope
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.sample
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.add
import kotlinx.serialization.json.addJsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonArray
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.Request
import okhttp3.RequestBody
import okhttp3.RequestBody.Companion.toRequestBody
import okhttp3.Response
import okio.BufferedSink
import java.io.File
import java.io.RandomAccessFile
import java.util.Base64
import java.util.TimeZone
import java.util.UUID
import java.util.concurrent.atomic.AtomicBoolean

/** libFLAC through `native/audio`: integer PCM WAV to verified, lossless FLAC; other audio copied. */
object AudioCodec {
    init { System.loadLibrary("fundus_audio") }
    private var listener: ((Double) -> Boolean)? = null

    /** Empty on success, otherwise why it failed. `progress` returns true to stop. */
    @Synchronized
    fun prepare(source: String, destination: String, progress: (Double) -> Boolean): String {
        listener = progress
        try { return nativePrepare(source, destination) } finally { listener = null }
    }

    /** Called from C; kept by name through minification. */
    @Keep
    fun progress(amount: Double): Boolean = listener?.invoke(amount) ?: false

    private external fun nativePrepare(source: String, destination: String): String
}

@Serializable
data class SessionPlan(val sourceIds: List<String>)

@Serializable
data class UploadItem(
    val id: String = UUID.randomUUID().toString(),
    val name: String,
    /** The recording on the mic, until it is copied to the phone. */
    val uri: String? = null,
    val size: Long = 0,
    val source: String,
    val sourceId: String? = null,
    val preparationProgress: Double? = null,
    val prepared: String? = null,
    val location: String? = null,
    val offset: Long = 0,
    val length: Long = 0,
    val state: String = "waiting",
)

@Serializable
data class QueueState(
    val server: String = "",
    val timezone: String = TimeZone.getDefault().id,
    val importId: String? = null,
    val sessions: List<SessionPlan>? = null,
    val completed: Boolean = false,
    val state: String = "idle",
    val error: String? = null,
    val files: List<UploadItem> = emptyList(),
) {
    val isTransferring get() = state in setOf("copying", "uploading", "verifying")
    /** A new import can start. */
    val isIdle get() = state == "idle" || state == "verified"
    /** Holds recordings the vault has not confirmed yet: running, paused or failed. */
    val unfinished get() = files.isNotEmpty() && state != "verified"
}

/**
 * The phone's upload queue and what the vault knows about past imports (the Mac's TransferQueue,
 * SourceSessions history and AppModel import steps). One import at a time; sessions asked for
 * meanwhile go next, together. A journal on disk lets a stopped upload resume where it was.
 */
class Uploads(private val context: Context, private val vault: Vault, private val recorder: Recorder) {
    private val folder = File(context.filesDir, "upload-queue").apply { mkdirs() }
    private val journal = AtomicFile(File(folder, "queue.json"))
    private val lock = Any()
    private val _queue = MutableStateFlow(
        runCatching { VaultJson.decodeFromString<QueueState>(String(journal.readFully())) }.getOrDefault(QueueState())
    )
    val queue: StateFlow<QueueState> = _queue.asStateFlow()

    private val prefs = context.getSharedPreferences("uploads", Context.MODE_PRIVATE)
    private val _status = MutableStateFlow(
        prefs.getString(STATUS, null)?.let { runCatching { VaultJson.decodeFromString<ImportStatus>(it) }.getOrNull() }
            ?: prefs.getString(HISTORY, null)?.let { runCatching { VaultJson.decodeFromString<CachedHistory>(it) }.getOrNull() }
                ?.let { ImportStatus(histories = mapOf(it.server to ImportStatus.History(it.entries))) }
            ?: ImportStatus()
    )
    /** The vault's imported sessions, from this phone or anywhere else, and this phone's verified ones. */
    val status: StateFlow<ImportStatus> = _status.asStateFlow()

    private val _waiting = MutableStateFlow<List<RecordingSession>>(emptyList())
    /** Asked for from the mic window while another upload runs. */
    val waiting: StateFlow<List<RecordingSession>> = _waiting.asStateFlow()

    private val running = AtomicBoolean(false)
    @Volatile private var stopping = false
    /** Resume was pressed while a pause was still stopping the transfer. */
    @Volatile private var resumeAfterStopping = false

    @Serializable
    private data class CachedHistory(val server: String, val entries: List<SourceHistoryEntry>)

    /** The connected vault's imports, this phone's verified ones it does not list yet, then the queue's. */
    fun records(): List<ImportRecord> = _status.value.records(vault.server.value, _queue.value)

    private val statusLock = Any()

    /**
     * Changes the import status; [persist] when it holds something a restart must keep. Change and save happen
     * under one lock, so a history poll and an upload's verification on other threads cannot save out of order.
     */
    private fun changeStatus(persist: Boolean = true, change: (ImportStatus) -> ImportStatus): ImportStatus = synchronized(statusLock) {
        val current = _status.value
        val next = change(current)
        _status.value = next
        if (persist && next !== current) prefs.edit { putString(STATUS, VaultJson.encodeToString(ImportStatus.serializer(), next)) }
        next
    }

    /** One session from the mic window: now, or after the current upload. */
    fun importNext(session: RecordingSession) {
        _waiting.value += session
        if (_queue.value.isIdle) start()
    }

    fun pause() {
        resumeAfterStopping = false
        stopping = true
        update { if (it.isTransferring) it.copy(state = "paused", error = null) else it }
    }

    fun resume() {
        val queue = _queue.value
        if (queue.files.isEmpty() || queue.state !in setOf("failed", "paused")) return
        if (running.get()) { resumeAfterStopping = true; return }
        restart()
        start()
    }

    private fun restart() = update { queue ->
        val state = when {
            queue.files.any { it.uri != null } -> "copying"
            queue.completed -> "verifying"
            else -> "uploading"
        }
        queue.copy(state = state, error = null)
    }

    /** Forgets a stopped upload and its copies on the phone. Recordings already in the vault stay there. */
    fun discard() {
        if (running.get() || _queue.value.isTransferring) return
        _waiting.value = emptyList()
        deleteAudio()
        update { QueueState(server = it.server) }
        // The paused or failed notification described this upload.
        context.getSystemService(NotificationManager::class.java).cancel(UploadService.NOTIFICATION)
    }

    /** An upload the app was closed in the middle of. */
    fun resumeInterrupted() {
        if (_queue.value.isTransferring && !running.get()) start()
    }

    private fun start() {
        runCatching { context.startForegroundService(Intent(context, UploadService::class.java)) }
    }

    suspend fun refreshHistory() {
        val server = vault.server.value ?: return
        // A ticket from before the request, so a slower, older read can never replace this one's answer.
        val ticket = changeStatus(persist = false) { it.begin().first }.issued
        try {
            val entries = mutableListOf<SourceHistoryEntry>()
            var cursor: Int? = 0
            while (cursor != null) {
                // Every page comes from the vault captured above, not whichever is connected by then. The client
                // signs only requests to the connected vault, so after a switch a page fails instead of mixing
                // two vaults' histories under one.
                val request = Request.Builder().url(endpoint(server, "/source-history?limit=500&cursor=$cursor")).build()
                val page = withContext(Dispatchers.IO) {
                    vault.http.newCall(request).execute().use { response ->
                        if (!response.isSuccessful) throw VaultError("The vault could not list its imports (HTTP ${response.code}).")
                        VaultJson.decodeFromString<SourceHistoryPage>(response.body.string())
                    }
                }
                entries += page.entries
                cursor = page.nextCursor
            }
            changeStatus { it.accept(ticket, server, entries) }
        } catch (error: CancellationException) {
            throw error
        } catch (_: Exception) {
            // Offline, or an older vault: the cached history still marks imported sessions.
        }
    }

    // MARK: Transfer

    /** Runs the queue until it is done, stopped or failed; the upload service calls it. */
    suspend fun run() {
        if (!running.compareAndSet(false, true)) return
        try {
            while (true) {
                if (_queue.value.isIdle) {
                    val next = takeWaiting()
                    if (next.isEmpty() || !enqueue(next)) return
                } else if (!_queue.value.isTransferring) {
                    return
                }
                if (!advance()) {
                    if (!resumeAfterStopping) return
                    resumeAfterStopping = false
                    restart()
                }
            }
        } finally {
            running.set(false)
        }
    }

    /** Imported meanwhile, or the mic was unplugged: those are dropped. */
    private fun takeWaiting(): List<RecordingSession> {
        val present = recorder.state.value.sessions.associateBy { it.id }
        val records = records()
        val next = _waiting.value.mapNotNull { present[it.id] }.distinctBy { it.id }.filter { Imports.record(it, records) == null }
        _waiting.value = emptyList()
        return next
    }

    private fun enqueue(sessions: List<RecordingSession>): Boolean {
        val server = vault.server.value ?: return false
        deleteAudio()
        val files = sessions.flatMap { it.recordings }.map {
            UploadItem(name = it.filename, uri = it.uri, size = it.fileSize, source = "${UUID.randomUUID()}.wav", sourceId = it.sourceId, state = "copying")
        }
        update { QueueState(server = server, timezone = recorder.clock.value.id, sessions = sessions.map { SessionPlan(it.sourceIds) }, state = "copying", files = files) }
        return true
    }

    /** True when the import was verified; false when it stopped. */
    private suspend fun advance(): Boolean = withContext(Dispatchers.IO) {
        stopping = false
        try {
            val server = _queue.value.server
            if (vault.server.value != server) throw VaultError("Connect to ${host(server)} again to finish this upload.")
            // Everything comes off the mic first, so it can be unplugged while the rest runs.
            for (item in _queue.value.files.filter { it.uri != null }) copy(item)
            if (_queue.value.state == "copying") update { it.copy(state = "uploading") }
            if (!_queue.value.completed) {
                if (_queue.value.importId == null) {
                    val body = buildJsonObject {
                        put("recorder_timezone", _queue.value.timezone)
                        _queue.value.sessions?.let { plans ->
                            putJsonArray("sessions") { plans.forEach { plan -> addJsonObject { putJsonArray("source_ids") { plan.sourceIds.forEach(::add) } } } }
                        }
                    }
                    val created = call(Request.Builder().url(api("/imports")).post(body.toString().toRequestBody(JSON)).build()) { it.body.string() }
                    val id = json(created).string("id") ?: throw VaultError("The vault sent an invalid import.")
                    update { it.copy(importId = id) }
                }
                for (item in _queue.value.files) {
                    checkStopping()
                    if (item.state == "uploaded") continue
                    if (item.prepared == null) prepare(item.id)
                    upload(item.id)
                }
                call(Request.Builder().url(api("/imports/${segment(_queue.value.importId!!)}/complete")).post(ByteArray(0).toRequestBody()).build()) {}
                update { it.copy(completed = true, state = "verifying") }
            }
            verify()
            // Imported until a newer read of the vault's history decides, even if that read fails now.
            val verified = _queue.value
            changeStatus { it.verified(verified.server, verified.sessions.orEmpty().map { plan -> plan.sourceIds }) }
            deleteAudio()
            refreshHistory()
            true
        } catch (error: CancellationException) {
            throw error
        } catch (error: Exception) {
            update { if (stopping) it.copy(state = "paused", error = null) else it.copy(state = "failed", error = error.describe()) }
            false
        }
    }

    private fun copy(item: UploadItem) {
        checkStopping()
        val partial = File(folder, "${item.source}.copying")
        try {
            var copied = 0L
            var reported = 0L
            context.contentResolver.openInputStream(Uri.parse(item.uri))!!.use { input ->
                partial.outputStream().use { output ->
                    val buffer = ByteArray(1 shl 20)
                    while (true) {
                        checkStopping()
                        val count = input.read(buffer)
                        if (count < 0) break
                        output.write(buffer, 0, count)
                        copied += count
                        val now = System.currentTimeMillis()
                        if (now - reported >= 250) { reported = now; updateItem(item.id, persist = false) { it.copy(offset = copied) } }
                    }
                    output.fd.sync()
                }
            }
            if (copied != item.size) throw VaultError("${item.name} changed while it was copied. Plug the mic in again and resume.")
            if (!partial.renameTo(File(folder, item.source))) throw VaultError("Could not keep a copy of ${item.name} on this phone.")
        } catch (error: Exception) {
            partial.delete()
            if (error is VaultError) throw error
            throw VaultError("The mic was disconnected before ${item.name} was copied. Plug it in again and resume.")
        }
        updateItem(item.id) { it.copy(uri = null, offset = 0, state = "waiting") }
    }

    private fun prepare(id: String) {
        val item = item(id)
        val source = File(folder, item.source)
        val format = runCatching { source.inputStream().buffered().use { WavHeader.read(it, source.length()).first } }
            .getOrElse { throw VaultError("${item.name} is unreadable or its format is unsupported.") }
        val prepared = File(folder, "${item.id}.${if (format.compresses) "flac" else "wav"}")
        prepared.delete()
        updateItem(id) { it.copy(state = "preparing", preparationProgress = 0.0) }
        var reported = 0L
        val error = AudioCodec.prepare(source.path, prepared.path) { progress ->
            val now = System.currentTimeMillis()
            if (now - reported > 500) {
                reported = now
                updateItem(id, persist = false) { it.copy(preparationProgress = progress.coerceIn(0.0, 1.0)) }
            }
            stopping
        }
        checkStopping()
        if (error.isNotEmpty()) throw VaultError(error)
        updateItem(id) { it.copy(prepared = prepared.name, length = prepared.length(), preparationProgress = 1.0, state = "ready") }
        // ponytail: the WAV copy goes once its lossless copy is verified; phones have little room.
        source.delete()
    }

    /** tus, in 8 MB chunks: the vault's request size limit. */
    private fun upload(id: String) {
        val importId = _queue.value.importId!!
        if (item(id).location == null) {
            val item = item(id)
            val metadata = listOfNotNull("import_id" to importId, "filename" to item.prepared!!, "original_name" to item.name, item.sourceId?.let { "source_id" to it })
                .sortedBy { it.first }.joinToString(",") { (key, value) -> "$key ${Base64.getEncoder().encodeToString(value.toByteArray())}" }
            val request = Request.Builder().url(api("/uploads")).post(ByteArray(0).toRequestBody())
                .header("Tus-Resumable", "1.0.0").header("Upload-Length", item.length.toString()).header("Upload-Metadata", metadata).build()
            val location = call(request) { it.header("Location") }
            if (uploadPath(location) == null) throw VaultError("The vault sent an invalid upload address.")
            updateItem(id) { it.copy(location = location) }
        }
        val url = api(uploadPath(item(id).location) ?: throw VaultError("The vault sent an invalid upload address."))
        val offset = call(Request.Builder().url(url).head().header("Tus-Resumable", "1.0.0").build()) { it.header("Upload-Offset")?.toLongOrNull() }
        if (offset == null || offset !in 0..item(id).length) throw VaultError("The vault sent an invalid upload offset.")
        updateItem(id) { it.copy(offset = offset, state = "uploading") }
        val file = File(folder, item(id).prepared!!)
        while (item(id).offset < item(id).length) {
            checkStopping()
            val start = item(id).offset
            val count = minOf(CHUNK, item(id).length - start)
            val request = Request.Builder().url(url).patch(Chunk(file, start, count, id))
                .header("Tus-Resumable", "1.0.0").header("Upload-Offset", start.toString()).build()
            val confirmed = call(request) { it.header("Upload-Offset")?.toLongOrNull() }
            if (confirmed != start + count) throw VaultError("The vault returned an unexpected upload offset. Resume to reconcile it.")
            updateItem(id) { it.copy(offset = confirmed) }
        }
        updateItem(id) { it.copy(state = "uploaded") }
    }

    private inner class Chunk(val file: File, val start: Long, val count: Long, val id: String) : RequestBody() {
        override fun contentType() = "application/offset+octet-stream".toMediaType()
        override fun contentLength() = count
        override fun writeTo(sink: BufferedSink) {
            RandomAccessFile(file, "r").use { input ->
                input.seek(start)
                val buffer = ByteArray(65_536)
                var sent = 0L
                var reported = 0L
                while (sent < count) {
                    checkStopping()
                    val n = input.read(buffer, 0, minOf(buffer.size.toLong(), count - sent).toInt())
                    if (n < 0) throw VaultError("The prepared recording is incomplete. Discard this upload and import it again.")
                    sink.write(buffer, 0, n)
                    sent += n
                    val now = System.currentTimeMillis()
                    if (now - reported > 250) {
                        reported = now
                        updateItem(id, persist = false) { it.copy(offset = start + sent) }
                    }
                }
            }
        }
    }

    private suspend fun verify() {
        val id = _queue.value.importId ?: return
        repeat(120) {
            checkStopping()
            val receipt = json(call(Request.Builder().url(api("/imports/${segment(id)}")).build()) { it.body.string() })
            val files = receipt["files"]?.jsonArray?.map { it.jsonObject }.orEmpty()
            when (receipt.string("state")) {
                "failed" -> throw VaultError(files.firstNotNullOfOrNull { it.string("failure") } ?: "The vault could not verify this import.")
                "complete" -> {
                    val rejected = files.filter { it.string("state") !in setOf("verified", "duplicate") }
                    if (files.isEmpty() || rejected.isNotEmpty()) throw VaultError(rejected.firstOrNull()?.string("failure") ?: "The vault could not verify every recording.")
                    update { queue -> queue.copy(state = "verified", files = queue.files.map { it.copy(state = "verified") }) }
                    return
                }
            }
            delay(2_000)
        }
        throw VaultError("The vault is still verifying this import. Resume to check again.")
    }

    /**
     * On the upload's own vault, whichever is connected now: Vault signs only its own vault's requests, so after
     * switching vaults these go out unsigned and fail instead of landing on the other vault.
     */
    private fun api(path: String) = endpoint(_queue.value.server, path)

    private fun <T> call(request: Request, read: (Response) -> T): T = vault.http.newCall(request).execute().use { response ->
        if (response.code == 401 && vault.server.value != _queue.value.server) throw VaultError("Connect to ${host(_queue.value.server)} again to finish this upload.")
        if (!response.isSuccessful) {
            val message = runCatching { json(response.body.string()).string("error") }.getOrNull()
            throw VaultError(message ?: "The vault refused the upload (HTTP ${response.code}).")
        }
        read(response)
    }

    private fun checkStopping() { if (stopping) throw VaultError("Paused") }

    private fun item(id: String) = _queue.value.files.first { it.id == id }

    private fun update(persist: Boolean = true, change: (QueueState) -> QueueState) = synchronized(lock) {
        _queue.value = change(_queue.value)
        if (persist) {
            val stream = journal.startWrite()
            try {
                stream.write(VaultJson.encodeToString(QueueState.serializer(), _queue.value).toByteArray())
                journal.finishWrite(stream)
            } catch (error: Exception) {
                journal.failWrite(stream)
                throw error
            }
        }
    }

    private fun updateItem(id: String, persist: Boolean = true, change: (UploadItem) -> UploadItem) =
        update(persist) { queue -> queue.copy(files = queue.files.map { if (it.id == id) change(it) else it }) }

    private fun deleteAudio() = folder.listFiles()?.filter { it.name != "queue.json" }?.forEach { it.delete() }

    companion object {
        /** Where the import status is kept; [HISTORY] is the older history-only cache it replaces. */
        private const val STATUS = "import-status"
        private const val HISTORY = "source-history"
        private const val CHUNK = 8L * 1024 * 1024
        private val JSON = "application/json".toMediaType()

        fun endpoint(server: String, path: String) = "$server/api/native/v1$path"

        /** The vault's tus address for a file, relative to its native API; anything else is refused. */
        fun uploadPath(location: String?): String? =
            location?.takeIf { it.startsWith("/api/native/v1/uploads/") }?.removePrefix("/api/native/v1")

        /**
         * The import in this phone's queue, until the vault lists it: only while it is unfinished and on the
         * connected vault. Once verified, the vault's history alone decides (it may have been trashed since).
         */
        fun queued(queue: QueueState, history: List<ImportRecord>, server: String?): List<ImportRecord> {
            if (!queue.unfinished || queue.server != server) return emptyList()
            val known = history.mapTo(HashSet()) { it.sourceIds }
            return queue.sessions.orEmpty().filter { it.sourceIds !in known }
                .map { ImportRecord(it.sourceIds, "uploading", importId = queue.importId.orEmpty()) }
        }

        /** Whole-batch progress: copying off the mic and preparing count for a fifth of a file each, the upload for the rest. */
        fun progress(queue: QueueState): Double {
            if (queue.files.isEmpty()) return 0.0
            return queue.files.sumOf { item ->
                when (item.state) {
                    "copying" -> 0.2 * (if (item.size > 0) item.offset.toDouble() / item.size else 0.0)
                    "waiting" -> 0.2
                    "preparing" -> 0.2 + 0.2 * (item.preparationProgress ?: 0.0)
                    "ready" -> 0.4
                    "uploading" -> 0.4 + 0.6 * (if (item.length > 0) item.offset.toDouble() / item.length else 0.0)
                    "uploaded", "verified" -> 1.0
                    else -> 0.0
                }
            } / queue.files.size
        }

        /** [waiting]: sessions asked for meanwhile, still to be copied off the mic. */
        fun headline(queue: QueueState, waiting: Int = 0): String {
            val done = queue.files.count { it.state in setOf("uploaded", "verified") }
            val count = queue.files.size
            val files = if (count == 1) "file" else "files"
            return when (queue.state) {
                "copying" -> "Copying from the mic · keep it plugged in"
                "uploading" -> "Uploading $done of $count $files · " + if (waiting > 0) "keep the mic plugged in for the next one" else "the mic can be unplugged"
                "verifying" -> "Uploaded · the vault is checking the files"
                "verified" -> "$count $files sent to the vault"
                "paused" -> "Upload paused"
                "failed" -> queue.error ?: "Upload stopped"
                else -> ""
            }
        }
    }
}

private fun json(text: String): JsonObject = VaultJson.parseToJsonElement(text).jsonObject
private fun JsonObject.string(key: String) = get(key)?.jsonPrimitive?.contentOrNull

/** Keeps the upload going with the screen off, with its progress in the notification. */
class UploadService : Service() {
    private val scope = MainScope()
    private var job: Job? = null
    /** Started again while running: sessions may have been asked for just as the last upload ended. */
    private var again = false
    private val notifications get() = getSystemService(NotificationManager::class.java)

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        notifications.createNotificationChannel(NotificationChannel(CHANNEL, "Uploads", NotificationManager.IMPORTANCE_LOW))
        startForeground(NOTIFICATION, notification(fundus.uploads.queue.value), ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
    }

    @OptIn(FlowPreview::class)
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        again = true
        if (job?.isActive != true) {
            job = scope.launch {
                // The headline also depends on the sessions waiting their turn, so either change redraws it.
                val progress = launch {
                    combine(fundus.uploads.queue, fundus.uploads.waiting) { queue, _ -> queue }.sample(1_000)
                        .collect { notifications.notify(NOTIFICATION, notification(it)) }
                }
                while (again) {
                    again = false
                    fundus.uploads.run()
                }
                progress.cancel()
                finish()
            }
        }
        return START_NOT_STICKY
    }

    /**
     * Android limits how long data transfers run in the background and wants the service gone within seconds:
     * pause, and stop now rather than when the transfer unwinds. Nothing is posted here: a step already in flight
     * may still change the outcome, and the window (from the library's import button) shows where it ended.
     */
    override fun onTimeout(startId: Int, fgsType: Int) {
        fundus.uploads.pause()
        job?.cancel()
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    /** Paused or failed: the notification stays (not ongoing), so a stop is never mistaken for done. */
    private fun finish() {
        val last = fundus.uploads.queue.value
        if (last.unfinished && !last.isTransferring) {
            stopForeground(STOP_FOREGROUND_DETACH)
            notifications.notify(NOTIFICATION, notification(last))
        } else {
            stopForeground(STOP_FOREGROUND_REMOVE)
        }
        stopSelf()
    }

    override fun onDestroy() {
        scope.cancel()
        super.onDestroy()
    }

    private fun notification(queue: QueueState) = NotificationCompat.Builder(this, CHANNEL)
        .setSmallIcon(android.R.drawable.stat_sys_upload)
        .setContentTitle(Uploads.headline(queue, fundus.uploads.waiting.value.size).ifEmpty { "Uploading recordings" })
        .setContentText(host(queue.server))
        .apply { if (queue.isTransferring) setProgress(100, (Uploads.progress(queue) * 100).toInt(), queue.state == "verifying") }
        .setOngoing(queue.isTransferring)
        .setContentIntent(PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java).setAction(Recorder.OPEN_PROMPT), PendingIntent.FLAG_IMMUTABLE))
        .build()

    companion object {
        private const val CHANNEL = "uploads"
        const val NOTIFICATION = 1001
    }
}
