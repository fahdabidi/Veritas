package com.veritas.gbn.mobile

import com.veritas.gbn.mobile.model.HostSeedGuard
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class HostSeedGuardTest {
    @Test
    fun previewsMobileReachableHostSeed() {
        val preview = HostSeedGuard.preview(validSeed(), nowMs = 1_000)
        assertEquals("host-creator", preview.hostCreatorId)
        assertEquals("host-creator.example.test", preview.host)
        assertEquals("pass4-seed", preview.chainId)
    }

    @Test
    fun rejectsPublisherShortcutAndPrivateAdminEndpoint() {
        val publisherShortcut = validSeed().replace(
            """"signature": "sig"""",
            """"signature": "sig", "publisher_entry": {}""",
        )
        assertTrue(runCatching { HostSeedGuard.preview(publisherShortcut, nowMs = 1_000) }.isFailure)

        val privateEndpoint = validSeed().replace("host-creator.example.test", "127.0.0.1/admin/bootstrap")
        assertTrue(runCatching { HostSeedGuard.preview(privateEndpoint, nowMs = 1_000) }.isFailure)
    }

    private fun validSeed(): String =
        """
        {
          "chain_id": "pass4-seed",
          "host_creator_id": "host-creator",
          "host_creator_public_key_hex": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
          "host_creator_bootstrap_endpoints": [
            {"host": "host-creator.example.test", "port": 443}
          ],
          "expires_at_ms": 2000,
          "signature": "sig"
        }
        """.trimIndent()
}
