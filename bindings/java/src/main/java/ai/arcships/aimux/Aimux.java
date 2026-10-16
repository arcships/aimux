package ai.arcships.aimux;

import ai.arcships.aimux.types.*;

import ai.arcships.aimux.generated.AimuxOps;

import com.fasterxml.jackson.databind.JsonNode;
import com.sun.jna.Library;
import com.sun.jna.Memory;
import com.sun.jna.Native;
import com.sun.jna.Pointer;
import com.sun.jna.ptr.PointerByReference;
import java.math.BigInteger;
import java.util.Collections;
import java.util.Objects;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ConcurrentMap;
import java.util.concurrent.Executor;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;
import java.util.concurrent.atomic.AtomicLong;
import java.util.concurrent.atomic.AtomicReference;
import java.util.function.Consumer;

/** Entry point for ABI 1 / ops 1.0. No native library is loaded before load(). */
public final class Aimux {
    public static final int ABI_VERSION = (int) ai.arcships.aimux.generated.AimuxOps.ABI_VERSION;
    public static final String OPS_VERSION = ai.arcships.aimux.generated.AimuxOps.OPS_VERSION;
    private static final AtomicLong CONTEXT_IDS = new AtomicLong();
    private static final ConcurrentMap<Long, StreamContext> CONTEXTS = new ConcurrentHashMap<>();
    private static final ThreadLocal<Boolean> IN_CALLBACK = new ThreadLocal<>();
    private static final Executor REAPER = Executors.newSingleThreadExecutor(task -> {
        Thread thread = new Thread(task, "aimux-resource-reaper");
        thread.setDaemon(true);
        return thread;
    });
    final AimuxFFI ffi;
    private final JsonNode manifest;

    Aimux(AimuxFFI ffi) {
        this.ffi = Objects.requireNonNull(ffi, "ffi");
        int abi = ffi.aimux_abi_version();
        if (abi != ABI_VERSION) throw new IllegalStateException("Incompatible aimux ABI: " + abi);
        PointerByReference out = new PointerByReference();
        manifest = AimuxResult.result(ffi, ffi.aimux_manifest_json(out), out);
        if (!manifest.path("abiVersion").isIntegralNumber()
            || manifest.path("abiVersion").asLong() != ABI_VERSION
            || !OPS_VERSION.equals(manifest.path("opsVersion").asText())) {
            throw new IllegalStateException("Incompatible aimux ABI/ops manifest");
        }
    }

    public static Aimux load() { return load("aimux_ffi"); }
    public static Aimux load(String library) {
        return new Aimux(Native.load(Objects.requireNonNull(library, "library"), AimuxFFI.class,
            Collections.singletonMap(Library.OPTION_STRING_ENCODING, "UTF-8")));
    }

    public JsonNode getManifest() { return manifest.deepCopy(); }

    /** Global op call, using the exact request DTO described by the manifest. */
    public JsonNode call(String op, String paramsJson) {
        return call(0, op, AimuxResult.params(paramsJson), 0);
    }

    public <T> T call(String op, WireValue params, Class<T> resultType) {
        return WireJson.decode(call(op, params.toString()), resultType);
    }

    JsonNode call(long handle, String op, String params, long abort) {
        Objects.requireNonNull(op, "op");
        if (op.indexOf('\0') >= 0) throw new IllegalArgumentException("NUL in op");
        PointerByReference out = new PointerByReference();
        return AimuxResult.result(ffi, ffi.aimux_call(handle, op, params, abort, out), out);
    }

    public ProviderHandle createProvider(String paramsJson) {
        return new ProviderHandle(this, AimuxResult.handle(call(AimuxOps.OP_PROVIDER_CREATE, paramsJson)));
    }

    /** Generic owned resource for Registry, Recorder, Middleware and the other handle kinds. */
    public Resource create(String op, String paramsJson) {
        return new Resource(this, AimuxResult.handle(call(op, paramsJson)));
    }

    public Abort createAbort() {
        return new Abort(this, AimuxResult.handle(call(AimuxOps.OP_ABORT_CREATE, "{}")));
    }

    /** Takes ownership of a handle returned by an op; never wrap the same handle twice. */
    public Resource own(String decimalHandle) {
        return new Resource(this, AimuxResult.handle(WireJson.MAPPER.getNodeFactory().textNode(decimalHandle)));
    }

