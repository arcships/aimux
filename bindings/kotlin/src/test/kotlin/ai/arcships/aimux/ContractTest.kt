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

    /// Every `GenerateContent` fixture decodes into its concrete variant.
    ///
    /// The variant type is asserted, not merely the absence of an exception:
    /// `GenerateContent` falls back to [GenerateContent.Unknown] for forward
    /// compatibility, so a variant whose shape drifted would still decode —
    /// silently, as `Unknown`. Only naming the expected class catches that.
    /// The fixture count is asserted so a newly added fixture cannot slip past
    /// this test unnoticed.
    @Test
    fun `generate content fixtures decode into their variants`() {
        val fixtures = loadFixtures()
        val byName = LinkedHashMap<String, GenerateContent>()
        for (element in fixtures) {
            val obj = element.jsonObject
            if (obj["type"]?.jsonPrimitive?.content != "GenerateContent") continue
            val name = obj["name"]!!.jsonPrimitive.content
            val json = obj["json"]!!.jsonPrimitive.content
            byName[name] = AimuxJson.decodeFromString<GenerateContent>(json)
        }
        assertThat(byName).hasSize(8)

        assertThat(byName["generate_content_text"]).isInstanceOf(GenerateContent.Text::class.java)
        assertThat(byName["generate_content_reasoning_no_metadata"])
            .isInstanceOf(GenerateContent.Reasoning::class.java)

        // The shapes a parse-only check leaves invisible: the tool-call input,
        // the nested file union, and Source's optionals.
        val toolCall = byName["generate_content_tool_call"] as GenerateContent.ToolCall
        assertThat(toolCall.toolCallId).isEqualTo("call_1")
        assertThat(toolCall.input.jsonObject["city"]?.jsonPrimitive?.content).isEqualTo("Paris")
        assertThat(toolCall.providerExecuted).isTrue()

        val file = byName["generate_content_file"] as GenerateContent.File
        assertThat(file.mediaType).isEqualTo("image/png")

        val source =
            byName["generate_content_source_unset_optionals"] as GenerateContent.Source
        assertThat(source.url).isNull()
        assertThat(source.title).isNull()
    }

    /// The regression lock for `topK`, which this binding declared as `Long`
    /// against an `f64` wire.
    ///
    /// Every value is asserted explicitly rather than relying on decoding to
    /// fail: a decode-only test would pass again the moment someone restored
    /// the integer type, since a serializer is free to coerce `40.5` rather
    /// than reject it. `topK` is the field that was wrong; the others are
    /// asserted so a narrowing of any single numeric type is caught here too.
    @Test
    fun `numeric options decode with full precision`() {
        val json = fixtureJson(loadFixtures(), "generate_text_options_numeric_types")
        val opts = AimuxJson.decodeFromString<GenerateTextOptions>(json)

        assertThat(opts.topK).isEqualTo(40.5)
        assertThat(opts.frequencyPenalty).isEqualTo(-0.5)
        assertThat(opts.temperature).isEqualTo(0.7)
        assertThat(opts.topP).isEqualTo(0.95)
        assertThat(opts.presencePenalty).isEqualTo(0.5)
        assertThat(opts.maxOutputTokens).isEqualTo(256L)
        assertThat(opts.seed).isEqualTo(42L)
        assertThat(opts.maxRetries).isEqualTo(3L)
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
