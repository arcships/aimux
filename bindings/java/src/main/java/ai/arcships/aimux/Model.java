package ai.arcships.aimux;

import com.sun.jna.Pointer;

import java.io.Closeable;
import java.util.Spliterator;
import java.util.concurrent.LinkedBlockingQueue;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.locks.ReentrantReadWriteLock;
import java.util.function.Consumer;
import java.util.stream.Stream;
import java.util.stream.StreamSupport;

/**
 * A model instance backed by a Rust {@code Arc<dyn LanguageModel>}.
 *
 * <p>This is the raw JSON layer of the aimux Java binding: it wraps the
 * aimux-ffi C ABI via JNA and exposes the same wire format as the Kotlin
 * binding (JSON in, JSON out). Failures throw {@link AimuxException} (typed
 * subclasses) from the C {@code AimuxError} out-param.
 *
 * <p>Implements {@link Closeable} — you MUST call {@link #close()} (or use a
 * try-with-resources block) to release the native handle and avoid memory
 * leaks. {@link #finalize()} is only a best-effort backstop and is
 * unreliable; try-with-resources is the primary path.
 *
 * <pre>{@code
 * try (Model model = Model.openai("sk-...", "gpt-4o-mini")) {
 *     String result = model.generateText("\"Hello!\"");
 * }
 * }</pre>
 *
 * <p><b>Thread-safety / concurrency.</b> Model is safe for concurrent use.
 * It guards the native handle with a Go-style {@link ReentrantReadWriteLock}
 * (fair, FIFO): every FFI call ({@link #generateText}, {@link #streamText},
 * streaming variants) holds the <em>read</em> lock for its entire duration,
 * and {@link #close()} takes the <em>write</em> lock. As a result {@code close()}
 * blocks until all in-flight calls finish before dropping the native handle —
 * this closes the use-after-free race where a caller could observe a non-zero
 * handle and then race with {@code close()}'s drop. Because a streaming call
 * holds the read lock until the stream completes, {@code close()} will not
 * interrupt or drop a handle out from under an active stream. Do not call
 * {@code close()} from within a stream callback (would self-deadlock).
 */
public class Model implements Closeable {

    // Go-style read/write lock: every FFI call holds the read lock for its
    // entire duration; close() takes the write lock and thus blocks until all
    // in-flight calls finish before dropping the native handle. This closes the
    // read-then-drop use-after-free race the AtomicLong version had (a reader
    // could observe a non-zero handle, then race with close()'s getAndSet+drop).
    // Fair (FIFO) so a pending close is not starved by barging readers — matches
    // Go's sync.RWMutex writer-priority semantics.
    private final ReentrantReadWriteLock lock = new ReentrantReadWriteLock(true);
    private long handle;
    private boolean closed;

    // Package-private: ProviderHandle.model() (same package) needs to construct
    // a Model; external callers can still only go through the static factories
    // openai()/provider()/mockReplay().
    Model(long handle) {
        this.handle = handle;
    }

    /**
     * Release the native handle. Idempotent and thread-safe: subsequent calls
     * are no-ops, and every other method throws {@link IllegalStateException}.
     *
     * <p>Acquires the write lock and therefore <em>blocks until all in-flight
     * FFI calls</em> (which hold the read lock) finish before dropping the
     * native handle — prevents a use-after-free race between a concurrent
     * caller and {@code close()}. A streaming call holds the read lock for the
     * entire stream, so {@code close()} blocks until the stream completes. Do
     * not call {@code close()} from within a stream callback (would
     * self-deadlock).
     */
    @Override
    public void close() {
        lock.writeLock().lock();
        try {
            if (closed || handle == 0L) {
                return;
            }
            long h = handle;
            handle = 0L;
            closed = true;
            AimuxFFI.INSTANCE.aimux_drop_handle(h);
        } finally {
            lock.writeLock().unlock();
        }
    }

