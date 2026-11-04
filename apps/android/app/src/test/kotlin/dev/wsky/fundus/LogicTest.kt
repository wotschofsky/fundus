package dev.wsky.fundus

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Test
import java.time.Instant

class LogicTest {
    @Test
    fun pkceMatchesRfc7636() {
        assertEquals("E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM", OAuth.challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"))
    }

    @Test
    fun callbackMustAnswerOurRequestFromOurVault() {
        val issuer = "https://vault.example.com"
        val ok = "dev.wsky.fundus:/oauth?code=abc&state=s1&iss=https%3A%2F%2Fvault.example.com"
        assertEquals("abc", OAuth.authorizationCode(ok, "s1", issuer))
        assertThrows(VaultError::class.java) { OAuth.authorizationCode(ok, "other", issuer) }
        assertThrows(VaultError::class.java) { OAuth.authorizationCode(ok, "s1", "https://evil.example.com") }
        assertThrows(VaultError::class.java) { OAuth.authorizationCode("$ok&state=s1", "s1", issuer) }
        val denied = "dev.wsky.fundus:/oauth?error=access_denied&state=s1&iss=https%3A%2F%2Fvault.example.com"
        assertEquals("Vault access was not approved.", assertThrows(VaultError::class.java) { OAuth.authorizationCode(denied, "s1", issuer) }.message)
    }

    @Test
    fun addressesBecomeOrigins() {
        assertEquals("https://vault.example.com", OAuth.origin(" Vault.Example.com "))
        assertEquals("https://vault.example.com", OAuth.origin("https://vault.example.com:443/"))
        assertEquals("https://vault.example.com:8443", OAuth.origin("https://vault.example.com:8443"))
        assertEquals("http://127.0.0.1:3917", OAuth.origin("http://127.0.0.1:3917"))
        for (bad in listOf("", "http://vault.example.com", "https://vault.example.com/library", "https://me@vault.example.com", "ftp://vault.example.com")) {
            assertThrows(bad, VaultError::class.java) { OAuth.origin(bad) }
        }
    }

    @Test
    fun tokenResponsesNeedOwnerScope() {
        val good = Json.parseToJsonElement("""{"access_token":"a","refresh_token":"r","expires_in":3600,"scope":"vault:owner"}""") as JsonObject
        assertEquals(1_000 + 3_600_000L, OAuth.credentials(good, "https://v", "c", now = 1_000).expiresAt)
        val narrow = Json.parseToJsonElement("""{"access_token":"a","refresh_token":"r","expires_in":3600,"scope":"vault:read"}""") as JsonObject
        assertThrows(VaultError::class.java) { OAuth.credentials(narrow, "https://v", "c") }
    }

    @Test
    fun entriesDecodeAndUnnamedSpeakersAreNumbered() {
        val entry = VaultJson.decodeFromString<Entry>(
            """{"id":"e1","title":"Planning","recorded_at":"2026-10-30T08:15:00.000Z","created_at":"2026-10-30T09:20:00+01:00",
            "duration_ms":3240000,"timezone":"Europe/Berlin","stage":"complete","active_stage":null,"pause_requested":false,
            "labels":["work"],"warnings":[],"audio_available":true,"something_new":1,
            "speakers":[{"label":"A","person_id":"p1","name":"Alex","state":"automatic","evidence":[{"source":"voice","applied":true}]},
                        {"label":"B","person_id":null,"name":null,"state":"unknown","evidence":[]}]}""",
        )
        assertEquals(Instant.parse("2026-10-30T08:15:00Z"), entry.date)
        val speakers = entry.speakers.orEmpty()
        assertEquals("Alex", speakers.nameOf("A"))
        assertEquals("Speaker 2", speakers.nameOf("B"))
        assertEquals(listOf("Voice matches a saved profile"), speakers[0].explanations)
        assertNull(speakers[1].name)
    }

    @Test
    fun durationsAndPositions() {
        assertEquals("32s", Format.duration(32.0))
        assertEquals("54m", Format.duration(54 * 60.0))
        assertEquals("2h 09m", Format.duration(129 * 60.0))
        assertEquals("2:03", Format.position(123.4))
        assertEquals("1:02:03", Format.position(3723.0))
    }

    @Test
    fun timelineFollowsSourceFilesAfterTheTrim() {
        val entry = VaultJson.decodeFromString<Entry>(
            """{"id":"e","created_at":"2026-10-30T09:00:00Z","recorded_at":"2026-10-30T09:00:00Z","stage":"complete","trim_start_frame":48000,
            "sources":[{"offset_ms":0,"recorded_at":"2026-10-30T09:00:00Z","format":{"sample_rate":48000}},
                       {"offset_ms":60000,"recorded_at":"2026-10-30T10:00:00Z","format":{"sample_rate":48000}}]}""",
        )
        val timeline = SessionTimeline.of(entry)
        // One trimmed second: audio position 0 is 09:00:01; the second file starts at 59 s.
        assertEquals(Instant.parse("2026-10-30T09:00:01Z"), timeline.at(0))
        assertEquals(Instant.parse("2026-10-30T10:00:00Z"), timeline.at(59_000))
    }

    // MARK: Recorder sessions (parity with apps/macos SessionGrouper and SourceSessions)

    private fun recording(name: String, seconds: Double, zone: java.time.ZoneId = java.time.ZoneOffset.UTC): SourceRecording {
        val parsed = DJIFilename.parse(name)!!
        val frames = (seconds * 48_000).toLong()
        return SourceRecording("content://$name", name, parsed.transmitter, parsed.variant, parsed.sequence, parsed.startInstant(zone), frames, frames * 3, AudioFormat(48_000, 1, 24, false))
    }

    @Test
    fun djiNamesAreParsedAndImpossibleDatesRejected() {
        val name = DJIFilename.parse("tx01_MIC002_20261001_204349_ORIG.wav")!!
        assertEquals("TX01", name.transmitter)
        assertEquals(2, name.sequence)
        assertEquals("orig", name.variant)
        assertEquals(java.time.LocalDateTime.of(2026, 10, 1, 20, 43, 49), name.start)
        assertNull(DJIFilename.parse("TX01_MIC001_20260230_120000.wav"))
        assertNull(DJIFilename.parse("TX01_MIC001_20261001_120000.mp3"))
    }

    @Test
    fun sourceIdsMatchTheMac() {
        // SHA-256 of the Mac's fingerprint `lowercased name|bytes|frames`.
        val file = recording("TX01_MIC001_20261001_120000.wav", 1.0).copy(fileSize = 1000, frameCount = 48_000)
        assertEquals("8a4534a6b8389f2512636a6b02d90d6aa36b16097a3b94fc031ab952ad9237f0", file.sourceId)
    }

    @Test
    fun rolloversJoinAndEverythingElseSplits() {
        val sessions = SessionGrouper.group(listOf(
            recording("TX01_MIC001_20261001_100000_orig.wav", 1800.0),
            recording("TX01_MIC002_20261001_103002_orig.wav", 600.0), // 2 s late: still the same session
            recording("TX01_MIC003_20261001_104500_orig.wav", 1800.0), // a gap
            recording("TX01_MIC004_20261001_111503_orig.wav", 60.0),  // 3 s late: a new session
            recording("TX02_MIC001_20261001_100000_orig.wav", 1800.0), // another microphone
            recording("TX01_MIC001_20261001_100000_nc.wav", 1800.0),   // another rendition
        ))
        assertEquals(listOf(2, 1, 1, 1, 1), sessions.map { it.recordings.size })
        assertEquals(2400.0, sessions.first { it.recordings.size == 2 }.duration, 0.001)
    }

    @Test
    fun importedSessionsAreNotProposedAgain() {
        val long = SessionGrouper.group(listOf(recording("TX01_MIC001_20261001_100000.wav", 600.0)))
        val short = SessionGrouper.group(listOf(recording("TX01_MIC002_20261001_120000.wav", 60.0)))
        val sessions = long + short
        assertEquals(long, Imports.proposed(sessions, emptyList()))
        val done = ImportRecord(long[0].sourceIds, "complete")
        assertEquals(done, Imports.record(long[0], listOf(done)))
        assertEquals(emptyList<RecordingSession>(), Imports.proposed(sessions, listOf(done)))
        // Imported trimmed elsewhere: only a part of it is in the vault.
        val trimmed = done.copy(trimStartFrame = 48_000)
        assertNull(Imports.record(long[0], listOf(trimmed)))
        assertEquals(true, Imports.partlyImported(long[0], listOf(trimmed)))
    }

    @Test
    fun wavHeadersGiveExactFrameCounts() {
        fun wav(bits: Int, format: Int, frames: Int, junk: Int = 0): ByteArray {
            val data = frames * 2 * bits / 8
            val buffer = java.nio.ByteBuffer.allocate(12 + 8 + junk + junk % 2 + 24 + 8 + data).order(java.nio.ByteOrder.LITTLE_ENDIAN)
            buffer.put("RIFF".toByteArray()).putInt(buffer.capacity() - 8).put("WAVE".toByteArray())
            buffer.put("JUNK".toByteArray()).putInt(junk).put(ByteArray(junk + junk % 2)) // odd chunks carry a pad byte
            buffer.put("fmt ".toByteArray()).putInt(16).putShort(format.toShort()).putShort(2).putInt(48_000).putInt(48_000 * 2 * bits / 8).putShort((2 * bits / 8).toShort()).putShort(bits.toShort())
            buffer.put("data".toByteArray()).putInt(data)
            return buffer.array()
        }
        val pcm = wav(24, 1, 1000, junk = 27).let { WavHeader.read(it.inputStream(), it.size.toLong()) }
        assertEquals(AudioFormat(48_000, 2, 24, false) to 1000L, pcm)
        assertEquals(true, pcm.first.compresses)
        val float = wav(32, 3, 10).let { WavHeader.read(it.inputStream(), it.size.toLong()) }
        assertEquals(false, float.first.compresses)
        // DJI mics declare 8 bytes more than they write; like Core Audio, only whole frames in the file count.
        val dji = wav(24, 1, 1000).let { it.copyOf(it.size - 8) }
        assertEquals(1000L - 2, WavHeader.read(dji.inputStream(), dji.size.toLong()).second)
        val empty = wav(16, 1, 0)
        assertThrows(IllegalArgumentException::class.java) { WavHeader.read(empty.inputStream(), empty.size.toLong()) }
    }

    @Test
    fun aVaultsTokenOnlyGoesToThatVault() {
        assertEquals(true, OAuth.sameOrigin("https://vault.example.com/api/native/v1/uploads/abc", "https://vault.example.com"))
        assertEquals(true, OAuth.sameOrigin("https://VAULT.example.com:443/x", "https://vault.example.com"))
        assertEquals(false, OAuth.sameOrigin("https://other.example.com/api/native/v1/uploads/abc", "https://vault.example.com"))
        assertEquals(false, OAuth.sameOrigin("http://vault.example.com/x", "https://vault.example.com"))
        assertEquals(false, OAuth.sameOrigin("https://vault.example.com:8443/x", "https://vault.example.com"))
        assertEquals(false, OAuth.sameOrigin("https://vault.example.com.evil.example/x", "https://vault.example.com"))
    }

    @Test
    fun onlyAnUnfinishedUploadOnThisVaultStandsInForTheVaultsHistory() {
        val plan = SessionPlan(listOf("a", "b"))
        val file = UploadItem(name = "TX01_MIC001_20261001_120000.wav", source = "x.wav")
        val running = QueueState(server = "https://vault", state = "uploading", sessions = listOf(plan), files = listOf(file))
        assertEquals(listOf(ImportRecord(listOf("a", "b"), "uploading", importId = "")), Uploads.queued(running, emptyList(), "https://vault"))
        // The vault lists it: the vault's record is the one that counts.
        assertEquals(emptyList<ImportRecord>(), Uploads.queued(running, listOf(ImportRecord(listOf("a", "b"), "complete")), "https://vault"))
        // Done, then trashed in the vault: importable again.
        assertEquals(emptyList<ImportRecord>(), Uploads.queued(running.copy(state = "verified"), emptyList(), "https://vault"))
        // Another vault knows nothing of it.
        assertEquals(emptyList<ImportRecord>(), Uploads.queued(running, emptyList(), "https://other"))
        assertEquals(true, running.unfinished)
        assertEquals(true, running.copy(state = "failed").unfinished)
        assertEquals(false, running.copy(state = "verified").unfinished)
        assertEquals(false, QueueState().unfinished)
    }

    @Test
    fun uploadAddressesArePathsOnTheConnectedVault() {
        assertEquals("/uploads/abc", Uploads.uploadPath("/api/native/v1/uploads/abc"))
        assertNull(Uploads.uploadPath("https://elsewhere.example/api/native/v1/uploads/abc"))
        assertNull(Uploads.uploadPath("/api/native/v1/entries"))
        assertNull(Uploads.uploadPath(null))
    }

    @Test
    fun micsAreCountedByDriveNotByGrant() {
        assertEquals("5D17-AC17", Recorder.drive("5D17-AC17:"))
        assertEquals("5D17-AC17", Recorder.drive("5D17-AC17:TX_MIC003_20261101_224401"))
        // The phone's own storage never holds a mic.
        assertNull(Recorder.drive("primary:Music"))
        assertNull(Recorder.drive("home:"))
    }

    @Test
    fun aPluggedInMicIsOnlyAskedForWhenItWasNeverAllowed() {
        // Never allowed: ask at once.
        assertEquals(Recorder.MicState(pending = true, waiting = false), Recorder.micState(mics = 1, drives = 0, readable = 0, withRecordings = 0))
        // Allowed, drive still mounting: wait for it rather than asking again.
        assertEquals(Recorder.MicState(pending = true, waiting = true), Recorder.micState(mics = 1, drives = 1, readable = 0, withRecordings = 0))
        // Allowed and readable but empty (or its listing not there yet): never a permission prompt.
        assertEquals(Recorder.MicState(pending = false, waiting = true), Recorder.micState(mics = 1, drives = 1, readable = 1, withRecordings = 0))
        assertEquals(Recorder.MicState(pending = false, waiting = false), Recorder.micState(mics = 1, drives = 1, readable = 1, withRecordings = 1))
        // A second transmitter that was never allowed.
        assertEquals(Recorder.MicState(pending = true, waiting = false), Recorder.micState(mics = 2, drives = 1, readable = 1, withRecordings = 1))
    }
}
