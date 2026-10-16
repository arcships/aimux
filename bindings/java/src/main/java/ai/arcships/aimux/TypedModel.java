package ai.arcships.aimux;

import ai.arcships.aimux.types.*;

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

    public GenerateTextResult generate(String paramsJson) {
        return decode(model.generate(paramsJson), GenerateTextResult.class);
    }

    public GenerateObjectResult generateObject(String paramsJson) {
        return decode(model.generateObject(paramsJson), GenerateObjectResult.class);
    }

    public GenerateTextResult consume(String paramsJson) {
        return decode(model.consume(paramsJson), GenerateTextResult.class);
    }

    public ChatCompletion generateOpenAI(String paramsJson) {
        return decode(model.generateOpenAI(paramsJson), ChatCompletion.class);
    }

    public Future<GenerateTextResult> generateAsync(String paramsJson, Executor executor) {
        return new DecodedFuture<>(model.generateAsync(paramsJson, executor), GenerateTextResult.class);
    }

    public void stream(String paramsJson, Consumer<TextStreamPart> consumer) {
        Objects.requireNonNull(consumer, "consumer");
        model.stream(paramsJson, frame -> {
            if (frame instanceof StreamFrame.Event) {
                StreamEvent event = ((StreamFrame.Event) frame).getEvent();
                if (!(event instanceof StreamEvent.TextStreamPart)) {
                    throw new IllegalStateException("Expected text stream event");
                }
                consumer.accept(((StreamEvent.TextStreamPart) event).getValue());
            }
        });
    }

    private static <T> T decode(JsonNode value, Class<T> type) {
        try { return DtoJson.MAPPER.treeToValue(value, type); }
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
