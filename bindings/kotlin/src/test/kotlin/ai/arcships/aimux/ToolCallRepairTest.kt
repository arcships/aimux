package ai.arcships.aimux

import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.assertj.core.api.Assertions.assertThat
import org.assertj.core.api.Assertions.assertThatThrownBy
import org.junit.jupiter.api.AfterEach
import org.junit.jupiter.api.Assertions.assertTimeoutPreemptively
import org.junit.jupiter.api.BeforeEach
import org.junit.jupiter.api.Test
import java.io.File
import java.time.Duration
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

// ─────────────────────────────────────────────────────────────────────────────
// Host-side tool-call repair (RFC-0035).
//
// Two halves:
//   1. the shared `tool-call-repair.json` fixture replayed through the three
//      native entry points (pure functions — no model, no server),
//   2. end-to-end through TypedModel against MockProviderServer: the provider
//      returns arguments that violate the tool schema, Core marks the call
//      invalid, and the repair function runs on the JVM.
// ─────────────────────────────────────────────────────────────────────────────

class ToolCallRepairTest {

    private lateinit var server: MockProviderServer

    @BeforeEach
    fun setUp() {
        server = MockProviderServer()
    }

    @AfterEach
    fun tearDown() {
        server.stop()
    }

    // ── shared fixture ──────────────────────────────────────────────────

    private val fixtureCandidates = listOf(
        "../../contract-tests/fixtures/tool-call-repair.json", // gradle test cwd = bindings/kotlin
        "contract-tests/fixtures/tool-call-repair.json",
        "../contract-tests/fixtures/tool-call-repair.json",
    )

    private fun fixtureCases(): List<JsonObject> {
        val file = fixtureCandidates.map(::File).firstOrNull { it.isFile }
            ?: throw IllegalStateException(
                "cannot find tool-call-repair.json; tried $fixtureCandidates from ${File(".").absolutePath}"
            )
        val cases = AimuxJson.parseToJsonElement(file.readText()).jsonObject["cases"]!!
        return cases.let { it as kotlinx.serialization.json.JsonArray }
            .map { it.jsonObject }
    }

    /** `opts` may be JSON null in the fixture, which is the binding's "defaults". */
    private fun optsOf(input: JsonObject): String? =
        input["opts"]?.takeIf { it !is JsonNull }?.toString()

    @Test
    fun `all shared contract fixture cases replay`() {
        fixtureCases().forEach { case ->
            val name = case["name"]!!.jsonPrimitive.content
            val input = case["input"]!!.jsonObject
            val invoke = {
                when (val function = case["function"]!!.jsonPrimitive.content) {
                    "tool_call_repair_context" -> toolCallRepairContext(
                        input["tool_call"]!!.toString(), input["prompt"]!!.toString(), optsOf(input))
                    "apply_tool_call_repair" -> applyToolCallRepair(
                        input["tool_call"]!!.toString(), optsOf(input), input["reply"]!!.toString())
                    "apply_tool_call_repair_to_result" -> applyToolCallRepairToResult(
                        input["result"]!!.toString(), optsOf(input),
                        input["tool_call_id"]!!.jsonPrimitive.content, input["reply"]!!.toString())
                    else -> throw AssertionError("unknown fixture function: $function")
                }
            }
            if (case["expected_error"] != null) {
                assertThat(case["expected_error"]!!.jsonPrimitive.content).isEqualTo("InvalidArgument")
                assertThatThrownBy { invoke() }.describedAs(name)
                    .isInstanceOf(InvalidArgumentError::class.java)
            } else {
                assertThat(AimuxJson.parseToJsonElement(invoke())).describedAs(name)
                    .isEqualTo(case["expected"]!!)
            }
        }
    }

    // ── end to end ──────────────────────────────────────────────────────

    /** `city` is required; the canned response sends `town` instead. */
    private val weatherTool: Tool = Tool.Function(
        name = "weather",
        inputSchema = AimuxJson.parseToJsonElement(
            """{"type":"object","properties":{"city":{"type":"string"}},"required":["city"],"additionalProperties":false}"""
        ),
    )

    private fun openAiToolCall(arguments: String, name: String = "weather"): String =
        """
        {"id":"chatcmpl-1","model":"gpt-4o","choices":[{"message":{"role":"assistant","content":null,
         "tool_calls":[{"id":"call-1","type":"function","function":{"name":"$name","arguments":${
            JsonPrimitive(arguments)
        }}}]},"finish_reason":"tool_calls"}],
         "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}
        """.trimIndent()

