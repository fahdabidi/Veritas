package com.veritas.gbn.mobile.evidence

import java.io.File
import java.io.FileInputStream
import java.io.FileOutputStream
import java.security.MessageDigest
import java.util.zip.ZipEntry
import java.util.zip.ZipOutputStream

data class EvidenceBundleResult(
    val bundleId: String,
    val bundleDir: File,
    val zipFile: File,
    val zipSha256: String,
    val fileCount: Int,
)

object EvidenceBundleWriter {
    val requiredFiles = listOf(
        "evidence.json",
        "events.jsonl",
        "trace_events.jsonl",
        "local_dht.json",
        "node_metadata.json",
        "host_creator_seed.redacted.json",
        "upload_sessions.json",
        "endpoint_config.redacted.json",
        "device_context.json",
        "network_context.json",
        "app_build.json",
        "rust_build.json",
        "chain_ids.txt",
        "remote_trace_queries.json",
        "manifest.sha256.json",
    )

    fun writeBundle(rootDir: File, bundleId: String, files: Map<String, String>): EvidenceBundleResult {
        val bundleDir = File(rootDir, bundleId)
        if (bundleDir.exists()) {
            bundleDir.deleteRecursively()
        }
        bundleDir.mkdirs()
        val completeFiles = requiredFiles.associateWith { files[it] ?: defaultContent(it, bundleId) } + files
        val manifest = linkedMapOf<String, String>()
        completeFiles.forEach { (name, content) ->
            val file = File(bundleDir, name)
            file.parentFile?.mkdirs()
            file.writeText(content)
            if (name != "manifest.sha256.json") {
                manifest[name] = sha256(file)
            }
        }
        File(bundleDir, "manifest.sha256.json").writeText(
            manifest.entries.joinToString(prefix = "{\n", postfix = "\n}") { (path, hash) ->
                """  "$path": "$hash""""
            },
        )
        val zipFile = File(rootDir, "$bundleId.zip")
        if (zipFile.exists()) {
            zipFile.delete()
        }
        zip(bundleDir, zipFile)
        return EvidenceBundleResult(
            bundleId = bundleId,
            bundleDir = bundleDir,
            zipFile = zipFile,
            zipSha256 = sha256(zipFile),
            fileCount = completeFiles.size,
        )
    }

    fun sha256(file: File): String {
        val digest = MessageDigest.getInstance("SHA-256")
        FileInputStream(file).use { input ->
            val buffer = ByteArray(DEFAULT_BUFFER_SIZE)
            while (true) {
                val read = input.read(buffer)
                if (read <= 0) break
                digest.update(buffer, 0, read)
            }
        }
        return digest.digest().joinToString("") { "%02x".format(it) }
    }

    private fun defaultContent(name: String, bundleId: String): String =
        when (name) {
            "events.jsonl", "trace_events.jsonl", "chain_ids.txt" -> ""
            "evidence.json" -> """{"bundle_id":"$bundleId","phase":"pass4-phase3"}"""
            else -> "{}"
        }

    private fun zip(sourceDir: File, zipFile: File) {
        ZipOutputStream(FileOutputStream(zipFile)).use { output ->
            sourceDir.walkTopDown()
                .filter { it.isFile }
                .forEach { file ->
                    val relative = sourceDir.toPath().relativize(file.toPath()).toString().replace('\\', '/')
                    output.putNextEntry(ZipEntry(relative))
                    file.inputStream().use { it.copyTo(output) }
                    output.closeEntry()
                }
        }
    }
}
