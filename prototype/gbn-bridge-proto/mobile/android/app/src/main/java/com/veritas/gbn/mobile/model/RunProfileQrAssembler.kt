package com.veritas.gbn.mobile.model

import java.security.MessageDigest
import java.util.Base64

data class RunProfileQrImportResult(
    val complete: Boolean,
    val profileJson: String?,
    val message: String,
)

class RunProfileQrAssembler {
    private var profileId: String? = null
    private var expectedCount: Int = 0
    private var expectedSha256: String? = null
    private val chunks = sortedMapOf<Int, String>()

    fun accept(payload: String): RunProfileQrImportResult {
        val trimmed = payload.trim()
        if (trimmed.startsWith("{") && JsonText.stringField(trimmed, "profile") != null) {
            val config = RunProfileConfig.parse(trimmed)
            return RunProfileQrImportResult(
                complete = true,
                profileJson = trimmed,
                message = "Imported run profile ${config.runId}; profile=${config.profile}",
            )
        }
        require(JsonText.stringField(trimmed, "schema") == "veritas.pass4.run_profile_qr.v1") {
            "QR payload is not a run profile or run-profile chunk"
        }

        val chunkProfileId = JsonText.stringField(trimmed, "profile_id")
            ?: throw IllegalArgumentException("run profile chunk requires profile_id")
        val chunkIndex = JsonText.longField(trimmed, "index")?.toInt()
            ?: throw IllegalArgumentException("run profile chunk requires index")
        val chunkCount = JsonText.longField(trimmed, "count")?.toInt()
            ?: throw IllegalArgumentException("run profile chunk requires count")
        val chunkSha = JsonText.stringField(trimmed, "sha256")
            ?: throw IllegalArgumentException("run profile chunk requires sha256")
        val data = JsonText.stringField(trimmed, "data")
            ?: throw IllegalArgumentException("run profile chunk requires data")
        val decodedData = if (JsonText.stringField(trimmed, "encoding") == "base64") {
            String(Base64.getDecoder().decode(data), Charsets.UTF_8)
        } else {
            data
        }
        require(chunkIndex in 1..chunkCount) { "run profile chunk index is out of range" }

        if (profileId == null) {
            profileId = chunkProfileId
            expectedCount = chunkCount
            expectedSha256 = chunkSha
        } else {
            require(profileId == chunkProfileId) { "run profile chunk profile_id mismatch" }
            require(expectedCount == chunkCount) { "run profile chunk count mismatch" }
            require(expectedSha256 == chunkSha) { "run profile chunk sha256 mismatch" }
        }

        chunks[chunkIndex]?.let { existing ->
            require(existing == decodedData) { "run profile duplicate chunk payload mismatch" }
        }
        chunks[chunkIndex] = decodedData
        if (chunks.size < expectedCount) {
            return RunProfileQrImportResult(
                complete = false,
                profileJson = null,
                message = "Imported run profile QR chunk ${chunks.size}/$expectedCount",
            )
        }

        val reconstructed = (1..expectedCount).joinToString(separator = "") { index ->
            chunks[index] ?: throw IllegalArgumentException("run profile chunk $index missing")
        }
        require(sha256(reconstructed) == expectedSha256) {
            "run profile reconstructed SHA-256 mismatch"
        }
        val config = RunProfileConfig.parse(reconstructed)
        clear()
        return RunProfileQrImportResult(
            complete = true,
            profileJson = reconstructed,
            message = "Imported run profile ${config.runId}; profile=${config.profile}",
        )
    }

    fun clear() {
        profileId = null
        expectedCount = 0
        expectedSha256 = null
        chunks.clear()
    }

    private fun sha256(value: String): String =
        MessageDigest.getInstance("SHA-256")
            .digest(value.toByteArray(Charsets.UTF_8))
            .joinToString(separator = "") { "%02x".format(it) }
}
