package ai.arcships.aimux

import kotlinx.serialization.SerializationException
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.assertj.core.api.Assertions.assertThat
import org.assertj.core.api.Assertions.assertThatThrownBy
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
//   2. surfaces `.text`, `.toolCalls[0].toolName`, and `.content`,
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
            assertThat(server.lastRequestPath).isEqualTo("/chat/completions")

            // A plain-text response carries no tool calls.
            assertThat(result.toolCalls).isEmpty()

            // .content is accessible as a list (no JSON digging required).
            assertThat(result.content).isNotEmpty
            // Everything else is derived from the steps.
            assertThat(result.steps).hasSize(1)
            assertThat(result.finishReason).isEqualTo(FinishReasonUnified.STOP)
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

            // .content carries the tool-call item.
            assertThat(result.content.any { (it as JsonObject)["type"]?.jsonPrimitive?.content == "tool-call" }).isTrue()
            assertThat(result.finishReason).isEqualTo(FinishReasonUnified.TOOL_CALLS)
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
    fun `provider tool deferred results round-trip and omit null`() {
        for (supported in listOf(null, true, false)) {
            val original = ProviderTool(
                id = "anthropic.code_execution_20250522",
                name = "code_execution",
                supportsDeferredResults = supported,
            )
            val json = AimuxJson.encodeToString(ProviderTool.serializer(), original)
            assertThat(AimuxJson.parseToJsonElement(json).jsonObject.containsKey("supportsDeferredResults"))
                .isEqualTo(supported != null)
            assertThat(AimuxJson.decodeFromString(ProviderTool.serializer(), json)).isEqualTo(original)

            val tool = Tool.Provider.from(original)
            assertThat(tool.supportsDeferredResults).isEqualTo(supported)
            val toolJson = AimuxJson.encodeToString(Tool.serializer(), tool)
            assertThat(AimuxJson.parseToJsonElement(toolJson).jsonObject.containsKey("supportsDeferredResults"))
                .isEqualTo(supported != null)
            assertThat(AimuxJson.decodeFromString(Tool.serializer(), toolJson)).isEqualTo(tool)
        }
    }

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

    // ── FileBytes (a base64 string) / FileData (tagged on `type`) ─────────────────

    @Test
    fun `FileBytes Binary is a base64 string`() {
        // SharedV4FileDataData.data is Uint8Array | string upstream
        // (shared-v4-file-data.ts:7-10): one base64 string in the JSON
        // (uint8-utils.ts:12-23), never an array of numbers.
        val original = FileBytes.Binary(data = listOf(0, 1, 2, 250, 255))
        val json = AimuxJson.encodeToString(FileBytes.serializer(), original)
        assertThat(json).isEqualTo("\"AAEC+v8=\"")
        val decoded = AimuxJson.decodeFromString(FileBytes.serializer(), json)
        assertThat(decoded).isEqualTo(FileBytes.Base64("AAEC+v8="))
        assertThat(Base64Bytes.decode("AAEC-v8")).isEqualTo(original.data)
        assertThatThrownBy { AimuxJson.decodeFromString(FileBytes.serializer(), "[1,2,3,255]") }
            .isInstanceOf(kotlinx.serialization.SerializationException::class.java)
    }

    @Test
    fun `unknown fields keep null apart from missing`() {
        // body?: unknown (provider language-model-v4-generate-result.ts:41, :56)
        // and inputSchemaInput?: unknown (provider-utils
        // types/tool-approval-request.ts:41): the AI SDK's JSON keeps null and
        // omits undefined, and a parsed value writes back the same JSON.
        for (wire in listOf("{}", "{\"body\":null}", "{\"body\":{\"a\":null}}")) {
            val info = AimuxJson.decodeFromString(ResponseInfo.serializer(), wire)
            assertThat(AimuxJson.encodeToString(ResponseInfo.serializer(), info)).isEqualTo(wire)
            val request = AimuxJson.decodeFromString(RequestInfo.serializer(), wire)
            assertThat(AimuxJson.encodeToString(RequestInfo.serializer(), request)).isEqualTo(wire)
        }
        val approval = "{\"type\":\"tool-approval-request\",\"approvalId\":\"a1\"," +
            "\"toolCallId\":\"c1\",\"inputSchemaInput\":null}"
        val part = AimuxJson.decodeFromString(ContentPart.serializer(), approval)
        assertThat(AimuxJson.encodeToString(ContentPart.serializer(), part)).isEqualTo(approval)
    }

    @Test
    fun `image and file part bytes are base64 strings`() {
        // ImagePart.image / FilePart.data are DataContent (provider-utils
        // types/content-part.ts:42, :75): base64 strings in the JSON.
        val image = ContentPart.Image(image = listOf(137, 80, 78, 71), mediaType = "image/png")
        val json = AimuxJson.encodeToString(ContentPart.serializer(), image)
        assertThat(json).isEqualTo("{\"type\":\"image\",\"image\":\"iVBORw==\",\"mediaType\":\"image/png\"}")
        assertThat(AimuxJson.decodeFromString(ContentPart.serializer(), json)).isEqualTo(image)
        val file = ContentPart.File(data = listOf(0, 1, 2, 250, 255), mediaType = "application/pdf")
        assertThat(AimuxJson.encodeToString(ContentPart.serializer(), file)).isEqualTo(
            "{\"type\":\"file\",\"data\":{\"type\":\"data\",\"data\":\"AAEC+v8=\"},\"mediaType\":\"application/pdf\"}"
        )
    }

    @Test
    fun `generated files are base64 and mediaType`() {
        // GeneratedFile is {base64, mediaType, providerMetadata?} (ai
        // generate-text/generated-file.ts:10-32); a stream file part is
        // {type, file, providerMetadata?} (stream-language-model-call.ts:569-590);
        // the provider's LanguageModelV4File keeps {type, data, mediaType}.
        for (tag in listOf("file", "reasoning-file")) {
            val wire = """{"type":"$tag","file":{"base64":"aGk=","mediaType":"text/plain"},""" +
                """"providerMetadata":{"google":{"thoughtSignature":"sig"}}}"""
            val part = AimuxJson.decodeFromString(StreamPart.serializer(), wire)
            assertThat(AimuxJson.encodeToString(StreamPart.serializer(), part)).isEqualTo(wire)
        }
        val file = AimuxJson.decodeFromString(GeneratedFile.serializer(), """{"base64":"aGk=","mediaType":"text/plain"}""")
        assertThat(file.bytes()).isEqualTo(listOf(104, 105))

        val raw = """{"type":"file","data":{"type":"url","url":"https://example.com/a.png"},"mediaType":"image/png"}"""
        val content = AimuxJson.decodeFromString(GenerateContent.serializer(), raw)
        assertThat(AimuxJson.encodeToString(GenerateContent.serializer(), content)).isEqualTo(raw)
    }

    @Test
    fun `text stream deltas are text`() {
        // The text stream's text-delta and reasoning-delta carry `text` (ai
        // generate-text/stream-text-result.ts:435-440, :466-471); a
        // tool-input-delta keeps `delta` (:496-501).
        val wires = listOf(
            """{"type":"text-delta","id":"t1","text":"Hello"}""",
            """{"type":"reasoning-delta","id":"t1","text":"Hmm"}""",
            """{"type":"tool-input-delta","id":"t1","delta":"{\"a\""}""",
        )
        for (wire in wires) {
            val part = AimuxJson.decodeFromString(StreamPart.serializer(), wire)
            assertThat(AimuxJson.encodeToString(StreamPart.serializer(), part)).isEqualTo(wire)
        }
        val text = AimuxJson.decodeFromString(StreamPart.serializer(), wires[0]) as StreamPart.TextDelta
        assertThat(text.text).isEqualTo("Hello")
    }

    @Test
    fun `standalone tool calls and sources are tagged`() {
        // A result's toolCalls and sources are the AI SDK's TypedToolCall and
        // Source, tagged {type: 'tool-call'} (ai generate-text/tool-call.ts:8-14)
        // and {type: 'source', sourceType} (provider language-model-v4-source.ts:8,
        // :36); generate-text-result.ts:73, :78. A repair function's call is a
        // tagged LanguageModelV4ToolCall (language-model-v4-tool-call.ts:7).
        val call = """{"type":"tool-call","toolCallId":"call-1","toolName":"weather","input":{"city":"SG"}}"""
        val url = """{"type":"source","sourceType":"url","id":"s1","url":"https://example.com"}"""
        val result = AimuxJson.decodeFromString(
            GenerateTextResult.serializer(),
            """{"text":"","toolCalls":[$call],"sources":[$url],"finishReason":"stop","warnings":[]}""",
        )
        val written = AimuxJson.parseToJsonElement(AimuxJson.encodeToString(GenerateTextResult.serializer(), result))
            .jsonObject
        assertThat(written["toolCalls"]).isEqualTo(AimuxJson.parseToJsonElement("[$call]"))
        assertThat(written["sources"]).isEqualTo(AimuxJson.parseToJsonElement("[$url]"))
        val raw = RawToolCall(toolCallId = "c", toolName = "t", input = "{}")
        assertThat(AimuxJson.parseToJsonElement(AimuxJson.encodeToString(RawToolCall.serializer(), raw)))
            .isEqualTo(AimuxJson.parseToJsonElement("""{"type":"tool-call","toolCallId":"c","toolName":"t","input":"{}"}"""))

        // JSON written before the tag still reads; another tag is rejected.
        val untagged = AimuxJson.decodeFromString(ToolCall.serializer(), """{"toolCallId":"c","toolName":"t"}""")
        assertThat(untagged.type).isEqualTo("tool-call")
        assertThatThrownBy {
            AimuxJson.decodeFromString(ToolCall.serializer(), """{"type":"source","toolCallId":"c","toolName":"t"}""")
        }.isInstanceOf(SerializationException::class.java)
    }

    @Test
    fun `a parsed null output stays apart from no output`() {
        // aimux writes `output` only when the output was parsed; a parsed JSON
        // null is written as null (Rust GenerateTextResult, the public JSON
        // keeping null apart from undefined).
        val parsedNull = AimuxJson.decodeFromString(
            GenerateTextResult.serializer(), """{"text":"null","finishReason":"stop","output":null}""",
        )
        assertThat(parsedNull.output).isEqualTo(JsonNull)
        assertThat(AimuxJson.encodeToString(GenerateTextResult.serializer(), parsedNull)).contains(""""output":null""")
        val notParsed = AimuxJson.decodeFromString(
            GenerateTextResult.serializer(), """{"text":"","finishReason":"tool-calls"}""",
        )
        assertThat(notParsed.output).isNull()
        assertThat(AimuxJson.encodeToString(GenerateTextResult.serializer(), notParsed)).doesNotContain("output")
    }

    @Test
    fun `namespace maps keep insertion order`() {
        // Provider options, provider metadata and provider references are JS
        // objects upstream (@ai-sdk/provider shared-v4-provider-options.ts:24,
        // shared-v4-provider-metadata.ts:24, shared-v4-provider-reference.ts:20),
        // so their keys keep insertion order, through Rust and back (#229 NP2-1).
        val metadata = """{"zeta":{"z":1,"a":2},"alpha":{},"mid":{}}"""
        val invalid = """{"toolCallId":"call-1","toolName":"weather","input":{"town":"SG"},""" +
            """"dynamic":true,"invalid":true,"providerMetadata":$metadata,""" +
            """"error":{"name":"AI_InvalidToolInputError","toolName":"weather",""" +
            """"toolInput":"{\"town\":\"SG\"}","cause":"missing city"}}"""
        val opts = """{"tools":[{"type":"function","name":"weather","inputSchema":{"type":"object"}}]}"""
        val out = applyToolCallRepair(invalid, opts, """{"type":"unchanged"}""")
        val call = AimuxJson.decodeFromString(ToolCall.serializer(), out)
        assertThat(call.providerMetadata!!.keys).containsExactly("zeta", "alpha", "mid")
        assertThat(AimuxJson.encodeToString(ToolCall.serializer(), call)).contains(""""providerMetadata":$metadata""")

        val reference = """{"openai":"file-abc123","anthropic":"file-xyz789","google":"files/1"}"""
        val upload = AimuxJson.decodeFromString(UploadFileResult.serializer(), """{"providerReference":$reference}""")
        assertThat(upload.providerReference.keys).containsExactly("openai", "anthropic", "google")
        assertThat(AimuxJson.encodeToString(UploadFileResult.serializer(), upload))
            .startsWith("""{"providerReference":$reference""")
    }

    @Test
    fun `file parts are the AI SDK file part`() {
        // FilePart.data is FileData | DataContent | URL | ProviderReference
        // (provider-utils types/content-part.ts:75, file-data.ts:44-48) and
        // ReasoningFilePart.data is tagged data or a URL, or a string (:164).
        // There is no file-base64 / file-url / file-reference part.
        val wires = listOf(
            """{"type":"file","data":{"type":"data","data":"aGk="},"mediaType":"text/plain","filename":"a.txt"}""",
            """{"type":"file","data":{"type":"url","url":"https://example.com/a.pdf"},"mediaType":"application/pdf"}""",
            """{"type":"file","data":{"type":"reference","reference":{"openai":"file-abc"}},"mediaType":"application/pdf"}""",
            """{"type":"file","data":{"type":"text","text":"hello"},"mediaType":"text/plain"}""",
            """{"type":"file","data":"aGk=","mediaType":"text/plain"}""",
            """{"type":"file","data":"https://example.com/a.pdf","mediaType":"application/pdf"}""",
            """{"type":"file","data":{"openai":"file-abc"},"mediaType":"application/pdf"}""",
            """{"type":"reasoning-file","data":"aGk=","mediaType":"image/png"}""",
            """{"type":"reasoning-file","data":{"type":"url","url":"https://example.com/a.png"},"mediaType":"image/png"}""",
        )
        for (wire in wires) {
            val part = AimuxJson.decodeFromString(ContentPart.serializer(), wire)
            assertThat(AimuxJson.encodeToString(ContentPart.serializer(), part)).isEqualTo(wire)
        }
        assertThat((AimuxJson.decodeFromString(ContentPart.serializer(), wires[6]) as ContentPart.File).data)
            .isEqualTo(FilePartData.Reference(mapOf("openai" to "file-abc")))
        for (removed in listOf("file-base64", "file-url", "file-reference")) {
            assertThatThrownBy {
                AimuxJson.decodeFromString(
                    ContentPart.serializer(),
                    """{"type":"$removed","data":"aGk=","url":"https://example.com/a.pdf","mediaType":"application/pdf"}""",
                )
            }.isInstanceOf(SerializationException::class.java)
        }
        val url = ContentPart.File(FileData.Url("https://example.com/a.pdf"), "application/pdf")
        assertThat(AimuxJson.encodeToString(ContentPart.serializer(), url)).isEqualTo(wires[1])
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
    fun `stream parts decode the call-level framing`() {
        val decode = { json: String -> AimuxJson.decodeFromString(StreamPart.serializer(), json) }
        assertThat(decode("""{"type":"start"}""")).isEqualTo(StreamPart.Start())
        assertThat(decode("""{"type":"start-step","request":{},"warnings":[]}"""))
            .isInstanceOf(StreamPart.StartStep::class.java)
        assertThat(decode("""{"type":"text-delta","id":"t","text":"Hi"}"""))
            .isEqualTo(StreamPart.TextDelta(id = "t", text = "Hi"))
        val finish = decode(
            """{"type":"finish","finishReason":"tool-calls","rawFinishReason":"tool_use",""" +
                """"totalUsage":{"inputTokens":3,"inputTokenDetails":{"cacheReadTokens":1},""" +
                """"outputTokens":2,"outputTokenDetails":{},"totalTokens":5}}""",
        ) as StreamPart.Finish
        assertThat(finish.finishReason).isEqualTo(FinishReasonUnified.TOOL_CALLS)
        assertThat(finish.rawFinishReason).isEqualTo("tool_use")
        assertThat(finish.totalUsage.totalTokens).isEqualTo(5L)
        assertThat(finish.totalUsage.inputTokenDetails.cacheReadTokens).isEqualTo(1L)
        // LanguageModelUsage (ai types/usage.ts:10-67): camelCase, an unknown
        // count absent.
        assertThat(AimuxJson.encodeToString(LanguageModelUsage.serializer(), finish.totalUsage))
            .isEqualTo("""{"inputTokens":3,"inputTokenDetails":{"cacheReadTokens":1},"outputTokens":2,"outputTokenDetails":{},"totalTokens":5}""")
        assertThat(decode("""{"type":"abort","reason":"stop"}""")).isEqualTo(StreamPart.Abort("stop"))
        // Encoding writes the type tag back.
        assertThat(AimuxJson.encodeToString(StreamPart.serializer(), StreamPart.Start()))
            .isEqualTo("""{"type":"start"}""")
    }

    // The text stream's tool parts are the AI SDK's TypedToolResult,
    // TypedToolError, TypedToolOutputDenied and ToolApprovalResponseOutput,
    // camelCase (ai generate-text/tool-result.ts:10-42, tool-error.ts:6-36,
    // tool-output-denied.ts:7-15, tool-approval-response-output.ts:7-35).
    @Test
    fun `stream parts decode tool results and tool errors`() {
        val decode = { json: String -> AimuxJson.decodeFromString(StreamPart.serializer(), json) }
        val result = decode(
            """{"type":"tool-result","toolCallId":"srv-1","toolName":"web_search","input":{"q":"rust"},""" +
                """"output":[{"url":"https://example.com"}],"providerExecuted":true,"dynamic":true}""",
        ) as StreamPart.ToolResult
        assertThat(result.toolCallId).isEqualTo("srv-1")
        assertThat(result.input.toString()).isEqualTo("""{"q":"rust"}""")
        assertThat(result.output.toString()).isEqualTo("""[{"url":"https://example.com"}]""")
        assertThat(result.providerExecuted).isTrue()
        val deferred = decode(
            """{"type":"tool-result","toolCallId":"srv-0","toolName":"code_execution","output":{"stdout":"2"},"providerExecuted":true}""",
        ) as StreamPart.ToolResult
        assertThat(deferred.input).isNull()
        val error = decode(
            """{"type":"tool-error","toolCallId":"call-1","toolName":"weather","input":{"city":"Paris"},""" +
                """"error":"no such tool","dynamic":true}""",
        ) as StreamPart.ToolError
        assertThat(error.error.toString()).isEqualTo("\"no such tool\"")
        assertThat(error.providerExecuted).isNull()
        assertThat(error.dynamic).isTrue()
        assertThat(decode("""{"type":"tool-output-denied","toolCallId":"c","toolName":"t"}"""))
            .isEqualTo(StreamPart.ToolOutputDenied(toolCallId = "c", toolName = "t"))
        val response = decode(
            """{"type":"tool-approval-response","approvalId":"a","toolCall":{"type":"tool-call","toolCallId":"c","toolName":"t","input":{}},"approved":false}""",
        ) as StreamPart.ToolApprovalResponse
        assertThat(response.approved).isFalse()
        assertThat(AimuxJson.encodeToString(StreamPart.serializer(), StreamPart.ToolOutputDenied(toolCallId = "c", toolName = "t")))
            .isEqualTo("""{"type":"tool-output-denied","toolCallId":"c","toolName":"t"}""")
    }

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
