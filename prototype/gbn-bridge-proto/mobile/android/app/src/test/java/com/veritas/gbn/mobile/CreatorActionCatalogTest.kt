package com.veritas.gbn.mobile

import com.veritas.gbn.mobile.model.CreatorActionCatalog
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class CreatorActionCatalogTest {
    @Test
    fun includesRequiredRelayControlButtons() {
        val ids = CreatorActionCatalog.requiredButtonIds
        listOf(
            "RefreshStatus",
            "RefreshState",
            "RefreshBridgeCatalog",
            "DumpLocalDht",
            "DumpNodeState",
            "RuntimeMetrics",
            "HostCreatorDHTQRReader",
            "BootstrapNewCreator",
            "BuildUploadSession",
            "SendDummy",
            "SendUpload",
            "SessionFrameSummary",
            "ExportEvidence",
            "UploadEvidenceToS3",
            "ResetCreatorState",
        ).forEach { assertTrue("missing $it", ids.contains(it)) }
    }

    @Test
    fun futureNetworkActionsRemainDisabledInPhase3() {
        val actions = CreatorActionCatalog.actions.associateBy { it.buttonId }
        assertFalse(actions.getValue("BootstrapNewCreator").phase3Enabled)
        assertFalse(actions.getValue("SendDummy").phase3Enabled)
        assertFalse(actions.getValue("SendUpload").phase3Enabled)
    }
}
