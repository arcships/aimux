package io.aimux;

import java.io.Closeable;
import java.util.Spliterator;
import java.util.concurrent.LinkedBlockingQueue;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicLong;
import java.util.concurrent.atomic.AtomicReference;
import java.util.function.Consumer;
import java.util.stream.Stream;
import java.util.stream.StreamSupport;

/**
 * A model instance backed by a Rust {@code Arc<dyn LanguageModel>}.
 *
 * <p>This is the raw JSON layer of the aimux Java binding: it wraps the
 * aimux-ffi C ABI via JNA and exposes the same wire format as the Kotlin
 * binding (JSON in, JSON out).
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
 */
public class Model implements Closeable {

    private final AtomicLong handle;

    private Model(long handle) {
        this.handle = new AtomicLong(handle);
    }

    /**
     * Release the native handle. Idempotent and thread-safe: subsequent calls
     * are no-ops, and every other method throws {@link IllegalStateException}.
     */
    @Override
    public void close() {
        long h = handle.getAndSet(0L);
        if (h != 0L) {
            AimuxFFI.INSTANCE.aimux_drop_handle(h);
        }
    }

    /** Best-effort backstop; try-with-resources is the primary release path. */
    @Override
    protected void finalize() throws Throwable {
        close();
        super.finalize();
    }

    private long requireHandle() {
        long h = handle.get();
        if (h == 0L) {
            throw new IllegalStateException("Model is closed");
        }
        return h;
    }

    // ── Provider constructors ──────────────────────────────────────────────

    /** Create an OpenAI model instance. */
    public static Model openai(String apiKey, String modelId) {
        long h = AimuxFFI.INSTANCE.aimux_openai_new(apiKey, modelId);
        if (h == 0L) {
            throw new IllegalArgumentException("Failed to create OpenAI model");
        }
        return new Model(h);
    }

    /** Create an OpenAI model instance with a custom base URL. */
    public static Model openaiWithBase(String apiKey, String modelId, String baseUrl) {
        long h = AimuxFFI.INSTANCE.aimux_openai_new_with_base(apiKey, modelId, baseUrl);
        if (h == 0L) {
            throw new IllegalArgumentException("Failed to create OpenAI model");
        }
        return new Model(h);
    }

    /** Create an Anthropic model instance. */
    public static Model anthropic(String apiKey, String modelId) {
        long h = AimuxFFI.INSTANCE.aimux_anthropic_new(apiKey, modelId);
        if (h == 0L) {
            throw new IllegalArgumentException("Failed to create Anthropic model");
        }
        return new Model(h);
    }

    /** Create an Anthropic model instance with a custom base URL. */
    public static Model anthropicWithBase(String apiKey, String modelId, String baseUrl) {
        long h = AimuxFFI.INSTANCE.aimux_anthropic_new_with_base(apiKey, modelId, baseUrl);
        if (h == 0L) {
            throw new IllegalArgumentException("Failed to create Anthropic model");
        }
        return new Model(h);
    }

    /** Create a DeepSeek model instance. */
    public static Model deepseek(String apiKey, String modelId) {
        long h = AimuxFFI.INSTANCE.aimux_deepseek_new(apiKey, modelId);
        if (h == 0L) {
            throw new IllegalArgumentException("Failed to create DeepSeek model");
        }
        return new Model(h);
    }

    // ── Generation ─────────────────────────────────────────────────────────

    /**
     * Generate text (non-streaming).
     *
     * @param promptJson JSON prompt string (bare value or {@code {"prompt": ...}}).
     * @param optsJson   Optional JSON-serialized options, or {@code null} for defaults.
     * @return JSON-serialized result (or {@code {"error":"..."}} on failure).
     */
    public String generateText(String promptJson, String optsJson) {
        com.sun.jna.Pointer ptr =
            AimuxFFI.INSTANCE.aimux_generate_text(requireHandle(), promptJson, optsJson);
        if (ptr == null) {
            throw new RuntimeException("generate_text returned null");
        }
        try {
            return ptr.getString(0, "UTF-8");
        } finally {
            AimuxFFI.INSTANCE.aimux_free_string(ptr);
        }
    }

    /** Generate text with default options. */
    public String generateText(String promptJson) {
        return generateText(promptJson, null);
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
     * @param promptJson JSON prompt string.
     * @param optsJson   Optional JSON-serialized options, or {@code null} for defaults.
     * @param onPart     Called for each stream part (JSON string).
     * @param onDone     Called once when the stream ends normally.
     * @param onError    Called on a stream-level error (JSON error string).
     */
    public void streamText(String promptJson, String optsJson,
                           final Consumer<String> onPart,
                           final Runnable onDone,
                           final Consumer<String> onError) {
        // JNA callbacks — must be held in local variables to prevent GC.
        final com.sun.jna.Callback partCb = new com.sun.jna.Callback() {
            @SuppressWarnings("unused")
            public void callback(com.sun.jna.Pointer jsonPtr) {
                // The pointer is valid only during the callback — copy now.
                if (jsonPtr != null) {
                    onPart.accept(jsonPtr.getString(0, "UTF-8"));
                }
            }
        };
        final com.sun.jna.Callback doneCb = new com.sun.jna.Callback() {
            @SuppressWarnings("unused")
            public void callback() {
                onDone.run();
            }
        };
        final com.sun.jna.Callback errCb = new com.sun.jna.Callback() {
            @SuppressWarnings("unused")
            public void callback(com.sun.jna.Pointer errPtr) {
                if (errPtr != null) {
                    onError.accept(errPtr.getString(0, "UTF-8"));
                }
            }
        };

        AimuxFFI.INSTANCE.aimux_stream_text(
            requireHandle(), promptJson, optsJson, partCb, doneCb, errCb);
    }

    /**
     * Stream text as a lazy {@link Stream} of stream-part JSON strings.
     *
     * <p>The FFI call starts on the first terminal operation of the returned
     * stream (mirror of Kotlin's {@code streamTextSequence}). Iteration pulls
     * parts from a {@link LinkedBlockingQueue} fed by the stream callbacks;
     * the stream ends at the sentinel. If the provider reported a stream
     * error, a {@link RuntimeException} carrying the error JSON is thrown when
     * the end of the stream is reached.
     *
     * <pre>{@code
     * model.streamTextStream("\"Write a haiku\"").forEach(System.out::println);
     * }</pre>
     *
     * @param promptJson JSON prompt string.
     * @param optsJson   Optional JSON-serialized options, or {@code null} for defaults.
     */
    public Stream<String> streamTextStream(final String promptJson, final String optsJson) {        // Sentinel for end-of-stream. Java's LinkedBlockingQueue rejects null
        // elements, so use a unique object instead of Kotlin's null sentinel.
        final Object END = new Object();
        return StreamSupport.stream(
            new java.util.Spliterators.AbstractSpliterator<String>(
                Long.MAX_VALUE, Spliterator.ORDERED) {
                private final LinkedBlockingQueue<Object> parts =
                    new LinkedBlockingQueue<>();
                private final AtomicBoolean started = new AtomicBoolean(false);
                private final AtomicReference<String> error = new AtomicReference<>();
                private boolean exhausted;

                @Override
                public boolean tryAdvance(Consumer<? super String> action) {
                    if (!started.getAndSet(true)) {
                        try {
                            streamText(promptJson, optsJson,
                                parts::add,
                                () -> parts.add(END), // sentinel = end of stream
                                err -> {
                                    error.set(err);
                                    parts.add(END);
                                });
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
                        String err = error.get();
                        if (err != null) {
                            throw new RuntimeException(err);
                        }
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
}
