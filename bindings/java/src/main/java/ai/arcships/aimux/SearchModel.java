package ai.arcships.aimux;

import com.sun.jna.Pointer;
import com.sun.jna.ptr.LongByReference;
import com.sun.jna.ptr.PointerByReference;
import java.io.Closeable;
import java.util.Objects;
import java.util.concurrent.atomic.AtomicLong;

/**
 * Performs web search. Wraps a Rust {@code Arc<dyn SearchModel>}.
 *
 * <p>Implements {@link Closeable} — call {@link #close()} (or use a
 * try-with-resources block) to release the native handle.
 *
 * <pre>{@code
 * try (SearchModel model = SearchModel.tavily("sk-...")) {
 *     String result = model.search("{\"query\":\"What is Rust?\",\"max_results\":5}");
 * }
 * }</pre>
 */
public final class SearchModel implements Closeable {

    private final AtomicLong handle;

    private SearchModel(long handle) {
        this.handle = new AtomicLong(handle);
    }

    /** Release the native handle. Idempotent and thread-safe. */
    @Override
    public void close() {
        long h = handle.getAndSet(0L);
        if (h != 0L) AimuxFFI.INSTANCE.aimux_drop_handle(h);
    }

    /** Best-effort backstop; try-with-resources is the primary release path. */
    @Override
    protected void finalize() throws Throwable {
        close();
        super.finalize();
    }

    private long requireHandle() {
        long h = handle.get();
        if (h == 0L) throw new IllegalStateException("SearchModel is closed");
        return h;
    }

    /**
     * Create a Tavily search model. Tavily uses a fixed endpoint, so no model
     * ID is needed (the C ABI still takes one, so an empty string is passed
     * and ignored).
     *
     * @param apiKey Tavily API key.
     * @return a new SearchModel.
     * @throws AimuxException on failure.
     */
    public static SearchModel tavily(String apiKey) {
        Objects.requireNonNull(apiKey, "apiKey");
        LongByReference out = new LongByReference();
        Pointer e = AimuxFFI.INSTANCE.aimux_tavily_search_new(apiKey, "", out);
        return new SearchModel(AimuxResult.extractHandle(e, out, "Failed to create Tavily search model"));
    }

    /**
     * Create a Tavily search model with a custom base URL (e.g. for mocks).
     *
     * @param apiKey  Tavily API key.
     * @param baseUrl Custom base URL.
     * @return a new SearchModel.
     * @throws AimuxException on failure.
     */
    public static SearchModel tavilyWithBase(String apiKey, String baseUrl) {
        Objects.requireNonNull(apiKey, "apiKey");
        LongByReference out = new LongByReference();
        Pointer e = AimuxFFI.INSTANCE.aimux_tavily_search_new_with_base(apiKey, "", baseUrl, out);
        return new SearchModel(AimuxResult.extractHandle(e, out, "Failed to create Tavily search model"));
    }

    /**
     * Perform a web search.
     *
     * @param optsJson JSON-serialized {@code SearchCallOptions}.
     *                 Required: carries the input.
     * @return JSON-serialized {@code SearchResult}.
     * @throws NullPointerException if {@code optsJson} is null.
     * @throws IllegalArgumentException if {@code optsJson} is blank or malformed JSON.
     * @throws AimuxException on engine / transport failure.
     */
    public String search(String optsJson) {
        AimuxResult.requireJsonNonNull(optsJson, "optsJson");
        PointerByReference out = new PointerByReference();
        return AimuxResult.extractString(
            AimuxFFI.INSTANCE.aimux_search(requireHandle(), optsJson, out),
            out,
            "search");
    }
}
