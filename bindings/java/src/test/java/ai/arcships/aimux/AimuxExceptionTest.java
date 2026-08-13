package ai.arcships.aimux;

import org.junit.jupiter.api.Test;

import static org.assertj.core.api.Assertions.assertThat;

/**
 * Unit tests for {@link AimuxException#fromC} / {@link AimuxException#of}
 * mapping — pure-Java probes (the message is overridden, so no C-allocated
 * pointer is involved).
 */
class AimuxExceptionTest {

    @Test
    void cErrorStructIs40Bytes() {
        // Mirrors the C AimuxError layout:
        // int32 + int32 + int64 + char* + char* + void*[1].
        assertThat(new AimuxCError().size()).isEqualTo(40);
    }

    @Test
    void fromCMapsAuthToApiCallWith401() {
        AimuxCError err = filled(
            AimuxException.AIMUX_E_API_CALL, 401, -1L, "API call error: HTTP 401: bad key");
        AimuxException e = AimuxException.fromC(err);
        assertThat(e).isInstanceOf(AimuxException.APICallError.class);
        assertThat(e.getCode()).isEqualTo(AimuxException.AIMUX_E_API_CALL);
        assertThat(e.getStatusCode()).isEqualTo(401);
        assertThat(e.getRetryMs()).isEqualTo(-1L);
        assertThat(e.getMessage()).isEqualTo("API call error: HTTP 401: bad key");
    }

    @Test
    void fromCMapsRateLimitToApiCallWith429AndRetryMs() {
        AimuxCError err = filled(
            AimuxException.AIMUX_E_API_CALL, 429, 1500L, "API call error: HTTP 429: slow down");
        AimuxException e = AimuxException.fromC(err);
        assertThat(e).isInstanceOf(AimuxException.APICallError.class);
        assertThat(e.getStatusCode()).isEqualTo(429);
        assertThat(e.getRetryMs()).isEqualTo(1500L);
        assertThat(e.getMessage()).isEqualTo("API call error: HTTP 429: slow down");
    }

    @Test
    void apiCallReportsStatusVerbatim() {
        // No per-status defaulting: the status field is the classification, and
        // a transport failure (no response) legitimately has none.
        AimuxCError notFound = filled(AimuxException.AIMUX_E_API_CALL, 404, -1L, "gone");
        assertThat(AimuxException.fromC(notFound).getStatusCode()).isEqualTo(404);

        AimuxCError transport = filled(AimuxException.AIMUX_E_API_CALL, -1, -1L, "connection reset");
        assertThat(AimuxException.fromC(transport).getStatusCode()).isEqualTo(-1);

        // TokenExpired is a 401 by contract even when the C struct omits it.
        AimuxCError expired = filled(AimuxException.AIMUX_E_TOKEN_EXPIRED, -1, -1L, "expired");
        assertThat(AimuxException.fromC(expired).getStatusCode()).isEqualTo(401);
    }

    @Test
    void fromCMapsEveryVariantToExpectedClass() {
        assertThat(fromCode(AimuxException.AIMUX_E_JSON_PARSE))
            .isInstanceOf(AimuxException.JSONParseError.class);
        assertThat(fromCode(AimuxException.AIMUX_E_INVALID_RESPONSE_DATA))
            .isInstanceOf(AimuxException.InvalidResponseDataError.class);
        assertThat(fromCode(AimuxException.AIMUX_E_TOOL))
            .isInstanceOf(AimuxException.ToolError.class);
        assertThat(fromCode(AimuxException.AIMUX_E_INVALID_ARGUMENT))
            .isInstanceOf(AimuxException.InvalidArgumentError.class);
        assertThat(fromCode(AimuxException.AIMUX_E_INVALID_PROMPT))
            .isInstanceOf(AimuxException.InvalidPromptError.class);
        assertThat(fromCode(AimuxException.AIMUX_E_TOKEN_EXPIRED))
            .isInstanceOf(AimuxException.TokenExpiredError.class);
        assertThat(fromCode(AimuxException.AIMUX_E_UNSUPPORTED_FUNCTIONALITY))
            .isInstanceOf(AimuxException.UnsupportedFunctionalityError.class);
        assertThat(fromCode(AimuxException.AIMUX_E_NO_SUCH_MODEL))
            .isInstanceOf(AimuxException.NoSuchModelError.class);
        assertThat(fromCode(AimuxException.AIMUX_E_NO_SUCH_PROVIDER))
            .isInstanceOf(AimuxException.NoSuchProviderError.class);
        assertThat(fromCode(AimuxException.AIMUX_E_API_CALL))
            .isInstanceOf(AimuxException.APICallError.class);
        assertThat(fromCode(AimuxException.AIMUX_E_TIMEOUT))
            .isInstanceOf(AimuxException.TimeoutError.class);
        assertThat(fromCode(AimuxException.AIMUX_E_ABORTED))
            .isInstanceOf(AimuxException.RequestAbortedError.class);
        assertThat(fromCode(AimuxException.AIMUX_E_OTHER))
            .isInstanceOf(AimuxException.OtherError.class);
    }

