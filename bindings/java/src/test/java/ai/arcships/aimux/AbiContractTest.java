package ai.arcships.aimux;

import ai.arcships.aimux.types.StreamFrame;
import ai.arcships.aimux.types.Outcome;
import ai.arcships.aimux.types.DtoJson;

import com.fasterxml.jackson.databind.JsonNode;
import com.sun.jna.Library;
import com.sun.jna.Native;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;
import java.time.Duration;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import java.util.concurrent.CancellationException;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.Executor;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;
import java.util.concurrent.RejectedExecutionException;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicReference;
import java.util.function.BooleanSupplier;
import static org.assertj.core.api.Assertions.*;
import static org.junit.jupiter.api.Assertions.assertTimeoutPreemptively;

/** Runs through JNA into a test-only C library; no provider network or Rust ABI stub is used. */
class AbiContractTest {
    interface Fixture extends Library {
        int fixture_entered();
        int fixture_live_strings();
        int fixture_live_errors();
        int fixture_drop_in_callback();
        int fixture_drops(int id);
        int fixture_signals(int id);
        int fixture_callbacks();
        long fixture_last_handle();
        long fixture_last_length();
        long fixture_last_timeout();
        int fixture_provider_calls();
        void fixture_abi(int version);
        void fixture_ops(int mismatch);
        void fixture_poll(int state);
        void fixture_stream(int mode);
        void fixture_reset();
    }

    private Fixture fixture;
    private Aimux aimux;
    private ExecutorService executor;

    @BeforeEach void setUp() {
        fixture = Native.load(System.getProperty("aimux.testLibrary"), Fixture.class);
        fixture.fixture_reset();
        aimux = Aimux.load(System.getProperty("aimux.testLibrary"));
        executor = Executors.newSingleThreadExecutor(task -> {
            Thread thread = new Thread(task, "aimux-contract-test");
            thread.setDaemon(true);
            return thread;
        });
    }

    @AfterEach void tearDown() throws Exception {
        executor.shutdown();
        assertThat(executor.awaitTermination(3, TimeUnit.SECONDS)).isTrue();
        await(() -> fixture.fixture_live_strings() == 0 && fixture.fixture_live_errors() == 0);
        assertThat(fixture.fixture_live_strings()).isZero();
        assertThat(fixture.fixture_live_errors()).isZero();
    }

    private static void await(BooleanSupplier condition) throws InterruptedException {
        long deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(3);
        while (!condition.getAsBoolean() && System.nanoTime() < deadline) Thread.sleep(2);
        assertThat(condition.getAsBoolean()).as("condition before deadline").isTrue();
    }

    @Test void declaresOnlyTheFourteenPrimitivesWithPortableSizeT() {
        assertThat(Arrays.stream(AimuxFFI.class.getDeclaredMethods()).map(m -> m.getName()))
            .containsExactlyInAnyOrder("aimux_abi_version", "aimux_manifest_json", "aimux_call", "aimux_stream",
                "aimux_operation_push", "aimux_operation_next_part", "aimux_abort", "aimux_drop", "aimux_handle_finalizer",
                "aimux_free_string", "aimux_error_free", "aimux_error_code", "aimux_error_message", "aimux_error_json");
        assertThat(Native.getNativeSize(AimuxFFI.SizeT.class)).isEqualTo(Native.SIZE_T_SIZE);
        assertThatThrownBy(() -> new AimuxFFI.SizeT(-1)).isInstanceOf(IllegalArgumentException.class);
    }

    @Test void rejectsAbiAndOpsMismatchBeforeCreatingAnyProvider() {
        fixture.fixture_abi(2 << 16);
        assertThatThrownBy(() -> Aimux.load(System.getProperty("aimux.testLibrary")))
            .isInstanceOf(IllegalStateException.class).hasMessageContaining("ABI");
        fixture.fixture_abi(Aimux.ABI_VERSION);
        fixture.fixture_ops(1);
        assertThatThrownBy(() -> Aimux.load(System.getProperty("aimux.testLibrary")))
            .isInstanceOf(IllegalStateException.class).hasMessageContaining("ops");
        assertThat(fixture.fixture_provider_calls()).isZero();
    }

