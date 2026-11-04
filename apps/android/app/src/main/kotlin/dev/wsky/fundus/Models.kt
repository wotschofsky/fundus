package dev.wsky.fundus

import kotlinx.serialization.ExperimentalSerializationApi
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonNamingStrategy
import java.time.Instant
import java.time.OffsetDateTime

// The vault's owner API, as the Mac app reads it (apps/macos/Sources/Fundus/VaultModels.swift).

@OptIn(ExperimentalSerializationApi::class)
val VaultJson = Json {
    ignoreUnknownKeys = true
    namingStrategy = JsonNamingStrategy.SnakeCase
}

/** A recording in the vault. Detail reads add speakers and sources. */
@Serializable
data class Entry(
    val id: String,
    val title: String = "",
    val recordedAt: String? = null,
    val createdAt: String,
    val durationMs: Long = 0,
    val stage: String,
    val activeStage: String? = null,
    val failure: String? = null,
    val pauseRequested: Boolean = false,
    val labels: List<String> = emptyList(),
    val summary: String? = null,
    val warnings: List<String> = emptyList(),
    val audioAvailable: Boolean = false,
    val speakers: List<Speaker>? = null,
    val sources: List<Source>? = null,
    val trimStartFrame: Long? = null,
    /** The recorder clock it was imported with (e.g. the DJI factory UTC+8), not where the person was. */
    val timezone: String? = null,
) {
    @Serializable
    data class Source(val offsetMs: Long = 0, val recordedAt: String? = null, val format: Format? = null) {
        @Serializable
        data class Format(val sampleRate: Int? = null)
    }

    val isProcessing get() = stage !in setOf("complete", "failed", "paused")
    val canResume get() = stage == "failed" || stage == "paused"
    val recorded: Instant? by lazy { recordedAt?.let(::instant) }
    /** When it was recorded, or uploaded if the recorder did not say. */
    val date: Instant by lazy { recorded ?: instant(createdAt) ?: Instant.EPOCH }
    val durationSeconds get() = durationMs / 1000.0

    /** Until a summary names it, a recording is known by when it was made. */
    val displayTitle: String
        get() {
            val trimmed = title.trim()
            return if (trimmed.isEmpty() || trimmed.startsWith("Recording · ")) dateLine else trimmed
        }

    /** `Nov 2 · 14:05`; undated recordings say so. */
    val dateLine get() = recorded?.let { "${Format.day(it)} · ${Format.time(it)}" } ?: "Date unknown"

    val status get() = describe(stage, activeStage, pauseRequested)

    companion object {
        /** What the server is doing with a recording, in words. */
        fun describe(stage: String, activeStage: String?, pauseRequested: Boolean): String {
            if (pauseRequested) return "Pausing after this step…"
            // A stopped recording can still name the step it stopped in; it is no longer doing it.
            return when (val step = if (stage == "failed" || stage == "paused") stage else activeStage ?: stage) {
                "uploading" -> "Uploading"
                "queued" -> "Waiting to process"
                "preparing" -> "Preparing audio"
                "transcribing" -> "Transcribing"
                "recognizing" -> "Recognizing speakers"
                "enriching" -> "Writing summary"
                "indexing" -> "Indexing for search"
                "paused" -> "Paused"
                "failed" -> "Processing stopped"
                "complete" -> "Ready"
                else -> step.replaceFirstChar(Char::uppercase)
            }
        }
    }
}

@Serializable
data class Speaker(
    val label: String,
    val personId: String? = null,
    val name: String? = null,
    val state: String = "unknown",
    val evidence: List<Evidence> = emptyList(),
) {
    @Serializable
    data class Evidence(val source: String = "", val name: String? = null, val quote: String? = null, val applied: Boolean? = null)

    val isIdentified get() = personId != null
    val keptUnidentified get() = state == "manual_unknown"

    /** Why the speaker is (or is not) named. */
    val explanations: List<String>
        get() {
            if (state == "manual") return listOf("Assigned by you")
            return evidence.mapNotNull { item ->
                // Evidence saved before `applied` existed was always applied.
                val applied = item.applied ?: true
                when {
                    item.source == "voice" -> if (applied) "Voice matches a saved profile" else "Voice resembles a saved profile"
                    !item.quote.isNullOrEmpty() ->
                        if (applied) "Introduced as ${item.name ?: "named"}: “${item.quote}”" else "Possibly ${item.name ?: "named"}: “${item.quote}”"
                    else -> null
                }
            }
        }
}

@Serializable
data class Segment(val id: String, val speaker: String, val startMs: Long, val endMs: Long, val text: String)

@Serializable
data class Person(val id: String, val name: String, val samples: Int = 0)

@Serializable
data class EntryPage(val entries: List<Entry> = emptyList(), val nextCursor: Int? = null)

@Serializable
data class SegmentPage(val segments: List<Segment> = emptyList(), val nextCursor: Int? = null)

@Serializable
data class PeoplePage(val people: List<Person> = emptyList())

/** The person's name, or `Speaker 2` by position in the speaker list. */
fun List<Speaker>.nameOf(label: String): String {
    val index = indexOfFirst { it.label == label }
    return getOrNull(index)?.name ?: "Speaker ${(if (index < 0) size else index) + 1}"
}

/** ISO 8601 with fractional seconds and any UTC offset. */
fun instant(text: String): Instant? = runCatching { OffsetDateTime.parse(text).toInstant() }.getOrNull()

/** A transcript passage that matches a search, ranked by the vault (keywords and meaning). */
@Serializable
data class SearchHit(
    val id: String,
    val entryId: String,
    val title: String = "",
    val recordedAt: String? = null,
    val startMs: Long = 0,
    val excerpt: String = "",
    val summary: Boolean = false,
)

@Serializable
data class SearchPage(val results: List<SearchHit> = emptyList())
