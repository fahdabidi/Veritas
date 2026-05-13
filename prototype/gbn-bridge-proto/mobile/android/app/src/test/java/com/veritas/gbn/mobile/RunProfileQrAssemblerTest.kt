package com.veritas.gbn.mobile

import com.veritas.gbn.mobile.model.JsonText
import com.veritas.gbn.mobile.model.RunProfileQrAssembler
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.security.MessageDigest
import java.util.Base64

class RunProfileQrAssemblerTest {
    @Test
    fun acceptsRawRunProfileJson() {
        val profile = """{"profile":"aws_public","run_id":"pass4-aws","endpoints":[]}"""
        val result = RunProfileQrAssembler().accept(profile)
        assertTrue(result.complete)
        assertEquals(profile, result.profileJson)
        assertTrue(result.message.contains("aws_public"))
    }

    @Test
    fun reassemblesChunkedRunProfileJson() {
        val profile = """{"profile":"aws_public","run_id":"pass4-aws","endpoints":[]}"""
        val chunks = chunks(profile, size = 18)
        val assembler = RunProfileQrAssembler()

        val first = assembler.accept(chunks.first())
        assertFalse(first.complete)

        val final = chunks.drop(1).fold(first) { _, chunk -> assembler.accept(chunk) }
        assertTrue(final.complete)
        assertEquals(profile, final.profileJson)
    }

    private fun chunks(profile: String, size: Int): List<String> {
        val sha = sha256(profile)
        val parts = profile.chunked(size)
        return parts.mapIndexed { index, data ->
            """
            {
              "schema": "veritas.pass4.run_profile_qr.v1",
              "profile_id": "profile-1",
              "index": ${index + 1},
              "count": ${parts.size},
              "sha256": "$sha",
              "encoding": "base64",
              "data": ${JsonText.quote(Base64.getEncoder().encodeToString(data.toByteArray(Charsets.UTF_8)))}
            }
            """.trimIndent()
        }
    }

    private fun sha256(value: String): String =
        MessageDigest.getInstance("SHA-256")
            .digest(value.toByteArray(Charsets.UTF_8))
            .joinToString(separator = "") { "%02x".format(it) }
}
