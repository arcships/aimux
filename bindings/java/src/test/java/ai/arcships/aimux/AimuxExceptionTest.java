package ai.arcships.aimux;

import com.sun.jna.Pointer;
import com.sun.jna.ptr.LongByReference;
import com.sun.jna.ptr.PointerByReference;
import org.junit.jupiter.api.Test;

import static org.assertj.core.api.Assertions.assertThat;
import static org.assertj.core.api.Assertions.assertThatThrownBy;

/**
 * Tests for {@link AimuxException#fromC} / {@link AimuxException#of} mapping
 * and the {@link AimuxResult} decoders. An error is an opaque
 * {@code aimux_error_t *} returned by the failed call, so the decoder
 * probes drive real errors obtained offline (unknown provider, a 401 from the
 * loopback mock server). Bridge-layer misuse is absorbed by the binding and
 * surfaces as native Java exceptions, never {@link AimuxException}: malformed
 * raw JSON is caught before the C call ({@link IllegalArgumentException});
 * codes 200–206 are binding
 * invariant ({@link IllegalStateException}).
 */
class AimuxExceptionTest {

    /** Real C ABI error: {@code aimux_openai_new(NULL, ...)}. */
    private static Pointer nullPointerError() {
        LongByReference out = new LongByReference();
        Pointer e = AimuxFFI.INSTANCE.aimux_openai_new(null, "gpt-4o-mini", out);
        assertThat(e).isNotNull();
        assertThat(out.getValue()).isZero();
        return e;
    }

    /** Real NO_SUCH_PROVIDER error: {@code aimux_provider_new("no-such-provider", ...)}. */
    private static Pointer noSuchProviderError() {
        LongByReference out = new LongByReference();
        Pointer e = AimuxFFI.INSTANCE.aimux_provider_new("no-such-provider", "sk-x", "m", null, out);
        assertThat(e).isNotNull();
        assertThat(out.getValue()).isZero();
        return e;
    }

    @Test
    void expectAimuxErrorMapsAiMuxErrorAndFreesIt() {
        Pointer eh = noSuchProviderError();
        assertThat(AimuxFFI.INSTANCE.aimux_error_code(eh))
            .isEqualTo(AimuxException.AIMUX_E_NO_SUCH_PROVIDER);
        RuntimeException e = AimuxResult.expectAimuxError(eh, null);
        assertThat(e).isInstanceOf(AimuxException.NoSuchProviderError.class);
        AimuxException ae = (AimuxException) e;
        assertThat(ae.getCode()).isEqualTo(AimuxException.AIMUX_E_NO_SUCH_PROVIDER);
        assertThat(ae.getStatusCode()).isEqualTo(-1);
        assertThat(ae.getRetryMs()).isEqualTo(-1L);
        assertThat(ae.isRetryable()).isFalse();
        assertThat(ae.getMessage()).isNotEmpty();
    }

    @Test
    void expectAimuxErrorPrefixesContext() {
        RuntimeException e = AimuxResult.expectAimuxError(noSuchProviderError(), "Failed to create model");
        assertThat(e.getMessage()).startsWith("Failed to create model: ");
    }

    // ── C ABI failures: native Java exceptions, not AimuxException ──────────

    @Test
    void nullStringArgIsBindingInvariant() {
        Pointer eh = nullPointerError();
        // A C ABI error has its own code and a message …
        assertThat(AimuxFFI.INSTANCE.aimux_error_code(eh)).isEqualTo(200);
        assertThat(AimuxResult.takeString(AimuxFFI.INSTANCE.aimux_error_message(eh))).isNotEmpty();
        // … and decodes to the invariant error.
        RuntimeException e = AimuxResult.expectAimuxError(eh, "ctx");
        assertThat(e).isInstanceOf(IllegalStateException.class).isNotInstanceOf(AimuxException.class);
        assertThat(e.getMessage()).startsWith("ctx: aimux ffi: ").contains("api_key");
    }

    @Test
    void deadHandleIsBindingInvariant() {
        PointerByReference out = new PointerByReference();
        Pointer eh = AimuxFFI.INSTANCE.aimux_provider_list_models(0x7FFF_FFFF_FFFFL, out);
        assertThat(eh).isNotNull();
        assertThat(out.getValue()).isNull();
        assertThat(AimuxFFI.INSTANCE.aimux_error_code(eh)).isEqualTo(203);
        RuntimeException e = AimuxResult.expectAimuxError(eh, null);
        assertThat(e).isInstanceOf(IllegalStateException.class).isNotInstanceOf(AimuxException.class);
        assertThat(e.getMessage()).startsWith("aimux ffi: ");
    }

    @Test
    void ffiInvalidWireJsonIsBindingInvariant() {
        Pointer eh = AimuxFFI.INSTANCE.aimux_register_providers("{not json");
        assertThat(eh).isNotNull();
        assertThat(AimuxFFI.INSTANCE.aimux_error_code(eh)).isEqualTo(202);
        RuntimeException e = AimuxResult.expectAimuxError(eh, "ctx");
        assertThat(e).isInstanceOf(IllegalStateException.class).isNotInstanceOf(AimuxException.class);
        assertThat(e.getMessage()).startsWith("ctx: aimux ffi: ").contains("config_json");
    }