    @Test void roundTripsUtf8AndUnsigned64BitHandlesWithoutNumericJson() {
        try (Resource resource = aimux.own("18446744073709551615")) {
            assertThat(resource.getHandle()).isEqualTo("18446744073709551615");
            JsonNode result = resource.call("test.echo", "{\"prompt\":\"你好 🌏\\u0000\"}");
            assertThat(result.path("prompt").asText()).isEqualTo("你好 🌏\0");
            assertThat(fixture.fixture_last_handle()).isEqualTo(-1L);
        }
        assertThatThrownBy(() -> aimux.own("18446744073709551616")).isInstanceOf(IllegalStateException.class);
        assertThatThrownBy(() -> aimux.own("0")).isInstanceOf(IllegalStateException.class);
        assertThatThrownBy(() -> AimuxResult.handle(AimuxResult.parse("9007199254740993")))
            .isInstanceOf(IllegalStateException.class);
    }

    @Test void rejectsInvalidControlJsonBeforeEnteringNativeCode() {
        for (String json : new String[] {"", "{} garbage", "[]", "null", "{broken"}) {
            assertThatThrownBy(() -> aimux.call("test.echo", json)).isInstanceOf(IllegalArgumentException.class);
        }
        assertThatThrownBy(() -> aimux.call("test.echo", null)).isInstanceOf(NullPointerException.class);
        assertThatThrownBy(() -> aimux.call("test\0echo", "{}")).isInstanceOf(IllegalArgumentException.class);
        // Unknown control fields remain available for the native DTO's deny_unknown_fields validation.
        assertThat(aimux.call("test.echo", "{\"unexpected\":true}").path("unexpected").asBoolean()).isTrue();
    }

    @Test void mapsOwnedErrorsFromEnvelopeAndRetainsOpenPayload() {
        Throwable error = catchThrowable(() -> aimux.call("test.fail", "{}"));
        assertThat(error).isInstanceOf(AimuxException.APICallError.class);
        AimuxException api = (AimuxException) error;
        assertThat(api.isRetryable()).isTrue();
        assertThat(api.getStatusCode()).isEqualTo(429);
        assertThat(api.getRetryAfterMs()).isEqualTo(0L);
        assertThat(api.getError().path("responseHeaders").path("x-request-id").asText()).isEqualTo("r1");
        assertThatThrownBy(() -> aimux.call("test.binding", "{}"))
            .isInstanceOf(BindingException.InvalidHandle.class);
        assertThatThrownBy(() -> aimux.call("test.protocol", "{}"))
            .isInstanceOf(ProtocolException.InvalidOp.class);
    }

    @Test void exceptionSubtypesCoverEveryDomainVariantAndUseNameDiscriminator() {
        String[] names = {"AI_Error", "AI_JSONParseError", "AI_InvalidResponseDataError",
            "AI_ToolCallNotFoundForApprovalError", "AI_InvalidArgumentError", "AI_InvalidPromptError",
            "AI_TokenExpiredError", "AI_UnsupportedFunctionalityError", "AI_NoSuchModelError",
            "AI_NoSuchProviderError", "AI_APICallError", "TimeoutError", "AbortError", "AI_RetryError",
            "AI_NoSuchToolError", "AI_InvalidToolInputError", "AI_ToolCallRepairError", "AI_NoOutputGeneratedError",
            "AI_LoadAPIKeyError", "AI_LoadSettingError", "AI_MissingToolResultsError", "AI_ToolChoiceViolationError"};
        java.util.Set<Class<?>> classes = new java.util.HashSet<>();
        for (String name : names) {
            NativeException error = envelope("aimux", 1, name);
            assertThat(error).isInstanceOf(AimuxException.class);
            assertThat(error.getErrorName()).isEqualTo(name);
            classes.add(error.getClass());
        }
        assertThat(classes).hasSize(names.length);
        assertVariants("binding", 200, "NullPointer", "InvalidUtf8", "InvalidWireJson", "InvalidHandle",
            "ReentrantCall", "ResultSerialization", "CallbackFailure");
        assertVariants("protocol", 300, "InvalidOp", "InvalidParams", "UnsupportedOpsVersion",
            "ConcurrentConsumer", "InvalidState", "ResourceLimit", "HandleExhausted");
        assertVariants("recording", 100, "Init", "OpenFile", "Spawn", "WriterGone", "FlushTimeout", "Write");
        NativeException absent = envelope("aimux", 11, "AI_APICallError");
        assertThat(absent.getRetryAfterMs()).isNull();
        assertThatThrownBy(() -> AimuxResult.decode(AimuxResult.parse("{\"kind\":\"aimux\",\"code\":11,\"message\":\"x\","
            + "\"retryAfterMs\":18446744073709551615,\"error\":{\"name\":\"AI_APICallError\"}}"))).isInstanceOf(IllegalStateException.class);
    }

