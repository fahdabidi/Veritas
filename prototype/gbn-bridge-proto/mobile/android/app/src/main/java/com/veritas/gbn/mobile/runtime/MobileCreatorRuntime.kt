package com.veritas.gbn.mobile.runtime

class MobileCreatorRuntime(configJson: String) : AutoCloseable {
    private var handle: Long = 0

    init {
        val response = Native.gbnMobileRuntimeCreate(configJson)
        val parsed = MobileJson.requireOk(response)
        handle = MobileJson.requireLong(parsed, "handle")
    }

    fun nodeMetadata(): String = call("nodeMetadata", "{}")

    fun localDht(): String = call("localDht", "{}")

    fun traceEvents(filterJson: String): String = call("traceEvents", filterJson)

    fun resetState(chainId: String): String = call("resetState", """{"chain_id":"$chainId"}""")

    fun previewBootstrapDhtQr(payload: String): String =
        call("previewBootstrapDhtQr", MobileJson.objectWithPayload(payload))

    fun importHostCreatorDhtSeed(payload: String): String =
        call("importHostCreatorDhtSeed", MobileJson.objectWithPayload(payload))

    fun refreshBridgeCatalog(requestJson: String = "{}"): String =
        call("refreshBridgeCatalog", requestJson)

    fun buildSyntheticUploadSession(requestJson: String): String =
        call("buildSyntheticUploadSession", requestJson)

    fun exportEvidence(): String = call("exportEvidence", "{}")

    fun subscribeEvents(): String = call("subscribeEvents", "{}")

    fun seedHostCreator(requestJson: String): String = call("seedHostCreator", requestJson)

    fun bootstrapNewCreator(requestJson: String): String = call("bootstrapNewCreator", requestJson)

    fun sendDummy(requestJson: String): String = call("sendDummy", requestJson)

    fun sendUpload(requestJson: String): String = call("sendUpload", requestJson)

    private fun call(method: String, requestJson: String): String {
        check(handle != 0L) { "MobileCreatorRuntime is closed" }
        return MobileJson.requireOk(Native.gbnMobileRuntimeCall(handle, method, requestJson))
    }

    override fun close() {
        val current = handle
        if (current != 0L) {
            Native.gbnMobileRuntimeClose(current)
            handle = 0
        }
    }

    private object Native {
        init {
            System.loadLibrary("gbn_bridge_mobile_ffi")
        }

        external fun gbnMobileRuntimeCreate(configJson: String): String
        external fun gbnMobileRuntimeCall(handle: Long, method: String, requestJson: String): String
        external fun gbnMobileRuntimeClose(handle: Long): String
    }
}

object MobileJson {
    fun requireOk(responseJson: String): String {
        if (!responseJson.contains(""""ok":true""")) {
            error("native runtime call failed: $responseJson")
        }
        val bodyIndex = responseJson.indexOf(""""body":""")
        return if (bodyIndex >= 0) {
            val bodyStart = responseJson.skipWhitespace(bodyIndex + 7)
            responseJson.substring(bodyStart, responseJson.jsonValueEnd(bodyStart))
        } else {
            responseJson
        }
    }

    fun requireLong(json: String, field: String): Long {
        val marker = """"$field":"""
        val start = json.indexOf(marker)
        require(start >= 0) { "missing field $field in $json" }
        val valueStart = start + marker.length
        val valueEnd = json.indexOfAny(charArrayOf(',', '}'), valueStart)
        return json.substring(valueStart, valueEnd).trim().toLong()
    }

    fun objectWithPayload(payload: String): String =
        """{"payload":${quote(payload)}}"""

    fun quote(value: String): String =
        buildString {
            append('"')
            value.forEach { ch ->
                when (ch) {
                    '\\' -> append("\\\\")
                    '"' -> append("\\\"")
                    '\n' -> append("\\n")
                    '\r' -> append("\\r")
                    '\t' -> append("\\t")
                    else -> append(ch)
                }
            }
            append('"')
        }

    private fun String.skipWhitespace(start: Int): Int {
        var index = start
        while (index < length && this[index].isWhitespace()) index++
        return index
    }

    private fun String.jsonValueEnd(start: Int): Int {
        require(start in indices) { "missing body value in $this" }
        return when (this[start]) {
            '{', '[' -> compositeJsonValueEnd(start)
            '"' -> quotedJsonValueEnd(start)
            else -> primitiveJsonValueEnd(start)
        }
    }

    private fun String.compositeJsonValueEnd(start: Int): Int {
        var depth = 0
        var inString = false
        var escaping = false
        for (index in start until length) {
            val ch = this[index]
            if (inString) {
                when {
                    escaping -> escaping = false
                    ch == '\\' -> escaping = true
                    ch == '"' -> inString = false
                }
                continue
            }
            when (ch) {
                '"' -> inString = true
                '{', '[' -> depth++
                '}', ']' -> {
                    depth--
                    if (depth == 0) return index + 1
                }
            }
        }
        error("unterminated JSON body in $this")
    }

    private fun String.quotedJsonValueEnd(start: Int): Int {
        var escaping = false
        for (index in start + 1 until length) {
            val ch = this[index]
            when {
                escaping -> escaping = false
                ch == '\\' -> escaping = true
                ch == '"' -> return index + 1
            }
        }
        error("unterminated quoted JSON body in $this")
    }

    private fun String.primitiveJsonValueEnd(start: Int): Int {
        var index = start
        while (index < length && this[index] != ',' && this[index] != '}') index++
        return index
    }
}
