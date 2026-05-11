package com.veritas.gbn.mobile.service

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.IBinder

class CreatorForegroundService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val chainId = intent?.getStringExtra(EXTRA_CHAIN_ID) ?: "phase3-runtime"
        val operation = intent?.getStringExtra(EXTRA_OPERATION) ?: "MobileCreatorRuntime"
        ensureChannel()
        startForeground(
            NOTIFICATION_ID,
            Notification.Builder(this, CHANNEL_ID)
                .setContentTitle("GBN Mobile Creator")
                .setContentText("$operation: $chainId")
                .setSmallIcon(android.R.drawable.stat_sys_upload)
                .setOngoing(true)
                .build(),
        )
        return START_STICKY
    }

    private fun ensureChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val manager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
            manager.createNotificationChannel(
                NotificationChannel(
                    CHANNEL_ID,
                    "GBN Creator Validation",
                    NotificationManager.IMPORTANCE_LOW,
                ),
            )
        }
    }

    companion object {
        private const val CHANNEL_ID = "gbn_creator_validation"
        private const val NOTIFICATION_ID = 13013
        const val EXTRA_CHAIN_ID = "chain_id"
        const val EXTRA_OPERATION = "operation"
    }
}
