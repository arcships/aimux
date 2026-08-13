package ai.arcships.aimux;

import com.fasterxml.jackson.core.JsonProcessingException;

import java.io.Closeable;
import java.io.IOException;
import java.util.List;
import java.util.Spliterator;
import java.util.concurrent.LinkedBlockingQueue;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicReference;
import java.util.function.Consumer;
import java.util.stream.Stream;
import java.util.stream.StreamSupport;

/**
 * A typed view over a {@link Model}. Delegates every call to the raw model and
 * (de)serializes the JSON boundary with {@link Types.AimuxJson}, so callers
 * work with {@link Types.GenerateTextResult}, {@link Types.StreamPart}, etc.
 * instead of raw JSON strings.
 *
 * <p>The wrapped {@link Model} is NOT closed by this wrapper when constructed
 * via {@link #of(Model)} — the caller owns the underlying handle. Factories
 * that create their own model ({@link #openai(String, String)},
 * {@link #openaiWithBase(String, String, String)},
 * {@link #anthropic(String, String)}, {@link #anthropicWithBase(String, String, String)})
 * own the handle, so closing the {@code TypedModel} releases it.
 *
 * <pre>{@code
 * try (TypedModel model = TypedModel.openaiWithBase("sk-...", "gpt-4o", baseUrl)) {
 *     Types.GenerateTextResult result = model.generateText("What is Rust?");
 *     System.out.println(result.getText());
 * }
 * }</pre>
 */
public class TypedModel implements Closeable {

    private final Model raw;
    private final boolean ownsModel;

    private TypedModel(Model raw, boolean ownsModel) {
        this.raw = raw;
        this.ownsModel = ownsModel;
    }

    @Override
    public void close() {
        if (ownsModel) {
            raw.close();
        }
    }

    // ── generateText ──────────────────────────────────────────────────────

    /**
     * Generate text from a simple string prompt.
     *
     * @param prompt Plain text prompt (encoded as a JSON string on the wire).
     * @return Decoded {@link Types.GenerateTextResult}.
     */
    public Types.GenerateTextResult generateText(String prompt) {
        return generateText(prompt, null);
    }

    /**
     * Generate text from a simple string prompt with typed options.
     *
     * @param prompt  Plain text prompt.
     * @param options Optional typed {@link Types.GenerateTextOptions}, or {@code null}.
     * @return Decoded {@link Types.GenerateTextResult}.
     */
    public Types.GenerateTextResult generateText(String prompt, Types.GenerateTextOptions options) {
        return decodeResult(raw.generateText(encode(prompt), encodeOptions(options)));
    }

    /**
     * Generate text from a multi-role message list.
     *
     * @param messages Conversation as a list of {@link Types.ModelMessage}s.
     * @return Decoded {@link Types.GenerateTextResult}.
     */
    public Types.GenerateTextResult generateText(List<Types.ModelMessage> messages) {
        return generateText(messages, null);
    }

    /**
     * Generate text from a multi-role message list with typed options.
     *
     * @param messages Conversation as a list of {@link Types.ModelMessage}s.
     * @param options  Optional typed {@link Types.GenerateTextOptions}, or {@code null}.
     * @return Decoded {@link Types.GenerateTextResult}.
     */
    public Types.GenerateTextResult generateText(List<Types.ModelMessage> messages, Types.GenerateTextOptions options) {
        return decodeResult(raw.generateText(encode(messages), encodeOptions(options)));
    }

    private Types.GenerateTextResult decodeResult(String resultJson) {
        // Failures throw from the raw Model via AimuxCError → AimuxException.
        try {
            return Types.AimuxJson.MAPPER.readValue(resultJson, Types.GenerateTextResult.class);
        } catch (IOException e) {
            throw new AimuxException("failed to decode GenerateTextResult: " + e.getMessage(), e);
        }
    }

    // ── generateObject (M12, RFC-0016) ────────────────────────────────────

    /**
     * Generate a structured JSON object from a simple string prompt (M12).
     *
     * @param prompt Plain text prompt.
     * @return Decoded {@link Types.GenerateObjectResult}.
     */
    public Types.GenerateObjectResult generateObject(String prompt) {
        return generateObject(prompt, null);
    }