    @Test
    void expectFfiErrorReadsMessageOnly() {
        RuntimeException e = AimuxResult.expectFfiError(nullPointerError(), "ctx");
        assertThat(e).isInstanceOf(IllegalStateException.class);
        assertThat(e.getMessage()).startsWith("ctx: aimux ffi: ").contains("api_key");
    }

    @Test
    void expectRecordingErrorRejectsAnAiMuxErrorCode() {
        RuntimeException e = AimuxResult.expectRecordingError(noSuchProviderError(), "ctx");
        assertThat(e).isInstanceOf(IllegalStateException.class).isNotInstanceOf(RecordingException.class);
        assertThat(e.getMessage()).startsWith("ctx: aimux ffi: ");
    }

    @Test
    void trailingGarbageJsonIsIllegalArgument() {
        assertThatThrownBy(() -> Aimux.registerProviders("{} garbage"))
            .isInstanceOf(IllegalArgumentException.class)
            .hasMessageContaining("configJson");
        assertThatThrownBy(() -> Aimux.registerProviders(""))
            .isInstanceOf(IllegalArgumentException.class)
            .hasMessageContaining("configJson: invalid JSON: empty");
        assertThatThrownBy(() -> Aimux.registerProviders(null))
            .isInstanceOf(NullPointerException.class);
    }

    @Test
    void malformedConfigJsonIsIllegalArgument() {
        assertThatThrownBy(() -> Aimux.registerProviders("{not json"))
            .isInstanceOf(IllegalArgumentException.class)
            .isNotInstanceOf(AimuxException.class)
            .hasMessageContaining("configJson");
    }

    @Test
    void malformedPromptJsonIsIllegalArgumentBeforeTheCall() {
        try (Model model = Model.openai("sk-test", "gpt-4o-mini")) {
            assertThatThrownBy(() -> model.generateText("{not json"))
                .isInstanceOf(IllegalArgumentException.class)
                .hasMessageContaining("promptJson");
            assertThatThrownBy(() -> model.generateText("\"hi\"", "{not json"))
                .isInstanceOf(IllegalArgumentException.class)
                .hasMessageContaining("optsJson");
        }
    }

    @Test
    void fromCMapsHttp401ToApiCallWithStatus() {
        try (MockProviderServer server = new MockProviderServer();
             Model model = Model.openaiWithBase("sk-bad", "gpt-4o-mini", server.baseUrl())) {
            server.setStatus(401);
            server.setResponseBody("{\"error\":{\"message\":\"bad key\",\"type\":\"invalid_request_error\"}}");
            try {
                model.generateText("\"hi\"");
                throw new AssertionError("expected APICallError");
            } catch (AimuxException.APICallError e) {
                assertThat(e.getCode()).isEqualTo(AimuxException.AIMUX_E_API_CALL);
                assertThat(e.getStatusCode()).isEqualTo(401);
                assertThat(e.getMessage()).contains("401");
            }
        }
    }

    @Test
    void fromCThreadsProviderIdToNoSuchProvider() {
        try {
            Model.provider("no-such-provider", "sk-x", "m", null);
            throw new AssertionError("expected NoSuchProviderError");
        } catch (AimuxException.NoSuchProviderError e) {
            assertThat(e.getCode()).isEqualTo(AimuxException.AIMUX_E_NO_SUCH_PROVIDER);
            assertThat(e.getProviderId()).isEqualTo("no-such-provider");
            assertThat(e.isRetryable()).isFalse();
        }
    }

    @Test
    void decodingNullIsAContractViolation() {
        assertThatThrownBy(() -> AimuxResult.expectAimuxError(null, null))
            .isInstanceOf(IllegalStateException.class);
        assertThatThrownBy(() -> AimuxResult.expectRecordingError(null, null))
            .isInstanceOf(IllegalStateException.class);
        assertThatThrownBy(() -> AimuxResult.expectFfiError(null, null))
            .isInstanceOf(IllegalStateException.class);
    }

    @Test
    void ofDefaultsStatusAndRetry() {
        AimuxException e = AimuxException.of(
            AimuxException.AIMUX_E_INVALID_ARGUMENT, "bad arg");
        assertThat(e).isInstanceOf(AimuxException.InvalidArgumentError.class);
        assertThat(e.getStatusCode()).isEqualTo(-1);
        assertThat(e.getRetryMs()).isEqualTo(-1L);
        assertThat(e.getMessage()).isEqualTo("bad arg");
        assertThat(e.isRetryable()).isFalse();
    }

