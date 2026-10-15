package ai.arcships.aimux;

import ai.arcships.aimux.generated.AimuxOps;
import com.fasterxml.jackson.databind.JsonNode;
import com.sun.jna.NativeLibrary;
import com.sun.jna.Pointer;
import com.sun.jna.ptr.IntByReference;
import com.sun.jna.ptr.PointerByReference;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.CancellationException;
import java.util.concurrent.Future;
import java.util.concurrent.atomic.AtomicReference;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;
import static org.assertj.core.api.Assertions.*;

/** Exercises the built Rust library without provider credentials or network access. */
class NativeLibraryTest {
    private Aimux aimux;
    private static final String PROVIDER = "{\"package\":\"openai\",\"settings\":{\"apiKey\":\"local-test\"}}";
    private static final String TEXT = "{\"prompt\":\"你好 🌏\",\"options\":{\"maxRetries\":0}}";

    @BeforeEach void load() { aimux = Aimux.load(); }

    @Test void loadsTheGeneratedAbiAndAllFourteenPrimitives() {
        JsonNode manifest = aimux.getManifest();
        assertThat(manifest.path("abiVersion").asLong()).isEqualTo(AimuxOps.ABI_VERSION);
        assertThat(manifest.path("opsVersion").asText()).isEqualTo(AimuxOps.OPS_VERSION);
        List<String> names = new ArrayList<>();
        manifest.path("ops").forEach(op -> names.add(op.path("name").asText()));
        assertThat(names).contains(AimuxOps.OP_DECISION_DECIDE, AimuxOps.OP_TRANSCRIPTION_START,
            AimuxOps.OP_TEXT_STREAM, AimuxOps.OP_PROVIDER_CREATE);
        NativeLibrary library = NativeLibrary.getInstance("aimux_ffi");
        for (java.lang.reflect.Method method : AimuxFFI.class.getDeclaredMethods()) {
            assertThat(library.getFunction(method.getName())).isNotNull();
        }
    }

    @Test void reportsRealProtocolAndBindingEnvelopesAndDropsIdempotently() {
        assertThatThrownBy(() -> aimux.call("unknown.operation", "{}"))
            .isInstanceOf(ProtocolException.InvalidOp.class);
        assertThatThrownBy(() -> aimux.call(AimuxOps.OP_ABORT_CREATE, "{\"unexpected\":true}"))
            .isInstanceOf(ProtocolException.InvalidParams.class);
        try (Resource signal = aimux.create(AimuxOps.OP_ABORT_CREATE, "{}")) {
            long id = Long.parseUnsignedLong(signal.getHandle());
            assertThatThrownBy(() -> signal.call(AimuxOps.OP_MODEL_INFO, "{}"))
                .isInstanceOf(BindingException.InvalidHandle.class);
            assertThat(signal.call(AimuxOps.OP_ABORT_SIGNAL, "{}").isNull()).isTrue();
            signal.close();
            signal.close();
            assertThatThrownBy(() -> aimux.call(id, AimuxOps.OP_ABORT_SIGNAL, "{}", 0))
                .isInstanceOf(BindingException.InvalidHandle.class);
            aimux.ffi.aimux_drop(id);
            aimux.ffi.aimux_drop(0);
        }
    }

    @Test void providerChildrenRetainTheirModelsAfterTheProviderCloses() {
        ProviderHandle provider = aimux.createProvider(PROVIDER);
        try (Model model = provider.languageModel("你好-model");
             EmbeddingModel embedding = provider.embeddingModel("text-embedding-3-small");
             ImageModel image = provider.imageModel("gpt-image-1");
             SpeechModel speech = provider.speechModel("tts-1");
             TranscriptionModel transcription = provider.transcriptionModel("whisper-1");
             Files files = provider.files()) {
            provider.close();
            assertThat(model.info().path("modelId").asText()).isEqualTo("你好-model");
            assertThat(embedding.info().path("modelId").asText()).isEqualTo("text-embedding-3-small");
            assertThat(image.info()).isNotNull();
            assertThat(speech.info()).isNotNull();
            assertThat(transcription.info()).isNotNull();
        } finally { provider.close(); }
    }

