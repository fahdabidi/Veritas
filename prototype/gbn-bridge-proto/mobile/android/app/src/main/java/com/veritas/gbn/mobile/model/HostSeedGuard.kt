package com.veritas.gbn.mobile.model

data class HostSeedPreview(
    val hostCreatorId: String,
    val publicKeyFingerprint: String,
    val host: String,
    val port: String,
    val chainId: String,
    val expiresAtMs: Long,
) {
    fun toDisplay(): String =
        "host=$hostCreatorId key=$publicKeyFingerprint endpoint=$host:$port chain=$chainId expires=$expiresAtMs"
}

object HostSeedGuard {
    private val forbiddenFields = listOf(
        "publisher_entry",
        "publisher_dht",
        "publisher_public_key_hex",
        "seed_exit_bridge",
        "seed_exit_bridge_dht",
        "bridge_catalog",
        "bridge_entries",
    )

    fun preview(payload: String, nowMs: Long = System.currentTimeMillis()): HostSeedPreview {
        forbiddenFields.firstOrNull { JsonText.hasField(payload, it) }?.let { field ->
            throw IllegalArgumentException("HostCreator seed must not preload `$field`")
        }
        val publicKey = JsonText.stringField(payload, "host_creator_public_key_hex")
            ?: throw IllegalArgumentException("HostCreator seed requires host_creator_public_key_hex")
        require(publicKey.length >= 16) { "HostCreator public key is too short" }
        val expiresAtMs = JsonText.longField(payload, "expires_at_ms")
            ?: throw IllegalArgumentException("HostCreator seed requires expires_at_ms")
        require(expiresAtMs > nowMs) { "HostCreator seed is expired" }
        val host = JsonText.stringField(payload, "host")
            ?: throw IllegalArgumentException("HostCreator seed requires mobile-reachable endpoint host")
        val normalizedHost = host.lowercase()
        require(normalizedHost != "localhost" && normalizedHost != "127.0.0.1" && normalizedHost != "::1") {
            "HostCreator seed endpoint must not be localhost"
        }
        require(!normalizedHost.endsWith(".svc") && !normalizedHost.contains(".cluster.local")) {
            "HostCreator seed endpoint must not be cluster-local"
        }
        require(!Regex("""^(10|172\.(1[6-9]|2[0-9]|3[0-1])|192\.168)\.""").containsMatchIn(normalizedHost)) {
            "HostCreator seed endpoint must not be a private IP"
        }
        require(!payload.contains("/admin/", ignoreCase = true) && !payload.contains("\"admin", ignoreCase = true)) {
            "HostCreator seed endpoint must not be an admin listener"
        }
        return HostSeedPreview(
            hostCreatorId = JsonText.stringField(payload, "host_creator_id") ?: "host-creator",
            publicKeyFingerprint = publicKey.take(12),
            host = host,
            port = JsonText.longField(payload, "port")?.toString() ?: "443",
            chainId = JsonText.stringField(payload, "chain_id") ?: "pass4-host-seed",
            expiresAtMs = expiresAtMs,
        )
    }

    fun redacted(payload: String): String {
        forbiddenFields.firstOrNull { JsonText.hasField(payload, it) }?.let { field ->
            throw IllegalArgumentException("HostCreator seed must not preload `$field`")
        }
        return payload
            .replace(Regex(""""signature"\s*:\s*"[^"]*"""")) { """"signature":"redacted"""" }
            .replace(Regex(""""private_key[^"]*"\s*:\s*"[^"]*"""")) { """"private_key":"redacted"""" }
    }
}
