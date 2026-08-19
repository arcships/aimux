package ai.arcships.aimux;

import com.sun.jna.Pointer;
import com.sun.jna.ptr.PointerByReference;
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

    /** Call with an invalid handle and assert code 203 is a binding invariant. */
    private static void assertInvalidHandle(Pointer err, PointerByReference out) {
        assertThat(err).isNotNull();
        assertThat(out.getValue()).isNull();
        assertThat(AimuxFFI.INSTANCE.aimux_error_code(err)).isEqualTo(203);
        RuntimeException e = AimuxResult.expectAimuxError(err, null);
        assertThat(e).isInstanceOf(IllegalStateException.class).isNotInstanceOf(AimuxException.class);
        assertThat(e.getMessage()).startsWith("aimux ffi: ");
    }

    @Test
    void unknownProviderCarriesProviderId() {
        try {
            Model.provider("definitely-not-a-provider", "sk-x", "some-model", null);
            throw new AssertionError("expected AimuxException");
        } catch (AimuxException.NoSuchProviderError e) {
            assertThat(e.getProviderId()).isEqualTo("definitely-not-a-provider");
        }
    }

    @Test
    void transcriptionGenerateBadHandle() {
        PointerByReference out = new PointerByReference();
        assertInvalidHandle(
            AimuxFFI.INSTANCE.aimux_transcription_generate(0L, "aGk=", "audio/wav", null, out), out);
    }

    @Test
    void fileUploadBadHandle() {
        PointerByReference out = new PointerByReference();
        assertInvalidHandle(
            AimuxFFI.INSTANCE.aimux_file_upload(0L, "aGk=", "text/plain", null, out), out);
    }

    @Test
    void videoGenerateBadHandle() {
        PointerByReference out = new PointerByReference();
        assertInvalidHandle(
            AimuxFFI.INSTANCE.aimux_video_generate(0L, "{}", out), out);
    }

    @Test
    void providerListModelsBadHandle() {
        PointerByReference out = new PointerByReference();
        assertInvalidHandle(
            AimuxFFI.INSTANCE.aimux_provider_list_models(0L, out), out);
    }

    @Test
    void getModelSpecsUnreachableUrlFails() {
        // Unroutable local port: fails fast without touching the network default.
        assertThatThrownBy(() -> Model.getModelSpecs("http://127.0.0.1:1/specs"))
            .isInstanceOf(AimuxException.class);
    }

    @Test
    void mockReplayNewRejectsGarbageJsonl() {
        // Malformed JSONL is rejected by the binding before the C call.
        assertThatThrownBy(() -> Model.mockReplay("not json at all"))
            .isInstanceOf(IllegalArgumentException.class)
            .hasMessageContaining("recordingsJsonl");
    }
}
