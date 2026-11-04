package dev.wsky.fundus

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import androidx.core.content.edit
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonArray
import kotlinx.serialization.json.add
import okhttp3.FormBody
import okhttp3.Interceptor
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import okhttp3.Response
import java.io.IOException
import java.net.URI
import java.net.URLDecoder
import java.net.URLEncoder
import java.security.KeyStore
import java.security.MessageDigest
import java.security.SecureRandom
import java.util.Base64
import java.util.concurrent.TimeUnit
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** A failure worth showing as is. An IOException, so OkHttp and the player pass it through. */
open class VaultError(message: String) : IOException(message)

/** What went wrong, in words for the screen. */
fun Throwable.describe(): String = when (this) {
    is VaultError -> message.orEmpty()
    is IOException -> "Could not reach your vault. Check your connection and try again."
    else -> "Something went wrong: ${message ?: javaClass.simpleName}"
}

@Serializable
data class Credentials(
    val server: String,
    val clientId: String,
    val accessToken: String,
    val refreshToken: String,
    /** Epoch milliseconds. */
    val expiresAt: Long,
)

/**
 * The phone's owner connection to one vault: the Mac's browser OAuth with PKCE, except that the
 * approval comes back on `dev.wsky.fundus:/oauth` (see the manifest) instead of a loopback address.
 */
class Vault(context: Context) {
    private val prefs = context.getSharedPreferences("vault", Context.MODE_PRIVATE)
    private var credentials: Credentials? =
        prefs.getString(CREDENTIALS, null)?.let(Sealed::open)?.let { runCatching { VaultJson.decodeFromString<Credentials>(it) }.getOrNull() }

    private val _server = MutableStateFlow(credentials?.server)
    /** The connected vault's origin, or null when this phone is not connected. */
    val server: StateFlow<String?> = _server.asStateFlow()

    private val _signIn = MutableStateFlow(SignIn())
    val signIn: StateFlow<SignIn> = _signIn.asStateFlow()

    data class SignIn(val waiting: Boolean = false, val error: String? = null)

    var lastAddress: String
        get() = prefs.getString(ADDRESS, "").orEmpty()
        set(value) = prefs.edit { putString(ADDRESS, value) }

    private val plain = OkHttpClient.Builder().callTimeout(30, TimeUnit.SECONDS).build()

    /** Authenticated calls, for the API and for streaming audio. */
    val http: OkHttpClient = plain.newBuilder()
        .callTimeout(0, TimeUnit.SECONDS)
        .readTimeout(30, TimeUnit.SECONDS)
        .addInterceptor(::authorize)
        .build()

    // MARK: Sign-in

    /** Registers this app with the vault and returns the approval page to open in the browser. */
    suspend fun authorizationUrl(address: String): String = withContext(Dispatchers.IO) {
        _signIn.value = SignIn(waiting = true)
        try {
            val origin = OAuth.origin(address)
            val registration = post(origin, "/oauth/register", json = buildJsonObject {
                put("client_name", "Fundus for Android")
                putJsonArray("redirect_uris") { add(OAuth.CALLBACK) }
                put("token_endpoint_auth_method", "none")
                put("scope", "vault:owner")
                putJsonArray("grant_types") { add("authorization_code"); add("refresh_token") }
                putJsonArray("response_types") { add("code") }
            })
            val client = registration.string("client_id")
                ?: throw VaultError("The server did not register this app. Update the server and try again.")
            // The browser can outlive this process, so the request waits on disk for the callback.
            val pending = Pending(origin, client, OAuth.random(), OAuth.random())
            prefs.edit { putString(PENDING, Sealed.seal(VaultJson.encodeToString(Pending.serializer(), pending))) }
            lastAddress = origin
            OAuth.authorizationUrl(pending.server, pending.client, pending.state, OAuth.challenge(pending.verifier))
        } catch (error: Throwable) {
            _signIn.value = SignIn(error = error.describe())
            throw error
        }
    }

    /** Handles `dev.wsky.fundus:/oauth?code=…` from the browser. */
    suspend fun finishSignIn(callback: String) = withContext(Dispatchers.IO) {
        val pending = prefs.getString(PENDING, null)?.let(Sealed::open)
            ?.let { runCatching { VaultJson.decodeFromString(Pending.serializer(), it) }.getOrNull() }
        if (!callback.startsWith(OAuth.CALLBACK) || pending == null) return@withContext
        prefs.edit { remove(PENDING) }
        try {
            val code = OAuth.authorizationCode(callback, pending.state, pending.server)
            val tokens = post(pending.server, "/oauth/token", form = mapOf(
                "grant_type" to "authorization_code", "client_id" to pending.client, "code" to code,
                "redirect_uri" to OAuth.CALLBACK, "code_verifier" to pending.verifier,
                "resource" to pending.server + "/api/native",
            ))
            store(OAuth.credentials(tokens, pending.server, pending.client))
            _signIn.value = SignIn()
        } catch (error: Throwable) {
            _signIn.value = SignIn(error = error.describe())
        }
    }

