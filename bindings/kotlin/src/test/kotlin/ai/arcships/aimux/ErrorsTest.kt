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
    fun `fromC maps RateLimited with null message to fallback and default 429`() {
        val err = AimuxCError()
        err.code = AIMUX_E_RATE_LIMITED
        val ex = AimuxException.fromC(err)
        assertThat(ex).isInstanceOf(RateLimitedError::class.java)
        assertThat(ex.status).isEqualTo(429)
        assertThat(ex.message).isEqualTo("aimux: RateLimited")
        assertThat(ex.errorValue).isNull()
    }

    @Test
    fun `fromC reads a NUL-terminated message and carries status, retry_ms and error_value`() {
        val json = """{"RateLimited":{"retry_after_ms":1500,"message":"too many requests"}}"""
        val mem = Memory(64)
        mem.setString(0, "too many requests", "UTF-8")
        val valueMem = Memory(128)
        valueMem.setString(0, json, "UTF-8")
        val err = AimuxCError()
        err.code = AIMUX_E_RATE_LIMITED
        err.status = 429
        err.retry_ms = 1500
        err.message = mem
        err.error_value = valueMem
        val ex = AimuxException.fromC(err)
        assertThat(ex).isInstanceOf(RateLimitedError::class.java)
        assertThat(ex.message).isEqualTo("too many requests")
        assertThat(ex.retryMs).isEqualTo(1500L)
        assertThat(ex.errorValue).isEqualTo(json)
        // fromC did not free: both Memory blocks are still valid and unchanged.
        assertThat(mem.getString(0, "UTF-8")).isEqualTo("too many requests")
        assertThat(valueMem.getString(0, "UTF-8")).isEqualTo(json)
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
