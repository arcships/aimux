package ai.arcships.aimux

import com.sun.net.httpserver.HttpExchange
import com.sun.net.httpserver.HttpServer
import org.assertj.core.api.Assertions.assertThat
import org.json.JSONArray
import org.json.JSONObject
import org.junit.jupiter.api.AfterEach
import org.junit.jupiter.api.BeforeEach
import org.junit.jupiter.api.Test
import java.net.InetSocketAddress
import java.nio.charset.StandardCharsets

// ─────────────────────────────────────────────────────────────────────────────
// Structured end-to-end tests for the Kotlin/JVM binding.
//
// These spin up a local mock HTTP server speaking the OpenAI chat-completions
// wire format, point an OpenAI `Model` at it via the `base_url` constructor,
// and assert that the binding correctly:
//   1. parses tool_calls out of a provider response,
//   2. forwards multi-role messages,
//   3. forwards `tool_choice`,
//   4. parses tool-call stream parts from an SSE stream.
//
// No real network access is performed — every request hits 127.0.0.1.
// ─────────────────────────────────────────────────────────────────────────────

/// A tiny mock HTTP server that records the last request body and replays a
/// preset response. Listens on a random loopback port.
internal class MockProviderServer {
    private val server: HttpServer =
        HttpServer.create(InetSocketAddress("127.0.0.1", 0), 0)

    /** Last received request body (raw UTF-8 string). */
    @Volatile
    var lastRequestBody: String = ""
        private set

    /** Last received request method (e.g. "POST"). */
    @Volatile
    var lastRequestMethod: String = ""
        private set

    /** Status code to return. */
    var status: Int = 200

    /** Value of the Content-Type response header. */
    var contentType: String = "application/json"

    /** Body bytes to return (single-response mode; used when the queue is empty). */
    var responseBody: String = ""

    /** Response queue — each request pops the next response (FIFO). */
    private val responseQueue = ArrayDeque<String>()

    /** Set the list of responses to return (FIFO). */
    fun setResponses(vararg responses: String) {
        synchronized(responseQueue) {
            responseQueue.clear()
            responses.forEach { responseQueue.addLast(it) }
        }
    }

    val port: Int
        get() = server.address.port

    val baseUrl: String
        get() = "http://127.0.0.1:$port"

    init {
        server.createContext("/") { exchange: HttpExchange ->
            handle(exchange)
        }
        server.start()
    }

    private fun handle(exchange: HttpExchange) {
        // Capture the inbound request body for assertions.
        val reqBytes = exchange.requestBody.readBytes()
        lastRequestBody = String(reqBytes, StandardCharsets.UTF_8)
        lastRequestMethod = exchange.requestMethod

        // Pop the next queued response if any, otherwise fall back to the
        // single-response field (backward compatible with the existing tests).
        val resp = synchronized(responseQueue) {
            if (responseQueue.isNotEmpty()) responseQueue.removeFirst() else responseBody
        }
        val respBytes = resp.toByteArray(StandardCharsets.UTF_8)
        exchange.responseHeaders.add("Content-Type", contentType)
        // SSE streams use chunked transfer encoding (response length 0) so the
        // client's streaming reader observes a proper end-of-stream. A fixed
        // content length can leave the streaming reader blocked waiting for more
        // data. Plain JSON responses use a fixed length.
        val responseLength =
            if (contentType.contains("text/event-stream")) 0L else respBytes.size.toLong()
        exchange.sendResponseHeaders(status, responseLength)
        exchange.responseBody.use {
            it.write(respBytes)
            it.flush()
        }
        exchange.close()
    }

    fun stop() = server.stop(0)
}

class StructuredE2ETest {

    private lateinit var server: MockProviderServer

    @BeforeEach
    fun setUp() {
        server = MockProviderServer()
    }

    @AfterEach
    fun tearDown() {
        server.stop()
    }

    // ── canned OpenAI responses ───────────────────────────────────────────

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

    /** A `tools` option describing a single get_weather function tool. */
    private fun weatherToolsOpts(includeToolChoice: String? = null): String =
        JSONObject().apply {
            put(
                "tools",
                JSONArray().put(
                    JSONObject().apply {
                        put("type", "function")
                        put("name", "get_weather")
                        put(
                            "input_schema",
                            JSONObject().apply {
                                put("type", "object")
                                put(
                                    "properties",
                                    JSONObject().apply {
                                        put("location", JSONObject().put("type", "string"))
                                    },
                                )
                            },
                        )
                    },
                ),
            )
            if (includeToolChoice != null) {
                put("tool_choice", includeToolChoice)
            }
        }.toString()

    // ── Tests ─────────────────────────────────────────────────────────────