    /** The person came back from the browser without approving. */
    fun cancelSignIn() {
        prefs.edit { remove(PENDING) }
        _signIn.value = SignIn()
    }

    suspend fun signOut() = withContext(Dispatchers.IO) {
        credentials?.let { saved ->
            runCatching { post(saved.server, "/oauth/revoke", form = mapOf("client_id" to saved.clientId, "token" to saved.refreshToken)) }
        }
        forget(null)
    }

    private fun store(updated: Credentials) {
        prefs.edit(commit = true) { putString(CREDENTIALS, Sealed.seal(VaultJson.encodeToString(Credentials.serializer(), updated))) }
        credentials = updated
        _server.value = updated.server
    }

    private fun forget(reason: String?) {
        prefs.edit(commit = true) { remove(CREDENTIALS) }
        credentials = null
        _server.value = null
        _signIn.value = SignIn(error = reason)
    }

    // MARK: Requests

    /** An owner API call (`/entries`, …) on the connected vault; returns the response body. */
    suspend fun request(path: String, method: String = "GET", body: JsonObject? = null): String = withContext(Dispatchers.IO) {
        val request = Request.Builder().url(url(path))
            .method(method, body?.toString()?.toRequestBody(JSON) ?: if (method == "GET") null else ByteArray(0).toRequestBody())
            .build()
        http.newCall(request).execute().use { response ->
            val text = response.body.string()
            if (!response.isSuccessful) throw VaultError(errorMessage(text) ?: "The vault could not do that (HTTP ${response.code}).")
            text
        }
    }

    fun url(path: String): String {
        val server = server.value ?: throw VaultError("Connect this app to your vault first.")
        return "$server/api/native/v1$path"
    }

    private fun authorize(chain: Interceptor.Chain): Response {
        val used = runBlocking { current(rejected = null) }
        val response = chain.proceed(chain.request().newBuilder().header("Authorization", "Bearer ${used.accessToken}").build())
        if (response.code != 401) return response
        response.close()
        val fresh = runBlocking { current(rejected = used.accessToken) }
        return chain.proceed(chain.request().newBuilder().header("Authorization", "Bearer ${fresh.accessToken}").build())
    }

    /** One refresh at a time: the server rotates refresh tokens and revokes a reused one. */
    private val refreshing = Mutex()

    private suspend fun current(rejected: String?): Credentials {
        val saved = credentials ?: throw VaultError("Connect this app to your vault first.")
        if (rejected == null && saved.fresh) return saved
        return refreshing.withLock {
            val latest = credentials ?: throw VaultError("Connect this app to your vault first.")
            if (latest.accessToken != rejected && latest.fresh) return@withLock latest
            val tokens = try {
                post(latest.server, "/oauth/token", form = mapOf(
                    "grant_type" to "refresh_token", "client_id" to latest.clientId,
                    "refresh_token" to latest.refreshToken, "resource" to latest.server + "/api/native",
                ))
            } catch (expired: Expired) {
                forget(expired.message)
                throw VaultError(expired.message.orEmpty())
            }
            OAuth.credentials(tokens, latest.server, latest.clientId).also(::store)
        }
    }

    private val Credentials.fresh get() = expiresAt - System.currentTimeMillis() > 60_000

    private class Expired : VaultError("This connection has expired or was revoked. Connect again.")

    private fun post(server: String, path: String, json: JsonObject? = null, form: Map<String, String>? = null): JsonObject {
        val body = json?.toString()?.toRequestBody(JSON)
            ?: FormBody.Builder().apply { form.orEmpty().forEach { (key, value) -> add(key, value) } }.build()
        plain.newCall(Request.Builder().url(server + path).post(body).build()).execute().use { response ->
            val value = runCatching { VaultJson.parseToJsonElement(response.body.string()).jsonObject }.getOrDefault(JsonObject(emptyMap()))
            if (response.isSuccessful) return value
            if (value.string("error") == "invalid_grant") throw Expired()
            throw VaultError(
                if (value.string("error") == "invalid_client_metadata") "This vault does not accept phone sign-in yet. Update the server and try again."
                else if (path == "/oauth/register") "The server could not register this app. Check the address and try again."
                else "The server could not connect this app. Check the server address and try again."
            )
        }
    }

    @Serializable
    private data class Pending(val server: String, val client: String, val state: String, val verifier: String)

    private companion object {
        const val CREDENTIALS = "credentials"
        const val PENDING = "pending-sign-in"
        const val ADDRESS = "address"
        val JSON = "application/json".toMediaType()

        fun errorMessage(body: String) = runCatching { VaultJson.parseToJsonElement(body).jsonObject.string("error") }.getOrNull()
    }
}

private fun JsonObject.string(key: String) = get(key)?.jsonPrimitive?.contentOrNull

