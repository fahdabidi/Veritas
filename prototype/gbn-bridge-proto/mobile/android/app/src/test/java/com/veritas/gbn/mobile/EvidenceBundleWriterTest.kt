package com.veritas.gbn.mobile

import com.veritas.gbn.mobile.evidence.EvidenceBundleWriter
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File
import java.util.zip.ZipFile

class EvidenceBundleWriterTest {
    @Test
    fun writesRequiredEvidenceFilesAndZip() {
        val root = File(System.getProperty("java.io.tmpdir"), "gbn-mobile-evidence-test-${System.nanoTime()}")
        val result = EvidenceBundleWriter.writeBundle(
            rootDir = root,
            bundleId = "bundle",
            files = mapOf(
                "events.jsonl" to """{"event":"button"}""" + "\n",
                "local_dht.json" to """{"role":"creator"}""",
                "node_metadata.json" to """{"role":"creator"}""",
            ),
        )
        assertTrue(result.zipFile.exists())
        ZipFile(result.zipFile).use { zip ->
            EvidenceBundleWriter.requiredFiles.forEach { required ->
                assertTrue("missing $required", zip.getEntry(required) != null)
            }
        }
    }
}
