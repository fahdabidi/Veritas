package com.veritas.gbn.mobile

import com.veritas.gbn.mobile.model.S3GrantQrAssembler
import java.security.MessageDigest
import java.util.Base64
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class S3GrantQrAssemblerTest {
    @Test
    fun reconstructsOutOfOrderChunks() {
        val grant = grantJson(expiresAtMs = 3000)
        val chunks = chunks(grant, size = grant.length / 2)
        val assembler = S3GrantQrAssembler()

        val second = assembler.accept(chunks[1], nowMs = 1000)
        assertFalse(second.complete)
        val first = assembler.accept(chunks[0], nowMs = 1000)

        assertTrue(first.complete)
        assertEquals(grant, first.grantJson)
    }

    @Test
    fun rejectsHashMismatchAndExpiredGrant() {
        val grant = grantJson(expiresAtMs = 3000)
        val badChunk = chunks(grant, size = grant.length, sha = "0".repeat(64)).single()
        assertTrue(runCatching { S3GrantQrAssembler().accept(badChunk, nowMs = 1000) }.isFailure)

        val expired = grantJson(expiresAtMs = 900)
        val expiredChunk = chunks(expired, size = expired.length).single()
        assertTrue(runCatching { S3GrantQrAssembler().accept(expiredChunk, nowMs = 1000) }.isFailure)
    }

    @Test
    fun clearsExpiredChunkAssemblyBeforeNextGrant() {
        val expired = grantJson(expiresAtMs = 900)
        val fresh = grantJson(expiresAtMs = 3000)
        val freshChunks = chunks(fresh, size = twoChunkSize(fresh))
        val assembler = S3GrantQrAssembler()

        assertTrue(
            runCatching {
                assembler.accept(chunks(expired, size = expired.length).single(), nowMs = 1000)
            }.isFailure,
        )

        assertFalse(assembler.accept(freshChunks[0], nowMs = 1000).complete)
        val imported = assembler.accept(freshChunks[1], nowMs = 1000)
        assertTrue(imported.complete)
        assertEquals(fresh, imported.grantJson)
    }

    @Test
    fun startsNewAssemblyWhenGrantIdentityChanges() {
        val oldGrant = grantJson(expiresAtMs = 3000)
        val freshGrant = grantJson(expiresAtMs = 4000)
        val oldChunks = chunks(oldGrant, size = twoChunkSize(oldGrant))
        val freshChunks = chunks(freshGrant, size = twoChunkSize(freshGrant))
        val assembler = S3GrantQrAssembler()

        assertFalse(assembler.accept(oldChunks[0], nowMs = 1000).complete)
        assertFalse(assembler.accept(freshChunks[0], nowMs = 1000).complete)
        val imported = assembler.accept(freshChunks[1], nowMs = 1000)

        assertTrue(imported.complete)
        assertEquals(freshGrant, imported.grantJson)
    }

    @Test
    fun rejectsDuplicateChunkMismatchAndRawCredentialGrant() {
        val grant = grantJson(expiresAtMs = 3000)
        val chunks = chunks(grant, size = grant.length / 2)
        val assembler = S3GrantQrAssembler()
        assembler.accept(chunks[0], nowMs = 1000)
        val mismatchedDuplicate = chunks[0].replace("\"data\": \"", "\"data\": \"x")
        assertTrue(runCatching { assembler.accept(mismatchedDuplicate, nowMs = 1000) }.isFailure)

        val credentialGrant = grant.replace(
            """"expires_at_ms": 3000""",
            """"expires_at_ms": 3000, "AWS_SECRET_ACCESS_KEY": "not-allowed"""",
        )
        assertTrue(
            runCatching {
                S3GrantQrAssembler().accept(chunks(credentialGrant, size = credentialGrant.length).single(), nowMs = 1000)
            }.isFailure,
        )
    }

    private fun grantJson(expiresAtMs: Long): String =
        """
        {
          "upload_mode": "s3_presigned_put",
          "bucket": "veritas-pass4-mobile-evidence",
          "object_key": "mobile-evidence/run/chain/bundle.zip",
          "presigned_put_url": "https://s3.example.test/upload",
          "expires_at_ms": $expiresAtMs
        }
        """.trimIndent()

    private fun chunks(grant: String, size: Int, sha: String = sha256(grant)): List<String> =
        grant.chunked(size).mapIndexed { index, chunk ->
            val data = Base64.getUrlEncoder().withoutPadding()
                .encodeToString(chunk.toByteArray(Charsets.UTF_8))
            """
            {
              "type": "gbn.s3_grant.chunk",
              "version": 1,
              "grant_id": "grant-1",
              "index": ${index + 1},
              "count": ${grant.chunked(size).size},
              "sha256": "$sha",
              "data": "$data"
            }
            """.trimIndent()
        }

    private fun twoChunkSize(value: String): Int = (value.length + 1) / 2

    private fun sha256(value: String): String {
        val digest = MessageDigest.getInstance("SHA-256").digest(value.toByteArray(Charsets.UTF_8))
        return digest.joinToString("") { "%02x".format(it) }
    }
}
