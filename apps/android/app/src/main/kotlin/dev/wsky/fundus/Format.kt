package dev.wsky.fundus

import android.text.format.DateFormat
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.util.Locale
import java.util.concurrent.ConcurrentHashMap
import kotlin.math.roundToInt

/** Display strings. Dates and times follow the phone's language, time zone and 24-hour setting. */
object Format {
    /** The system's 24-hour setting; MainActivity reads it whenever the app comes to the front. */
    @Volatile var hour24 = true

    /** `32s`, `54m`, `2h 09m`. */
    fun duration(seconds: Double): String {
        val total = seconds.roundToInt()
        if (total < 60) return "${total}s"
        val minutes = (seconds / 60).roundToInt()
        return if (minutes < 60) "${minutes}m" else "%dh %02dm".format(minutes / 60, minutes % 60)
    }

    /** `2:03` or `1:02:03`, for positions in the audio. */
    fun position(seconds: Double): String {
        val total = seconds.toInt().coerceAtLeast(0)
        return if (total >= 3600) "%d:%02d:%02d".format(total / 3600, total % 3600 / 60, total % 60)
        else "%d:%02d".format(total / 60, total % 60)
    }

    /** `Nov 2`. */
    fun day(instant: Instant): String = pattern("MMMd").format(instant.atZone(zone))

    /** `14:05`, or `14:05:03` with seconds. */
    fun time(instant: Instant, seconds: Boolean = false): String {
        val skeleton = (if (hour24) "Hm" else "hm") + if (seconds) "s" else ""
        return pattern(skeleton).format(instant.atZone(zone))
    }

    /** `Today`, `Yesterday`, `Monday, November 2`, and the year once it is not this one. */
    fun dayHeader(date: LocalDate, today: LocalDate = LocalDate.now(zone)): String = when (date) {
        today -> "Today"
        today.minusDays(1) -> "Yesterday"
        else -> pattern(if (date.year == today.year) "EEEEMMMMd" else "EEEEyMMMMd").format(date)
    }

    val zone: ZoneId get() = ZoneId.systemDefault()

    private val patterns = ConcurrentHashMap<String, DateTimeFormatter>()
    private fun pattern(skeleton: String): DateTimeFormatter {
        val locale = Locale.getDefault()
        return patterns.getOrPut("$locale|$skeleton") {
            DateTimeFormatter.ofPattern(DateFormat.getBestDateTimePattern(locale, skeleton), locale)
        }
    }
}

/**
 * Positions in the playback audio to wall-clock times. A session can join several recorder files,
 * each with its own start time, and the server trims the start before processing.
 */
class SessionTimeline(private val chunks: List<Pair<Double, Instant>>) {
    fun at(milliseconds: Long): Instant {
        val seconds = milliseconds / 1000.0
        val (offset, start) = chunks.lastOrNull { it.first <= seconds } ?: chunks.firstOrNull()
            ?: return Instant.ofEpochMilli(milliseconds)
        return start.plusMillis(((seconds - offset) * 1000).toLong())
    }

    companion object {
        fun of(entry: Entry): SessionTimeline {
            val sources = entry.sources.orEmpty()
            val rate = (sources.firstOrNull()?.format?.sampleRate ?: 48_000).coerceAtLeast(1).toDouble()
            val trimmed = (entry.trimStartFrame ?: 0) / rate
            val chunks = sources.mapNotNull { source ->
                source.recordedAt?.let(::instant)?.let { source.offsetMs / 1000.0 - trimmed to it }
            }.ifEmpty { listOfNotNull(entry.recorded?.let { 0.0 to it }) }
            return SessionTimeline(chunks)
        }
    }
}