    private fun model(): TypedModel = TypedModel.openai("sk-test-fake-key", "gpt-4o", server.baseUrl)

    private fun options(repair: RepairToolCall?): GenerateTextOptions =
        GenerateTextOptions(tools = listOf(weatherTool), repairToolCall = repair)

    @Test
    fun `a repaired call becomes valid in both tool_calls and response_messages`() {
        server.responseBody = openAiToolCall("""{"town":"Singapore"}""")

        model().use { model ->
            val result = model.generateText(
                "weather in Singapore?",
                options { context ->
                    // The repair function sees the RAW argument text, not a parsed object.
                    assertThat(context.toolCall.input).isEqualTo("""{"town":"Singapore"}""")
                    assertThat(context.inputSchema.jsonObject["required"].toString()).contains("city")
                    assertThat(context.messages).hasSize(1)
                    context.toolCall.copy(input = """{"city":"Singapore"}""")
                },
            )

            val call = result.toolCalls.single()
            assertThat(call.invalid).isNotEqualTo(true)
            assertThat(call.error).isNull()
            assertThat(call.input.jsonObject["city"]?.jsonPrimitive?.content).isEqualTo("Singapore")

            // The transcript must agree, or the next turn replays bad arguments.
            val part = result.responseMessages
                .flatMap { it.contentParts ?: emptyList() }
                .filterIsInstance<ContentPart.ToolCall>()
                .single()
            assertThat(part.toolCallId).isEqualTo("call-1")
            assertThat(part.input.jsonObject["city"]?.jsonPrimitive?.content).isEqualTo("Singapore")
        }
    }

    @Test
    fun `generateTextAsOpenAI carries the repaired arguments`() {
        server.responseBody = openAiToolCall("""{"town":"Singapore"}""")

        model().use { model ->
            // A ChatCompletion has no invalid marker: the native result is
            // repaired, then converted.
            val completion = model.generateTextAsOpenAI(
                "weather in Singapore?",
                options { context -> context.toolCall.copy(input = """{"city":"Singapore"}""") },
            )

            val call = completion.choices.single().message.toolCalls!!.single()
            assertThat(call.id).isEqualTo("call-1")
            assertThat(call.function.arguments).isEqualTo("""{"city":"Singapore"}""")
        }
    }

    /**
     * The host loop's non-happy branches: what a hook's two failure modes do
     * to the call, and the two cases where the hook must not run at all.
     */
    @Test
    fun `a throwing hook wraps, a null hook preserves, and valid or untooled calls skip repair`() {
        val invalidCall = openAiToolCall("""{"town":"Singapore"}""")
        var hookCalls = 0

        model().use { model ->
            // Throwing means "repair failed": the call stays invalid and the
            // original failure is kept under the ToolCallRepair wrapper.
            server.responseBody = invalidCall
            var call = model.generateText(
                "weather in Singapore?",
                options { hookCalls++; throw RuntimeException("repair model unavailable") },
            ).toolCalls.single()
            assertThat(hookCalls).isEqualTo(1)
            assertThat(call.invalid).isEqualTo(true)
            val repairError = call.error!!.jsonObject["ToolCallRepair"]!!.jsonObject
            assertThat(repairError["cause"]!!.jsonObject["Other"]!!.jsonPrimitive.content)
                .isEqualTo("repair model unavailable")
            assertThat(repairError["original_error"]!!.jsonObject.keys)
                .containsExactly("InvalidToolInput")

            // Returning null declines the repair: the ORIGINAL error survives
            // unwrapped, so the caller cannot tell a decline from no hook.
            server.responseBody = invalidCall
            call = model.generateText(
                "weather in Singapore?",
                options { hookCalls++; null },
            ).toolCalls.single()
            assertThat(hookCalls).isEqualTo(2)
            assertThat(call.invalid).isEqualTo(true)
            assertThat(call.error!!.jsonObject.keys).containsExactly("InvalidToolInput")

            // A valid call never reaches the hook.
            server.responseBody = openAiToolCall("""{"city":"Singapore"}""")
            call = model.generateText(
                "weather in Singapore?",
                options { hookCalls++; null },
            ).toolCalls.single()
            assertThat(hookCalls).isEqualTo(2)
            assertThat(call.invalid).isNotEqualTo(true)

            // Without a tool set the native context is null (AI SDK never
            // repairs those), so the invalid call is returned as-is.
            server.responseBody = invalidCall
            call = model.generateText(
                "weather in Singapore?",
                GenerateTextOptions(repairToolCall = { hookCalls++; null }),
            ).toolCalls.single()
            assertThat(hookCalls).isEqualTo(2)
            assertThat(call.invalid).isEqualTo(true)
        }
    }

