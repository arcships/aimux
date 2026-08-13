package ai.arcships.aimux

import com.sun.jna.Memory
import org.assertj.core.api.Assertions.assertThat
import org.junit.jupiter.api.Test

/**
 * Pure-JVM tests for the C error struct layout and [AimuxException.fromC].
 * No aimux native library is loaded: [AimuxCError] fields are hand-filled,
 * and fromC is a pure mapping that never frees (the FFI call sites own the
 * message allocation), so Memory-backed messages are safe here.
 */
class ErrorsTest {

    @Test
    fun `AimuxCError matches the 40-byte C layout`() {
        assertThat(AimuxCError().size()).isEqualTo(40)
    }

    @Test
    fun `fromC maps ApiCall with null message to fallback and keeps the reported status`() {
        val err = AimuxCError()
        err.code = AIMUX_E_API_CALL
        err.status = 429
        val ex = AimuxException.fromC(err)
        assertThat(ex).isInstanceOf(APICallError::class.java)
        assertThat(ex.status).isEqualTo(429)
        assertThat(ex.message).isEqualTo("aimux: ApiCall")
        assertThat(ex.errorValue).isNull()
    }

    @Test
    fun `fromC reads a NUL-terminated message and carries status, retry_ms and error_value`() {
        val json =
            """{"ApiCall":{"status_code":429,"message":"too many requests","retry_after_ms":1500,"is_retryable":true}}"""
        val mem = Memory(64)
        mem.setString(0, "API call error: HTTP 429: too many requests", "UTF-8")
        val valueMem = Memory(256)
        valueMem.setString(0, json, "UTF-8")
        val err = AimuxCError()
        err.code = AIMUX_E_API_CALL
        err.status = 429
        err.retry_ms = 1500
        err.message = mem
        err.error_value = valueMem
        val ex = AimuxException.fromC(err)
        assertThat(ex).isInstanceOf(APICallError::class.java)
        assertThat(ex.message).isEqualTo("API call error: HTTP 429: too many requests")
        assertThat(ex.status).isEqualTo(429)
        assertThat(ex.retryMs).isEqualTo(1500L)
        assertThat(ex.errorValue).isEqualTo(json)
        // fromC did not free: both Memory blocks are still valid and unchanged.
        assertThat(mem.getString(0, "UTF-8")).isEqualTo("API call error: HTTP 429: too many requests")
        assertThat(valueMem.getString(0, "UTF-8")).isEqualTo(json)
    }

    /** A 401 is the same class; only the status distinguishes it. */
    @Test
    fun `fromC maps a 401 to APICallError with status 401`() {
        val mem = Memory(64)
        mem.setString(0, "API call error: HTTP 401: invalid api key", "UTF-8")
        val err = AimuxCError()
        err.code = AIMUX_E_API_CALL
        err.status = 401
        err.message = mem
        val ex = AimuxException.fromC(err)
        assertThat(ex).isInstanceOf(APICallError::class.java)
        assertThat(ex.status).isEqualTo(401)
        assertThat(ex.retryMs).isEqualTo(-1L)
    }

    /** A 404 no longer has a class of its own either. */
    @Test
    fun `fromC maps a 404 to APICallError with status 404`() {
        val err = AimuxCError()
        err.code = AIMUX_E_API_CALL
        err.status = 404
        val ex = AimuxException.fromC(err)
        assertThat(ex).isInstanceOf(APICallError::class.java)
        assertThat(ex.status).isEqualTo(404)
    }

    /** Transport failure: no response, so no status — retryability lives in error_value. */
    @Test
    fun `fromC maps a statusless ApiCall transport failure`() {
        val json = """{"ApiCall":{"message":"connection reset","is_retryable":true}}"""
        val valueMem = Memory(128)
        valueMem.setString(0, json, "UTF-8")
        val err = AimuxCError()
        err.code = AIMUX_E_API_CALL
        err.error_value = valueMem
        val ex = AimuxException.fromC(err)
        assertThat(ex).isInstanceOf(APICallError::class.java)
        assertThat(ex.status).isEqualTo(-1)
        assertThat(ex.errorValue).isEqualTo(json)
    }

    /** TokenExpired keeps its own class: 401 whose fix is a refresh, not a retry. */
    @Test
    fun `fromC maps TokenExpired to status 401`() {
        val err = AimuxCError()
        err.code = AIMUX_E_TOKEN_EXPIRED
        val ex = AimuxException.fromC(err)
        assertThat(ex).isInstanceOf(TokenExpiredError::class.java)
        assertThat(ex.status).isEqualTo(401)
    }

    @Test
    fun `fromC preserves unrecognized codes`() {
        val err = AimuxCError()
        err.code = 999
        val ex = AimuxException.fromC(err)
        assertThat(ex).isInstanceOf(UnknownAimuxError::class.java)
        assertThat(ex.code).isEqualTo(999)
    }
}