    private static NativeException envelope(String kind, int code, String name) {
        return AimuxResult.decode(AimuxResult.parse("{\"kind\":\"" + kind + "\",\"code\":" + code
            + ",\"message\":\"x\",\"error\":{\"name\":\"" + name + "\",\"extra\":42}}"));
    }

    private static void assertVariants(String kind, int first, String... names) {
        java.util.Set<Class<?>> classes = new java.util.HashSet<>();
        for (int i = 0; i < names.length; i++) {
            NativeException error = envelope(kind, first + i, names[i]);
            assertThat(error.getCode()).isEqualTo(first + i);
            assertThat(error.getError().path("extra").asInt()).isEqualTo(42);
            classes.add(error.getClass());
        }
        assertThat(classes).hasSize(names.length);
    }

    @Test void preservesUnrecognizedVariantsWithinEachErrorDomain() {
        assertThat(envelope("aimux", 21, "AI_NewError")).isInstanceOf(AimuxException.Unknown.class);
        assertThat(envelope("binding", 207, "NewBindingError")).isInstanceOf(BindingException.Unknown.class);
        assertThat(envelope("protocol", 307, "NewProtocolError")).isInstanceOf(ProtocolException.Unknown.class);
        NativeException error = envelope("recording", 106, "NewRecordingError");
        assertThat(error).isInstanceOf(RecordingException.Unknown.class);
        assertThat(error.getErrorName()).isEqualTo("NewRecordingError");
        assertThat(error.getError().path("extra").asInt()).isEqualTo(42);
    }

    @Test void closeFromConsumerDoesNotWaitAndDropsAfterNativeReturn() {
        Resource resource = aimux.own("120");
        AtomicInteger delivered = new AtomicInteger();
        assertTimeoutPreemptively(Duration.ofSeconds(2), () -> resource.stream("text.stream", "{}", frame -> {
            delivered.incrementAndGet();
            resource.close();
            resource.close();
            assertThat(fixture.fixture_drops(120)).isZero();
        }));
        assertThat(delivered.get()).isEqualTo(3);
        assertThat(fixture.fixture_drops(120)).isEqualTo(1);
        assertThat(fixture.fixture_drop_in_callback()).isZero();
        assertThatThrownBy(() -> resource.call("model.info", "{}")).isInstanceOf(IllegalStateException.class);
    }

    @Test void copiesFramesAndRetainsConsumerThroughSlowCallbacksAndGc() {
        List<StreamFrame> frames = new ArrayList<>();
        try (Resource resource = aimux.own("121")) {
            resource.stream("text.stream", "{}", frame -> {
                frames.add(frame);
                System.gc();
                try { Thread.sleep(20); } catch (InterruptedException e) { throw new AssertionError(e); }
                assertThat(fixture.fixture_callbacks()).isEqualTo(frames.size() - 1);
            });
        }
        assertThat(frames).hasSize(3);
        assertThat(DtoJson.MAPPER.<JsonNode>valueToTree(((StreamFrame.Event) frames.get(0)).getEvent()).path("text").asText()).isEqualTo("你好 🌏");
        assertThat(((StreamFrame.End) frames.get(2)).getOutcome()).isEqualTo(Outcome.SUCCESS);
    }

    @Test void latchesConsumerThrowableSignalsAbortAndThrowsOnceAfterNativeReturns() {
        AssertionError cause = new AssertionError("consumer broke");
        AtomicInteger deliveries = new AtomicInteger();
        try (Resource resource = aimux.own("122"); Abort abort = aimux.createAbort()) {
            Throwable thrown = catchThrowable(() -> resource.stream("text.stream", "{}", abort, frame -> {
                deliveries.incrementAndGet();
                throw cause;
            }));
            assertThat(thrown).isInstanceOf(BindingException.CallbackFailure.class).hasCause(cause);
            assertThat(fixture.fixture_callbacks()).isEqualTo(3); // native returned after all three callbacks
            assertThat(deliveries.get()).isEqualTo(1);
            assertThat(fixture.fixture_signals(Integer.parseInt(abort.getHandle()))).isEqualTo(1);
        }
    }

