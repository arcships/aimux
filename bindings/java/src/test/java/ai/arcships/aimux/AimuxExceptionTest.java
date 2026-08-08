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
    void fromCMapsAuthToAuthenticationError() {
        AimuxCError err = filled(AimuxException.AIMUX_E_AUTH, 401, -1L, "bad key");
        AimuxException e = AimuxException.fromC(err);
        assertThat(e).isInstanceOf(AimuxException.AuthenticationError.class);
        assertThat(e.getCode()).isEqualTo(AimuxException.AIMUX_E_AUTH);
        assertThat(e.getStatusCode()).isEqualTo(401);
        assertThat(e.getRetryMs()).isEqualTo(-1L);
        assertThat(e.getMessage()).isEqualTo("bad key");
    }

    @Test
    void fromCMapsRateLimitedWithRetryMs() {
        AimuxCError err = filled(AimuxException.AIMUX_E_RATE_LIMITED, 429, 1500L, "slow down");
        AimuxException e = AimuxException.fromC(err);
        assertThat(e).isInstanceOf(AimuxException.RateLimitedError.class);
        assertThat(e.getStatusCode()).isEqualTo(429);
        assertThat(e.getRetryMs()).isEqualTo(1500L);
        assertThat(e.getMessage()).isEqualTo("slow down");
    }

    @Test
    void fromCDefaultsStatusWhenMinusOne() {
        AimuxCError auth = filled(AimuxException.AIMUX_E_AUTH, -1, -1L, "auth");
        assertThat(AimuxException.fromC(auth).getStatusCode()).isEqualTo(401);

        AimuxCError rl = filled(AimuxException.AIMUX_E_RATE_LIMITED, -1, 0L, "rl");
        AimuxException e = AimuxException.fromC(rl);
        assertThat(e.getStatusCode()).isEqualTo(429);
        assertThat(e.getRetryMs()).isEqualTo(0L);

        AimuxCError nf = filled(AimuxException.AIMUX_E_MODEL_NOT_FOUND, -1, -1L, "gone");
        assertThat(AimuxException.fromC(nf).getStatusCode()).isEqualTo(404);
    }

    @Test
    void fromCMapsEveryVariantToExpectedClass() {
        assertThat(fromCode(AimuxException.AIMUX_E_PROVIDER))
            .isInstanceOf(AimuxException.ProviderError.class);
        assertThat(fromCode(AimuxException.AIMUX_E_HTTP))
            .isInstanceOf(AimuxException.HttpError.class);
        assertThat(fromCode(AimuxException.AIMUX_E_JSON))
            .isInstanceOf(AimuxException.JsonError.class);
        assertThat(fromCode(AimuxException.AIMUX_E_STREAM))
            .isInstanceOf(AimuxException.StreamError.class);
        assertThat(fromCode(AimuxException.AIMUX_E_TOOL))
            .isInstanceOf(AimuxException.ToolError.class);
        assertThat(fromCode(AimuxException.AIMUX_E_INVALID_ARGUMENT))
            .isInstanceOf(AimuxException.InvalidArgumentError.class);
        assertThat(fromCode(AimuxException.AIMUX_E_INVALID_PROMPT))
            .isInstanceOf(AimuxException.InvalidPromptError.class);
        assertThat(fromCode(AimuxException.AIMUX_E_TOKEN_EXPIRED))
            .isInstanceOf(AimuxException.TokenExpiredError.class);
        assertThat(fromCode(AimuxException.AIMUX_E_UNSUPPORTED))
            .isInstanceOf(AimuxException.UnsupportedError.class);
        assertThat(fromCode(AimuxException.AIMUX_E_NO_SUCH_MODEL))
            .isInstanceOf(AimuxException.NoSuchModelError.class);
        assertThat(fromCode(AimuxException.AIMUX_E_UNKNOWN_PROVIDER))
            .isInstanceOf(AimuxException.UnknownProviderError.class);
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
        AimuxCError err = filled(AimuxException.AIMUX_E_UNKNOWN_PROVIDER, -1, -1L, "nope");
        try {
            AimuxResult.extractHandle(0L, err, "Failed to create provider");
            throw new AssertionError("expected AimuxException");
        } catch (AimuxException e) {
            assertThat(e).isInstanceOf(AimuxException.UnknownProviderError.class);
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
        assertThat(AimuxException.codeName(AimuxException.AIMUX_E_AUTH)).isEqualTo("Auth");
        assertThat(AimuxException.codeName(AimuxException.AIMUX_OK)).isEqualTo("OK");
        assertThat(AimuxException.codeName(999)).startsWith("Code(");
    }

    @Test
    void fromCThreadsErrorValueThrough() {
        String json = "{\"RateLimited\":{\"retry_after_ms\":1500,\"message\":\"slow down\"}}";
        AimuxCError err = filled(AimuxException.AIMUX_E_RATE_LIMITED, 429, 1500L, "slow down", json);
        AimuxException e = AimuxException.fromC(err);
        assertThat(e.getErrorValue()).isEqualTo(json);
    }

    @Test
    void errorValueIsNullWhenAbsent() {
        AimuxException fromC = AimuxException.fromC(
            filled(AimuxException.AIMUX_E_AUTH, 401, -1L, "bad key"));
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