    @Test
    void ofMapsEveryVariantToExpectedClass() {
        assertThat(AimuxException.of(AimuxException.AIMUX_E_JSON_PARSE, "m"))
            .isInstanceOf(AimuxException.JSONParseError.class);
        assertThat(AimuxException.of(AimuxException.AIMUX_E_INVALID_RESPONSE_DATA, "m"))
            .isInstanceOf(AimuxException.InvalidResponseDataError.class);
        assertThat(AimuxException.of(AimuxException.AIMUX_E_TOOL, "m"))
            .isInstanceOf(AimuxException.ToolError.class);
        assertThat(AimuxException.of(AimuxException.AIMUX_E_INVALID_ARGUMENT, "m"))
            .isInstanceOf(AimuxException.InvalidArgumentError.class);
        assertThat(AimuxException.of(AimuxException.AIMUX_E_INVALID_PROMPT, "m"))
            .isInstanceOf(AimuxException.InvalidPromptError.class);
        assertThat(AimuxException.of(AimuxException.AIMUX_E_TOKEN_EXPIRED, "m"))
            .isInstanceOf(AimuxException.TokenExpiredError.class);
        assertThat(AimuxException.of(AimuxException.AIMUX_E_UNSUPPORTED_FUNCTIONALITY, "m"))
            .isInstanceOf(AimuxException.UnsupportedFunctionalityError.class);
        assertThat(AimuxException.of(AimuxException.AIMUX_E_NO_SUCH_MODEL, "m"))
            .isInstanceOf(AimuxException.NoSuchModelError.class);
        assertThat(AimuxException.of(AimuxException.AIMUX_E_NO_SUCH_PROVIDER, "m"))
            .isInstanceOf(AimuxException.NoSuchProviderError.class);
        assertThat(AimuxException.of(AimuxException.AIMUX_E_API_CALL, "m"))
            .isInstanceOf(AimuxException.APICallError.class);
        assertThat(AimuxException.of(AimuxException.AIMUX_E_TIMEOUT, "m"))
            .isInstanceOf(AimuxException.TimeoutError.class);
        assertThat(AimuxException.of(AimuxException.AIMUX_E_ABORTED, "m"))
            .isInstanceOf(AimuxException.RequestAbortedError.class);
        assertThat(AimuxException.of(AimuxException.AIMUX_E_OTHER, "m"))
            .isInstanceOf(AimuxException.OtherError.class);
    }

    @Test
    void codesOutsideTheRustEnumAreRejected() {
        // Out-of-range (the old recording slot 15) is a header/library mismatch.
        assertThatThrownBy(() -> AimuxException.of(15, ""))
            .isInstanceOf(IllegalStateException.class);
        assertThatThrownBy(() -> AimuxException.of(999, "m"))
            .isInstanceOf(IllegalStateException.class);
    }

    @Test
    void tokenExpiredDefaultsTo401() {
        assertThat(AimuxException.of(AimuxException.AIMUX_E_TOKEN_EXPIRED, "expired").getStatusCode())
            .isEqualTo(401);
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
    void extractHandleErrorThrowsMappedException() {
        LongByReference out = new LongByReference();
        Pointer e = AimuxFFI.INSTANCE.aimux_provider_new("no-such-provider", "sk-x", "m", null, out);
        try {
            AimuxResult.extractHandle(e, out, "Failed to create provider");
            throw new AssertionError("expected AimuxException");
        } catch (AimuxException.NoSuchProviderError ex) {
            assertThat(ex.getMessage()).startsWith("Failed to create provider");
            assertThat(ex.getProviderId()).isEqualTo("no-such-provider");
        }
    }

    @Test
    void extractHandleNullErrorPassesThrough() {
        LongByReference out = new LongByReference(42L);
        assertThat(AimuxResult.extractHandle(null, out, "ctx")).isEqualTo(42L);
    }

    @Test
    void codeNameCoversKnownCodes() {
        assertThat(AimuxException.codeName(AimuxException.AIMUX_OK)).isEqualTo("OK");
        assertThat(AimuxException.codeName(AimuxException.AIMUX_E_API_CALL)).isEqualTo("ApiCall");
        assertThat(AimuxException.codeName(999)).startsWith("Code(");
    }

    @Test
    void recordingCodesOutsideTheRustEnumAreRejected() {
        assertThatThrownBy(() -> RecordingErrorCode.fromC(999))
            .isInstanceOf(IllegalStateException.class);
    }

    @Test
    void payloadIsNullForLocalFailures() {
        AimuxException.NoSuchProviderError local = (AimuxException.NoSuchProviderError)
            AimuxException.of(AimuxException.AIMUX_E_NO_SUCH_PROVIDER, "nope");
        assertThat(local.getProviderId()).isNull();
        AimuxException.APICallError api = (AimuxException.APICallError)
            AimuxException.of(AimuxException.AIMUX_E_API_CALL, "x");
        assertThat(api.getProviderCode()).isNull();
        assertThat(api.getRequestId()).isNull();
        assertThat(api.getResponseBody()).isNull();
        assertThat(api.isRetryable()).isFalse();
    }
}
