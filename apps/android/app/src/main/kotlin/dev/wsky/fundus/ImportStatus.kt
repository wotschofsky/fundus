package dev.wsky.fundus

import kotlinx.serialization.Serializable

/**
 * What the phone knows about past imports: the vault's source history (and which vault it came
 * from), plus this phone's verified imports that no history read since has confirmed.
 *
 * Every history read takes a ticket when it starts. An answer is kept only if no read that started
 * later was kept first, so a slow read cannot bring back an older list. A verified import counts as
 * imported until a read of its own vault, started after the verification, has been kept; from then on
 * that vault's history alone decides (the recording may have been trashed since).
 */
@Serializable
data class ImportStatus(
    val server: String? = null,
    val history: List<SourceHistoryEntry> = emptyList(),
    /** Tickets handed out so far, and the newest one whose answer was kept. */
    val issued: Long = 0,
    val accepted: Long = 0,
    val unconfirmed: List<Unconfirmed> = emptyList(),
) {
    @Serializable
    data class Unconfirmed(val server: String, val sourceIds: List<String>, val after: Long)

    /** A history read starts: the new state, and the read's ticket. */
    fun begin(): Pair<ImportStatus, Long> = copy(issued = issued + 1) to issued + 1

    /** A read with [ticket] of [server]'s history answered with [entries]. */
    fun accept(ticket: Long, server: String, entries: List<SourceHistoryEntry>): ImportStatus {
        if (ticket <= accepted) return this
        return copy(
            server = server,
            history = entries,
            accepted = ticket,
            unconfirmed = unconfirmed.filterNot { it.server == server && it.after < ticket },
        )
    }

    /** This phone's import of these sessions was verified by [server]. */
    fun verified(server: String, sessions: List<List<String>>) =
        copy(unconfirmed = unconfirmed + sessions.map { Unconfirmed(server, it, issued) })

    /** The imports to show for [server]: its history, verified imports it does not list yet, then the queue's. */
    fun records(server: String?, queue: QueueState): List<ImportRecord> {
        if (server == null) return emptyList()
        val listed = if (this.server == server) history.mapNotNull { it.record } else emptyList()
        val known = listed.mapTo(HashSet()) { it.sourceIds }
        val waiting = unconfirmed.filter { it.server == server && it.sourceIds !in known }.map { ImportRecord(it.sourceIds, "queued") }
        return listed + waiting + Uploads.queued(queue, listed + waiting, server)
    }
}
