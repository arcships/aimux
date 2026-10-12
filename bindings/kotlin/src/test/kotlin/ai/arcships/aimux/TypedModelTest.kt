package ai.arcships.aimux

import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.assertj.core.api.Assertions.assertThat
import org.json.JSONArray
import org.json.JSONObject
import org.junit.jupiter.api.AfterEach
import org.junit.jupiter.api.BeforeEach
import org.junit.jupiter.api.Test

// ─────────────────────────────────────────────────────────────────────────────
// Tests for the typed [TypedModel] wrapper over the raw JSON-string [Model].
//
// Like StructuredE2ETest, these spin up the shared [MockProviderServer]
// (OpenAI chat-completions wire format) and assert that the typed wrapper:
//   1. returns a [GenerateTextResult] object (no manual JSON parsing),
//   2. surfaces `.text`, `.toolCalls[0].toolName`, and `.raw.content`,
//   3. forwards typed `tools` / `toolChoice` onto the provider request,
//   4. forwards multi-role [ModelMessage] conversations.
//
// No real network access — every request hits 127.0.0.1.
// ─────────────────────────────────────────────────────────────────────────────

class TypedModelTest {

    private lateinit var server: MockProviderServer

    @BeforeEach
    fun setUp() {
        server = MockProviderServer()
    }

    @AfterEach
    fun tearDown() {
        server.stop()
    }

    // ── canned OpenAI responses (mirror StructuredE2ETest) ───────────────

    /** Plain OpenAI text response (no tool calls). */
    private val plainOpenAiResponse: String =
        JSONObject().apply {
            put("id", "chatcmpl-test")
            put("model", "gpt-4o")
            put(
                "choices",
                JSONArray().put(
                    JSONObject().apply {
                        put(
                            "message",
                            JSONObject().apply {
                                put("role", "assistant")
                                put("content", "Rust is a systems programming language.")
                            },
                        )
                        put("finish_reason", "stop")
                    },
                ),
            )
            put(
                "usage",
                JSONObject().apply {
                    put("prompt_tokens", 10)
                    put("completion_tokens", 8)
                    put("total_tokens", 18)
                },
            )
        }.toString()

    /** OpenAI response that requests a tool call (get_weather). */
    private val toolCallOpenAiResponse: String =
        JSONObject().apply {
            put("id", "chatcmpl-tc")
            put("model", "gpt-4o")
            put(
                "choices",
                JSONArray().put(
                    JSONObject().apply {
                        put(
                            "message",
                            JSONObject().apply {
                                put("role", "assistant")
                                put("content", JSONObject.NULL)
                                put(
                                    "tool_calls",
                                    JSONArray().put(
                                        JSONObject().apply {
                                            put("id", "call_abc")
                                            put("type", "function")
                                            put(
                                                "function",
                                                JSONObject().apply {
                                                    put("name", "get_weather")
                                                    put("arguments", "{\"location\":\"Tokyo\"}")
                                                },
                                            )
                                        },
                                    ),
                                )
                            },
                        )
                        put("finish_reason", "tool_calls")
                    },
                ),
            )
            put(
                "usage",
                JSONObject().apply {
                    put("prompt_tokens", 20)
                    put("completion_tokens", 10)
                    put("total_tokens", 30)
                },
            )
        }.toString()

    // ── typed builders ──────────────────────────────────────────────────

    /** A JSON Schema for the get_weather tool's `location` argument. */
    private val weatherSchema: JsonObject = JsonObject(
        mapOf(
            "type" to JsonPrimitive("object"),
            "properties" to JsonObject(
                mapOf("location" to JsonObject(mapOf("type" to JsonPrimitive("string"))))
            ),
        )
    )

    /** A typed function tool definition for `get_weather`. */
    private val weatherTool: Tool = Tool.Function(
        name = "get_weather",
        inputSchema = weatherSchema,
    )

    // ── Tests ───────────────────────────────────────────────────────────

    @Test
    fun `generateText returns a typed GenerateTextResult with text and raw content`() {
        server.responseBody = plainOpenAiResponse

        TypedModel(chatCompletionsModel(server.baseUrl), ownsModel = true).use { model ->
            val result = model.generateText("What is Rust?")

            // No manual parsing: the wrapper returned a typed object.
            assertThat(result).isInstanceOf(GenerateTextResult::class.java)

            // .text is a plain Kotlin String.
            assertThat(result.text).isEqualTo("Rust is a systems programming language.")

            // A plain-text response carries no tool calls.
            assertThat(result.toolCalls).isEmpty()

            // .raw.content is accessible as a list (no JSON digging required).
            assertThat(result.raw.content).isNotEmpty
        }
    }

