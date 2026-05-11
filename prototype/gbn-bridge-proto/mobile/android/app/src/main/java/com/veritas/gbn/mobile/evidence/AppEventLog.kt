package com.veritas.gbn.mobile.evidence

import com.veritas.gbn.mobile.model.CreatorAction
import com.veritas.gbn.mobile.model.CreatorActionCatalog
import com.veritas.gbn.mobile.model.JsonText
import java.io.File

class AppEventLog(private val file: File) {
    init {
        file.parentFile?.mkdirs()
    }

    fun appendAction(action: CreatorAction, chainId: String, state: String) {
        appendRaw(
            """
            {
              "timestamp_ms": ${System.currentTimeMillis()},
              "type": "button_action",
              "state": ${JsonText.quote(state)},
              "action": ${CreatorActionCatalog.eventJson(action, chainId)}
            }
            """.trimIndent(),
        )
    }

    fun appendEvent(event: String, chainId: String, details: String) {
        appendRaw(
            """
            {
              "timestamp_ms": ${System.currentTimeMillis()},
              "type": ${JsonText.quote(event)},
              "chain_id": ${JsonText.quote(chainId)},
              "details": ${JsonText.quote(details)}
            }
            """.trimIndent(),
        )
    }

    fun readText(): String = if (file.exists()) file.readText() else ""

    private fun appendRaw(json: String) {
        file.appendText(json.replace("\n", "") + "\n")
    }
}
