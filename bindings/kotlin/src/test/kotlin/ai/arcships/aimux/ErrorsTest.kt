package ai.arcships.aimux

import com.sun.jna.ptr.LongByReference
import com.sun.jna.ptr.PointerByReference
import org.assertj.core.api.Assertions.assertThat
import org.assertj.core.api.Assertions.assertThatThrownBy
import org.junit.jupiter.api.Test

/**
 * Tests for the error decoders: real `aimux_error_t *` objects obtained
 * offline (C ABI codes 200..206 → plain IllegalStateException; core failures
 * → [AimuxException] via [expectAimuxError]) and the pure
 * code → class mapping ([AimuxException.createByCode]) for shapes that need a
 * network to produce.
 */
class ErrorsTest {

    /** Dead handle via raw FFI: code 203 and message → IllegalStateException, then freed. */
    @Test
    fun `dead handle is an IllegalStateException, not an AiMuxError`() {
        val out = PointerByReference()
        val e = FFI.lib.aimux_generate_text(0x7FFF_FFFFL, "\"hi\"", null, out)
        assertThat(e).isNotNull
        assertThat(out.value).isNull()
        assertThat(FFI.lib.aimux_error_code(e)).isEqualTo(203)
        assertThat(takeString(FFI.lib.aimux_error_message(e))).isNotEmpty()
        assertThatThrownBy { throw expectAimuxError(e!!, "generateText") }
            .isInstanceOf(IllegalStateException::class.java)
            .isNotInstanceOf(AimuxException::class.java)
            .hasMessageContaining("aimux ffi:")
            .hasMessageContaining("model")
    }

    /** NULL required string (unreachable through the binding) → invariant naming the parameter. */
    @Test
    fun `null string argument is an IllegalStateException naming the parameter`() {
        val out = LongByReference(0)
        val e = FFI.lib.aimux_openai_new(null, "gpt-4o-mini", out)
        assertThat(e).isNotNull
        assertThat(out.value).isEqualTo(0L)
        assertThat(FFI.lib.aimux_error_code(e)).isEqualTo(200)
        assertThatThrownBy { throw expectAimuxError(e!!) }
            .isInstanceOf(IllegalStateException::class.java)
            .isNotInstanceOf(AimuxException::class.java)
            .hasMessageContaining("aimux ffi:")
            .hasMessageContaining("api_key")
    }

    /** Malformed JSON text that reaches C → invariant (the public API rejects it before C). */
    @Test
    fun `malformed wire JSON is a C ABI failure naming the parameter`() {
        val e = FFI.lib.aimux_register_providers("{not json")
        assertThat(e).isNotNull
        assertThat(FFI.lib.aimux_error_code(e)).isEqualTo(202)
        assertThatThrownBy { throw expectAimuxError(e!!) }
            .isInstanceOf(IllegalStateException::class.java)
            .isNotInstanceOf(AimuxException::class.java)
            .hasMessageContaining("config_json")
        // The public API never lets it reach C.
        assertThatThrownBy { Model.registerProviders("{not json") }
            .isInstanceOf(IllegalArgumentException::class.java)
            .hasMessageContaining("configJson")
    }

    /** A C-ABI-only utility decodes through expectFfiError. */
    @Test
    fun `expectFfiError reads the message and frees`() {
        val e = FFI.lib.aimux_transcription_input_done(0x7FFF_FFFFL)
        assertThat(e).isNotNull
        assertThatThrownBy { throw expectFfiError(e!!, "inputDone") }
            .isInstanceOf(IllegalStateException::class.java)
            .hasMessageStartingWith("inputDone: aimux ffi:")
    }

    /** The binding rejects malformed raw JSON before the C call, naming the Kotlin parameter. */
    @Test
    fun `requireJson rejects malformed text and passes null, empty and valid JSON`() {
        assertThatThrownBy { requireJsonRequired("promptJson", "{not json") }
            .isInstanceOf(IllegalArgumentException::class.java)
            .hasMessageContaining("promptJson")
        // Optional (String?) overload: null / empty are left to the FFI.
        val optionalEmpty: String? = ""
        requireJson("optsJson", null)
        requireJson("configJson", optionalEmpty)
        // Required (String) overload: blank is rejected up front.
        assertThatThrownBy { requireJsonRequired("promptJson", "") }
            .isInstanceOf(IllegalArgumentException::class.java)
            .hasMessage("promptJson: invalid JSON: empty")
        assertThatThrownBy { requireJsonRequired("promptJson", "  ") }
            .isInstanceOf(IllegalArgumentException::class.java)
        requireJsonRequired("promptJson", "\"hi\"")
        requireJson("optsJson", "{\"a\":[1,2]}")
    }

    /** Core-originated failure: the NoSuchProvider payload is available under code 10. */
    @Test
    fun `expectAimuxError maps the provider id onto NoSuchProviderError`() {
        val out = LongByReference(0)
        val e = FFI.lib.aimux_provider_new("definitely-not-a-provider", "k", "m", null, out)
        assertThat(e).isNotNull
        assertThat(out.value).isEqualTo(0L)
        assertThat(FFI.lib.aimux_error_code(e)).isEqualTo(AIMUX_E_NO_SUCH_PROVIDER)
        val ex = expectAimuxError(e!!, "provider") as NoSuchProviderError
        assertThat(ex.providerId).isEqualTo("definitely-not-a-provider")
        assertThat(ex.code).isEqualTo(AIMUX_E_NO_SUCH_PROVIDER)
        assertThat(ex.message).startsWith("provider: ")
    }

