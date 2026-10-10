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
                assertEquals(2, JSONObject(model.capabilities()).getInt("min_choices"))
                assertThrows(IllegalStateException::class.java) { provider.decisionModel("gpt-6-luna") }
            }
        }
    }

    @Test fun officialStructuredContractAndCapabilities() {
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
                val caps = JSONObject(model.capabilities())
                assertEquals(255, caps.getInt("max_choices"))
                assertEquals(2, caps.getJSONObject("rounding").getInt("probability_decimals"))
                assertNull(captured.get())
                val request = fixture.getJSONObject("request")
                val result = JSONObject(model.decide(request.toString()))
                assertEquals(0.9, result.getJSONObject("answers").getJSONObject("urgent").getDouble("probability_true"))
                assertTrue(request.getJSONArray("questions").getJSONObject(0).getJSONObject("criteria")
                    .similar(captured.get().getJSONObject("questions").getJSONObject("urgent").getJSONObject("criteria")))
                assertTrue(request.getJSONArray("questions").getJSONObject(2).getJSONArray("levels")
                    .similar(result.getJSONObject("answers").getJSONObject("severity").getJSONArray("levels")))
                assertThrows(IllegalArgumentException::class.java) { model.decide("{") }
                model.close()
                assertThrows(IllegalStateException::class.java) { model.capabilities() }
            }
        } finally { server.stop(0) }
        assertThrows(AimuxException::class.java) { DecisionModel.jev("test-key", "jev-latest", probabilitySource = "unknown") }
    }

    @Test fun openaiMediaAndTypedValues() {
        val fixture = JSONObject(String(Files.readAllBytes(Paths.get("../../contract-tests/fixtures/decision-openai-full.json"))))
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
                    val result = JSONObject(model.decide(fixture.getJSONObject("options").toString()))
                    assertTrue(fixture.getJSONObject("request").similar(captured.get()))
                    assertEquals(true, result.getJSONObject("answers").getJSONObject("choice").get("value"))
                    assertTrue(fixture.getJSONObject("options").getJSONArray("questions").getJSONObject(1).getJSONArray("levels")
                        .similar(result.getJSONObject("answers").getJSONObject("score").getJSONArray("levels")))
                }
            }
        } finally { server.stop(0) }
    }
}
