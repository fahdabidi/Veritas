package com.veritas.gbn.mobile

import com.veritas.gbn.mobile.model.RunProfileConfig
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class RunProfileConfigTest {
    @Test
    fun acceptsLocalAndHybridProfilesWithoutPublisherTrustRoot() {
        val local = RunProfileConfig.parse(
            """
            {
              "profile": "local_k8s_public",
              "run_id": "pass4-local",
              "evidence_bucket": "veritas-pass4-mobile-evidence",
              "evidence_prefix": "mobile-evidence/pass4-local/"
            }
            """.trimIndent(),
        )
        assertEquals("local_k8s_public", local.profile)

        val hybrid = RunProfileConfig.parse(
            """
            {
              "profile": "hybrid_local_publisher_aws_bridges",
              "run_id": "pass4-hybrid",
              "aws_exitbridge_region": "ca-central-1"
            }
            """.trimIndent(),
        )
        assertEquals("ca-central-1", hybrid.awsExitBridgeRegion)

        val aws = RunProfileConfig.parse(
            """
            {
              "profile": "aws_public",
              "run_id": "pass4-aws",
              "evidence_bucket": "veritas-pass4-mobile-evidence",
              "evidence_prefix": "mobile-evidence/pass4-aws/",
              "aws_exitbridge_region": "ca-central-1",
              "endpoints": []
            }
            """.trimIndent(),
        )
        assertEquals("aws_public", aws.profile)
        assertEquals("pass4-aws", aws.runId)
    }

    @Test
    fun rejectsFirstTimeBootstrapPreloadFields() {
        val result = runCatching {
            RunProfileConfig.parse(
                """
                {
                  "profile": "local_k8s_public",
                  "publisher_public_key_hex": "bad-preload"
                }
                """.trimIndent(),
            )
        }
        assertTrue(result.isFailure)
    }
}
