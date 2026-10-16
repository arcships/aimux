package ai.arcships.aimux.kotlin

import ai.arcships.aimux.Aimux
import ai.arcships.aimux.AimuxException as JavaAimuxException
import ai.arcships.aimux.KotlinInteropFixtures
import ai.arcships.aimux.types.StreamFrame
import ai.arcships.aimux.types.Outcome
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.flow.collect
import kotlinx.coroutines.flow.take
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.jupiter.api.Assertions.*
import org.junit.jupiter.api.Test
import java.util.concurrent.CompletableFuture

/** Uses the Rust cdylib through Java, without a fixture native implementation. */
class NativeIntegrationTest {
    private fun client(): Aimux = Aimux.load(
        System.getProperty("aimux.library") ?: System.getenv("AIMUX_LIBRARY_PATH") ?: "aimux_ffi"
    )

    private val providerParams = """{"package":"openai","settings":{"apiKey":"test-key"}}"""

    @Test
    fun `real manifest loads and Java provider model objects retain identity`() = runBlocking {
        val aimux = client()
        assertEquals(65536, aimux.manifest.path("abiVersion").asInt())
        assertEquals("1.0", aimux.manifest.path("opsVersion").asText())
        val provider = aimux.createProvider(providerParams)
        try {
            val model = provider.languageModel("test-model")
            try {
                assertSame(provider, KotlinInteropFixtures.acceptProvider(
                    aimuxCall { CompletableFuture.completedFuture(provider) }
                ))
                assertSame(model, KotlinInteropFixtures.acceptModel(
                    aimuxCall { CompletableFuture.completedFuture(model) }
                ))
                assertEquals("test-model", model.info().path("modelId").asText())
            } finally { model.close() }
        } finally { provider.close() }
    }

    @Test
    fun `Decision remains reachable through Java generic resources`() = runBlocking {
        client().createProvider("""{"package":"jev","settings":{"apiKey":"test-key"}}""").use { provider ->
            provider.create("provider.model", """{"method":"decision","modelId":"test-model"}""").use { decision ->
                val capabilities = aimuxCall { executor ->
                    decision.callAsync("decision.capabilities", "{}", executor)
                }
                assertTrue(capabilities.isObject)
                assertEquals("test-model", decision.call("model.info", "{}").path("modelId").asText())
            }
        }
    }

    @Test
    fun `Java Abort reaches the real native stream`() = runBlocking {
        val aimux = client()
        aimux.createProvider(providerParams).use { provider ->
            provider.languageModel("test-model").use { model ->
                aimux.createAbort().use { abort ->
                    abort.signal()
                    val frames = mutableListOf<StreamFrame>()
                    try {
                        withTimeout(5000) {
                            aimuxFlow<StreamFrame> { executor, consumer ->
                                val operation = CompletableFuture<Unit>()
                                executor.execute {
                                    try {
                                        model.stream("text.stream", """{"prompt":"hello"}""", abort, consumer)
                                        operation.complete(Unit)
                                    } catch (error: Throwable) { operation.completeExceptionally(error) }
                                }
                                operation
                            }.collect { frames.add(it) }
                        }
                        fail("expected native cancellation")
                    } catch (expected: CancellationException) {
                        val source = assertInstanceOf(
                            JavaAimuxException.RequestAbortedError::class.java,
                            generateSequence<Throwable>(expected) { it.cause }.last(),
                        )
                        assertTrue(frames.single().isEnd)
                        assertEquals(Outcome.CANCELLED, frames.single().outcome)
                        assertEquals(source.envelope, frames.single().error.toJson())
                    }
                }
            }
        }
    }

    @Test
    fun `real protocol envelope maps through Java async Future`() = runBlocking {
        val aimux = client()
        try {
            aimuxCall { executor -> aimux.callAsync("missing.kotlin.op", "{}", executor) }
            fail("expected invalid operation")
        } catch (error: ProtocolError) {
            assertEquals(300, error.code)
            assertEquals("InvalidOp", error.errorName)
            assertEquals(error.javaError.envelope, error.envelope)
        }
    }

    @Test
    fun `real binding envelope preserves an unknown handle`() = runBlocking {
        val resource = client().own("18446744073709551615")
        try {
            try {
                aimuxCall { executor -> resource.callAsync("model.info", "{}", executor) }
                fail("expected invalid handle")
            } catch (error: BindingError) {
                assertEquals(203, error.code)
                assertEquals("InvalidHandle", error.errorName)
            }
        } finally { resource.close(); resource.close() }
    }

    @Test
    fun `real stream rejects invalid controls before any frame`() = runBlocking {
        client().createProvider(providerParams).use { provider ->
            provider.languageModel("test-model").use { model ->
                val values = mutableListOf<StreamFrame>()
                try {
                    withTimeout(5000) {
                        aimuxFlow<StreamFrame>(1) { executor, consumer ->
                            model.streamAsync("{}", consumer, executor)
                        }.collect { values.add(it) }
                    }
                    fail("expected invalid params")
                } catch (error: ProtocolError) {
                    assertEquals(301, error.code)
                }
                assertTrue(values.isEmpty())
            }
        }
    }

    @Test
    fun `accepted stream drains terminal envelope and take one returns`() = runBlocking {
        client().createProvider(providerParams).use { provider ->
            provider.languageModel("test-model").use { model ->
                // An empty prompt fails core validation before provider I/O.
                val params = """{"prompt":[],"options":{"maxRetries":0}}"""
                val values = mutableListOf<StreamFrame>()
                try {
                    withTimeout(5000) {
                        aimuxFlow<StreamFrame>(1) { executor, consumer ->
                            model.streamAsync(params, consumer, executor)
                        }.collect { values.add(it) }
                    }
                    fail("expected invalid prompt")
                } catch (error: InvalidPromptError) {
                    assertTrue(values.last().isEnd)
                    assertEquals(Outcome.ERROR, values.last().outcome)
                    assertEquals(error.envelope, values.last().error.toJson())
                }
                val first = withTimeout(5000) {
                    aimuxFlow<StreamFrame>(1) { executor, consumer ->
                        model.streamAsync(params, consumer, executor)
                    }.take(1).toList()
                }
                assertEquals(1, first.size)
                assertTrue(first.single().isEnd)
            }
        }
    }
}
