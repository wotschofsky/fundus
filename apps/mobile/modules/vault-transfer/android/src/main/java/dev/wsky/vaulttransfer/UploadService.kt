package dev.wsky.vaulttransfer

import android.app.*
import android.content.Intent
import android.os.IBinder
import java.util.concurrent.Executors

class UploadService : Service() {
  private val executor = Executors.newSingleThreadExecutor()
  override fun onBind(intent: Intent?): IBinder? = null
  override fun onCreate() {
    super.onCreate()
    val manager = getSystemService(NotificationManager::class.java)
    manager.createNotificationChannel(NotificationChannel("uploads", "Recording uploads", NotificationManager.IMPORTANCE_LOW))
    startForeground(1001, Notification.Builder(this, "uploads").setContentTitle("Uploading recordings").setContentText("Preparing and sending audio to your vault").setSmallIcon(android.R.drawable.stat_sys_upload).setOngoing(true).build())
  }
  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    executor.execute { UploadQueue.get(this).run(); stopForeground(STOP_FOREGROUND_REMOVE); stopSelf(startId) }
    return START_STICKY
  }
  override fun onTimeout(startId: Int, fgsType: Int) { UploadQueue.get(this).pause(); stopSelf() }
  override fun onDestroy() { executor.shutdown(); super.onDestroy() }
}
