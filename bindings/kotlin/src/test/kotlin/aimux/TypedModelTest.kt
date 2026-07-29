package aimux

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
//   3. forwards typed `tools` / `tool_choice` onto the provider request,
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

        TypedModel.openai("sk-test-fake-key", "gpt-4o", server.baseUrl).use { model ->
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

        TypedModel.openai("sk-test-fake-key", "gpt-4o", server.baseUrl).use { model ->
            val result = model.generateText("What is the weather in Tokyo?", options)

            // .toolCalls[0].toolName / .toolCallId / .input — all typed.
            assertThat(result.toolCalls).hasSize(1)
            val call = result.toolCalls[0]
            assertThat(call.toolName).isEqualTo("get_weather")
            assertThat(call.toolCallId).isEqualTo("call_abc")
            // input is a JsonElement (tool arguments); inspect it directly.
            assertThat(call.input.jsonObject["location"]!!.jsonPrimitive.content)
                .isEqualTo("Tokyo")

            // .raw.content carries the ToolCall variant tag.
            assertThat(result.raw.hasContentVariant("ToolCall")).isTrue()
        }
    }

    @Test
    fun `typed tools and tool_choice reach the provider`() {
        server.responseBody = toolCallOpenAiResponse

        val options = GenerateTextOptions(
            tools = listOf(weatherTool),
            toolChoice = ToolChoice.REQUIRED,
        )

        TypedModel.openai("sk-test-fake-key", "gpt-4o", server.baseUrl).use { model ->
            model.generateText("What is the weather in Tokyo?", options)

            // The serialized options crossed the JSON boundary as the engine's
            // snake_case shape with only the fields the caller set.
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

        TypedModel.openai("sk-test-fake-key", "gpt-4o", server.baseUrl).use { model ->
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
