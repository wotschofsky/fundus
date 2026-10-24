package dev.wsky.vaulttransfer

import android.content.Context
import android.net.Uri
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.AtomicFile
import android.util.Base64
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.io.IOException
import java.io.RandomAccessFile
import java.net.HttpURLConnection
import java.net.URI
import java.security.KeyStore
import java.security.MessageDigest
import java.util.TimeZone
import java.util.UUID
import java.util.concurrent.atomic.AtomicBoolean
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

object AudioCodec {
  @Volatile var cancelled = false
  fun isCancelled(): Boolean = cancelled
  init { System.loadLibrary("vault_transfer") }
  external fun prepare(source: String, destination: String): String
  external fun canCompress(source: String): Boolean
}
class UploadQueue private constructor(private val context: Context) {
  companion object {
    @Volatile private var instance: UploadQueue? = null
    fun get(context: Context): UploadQueue = instance ?: synchronized(this) { instance ?: UploadQueue(context.applicationContext).also { instance = it } }
  }
  private val folder = File(context.filesDir, "upload-queue").apply { mkdirs() }
  private val journal = AtomicFile(File(folder, "queue.json"))
  private var state = try { JSONObject(String(journal.readFully(), Charsets.UTF_8)) } catch (_: Exception) { JSONObject().put("state", "idle").put("server", "").put("timezone", TimeZone.getDefault().id).put("files", JSONArray()) }
  private val running = AtomicBoolean(false)
  @Volatile private var paused = false
  @Synchronized fun status(): String = state.toString()
  @Synchronized private fun save() { val stream = journal.startWrite(); try { stream.write(state.toString().toByteArray()); journal.finishWrite(stream) } catch (e: Exception) { journal.failWrite(stream); throw e } }
  @Synchronized fun configure(server: String, token: String, timezone: String) {
    validateOrigin(server)
    require(token.isNotBlank()) { "Enter an upload token" }
    require(TimeZone.getAvailableIDs().contains(timezone)) { "Choose a valid recorder timezone" }
    require(state.optString("state") in listOf("idle", "verified") || state.optString("server") == server) { "Finish or clear the queue before changing servers" }
    val cipher = Cipher.getInstance("AES/GCM/NoPadding"); cipher.init(Cipher.ENCRYPT_MODE, key())
    val encrypted = cipher.doFinal(token.toByteArray())
    context.getSharedPreferences("vault", Context.MODE_PRIVATE).edit().putString("token", Base64.encodeToString(cipher.iv + encrypted, Base64.NO_WRAP)).commit()
    state.put("server", server.trimEnd('/')).put("timezone", timezone); save()
  }
  private fun key(): SecretKey {
    val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
    (store.getKey("vault-upload", null) as? SecretKey)?.let { return it }
    return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore").apply { init(KeyGenParameterSpec.Builder("vault-upload", KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT).setBlockModes(KeyProperties.BLOCK_MODE_GCM).setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE).build()) }.generateKey()
  }
  private fun token(): String {
    val value = context.getSharedPreferences("vault", Context.MODE_PRIVATE).getString("token", null) ?: throw IOException("Connect with an upload token first")
    val bytes = Base64.decode(value, Base64.NO_WRAP)
    val cipher = Cipher.getInstance("AES/GCM/NoPadding"); cipher.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, bytes.copyOfRange(0, 12)))
    return String(cipher.doFinal(bytes.copyOfRange(12, bytes.size)), Charsets.UTF_8)
  }
  @Synchronized fun enqueue(files: List<Map<String, String>>) {
    require(state.optString("state") in listOf("idle", "verified")) { "Finish or clear the current queue first" }
    require(files.isNotEmpty() && state.optString("server").isNotBlank()) { "Connect your vault and select recordings" }
    token(); clear(); state.put("state", "copying"); save()
    try {
      for (file in files) {
        val name = file["name"] ?: "Recording.wav"
        val ext = name.substringAfterLast('.', "").lowercase()
        require(ext in listOf("wav", "flac", "mp3", "m4a")) { "Select WAV, FLAC, MP3, or M4A recordings" }
        val source = "${UUID.randomUUID()}.$ext"
        val uri = Uri.parse(file["uri"] ?: throw IOException("Missing recording"))
        context.contentResolver.openInputStream(uri).use { input ->
          requireNotNull(input) { "The recording is no longer available" }
          File(folder, source).outputStream().use { output -> input.copyTo(output); output.fd.sync() }
        }
        state.getJSONArray("files").put(JSONObject().put("id", UUID.randomUUID().toString()).put("name", name).put("source", source).put("state", "waiting").put("offset", 0).put("length", 0)); save()
      }
      state.put("state", "uploading"); save(); paused = false; AudioCodec.cancelled = false
    } catch (e: Exception) { state.put("state", "failed").put("error", e.message); save(); throw e }
  }
  @Synchronized fun pause() { paused = true; AudioCodec.cancelled = true; state.put("state", "paused"); save() }
  @Synchronized fun resume() { require(state.getJSONArray("files").length() > 0) { "Select recordings first" }; paused = false; AudioCodec.cancelled = false; state.put("state", "uploading").remove("error"); save() }
  @Synchronized fun clear() {
    require(!running.get() && state.optString("state") in listOf("idle", "verified", "paused", "failed")) { "Wait for the transfer to pause before clearing it" }
    folder.listFiles()?.forEach { it.delete() }
    state = JSONObject().put("state", "idle").put("server", state.optString("server")).put("timezone", state.optString("timezone")).put("files", JSONArray()); save()
  }
  fun run() {
    if (!running.compareAndSet(false, true)) return
    try {
      if (state.optString("state") !in listOf("uploading", "verifying")) return
      if (!state.has("importID")) { val response = json("/api/v1/imports", "POST", JSONObject().put("recorder_timezone", state.getString("timezone"))); synchronized(this) { state.put("importID", response.getString("id")); save() } }
      val files = state.getJSONArray("files")
      for (i in 0 until files.length()) {
        checkPaused(); val item = files.getJSONObject(i)
        if (item.optString("state") == "uploaded") continue
        if (!item.has("prepared")) {
          update(item, "state", "preparing")
          val source = File(folder, item.getString("source")); val wav = source.extension == "wav"
          val compress = wav && AudioCodec.canCompress(source.path)
          val prepared = File(folder, "${item.getString("id")}.${if (compress) "flac" else source.extension}")
          val partial = File(folder, "${prepared.name}.partial"); partial.delete()
          if (wav) { val error = AudioCodec.prepare(source.path, partial.path); if (error.isNotEmpty()) throw IOException(error) }
          else { source.inputStream().use { input -> partial.outputStream().use { output -> input.copyTo(output); output.fd.sync() } }; require(digest(source).contentEquals(digest(partial))) { "Staged recording failed verification" } }
          require(partial.renameTo(prepared)) { "Could not commit prepared audio" }
          synchronized(this) { item.put("prepared", prepared.name).put("length", prepared.length()).put("state", "ready"); save() }
        }
        if (!item.has("location")) {
          val values = mapOf("import_id" to state.getString("importID"), "filename" to item.getString("prepared"), "original_name" to item.getString("name"))
          val connection = request("/api/v1/uploads", "POST", mapOf("Tus-Resumable" to "1.0.0", "Upload-Length" to item.getLong("length").toString(), "Upload-Metadata" to values.map { "${it.key} ${Base64.encodeToString(it.value.toByteArray(), Base64.NO_WRAP)}" }.joinToString(",")))
          try { checked(connection); update(item, "location", connection.getHeaderField("Location") ?: throw IOException("Missing upload location")) } finally { connection.disconnect() }
        }
        while (true) {
          checkPaused(); val location = item.getString("location")
          val head = request(location, "HEAD", mapOf("Tus-Resumable" to "1.0.0"))
          val offset = try { checked(head); head.getHeaderField("Upload-Offset")?.toLong() ?: throw IOException("Missing upload offset") } finally { head.disconnect() }
          require(offset in 0..item.getLong("length")) { "Invalid upload offset" }; update(item, "offset", offset)
          if (offset == item.getLong("length")) { update(item, "state", "uploaded"); break }
          val count = minOf(8L * 1024 * 1024, item.getLong("length") - offset).toInt()
          val connection = request(location, "PATCH", mapOf("Tus-Resumable" to "1.0.0", "Upload-Offset" to offset.toString(), "Content-Type" to "application/offset+octet-stream"))
          connection.doOutput = true; connection.setFixedLengthStreamingMode(count)
          try {
            update(item, "state", "uploading")
            RandomAccessFile(File(folder, item.getString("prepared")), "r").use { input -> input.seek(offset); connection.outputStream.use { out -> val buffer = ByteArray(65536); var remaining = count; while (remaining > 0) { checkPaused(); val n = input.read(buffer, 0, minOf(buffer.size, remaining)); if (n < 0) throw IOException("Prepared audio is truncated"); out.write(buffer, 0, n); remaining -= n } } }
            checked(connection)
          } finally { connection.disconnect() }
        }
      }
      json("/api/v1/imports/${state.getString("importID")}/complete", "POST", JSONObject())
      synchronized(this) { state.put("state", "verifying"); save() }
      for (attempt in 0 until 120) {
        checkPaused(); val receipt = json("/api/v1/imports/${state.getString("importID")}")
        if (receipt.optString("state") == "complete") {
          val received = receipt.getJSONArray("files")
          require(received.length() > 0) { "Empty server receipt" }
          for (j in 0 until received.length()) { val file = received.getJSONObject(j); require(file.optString("state") in listOf("verified", "duplicate")) { file.optString("failure", "Server verification failed; local audio was kept") } }
          synchronized(this) { state.put("state", "verified"); for (j in 0 until files.length()) files.getJSONObject(j).put("state", "verified"); save() }; return
        }
        Thread.sleep(2000)
      }
      throw IOException("Server verification is still running. Resume to check the receipt.")
    } catch (e: Exception) { synchronized(this) { if (!paused) state.put("state", "failed").put("error", e.message ?: "Upload failed"); save() } }
    finally { running.set(false) }
  }
  @Synchronized private fun update(item: JSONObject, key: String, value: Any) { item.put(key, value); save() }
  private fun checkPaused() { if (paused) throw IOException("Paused") }
  private fun validateOrigin(value: String): URI { val uri = URI(value); require(uri.host != null && uri.userInfo == null && uri.query == null && uri.fragment == null && uri.path in listOf("", "/") && (uri.scheme == "https" || uri.scheme == "http" && uri.host in listOf("localhost", "127.0.0.1", "10.0.2.2"))) { "Use the external HTTPS origin of your vault" }; return uri }
  private fun request(path: String, method: String, headers: Map<String, String> = emptyMap()): HttpURLConnection {
    val origin = validateOrigin(state.getString("server")); val url = origin.resolve(path)
    require(url.scheme == origin.scheme && url.host == origin.host && url.port == origin.port) { "Upload URL must belong to your vault" }
    return (url.toURL().openConnection() as HttpURLConnection).apply { requestMethod = method; instanceFollowRedirects = false; connectTimeout = 30000; readTimeout = 120000; setRequestProperty("Authorization", "Bearer ${token()}"); headers.forEach { (k, v) -> setRequestProperty(k, v) } }
  }
  private fun checked(connection: HttpURLConnection) { if (connection.responseCode !in 200..299) { val message = connection.errorStream?.bufferedReader()?.use { it.readText().take(1000) }; throw IOException(try { JSONObject(message ?: "{}").optString("error", "Transfer failed (${connection.responseCode})") } catch (_: Exception) { "Transfer failed (${connection.responseCode})" }) } }
  private fun json(path: String, method: String = "GET", body: JSONObject? = null): JSONObject {
    val c = request(path, method)
    try { if (body != null) { c.doOutput = true; c.setRequestProperty("Content-Type", "application/json"); c.outputStream.use { it.write(body.toString().toByteArray()) } }; checked(c); return JSONObject(c.inputStream.bufferedReader().use { it.readText() }) } finally { c.disconnect() }
  }
  private fun digest(file: File): ByteArray { val hash = MessageDigest.getInstance("SHA-256"); file.inputStream().use { input -> val bytes = ByteArray(65536); while (true) { val n = input.read(bytes); if (n < 0) break; hash.update(bytes, 0, n) } }; return hash.digest() }
}
