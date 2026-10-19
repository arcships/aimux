# aimux Kotlin coroutine facade

The Kotlin artifact depends on `ai.arcships:aimux-java` at the same version.
Java owns native loading, ABI/ops validation, resources, wire DTOs and error
decoding. Kotlin adds coroutines, bounded Flow collection and a sealed exception
facade in `ai.arcships.aimux.kotlin`.

```kotlin
import ai.arcships.aimux.Aimux
import ai.arcships.aimux.StreamFrame
import ai.arcships.aimux.kotlin.aimuxCall
import ai.arcships.aimux.kotlin.aimuxFlow
import kotlinx.coroutines.flow.collect

suspend fun generate() {
    val aimux = Aimux.load()
    aimux.createProvider("""{"package":"openai","settings":{}}""").use { provider ->
        provider.languageModel("gpt-4o-mini").use { model ->
            val params = """{"prompt":"Hello","options":{"maxRetries":0}}"""
            val result = aimuxCall { executor -> model.generateAsync(params, executor) }
            println(result)
            aimuxFlow<StreamFrame> { executor, consumer ->
                model.streamAsync(params, consumer, executor)
            }.collect { frame -> println(frame.json) }
        }
    }
}
```

Use the Java provider/model, `Types`, and `MultimodalTypes` objects directly.
They pass unchanged between Java and Kotlin; Java retains ownership and
`close()` semantics. Generic Java `Resource` operations expose all registered
ops, including Decision. For example, obtain a Decision resource from a Jev
provider with `provider.create("provider.model", request)`, where the request is
`{"method":"decision","modelId":"..."}`, then call `decision.capabilities` or
`decision.decide` on that resource with their manifest request DTOs.

`aimuxCall` starts and awaits Java Futures on `Dispatchers.IO`.
`Future.awaitAimux()` adapts an already started Java operation. Coroutine
cancellation invokes `Future.cancel(true)`, which signals the Java-owned Abort.
Java retains native resources until its work actually returns.

`aimuxFlow` is cold. Each collection starts its own Java operation. The supplied
executor must run all callback work, and the start function must return its
cancellation-aware Future promptly. Its channel holds 64 frames by default,
configurable from 1 through 256. A full channel blocks callbacks on IO without
dropping frames. Cancellation, `take(n)` and collector failures release blocked
callbacks and cancel the Future. Flow collection waits for the submitted work
to return even when cancellation has already completed the Future.

Java callbacks include both Event and End frames. Buffered frames precede a
terminal exception; successful Java completion closes the Flow. A business
error event remains ordinary data. Native callbacks stop before the executor's
task returns. A native operation that ignores Abort can delay Flow cleanup.
The channel bounds frame count; native stream limits govern frame byte sizes.

The sealed `AimuxException` delegates `code`, `kind`, `errorName`, `error`,
`envelope` and nullable unsigned `retryAfterMs` (`BigInteger`) to Java. All 22
known core subtypes have corresponding Kotlin variants with typed Java
`details`. `BindingError`, `ProtocolError` and `RecordingError` retain their Java
domain errors. `UnknownError` preserves a future Java exception subtype.
`javaError` is also the cause, so the Java `ErrorCause` chain follows it.
Kotlin performs no JSON error decoding. Java's current decoder rejects unknown
wire variants before a Kotlin projection can occur; those decoder failures
propagate unchanged.

## Breaking changes

The former independent Kotlin implementation in `ai.arcships.aimux` is removed,
including its JNA declarations, native loader, provider-specific constructors,
wire DTO copies and callback/Sequence streams. Import Java objects from that
package and Kotlin additions from `ai.arcships.aimux.kotlin`. No compatibility
aliases are provided. Pass the actual request DTO shape described by the native
manifest; for example, text ops use `{"prompt":...,"options":...}`.

## Build and test

Build `aimux-ffi` first and set `AIMUX_LIBRARY_PATH` to the absolute cdylib path.
The Gradle test task also accepts `AIMUX_FFI_LIB_DIR` for JNA's search path,
defaulting to `target/debug`.

```sh
AIMUX_LIBRARY_PATH=/absolute/path/libaimux_ffi.dylib \
  mise exec java@temurin-17 gradle@8 -- gradle -p bindings/kotlin test
mise exec java@temurin-17 gradle@8 -- gradle -p bindings/kotlin dependencies --configuration compileClasspath
```

The composite build substitutes the sibling Java source build for the Maven
coordinate. Version verification runs before Kotlin compilation; published
metadata retains the strict same-version Java dependency.

When sandbox restrictions prevent Gradle's lock socket, a cached-jar harness
can compile Java 8 bytecode, compile Kotlin/JVM 17 and invoke JUnit test methods
with real assertions. Set `AIMUX_LIBRARY_PATH` to include native tests; without
it the harness reports native tests as not run.

```sh
AIMUX_LIBRARY_PATH=/absolute/path/libaimux_ffi.dylib \
  mise exec java@temurin-17 gradle@8 -- python3 bindings/kotlin/scripts/test-offline.py
```

This harness does not verify Gradle resolution, publishing metadata, or the
JUnit engine. It requires cached Kotlin, coroutine, Jackson, JNA and JUnit jars.