    public Future<JsonNode> callAsync(String op, String paramsJson, Executor executor) {
        return submit(null, op, paramsJson, null, executor);
    }

    Future<JsonNode> submit(NativeHandle resource, String op, String paramsJson,
                            Consumer<StreamFrame> consumer, Executor executor) {
        Objects.requireNonNull(executor, "executor");
        Objects.requireNonNull(op, "op");
        String params = AimuxResult.params(paramsJson);
        NativeHandle.Lease lease = resource == null ? null : resource.admit();
        Abort abort;
        try { abort = createAbort(); }
        catch (Throwable error) { if (lease != null) lease.close(); throw error; }
        return new NativeFuture(this, lease, abort, op, params, consumer, executor);
    }

    void stream(long handle, String op, String params, long abort, Consumer<StreamFrame> consumer) {
        Objects.requireNonNull(op, "op");
        if (op.indexOf('\0') >= 0) throw new IllegalArgumentException("NUL in op");
        StreamContext context = new StreamContext(this, abort, Objects.requireNonNull(consumer, "consumer"));
        long id = CONTEXT_IDS.incrementAndGet();
        CONTEXTS.put(id, context); // Java 8 strong reachability through the native return boundary.
        Pointer error = null;
        try {
            error = ffi.aimux_stream(handle, op, params, abort, context, context.memory);
            Throwable failure = context.failure.get();
            if (failure != null) {
                if (error != null) { ffi.aimux_error_free(error); error = null; }
                throw AimuxResult.callbackFailure(failure);
            }
            Pointer returnedError = error;
            error = null;
            AimuxResult.check(ffi, returnedError);
            if (context.terminal == null) throw new IllegalStateException("Native stream returned without End");
            NativeException terminalError = context.terminal.getError() == null ? null : AimuxResult.decode(context.terminal.getError());
            if (context.terminal.getOutcome() == Outcome.ERROR && terminalError == null) {
                throw new IllegalStateException("Error End is missing its envelope");
            }
            if (context.terminal.getOutcome() == Outcome.CANCELLED) {
                java.util.concurrent.CancellationException cancelled =
                    new java.util.concurrent.CancellationException("Native stream cancelled");
                if (terminalError != null) cancelled.initCause(terminalError);
                throw cancelled;
            }
            if (terminalError != null) throw terminalError;
        } finally {
            if (error != null) ffi.aimux_error_free(error);
            CONTEXTS.remove(id);
            context.memory.close();
        }
    }

    void drop(long id) {
        if (Boolean.TRUE.equals(IN_CALLBACK.get())) REAPER.execute(() -> ffi.aimux_drop(id));
        else ffi.aimux_drop(id);
    }

    private static final class StreamContext implements AimuxFFI.EventCallback {
        private final Aimux aimux;
        private final long abort;
        private final Consumer<StreamFrame> consumer;
        private final Memory memory = new Memory(1);
        private final AtomicReference<Throwable> failure = new AtomicReference<>();
        private BigInteger previous;
        private StreamFrame.End terminal;

        StreamContext(Aimux aimux, long abort, Consumer<StreamFrame> consumer) {
            this.aimux = aimux; this.abort = abort; this.consumer = consumer;
        }

        @Override public void invoke(Pointer json, Pointer ignored) {
            Boolean prior = IN_CALLBACK.get();
            IN_CALLBACK.set(true);
            try {
                if (failure.get() != null) return;
                StreamFrame frame = StreamFrame.fromJson(AimuxResult.parse(json.getString(0, "UTF-8")));
                if (terminal != null || (previous != null && WireJson.u64(frame.getSeq()).compareTo(previous) <= 0)) {
                    throw new IllegalStateException("Out-of-order frame or frame after End");
                }
                previous = WireJson.u64(frame.getSeq());
                if (frame instanceof StreamFrame.End) terminal = (StreamFrame.End) frame;
                consumer.accept(frame);
            } catch (Throwable error) {
                if (failure.compareAndSet(null, error)) {
                    try { aimux.ffi.aimux_abort(abort); }
                    catch (Throwable abortError) { error.addSuppressed(abortError); }
                }
            } finally {
                if (prior == null) IN_CALLBACK.remove(); else IN_CALLBACK.set(prior);
            }
        }
    }
}
