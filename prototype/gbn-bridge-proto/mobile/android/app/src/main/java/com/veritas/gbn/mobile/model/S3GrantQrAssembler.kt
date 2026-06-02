package com.veritas.gbn.mobile.model

import java.security.MessageDigest
import java.util.Base64

data class S3GrantQrImportResult(
    val complete: Boolean,
    val message: String,
    val grantJson: String? = null,
)

class S3GrantQrAssembler {
    private var grantId: String? = null
    private var expectedCount: Int = 0
    private var expectedSha256: String? = null
    private val chunks = linkedMapOf<Int, String>()

    fun accept(payload: String, nowMs: Long = System.currentTimeMillis()): S3GrantQrImportResult {
        val trimmed = payload.trim()
        if (JsonText.stringField(trimmed, "upload_mode") == "s3_presigned_put") {
            val config = EvidenceUploadConfig.parse(trimmed, nowMs)
            return S3GrantQrImportResult(
                complete = true,
                message = "Imported S3 grant object_key=${config.objectKey} expires_at_ms=${config.expiresAtMs ?: 0}",
                grantJson = trimmed,
            )
        }

        require(JsonText.stringField(trimmed, "type") == "gbn.s3_grant.chunk") {
            "QR payload is not an S3 grant chunk"
        }
        val chunkGrantId = JsonText.stringField(trimmed, "grant_id")
            ?: throw IllegalArgumentException("S3 grant chunk requires grant_id")
        val chunkIndex = JsonText.longField(trimmed, "index")?.toInt()
            ?: throw IllegalArgumentException("S3 grant chunk requires index")
        val chunkCount = JsonText.longField(trimmed, "count")?.toInt()
            ?: throw IllegalArgumentException("S3 grant chunk requires count")
        val chunkSha = JsonText.stringField(trimmed, "sha256")
            ?: throw IllegalArgumentException("S3 grant chunk requires sha256")
        val encodedData = JsonText.stringField(trimmed, "data")
            ?: throw IllegalArgumentException("S3 grant chunk requires data")
        require(chunkIndex in 1..chunkCount) { "S3 grant chunk index is out of range" }

        if (grantId != null && !matchesActiveGrant(chunkGrantId, chunkCount, chunkSha)) {
            clear()
        }
        if (grantId == null) {
            grantId = chunkGrantId
            expectedCount = chunkCount
            expectedSha256 = chunkSha
        }

        val paddedData = encodedData + "=".repeat((4 - encodedData.length % 4) % 4)
        val data = String(Base64.getUrlDecoder().decode(paddedData), Charsets.UTF_8)
        chunks[chunkIndex]?.let { existing ->
            require(existing == data) { "S3 grant duplicate chunk payload mismatch" }
        }
        chunks[chunkIndex] = data

        if (chunks.size < expectedCount) {
            return S3GrantQrImportResult(
                complete = false,
                message = "Imported S3 grant QR chunk ${chunks.size}/$expectedCount",
            )
        }

        val grantJson = (1..expectedCount).joinToString(separator = "") { index ->
            chunks[index] ?: throw IllegalArgumentException("S3 grant chunk $index missing")
        }
        val actualSha = sha256(grantJson)
        if (actualSha != expectedSha256) {
            clear()
            throw IllegalArgumentException("S3 grant reconstructed SHA-256 mismatch")
        }
        try {
            val config = EvidenceUploadConfig.parse(grantJson, nowMs)
            return S3GrantQrImportResult(
                complete = true,
                message = "Imported S3 grant object_key=${config.objectKey} expires_at_ms=${config.expiresAtMs ?: 0}",
                grantJson = grantJson,
            )
        } finally {
            clear()
        }
    }

    fun clear() {
        grantId = null
        expectedCount = 0
        expectedSha256 = null
        chunks.clear()
    }

    private fun sha256(value: String): String {
        val digest = MessageDigest.getInstance("SHA-256").digest(value.toByteArray(Charsets.UTF_8))
        return digest.joinToString("") { "%02x".format(it) }
    }

    private fun matchesActiveGrant(chunkGrantId: String, chunkCount: Int, chunkSha: String): Boolean =
        grantId == chunkGrantId && expectedCount == chunkCount && expectedSha256 == chunkSha
}