    @Test
    fun `generateText parses tool_calls into typed ToolCall objects`() {
        server.responseBody = toolCallOpenAiResponse

        val options = GenerateTextOptions(
            tools = listOf(weatherTool),
            toolChoice = ToolChoice.AUTO,
        )

        TypedModel(chatCompletionsModel(server.baseUrl), ownsModel = true).use { model ->
            val result = model.generateText("What is the weather in Tokyo?", options)

            // .toolCalls[0].toolName / .toolCallId / .input — all typed.
            assertThat(result.toolCalls).hasSize(1)
            val call = result.toolCalls[0]
            assertThat(call.toolName).isEqualTo("get_weather")
            assertThat(call.toolCallId).isEqualTo("call_abc")
            // input is a JsonElement (tool arguments); inspect it directly.
            assertThat(call.input.jsonObject["location"]!!.jsonPrimitive.content)
                .isEqualTo("Tokyo")

            // .raw.content carries the tool-call item (input is the raw argument text).
            assertThat(result.raw.content.any { it is GenerateContent.ToolCall }).isTrue()
        }
    }

    @Test
    fun `typed tools and tool_choice reach the provider`() {
        server.responseBody = toolCallOpenAiResponse

        val options = GenerateTextOptions(
            tools = listOf(weatherTool),
            toolChoice = ToolChoice.REQUIRED,
        )

        TypedModel(chatCompletionsModel(server.baseUrl), ownsModel = true).use { model ->
            model.generateText("What is the weather in Tokyo?", options)

            // The serialized options crossed the JSON boundary in the engine's
            // camelCase shape with only the fields the caller set.
            val reqBody = JSONObject(server.lastRequestBody)
            assertThat(reqBody.getString("tool_choice")).isEqualTo("required")
            assertThat(reqBody.has("tools")).isTrue()
            val tools = reqBody.getJSONArray("tools")
            assertThat(tools.length()).isEqualTo(1)
            val tool = tools.getJSONObject(0)
            // The engine forwards the typed tool to the provider in OpenAI's
            // wire format: {type:"function", function:{name, parameters}}.
            assertThat(tool.getString("type")).isEqualTo("function")
            assertThat(tool.getJSONObject("function").getString("name")).isEqualTo("get_weather")
            assertThat(tool.getJSONObject("function").getJSONObject("parameters").getString("type"))
                .isEqualTo("object")
        }
    }