    @Test
    fun `generateText parses tool_calls`() {
        server.responseBody = toolCallOpenAiResponse

        Model.openai("sk-test-fake-key", "gpt-4o", server.baseUrl).use { model ->
            val resultJson = model.generateText("\"What is the weather in Tokyo?\"", weatherToolsOpts())
            val result = JSONObject(resultJson)

            // tool_calls[0].tool_name == "get_weather"
            val toolCalls = result.getJSONArray("tool_calls")
            assertThat(toolCalls.length()).isGreaterThan(0)
            val firstCall = toolCalls.getJSONObject(0)
            assertThat(firstCall.getString("tool_name")).isEqualTo("get_weather")
            assertThat(firstCall.getString("tool_call_id")).isEqualTo("call_abc")
            assertThat(firstCall.getJSONObject("input").getString("location")).isEqualTo("Tokyo")

            // raw.content contains a "ToolCall" variant.
            val rawContent = result.getJSONObject("raw").getJSONArray("content")
            val hasToolCallVariant = (0 until rawContent.length()).any {
                rawContent.getJSONObject(it).has("ToolCall")
            }
            assertThat(hasToolCallVariant).isTrue()
        }
    }

    @Test
    fun `multi-role messages reach provider`() {
        server.responseBody = plainOpenAiResponse

        // [{role:"system",content:"You are a helpful assistant."},
        //  {role:"user",content:"What is Rust?"}]
        val promptJson = JSONArray().apply {
            put(JSONObject().apply {
                put("role", "system")
                put("content", "You are a helpful assistant.")
            })
            put(JSONObject().apply {
                put("role", "user")
                put("content", "What is Rust?")
            })
        }.toString()

        Model.openai("sk-test-fake-key", "gpt-4o", server.baseUrl).use { model ->
            val resultJson = model.generateText(promptJson)
            val result = JSONObject(resultJson)
            assertThat(result.getString("text"))
                .isEqualTo("Rust is a systems programming language.")

            // The provider received both messages (system + user).
            val reqBody = JSONObject(server.lastRequestBody)
            assertThat(reqBody.getString("model")).isEqualTo("gpt-4o")
            val messages = reqBody.getJSONArray("messages")
            assertThat(messages.length()).isEqualTo(2)
            assertThat(messages.getJSONObject(0).getString("role")).isEqualTo("system")
            assertThat(messages.getJSONObject(1).getString("role")).isEqualTo("user")
        }
    }

    @Test
    fun `tool_choice reaches provider`() {
        server.responseBody = toolCallOpenAiResponse

        Model.openai("sk-test-fake-key", "gpt-4o", server.baseUrl).use { model ->
            model.generateText("\"What is the weather in Tokyo?\"", weatherToolsOpts(includeToolChoice = "required"))

            val reqBody = JSONObject(server.lastRequestBody)
            assertThat(reqBody.getString("tool_choice")).isEqualTo("required")
            // tools were forwarded too.
            assertThat(reqBody.has("tools")).isTrue()
        }
    }

    @Test
    fun `streamText parses tool-call stream parts`() {
        // SSE: tool_calls delta (name) -> arguments delta -> finish_reason.
        server.contentType = "text/event-stream"
        server.responseBody = buildString {
            append("data: ")
            append(
                JSONObject().apply {
                    put("id", "1")
                    put("model", "gpt-4o")
                    put(
                        "choices",
                        JSONArray().put(
                            JSONObject().apply {
                                put(
                                    "delta",
                                    JSONObject().apply {
                                        put("role", "assistant")
                                        put(
                                            "tool_calls",
                                            JSONArray().put(
                                                JSONObject().apply {
                                                    put("index", 0)
                                                    put("id", "call_xyz")
                                                    put("type", "function")
                                                    put(
                                                        "function",
                                                        JSONObject().apply {
                                                            put("name", "get_weather")
                                                            put("arguments", "")
                                                        },
                                                    )
                                                },
                                            ),
                                        )
                                    },
                                )
                            },
                        ),
                    )
                }.toString(),
            )
            append("\n\n")
            append("data: ")
            append(
                JSONObject().apply {
                    put("id", "1")
                    put("model", "gpt-4o")
                    put(
                        "choices",
                        JSONArray().put(
                            JSONObject().apply {
                                put(
                                    "delta",
                                    JSONObject().apply {
                                        put(
                                            "tool_calls",
                                            JSONArray().put(
                                                JSONObject().apply {
                                                    put("index", 0)
                                                    put(
                                                        "function",
                                                        JSONObject().apply {
                                                            put("arguments", "{\"location\":\"Tokyo\"}")
                                                        },
                                                    )
                                                },
                                            ),
                                        )
                                    },
                                )
                            },
                        ),
                    )
                }.toString(),
            )
            append("\n\n")
            append("data: ")
            append(
                JSONObject().apply {
                    put("id", "1")
                    put("model", "gpt-4o")
                    put(
                        "choices",
                        JSONArray().put(
                            JSONObject().apply {
                                put("delta", JSONObject())
                                put("finish_reason", "tool_calls")
                            },
                        ),
                    )
                    put(
                        "usage",
                        JSONObject().apply {
                            put("prompt_tokens", 5)
                            put("completion_tokens", 2)
                            put("total_tokens", 7)
                        },
                    )
                }.toString(),
            )
            append("\n\n")
            append("data: [DONE]\n\n")
        }

        Model.openai("sk-test-fake-key", "gpt-4o", server.baseUrl).use { model ->
            // Run the blocking FFI stream call on a separate thread so the JNA
            // callbacks (which attach to the calling thread) don't deadlock with
            // tokio's block_on. The main thread collects parts via a queue.
            val parts = java.util.concurrent.LinkedBlockingQueue<String?>()
            val streamThread = Thread {
                model.streamText(
                    "\"What is the weather in Tokyo?\"",
                    weatherToolsOpts(),
                    onPart = { parts.put(it) },
                    onDone = { parts.put(null) },
                )
            }
            streamThread.isDaemon = true
            streamThread.start()

            val collected = mutableListOf<String>()
            while (true) {
                val part = parts.poll(30, java.util.concurrent.TimeUnit.SECONDS) ?: break
                collected.add(part)
            }
            streamThread.join(5000)

            assertThat(collected).isNotEmpty()
            // Stream must surface tool-call activity: either a complete ToolCall
            // part or ToolInputDelta deltas (or both).
            val hasToolPart = collected.any {
                it.contains("\"ToolCall\"") || it.contains("\"ToolInputDelta\"")
            }
            assertThat(hasToolPart)
                .`as`("expected a ToolCall or ToolInputDelta stream part; got: $collected")
                .isTrue()

            // The finish part should also be present, carrying the tool-calls
            // finish reason.
            val hasFinish = collected.any { it.contains("\"Finish\"") }
            assertThat(hasFinish).isTrue()
        }
    }

