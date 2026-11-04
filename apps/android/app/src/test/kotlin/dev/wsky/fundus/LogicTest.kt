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
}
