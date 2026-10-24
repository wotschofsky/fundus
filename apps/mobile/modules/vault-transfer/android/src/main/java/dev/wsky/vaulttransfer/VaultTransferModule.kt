package dev.wsky.vaulttransfer

import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition
import android.content.Intent
import android.os.Build

class VaultTransferModule : Module() {
  private fun context() = appContext.reactContext ?: throw IllegalStateException("App is not ready")
  override fun definition() = ModuleDefinition {
    Name("VaultTransfer")
    AsyncFunction("status") { UploadQueue.get(context()).status() }
    AsyncFunction("configure") { server: String, token: String, timezone: String -> UploadQueue.get(context()).configure(server, token, timezone) }
    AsyncFunction("enqueue") { files: List<Map<String, String>> ->
      UploadQueue.get(context()).enqueue(files)
      start()
    }
    AsyncFunction("resume") { UploadQueue.get(context()).resume(); start() }
    AsyncFunction("pause") { UploadQueue.get(context()).pause(); context().stopService(Intent(context(), UploadService::class.java)) }
    AsyncFunction("clear") { UploadQueue.get(context()).clear() }
  }
  private fun start() {
    val intent = Intent(context(), UploadService::class.java)
    if (Build.VERSION.SDK_INT >= 26) context().startForegroundService(intent) else context().startService(intent)
  }
}
