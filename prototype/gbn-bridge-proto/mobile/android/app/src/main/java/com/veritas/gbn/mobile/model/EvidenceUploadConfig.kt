package com.veritas.gbn.mobile.model

data class EvidenceUploadConfig(
    val uploadMode: String,
    val bucket: String,
    val objectKey: String,
    val presignedPutUrl: String?,
    val expiresAtMs: Long?,
    val expectedSha256: String?,
) {
    companion object {
        fun parse(rawJson: String, nowMs: Long = System.currentTimeMillis()): EvidenceUploadConfig {
            val mode = JsonText.stringField(rawJson, "upload_mode")
                ?: throw IllegalArgumentException("upload config requires upload_mode")
            require(mode == "s3_presigned_put") { "unsupported upload_mode `$mode`" }
            val bucket = JsonText.stringField(rawJson, "bucket")
                ?: throw IllegalArgumentException("upload config requires bucket")
            val objectKey = JsonText.stringField(rawJson, "object_key")
                ?: throw IllegalArgumentException("upload config requires object_key")
            require(objectKey.startsWith("mobile-evidence/")) {
                "object_key must stay under mobile-evidence/"
            }
            require(!JsonText.hasField(rawJson, "aws_secret_access_key")) {
                "long-lived AWS secret keys are not allowed in the app"
            }
            require(!JsonText.hasField(rawJson, "aws_access_key_id")) {
                "long-lived AWS access keys are not allowed in the app"
            }
            require(!JsonText.hasField(rawJson, "aws_session_token")) {
                "raw AWS session tokens are not allowed outside the pre-signed URL"
            }
            require(!JsonText.hasField(rawJson, "AWS_SECRET_ACCESS_KEY")) {
                "long-lived AWS secret keys are not allowed in the app"
            }
            require(!JsonText.hasField(rawJson, "AWS_ACCESS_KEY_ID")) {
                "long-lived AWS access keys are not allowed in the app"
            }
            require(!JsonText.hasField(rawJson, "AWS_SESSION_TOKEN")) {
                "raw AWS session tokens are not allowed outside the pre-signed URL"
            }
            val expiresAtMs = JsonText.longField(rawJson, "expires_at_ms")
            if (expiresAtMs != null && expiresAtMs != 0L) {
                require(expiresAtMs > nowMs) { "S3 pre-signed PUT grant is expired" }
            }
            return EvidenceUploadConfig(
                uploadMode = mode,
                bucket = bucket,
                objectKey = objectKey,
                presignedPutUrl = JsonText.stringField(rawJson, "presigned_put_url"),
                expiresAtMs = expiresAtMs,
                expectedSha256 = JsonText.stringField(rawJson, "expected_sha256"),
            )
        }
    }
}