    /** The nested generation a repair hook performs: valid arguments as text. */
    private val repairCompletion: String =
        """{"id":"chatcmpl-2","model":"gpt-4o","choices":[{"message":{"role":"assistant",
           "content":"{\"city\":\"Singapore\"}"},"finish_reason":"stop"}],
           "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"""

    /** SSE: tool-call start, one arguments delta violating the schema, finish. */
    private val weatherSse: String = buildString {
        append("""data: {"id":"1","model":"gpt-4o","choices":[{"delta":{"role":"assistant","tool_calls":""")
        append("""[{"index":0,"id":"call-1","type":"function","function":{"name":"weather","arguments":""}}]}}]}""")
        append("\n\n")
        append("""data: {"id":"1","model":"gpt-4o","choices":[{"delta":{"tool_calls":""")
        append("""[{"index":0,"function":{"arguments":"{\"town\":\"Singapore\"}"}}]}}]}""")
        append("\n\n")
        append("""data: {"id":"1","model":"gpt-4o","choices":[{"delta":{},"finish_reason":"tool_calls"}]}""")
        append("\n\ndata: [DONE]\n\n")
    }

    @Test
    fun `streaming replaces the tool-call part and leaves the deltas alone`() {
        server.contentType = "text/event-stream"
        server.responseBody = weatherSse

        model().use { model ->
            val parts = model.streamTextSequence(
                "weather in Singapore?",
                options { it.toolCall.copy(input = """{"city":"Singapore"}""") },
            ).toList()

            val call = parts.filterIsInstance<StreamPart.ToolCall>().single()
            assertThat(call.invalid).isNotEqualTo(true)
            assertThat(call.input.jsonObject["city"]?.jsonPrimitive?.content).isEqualTo("Singapore")

            // Deltas are the provider's text, forwarded untouched.
            val delta = parts.filterIsInstance<StreamPart.ToolInputDelta>().single()
            assertThat(delta.delta).isEqualTo("""{"town":"Singapore"}""")
        }
    }

    @Test
    fun `a streaming hook may call the SAME model while another thread closes it`() {
        // The lock is fair, so a reader that arrives after close() has queued
        // for the write lock waits for it. Without the lent read hold this is a
        // three-way deadlock: stream thread (holds read, waits for the hook) ←
        // closer (waits for the write lock) ← hook (waits for a read lock).
        server.contentType = "text/event-stream"
        server.setResponses(weatherSse, repairCompletion)

        val model = model()
        val hookStarted = CountDownLatch(1)
        val closer = Thread {
            hookStarted.await()
            model.close()
        }
        closer.isDaemon = true
        // assertTimeoutPreemptively, not @Timeout: a regression deadlocks, and
        // the default @Timeout mode only checks after the body returns.
        assertTimeoutPreemptively(Duration.ofSeconds(30)) {
            closer.start()
            val parts = model.streamTextSequence(
                "weather in Singapore?",
                options { context ->
                    // The stream response is already fully written; the nested
                    // completion is plain JSON off the same one-server queue.
                    server.contentType = "application/json"
                    hookStarted.countDown()
                    // Let close() reach the write lock and queue ahead of this
                    // thread's read acquisition — that ordering is the bug, and
                    // without the sleep the hook wins the race and proves
                    // nothing.
                    Thread.sleep(250)
                    val fixed = model.generateText("Fix ${context.toolCall.input}")
                    context.toolCall.copy(input = fixed.text)
                },
            ).toList()

            val call = parts.filterIsInstance<StreamPart.ToolCall>().single()
            assertThat(call.error).isNull()
            assertThat(call.invalid).isNotEqualTo(true)
            assertThat(call.input.jsonObject["city"]?.jsonPrimitive?.content).isEqualTo("Singapore")

            closer.join(TimeUnit.SECONDS.toMillis(10))
            assertThat(closer.isAlive).describedAs("close() never completed").isFalse()
        }
    }

}
