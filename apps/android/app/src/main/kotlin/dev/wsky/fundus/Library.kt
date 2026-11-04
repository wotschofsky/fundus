package dev.wsky.fundus

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import java.net.URLEncoder

/** A typed owner API read; decoding runs off the main thread. */
suspend inline fun <reified T> Vault.get(path: String): T {
    val text = request(path)
    return withContext(Dispatchers.Default) {
        try {
            VaultJson.decodeFromString<T>(text)
        } catch (_: IllegalArgumentException) {
            throw VaultError("The vault sent a response this app could not read. Update the server or the app.")
        }
    }
}

/** One path segment, e.g. an entry id or speaker label. */
fun segment(value: String): String = URLEncoder.encode(value, "UTF-8").replace("+", "%20")

/**
 * The connected vault's recordings and people, shared by every screen (the Mac's `Library.swift`).
 * Call it from the main thread; only the requests leave it.
 */
class Library(private val vault: Vault) {
    data class State(
        val entries: List<Entry> = emptyList(),
        val people: List<Person> = emptyList(),
        val hasMore: Boolean = false,
        val loaded: Boolean = false,
        val loading: Boolean = false,
        val error: String? = null,
    ) {
        val isProcessing get() = entries.any { it.isProcessing }
    }

    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    private var server: String? = null
    private var nextCursor: Int? = null
    /** Bumped when the vault changes, so a late response from the previous vault is dropped. */
    private var generation = 0

    /** Recordings whose title, summary or labels contain `query`. */
    fun matching(query: String): List<Entry> {
        val needle = query.trim()
        val entries = state.value.entries
        if (needle.isEmpty()) return entries
        return entries.filter { entry ->
            entry.displayTitle.contains(needle, ignoreCase = true) ||
                entry.summary.orEmpty().contains(needle, ignoreCase = true) ||
                entry.labels.any { it.contains(needle, ignoreCase = true) }
        }
    }

    /** Reloads the newest page and keeps older pages that were already loaded. */
    suspend fun refresh() {
        val current = vault.server.value ?: return
        if (current != server) {
            generation++
            server = current
            nextCursor = null
            _state.value = State()
        }
        val generation = generation
        _state.update { it.copy(loading = true) }
        try {
            val page = vault.get<EntryPage>("/entries?limit=100")
            if (generation != this.generation) return
            val known = state.value.entries
            var merged = page.entries
            val boundary = page.entries.lastOrNull()?.let { last -> known.indexOfFirst { it.id == last.id } } ?: -1
            if (page.nextCursor != null && boundary >= 0) {
                val fresh = page.entries.mapTo(HashSet()) { it.id }
                merged = merged + known.drop(boundary + 1).filter { it.id !in fresh }
            }
            nextCursor = if (page.nextCursor == null) null else merged.size
            _state.update { it.copy(entries = merged, hasMore = nextCursor != null, loaded = true, error = null) }
        } catch (error: Exception) {
            fail(error, generation)
        } finally {
            if (generation == this.generation) _state.update { it.copy(loading = false) }
        }
    }

    suspend fun loadMore() {
        val cursor = nextCursor ?: return
        if (state.value.loading) return
        val generation = generation
        _state.update { it.copy(loading = true) }
        try {
            val page = vault.get<EntryPage>("/entries?limit=100&cursor=$cursor")
            if (generation != this.generation) return
            nextCursor = page.nextCursor
            _state.update { state ->
                val known = state.entries.mapTo(HashSet()) { it.id }
                state.copy(entries = state.entries + page.entries.filter { it.id !in known }, hasMore = nextCursor != null)
            }
        } catch (error: Exception) {
            fail(error, generation)
        } finally {
            if (generation == this.generation) _state.update { it.copy(loading = false) }
        }
    }

    /** Passages inside transcripts, from the vault's keyword and semantic search. */
    suspend fun search(query: String): List<SearchHit> =
        vault.get<SearchPage>("/search?limit=30&q=${URLEncoder.encode(query.trim(), "UTF-8")}").results

    suspend fun refreshPeople() {
        val generation = generation
        try {
            val page = vault.get<PeoplePage>("/people")
            if (generation == this.generation) _state.update { it.copy(people = page.people.sortedBy { person -> person.name.lowercase() }) }
        } catch (error: Exception) {
            fail(error, generation)
        }
    }

    private fun fail(error: Exception, generation: Int) {
        if (error is CancellationException) throw error
        if (generation == this.generation) _state.update { it.copy(error = error.describe()) }
    }

    // MARK: Recordings

    enum class Action(val suffix: String) {
        Trash("/trash"), Restore("/restore"), Pause("/pause"), Resume("/retry"),
        ReanalyzeSpeakers("/reanalyze"), RegenerateSummary("/summarize"),
    }

    /** Throws when the vault refuses; the list is reloaded either way. */
    suspend fun perform(action: Action, id: String) {
        if (action == Action.Trash) _state.update { state -> state.copy(entries = state.entries.filterNot { it.id == id }) }
        try {
            vault.request("/entries/${segment(id)}${action.suffix}", "POST")
        } finally {
            refresh()
        }
    }

    // MARK: Speakers and people

    /** Names the speaker, or with `personId == null` keeps it unidentified (`locked`) or hands it back to recognition. */
    suspend fun assign(personId: String?, speaker: String, entry: String, locked: Boolean = true) {
        vault.request("/entries/${segment(entry)}/speakers/${segment(speaker)}", "PATCH", buildJsonObject {
            put("person_id", personId)
            if (personId == null) put("locked", locked)
        })
        refreshPeople()
    }

    suspend fun addPerson(name: String): Person {
        val text = vault.request("/people", "POST", buildJsonObject { put("name", name.trim()) })
        refreshPeople()
        return VaultJson.decodeFromString<Person>(text)
    }
}