    @Test void callbackCanSignalAbortWithoutClosingTheModel() {
        try (Resource resource = aimux.own("123"); Abort abort = aimux.createAbort()) {
            assertThatThrownBy(() -> resource.stream("text.stream", "{}", abort, frame -> abort.signal()))
                .isInstanceOf(CancellationException.class);
            assertThat(resource.call("test.echo", "{\"stillOpen\":true}").path("stillOpen").asBoolean()).isTrue();
        }
    }

    @Test void reportsPreacceptanceAndTerminalErrorsAndInvalidNativeTermination() {
        try (Resource resource = aimux.own("124")) {
            fixture.fixture_stream(1);
            assertThatThrownBy(() -> resource.stream("text.stream", "{}", frame -> fail("no frame expected")))
                .isInstanceOf(ProtocolException.InvalidOp.class);
            fixture.fixture_stream(3);
            List<StreamFrame> frames = new ArrayList<>();
            assertThatThrownBy(() -> resource.stream("text.stream", "{}", frames::add))
                .isInstanceOf(AimuxException.TimeoutError.class);
            assertThat(frames).hasSize(2);
            fixture.fixture_stream(4);
            assertThatThrownBy(() -> resource.stream("text.stream", "{}", frame -> {}))
                .isInstanceOf(IllegalStateException.class).hasMessageContaining("without End");
            fixture.fixture_stream(5);
            assertThatThrownBy(() -> resource.stream("text.stream", "{}", frame -> {}))
                .isInstanceOf(BindingException.CallbackFailure.class);
        }
    }

    @Test void cancellationOfRunningCallSignalsNativeAbortWithoutInterruptingThread() throws Exception {
        try (Resource resource = aimux.own("125")) {
            Future<JsonNode> future = resource.callAsync("test.wait", "{}", executor);
            await(() -> fixture.fixture_entered() == 1);
            resource.close();
            assertThat(fixture.fixture_drops(125)).isZero();
            assertThat(future.cancel(false)).isTrue();
            assertThat(future.cancel(true)).isFalse();
            assertThatThrownBy(future::get).isInstanceOf(CancellationException.class);
            await(() -> fixture.fixture_drops(125) == 1);
        }
    }

    @Test void cancellationOfRunningStreamAndCloseHaveIndependentLifetimes() throws Exception {
        fixture.fixture_stream(2);
        Resource resource = aimux.own("126");
        CountDownLatch callback = new CountDownLatch(1);
        Future<JsonNode> future = resource.streamAsync("text.stream", "{}", f -> callback.countDown(), executor);
        assertThat(callback.await(2, TimeUnit.SECONDS)).isTrue();
        resource.close();
        assertThat(fixture.fixture_drops(126)).isZero();
        assertThat(future.cancel(true)).isTrue();
        await(() -> fixture.fixture_drops(126) == 1);
        assertThat(fixture.fixture_callbacks()).isEqualTo(3);
    }

    @Test void cancellationBeforeExecutorStartsReleasesQueuedAdmissionAndNeverCallsNative() {
        AtomicReference<Runnable> queued = new AtomicReference<>();
        Resource resource = aimux.own("127");
        Future<JsonNode> future = resource.callAsync("test.wait", "{}", queued::set);
        resource.close();
        assertThat(fixture.fixture_drops(127)).isZero();
        assertThat(future.cancel(true)).isTrue();
        queued.get().run();
        assertThat(fixture.fixture_entered()).isZero();
        assertThat(fixture.fixture_drops(127)).isEqualTo(1);
    }

    @Test void executorRejectionCleansAbortAndAdmission() {
        Resource resource = aimux.own("128");
        assertThatThrownBy(() -> resource.callAsync("test.wait", "{}", r -> {
            throw new RejectedExecutionException("full");
        })).isInstanceOf(RejectedExecutionException.class);
        resource.close();
        assertThat(fixture.fixture_drops(128)).isEqualTo(1);
        assertThat(fixture.fixture_entered()).isZero();
    }

    @Test void directExecutorAndCompletedFutureDoNotSignalSpuriousCancellation() throws Exception {
        Future<JsonNode> future = aimux.callAsync("test.echo", "{\"value\":42}", Runnable::run);
        assertThat(future.get().path("value").asInt()).isEqualTo(42);
        assertThat(future.cancel(true)).isFalse();
    }

