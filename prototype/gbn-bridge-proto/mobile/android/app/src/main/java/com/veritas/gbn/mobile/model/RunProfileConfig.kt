package com.veritas.gbn.mobile.model

data class RunProfileConfig(
    val profile: String,
    val runId: String,
    val evidenceBucket: String?,
    val evidencePrefix: String?,
    val awsExitBridgeRegion: String?,
    val notes: String?,
    val rawJson: String,
) {
    val isOfflineWarning: Boolean = profile == PROFILE_OFFLINE_TEST

    companion object {
        const val PROFILE_OFFLINE_TEST = "offline_test"
        const val PROFILE_LOCAL_K8S_PUBLIC = "local_k8s_public"
        const val PROFILE_HYBRID = "hybrid_local_publisher_aws_bridges"

        private val allowedProfiles = setOf(PROFILE_OFFLINE_TEST, PROFILE_LOCAL_K8S_PUBLIC, PROFILE_HYBRID)
        private val forbiddenBootstrapFields = listOf(
            "publisher_entry",
            "publisher_dht",
            "publisher_public_key_hex",
            "seed_exit_bridge",
            "seed_exitbridge",
            "seed_exit_bridge_dht",
            "bridge_catalog",
            "bridge_entries",
            "exitbridge_entries",
        )

        fun parse(rawJson: String): RunProfileConfig {
            val profile = JsonText.stringField(rawJson, "profile")
                ?: throw IllegalArgumentException("run profile requires profile")
            require(profile in allowedProfiles) { "unsupported run profile `$profile`" }
            forbiddenBootstrapFields.firstOrNull { JsonText.hasField(rawJson, it) }?.let { field ->
                throw IllegalArgumentException("run profile must not preload first-time bootstrap field `$field`")
            }
            return RunProfileConfig(
                profile = profile,
                runId = JsonText.stringField(rawJson, "run_id") ?: "pass4-mobile-local",
                evidenceBucket = JsonText.stringField(rawJson, "evidence_bucket"),
                evidencePrefix = JsonText.stringField(rawJson, "evidence_prefix"),
                awsExitBridgeRegion = JsonText.stringField(rawJson, "aws_exitbridge_region"),
                notes = JsonText.stringField(rawJson, "notes"),
                rawJson = rawJson,
            )
        }

        fun default(profile: String): RunProfileConfig =
            parse(
                """
                {
                  "profile": "$profile",
                  "run_id": "pass4-phase3-emulator",
                  "evidence_prefix": "mobile-evidence/pass4-phase3-emulator/",
                  "notes": "Phase 3 emulator run; Publisher and bridge DHT arrive only through bootstrap payload in later phases"
                }
                """.trimIndent(),
            )
    }
}