    @Test void preservesDecisionThroughTheDataConfiguredProvider() {
        try (ProviderHandle provider = aimux.createProvider("{\"package\":\"jev\",\"settings\":{\"apiKey\":\"local-test\"}}");
             DecisionModel model = provider.decisionModel("decision-test")) {
            assertThat(model.capabilities().isObject()).isTrue();
            assertThatThrownBy(() -> model.decide("{}"))
                .isInstanceOf(ProtocolException.InvalidParams.class);
        }
    }

    @Test void genericResourcesResolveModelsThroughAnExplicitRegistry() {
        ProviderHandle provider = aimux.createProvider(PROVIDER);
        String entries = "{\"namespace\":\"java\",\"entries\":[{\"key\":\"local\",\"provider\":\""
            + provider.getHandle() + "\"}]}";
        try (Resource registry = aimux.create(AimuxOps.OP_REGISTRY_CREATE, entries)) {
            provider.close();
            try (Resource model = registry.create(AimuxOps.OP_MODEL_RESOLVE,
                    "{\"modelRef\":{\"namespace\":\"java\",\"key\":\"local\","
                    + "\"method\":\"languageModel\",\"modelId\":\"test-model\"}}")) {
                registry.close();
                assertThat(model.call(AimuxOps.OP_MODEL_INFO, "{}").path("modelId").asText())
                    .isEqualTo("test-model");
            }
        } finally { provider.close(); }
    }

    @Test void genericRecorderUsesRealCreateFlushAndExportShapes() {
        try (Resource recorder = aimux.create(AimuxOps.OP_RECORDER_CREATE,
                "{\"backend\":{\"type\":\"ring\",\"capacity\":8},\"capturePolicy\":\"full\"}")) {
            recorder.call(AimuxOps.OP_RECORDER_FLUSH, "{}");
            assertThat(recorder.call(AimuxOps.OP_RECORDER_EXPORT, "{}").path("jsonl").asText()).isEmpty();
            assertThat(recorder.call(AimuxOps.OP_RECORDER_STATS, "{}").isObject()).isTrue();
        }
    }

    @Test void cancellationOfQueuedWorkReleasesTheRealNativeAdmission() {
        AtomicReference<Runnable> queued = new AtomicReference<>();
        Resource signal = aimux.create(AimuxOps.OP_ABORT_CREATE, "{}");
        long id = Long.parseUnsignedLong(signal.getHandle());
        Future<JsonNode> future = signal.callAsync(AimuxOps.OP_ABORT_SIGNAL, "{}", queued::set);
        signal.close();
        assertThat(aimux.call(id, AimuxOps.OP_ABORT_SIGNAL, "{}", 0).isNull()).isTrue();
        assertThat(future.cancel(false)).isTrue();
        queued.get().run();
        assertThatThrownBy(future::get).isInstanceOf(CancellationException.class);
        assertThatThrownBy(() -> aimux.call(id, AimuxOps.OP_ABORT_SIGNAL, "{}", 0))
            .isInstanceOf(BindingException.InvalidHandle.class);
    }

    @Test void preabortedNativeStreamDeliversOneTerminalAndAllowsCloseInConsumer() {
        try (ProviderHandle provider = aimux.createProvider(PROVIDER);
             Model model = provider.languageModel("local-test"); Abort abort = aimux.createAbort()) {
            abort.signal();
            List<StreamFrame> frames = new ArrayList<>();
            assertThatThrownBy(() -> model.stream(AimuxOps.OP_TEXT_STREAM, TEXT, abort, frame -> {
                frames.add(frame);
                model.close();
                model.close();
                System.gc();
                try { Thread.sleep(20); }
                catch (InterruptedException error) { throw new AssertionError(error); }
            })).isInstanceOf(CancellationException.class);
            assertThat(frames).hasSize(1);
            assertThat(frames.get(0).isEnd()).isTrue();
            assertThat(frames.get(0).getOutcome()).isEqualTo(StreamFrame.Outcome.CANCELLED);
            assertThat(frames.get(0).getJson().path("type").asText()).isEqualTo("end");
        }
    }