    /** Best-effort backstop; try-with-resources is the primary release path. */
    @Override
    protected void finalize() throws Throwable {
        close();
        super.finalize();
    }

    // Caller MUST already hold the read lock (each public FFI method acquires
    // it and releases it in a finally after the FFI call returns). Holding the
    // read lock across the FFI call is what lets close()'s write lock wait for
    // the call to finish, closing the use-after-free race.
    private long requireHandleLocked() {
        if (closed || handle == 0L) {
            throw new IllegalStateException("Model is closed");
        }
        return handle;
    }

    /** Package-private handle read for composite-model factories (router/moa). */
    long handle() {
        readLock.lock();
        try {
            return requireHandleLocked();
        } finally {
            readLock.unlock();
        }
    }

    // ── Provider constructors ──────────────────────────────────────────────

    /** Create an OpenAI model instance. */
    public static Model openai(String apiKey, String modelId) {
        AimuxCError err = AimuxResult.newError();
        long h = AimuxFFI.INSTANCE.aimux_openai_new(apiKey, modelId, err);
        return new Model(AimuxResult.extractHandle(h, err, "Failed to create OpenAI model"));
    }

    /** Create an OpenAI model instance with a custom base URL. */
    public static Model openaiWithBase(String apiKey, String modelId, String baseUrl) {
        AimuxCError err = AimuxResult.newError();
        long h = AimuxFFI.INSTANCE.aimux_openai_new_with_base(apiKey, modelId, baseUrl, err);
        return new Model(AimuxResult.extractHandle(h, err, "Failed to create OpenAI model"));
    }

    /** Create an Anthropic model instance. */
    public static Model anthropic(String apiKey, String modelId) {
        AimuxCError err = AimuxResult.newError();
        long h = AimuxFFI.INSTANCE.aimux_anthropic_new(apiKey, modelId, err);
        return new Model(AimuxResult.extractHandle(h, err, "Failed to create Anthropic model"));
    }

    /** Create an Anthropic model instance with a custom base URL. */
    public static Model anthropicWithBase(String apiKey, String modelId, String baseUrl) {
        AimuxCError err = AimuxResult.newError();
        long h = AimuxFFI.INSTANCE.aimux_anthropic_new_with_base(apiKey, modelId, baseUrl, err);
        return new Model(AimuxResult.extractHandle(h, err, "Failed to create Anthropic model"));
    }

    /** Create a Cohere model instance. */
    public static Model cohere(String apiKey, String modelId) {
        AimuxCError err = AimuxResult.newError();
        long h = AimuxFFI.INSTANCE.aimux_cohere_new(apiKey, modelId, err);
        return new Model(AimuxResult.extractHandle(h, err, "Failed to create Cohere model"));
    }

    /** Create a Cohere model instance with a custom base URL. */
    public static Model cohereWithBase(String apiKey, String modelId, String baseUrl) {
        AimuxCError err = AimuxResult.newError();
        long h = AimuxFFI.INSTANCE.aimux_cohere_new_with_base(apiKey, modelId, baseUrl, err);
        return new Model(AimuxResult.extractHandle(h, err, "Failed to create Cohere model"));
    }

    /** Create a Mistral model instance. */
    public static Model mistral(String apiKey, String modelId) {
        AimuxCError err = AimuxResult.newError();
        long h = AimuxFFI.INSTANCE.aimux_mistral_new(apiKey, modelId, err);
        return new Model(AimuxResult.extractHandle(h, err, "Failed to create Mistral model"));
    }

    /** Create a Mistral model instance with a custom base URL. */
    public static Model mistralWithBase(String apiKey, String modelId, String baseUrl) {
        AimuxCError err = AimuxResult.newError();
        long h = AimuxFFI.INSTANCE.aimux_mistral_new_with_base(apiKey, modelId, baseUrl, err);
        return new Model(AimuxResult.extractHandle(h, err, "Failed to create Mistral model"));
    }

