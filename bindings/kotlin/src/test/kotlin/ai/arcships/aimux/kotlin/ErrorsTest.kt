package ai.arcships.aimux.kotlin

import ai.arcships.aimux.KotlinInteropFixtures
import org.junit.jupiter.api.Assertions.*
import org.junit.jupiter.api.Test
import java.math.BigInteger

class ErrorsTest {
    // Compiled without else, so additions must update the exhaustive facade.
    private fun classify(error: AimuxException): String = when (error) {
        is NoOutputGeneratedError -> "NoOutputGeneratedError"
        is JSONParseError -> "JSONParseError"
        is InvalidResponseDataError -> "InvalidResponseDataError"
        is ToolCallNotFoundForApprovalError -> "ToolCallNotFoundForApprovalError"
        is InvalidArgumentError -> "InvalidArgumentError"
        is InvalidPromptError -> "InvalidPromptError"
        is LoadAPIKeyError -> "LoadAPIKeyError"
        is LoadSettingError -> "LoadSettingError"
        is TokenExpiredError -> "TokenExpiredError"
        is UnsupportedFunctionalityError -> "UnsupportedFunctionalityError"
        is NoSuchModelError -> "NoSuchModelError"
        is NoSuchProviderError -> "NoSuchProviderError"
        is APICallError -> "APICallError"
        is RetryError -> "RetryError"
        is TimeoutError -> "TimeoutError"
        is RequestAbortedError -> "RequestAbortedError"
        is NoSuchToolError -> "NoSuchToolError"
        is InvalidToolInputError -> "InvalidToolInputError"
        is ToolCallRepairError -> "ToolCallRepairError"
        is MissingToolResultsError -> "MissingToolResultsError"
        is ToolChoiceViolationError -> "ToolChoiceViolationError"
        is OtherError -> "OtherError"
        is BindingError -> "BindingError"
        is ProtocolError -> "ProtocolError"
        is RecordingError -> "RecordingError"
        is UnknownError -> "UnknownError"
    }

    @Test
    fun `every Java core subtype maps to an exhaustive sealed variant`() {
        val codes = listOf(18, 2, 3, 3, 5, 6, 19, 20, 7, 8, 9, 10, 11, 14, 12, 13, 15, 16, 17, 6, 3, 1)
        val variants = listOf(
            "NoOutputGeneratedError",
            "JSONParseError",
            "InvalidResponseDataError",
            "ToolCallNotFoundForApprovalError",
            "InvalidArgumentError",
            "InvalidPromptError",
            "LoadAPIKeyError",
            "LoadSettingError",
            "TokenExpiredError",
            "UnsupportedFunctionalityError",
            "NoSuchModelError",
            "NoSuchProviderError",
            "APICallError",
            "RetryError",
            "TimeoutError",
            "RequestAbortedError",
            "NoSuchToolError",
            "InvalidToolInputError",
            "ToolCallRepairError",
            "MissingToolResultsError",
            "ToolChoiceViolationError",
            "OtherError"
        )
        for ((name, code) in variants.zip(codes)) {
            val wireName = when (name) {
                "OtherError" -> "AI_Error"
                "TimeoutError" -> "TimeoutError"
                "RequestAbortedError" -> "AbortError"
                else -> "AI_$name"
            }
            val java = KotlinInteropFixtures.decode(
                """{"kind":"aimux","code":$code,"message":"错误😀","error":{"name":"$wireName"}}"""
            )
            val error = AimuxException.fromJava(java)
            assertEquals(name, classify(error))
            assertEquals(code, error.code)
            assertEquals("错误😀", error.message)
            assertSame(java, error.javaError)
            assertSame(java, error.cause)
            assertEquals(java.error, error.error)
        }
    }

    @Test
    fun `all Java error domains project without another decoder`() {
        for ((kind, code, name, variant) in listOf(
            listOf("binding", "203", "InvalidHandle", "BindingError"),
            listOf("protocol", "300", "InvalidOp", "ProtocolError"),
            listOf("recording", "105", "Write", "RecordingError"),
        )) {
            val java = KotlinInteropFixtures.decode(
                """{"kind":"$kind","code":$code,"message":"failed","error":{"name":"$name","extra":17}}"""
            )
            val error = AimuxException.fromJava(java)
            assertEquals(variant, classify(error))
            assertSame(java, error.javaError)
            assertEquals(java.envelope, error.envelope)
        }
    }

    @Test
    fun `open payload and unsigned retry hint survive unchanged`() {
        val java = KotlinInteropFixtures.decode("""{
            "kind":"aimux","code":11,"message":"limited","retryAfterMs":18446744073709551615,
            "error":{"name":"AI_APICallError","statusCode":429,"isRetryable":true,
            "requestBodyValues":{"model":"模型"},"responseHeaders":{"retry-after":"10"},
            "data":{"unknown":17}}
        }""")
        val error = AimuxException.fromJava(java) as APICallError
        assertSame(java, error.details)
        assertEquals(BigInteger("18446744073709551615"), error.retryAfterMs)
        assertTrue(error.details.isRetryable)
        assertEquals(429, error.details.statusCode)
        assertEquals(java.error, error.error)
        assertEquals(17, error.error.path("data").path("unknown").asInt())
    }

    @Test
    fun `future Java subtype retains its complete envelope`() {
        val java = KotlinInteropFixtures.unknown("""{
            "kind":"aimux","code":399,"message":"future","retryAfterMs":25,
            "error":{"name":"FutureError","extra":{"nested":true}}
        }""")
        val error = AimuxException.fromJava(java)
        assertEquals("UnknownError", classify(error))
        assertSame(java, error.javaError)
        assertEquals(java.envelope, error.envelope)
        assertEquals(BigInteger.valueOf(25), error.retryAfterMs)
    }

    @Test
    fun `nested retry payload is delegated without losing unknown fields`() {
        val java = KotlinInteropFixtures.decode("""{
            "kind":"aimux","code":14,"message":"failed","error":{"name":"AI_RetryError",
            "reason":"errorNotRetryable","errors":[{"name":"AI_APICallError","future":42}]}
        }""")
        val error = AimuxException.fromJava(java) as RetryError
        assertEquals(java.error, error.error)
        assertEquals(42, error.error.path("errors")[0].path("future").asInt())
    }
}
