package com.veritas.gbn.mobile

import com.veritas.gbn.mobile.model.EvidenceUploadConfig
import com.veritas.gbn.mobile.evidence.S3EvidenceUploader
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class EvidenceUploadConfigTest {
    @Test
    fun acceptsPresignedS3PutGrant() {
        val config = EvidenceUploadConfig.parse(
            """
            {
              "upload_mode": "s3_presigned_put",
              "bucket": "veritas-pass4-mobile-evidence",
              "object_key": "mobile-evidence/run/chain/bundle.zip",
              "presigned_put_url": "https://s3.example.test/upload",
              "expires_at_ms": 2000
            }
            """.trimIndent(),
        )
        assertEquals("veritas-pass4-mobile-evidence", config.bucket)
    }

    @Test
    fun rejectsLongLivedAwsCredentials() {
        val result = runCatching {
            EvidenceUploadConfig.parse(
                """
                {
                  "upload_mode": "s3_presigned_put",
                  "bucket": "veritas-pass4-mobile-evidence",
                  "object_key": "mobile-evidence/run/chain/bundle.zip",
                  "aws_secret_access_key": "not-allowed"
                }
                """.trimIndent(),
            )
        }
        assertTrue(result.isFailure)
    }

    @Test
    fun blankPresignedUrlReturnsOperatorMessage() {
        val config = EvidenceUploadConfig(
            uploadMode = "s3_presigned_put",
            bucket = "veritas-pass4-mobile-evidence",
            objectKey = "mobile-evidence/run/chain/bundle.zip",
            presignedPutUrl = "",
            expiresAtMs = null,
            expectedSha256 = null,
        )
        val zip = File.createTempFile("mobile-evidence", ".zip").apply {
            writeText("placeholder")
            deleteOnExit()
        }

        val result = runCatching { S3EvidenceUploader.uploadPresignedPut(config, zip) }

        assertTrue(result.isFailure)
        assertEquals("S3 pre-signed PUT URL is required", result.exceptionOrNull()?.message)
    }
}