    /** Create an xAI model instance. */
    public static Model xai(String apiKey, String modelId) {
        AimuxCError err = AimuxResult.newError();
        long h = AimuxFFI.INSTANCE.aimux_xai_new(apiKey, modelId, err);
        return new Model(AimuxResult.extractHandle(h, err, "Failed to create xAI model"));
    }

    /** Create an xAI model instance with a custom base URL. */
    public static Model xaiWithBase(String apiKey, String modelId, String baseUrl) {
        AimuxCError err = AimuxResult.newError();
        long h = AimuxFFI.INSTANCE.aimux_xai_new_with_base(apiKey, modelId, baseUrl, err);
        return new Model(AimuxResult.extractHandle(h, err, "Failed to create xAI model"));
    }

    /** Create a Bedrock model instance (AWS SigV4 credentials). */
    public static Model bedrock(String accessKeyId, String secretAccessKey, String region, String modelId) {
        AimuxCError err = AimuxResult.newError();
        long h = AimuxFFI.INSTANCE.aimux_bedrock_new(accessKeyId, secretAccessKey, region, modelId, err);
        return new Model(AimuxResult.extractHandle(h, err, "Failed to create Bedrock model"));
    }

    /** Create a Bedrock model instance with a custom base URL. */
    public static Model bedrockWithBase(String accessKeyId, String secretAccessKey, String region,
                                        String modelId, String baseUrl) {
        AimuxCError err = AimuxResult.newError();
        long h = AimuxFFI.INSTANCE.aimux_bedrock_new_with_base(
            accessKeyId, secretAccessKey, region, modelId, baseUrl, err);
        return new Model(AimuxResult.extractHandle(h, err, "Failed to create Bedrock model"));
    }

    /** Create a Vertex AI model instance (GCP bearer token). */
    public static Model vertex(String accessToken, String project, String location, String modelId) {
        AimuxCError err = AimuxResult.newError();
        long h = AimuxFFI.INSTANCE.aimux_vertex_new(accessToken, project, location, modelId, err);
        return new Model(AimuxResult.extractHandle(h, err, "Failed to create Vertex model"));
    }

    /** Create a Vertex AI model instance with a custom base URL. */
    public static Model vertexWithBase(String accessToken, String project, String location,
                                       String modelId, String baseUrl) {
        AimuxCError err = AimuxResult.newError();
        long h = AimuxFFI.INSTANCE.aimux_vertex_new_with_base(
            accessToken, project, location, modelId, baseUrl, err);
        return new Model(AimuxResult.extractHandle(h, err, "Failed to create Vertex model"));
    }

    /** Create an Anthropic-on-AWS model instance (API key + region). */
    public static Model anthropicAws(String apiKey, String region, String modelId) {
        AimuxCError err = AimuxResult.newError();
        long h = AimuxFFI.INSTANCE.aimux_anthropic_aws_new(apiKey, region, modelId, err);
        return new Model(AimuxResult.extractHandle(h, err, "Failed to create Anthropic AWS model"));
    }

    /** Create an Anthropic-on-AWS model instance with a custom base URL. */
    public static Model anthropicAwsWithBase(String apiKey, String region, String modelId, String baseUrl) {
        AimuxCError err = AimuxResult.newError();
        long h = AimuxFFI.INSTANCE.aimux_anthropic_aws_new_with_base(apiKey, region, modelId, baseUrl, err);
        return new Model(AimuxResult.extractHandle(h, err, "Failed to create Anthropic AWS model"));
    }

    /** Create an Azure OpenAI model instance (API key + resource name). */
    public static Model azure(String apiKey, String resourceName, String deployment) {
        AimuxCError err = AimuxResult.newError();
        long h = AimuxFFI.INSTANCE.aimux_azure_new(apiKey, resourceName, deployment, null, err);
        return new Model(AimuxResult.extractHandle(h, err, "Failed to create Azure model"));
    }

