package ai.arcships.aimux

import com.sun.net.httpserver.HttpServer
import java.net.InetSocketAddress
import java.nio.file.Files
import java.nio.file.Paths
import java.util.concurrent.atomic.AtomicReference
import org.json.JSONObject
import org.junit.jupiter.api.Test
import org.junit.jupiter.api.Assertions.*

class DecisionModelTest {
    @Test fun providerDecisionModelOwnsItsHandle() {
        Model.createProvider("openai", "test-key").use { provider ->
            provider.decisionModel("gpt-6-luna").use { model ->
                provider.close()
                val description = JSONObject(model.describe())
                assertEquals("v4", description.getString("specificationVersion"))
                assertEquals("openai", description.getString("provider"))
                assertEquals("gpt-6-luna", description.getString("modelId"))
                assertThrows(IllegalStateException::class.java) { provider.decisionModel("gpt-6-luna") }
            }
        }
    }

    @Test fun officialStructuredContractAndDescription() {
        val fixture = JSONObject(String(Files.readAllBytes(Paths.get("../../contract-tests/fixtures/decision-native.json"))))
        val captured = AtomicReference<JSONObject>()
        val server = HttpServer.create(InetSocketAddress("127.0.0.1", 0), 0)
        server.createContext("/v1/systemone") { exchange ->
            captured.set(JSONObject(exchange.requestBody.bufferedReader().readText()))
            val bytes = fixture.getJSONObject("response").toString().toByteArray()
            exchange.responseHeaders.set("Content-Type", "application/json")
            exchange.sendResponseHeaders(200, bytes.size.toLong())
            exchange.responseBody.use { it.write(bytes) }
            exchange.close()
        }
        server.start()
        try {
            DecisionModel.jev("test-key", "jev-latest", "http://127.0.0.1:${server.address.port}/v1/systemone").use { model ->
                val description = JSONObject(model.describe())
                assertEquals("jev", description.getString("provider"))
                assertEquals(3, description.getJSONArray("supportedQuestionTypes").length())
                assertNull(captured.get())
                val request = fixture.getJSONObject("request")
                val result = JSONObject(model.decide(request.toString()))
                assertEquals(0.9, result.getJSONObject("answers").getJSONObject("urgent").getDouble("probability"))
                assertEquals(2, result.getJSONObject("rounding").getInt("probabilityDecimals"))
                assertTrue(JSONObject(mapOf("inputTokens" to 50, "outputTokens" to 10, "totalTokens" to 60))
                    .similar(result.getJSONObject("usage")))
                val questions = request.getJSONObject("questions")
                assertTrue(questions.getJSONObject("urgent").getJSONObject("criteria")
                    .similar(captured.get().getJSONObject("questions").getJSONObject("urgent").getJSONObject("criteria")))
                assertTrue(questions.getJSONObject("severity").getJSONArray("criteria")
                    .similar(captured.get().getJSONObject("questions").getJSONObject("severity").getJSONArray("criteria")))
                assertThrows(IllegalArgumentException::class.java) { model.decide("{") }
                model.close()
                assertThrows(IllegalStateException::class.java) { model.describe() }
            }
        } finally { server.stop(0) }
        assertThrows(AimuxException::class.java) { DecisionModel.jev("test-key", "jev-latest", probabilitySource = "unknown") }
    }

    @Test fun openaiMediaAndTypedValues() {
        val text = String(Files.readAllBytes(Paths.get("../../contract-tests/fixtures/decision-openai-full.json")))
        val fixture = JSONObject(text)
        val captured = AtomicReference<JSONObject>()
        val server = HttpServer.create(InetSocketAddress("127.0.0.1", 0), 0)
        server.createContext("/v1/decisions") { exchange ->
            captured.set(JSONObject(exchange.requestBody.bufferedReader().readText()))
            val bytes = fixture.getJSONObject("response").toString().toByteArray()
            exchange.responseHeaders.set("Content-Type", "application/json")
            exchange.sendResponseHeaders(200, bytes.size.toLong())
            exchange.responseBody.use { it.write(bytes) }
            exchange.close()
        }
        server.start()
        try {
            val config = JSONObject().put("base_url", "http://127.0.0.1:${server.address.port}/v1").toString()
            Model.createProvider("openai", "test-key", config).use { provider ->
                provider.decisionModel("gpt-6-luna").use { model ->
                    val result = JSONObject(model.decide(rawObject(text, "options")))
                    assertTrue(fixture.getJSONObject("request").similar(captured.get()))
                    assertEquals("boolean_true", result.getJSONObject("answers").getJSONObject("choice").getString("choice"))
                    assertEquals(true, result.getJSONObject("providerMetadata").getJSONObject("openai")
                        .getJSONObject("answers").getJSONObject("choice").get("value"))
                    assertEquals(0.75, result.getJSONObject("answers").getJSONObject("score").getDouble("score"))
                }
            }
        } finally { server.stop(0) }
    }

    /**
     * The raw text of the fixture's top-level `key` object. The questions and
     * criteria are maps whose order is the request's order, and org.json's
     * JSONObject does not keep key order.
     */
    private fun rawObject(text: String, key: String): String {
        val start = text.indexOf('{', text.indexOf("\"$key\":"))
        var depth = 0
        var inString = false
        var i = start
        while (true) {
            val c = text[i]
            if (inString) {
                if (c == '\\') i++ else if (c == '"') inString = false
            } else when (c) {
                '"' -> inString = true
                '{' -> depth++
                '}' -> if (--depth == 0) return text.substring(start, i + 1)
            }
            i++
        }
    }
}