    @Test
    fun `tool-call full round-trip`() {
        // A tool-call round-trip = two generateText calls with a ToolResult
        // back-filled between them. The mock server returns a tool-call
        // response for the first request, then a plain text response for the
        // second (FIFO queue).
        val finalTextResponse = JSONObject().apply {
            put("id", "chatcmpl-2")
            put("model", "gpt-4o")
            put(
                "choices",
                JSONArray().put(
                    JSONObject().apply {
                        put(
                            "message",
                            JSONObject().apply {
                                put("role", "assistant")
                                put("content", "The weather in Tokyo is sunny.")
                            },
                        )
                        put("finish_reason", "stop")
                    },
                ),
            )
            put(
                "usage",
                JSONObject().apply {
                    put("prompt_tokens", 30)
                    put("completion_tokens", 8)
                    put("total_tokens", 38)
                },
            )
        }.toString()

        server.setResponses(toolCallOpenAiResponse, finalTextResponse)

        Model.openai("sk-test-fake-key", "gpt-4o", server.baseUrl).use { model ->
            // 1st call: provider asks to call get_weather.
            val firstResultJson =
                model.generateText("\"What's the weather in Tokyo?\"", weatherToolsOpts())
            val firstResult = JSONObject(firstResultJson)
            val toolCalls = firstResult.getJSONArray("tool_calls")
            assertThat(toolCalls.length()).isGreaterThan(0)
            val firstCall = toolCalls.getJSONObject(0)
            assertThat(firstCall.getString("tool_name")).isEqualTo("get_weather")
            assertThat(firstCall.getString("tool_call_id")).isEqualTo("call_abc")
            assertThat(firstCall.getJSONObject("input").getString("location"))
                .isEqualTo("Tokyo")

            // 2nd call: echo the full conversation back, including the
            // assistant's tool_call and the ToolResult we synthesised.
            // Input uses the engine's ContentPart variants (tool_call /
            // tool_result); the engine converts these to the OpenAI wire
            // format on the outbound request.
            val messages = JSONArray().apply {
                put(JSONObject().apply {
                    put("role", "user")
                    put("content", "What's the weather in Tokyo?")
                })
                put(JSONObject().apply {
                    put("role", "assistant")
                    put(
                        "content",
                        JSONArray().put(
                            JSONObject().apply {
                                put("type", "tool_call")
                                put("tool_call_id", "call_abc")
                                put("tool_name", "get_weather")
                                put("input", JSONObject().put("location", "Tokyo"))
                            },
                        ),
                    )
                })
                put(JSONObject().apply {
                    put("role", "tool")
                    put(
                        "content",
                        JSONArray().put(
                            JSONObject().apply {
                                put("type", "tool_result")
                                put("tool_call_id", "call_abc")
                                put(
                                    "output",
                                    JSONObject().put("temperature", 22).put("condition", "sunny"),
                                )
                            },
                        ),
                    )
                })
            }.toString()

            val secondResultJson = model.generateText(messages, weatherToolsOpts())
            val secondResult = JSONObject(secondResultJson)
            assertThat(secondResult.getString("text"))
                .isEqualTo("The weather in Tokyo is sunny.")

            // The provider received 3 messages; the last is the tool result,
            // carrying the matching tool_call_id.
            val reqBody = JSONObject(server.lastRequestBody)
            val reqMessages = reqBody.getJSONArray("messages")
            assertThat(reqMessages.length()).isEqualTo(3)
            val lastMsg = reqMessages.getJSONObject(2)
            assertThat(lastMsg.getString("role")).isEqualTo("tool")
            assertThat(lastMsg.getString("tool_call_id")).isEqualTo("call_abc")
        }
    }
}