    /** Create an Azure OpenAI model instance with an explicit api-version. */
    public static Model azureWithVersion(String apiKey, String resourceName, String deployment,
                                         String apiVersion) {
        AimuxCError err = AimuxResult.newError();
        long h = AimuxFFI.INSTANCE.aimux_azure_new(apiKey, resourceName, deployment, apiVersion, err);
        return new Model(AimuxResult.extractHandle(h, err, "Failed to create Azure model"));
    }

    /** Create an Azure OpenAI model instance with a custom base URL. */
    public static Model azureWithBase(String apiKey, String baseUrl, String deployment) {
        AimuxCError err = AimuxResult.newError();
        long h = AimuxFFI.INSTANCE.aimux_azure_new_with_base(apiKey, baseUrl, deployment, null, err);
        return new Model(AimuxResult.extractHandle(h, err, "Failed to create Azure model"));
    }

    /**
     * Create a model from the provider registry by name (RFC-0017 phase 4).
     *
     * @param name       Registry provider name (e.g. "deepseek", "groq").
     * @param apiKey     API key, or {@code null} to read the provider's env var
     *                   from the registry entry.
     * @param modelId    Model id.
     * @param configJson Optional JSON object of ProviderOptions
     *                   ({@code {"base_url": "...", "headers": {...}, "max_retries": 0,
     *                   "body_overrides": {...}}}); {@code null} for defaults.
     * @return A new {@link Model}.
     * @throws AimuxException if the provider could not be constructed
     *                        (unknown provider, bad config, missing env key).
     */
    public static Model provider(String name, String apiKey, String modelId, String configJson) {
        AimuxCError err = AimuxResult.newError();
        long h = AimuxFFI.INSTANCE.aimux_provider_new(name, apiKey, modelId, configJson, err);
        return new Model(AimuxResult.extractHandle(h, err, "Failed to create provider model: " + name));
    }

    /**
     * Create a model from the provider registry, reading the API key from the
     * provider's env var (RFC-0017 phase 4).
     */
    public static Model providerFromEnv(String name, String modelId) {
        AimuxCError err = AimuxResult.newError();
        long h = AimuxFFI.INSTANCE.aimux_provider_from_env(name, modelId, err);
        return new Model(AimuxResult.extractHandle(h, err, "Failed to create provider model from env: " + name));
    }

    /**
     * Create a <strong>provider handle</strong> (RFC-0027) for a registry-backed provider.
     *
     * <p>Unlike {@link #provider(String, String, String, String)} (which binds to
     * a single modelId), this returns a {@link ProviderHandle} that supports
     * {@link ProviderHandle#listModels()} and {@link ProviderHandle#model(String)}.
     */
    public static ProviderHandle createProvider(String name, String apiKey, String configJson) {
        AimuxCError err = AimuxResult.newError();
        long h = AimuxFFI.INSTANCE.aimux_provider_handle_new(name, apiKey, configJson, err);
        return new ProviderHandle(AimuxResult.extractHandle(h, err, "Failed to create provider handle: " + name));
    }

    /**
     * Fetch the community model catalogue (anya2a). Returns a JSON-serialized
     * Catalogue string. Thin fetch — no caching.
     *
     * @param sourceUrl Optional URL override (null = default endpoint).
     */
    public static String getModelSpecs(String sourceUrl) {
        AimuxCError err = AimuxResult.newError();
        return AimuxResult.extractString(
            AimuxFFI.INSTANCE.aimux_get_model_specs(sourceUrl, err),
            err,
            "get_model_specs");
    }

    /**
     * Create a DeepSeek model instance.
     *
     * <p>The retired {@code aimux_deepseek_new} C symbol has been removed; this
     * now routes through {@link #provider(String, String, String, String)} with
     * the registry name {@code "deepseek"} (RFC-0017 phase 4).
     */
    public static Model deepseek(String apiKey, String modelId) {
        return provider("deepseek", apiKey, modelId, null);
    }

