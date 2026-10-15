# Java binding — ABI 1 / ops 1.0

Java 8 is supported. Wire DTOs use ordinary classes; existing `Types` and
`MultimodalTypes` builders remain available. `AimuxFFI` declares the 14 C
primitives once. `size_t` uses the native pointer size, including Windows LLP64.
Operation names and version constants come from the packaged generated `AimuxOps`.

This is a breaking replacement of the unversioned binding. Load explicitly with
`Aimux.load()` or `Aimux.load(libraryPath)`. ABI and manifest ops versions are
checked before the client becomes usable. Provider-specific constructors have
been removed; provider settings are plain data.

```java
Aimux aimux = Aimux.load();
String providerJson = "{\"package\":\"openai\",\"settings\":{\"apiKey\":\"your-key\"}}";
String requestJson = "{\"prompt\":\"Hello\",\"options\":{\"maxRetries\":0}}";
try (ProviderHandle provider = aimux.createProvider(providerJson);
     Model model = provider.languageModel("gpt-4.1-mini")) {
    System.out.println(model.generate(requestJson));
    model.stream(requestJson, frame -> {
        if (!frame.isEnd()) System.out.println(frame.getEvent());
    });
}
```

Use the native request DTO for each operation. Parameters must be an object,
including `{}` for operations without parameters. Text operations use `prompt`
and `options`; embedding calls use `values`; `decision.decide` uses `options`.
Provider options pass through unchanged. Native validation rejects unknown
control fields. Handles are unsigned decimal strings; operation results may
return a handle string directly or an object containing `handle`.

`ProviderHandle` exposes Language, Embedding, Image, Speech, Transcription,
Reranking, Video, Search, Decision and Files. `decisionModel(id)` selects the
native `decision` method. Support depends on the provider. Every resource also
supports `call`, `stream` and `create` with the operations accepted by its native
handle kind. `Aimux.call` uses global handle 0. `Aimux.create` creates resources
such as registries, recorders, transports, middleware, trace stores and replay
sessions. `own(id)` transfers ownership; wrap each returned handle once.
`TypedModel` owns a supplied `Model` and decodes the existing core result classes.

Use try-with-resources. `close()` is idempotent and stops new calls immediately.
Admitted and queued calls retain the resource until their native calls return.
Closing a model does not signal cancellation; `close()` inside a Consumer never
waits for that Consumer to finish. A release requested from a callback runs on
a daemon cleanup thread. Explicit close governs ownership; no Java finalizer
blocks on native cleanup.

Every asynchronous method requires an explicit `Executor`. The internal
`Cancellation` helper forwards successful `Future.cancel(false)` and
`Future.cancel(true)` to native Abort, including cancellation before execution
starts. The Future becomes done immediately; running native work retains its
resources until it returns. Thread interruption is not used to stop native work.
For synchronous cancellation, use `createAbort()` and the `call`/`stream`
overloads that accept it.

Consumers receive copied `StreamFrame` values, including the unique End frame.
An error End is delivered before its typed exception is thrown. A cancelled End
throws `CancellationException` after native return, with its native error as the
cause when present. Slow Consumers apply synchronous backpressure and must return
in bounded time. Callback/context references stay alive until `aimux_stream`
returns. A Consumer Throwable is latched once, triggers Abort, suppresses further
Consumer calls, and becomes `BindingException.CallbackFailure` with the original
cause after native return. Exceptions never cross the C callback.

`NativeException` retains `kind`, `code`, message, the full `error` payload and
optional unsigned `retryAfterMs` as `BigInteger`. Domain exceptions select
subclasses by `error.name`; unfamiliar variants use the domain's `Unknown`
subclass and retain their payload. Retryability comes from `error.isRetryable`.
A missing retry hint differs from zero. The Java adapter performs no retries.

`TranscriptionSession` uses binary push, `operation.inputDone` and native polling.
`next(timeoutMs)` returns `PART`, `ENDED`, or `TIMEOUT`; timeouts must be
nonnegative. Operation failures throw exceptions. Cancel through
`operation.cancel`. Generic JSON calls cannot push binary data.

Removed APIs include implicit recording helpers, lock-lending repair hooks and
the unbounded Java Stream adapter. Use explicit recorder resources and the pure
`ToolCallRepairs` DTO operations. Core recorder/session/infer state remains
process-wide. Recording schema migration and business DTO schema generation are
separate work. Replay transport capabilities and credential creation currently
report UnsupportedFunctionality. Per-platform release asset installation is not
verified by this binding's tests.

## Tests

Build the native library from the repository root with `cargo build -p aimux-ffi`.
Point JNA at its directory and run the Java tests:

```sh
AIMUX_FFI_LIB_DIR=/absolute/path/to/target/debug gradle -p bindings/java test
```

The suite includes tests against the actual Rust library, native callback and
lifecycle cases using a deterministic C fixture compiled against `aimux.h`,
cancellation helper tests, and DTO round trips. The C fixture is test-only.
Real-library tests cover manifest loading, exported functions, provider/model
ownership, Decision, cancellation terminals, callback errors, binary limits and
poll validation without requiring provider network access.

When Gradle cannot open its local lock-listener socket, use the offline fallback
with a cached dependency classpath file:

```sh
AIMUX_FFI_LIB_DIR=/absolute/path/to/target/debug python3 bindings/java/verify-local.py /path/to/classpath.txt
```

The classpath file contains JNA 5.14, Jackson 2.17, JUnit Jupiter/Platform
(including the launcher), AssertJ and org.json JAR paths, separated by the
platform path separator. The fallback requires JDK 9 or newer, compiles all
sources including generated constants with `javac --release 8`, and discovers
all binding JUnit tests. Run the JDK 8/11/17/21 runtime matrix and a 32-bit
JVM/native library separately; a Java 8 bytecode target does not verify them.
