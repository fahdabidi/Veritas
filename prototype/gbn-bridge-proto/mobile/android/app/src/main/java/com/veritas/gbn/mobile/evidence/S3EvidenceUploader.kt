package com.veritas.gbn.mobile.evidence

import com.veritas.gbn.mobile.model.EvidenceUploadConfig
import java.io.File
import java.net.HttpURLConnection
import java.net.URL

data class EvidenceUploadResult(
    val bucket: String,
    val objectKey: String,
    val etag: String?,
    val uploadedAtMs: Long,
    val localSha256: String,
)

object S3EvidenceUploader {
    fun uploadPresignedPut(config: EvidenceUploadConfig, zipFile: File): EvidenceUploadResult {
        val putUrl = config.presignedPutUrl
            ?.trim()
            ?.takeIf { it.isNotEmpty() }
            ?: throw IllegalArgumentException("S3 pre-signed PUT URL is required")
        require(putUrl.startsWith("https://") || putUrl.startsWith("http://")) {
            "S3 pre-signed PUT URL must use http or https"
        }
        val connection = (URL(putUrl).openConnection() as HttpURLConnection).apply {
            requestMethod = "PUT"
            doOutput = true
            setRequestProperty("Content-Type", "application/zip")
            setRequestProperty("Content-Length", zipFile.length().toString())
        }
        zipFile.inputStream().use { input ->
            connection.outputStream.use { output -> input.copyTo(output) }
        }
        val code = connection.responseCode
        require(code in 200..299) { "S3 upload failed with HTTP $code" }
        return EvidenceUploadResult(
            bucket = config.bucket,
            objectKey = config.objectKey,
            etag = connection.getHeaderField("ETag"),
            uploadedAtMs = System.currentTimeMillis(),
            localSha256 = EvidenceBundleWriter.sha256(zipFile),
        )
    }
}
