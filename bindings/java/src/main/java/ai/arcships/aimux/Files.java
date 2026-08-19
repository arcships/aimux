package ai.arcships.aimux;

import com.sun.jna.Pointer;
import com.sun.jna.ptr.LongByReference;
import com.sun.jna.ptr.PointerByReference;
import java.io.Closeable;
import java.util.Objects;
import java.util.concurrent.atomic.AtomicLong;

/**
 * Manages file uploads to providers. Wraps a Rust {@code Arc<dyn FilesModel>}.
 *
 * <p>Implements {@link Closeable} — call {@link #close()} (or use a
 * try-with-resources block) to release the native handle.
 *
 * <pre>{@code
 * try (Files files = Files.openai("sk-...")) {
 *     String result = files.uploadFile(base64Data, "application/pdf");
 * }
 * }</pre>
 */
public final class Files implements Closeable {

    private final AtomicLong handle;

    private Files(long handle) {
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
        if (h == 0L) throw new IllegalStateException("Files is closed");
        return h;
    }

    /**
     * Create an OpenAI files manager.
     *
     * @param apiKey OpenAI API key.
     * @return a new Files.
     * @throws AimuxException on failure.
     */
    public static Files openai(String apiKey) {
        Objects.requireNonNull(apiKey, "apiKey");
        LongByReference out = new LongByReference();
        Pointer e = AimuxFFI.INSTANCE.aimux_openai_files_new(apiKey, out);
        return new Files(AimuxResult.extractHandle(e, out, "Failed to create OpenAI files manager"));
    }

    /**
     * Create an OpenAI files manager with a custom base URL.
     *
     * @param apiKey  OpenAI API key.
     * @param baseUrl Custom base URL.
     * @return a new Files.
     * @throws AimuxException on failure.
     */
    public static Files openaiWithBase(String apiKey, String baseUrl) {
        Objects.requireNonNull(apiKey, "apiKey");
        LongByReference out = new LongByReference();
        Pointer e = AimuxFFI.INSTANCE.aimux_openai_files_new_with_base(apiKey, baseUrl, out);
        return new Files(AimuxResult.extractHandle(e, out, "Failed to create OpenAI files manager"));
    }

    /**
     * Upload a file (base64-encoded) to the provider.
     *
     * @param dataBase64 Base64-encoded file bytes.
     * @param mediaType  Media type of the file (e.g. {@code application/pdf}).
     * @return JSON-serialized {@code UploadFileResult}.
     * @throws AimuxException on engine / transport failure.
     */
    public String uploadFile(String dataBase64, String mediaType) {
        return uploadFile(dataBase64, mediaType, null);
    }

    /**
     * Upload a file (base64-encoded) to the provider.
     *
     * @param dataBase64 Base64-encoded file bytes.
     * @param mediaType  Media type of the file (e.g. {@code application/pdf}).
     * @param optsJson   Optional JSON-serialized {@code UploadFileCallOptions},
     *                   or {@code null} for defaults.
     * @return JSON-serialized {@code UploadFileResult}.
     * @throws AimuxException on engine / transport failure.
     */
    public String uploadFile(String dataBase64, String mediaType, String optsJson) {
        Objects.requireNonNull(dataBase64, "dataBase64");
        Objects.requireNonNull(mediaType, "mediaType");
        AimuxResult.requireJson(optsJson, "optsJson");
        PointerByReference out = new PointerByReference();
        return AimuxResult.extractString(
            AimuxFFI.INSTANCE.aimux_file_upload(
                requireHandle(), dataBase64, mediaType, optsJson, out),
            out,
            "file_upload");
    }
}
