package com.veritas.gbn.mobile.model

data class CreatorAction(
    val buttonId: String,
    val relayAction: String,
    val group: String,
    val phase3Enabled: Boolean,
    val disabledReason: String,
)

object CreatorActionCatalog {
    val actions = listOf(
        CreatorAction("RefreshStatus", "Status", "Runtime", true, ""),
        CreatorAction("RefreshState", "Refresh", "Runtime", true, ""),
        CreatorAction("RefreshBridgeCatalog", "ShowCatalog", "DHT/Catalog", false, "requires Publisher-signed catalog learned after bootstrap"),
        CreatorAction("DumpLocalDht", "DumpLocalDht", "DHT/Catalog", true, ""),
        CreatorAction("DumpNodeState", "DumpNodeDht", "DHT/Catalog", true, ""),
        CreatorAction("RuntimeMetrics", "AdminMetrics", "Runtime", true, ""),
        CreatorAction("HostCreatorDHTQRReader", "BootstrapDHTQRCode", "Bootstrap", true, ""),
        CreatorAction("SeedHostCreator", "SeedHostCreator", "Bootstrap", false, "future phase HostCreator mode"),
        CreatorAction("BootstrapNewCreator", "SeedNewCreator", "Bootstrap", false, "enabled in Phase 5 public HostCreator path"),
        CreatorAction("BuildUploadSession", "BuildUploadSession", "Upload", true, ""),
        CreatorAction("SendDummy", "SendDummy", "Upload", false, "enabled in Phase 5 after mobile bootstrap"),
        CreatorAction("SendUpload", "SendUpload", "Upload", false, "enabled in Phase 5 after mobile bootstrap"),
        CreatorAction("SessionFrameSummary", "DumpFrames", "Upload", true, ""),
        CreatorAction("ExportEvidence", "CollectTraces", "Evidence", true, ""),
        CreatorAction("UploadEvidenceToS3", "CollectTraces", "Evidence", true, ""),
        CreatorAction("ResetCreatorState", "ResetCreatorState", "Maintenance", true, ""),
    )

    val requiredButtonIds: Set<String> = actions.mapTo(sortedSetOf()) { it.buttonId }

    fun eventJson(action: CreatorAction, chainId: String): String =
        """
        {
          "button_id": "${action.buttonId}",
          "relay_action": "${action.relayAction}",
          "operation": "${action.buttonId}",
          "chain_id": "$chainId"
        }
        """.trimIndent()
}