    /**
     * Create a mock replay model from recorded JSONL (RFC-0023). The returned
     * model's {@code generateText} / {@code streamText} calls replay recorded
     * responses from {@code recordingsJsonl} (one Recording per line) instead of
     * sending real API requests.
     *
     * @param recordingsJsonl Recorded JSONL (one Recording per line).
     * @return A new {@link Model} backed by the replay handle.
     * @throws AimuxException if the mock replay model could not be constructed.
     */
    public static Model mockReplay(String recordingsJsonl) {
        AimuxCError err = AimuxResult.newError();
        long h = AimuxFFI.INSTANCE.aimux_mock_replay_new(recordingsJsonl, err);
        return new Model(AimuxResult.extractHandle(h, err, "Failed to create mock replay model"));
    }

    // ── Composite models (RFC-0021 / RFC-0022) ──────────────────────────────

    /**
     * Create a RouterModel (RFC-0021) over the given child models. The returned
     * model routes each call to one child and falls back across the rest on
     * error (per {@code configJson}).
     *
     * @param models     child models (must be non-empty; closed models throw).
     * @param configJson optional config: {@code {"router": "rule"|"weighted",
     *                   "weights": [...], "fallback": "on_error"|"none",
     *                   "provider_name", "model_id"}} — all optional.
     * @return a new RouterModel wrapping the children.
     */
    public static Model router(java.util.List<Model> models, String configJson) {
        if (models == null || models.isEmpty()) {
            throw new IllegalArgumentException("router: models must be non-empty");
        }
        long[] handles = new long[models.size()];
        for (int i = 0; i < models.size(); i++) {
            handles[i] = models.get(i).handle();
        }
        AimuxCError err = AimuxResult.newError();
        long h = AimuxFFI.INSTANCE.aimux_router_new(handles, handles.length, configJson, err);
        return new Model(AimuxResult.extractHandle(h, err, "Failed to create router model"));
    }

    /**
     * Create a MoaModel (RFC-0022) over reference models + one aggregator.
     * References fan out in parallel, then the aggregator synthesizes a final
     * answer.
     *
     * @param references reference models (may be null/empty — runs aggregator only).
     * @param aggregator the aggregator model (must be non-null and open).
     * @param configJson optional MoaConfig: {@code {"provider_name", "model_id",
     *                   "aggregator_instructions", "strip_reference_tools",
     *                   "fail_mode": "best_effort"|"fail_fast"}}.
     * @return a new MoaModel.
     */
    public static Model moa(java.util.List<Model> references, Model aggregator, String configJson) {
        if (aggregator == null) {
            throw new IllegalArgumentException("moa: aggregator must be non-null");
        }
        long[] refHandles;
        if (references == null || references.isEmpty()) {
            refHandles = new long[0];
        } else {
            refHandles = new long[references.size()];
            for (int i = 0; i < references.size(); i++) {
                refHandles[i] = references.get(i).handle();
            }
        }
        AimuxCError err = AimuxResult.newError();
        long h = AimuxFFI.INSTANCE.aimux_moa_new(
                refHandles, refHandles.length, aggregator.handle(), configJson, err);
        return new Model(AimuxResult.extractHandle(h, err, "Failed to create moa model"));
    }

    // ── Generation ─────────────────────────────────────────────────────────

    /**
     * Generate text (non-streaming).
     *
     * @param promptJson JSON prompt string (bare value or {@code {"prompt": ...}}).
     * @param optsJson   Optional JSON-serialized options, or {@code null} for defaults.
     * @return JSON-serialized result.
     * @throws AimuxException on engine / transport failure.
     */
    public String generateText(String promptJson, String optsJson) {
        lock.readLock().lock();
        try {
            long h = requireHandleLocked();
            AimuxCError err = AimuxResult.newError();
            return AimuxResult.extractString(
                AimuxFFI.INSTANCE.aimux_generate_text(h, promptJson, optsJson, err),
                err,
                "generate_text");
        } finally {
            lock.readLock().unlock();
        }
    }

