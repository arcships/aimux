package ai.arcships.aimux;

import com.sun.jna.Pointer;
import com.sun.jna.ptr.LongByReference;
import com.sun.jna.ptr.PointerByReference;
import java.io.Closeable;
import java.util.Objects;
import java.util.concurrent.atomic.AtomicLong;

/**
 * Embedding model — generate vector embeddings for text. Wraps a Rust
 * {@code Arc<dyn EmbeddingModel>}.
 *
 * <p>Implements {@link Closeable} — call {@link #close()} (or use a
 * try-with-resources block) to release the native handle.
 *
 * <pre>{@code
 * try (EmbeddingModel model = EmbeddingModel.openai("sk-...", "text-embedding-3-small")) {
 *     String result = model.embed("[\"hello\"]");
 * }
 * }</pre>
 */
public final class EmbeddingModel implements Closeable {

    private final AtomicLong handle;

    private EmbeddingModel(long handle) {
        this.handle = new AtomicLong(handle);
    }

    @Override
    public void close() {
        long h = handle.getAndSet(0L);
        if (h != 0L) {
            AimuxFFI.INSTANCE.aimux_drop_handle(h);
        }
    }

    @Override
    protected void finalize() throws Throwable {
        close();
        super.finalize();
    }

    private long requireHandle() {
        long h = handle.get();
        if (h == 0L) {
            throw new IllegalStateException("EmbeddingModel is closed");
        }
        return h;
    }

    // ── Provider constructors ──────────────────────────────────────────────

    public static EmbeddingModel openai(String apiKey, String modelId) {
        Objects.requireNonNull(apiKey, "apiKey");
        Objects.requireNonNull(modelId, "modelId");
        LongByReference out = new LongByReference();
        Pointer e = AimuxFFI.INSTANCE.aimux_openai_embedding_new(apiKey, modelId, out);
        return new EmbeddingModel(AimuxResult.extractHandle(e, out, "Failed to create OpenAI embedding model"));
    }

    public static EmbeddingModel openaiWithBase(String apiKey, String modelId, String baseUrl) {
        Objects.requireNonNull(apiKey, "apiKey");
        Objects.requireNonNull(modelId, "modelId");
        LongByReference out = new LongByReference();
        Pointer e = AimuxFFI.INSTANCE.aimux_openai_embedding_new_with_base(apiKey, modelId, baseUrl, out);
        return new EmbeddingModel(AimuxResult.extractHandle(e, out, "Failed to create OpenAI embedding model"));
    }

    public static EmbeddingModel cohere(String apiKey, String modelId) {
        Objects.requireNonNull(apiKey, "apiKey");
        Objects.requireNonNull(modelId, "modelId");
        LongByReference out = new LongByReference();
        Pointer e = AimuxFFI.INSTANCE.aimux_cohere_embedding_new(apiKey, modelId, out);
        return new EmbeddingModel(AimuxResult.extractHandle(e, out, "Failed to create Cohere embedding model"));
    }

    public static EmbeddingModel cohereWithBase(String apiKey, String modelId, String baseUrl) {
        Objects.requireNonNull(apiKey, "apiKey");
        Objects.requireNonNull(modelId, "modelId");
        LongByReference out = new LongByReference();
        Pointer e = AimuxFFI.INSTANCE.aimux_cohere_embedding_new_with_base(apiKey, modelId, baseUrl, out);
        return new EmbeddingModel(AimuxResult.extractHandle(e, out, "Failed to create Cohere embedding model"));
    }

    public static EmbeddingModel google(String apiKey, String modelId) {
        Objects.requireNonNull(apiKey, "apiKey");
        Objects.requireNonNull(modelId, "modelId");
        LongByReference out = new LongByReference();
        Pointer e = AimuxFFI.INSTANCE.aimux_google_embedding_new(apiKey, modelId, out);
        return new EmbeddingModel(AimuxResult.extractHandle(e, out, "Failed to create Google embedding model"));
    }

    public static EmbeddingModel googleWithBase(String apiKey, String modelId, String baseUrl) {
        Objects.requireNonNull(apiKey, "apiKey");
        Objects.requireNonNull(modelId, "modelId");
        LongByReference out = new LongByReference();
        Pointer e = AimuxFFI.INSTANCE.aimux_google_embedding_new_with_base(apiKey, modelId, baseUrl, out);
        return new EmbeddingModel(AimuxResult.extractHandle(e, out, "Failed to create Google embedding model"));
    }

    // ── Embedding ──────────────────────────────────────────────────────────

    public String embed(String valuesJson) {
        return embed(valuesJson, null);
    }

    public String embed(String valuesJson, String optsJson) {
        AimuxResult.requireJsonNonNull(valuesJson, "valuesJson");
        AimuxResult.requireJson(optsJson, "optsJson");
        PointerByReference out = new PointerByReference();
        return AimuxResult.extractString(
            AimuxFFI.INSTANCE.aimux_embed(requireHandle(), valuesJson, optsJson, out),
            out,
            "embed");
    }
}
