package dev.wsky.fundus

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.serialization.Serializable

/**
 * What the phone knows about past imports: each vault's source history, plus this phone's verified imports
 * that their vault has not listed yet.
 *
 * Every history read takes a ticket when it starts. A vault's answer is kept only if no read of that vault that
 * started later was kept first, so a slow read cannot bring back an older list; reads of different vaults never
 * displace each other. A verified import counts as imported until a read of its own vault, started after the
 * verification, lists it as a whole import, or until [SETTLE_READS] such reads have not: one read can miss a session the vault
 * has (a page skipped while something was trashed, a session sharing another's entry), and after a few the
 * recording may well have been trashed since, so that vault's history alone decides.
 */
@Serializable
data class ImportStatus(
    val histories: Map<String, History> = emptyMap(),
    /** Tickets handed out so far. */
    val issued: Long = 0,
    val unconfirmed: List<Unconfirmed> = emptyList(),
) {
    /** A vault's history, and the ticket of the read it came from. */
    @Serializable
    data class History(val entries: List<SourceHistoryEntry> = emptyList(), val accepted: Long = 0)

    @Serializable
    data class Unconfirmed(val server: String, val sourceIds: List<String>, val after: Long, val misses: Int = 0)

    companion object {
        /** Later reads of its vault that must omit a verified import before it stops counting. */
        const val SETTLE_READS = 3
    }

    /** A history read starts: the new state, and the read's ticket. */
    fun begin(): Pair<ImportStatus, Long> = copy(issued = issued + 1) to issued + 1

    /** A read with [ticket] of [server]'s history answered with [entries]. */
    fun accept(ticket: Long, server: String, entries: List<SourceHistoryEntry>): ImportStatus {
        if (ticket <= (histories[server]?.accepted ?: 0)) return this
        val listed = entries.mapNotNull { it.record }.filter { it.isWhole }.mapTo(HashSet()) { it.sourceIds }
        return copy(
            histories = histories + (server to History(entries, ticket)),
            unconfirmed = unconfirmed.mapNotNull {
                when {
                    it.server != server || it.after >= ticket -> it
                    it.sourceIds in listed || it.misses + 1 >= SETTLE_READS -> null
                    else -> it.copy(misses = it.misses + 1)
                }
            },
        )
    }

    /** This phone's import of these sessions was verified by [server], replacing an earlier record of them. */
    fun verified(server: String, sessions: List<List<String>>) = copy(
        unconfirmed = unconfirmed.filterNot { it.server == server && it.sourceIds in sessions } +
            sessions.map { Unconfirmed(server, it, issued) },
    )

    /** The imports to show for [server]: its history, verified imports it does not list yet, then the queue's. */
    fun records(server: String?, queue: QueueState): List<ImportRecord> {
        if (server == null) return emptyList()
        val listed = histories[server]?.entries.orEmpty().mapNotNull { it.record }
        // Only a whole-session import settles this phone's: a trimmed one (the Mac can trim) leaves the rest of the
        // session unimported.
        val whole = listed.filter { it.isWhole }
        val known = whole.mapTo(HashSet()) { it.sourceIds }
        val waiting = unconfirmed.filter { it.server == server && it.sourceIds !in known }.map { ImportRecord(it.sourceIds, "queued") }
        return listed + waiting + Uploads.queued(queue, whole + waiting, server)
    }
}

/** An import of the whole session: the vault stores no trim for one. */
private val ImportRecord.isWhole get() = (trimStartFrame ?: 0) == 0L && trimEndFrame == null

/** Changes the state and saves a changed result under [lock], so changes on different threads save in the order they happened. */
internal fun <T> MutableStateFlow<T>.changeAndSave(lock: Any, save: (T) -> Unit, change: (T) -> T): T = synchronized(lock) {
    val current = value
    val next = change(current)
    value = next
    if (next !== current) save(next)
    next
}
