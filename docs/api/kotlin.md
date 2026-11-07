# aimux · Kotlin API

> **0.6.0.** Kotlin now depends on the matching Java artifact and uses
> ABI 1 / ops 1.0. The former independent Kotlin JNA implementation is
> removed. See the [0.6 upgrade guide](../upgrading-to-0.6.md).

The Kotlin artifact adds coroutines, bounded Flow collection and a sealed
exception facade in `ai.arcships.aimux.kotlin`. Java owns native loading,
ABI/ops validation, resources, wire DTOs and error decoding.

## Install

```kotlin
implementation("ai.arcships:aimux-kotlin:0.6.0")
```

This pulls in `ai.arcships:aimux-java` at the same version. Provide the native
`aimux_ffi` library on the JNA search path as for Java.

## Quick start

```kotlin
import ai.arcships.aimux.Aimux
import ai.arcships.aimux.types.StreamFrame
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
            }.collect { frame -> println("${frame.type}: ${frame.event ?: frame.outcome}") }
        }
    }
}
```

Use the Java provider/model and generated `ai.arcships.aimux.types` classes
directly. They pass unchanged between Java and Kotlin; Java retains ownership
and `close()` semantics. Generic resource operations expose all registered
ops, including Decision.

## Coroutines and streams

- `aimuxCall` starts and awaits Java Futures on `Dispatchers.IO`.
- `Future.awaitAimux()` adapts an already started Java operation.
- Coroutine cancellation invokes `Future.cancel(true)`, which signals the
  Java-owned Abort. Java retains native resources until its work returns.
- `aimuxFlow` is cold; each collection starts its own Java operation. The
  supplied executor must run all callback work.
- The Flow channel holds 64 frames by default (1–256 configurable). A full
  channel blocks callbacks without dropping frames; cancellation, `take(n)`
  and collector failures release blocked callbacks.
- Java callbacks include both Event and End frames. Buffered frames precede a
  terminal exception; a business error event remains ordinary data.

## Errors

`AimuxException` delegates `code`, `kind`, `errorName`, `error`, `envelope` to
Java. All 22 known core subtypes have Kotlin variants. `javaError` is the
cause, so the Java `ErrorCause` chain follows it. Kotlin performs no JSON error
decoding.

Read a retry delay from `error.responseHeaders`, not from a dedicated retry
field.

## Breaking changes in 0.6

- The independent Kotlin implementation in `ai.arcships.aimux` is removed,
  including its JNA declarations, native loader, provider constructors, DTO
  copies and callback/Sequence streams.
- Import Java objects from `ai.arcships.aimux` and Kotlin additions from
  `ai.arcships.aimux.kotlin`. No compatibility aliases are provided.
- Pass the canonical request DTO shape described by the native manifest.
- Errors are the shared envelope; the old Kotlin error hierarchy is replaced
  by the Java-backed sealed facade.

## See also

- [bindings/kotlin/README.md](../../bindings/kotlin/README.md) — coroutine,
  Flow and test details.
- [docs/api/ops.md](ops.md) — operation names.
- [docs/error-model.md](../error-model.md) — error codes and cause chains.
- [Kotlin provider list](providers.md).
