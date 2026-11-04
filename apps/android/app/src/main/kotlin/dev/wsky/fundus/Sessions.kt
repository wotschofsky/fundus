package dev.wsky.fundus

import kotlinx.serialization.Serializable
import java.io.DataInputStream
import java.io.EOFException
import java.io.InputStream
import java.security.MessageDigest
import java.time.DateTimeException
import java.time.Instant
import java.time.LocalDateTime
import java.time.ZoneId

// Recorder sessions as the Mac app finds them (apps/macos/Sources/Fundus: DJIFilename, SessionGrouper,
// SourceSessions). Fingerprints and source IDs must match the Mac's byte for byte: the vault's source
// history is how either app knows a session was already imported by the other.

@Serializable
data class AudioFormat(val sampleRate: Int, val channels: Int, val bitDepth: Int, val isFloat: Boolean) {
    /** Integer PCM up to 24 bits becomes FLAC; float and 32-bit audio go up unchanged. */
    val compresses get() = !isFloat && bitDepth <= 24
}

/** One WAV on the recorder. Read-only: nothing in the app writes to `uri`. */
data class SourceRecording(
    val uri: String,
    val filename: String,
    val transmitter: String,
    val variant: String,
    val sequence: Int,
    val start: Instant,
    val frameCount: Long,
    val fileSize: Long,
    val format: AudioFormat,
) {
    val duration get() = frameCount / format.sampleRate.toDouble()
    val startSeconds get() = start.epochSecond + start.nano / 1e9
    val end: Instant get() = start.plusMillis((duration * 1000).toLong())
    /** Name + byte size + exact frame count; hashing gigabytes on every scan is not needed. */
    val fingerprint get() = "${filename.lowercase()}|$fileSize|$frameCount"
    val sourceId get() = sha256(fingerprint)
}

/** One real-world recording session: what the person sees and imports. */
data class RecordingSession(val recordings: List<SourceRecording>) {
    val transmitter get() = recordings[0].transmitter
    val start get() = recordings[0].start
    val end get() = recordings.last().end
    val duration get() = recordings.sumOf { it.duration }
    val totalFrames get() = recordings.sumOf { it.frameCount }
    val sourceIds get() = recordings.map { it.sourceId }
    val id by lazy { sha256(recordings.joinToString("\n") { it.fingerprint }) }
}

/** `TX01_MIC002_20261001_204349_orig.wav`. */
data class DJIFilename(val transmitter: String, val sequence: Int, val variant: String, val start: LocalDateTime) {
    companion object {
        private val pattern = Regex("""^([A-Za-z]+[0-9]+)_MIC([0-9]+)_([0-9]{8})_([0-9]{6})((?:_[A-Za-z0-9]+)*)\.wav$""", RegexOption.IGNORE_CASE)

        fun parse(name: String): DJIFilename? {
            val groups = pattern.matchEntire(name)?.groupValues ?: return null
            val (date, time) = groups[3] to groups[4]
            fun number(text: String, from: Int, length: Int) = text.substring(from, from + length).toInt()
            // Impossible timestamps (month 13, 30 February) are rejected, not rolled over.
            val start = try {
                LocalDateTime.of(number(date, 0, 4), number(date, 4, 2), number(date, 6, 2), number(time, 0, 2), number(time, 2, 2), number(time, 4, 2))
            } catch (_: DateTimeException) { return null }
            return DJIFilename(groups[1].uppercase(), groups[2].toIntOrNull() ?: return null, groups[5].drop(1).lowercase(), start)
        }
    }
}

object SessionGrouper {
    private const val ROLLOVER = 1800.0
    private const val SLACK = 15.0
    private const val TOLERANCE = 2.0

    /** The recorder hit its maximum length and the next file starts where it ended. */
    fun rollsOver(previous: SourceRecording, next: SourceRecording): Boolean =
        Math.abs(next.startSeconds - (previous.startSeconds + previous.duration)) <= TOLERANCE && previous.duration >= ROLLOVER - SLACK

    // ponytail: no manual merge/split on the phone; sessions merged on the Mac show as partly imported.
    fun group(recordings: List<SourceRecording>): List<RecordingSession> {
        // Each transmitter (and file rendition) is its own timeline; recordings never merge across them.
        val streams = recordings.groupBy { "${it.transmitter}|${it.variant}|${it.format}" }
        val sessions = mutableListOf<RecordingSession>()
        for (stream in streams.values) {
            val ordered = stream.sortedWith(compareBy<SourceRecording> { it.start }.thenBy { it.sequence }.thenBy { it.filename })
            var current = mutableListOf<SourceRecording>()
            ordered.forEachIndexed { index, recording ->
                if (index > 0 && !rollsOver(ordered[index - 1], recording)) {
                    sessions += RecordingSession(current)
                    current = mutableListOf()
                }
                current += recording
            }
            if (current.isNotEmpty()) sessions += RecordingSession(current)
        }
        return sessions.sortedWith(compareBy<RecordingSession> { it.start }.thenBy { it.transmitter })
    }
}

/** A session's import as the vault (or the running upload) knows it. */
data class ImportRecord(
    val sourceIds: List<String>,
    val stage: String,
    val activeStage: String? = null,
    val pauseRequested: Boolean = false,
    val trimStartFrame: Long? = null,
    val trimEndFrame: Long? = null,
    /** Set while this phone's upload queue holds it. */
    val importId: String? = null,
)