    /** A recording call returns the recording code range. */
    @Test
    fun `expectRecordingError maps the recording code`() {
        // Directory path that cannot be created (a file stands in the way).
        val f = java.io.File.createTempFile("aimux-rec", ".tmp").apply { deleteOnExit() }
        val e = FFI.lib.aimux_init_recording(java.io.File(f, "sub").path)
        assertThat(e).isNotNull
        assertThat(FFI.lib.aimux_error_code(e)).isEqualTo(100)
        val ex = expectRecordingError(e!!, "initRecording") as RecordingException
        assertThat(ex.code).isEqualTo(RecordingErrorCode.INIT)
        assertThat(ex.message).startsWith("initRecording: ")
    }

    /** Each payload lands on the class its code owns and nowhere else. */
    @Test
    fun `createByCode maps per-code payloads onto the owning subclass`() {
        val apiEx = AimuxException.createByCode(
            AIMUX_E_API_CALL, "API call error: HTTP 429: slow down", 429, 1500,
            retryable = true, providerCode = "insufficient_quota", providerMessage = "slow down",
            requestId = "req_123", responseBody = "{\"error\":{}}",
        ) as APICallError
        assertThat(apiEx.status).isEqualTo(429)
        assertThat(apiEx.retryMs).isEqualTo(1500L)
        assertThat(apiEx.retryable).isTrue()
        assertThat(apiEx.providerCode).isEqualTo("insufficient_quota")
        assertThat(apiEx.providerMessage).isEqualTo("slow down")
        assertThat(apiEx.requestId).isEqualTo("req_123")
        assertThat(apiEx.responseBody).isEqualTo("{\"error\":{}}")

        val modelEx = AimuxException.createByCode(
            AIMUX_E_NO_SUCH_MODEL, "no such model", modelId = "gpt-nope", modelType = "language",
        ) as NoSuchModelError
        assertThat(modelEx.modelId).isEqualTo("gpt-nope")
        assertThat(modelEx.modelType).isEqualTo("language")

        // Absent: null on APICallError, "" on the id classes.
        assertThat((AimuxException.createByCode(AIMUX_E_API_CALL, "x") as APICallError).requestId).isNull()
        assertThat((AimuxException.createByCode(AIMUX_E_NO_SUCH_MODEL, "x") as NoSuchModelError).modelId).isEmpty()
        assertThat((AimuxException.createByCode(AIMUX_E_NO_SUCH_PROVIDER, "x") as NoSuchProviderError).providerId).isEmpty()
    }

    /** 401 / 404 are the same class; only the status distinguishes them. */
    @Test
    fun `createByCode maps HTTP statuses onto APICallError`() {
        val auth = AimuxException.createByCode(AIMUX_E_API_CALL, "invalid api key", 401)
        assertThat(auth).isInstanceOf(APICallError::class.java)
        assertThat(auth.status).isEqualTo(401)
        assertThat(auth.retryMs).isEqualTo(-1L)
        assertThat(AimuxException.createByCode(AIMUX_E_API_CALL, "nope", 404).status).isEqualTo(404)
    }

    /** TokenExpired keeps its own class: 401 whose fix is a refresh, not a retry. */
    @Test
    fun `createByCode maps TokenExpired to status 401`() {
        val ex = AimuxException.createByCode(AIMUX_E_TOKEN_EXPIRED, "expired")
        assertThat(ex).isInstanceOf(TokenExpiredError::class.java)
        assertThat(ex.status).isEqualTo(401)
    }

    /** Same status (-1), opposite verdicts: status cannot stand in for retryable. */
    @Test
    fun `createByCode carries the retry verdict that status cannot express`() {
        val transport = AimuxException.createByCode(AIMUX_E_API_CALL, "reset", retryable = true)
        val noKey = AimuxException.createByCode(AIMUX_E_API_CALL, "no key")
        assertThat(transport.status).isEqualTo(noKey.status)
        assertThat(transport.retryable).isTrue()
        assertThat(noKey.retryable).isFalse()
    }

    /** A code outside 1..13 is a header/library mismatch, not an error type. */
    @Test
    fun `createByCode rejects codes outside the enum`() {
        assertThatThrownBy { AimuxException.createByCode(999, "?") }
            .isInstanceOf(IllegalStateException::class.java)
        assertThatThrownBy { AimuxException.createByCode(AIMUX_OK, "?") }
            .isInstanceOf(IllegalStateException::class.java)
    }

    @Test
    fun `RecordingException is not an AimuxException`() {
        val rec = RecordingException(RecordingErrorCode.FLUSH_TIMEOUT, "flush")
        assertThat(rec).isNotInstanceOf(AimuxException::class.java)
        assertThat(rec.code).isEqualTo(RecordingErrorCode.FLUSH_TIMEOUT)
        assertThat(rec.message).isEqualTo("flush")
    }

    @Test
    fun `recording codes outside the Rust enum are rejected`() {
        assertThatThrownBy { RecordingErrorCode.fromC(999) }
            .isInstanceOf(IllegalStateException::class.java)
    }
}