    /**
     * Generate a structured JSON object from a simple string prompt (M12).
     *
     * @param prompt  Plain text prompt.
     * @param options Optional typed {@link Types.GenerateTextOptions}, or {@code null}.
     * @return Decoded {@link Types.GenerateObjectResult}.
     */
    public Types.GenerateObjectResult generateObject(String prompt, Types.GenerateTextOptions options) {
        return decodeObjectResult(raw.generateObject(encode(prompt), encodeOptions(options)));
    }

    /**
     * Generate a structured JSON object from a multi-role message list (M12).
     *
     * @param messages Conversation as a list of {@link Types.ModelMessage}s.
     * @return Decoded {@link Types.GenerateObjectResult}.
     */
    public Types.GenerateObjectResult generateObject(List<Types.ModelMessage> messages) {
        return generateObject(messages, null);
    }

    /**
     * Generate a structured JSON object from a multi-role message list (M12).
     *
     * @param messages Conversation as a list of {@link Types.ModelMessage}s.
     * @param options  Optional typed {@link Types.GenerateTextOptions}, or {@code null}.
     * @return Decoded {@link Types.GenerateObjectResult}.
     */
    public Types.GenerateObjectResult generateObject(List<Types.ModelMessage> messages, Types.GenerateTextOptions options) {
        return decodeObjectResult(raw.generateObject(encode(messages), encodeOptions(options)));
    }

    private Types.GenerateObjectResult decodeObjectResult(String resultJson) {
        try {
            return Types.AimuxJson.MAPPER.readValue(resultJson, Types.GenerateObjectResult.class);
        } catch (IOException e) {
            throw new AimuxException("failed to decode GenerateObjectResult: " + e.getMessage(), e);
        }
    }

    // ── consumeStreamText (M11, RFC-0016) ─────────────────────────────────

    /**
     * Consume a stream to completion from a simple string prompt and return
     * the aggregated result (M11).
     *
     * @param prompt Plain text prompt.
     * @return Decoded {@link Types.StreamTextResultAggregated}.
     */
    public Types.StreamTextResultAggregated consumeStreamText(String prompt) {
        return consumeStreamText(prompt, null);
    }

    /**
     * Consume a stream to completion from a simple string prompt and return
     * the aggregated result (M11).
     *
     * @param prompt  Plain text prompt.
     * @param options Optional typed {@link Types.GenerateTextOptions}, or {@code null}.
     * @return Decoded {@link Types.StreamTextResultAggregated}.
     */
    public Types.StreamTextResultAggregated consumeStreamText(String prompt, Types.GenerateTextOptions options) {
        return decodeAggregated(raw.consumeStreamText(encode(prompt), encodeOptions(options)));
    }

    /**
     * Consume a stream to completion from a multi-role message list and
     * return the aggregated result (M11).
     *
     * @param messages Conversation as a list of {@link Types.ModelMessage}s.
     * @return Decoded {@link Types.StreamTextResultAggregated}.
     */
    public Types.StreamTextResultAggregated consumeStreamText(List<Types.ModelMessage> messages) {
        return consumeStreamText(messages, null);
    }

    /**
     * Consume a stream to completion from a multi-role message list and
     * return the aggregated result (M11).
     *
     * @param messages Conversation as a list of {@link Types.ModelMessage}s.
     * @param options  Optional typed {@link Types.GenerateTextOptions}, or {@code null}.
     * @return Decoded {@link Types.StreamTextResultAggregated}.
     */
    public Types.StreamTextResultAggregated consumeStreamText(List<Types.ModelMessage> messages, Types.GenerateTextOptions options) {
        return decodeAggregated(raw.consumeStreamText(encode(messages), encodeOptions(options)));
    }

    private Types.StreamTextResultAggregated decodeAggregated(String resultJson) {
        try {
            return Types.AimuxJson.MAPPER.readValue(resultJson, Types.StreamTextResultAggregated.class);
        } catch (IOException e) {
            throw new AimuxException("failed to decode StreamTextResultAggregated: " + e.getMessage(), e);
        }
    }

    // ── streamText (callback version) ─────────────────────────────────────

    /**
     * Stream text from a simple string prompt, delivering each chunk as a
     * decoded {@link Types.StreamPart}.
     *
     * <p>Blocks the calling thread until the stream completes. A part that
     * fails to decode is reported via {@code onError}. Terminal C-level stream
     * failures throw {@link AimuxException} from the raw layer.
     *
     * @param prompt  Plain text prompt.
     * @param onPart  Called for each decoded {@link Types.StreamPart}.
     * @param onDone  Called once when the stream completes normally.
     * @param onError Called with a message on a decode error (not C terminal failures).
     */
    public void streamText(String prompt, Consumer<Types.StreamPart> onPart,
                           Runnable onDone, Consumer<String> onError) {
        streamText(prompt, null, onPart, onDone, onError);
    }

