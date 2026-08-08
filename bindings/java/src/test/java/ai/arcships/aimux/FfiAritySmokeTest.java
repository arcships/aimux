package ai.arcships.aimux;

import com.sun.jna.Pointer;
import org.junit.jupiter.api.Test;

import static org.assertj.core.api.Assertions.assertThat;
import static org.assertj.core.api.Assertions.assertThatThrownBy;

/**
 * Arity smoke tests: exercise FFI symbols that no other test touches through
 * the real library. Each call is expected to fail (bad handle / bad args), but
 * it must go through JNA — a C-signature/arity mismatch would crash or throw
 * an UnsatisfiedLinkError instead of returning a clean error.
 */
class FfiAritySmokeTest {

    /** Call with an invalid handle and assert the C layer reports INVALID_ARGUMENT. */
    private static void assertInvalidHandle(Pointer result, AimuxCError err) {
        assertThat(result).isNull();
        AimuxException e = AimuxException.fromC(err);
        assertThat(e).isInstanceOf(AimuxException.InvalidArgumentError.class);
        assertThat(e.getMessage()).contains("invalid or expired");
        // FFI-synthesized failure: no engine AiMuxError, so no error_value JSON.
        assertThat(e.getErrorValue()).isNull();
    }

    @Test
    void unknownProviderCarriesErrorValueJson() {
        try {
            Model.provider("definitely-not-a-provider", "sk-x", "some-model", null);
            throw new AssertionError("expected AimuxException");
        } catch (AimuxException e) {
            assertThat(e).isInstanceOf(AimuxException.UnknownProviderError.class);
            assertThat(e.getErrorValue()).isNotNull().contains("UnknownProvider");
        }
    }

    @Test
    void transcriptionGenerateBadHandle() {
        AimuxCError err = AimuxResult.newError();
        assertInvalidHandle(
            AimuxFFI.INSTANCE.aimux_transcription_generate(0L, "aGk=", "audio/wav", null, err), err);
    }

    @Test
    void fileUploadBadHandle() {
        AimuxCError err = AimuxResult.newError();
        assertInvalidHandle(
            AimuxFFI.INSTANCE.aimux_file_upload(0L, "aGk=", "text/plain", null, err), err);
    }

    @Test
    void videoGenerateBadHandle() {
        AimuxCError err = AimuxResult.newError();
        assertInvalidHandle(
            AimuxFFI.INSTANCE.aimux_video_generate(0L, "{}", err), err);
    }

    @Test
    void providerListModelsBadHandle() {
        AimuxCError err = AimuxResult.newError();
        assertInvalidHandle(
            AimuxFFI.INSTANCE.aimux_provider_list_models(0L, err), err);
    }

    @Test
    void getModelSpecsUnreachableUrlFails() {
        // Unroutable local port: fails fast without touching the network default.
        assertThatThrownBy(() -> Model.getModelSpecs("http://127.0.0.1:1/specs"))
            .isInstanceOf(AimuxException.class);
    }

    @Test
    void mockReplayNewRejectsGarbageJsonl() {
        assertThatThrownBy(() -> Model.mockReplay("not json at all"))
            .isInstanceOf(AimuxException.class);
    }
}
