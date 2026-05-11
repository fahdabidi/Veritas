package com.veritas.gbn.mobile.evidence

import android.content.Context
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.os.Build
import com.veritas.gbn.mobile.BuildConfig
import com.veritas.gbn.mobile.model.JsonText

object DeviceContext {
    fun deviceJson(context: Context): String =
        """
        {
          "manufacturer": ${JsonText.quote(Build.MANUFACTURER ?: "unknown")},
          "model": ${JsonText.quote(Build.MODEL ?: "unknown")},
          "android_sdk": ${Build.VERSION.SDK_INT},
          "abi": ${JsonText.quote(Build.SUPPORTED_ABIS.firstOrNull() ?: "unknown")},
          "app_version": ${JsonText.quote(BuildConfig.VERSION_NAME)},
          "install_source": ${JsonText.quote(context.packageManager.getInstallerPackageName(context.packageName) ?: "unknown")}
        }
        """.trimIndent()

    fun networkJson(context: Context, note: String): String {
        val manager = context.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
        val capabilities = manager.getNetworkCapabilities(manager.activeNetwork)
        val transport = when {
            capabilities == null -> "unknown"
            capabilities.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR) -> "cellular"
            capabilities.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) -> "wifi"
            capabilities.hasTransport(NetworkCapabilities.TRANSPORT_VPN) -> "vpn"
            else -> "unknown"
        }
        return """
        {
          "active_transport_type": ${JsonText.quote(transport)},
          "roaming": false,
          "carrier_name": "unknown",
          "public_validation_note": ${JsonText.quote(note)},
          "timestamp_ms": ${System.currentTimeMillis()}
        }
        """.trimIndent()
    }

    fun appBuildJson(): String =
        """
        {
          "application_id": ${JsonText.quote(BuildConfig.APPLICATION_ID)},
          "version_name": ${JsonText.quote(BuildConfig.VERSION_NAME)},
          "version_code": ${BuildConfig.VERSION_CODE},
          "build_type": ${JsonText.quote(BuildConfig.BUILD_TYPE)}
        }
        """.trimIndent()

    fun rustBuildJson(): String =
        """
        {
          "library": "gbn_bridge_mobile_ffi",
          "loaded_by": "System.loadLibrary",
          "phase": "pass4-phase3"
        }
        """.trimIndent()
}