    /**
     * Stream text from a simple string prompt with typed options.
     *
     * @param prompt  Plain text prompt.
     * @param options Optional typed {@link Types.GenerateTextOptions}, or {@code null}.
     * @param onPart  Called for each decoded {@link Types.StreamPart}.
     * @param onDone  Called once when the stream completes normally.
     * @param onError Called with a message on a stream or decode error.
     */
    public void streamText(String prompt, Types.GenerateTextOptions options,
                           Consumer<Types.StreamPart> onPart,
                           Runnable onDone, Consumer<String> onError) {
        streamTextParts(encode(prompt), encodeOptions(options), onPart, onDone, onError);
    }

    /**
     * Stream text from a multi-role message list, delivering typed parts.
     *
     * @param messages Conversation as a list of {@link Types.ModelMessage}s.
     * @param onPart   Called for each decoded {@link Types.StreamPart}.
     * @param onDone   Called once when the stream completes normally.
     * @param onError  Called with a message on a stream or decode error.
     */
    public void streamText(List<Types.ModelMessage> messages, Consumer<Types.StreamPart> onPart,
                           Runnable onDone, Consumer<String> onError) {
        streamText(messages, null, onPart, onDone, onError);
    }

    /**
     * Stream text from a multi-role message list with typed options.
     *
     * @param messages Conversation as a list of {@link Types.ModelMessage}s.
     * @param options  Optional typed {@link Types.GenerateTextOptions}, or {@code null}.
     * @param onPart   Called for each decoded {@link Types.StreamPart}.
     * @param onDone   Called once when the stream completes normally.
     * @param onError  Called with a message on a stream or decode error.
     */
    public void streamText(List<Types.ModelMessage> messages, Types.GenerateTextOptions options,
                           Consumer<Types.StreamPart> onPart,
                           Runnable onDone, Consumer<String> onError) {
        streamTextParts(encode(messages), encodeOptions(options), onPart, onDone, onError);
    }

    private void streamTextParts(String promptJson, String optsJson,
                                 Consumer<Types.StreamPart> onPart,
                                 Runnable onDone, Consumer<String> onError) {
        raw.streamText(
            promptJson, optsJson,
            partJson -> {
                try {
                    onPart.accept(decodePart(partJson));
                } catch (RuntimeException e) {
                    onError.accept("failed to decode StreamPart: " + e.getMessage());
                }
            },
            onDone);
    }

    // ── streamTextStream (pull version) ───────────────────────────────────

    /**
     * Stream text as a lazy {@link Stream} of typed {@link Types.StreamPart}s.
     *
     * <p>The FFI call starts on the first terminal operation of the returned
     * stream (mirror of Kotlin's {@code streamTextSequence}). Iteration pulls
     * parts from a {@link LinkedBlockingQueue} fed by the stream callbacks;
     * the stream ends at the sentinel. Terminal stream failures throw
     * {@link AimuxException} from the blocking FFI call.
     *
     * @param prompt Plain text prompt.
     * @return Lazy stream of decoded parts.
     */
    public Stream<Types.StreamPart> streamTextStream(String prompt) {
        return streamTextStream(prompt, null);
    }

    /**
     * Stream text as a lazy {@link Stream} of typed parts with options.
     *
     * @param prompt  Plain text prompt.
     * @param options Optional typed {@link Types.GenerateTextOptions}, or {@code null}.
     * @return Lazy stream of decoded parts.
     */
    public Stream<Types.StreamPart> streamTextStream(String prompt, Types.GenerateTextOptions options) {
        return streamTextStreamParts(encode(prompt), encodeOptions(options));
    }

    /**
     * Stream text from a multi-role message list as a lazy typed stream.
     *
     * @param messages Conversation as a list of {@link Types.ModelMessage}s.
     * @return Lazy stream of decoded parts.
     */
    public Stream<Types.StreamPart> streamTextStream(List<Types.ModelMessage> messages) {
        return streamTextStream(messages, null);
    }