    @Test void closeAndSignalOfSharedAbortWaitOnlyForAdmittedNativeUse() throws Exception {
        Resource resource = aimux.own("129");
        Abort abort = aimux.createAbort();
        CountDownLatch hold = new CountDownLatch(1);
        CountDownLatch entered = new CountDownLatch(1);
        Future<?> stream = executor.submit(() -> resource.stream("text.stream", "{}", abort, frame -> {
            if (frame instanceof StreamFrame.End) return;
            entered.countDown();
            try { if (!hold.await(2, TimeUnit.SECONDS)) throw new AssertionError("deadline"); }
            catch (InterruptedException error) { throw new AssertionError(error); }
        }));
        assertThat(entered.await(2, TimeUnit.SECONDS)).isTrue();
        abort.close();
        resource.close();
        assertThat(fixture.fixture_drops(Integer.parseInt(abort.getHandle()))).isZero();
        hold.countDown();
        stream.get(2, TimeUnit.SECONDS);
        assertThat(fixture.fixture_drops(Integer.parseInt(abort.getHandle()))).isEqualTo(1);
        assertThat(fixture.fixture_drops(129)).isEqualTo(1);
    }

    @Test void allEightModalitiesAndFilesUseTheSameOwnedProviderPrimitives() {
        try (ProviderHandle provider = aimux.createProvider("{\"package\":\"@ai-sdk/openai\",\"settings\":{}}")) {
            try (Model m = provider.languageModel("m")) { assertThat(m.info()).isNotNull(); }
            try (EmbeddingModel m = provider.embeddingModel("m")) {
                assertThat(m.embed("{\"value\":\"x\"}").path("value").asText()).isEqualTo("x");
                assertThat(m.embedMany("{\"values\":[\"a\",\"b\"]}").path("values")).hasSize(2);
            }
            try (ImageModel m = provider.imageModel("m")) { assertThat(m.generate("{}")).isNotNull(); }
            try (SpeechModel m = provider.speechModel("m")) { assertThat(m.generate("{}")).isNotNull(); }
            try (TranscriptionModel m = provider.transcriptionModel("m")) { assertThat(m.generate("{}")).isNotNull(); }
            try (RerankingModel m = provider.rerankingModel("m")) { assertThat(m.rerank("{}")).isNotNull(); }
            try (VideoModel m = provider.videoModel("m")) { assertThat(m.generate("{}")).isNotNull(); }
            try (SearchModel m = provider.searchModel("m")) { assertThat(m.search("{}")).isNotNull(); }
            try (DecisionModel m = provider.decisionModel("m")) { assertThat(m.capabilities()).isNotNull(); }
            try (Files files = provider.files()) { assertThat(files.upload("{}")).isNotNull(); }
        }
    }

    @Test void typedFacadeUsesCoreResultsAndForwardsFutureCancellation() throws Exception {
        try (TypedModel typed = new TypedModel(new Model(aimux, 130))) {
            assertThat(typed.generate("{}").getText()).isEqualTo("你好 🌏");
            assertThat(typed.generateAsync("{}", executor).get(2, TimeUnit.SECONDS).getText()).isEqualTo("你好 🌏");
            AtomicInteger parts = new AtomicInteger();
            typed.stream("{}", part -> parts.incrementAndGet());
            assertThat(parts.get()).isEqualTo(2);
        }
    }

    @Test void binaryPushAndPollUseNativeSizeTAndSignedInt64Timeout() {
        try (TranscriptionSession operation = new TranscriptionSession(aimux, 131)) {
            operation.push(new byte[] {42, 0, -1});
            assertThat(fixture.fixture_last_length()).isEqualTo(3);
            operation.push(new byte[0]);
            assertThat(fixture.fixture_last_length()).isZero();
            assertThat(operation.next(-1).getState()).isEqualTo(TranscriptionSession.PollResult.State.PART);
            assertThat(fixture.fixture_last_timeout()).isEqualTo(-1);
            fixture.fixture_poll(3);
            assertThat(operation.next(0).getState()).isEqualTo(TranscriptionSession.PollResult.State.TIMEOUT);
            fixture.fixture_poll(2);
            assertThat(operation.next(Long.MAX_VALUE).getState()).isEqualTo(TranscriptionSession.PollResult.State.ENDED);
            assertThat(fixture.fixture_last_timeout()).isEqualTo(Long.MAX_VALUE);
            fixture.fixture_poll(0);
            assertThatThrownBy(() -> operation.next(0)).isInstanceOf(ProtocolException.InvalidState.class);
        }
    }
}