    @Test
    fun `multi-role ModelMessage list reaches the provider`() {
        server.responseBody = plainOpenAiResponse

        val messages = listOf(
            ModelMessage.text(Role.SYSTEM, "You are a helpful assistant."),
            ModelMessage.text(Role.USER, "What is Rust?"),
        )

        TypedModel(chatCompletionsModel(server.baseUrl), ownsModel = true).use { model ->
            val result = model.generateText(messages)

            assertThat(result.text).isEqualTo("Rust is a systems programming language.")

            // The provider received both messages (system + user), in order.
            val reqBody = JSONObject(server.lastRequestBody)
            assertThat(reqBody.getString("model")).isEqualTo("gpt-4o")
            val reqMessages = reqBody.getJSONArray("messages")
            assertThat(reqMessages.length()).isEqualTo(2)
            assertThat(reqMessages.getJSONObject(0).getString("role")).isEqualTo("system")
            assertThat(reqMessages.getJSONObject(0).getString("content"))
                .isEqualTo("You are a helpful assistant.")
            assertThat(reqMessages.getJSONObject(1).getString("role")).isEqualTo("user")
            assertThat(reqMessages.getJSONObject(1).getString("content")).isEqualTo("What is Rust?")
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Round-trip (de)serialization tests for the typed content / file data classes.
//
// These are pure serialization tests — no network, no [MockProviderServer]. Each
// test builds a typed value, serializes it with [AimuxJson], decodes it back, and
// asserts the round-tripped value equals the original. They also pin down the
// wire-format quirks the typed wrappers rely on:
//   - [GenerateContent.File] carries no `filename` (unlike [ContentPart.File]),
//   - [GenerateContent.ToolResult] keeps the executed-tool `result` shape, while
//     [ContentPart.ToolResult] carries a [ToolResultOutput] under `output`,
//   - every union is tagged on `type` with the AI SDK's kebab-case names.
// ─────────────────────────────────────────────────────────────────────────────
class TypedModelRoundTripTest {

    @Test
    fun `top-level ToolCall provider metadata round-trips`() {
        val original = ToolCall(
            toolCallId = "call_1",
            toolName = "get_weather",
            input = JsonObject(mapOf("city" to JsonPrimitive("Paris"))),
            providerMetadata = mapOf("openai" to mapOf("itemId" to JsonPrimitive("item_1"))),
        )
        val json = AimuxJson.encodeToString(ToolCall.serializer(), original)
        assertThat(json).contains("\"providerMetadata\"")
        val decoded = AimuxJson.decodeFromString(ToolCall.serializer(), json)
        assertThat(decoded).isEqualTo(original)
    }

    // ── GenerateContent (tagged on `type`) ──────────────────────────────

    @Test
    fun `GenerateContent Text round-trips`() {
        val original = GenerateContent.Text(text = "hello", providerMetadata = null)
        val json = AimuxJson.encodeToString(GenerateContent.serializer(), original)
        val decoded = AimuxJson.decodeFromString(GenerateContent.serializer(), json)
        assertThat(decoded).isEqualTo(original)
    }

    @Test
    fun `GenerateContent ToolCall round-trips`() {
        val original = GenerateContent.ToolCall(
            toolCallId = "call_1",
            toolName = "get_weather",
            input = """{"location":"Tokyo"}""",
            providerExecuted = false,
            dynamic = true,
            providerMetadata = null,
        )
        val json = AimuxJson.encodeToString(GenerateContent.serializer(), original)
        assertThat(json).contains("\"type\":\"tool-call\"")
        val decoded = AimuxJson.decodeFromString(GenerateContent.serializer(), json)
        assertThat(decoded).isEqualTo(original)
    }

    @Test
    fun `GenerateContent Reasoning round-trips`() {
        val original = GenerateContent.Reasoning(text = "thinking...", providerMetadata = null)
        val json = AimuxJson.encodeToString(GenerateContent.serializer(), original)
        val decoded = AimuxJson.decodeFromString(GenerateContent.serializer(), json)
        assertThat(decoded).isEqualTo(original)
    }



    @Test
    fun `GenerateContent ToolResult round-trips using result not output`() {
        val original = GenerateContent.ToolResult(
            toolCallId = "call_1",
            toolName = "get_weather",
            result = JsonObject(mapOf("temp" to JsonPrimitive("20"))),
            isError = true,
            preliminary = true,
            dynamic = false,
            providerMetadata = null,
        )
        val json = AimuxJson.encodeToString(GenerateContent.serializer(), original)
        // The field is `result`, never `output`.
        assertThat(json).contains("\"result\"").doesNotContain("\"output\"")
        val decoded = AimuxJson.decodeFromString(GenerateContent.serializer(), json)
        assertThat(decoded).isEqualTo(original)
    }

    // ── ContentPart (tagged on `type`) ───────────────────────

    @Test
    fun `ContentPart Text round-trips`() {
        val original = ContentPart.Text(text = "hi", providerOptions = null)
        val json = AimuxJson.encodeToString(ContentPart.serializer(), original)
        val decoded = AimuxJson.decodeFromString(ContentPart.serializer(), json)
        assertThat(decoded).isEqualTo(original)
        assertThat(json).contains("\"text\"")
    }

    @Test
    fun `ContentPart ToolCall round-trips`() {
        val original = ContentPart.ToolCall(
            toolCallId = "call_1",
            toolName = "get_weather",
            input = JsonObject(mapOf("location" to JsonPrimitive("Tokyo"))),
            providerExecuted = true,
            providerOptions = null,
        )
        val json = AimuxJson.encodeToString(ContentPart.serializer(), original)
        val decoded = AimuxJson.decodeFromString(ContentPart.serializer(), json)
        assertThat(decoded).isEqualTo(original)
        assertThat(json).contains("\"type\":\"tool-call\"")
        assertThat(json).contains("\"providerExecuted\":true")
    }

    @Test
    fun `ContentPart ToolResult round-trips with a typed output`() {
        val original = ContentPart.ToolResult(
            toolCallId = "call_1",
            toolName = "get_weather",
            output = ToolResultOutput.JsonValue(JsonObject(mapOf("temp" to JsonPrimitive("20")))),
            providerOptions = null,
        )
        val json = AimuxJson.encodeToString(ContentPart.serializer(), original)

        // `output` is a tagged union; the old result / isError fields are gone.
        assertThat(json).contains("\"type\":\"tool-result\"")
        assertThat(json).contains("\"output\":{\"type\":\"json\",\"value\":{\"temp\":\"20\"}}")
        assertThat(json).doesNotContain("\"result\"").doesNotContain("\"isError\"")

        val decoded = AimuxJson.decodeFromString(ContentPart.serializer(), json)
        assertThat(decoded).isEqualTo(original)
    }

    // ── FileBytes (untagged) / FileData (tagged on `type`) ────────────────────────

    @Test
    fun `FileBytes Binary round-trips`() {
        val original = FileBytes.Binary(data = listOf(1, 2, 3, 255))
        val json = AimuxJson.encodeToString(FileBytes.serializer(), original)
        val decoded = AimuxJson.decodeFromString(FileBytes.serializer(), json)
        assertThat(decoded).isEqualTo(original)
        assertThat(json).isEqualTo("[1,2,3,255]")
    }

    @Test
    fun `FileBytes Base64 round-trips`() {
        val original = FileBytes.Base64(data = "aGVsbG8=")
        val json = AimuxJson.encodeToString(FileBytes.serializer(), original)
        val decoded = AimuxJson.decodeFromString(FileBytes.serializer(), json)
        assertThat(decoded).isEqualTo(original)
        assertThat(json).isEqualTo("\"aGVsbG8=\"")
    }

    @Test
    fun `FileData Data round-trips`() {
        val original = FileData.Data(data = FileBytes.Base64("aGVsbG8="))
        val json = AimuxJson.encodeToString(FileData.serializer(), original)
        val decoded = AimuxJson.decodeFromString(FileData.serializer(), json)
        assertThat(decoded).isEqualTo(original)
        assertThat(json).contains("\"type\":\"data\"")
    }

    @Test
    fun `FileData Url round-trips`() {
        val original = FileData.Url(url = "https://example.com/file")
        val json = AimuxJson.encodeToString(FileData.serializer(), original)
        val decoded = AimuxJson.decodeFromString(FileData.serializer(), json)
        assertThat(decoded).isEqualTo(original)
        assertThat(json).contains("\"type\":\"url\"")
    }

    // ── GenerateResult (integration: mixed content variants) ───────────

    @Test
    fun `GenerateResult round-trips with mixed content variants`() {
        val original = GenerateResult(
            content = listOf(
                GenerateContent.Text(text = "hello"),
                GenerateContent.ToolCall(
                    toolCallId = "call_1",
                    toolName = "get_weather",
                    input = """{"location":"Tokyo"}""",
                ),
                GenerateContent.ToolResult(
                    toolCallId = "call_1",
                    toolName = "get_weather",
                    result = JsonObject(mapOf("temp" to JsonPrimitive("20"))),
                    isError = false,
                ),
            ),
            finishReason = FinishReason(unified = FinishReasonUnified.STOP),
            usage = Usage.of(input = 10, output = 5),
        )
        val json = AimuxJson.encodeToString(GenerateResult.serializer(), original)
        val decoded = AimuxJson.decodeFromString(GenerateResult.serializer(), json)

        // The whole structure round-trips intact.
        assertThat(decoded).isEqualTo(original)

        // Variant order is preserved.
        assertThat(decoded.content.map { it::class.simpleName })
            .containsExactly("Text", "ToolCall", "ToolResult")

        // Spot-check the decoded content variants by type and field.
        assertThat(decoded.content[0]).isInstanceOf(GenerateContent.Text::class.java)
        assertThat((decoded.content[0] as GenerateContent.Text).text).isEqualTo("hello")

        assertThat(decoded.content[1]).isInstanceOf(GenerateContent.ToolCall::class.java)
        assertThat((decoded.content[1] as GenerateContent.ToolCall).toolName).isEqualTo("get_weather")

        assertThat(decoded.content[2]).isInstanceOf(GenerateContent.ToolResult::class.java)
        assertThat((decoded.content[2] as GenerateContent.ToolResult).result.toString())
            .isEqualTo("""{"temp":"20"}""")

        // The tags are present on the wire, and there is no `output`.
        assertThat(json).contains("\"type\":\"text\"").contains("\"type\":\"tool-call\"")
            .contains("\"type\":\"tool-result\"")
        assertThat(json).contains("\"result\"").doesNotContain("\"output\"")
    }
}