    /** Generate text with default options. */
    public String generateText(String promptJson) {
        return generateText(promptJson, null);
    }

    /**
     * Generate a structured JSON object (M12, RFC-0016).
     *
     * <p>Same signature as {@link #generateText}; returns a JSON-serialized
     * {@code GenerateObjectResult}. Pass {@code response_format: { "Json": { ... } }}
     * via {@code optsJson} for schema control; the engine applies JSON repair
     * before parsing.
     *
     * @param promptJson JSON prompt string (bare value or {@code {"prompt": ...}}).
     * @param optsJson   Optional JSON-serialized options, or {@code null} for defaults.
     * @return JSON-serialized GenerateObjectResult.
     * @throws AimuxException on engine / transport failure.
     */
    public String generateObject(String promptJson, String optsJson) {
        lock.readLock().lock();
        try {
            long h = requireHandleLocked();
            AimuxCError err = AimuxResult.newError();
            return AimuxResult.extractString(
                AimuxFFI.INSTANCE.aimux_generate_object(h, promptJson, optsJson, err),
                err,
                "generate_object");
        } finally {
            lock.readLock().unlock();
        }
    }

    /** Generate a structured JSON object with default options. */
    public String generateObject(String promptJson) {
        return generateObject(promptJson, null);
    }

    /**
     * Consume a stream to completion and return the aggregated result
     * (M11, RFC-0016). Synchronous (blocks until the stream finishes).
     *
     * <p>Same signature as {@link #generateText}; returns a JSON-serialized
     * {@code StreamTextResultAggregated}.
     *
     * @param promptJson JSON prompt string (bare value or {@code {"prompt": ...}}).
     * @param optsJson   Optional JSON-serialized options, or {@code null} for defaults.
     * @return JSON-serialized StreamTextResultAggregated.
     * @throws AimuxException on engine / transport failure.
     */
    public String consumeStreamText(String promptJson, String optsJson) {
        lock.readLock().lock();
        try {
            long h = requireHandleLocked();
            AimuxCError err = AimuxResult.newError();
            return AimuxResult.extractString(
                AimuxFFI.INSTANCE.aimux_consume_stream_text(h, promptJson, optsJson, err),
                err,
                "consume_stream_text");
        } finally {
            lock.readLock().unlock();
        }
    }

    /** Consume a stream to completion with default options. */
    public String consumeStreamText(String promptJson) {
        return consumeStreamText(promptJson, null);
    }

    /**
     * Stream text from the model. Blocks the calling thread until the stream
     * completes (same contract as the Kotlin/Go bindings).
     *
     * <p>The JNA {@code Callback} proxies are held in local variables for the
     * duration of the native call so the JVM cannot GC them mid-stream.
     * Callbacks run on the calling thread; do NOT re-enter the FFI layer from
     * inside a callback (would deadlock the tokio runtime).
     *
     * <p>C ABI has no {@code on_error} callback — terminal failures throw
     * {@link AimuxException} after the blocking call returns.
     *
     * @param promptJson JSON prompt string.
     * @param optsJson   Optional JSON-serialized options, or {@code null} for defaults.
     * @param onPart     Called for each stream part (JSON string).
     * @param onDone     Called once when the stream ends normally.
     * @throws AimuxException when the stream fails (return status 0).
     */
    public void streamText(String promptJson, String optsJson,
                           final Consumer<String> onPart,
                           final Runnable onDone) {
        // JNA callbacks — must be held in local variables to prevent GC.
        // C signatures: on_part(json, stream_ctx), on_done(stream_ctx).
        final AimuxFFI.StreamPartCallback partCb = new AimuxFFI.StreamPartCallback() {
            @Override
            public void invoke(Pointer jsonPtr, Pointer streamCtx) {
                if (jsonPtr != null) {
                    onPart.accept(jsonPtr.getString(0, "UTF-8"));
                }
            }
        };
        final AimuxFFI.StreamDoneCallback doneCb = new AimuxFFI.StreamDoneCallback() {
            @Override
            public void invoke(Pointer streamCtx) {
                onDone.run();
            }
        };

        // Hold the read lock for the whole blocking stream so close() cannot
        // drop the handle mid-stream (it blocks on the write lock until the
        // stream completes).
        lock.readLock().lock();
        try {
            long h = requireHandleLocked();
            AimuxCError err = AimuxResult.newError();
            int rc = AimuxFFI.INSTANCE.aimux_stream_text(
                h, promptJson, optsJson, partCb, doneCb, null, err);
            if (rc == 0) {
                // Prefer throw over onError for terminal C failures.
                throw AimuxException.fromC(err);
            }
        } finally {
            lock.readLock().unlock();
        }
    }