@Serializable
data class SourceHistoryEntry(
    val id: String,
    val stage: String,
    val activeStage: String? = null,
    val failure: String? = null,
    val pauseRequested: Boolean = false,
    val trimStartFrame: Long? = null,
    val trimEndFrame: Long? = null,
    val sourceIds: List<String?> = emptyList(),
) {
    /** Entries uploaded without source IDs (by older clients) cannot be matched. */
    val record get() = sourceIds.filterNotNull().takeIf { it.isNotEmpty() && it.size == sourceIds.size }
        ?.let { ImportRecord(it, stage, activeStage, pauseRequested, trimStartFrame, trimEndFrame) }
}

@Serializable
data class SourceHistoryPage(val entries: List<SourceHistoryEntry> = emptyList(), val nextCursor: Int? = null)

object Imports {
    /** The untrimmed session's import; the phone has no trimming, so a trimmed import is only a part. */
    fun record(session: RecordingSession, records: List<ImportRecord>): ImportRecord? {
        val ids = session.sourceIds
        val total = session.totalFrames
        return records.lastOrNull { it.sourceIds == ids && (it.trimStartFrame ?: 0) == 0L && (it.trimEndFrame ?: total) == total }
    }

    fun partlyImported(session: RecordingSession, records: List<ImportRecord>): Boolean {
        val imported = records.filter { it.stage == "complete" }.flatMapTo(HashSet()) { it.sourceIds }
        return session.sourceIds.any { it in imported }
    }

    /** New in the mic window: not in the vault yet, even in part. */
    fun proposed(sessions: List<RecordingSession>, records: List<ImportRecord>) =
        sessions.filter { record(it, records) == null && !partlyImported(it, records) }
}

/** Format and exact frame count from a WAV header (never from file size or dates), as `native/audio` reads it. */
object WavHeader {
    fun read(stream: InputStream, fileSize: Long): Pair<AudioFormat, Long> {
        val input = DataInputStream(stream)
        var position = 0L
        fun bytes(count: Int) = ByteArray(count).also { input.readFully(it); position += count }
        fun skip(count: Long) { var left = count; while (left > 0) { val n = input.skip(left); if (n <= 0) throw EOFException(); left -= n }; position += count }
        fun u16(b: ByteArray, at: Int) = (b[at].toInt() and 0xff) or (b[at + 1].toInt() and 0xff shl 8)
        fun u32(b: ByteArray, at: Int) = u16(b, at).toLong() or (u16(b, at + 2).toLong() shl 16)
        fun unsupported(): Nothing = throw IllegalArgumentException("Unsupported or unfinished WAV header")

        val head = bytes(12)
        val riff = String(head, 0, 4, Charsets.US_ASCII)
        if (String(head, 8, 4, Charsets.US_ASCII) != "WAVE" || riff != "RIFF" && riff != "RF64") unsupported()
        var rf64Data = 0L
        var format: ByteArray? = null
        var dataOffset = 0L
        var dataBytes = -1L
        try {
            while (dataBytes < 0) {
                val chunk = bytes(8)
                val id = String(chunk, 0, 4, Charsets.US_ASCII)
                var length = u32(chunk, 4)
                when (id) {
                    "ds64" -> { if (length < 28) unsupported(); val b = bytes(28); rf64Data = u32(b, 8) or (u32(b, 12) shl 32); skip(length - 28 + (length and 1)) }
                    "fmt " -> { if (length < 16) unsupported(); val want = minOf(length, 64).toInt(); format = bytes(want); skip(length - want + (length and 1)) }
                    "data" -> { if (length == 0xffffffffL) length = rf64Data; dataOffset = position; dataBytes = length }
                    else -> skip(length + (length and 1))
                }
            }
        } catch (_: EOFException) { unsupported() }
        val fmt = format ?: unsupported()
        var encoding = u16(fmt, 0)
        if (encoding == 0xfffe && fmt.size >= 40) encoding = u16(fmt, 24)
        val channels = u16(fmt, 2)
        val rate = u32(fmt, 4)
        val bits = u16(fmt, 14)
        if (encoding != 1 && encoding != 3 || channels !in 1..8 || rate !in 8000..384000 || bits !in 8..64 || bits % 8 != 0) unsupported()
        val frameBytes = channels.toLong() * bits / 8
        // Like Core Audio (the Mac app): only the whole frames actually in the file count, so frame counts
        // and source IDs match. DJI mics declare 8 bytes more than they write.
        if (dataOffset > fileSize) throw IllegalArgumentException("Truncated or empty WAV audio")
        val frames = minOf(dataBytes, fileSize - dataOffset) / frameBytes
        if (frames <= 0) throw IllegalArgumentException("Truncated or empty WAV audio")
        return AudioFormat(rate.toInt(), channels, bits, encoding == 3) to frames
    }
}

fun sha256(text: String): String =
    MessageDigest.getInstance("SHA-256").digest(text.toByteArray()).joinToString("") { "%02x".format(it) }

/** A recording's start from its DJI name, read in the zone the recorder's clock runs in (`Recorder.clock`). */
fun DJIFilename.startInstant(zone: ZoneId): Instant = start.atZone(zone).toInstant()
