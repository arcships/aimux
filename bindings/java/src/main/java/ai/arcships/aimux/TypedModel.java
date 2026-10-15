package ai.arcships.aimux;

import com.fasterxml.jackson.databind.JsonNode;
import java.io.IOException;
import java.util.Objects;
import java.util.concurrent.ExecutionException;
import java.util.concurrent.Executor;
import java.util.concurrent.Future;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.TimeoutException;
import java.util.function.Consumer;

/** Typed results over a language model. This wrapper owns the supplied model. */
public final class TypedModel implements AutoCloseable {
    private final Model model;
    public TypedModel(Model model) { this.model = Objects.requireNonNull(model, "model"); }
    @Override public void close() { model.close(); }

    public Types.GenerateTextResult generate(String paramsJson) {
        return decode(model.generate(paramsJson), Types.GenerateTextResult.class);
    }

    public Types.GenerateObjectResult generateObject(String paramsJson) {
        return decode(model.generateObject(paramsJson), Types.GenerateObjectResult.class);
    }

    public Types.GenerateTextResult consume(String paramsJson) {
        return decode(model.consume(paramsJson), Types.GenerateTextResult.class);
    }

    public Types.ChatCompletion generateOpenAI(String paramsJson) {
        return decode(model.generateOpenAI(paramsJson), Types.ChatCompletion.class);
    }

    public Future<Types.GenerateTextResult> generateAsync(String paramsJson, Executor executor) {
        return new DecodedFuture<>(model.generateAsync(paramsJson, executor), Types.GenerateTextResult.class);
    }

    public void stream(String paramsJson, Consumer<Types.StreamPart> consumer) {
        Objects.requireNonNull(consumer, "consumer");
        model.stream(paramsJson, frame -> {
            if (!frame.isEnd()) consumer.accept(decode(frame.getEvent(), Types.StreamPart.class));
        });
    }

    private static <T> T decode(JsonNode value, Class<T> type) {
        try { return Types.AimuxJson.MAPPER.treeToValue(value, type); }
        catch (IOException e) { throw new IllegalStateException("Invalid " + type.getSimpleName(), e); }
    }

    private static final class DecodedFuture<T> implements Future<T> {
        private final Future<JsonNode> source;
        private final Class<T> type;
        DecodedFuture(Future<JsonNode> source, Class<T> type) { this.source = source; this.type = type; }
        @Override public boolean cancel(boolean mayInterrupt) { return source.cancel(mayInterrupt); }
        @Override public boolean isCancelled() { return source.isCancelled(); }
        @Override public boolean isDone() { return source.isDone(); }
        @Override public T get() throws InterruptedException, ExecutionException {
            try { return decode(source.get(), type); }
            catch (RuntimeException e) {
                if (e instanceof java.util.concurrent.CancellationException) throw e;
                throw new ExecutionException(e);
            }
        }
        @Override public T get(long timeout, TimeUnit unit)
                throws InterruptedException, ExecutionException, TimeoutException {
            try { return decode(source.get(timeout, unit), type); }
            catch (RuntimeException e) {
                if (e instanceof java.util.concurrent.CancellationException) throw e;
                throw new ExecutionException(e);
            }
        }
    }
}
