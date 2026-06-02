package com.veritas.gbn.mobile.runtime

import org.junit.Assert.assertEquals
import org.junit.Test

class MobileJsonTest {
    @Test
    fun extractsObjectBodyWithoutWrapperTrailer() {
        val response = """{"body":{"chain_id":"abc","nested":{"x":1}},"ok":true}"""

        assertEquals("""{"chain_id":"abc","nested":{"x":1}}""", MobileJson.requireOk(response))
    }

    @Test
    fun extractsArrayBodyWithoutWrapperTrailer() {
        val response = """{"body":[{"chain_id":"one"},{"chain_id":"two"}],"ok":true}"""

        assertEquals("""[{"chain_id":"one"},{"chain_id":"two"}]""", MobileJson.requireOk(response))
    }

    @Test
    fun extractsQuotedBodyWithoutWrapperTrailer() {
        val response = """{"body":"created \"ok\"","ok":true}"""

        assertEquals(""""created \"ok\""""", MobileJson.requireOk(response))
    }
}
