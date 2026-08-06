package ai.arcships.aimux

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import org.assertj.core.api.Assertions.assertThat
import org.junit.jupiter.api.Test
import java.io.File

/**
 * Cross-language contract tests for the Kotlin binding.
 *
 * Consumes the shared `contract-tests/fixtures/wire-format.json` (same
 * fixtures as Rust `contract_test.rs`, Node `run-node.ts`, Java
 * `ContractTest` and Go `wire_format_test.go`).
 *
 * Pure serialization tests: no native library is loaded.
 */
class ContractTest {

    /** Candidate locations for the shared fixtures, relative to the test working dir. */
    private val fixtureCandidates = listOf(
        "../../contract-tests/fixtures/wire-format.json", // gradle test cwd = bindings/kotlin
        "contract-tests/fixtures/wire-format.json",       // run from the repo root
        "../contract-tests/fixtures/wire-format.json",
    )

    private fun loadFixtures(): JsonArray {
        val file = fixtureCandidates.map(::File).firstOrNull { it.isFile }
            ?: throw IllegalStateException(
                "cannot find wire-format.json; tried $fixtureCandidates from ${File(".").absolutePath}"
            )
        return Json.parseToJsonElement(file.readText()).jsonArray
    }

    private fun fixtureJson(fixtures: JsonArray, name: String): String {
        for (f in fixtures) {
            if (name == f.jsonObject["name"]?.jsonPrimitive?.content) {
                return f.jsonObject["json"]!!.jsonPrimitive.content
            }
        }
        throw IllegalStateException("no fixture named '$name'")
    }

    /// RFC-0016 M2: the shared `stream_part_raw` fixture decodes to the
    /// Kotlin `StreamPart.Raw` variant with the raw payload intact.
    @Test
    fun `stream part raw fixture decodes to Raw variant`() {
        val json = fixtureJson(loadFixtures(), "stream_part_raw")
        val part = AimuxJson.decodeFromString<StreamPart>(json)
        assertThat(part).isInstanceOf(StreamPart.Raw::class.java)
        val raw = part as StreamPart.Raw
        assertThat(raw.rawValue.jsonObject["id"]?.jsonPrimitive?.content).isEqualTo("c1")
        assertThat(raw.rawValue.jsonObject["choices"]).isNotNull
    }

    /// RFC-0016 M2 true-case: `include_raw_chunks: true` round-trips through
    /// the Kotlin typed options.
    @Test
    fun `include raw chunks true round-trips through typed options`() {
        val json = fixtureJson(loadFixtures(), "generate_text_options_include_raw_chunks_true")
        val opts = AimuxJson.decodeFromString<GenerateTextOptions>(json)
        assertThat(opts.includeRawChunks).isTrue()

        // Re-encode: include_raw_chunks must survive (encodeDefaults=false,
        // so only non-default fields are emitted).
        val reencoded = AimuxJson.encodeToString(GenerateTextOptions.serializer(), opts)
        assertThat(reencoded).contains("\"include_raw_chunks\":true")
    }

    /// RFC-0024: `session_id` decodes from the shared fixture and round-trips.
    @Test
    fun `session id wire format and round-trip`() {
        val json = fixtureJson(loadFixtures(), "generate_text_options_with_session_id")
        val opts = AimuxJson.decodeFromString<GenerateTextOptions>(json)
        assertThat(opts.sessionId).isEqualTo("sess-1")

        val reencoded = AimuxJson.encodeToString(GenerateTextOptions.serializer(), opts)
        assertThat(reencoded).contains("\"session_id\":\"sess-1\"")
    }

    /// RFC-0016 M10: `Usage.raw` with a vendor-specific field survives a
    /// Kotlin round-trip.
    @Test
    fun `usage raw preserves vendor fields round-trip`() {
        val usage = Usage(
            inputTokens = TokenUsage(total = 20L),
            outputTokens = TokenUsage(total = 5L),
            raw = buildJsonObject {
                put("prompt_cache_hit_tokens", 42)
                put("prompt_tokens", 20)
            },
        )
        val json = AimuxJson.encodeToString(Usage.serializer(), usage)
        assertThat(json).contains("\"raw\"")
        val decoded = AimuxJson.decodeFromString<Usage>(json)
        assertThat(decoded.raw?.jsonObject?.get("prompt_cache_hit_tokens")?.jsonPrimitive?.content)
            .isEqualTo("42")
    }
}
