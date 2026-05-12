package com.veritas.gbn.mobile.model

object JsonText {
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

    fun stringField(json: String, field: String): String? {
        val match = Regex(""""${Regex.escape(field)}"\s*:\s*"([^"]*)"""").find(json)
        return match?.groupValues?.get(1)
    }

    fun longField(json: String, field: String): Long? {
        val match = Regex(""""${Regex.escape(field)}"\s*:\s*([0-9]+)""").find(json)
        return match?.groupValues?.get(1)?.toLongOrNull()
    }

    fun hasField(json: String, field: String): Boolean =
        Regex(""""${Regex.escape(field)}"\s*:""").containsMatchIn(json)

    fun stringFieldInObject(json: String, objectField: String, field: String): String? {
        val body = objectBody(json, objectField) ?: return null
        return stringField("{$body}", field)
    }

    fun longFieldInObject(json: String, objectField: String, field: String): Long? {
        val body = objectBody(json, objectField) ?: return null
        return longField("{$body}", field)
    }

    fun objectWithFields(fields: Map<String, String?>): String =
        fields.entries
            .filter { it.value != null }
            .joinToString(prefix = "{", postfix = "}") { (key, value) ->
                """"$key":${quote(value.orEmpty())}"""
            }

    private fun objectBody(json: String, objectField: String): String? =
        Regex(""""${Regex.escape(objectField)}"\s*:\s*\{([^{}]*)\}""", RegexOption.DOT_MATCHES_ALL)
            .find(json)
            ?.groupValues
            ?.get(1)
}