    /**
     * Stream text as a lazy {@link Stream} of stream-part JSON strings.
     *
     * <p>The FFI call starts on the first terminal operation of the returned
     * stream (mirror of Kotlin's {@code streamTextSequence}). Iteration pulls
     * parts from a {@link LinkedBlockingQueue} fed by the stream callbacks;
     * the stream ends at the sentinel. Terminal stream failures throw
     * {@link AimuxException} from the blocking FFI call.
     *
     * <pre>{@code
     * model.streamTextStream("\"Write a haiku\"").forEach(System.out::println);
     * }</pre>
     *
     * @param promptJson JSON prompt string.
     * @param optsJson   Optional JSON-serialized options, or {@code null} for defaults.
     */
    public Stream<String> streamTextStream(final String promptJson, final String optsJson) {
        // Sentinel for end-of-stream. Java's LinkedBlockingQueue rejects null
        // elements, so use a unique object instead of Kotlin's null sentinel.
        final Object END = new Object();
        return StreamSupport.stream(
            new java.util.Spliterators.AbstractSpliterator<String>(
                Long.MAX_VALUE, Spliterator.ORDERED) {
                private final LinkedBlockingQueue<Object> parts =
                    new LinkedBlockingQueue<>();
                private final AtomicBoolean started = new AtomicBoolean(false);
                private boolean exhausted;

                @Override
                public boolean tryAdvance(Consumer<? super String> action) {
                    if (!started.getAndSet(true)) {
                        try {
                            streamText(promptJson, optsJson,
                                parts::add,
                                () -> parts.add(END));
                        } catch (RuntimeException e) {
                            parts.add(END); // prevent a hang on later iteration
                            throw e;
                        }
                    }
                    if (exhausted) {
                        return false;
                    }
                    final Object part;
                    try {
                        part = parts.take();
                    } catch (InterruptedException e) {
                        Thread.currentThread().interrupt();
                        throw new RuntimeException("interrupted while streaming", e);
                    }
                    if (part == END) {
                        exhausted = true;
                        return false;
                    }
                    action.accept((String) part);
                    return true;
                }
            },
            false);
    }

    /** Stream text with default options. */
    public Stream<String> streamTextStream(final String promptJson) {
        return streamTextStream(promptJson, null);
    }

    // ── OpenAI-compatible output (RFC-0026) ─────────────────────────────────

    /**
     * Generate text (non-streaming) with OpenAI Chat Completions output.
     *
     * @param promptJson JSON prompt string (bare value or {@code {"prompt": ...}}).
     * @param optsJson   Optional JSON-serialized options, or {@code null} for defaults.
     * @return JSON-serialized ChatCompletion.
     * @throws AimuxException on engine / transport failure.
     */
    public String generateTextAsOpenAI(String promptJson, String optsJson) {
        lock.readLock().lock();
        try {
            long h = requireHandleLocked();
            AimuxCError err = AimuxResult.newError();
            return AimuxResult.extractString(
                AimuxFFI.INSTANCE.aimux_generate_text_as_openai(h, promptJson, optsJson, err),
                err,
                "generate_text_as_openai");
        } finally {
            lock.readLock().unlock();
        }
    }