    @Test void nativeCallbackFailureIsLatchedUntilTheNativeStreamReturns() {
        try (ProviderHandle provider = aimux.createProvider(PROVIDER);
             Model model = provider.languageModel("local-test"); Abort abort = aimux.createAbort()) {
            abort.signal();
            AssertionError cause = new AssertionError("consumer failed");
            assertThatThrownBy(() -> model.stream(AimuxOps.OP_TEXT_STREAM, TEXT, abort, frame -> { throw cause; }))
                .isInstanceOf(BindingException.CallbackFailure.class).hasCause(cause);
            assertThat(model.info()).isNotNull();
        }
    }

    @Test void callbackReentryIsReportedAfterTheNativeStreamReturns() {
        try (ProviderHandle provider = aimux.createProvider(PROVIDER);
             Model model = provider.languageModel("local-test"); Abort abort = aimux.createAbort()) {
            abort.signal();
            assertThatThrownBy(() -> model.stream(AimuxOps.OP_TEXT_STREAM, TEXT, abort, frame -> model.info()))
                .isInstanceOf(BindingException.CallbackFailure.class)
                .hasCauseInstanceOf(BindingException.ReentrantCall.class);
        }
    }

    @Test void preacceptanceStreamErrorsProduceNoCallbacks() {
        try (ProviderHandle provider = aimux.createProvider(PROVIDER);
             Model model = provider.languageModel("local-test")) {
            List<StreamFrame> frames = new ArrayList<>();
            assertThatThrownBy(() -> model.stream("{}", frames::add))
                .isInstanceOf(ProtocolException.InvalidParams.class);
            assertThat(frames).isEmpty();
        }
    }

    @Test void primitiveErrorsResetOutputsAndHonorBinaryLengthAndSignedTimeout() {
        PointerByReference out = new PointerByReference(new Pointer(1));
        IntByReference state = new IntByReference(99);
        Pointer error = aimux.ffi.aimux_operation_next_part(0, -1, out, state);
        assertThat(out.getValue()).isNull();
        assertThat(state.getValue()).isZero();
        assertThatThrownBy(() -> AimuxResult.check(aimux.ffi, error))
            .isInstanceOf(ProtocolException.InvalidParams.class);
        assertThatThrownBy(() -> AimuxResult.check(aimux.ffi,
            aimux.ffi.aimux_operation_push(0, new byte[0], new AimuxFFI.SizeT(1048577))))
            .isInstanceOf(ProtocolException.ResourceLimit.class);
        assertThatThrownBy(() -> AimuxResult.check(aimux.ffi,
            aimux.ffi.aimux_operation_push(0, new byte[0], new AimuxFFI.SizeT(0))))
            .isInstanceOf(BindingException.InvalidHandle.class);
    }

    @Test void finalizerPrimitiveAcceptsNullAndReleasesAnOwnedAbort() {
        JsonNode result = aimux.call(AimuxOps.OP_ABORT_CREATE, "{}");
        long id = AimuxResult.handle(result);
        aimux.ffi.aimux_handle_finalizer(new Pointer(id));
        aimux.ffi.aimux_handle_finalizer(null);
        long deadline = System.nanoTime() + java.util.concurrent.TimeUnit.SECONDS.toNanos(2);
        while (System.nanoTime() < deadline) {
            try { aimux.call(id, AimuxOps.OP_ABORT_SIGNAL, "{}", 0); }
            catch (BindingException.InvalidHandle released) { return; }
            Thread.yield();
        }
        fail("Native finalizer did not release the handle");
    }
}
