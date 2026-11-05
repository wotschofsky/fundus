package dev.wsky.fundus

import kotlinx.serialization.Serializable

/**
 * What the phone knows about past imports: each vault's source history, plus this phone's verified imports
 * that no history read since has confirmed.
 *
 * Every history read takes a ticket when it starts. A vault's answer is kept only if no read of that vault that
 * started later was kept first, so a slow read cannot bring back an older list; reads of different vaults never
 * displace each other. A verified import counts as imported until a read of its own vault, started after the
 * verification, has been kept; from then on that vault's history alone decides (the recording may have been
 * trashed since).
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
    data class Unconfirmed(val server: String, val sourceIds: List<String>, val after: Long)

    /** A history read starts: the new state, and the read's ticket. */
    fun begin(): Pair<ImportStatus, Long> = copy(issued = issued + 1) to issued + 1

    /** A read with [ticket] of [server]'s history answered with [entries]. */
    fun accept(ticket: Long, server: String, entries: List<SourceHistoryEntry>): ImportStatus {
        if (ticket <= (histories[server]?.accepted ?: 0)) return this
        return copy(
            histories = histories + (server to History(entries, ticket)),
            unconfirmed = unconfirmed.filterNot { it.server == server && it.after < ticket },
        )
    }

    /** This phone's import of these sessions was verified by [server]. */
    fun verified(server: String, sessions: List<List<String>>) =
        copy(unconfirmed = unconfirmed + sessions.map { Unconfirmed(server, it, issued) })

    /** The imports to show for [server]: its history, verified imports it does not list yet, then the queue's. */
    fun records(server: String?, queue: QueueState): List<ImportRecord> {
        if (server == null) return emptyList()
        val listed = histories[server]?.entries.orEmpty().mapNotNull { it.record }
        val known = listed.mapTo(HashSet()) { it.sourceIds }
        val waiting = unconfirmed.filter { it.server == server && it.sourceIds !in known }.map { ImportRecord(it.sourceIds, "queued") }
        return listed + waiting + Uploads.queued(queue, listed + waiting, server)
    }
}