    /** Generate text with OpenAI output and default options. */
    public String generateTextAsOpenAI(String promptJson) {
        return generateTextAsOpenAI(promptJson, null);
    }

    /**
     * Stream text from the model with OpenAI Chat Completions output. Blocks the
     * calling thread until the stream completes. Each {@code onPart} receives a
     * serialized ChatCompletionChunk (OpenAI "chat.completion.chunk" object).
     *
     * @param promptJson JSON prompt string.
     * @param optsJson   Optional JSON-serialized options, or {@code null} for defaults.
     * @param onPart     Called for each ChatCompletionChunk (JSON string).
     * @param onDone     Called once when the stream ends normally.
     * @throws AimuxException when the stream fails (return status 0).
     */
    public void streamTextAsOpenAI(String promptJson, String optsJson,
                                   final Consumer<String> onPart,
                                   final Runnable onDone) {
        final AimuxFFI.StreamPartCallback partCb = new AimuxFFI.StreamPartCallback() {
            @Override
            public void invoke(Pointer jsonPtr, Pointer streamCtx) {
                if (jsonPtr != null) {
                    onPart.accept(jsonPtr.getString(0, "UTF-8"));
                }
            }
        };
        final AimuxFFI.StreamDoneCallback doneCb = new AimuxFFI.StreamDoneCallback() {
            @Override
            public void invoke(Pointer streamCtx) {
                onDone.run();
            }
        };

        // Hold the read lock for the whole blocking stream so close() cannot
        // drop the handle mid-stream.
        lock.readLock().lock();
        try {
            long h = requireHandleLocked();
            AimuxCError err = AimuxResult.newError();
            int rc = AimuxFFI.INSTANCE.aimux_stream_text_as_openai(
                h, promptJson, optsJson, partCb, doneCb, null, err);
            if (rc == 0) {
                throw AimuxException.fromC(err);
            }
        } finally {
            lock.readLock().unlock();
        }
    }

    /**
     * Stream text with OpenAI Chat Completions output as a lazy {@link Stream}
     * of ChatCompletionChunk JSON strings (RFC-0026).
     *
     * @param promptJson JSON prompt string.
     * @param optsJson   Optional JSON-serialized options, or {@code null} for defaults.
     */
    public Stream<String> streamTextAsOpenAIStream(final String promptJson, final String optsJson) {
        final Object END = new Object();
        return StreamSupport.stream(
            new java.util.Spliterators.AbstractSpliterator<String>(
                Long.MAX_VALUE, Spliterator.ORDERED) {
                private final LinkedBlockingQueue<Object> parts = new LinkedBlockingQueue<>();
                private final AtomicBoolean started = new AtomicBoolean(false);
                private boolean exhausted;

                @Override
                public boolean tryAdvance(Consumer<? super String> action) {
                    if (!started.getAndSet(true)) {
                        try {
                            streamTextAsOpenAI(promptJson, optsJson,
                                parts::add,
                                () -> parts.add(END));
                        } catch (RuntimeException e) {
                            parts.add(END);
                            throw e;
                        }
                    }
                    if (exhausted) {
                        return false;
                    }
                    final Object part;
                    try {
                        part = parts.take();
                    } catch (InterruptedException e) {
                        Thread.currentThread().interrupt();
                        throw new RuntimeException("interrupted while streaming", e);
                    }
                    if (part == END) {
                        exhausted = true;
                        return false;
                    }
                    action.accept((String) part);
                    return true;
                }
            },
            false);
    }

    /** Stream text with OpenAI output and default options. */
    public Stream<String> streamTextAsOpenAIStream(final String promptJson) {
        return streamTextAsOpenAIStream(promptJson, null);
    }
}
