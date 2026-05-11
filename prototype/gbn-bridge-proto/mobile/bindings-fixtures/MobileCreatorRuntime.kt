package com.veritas.gbn.mobile.runtime

/**
 * Phase 2 Kotlin-facing fixture for the Rust mobile creator runtime.
 *
 * The native ABI uses numeric handles and JSON payloads internally. This wrapper keeps those
 * handles private so app code does not manipulate raw native pointers.
 */
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

    fun subscribeEvents(): Subscription = Subscription(call("subscribeEvents", "{}"))

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

    data class Subscription(val descriptorJson: String)

    private object Native {
        init {
            System.loadLibrary("gbn_bridge_mobile_ffi")
        }

        external fun gbnMobileRuntimeCreate(configJson: String): String
        external fun gbnMobileRuntimeCall(handle: Long, method: String, requestJson: String): String
        external fun gbnMobileRuntimeClose(handle: Long): String
    }
}

private object MobileJson {
    fun requireOk(responseJson: String): String {
        // The Phase 3 Android app replaces this tiny fixture parser with generated
        // bindings or kotlinx.serialization models. Phase 2 keeps this file dependency-free.
        if (!responseJson.contains(""""ok":true""")) {
            error("native runtime call failed: $responseJson")
        }
        val bodyIndex = responseJson.indexOf(""""body":""")
        return if (bodyIndex >= 0) {
            responseJson.substring(bodyIndex + 7, responseJson.lastIndexOf('}'))
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

    private fun quote(value: String): String =
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
}
