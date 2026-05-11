package com.veritas.gbn.mobile.runtime

import com.veritas.gbn.mobile.model.JsonText
import java.io.File

object RuntimeConfigFactory {
    fun build(
        stateDir: File,
        evidenceDir: File,
        creatorId: String,
        networkProfile: String,
        endpointConfigJson: String?,
    ): String =
        """
        {
          "state_dir": ${JsonText.quote(stateDir.absolutePath)},
          "app_root_dir": ${JsonText.quote(stateDir.parentFile?.absolutePath ?: stateDir.absolutePath)},
          "creator_id": ${JsonText.quote(creatorId)},
          "network_profile": ${JsonText.quote(networkProfile)},
          "endpoint_config_json": ${endpointConfigJson?.let(JsonText::quote) ?: "null"},
          "log_level": "debug",
          "evidence_dir": ${JsonText.quote(evidenceDir.absolutePath)}
        }
        """.trimIndent()
}