    @Test
    void fromCNullYieldsOtherError() {
        AimuxException e = AimuxException.fromC(null);
        assertThat(e).isInstanceOf(AimuxException.OtherError.class);
        assertThat(e.getMessage()).contains("failed");
    }

    @Test
    void fromCEmptyMessageUsesCodeName() {
        AimuxCError err = filled(AimuxException.AIMUX_E_TIMEOUT, -1, -1L, "");
        AimuxException e = AimuxException.fromC(err);
        assertThat(e).isInstanceOf(AimuxException.TimeoutError.class);
        assertThat(e.getMessage()).isEqualTo("aimux: Timeout");
    }

    @Test
    void ofDefaultsStatusAndRetry() {
        AimuxException e = AimuxException.of(
            AimuxException.AIMUX_E_INVALID_ARGUMENT, "bad arg");
        assertThat(e).isInstanceOf(AimuxException.InvalidArgumentError.class);
        assertThat(e.getStatusCode()).isEqualTo(-1);
        assertThat(e.getRetryMs()).isEqualTo(-1L);
        assertThat(e.getMessage()).isEqualTo("bad arg");
    }

    @Test
    void messageOnlyConstructorKeepsBackwardCompat() {
        AimuxException e = new AimuxException("decode failed");
        assertThat(e.getCode()).isEqualTo(AimuxException.AIMUX_E_OTHER);
        assertThat(e.getStatusCode()).isEqualTo(-1);
        assertThat(e.getRetryMs()).isEqualTo(-1L);
        assertThat(e.getMessage()).isEqualTo("decode failed");
    }

    @Test
    void extractHandleZeroThrowsMappedException() {
        AimuxCError err = filled(AimuxException.AIMUX_E_NO_SUCH_PROVIDER, -1, -1L, "nope");
        try {
            AimuxResult.extractHandle(0L, err, "Failed to create provider");
            throw new AssertionError("expected AimuxException");
        } catch (AimuxException e) {
            assertThat(e).isInstanceOf(AimuxException.NoSuchProviderError.class);
            assertThat(e.getMessage()).startsWith("Failed to create provider");
            assertThat(e.getMessage()).contains("nope");
        }
    }

    @Test
    void extractHandleNonZeroPassesThrough() {
        AimuxCError err = AimuxResult.newError();
        assertThat(AimuxResult.extractHandle(42L, err, "ctx")).isEqualTo(42L);
    }

    @Test
    void codeNameCoversKnownCodes() {
        assertThat(AimuxException.codeName(AimuxException.AIMUX_OK)).isEqualTo("OK");
        assertThat(AimuxException.codeName(AimuxException.AIMUX_E_API_CALL)).isEqualTo("ApiCall");
        assertThat(AimuxException.codeName(999)).startsWith("Code(");
    }

    @Test
    void fromCThreadsErrorValueThrough() {
        String json = "{\"ApiCall\":{\"status_code\":429,\"message\":\"slow down\","
            + "\"retry_after_ms\":1500,\"is_retryable\":true}}";
        AimuxCError err = filled(AimuxException.AIMUX_E_API_CALL, 429, 1500L, "slow down", json);
        AimuxException e = AimuxException.fromC(err);
        assertThat(e.getErrorValue()).isEqualTo(json);
    }

    @Test
    void errorValueIsNullWhenAbsent() {
        AimuxException fromC = AimuxException.fromC(
            filled(AimuxException.AIMUX_E_API_CALL, 401, -1L, "bad key"));
        assertThat(fromC.getErrorValue()).isNull();

        AimuxException local = AimuxException.of(
            AimuxException.AIMUX_E_INVALID_ARGUMENT, "bad arg");
        assertThat(local.getErrorValue()).isNull();

        assertThat(AimuxException.fromC(null).getErrorValue()).isNull();
    }

    private static AimuxException fromCode(int code) {
        return AimuxException.fromC(filled(code, -1, -1L, "msg"));
    }

    private static AimuxCError filled(int code, int status, long retryMs, String msg) {
        return filled(code, status, retryMs, msg, null);
    }

    /**
     * Pure-Java probe: overrides {@code takeMessage()} / {@code takeErrorValue()}
     * so no C-allocated pointer (and no {@code aimux_free_string} call) is
     * involved.
     */
    private static AimuxCError filled(
            int code, int status, long retryMs, final String msg, final String errValue) {
        // The inherited `message` field (Pointer) would shadow a same-named
        // parameter, hence `msg`.
        AimuxCError err = new AimuxCError() {
            @Override
            String takeMessage() {
                return msg;
            }

            @Override
            String takeErrorValue() {
                return errValue;
            }
        };
        err.clear();
        err.code = code;
        err.status = status;
        err.retry_ms = retryMs;
        return err;
    }
}