/** The OAuth steps that need no network or Android, kept apart so tests can run them. */
object OAuth {
    const val CALLBACK = "dev.wsky.fundus:/oauth"

    /** `https://Vault.example.com/` → `https://vault.example.com`; a bare host gets `https://`. */
    fun origin(address: String): String {
        val trimmed = address.trim()
        if (trimmed.isEmpty()) throw VaultError("Enter your vault’s server address, for example vault.example.com.")
        val uri = runCatching { URI(if ("://" in trimmed) trimmed else "https://$trimmed") }.getOrNull()
        val scheme = uri?.scheme?.lowercase()
        val host = uri?.host?.lowercase()
        if (uri == null || host.isNullOrEmpty() || scheme !in setOf("https", "http")) {
            throw VaultError("Enter a complete server address, for example https://vault.example.com.")
        }
        if (uri.rawUserInfo != null || uri.rawQuery != null || uri.rawFragment != null || uri.rawPath !in setOf("", "/")) {
            throw VaultError("Use only your vault’s server address, without a page path, query, or login details.")
        }
        if (scheme == "http" && host !in setOf("localhost", "127.0.0.1", "[::1]")) {
            throw VaultError("Use your vault’s HTTPS address. HTTP is allowed only for local development.")
        }
        val port = uri.port.takeUnless { it == -1 || it == (if (scheme == "https") 443 else 80) }
        return "$scheme://$host" + (port?.let { ":$it" } ?: "")
    }

    fun authorizationUrl(server: String, client: String, state: String, challenge: String): String {
        val query = listOf(
            "response_type" to "code", "client_id" to client, "redirect_uri" to CALLBACK,
            "code_challenge" to challenge, "code_challenge_method" to "S256", "scope" to "vault:owner",
            "resource" to "$server/api/native", "state" to state,
        ).joinToString("&") { (key, value) -> "$key=${URLEncoder.encode(value, "UTF-8")}" }
        return "$server/oauth/authorize?$query"
    }

    fun random(): String = ByteArray(48).also(SecureRandom()::nextBytes).let(::base64Url)

    /** PKCE S256. */
    fun challenge(verifier: String): String =
        base64Url(MessageDigest.getInstance("SHA-256").digest(verifier.toByteArray()))

    private fun base64Url(bytes: ByteArray) = Base64.getUrlEncoder().withoutPadding().encodeToString(bytes)

    /** The code from the callback, after checking it answers our request and comes from our vault. */
    fun authorizationCode(callback: String, state: String, issuer: String): String {
        val items = URI(callback).rawQuery.orEmpty().split("&").filter { it.isNotEmpty() }.map {
            val (key, value) = (it.split("=", limit = 2) + "").take(2)
            URLDecoder.decode(key, "UTF-8") to URLDecoder.decode(value, "UTF-8")
        }
        fun single(name: String) = items.filter { it.first == name }.singleOrNull()?.second
        if (single("state") != state || single("iss") != issuer) {
            throw VaultError("The connection response could not be verified. Start connecting again.")
        }
        single("error")?.let {
            throw VaultError(if (it == "access_denied") "Vault access was not approved." else "The server could not approve this connection. Try again.")
        }
        return single("code")?.takeIf { it.isNotEmpty() }
            ?: throw VaultError("The server did not return an authorization code. Try connecting again.")
    }

    fun credentials(response: JsonObject, server: String, client: String, now: Long = System.currentTimeMillis()): Credentials {
        val access = response.string("access_token")
        val refresh = response.string("refresh_token")
        val expires = response["expires_in"]?.jsonPrimitive?.doubleOrNull
        if (response.string("scope") != "vault:owner" || access.isNullOrEmpty() || refresh.isNullOrEmpty() || expires == null || expires <= 0) {
            throw VaultError("The server returned an incomplete connection. Update the server and connect again.")
        }
        return Credentials(server, client, access, refresh, now + (expires * 1000).toLong())
    }
}

/** Secrets at rest: AES-GCM under a key that never leaves the Android Keystore. */
private object Sealed {
    private const val ALIAS = "fundus.vault"

    private fun key(): SecretKey {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (store.getKey(ALIAS, null) as? SecretKey)?.let { return it }
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore").apply {
            init(
                KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                    .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                    .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                    .build()
            )
        }.generateKey()
    }

    fun seal(text: String): String {
        val cipher = Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.ENCRYPT_MODE, key()) }
        return Base64.getEncoder().encodeToString(cipher.iv + cipher.doFinal(text.toByteArray()))
    }

    /** Null when the value cannot be read, e.g. after the key was lost with a device reset. */
    fun open(value: String): String? = runCatching {
        val bytes = Base64.getDecoder().decode(value)
        val cipher = Cipher.getInstance("AES/GCM/NoPadding").apply {
            init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, bytes, 0, 12))
        }
        String(cipher.doFinal(bytes, 12, bytes.size - 12))
    }.getOrNull()
}