    /**
     * Stream text from a multi-role message list as a lazy typed stream with options.
     *
     * @param messages Conversation as a list of {@link Types.ModelMessage}s.
     * @param options  Optional typed {@link Types.GenerateTextOptions}, or {@code null}.
     * @return Lazy stream of decoded parts.
     */
    public Stream<Types.StreamPart> streamTextStream(List<Types.ModelMessage> messages, Types.GenerateTextOptions options) {
        return streamTextStreamParts(encode(messages), encodeOptions(options));
    }

    private Stream<Types.StreamPart> streamTextStreamParts(final String promptJson, final String optsJson) {
        // Sentinel for end-of-stream. Java's LinkedBlockingQueue rejects null
        // elements, so use a unique object instead of Kotlin's null sentinel.
        final Object END = new Object();
        return StreamSupport.stream(
            new java.util.Spliterators.AbstractSpliterator<Types.StreamPart>(
                Long.MAX_VALUE, Spliterator.ORDERED) {
                private final LinkedBlockingQueue<Object> parts = new LinkedBlockingQueue<>();
                private final AtomicBoolean started = new AtomicBoolean(false);
                private final AtomicReference<String> error = new AtomicReference<>();
                private boolean exhausted;

                @Override
                public boolean tryAdvance(Consumer<? super Types.StreamPart> action) {
                    if (!started.getAndSet(true)) {
                        try {
                            streamTextParts(promptJson, optsJson,
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
                            throw new AimuxException(err);
                        }
                        return false;
                    }
                    action.accept((Types.StreamPart) part);
                    return true;
                }
            },
            false);
    }

    // ── OpenAI-compatible output (RFC-0026) — generate ─────────────────────

    /**
     * Generate text (non-streaming) with OpenAI Chat Completions output.
     *
     * @param prompt Plain text prompt (encoded as a JSON string on the wire).
     * @return Decoded {@link Types.ChatCompletion}.
     */
    public Types.ChatCompletion generateTextAsOpenAI(String prompt) {
        return generateTextAsOpenAI(prompt, null);
    }

    /**
     * Generate text (non-streaming) with OpenAI Chat Completions output.
     *
     * @param prompt  Plain text prompt.
     * @param options Optional typed {@link Types.GenerateTextOptions}, or {@code null}.
     * @return Decoded {@link Types.ChatCompletion}.
     */
    public Types.ChatCompletion generateTextAsOpenAI(String prompt, Types.GenerateTextOptions options) {
        return decodeChatCompletion(raw.generateTextAsOpenAI(encode(prompt), encodeOptions(options)));
    }

    /**
     * Generate text from a multi-role message list with OpenAI Chat Completions output.
     *
     * @param messages Conversation as a list of {@link Types.ModelMessage}s.
     * @return Decoded {@link Types.ChatCompletion}.
     */
    public Types.ChatCompletion generateTextAsOpenAI(List<Types.ModelMessage> messages) {
        return generateTextAsOpenAI(messages, null);
    }

    /**
     * Generate text from a multi-role message list with OpenAI Chat Completions output.
     *
     * @param messages Conversation as a list of {@link Types.ModelMessage}s.
     * @param options  Optional typed {@link Types.GenerateTextOptions}, or {@code null}.
     * @return Decoded {@link Types.ChatCompletion}.
     */
    public Types.ChatCompletion generateTextAsOpenAI(List<Types.ModelMessage> messages, Types.GenerateTextOptions options) {
        return decodeChatCompletion(raw.generateTextAsOpenAI(encode(messages), encodeOptions(options)));
    }

    private Types.ChatCompletion decodeChatCompletion(String resultJson) {
        try {
            return Types.AimuxJson.MAPPER.readValue(resultJson, Types.ChatCompletion.class);
        } catch (IOException e) {
            throw new AimuxException("failed to decode ChatCompletion: " + e.getMessage(), e);
        }
    }

    // ── OpenAI-compatible output (RFC-0026) — stream ───────────────────────

    /**
     * Stream text with OpenAI Chat Completions output, delivering typed chunks.
     *
     * @param prompt  Plain text prompt.
     * @param onPart  Called for each decoded {@link Types.ChatCompletionChunk}.
     * @param onDone  Called once when the stream completes normally.
     * @param onError Called with a message on a stream or decode error.
     */
    public void streamTextAsOpenAI(String prompt, Consumer<Types.ChatCompletionChunk> onPart,
                                   Runnable onDone, Consumer<String> onError) {
        streamTextAsOpenAI(prompt, null, onPart, onDone, onError);
    }

    /**
     * Stream text with OpenAI Chat Completions output, delivering typed chunks.
     *
     * @param prompt  Plain text prompt.
     * @param options Optional typed {@link Types.GenerateTextOptions}, or {@code null}.
     * @param onPart  Called for each decoded {@link Types.ChatCompletionChunk}.
     * @param onDone  Called once when the stream completes normally.
     * @param onError Called with a message on a stream or decode error.
     */
    public void streamTextAsOpenAI(String prompt, Types.GenerateTextOptions options,
                                   Consumer<Types.ChatCompletionChunk> onPart,
                                   Runnable onDone, Consumer<String> onError) {
        streamTextAsOpenAIChunks(encode(prompt), encodeOptions(options), onPart, onDone, onError);
    }

    /**
     * Stream text from a multi-role message list with OpenAI Chat Completions output.
     *
     * @param messages Conversation as a list of {@link Types.ModelMessage}s.
     * @param onPart   Called for each decoded {@link Types.ChatCompletionChunk}.
     * @param onDone   Called once when the stream completes normally.
     * @param onError  Called with a message on a stream or decode error.
     */
    public void streamTextAsOpenAI(List<Types.ModelMessage> messages, Consumer<Types.ChatCompletionChunk> onPart,
                                   Runnable onDone, Consumer<String> onError) {
        streamTextAsOpenAI(messages, null, onPart, onDone, onError);
    }

    /**
     * Stream text from a multi-role message list with OpenAI Chat Completions output.
     *
     * @param messages Conversation as a list of {@link Types.ModelMessage}s.
     * @param options  Optional typed {@link Types.GenerateTextOptions}, or {@code null}.
     * @param onPart   Called for each decoded {@link Types.ChatCompletionChunk}.
     * @param onDone   Called once when the stream completes normally.
     * @param onError  Called with a message on a stream or decode error.
     */
    public void streamTextAsOpenAI(List<Types.ModelMessage> messages, Types.GenerateTextOptions options,
                                   Consumer<Types.ChatCompletionChunk> onPart,
                                   Runnable onDone, Consumer<String> onError) {
        streamTextAsOpenAIChunks(encode(messages), encodeOptions(options), onPart, onDone, onError);
    }

    private void streamTextAsOpenAIChunks(String promptJson, String optsJson,
                                          Consumer<Types.ChatCompletionChunk> onPart,
                                          Runnable onDone, Consumer<String> onError) {
        raw.streamTextAsOpenAI(
            promptJson, optsJson,
            chunkJson -> {
                try {
                    onPart.accept(decodeChunk(chunkJson));
                } catch (RuntimeException e) {
                    onError.accept("failed to decode ChatCompletionChunk: " + e.getMessage());
                }
            },
            onDone);
    }

    /**
     * Stream text with OpenAI Chat Completions output as a lazy typed stream.
     *
     * @param prompt Plain text prompt.
     * @return Lazy stream of decoded chunks.
     */
    public Stream<Types.ChatCompletionChunk> streamTextAsOpenAIStream(String prompt) {
        return streamTextAsOpenAIStream(prompt, null);
    }

    /**
     * Stream text with OpenAI Chat Completions output as a lazy typed stream.
     *
     * @param prompt  Plain text prompt.
     * @param options Optional typed {@link Types.GenerateTextOptions}, or {@code null}.
     * @return Lazy stream of decoded chunks.
     */
    public Stream<Types.ChatCompletionChunk> streamTextAsOpenAIStream(String prompt, Types.GenerateTextOptions options) {
        return streamTextAsOpenAIChunksStream(encode(prompt), encodeOptions(options));
    }

    /**
     * Stream text from a multi-role message list with OpenAI Chat Completions output.
     *
     * @param messages Conversation as a list of {@link Types.ModelMessage}s.
     * @return Lazy stream of decoded chunks.
     */
    public Stream<Types.ChatCompletionChunk> streamTextAsOpenAIStream(List<Types.ModelMessage> messages) {
        return streamTextAsOpenAIStream(messages, null);
    }

    /**
     * Stream text from a multi-role message list with OpenAI Chat Completions output.
     *
     * @param messages Conversation as a list of {@link Types.ModelMessage}s.
     * @param options  Optional typed {@link Types.GenerateTextOptions}, or {@code null}.
     * @return Lazy stream of decoded chunks.
     */
    public Stream<Types.ChatCompletionChunk> streamTextAsOpenAIStream(List<Types.ModelMessage> messages, Types.GenerateTextOptions options) {
        return streamTextAsOpenAIChunksStream(encode(messages), encodeOptions(options));
    }

    private Stream<Types.ChatCompletionChunk> streamTextAsOpenAIChunksStream(final String promptJson, final String optsJson) {
        final Object END = new Object();
        return StreamSupport.stream(
            new java.util.Spliterators.AbstractSpliterator<Types.ChatCompletionChunk>(
                Long.MAX_VALUE, Spliterator.ORDERED) {
                private final LinkedBlockingQueue<Object> parts = new LinkedBlockingQueue<>();
                private final AtomicBoolean started = new AtomicBoolean(false);
                private final AtomicReference<String> error = new AtomicReference<>();
                private boolean exhausted;

                @Override
                public boolean tryAdvance(Consumer<? super Types.ChatCompletionChunk> action) {
                    if (!started.getAndSet(true)) {
                        try {
                            streamTextAsOpenAIChunks(promptJson, optsJson,
                                parts::add,
                                () -> parts.add(END),
                                err -> {
                                    error.set(err);
                                    parts.add(END);
                                });
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
                        String err = error.get();
                        if (err != null) {
                            throw new AimuxException(err);
                        }
                        return false;
                    }
                    action.accept((Types.ChatCompletionChunk) part);
                    return true;
                }
            },
            false);
    }

    // ── Companion factories ───────────────────────────────────────────────

    /** Wrap an existing {@link Model}; the caller retains ownership of the handle. */
    public static TypedModel of(Model model) {
        return new TypedModel(model, false);
    }

    /** Create an OpenAI-backed {@link TypedModel} that owns its {@link Model}. */
    public static TypedModel openai(String apiKey, String modelId) {
        return new TypedModel(Model.openai(apiKey, modelId), true);
    }

    /** Create an OpenAI-backed {@link TypedModel} with a custom base URL; owns its {@link Model}. */
    public static TypedModel openaiWithBase(String apiKey, String modelId, String baseUrl) {
        return new TypedModel(Model.openaiWithBase(apiKey, modelId, baseUrl), true);
    }

    /** Create an Anthropic-backed {@link TypedModel} that owns its {@link Model}. */
    public static TypedModel anthropic(String apiKey, String modelId) {
        return new TypedModel(Model.anthropic(apiKey, modelId), true);
    }

    /** Create an Anthropic-backed {@link TypedModel} with a custom base URL; owns its {@link Model}. */
    public static TypedModel anthropicWithBase(String apiKey, String modelId, String baseUrl) {
        return new TypedModel(Model.anthropicWithBase(apiKey, modelId, baseUrl), true);
    }

    /**
     * Create a registry-backed {@link TypedModel} (RFC-0017 phase 4) that owns
     * its {@link Model}. See {@link Model#provider(String, String, String, String)}.
     */
    public static TypedModel provider(String name, String apiKey, String modelId, String configJson) {
        return new TypedModel(Model.provider(name, apiKey, modelId, configJson), true);
    }

    // ── helpers ───────────────────────────────────────────────────────────

    private static String encodeOptions(Types.GenerateTextOptions options) {
        return options == null ? null : encode(options);
    }

    private static String encode(Object value) {
        try {
            return Types.AimuxJson.MAPPER.writeValueAsString(value);
        } catch (JsonProcessingException e) {
            throw new IllegalArgumentException("Failed to serialize typed input", e);
        }
    }

    private static Types.StreamPart decodePart(String partJson) {
        try {
            return Types.AimuxJson.MAPPER.readValue(partJson, Types.StreamPart.class);
        } catch (IOException e) {
            throw new IllegalArgumentException("failed to decode StreamPart", e);
        }
    }

    private static Types.ChatCompletionChunk decodeChunk(String chunkJson) {
        try {
            return Types.AimuxJson.MAPPER.readValue(chunkJson, Types.ChatCompletionChunk.class);
        } catch (IOException e) {
            throw new IllegalArgumentException("failed to decode ChatCompletionChunk", e);
        }
    }
}

