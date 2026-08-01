# RFC-0013: Java Bindings

> **Status**: DRAFT (pending review)
> **Date**: 2026-08-01
> **Related**: [RFC-0001](0001-multilang-bindings.md) multilang bindings, [RFC-0008](0008-multimodal-bindings.md) multimodal bindings, [RFC-0011](0011-golang-bindings.md) Golang bindings, [aimux-ffi.h](../aimux-ffi/aimux-ffi.h)

---

## 1. Background and Motivation

RFC-0001 §5.3 listed **Java / Scala** in the "third tier (worth doing, but each has friction)", with the reasoning "UniFFI (shared with Kotlin) or hand-written JNI; enterprise market, calling LLMs inside Spark; JVM GC needs `Closeable` for explicit release". Since then Go was promoted and landed ([RFC-0011](0011-golang-bindings.md)). Java is now the largest remaining gap: the JVM is the default platform of enterprise backends, and the Kotlin binding already proves the JNA + C ABI path works — Java can ride the same track at a lower cost than Go did.

### 1.1 Why Do Java Now

| Signal | Evidence |
|------|------|
| **Spring AI 2.0** | Released 2026-06; Spring AI is the de-facto standard for AI in Spring/Java enterprises |
| **LangChain4j** | Reached production-ready milestone releases in late 2025 |
| **Genkit Java** | Google's Genkit added Java support, actively maintained |
| **Official OpenAI Java SDK** | OpenAI shipped an official Java client, confirming enterprise demand |
| **JDK cadence** | Java 8 → 11 → 17 → 21 LTS adoption ladder means most enterprise JVMs are still 8/11/17; a binding must not assume 21+ |

The market consensus (e.g. Zep's research for Go, Spring's own messaging for Java): enterprise teams integrate LLM calls **inside existing JVM services** rather than building new polyglot services. aimux's positioning — "unify providers, don't orchestrate" — fits the enterprise pattern: Spring AI / LangChain4j are orchestration layers; aimux can sit **under** them as the provider access layer.

### 1.2 One Binding Serves the Whole JVM

A Java binding is automatically consumable from **Scala, Groovy, Clojure, and Kotlin (Java-API callers)**. RFC-0001 §5.3 grouped Java/Scala for this reason. The Kotlin binding remains for Kotlin-first users (and Android); the Java binding covers the enterprise Java/Scala surface.

### 1.3 Re-evaluation of the "Friction Points" in RFC-0001 §5.3

| Original concern | Re-evaluation | Conclusion |
|--------|---------|------|
| "UniFFI (shared with Kotlin) or hand-written JNI" | The Kotlin binding already ships a working **JNA** wrapper (bindings/kotlin, JNA 5.14.0). JNA is pure Java (no native compile step) and maps the C ABI via an interface + `Callback` — no JNI C code to maintain. UniFFI was rejected for all bindings in RFC-0001 §5.5 (cannot reuse aimux-ffi, narrow C ABI makes codegen pointless) | JNA, parity with Kotlin |
| "JVM GC needs `Closeable` for explicit release" | Proven pattern already in Kotlin's [Model.kt](../bindings/kotlin/src/main/kotlin/aimux/Model.kt): `AtomicLong` handle + idempotent `close()` + `finalize()` backstop | Solved by mirroring Kotlin |
| "Calling LLMs inside Spark" | JNA + blocking FFI call inside a worker thread is exactly the Spark executor model; no special support needed | Non-issue |

All friction points are resolved. Cost is **lower than the Go binding**: the JVM side of the C ABI contract is already implemented and tested in Kotlin, and the Java wrapper is a port, not a first implementation.

---

## 2. Technical Path: JNA (parity with Kotlin)

```
Java code ──JNA (pure Java)──→ libaimux_ffi (.so/.dylib/.dll) ──→ aimux-providers
```

| Option | Assessment | Adopted |
|------|------|:----:|
| **JNA** (`net.java.dev.jna:jna:5.14.0`) | Pure-Java jar, zero native build step, identical pattern to the Kotlin binding; Java 8+ | ✅ |
| Panama FFM (`java.lang.foreign`) | Finalized in Java 22; best raw performance, but excludes Java 8/11/17 — the majority of the enterprise JVM fleet | ❌ (future path, see §6.6) |
| Hand-written JNI | Per-platform C compile step + JNIEnv plumbing for ~39 functions; highest maintenance cost | ❌ |
| UniFFI | Rejected in RFC-0001 §5.5 for all bindings; cannot reuse the aimux-ffi C ABI | ❌ |
| gRPC sidecar | Extra process + hop; rejected for Go in RFC-0011 §2.1 | ❌ |

Design constraints inherited from aimux-ffi (unchanged from other C ABI bindings):

- **Wire format is JSON** — `prompt_json` / `opts_json` in, JSON result / `StreamPart` out (see [aimux-ffi.h](../aimux-ffi/aimux-ffi.h#L22))
- **Push-only streaming** — `aimux_stream_text` is a synchronous blocking call with callbacks (RFC-0001 §4.1)
- **Handle registry** — `u64` opaque handle; `aimux_drop_handle` releases it (RFC-0001 §4.2)

The C ABI has grown from the 6 functions of the RFC-0001 era to **39 functions** (9 core + 8 modalities × constructor/action pairs), so the JNA interface is a direct 1:1 port of [aimux-ffi.h](../aimux-ffi/aimux-ffi.h).

---

## 3. Streaming Mapping: JNA callback → BlockingQueue → `Stream` / `Iterator`

The C ABI is push-only and synchronous. Java gets three consumption shapes, mirroring the Kotlin binding's layering:

```
[aimux-ffi]               [JNA]                                            [Java user code]
aimux_stream_text   →   Callback onPart(json)  →  BlockingQueue<String>   →  Stream<String>
   (blocking)            (same thread)              (producer)                (pull consumer)
```

| Layer | Shape | Java API | Parity with |
|------|------|------|------|
| **Raw** | `onPart` / `onDone` / `onError` callbacks (blocking, same thread) | `streamText(...)` | Kotlin `Model.streamText` |
| **Pull** | `BlockingQueue` fed by the callback; consumer does `take()` | `Stream<String>` / `Iterator<String>` | Kotlin `streamTextSequence` (LinkedBlockingQueue + `Sequence`) |
| **Push (reactive)** | Out of scope for the base artifact — users wrap the callback/`Stream` API in Reactor/RxJava | — | Kotlin has no Flow adapter either |

Streaming **blocks the calling thread** (same contract as Kotlin/Go). The `Stream` variant is a lazy pull: iteration calls `queue.take()`; the sentinel `null` ends the stream; a `hasNext`-style error field is thrown as `AimuxException` on terminal iteration (mirror of Kotlin's `streamTextSequence`).

### 3.1 Threading contract

- All FFI functions are synchronous blocking; callbacks execute on the FFI-calling thread
- **Re-entering FFI inside a callback is forbidden** (would deadlock the tokio runtime) — documented in [aimux-ffi.h:20](../aimux-ffi/aimux-ffi.h#L20), enforced by documentation in the Javadoc
- The tokio runtime is managed internally by aimux-ffi; Java is unaware of it
- JNA `Callback` proxy objects **must be strongly referenced** for the duration of the native call (Kotlin holds them in locals — same pattern)

---

## 4. API Shape

Two layers, matching Kotlin's `Model` + `TypedModel` split and Go v0.2's coverage:

```java
// Raw JSON layer (only dependency: JNA)
try (Model model = Model.openai("sk-...", "gpt-4o")) {
    String result = model.generateText("\"What is Rust?\"");
    model.streamText("\"Write a haiku\"", part -> System.out.println(part),
                     () -> {}, err -> System.err.println(err));
}

// Typed layer (adds Jackson 2.x)
try (TypedModel model = TypedModel.openai("sk-...", "gpt-4o")) {
    GenerateTextResult r = model.generateText("What is Rust?");
    System.out.println(r.getText());

    Stream<StreamPart> parts = model.streamTextStream("\"Write a haiku\"");
    parts.forEach(System.out::println);
}

// Custom base URL (Ollama / OpenRouter / local proxy)
Model m = Model.openaiWithBase("sk-...", "gpt-4o", "http://localhost:11434");
```

### 4.1 Raw layer — `io.aimux.Model`

- Implements `java.io.Closeable` (mirror of Kotlin's `Closeable` / Go's `io.Closer`)
- Factories: `openai` / `openaiWithBase` / `anthropic` / `anthropicWithBase` / `deepseek` (all 39 C symbols exposed)
- `generateText(promptJson, optsJson)` returns a JSON string; errors come back as `{"error":"..."}`
- `streamText(promptJson, optsJson, onPart, onDone, onError)` — raw callbacks
- `streamTextStream(promptJson, optsJson)` — `Stream<String>` pull
- Handle lifecycle: `AtomicLong` + idempotent `close()` + `finalize()` backstop (Kotlin-proven, §6.3)

### 4.2 Typed layer — `io.aimux.TypedModel` + typed types

Port of Kotlin's [TypedModel.kt](../bindings/kotlin/src/main/kotlin/aimux/TypedModel.kt) and [Types.kt](../bindings/kotlin/src/main/kotlin/aimux/Types.kt) (1127 lines), serialized with **Jackson 2.x** (Java 8 compatible; the natural choice for the Spring ecosystem):

- Text: `GenerateTextOptions` / `GenerateTextResult` / `ModelMessage` / `ContentPart` / `TokenUsage` / `Usage` / `FinishReason` / `ResponseMetadata`
- Tools: `ToolCall` / `FunctionTool` / `ProviderTool` / `ToolChoice`
- Streaming: `StreamPart` (sealed hierarchy → Jackson `@JsonTypeInfo` polymorphism)
- **8 modalities** (same coverage as Go v0.2): Embedding / Speech / Image / Transcription / Files / Reranking / Video / Search — typed options + typed results + factory methods on `TypedModel`
- Error envelope: `{"error":"..."}` surfaces as `AimuxException` (checked-free, `RuntimeException` subclass, mirror of Kotlin's `AimuxException`)

### 4.3 Java 8 constraints on the typed layer

- **No `record`, no `var`, no `switch` expressions** — plain POJOs with private fields + getters + builders
- Estimated volume: ~40 types / ~1200 lines (≈ Kotlin's Types.kt line count, higher per-type cost without data classes)
- Decision: hand-written POJOs + static builders, **no Lombok** (annotation processor adds a compile-time dependency and codegen — repo stance since RFC-0001 §5.5 is "no codegen"). Re-evaluate Lombok only if the volume hurts maintenance
- Jackson pinned to a Java-8-compatible 2.x line

### 4.4 Packaging

| Item | Value |
|------|------|
| Group / artifact | `io.aimux:aimux-java` (Maven Central; the Kotlin binding publishes as `io.aimux` with Kotlin-specific artifact) |
| Package | `io.aimux` (reverse-domain convention; the Kotlin binding's flat `aimux` package is not idiomatic Java) |
| Minimum JDK | Java 8 (compiled with `--release 8`); tested on 8/11/17/21 |
| Dependencies | `net.java.dev.jna:jna:5.14.0` (core), `com.fasterxml.jackson.core:jackson-databind:2.x` (typed layer) |

---

## 5. Directory Structure

Aligns with other C ABI bindings:

```
bindings/java/
├── settings.gradle.kts          # Gradle build (decision: keep tooling consistent with Kotlin binding)
├── build.gradle.kts             # java-library + maven-publish; toolchain --release 8
├── gradle.properties
├── src/main/java/io/aimux/
│   ├── AimuxFFI.java            # JNA interface — 1:1 mapping of aimux-ffi.h (39 symbols)
│   ├── Model.java               # raw JSON API + Closeable handle lifecycle (~230 lines, port of Model.kt)
│   ├── TypedModel.java          # typed API: text + tools + 8 modalities (~300 lines, port of TypedModel.kt)
│   ├── Types.java               # typed wire types: POJOs + builders (~1200 lines, port of Types.kt)
│   ├── MultimodalTypes.java     # modality option/result types (embedding/speech/image/…)
│   └── AimuxException.java      # error envelope → RuntimeException
├── src/main/resources/native/   # per-platform libaimux_ffi.so/.dylib/.dll (classifier JARs in release, see §7.1)
├── src/test/java/io/aimux/
│   ├── ModelTest.java           # raw API (mirror of Kotlin ModelTest.kt)
│   ├── TypedModelTest.java      # typed API (mirror of Kotlin TypedModelTest.kt)
│   ├── MockProviderServer.java  # local mock HTTP server — port of Kotlin's
│   │                            #   MockProviderServer (StructuredE2ETest.kt:30),
│   │                            #   built on JDK's com.sun.net.httpserver (Java 8 built-in)
│   ├── MultimodalE2ETest.java   # 8-modality E2E suite (see §7.4)
│   └── ContractTest.java        # shared wire-format fixtures (see §7.3)
├── examples/
│   └── Generate.java            # minimal example
└── README.md                    # build/usage notes
```

Estimated code volume: **~2,500 lines of Java** (vs Go's ~2,900: the JNA surface is smaller than cgo glue because there is no native toolchain step).

---

## 6. Implementation Points

### 6.1 JNA interface

```java
public interface AimuxFFI extends Library {
    long aimux_openai_new(String apiKey, String modelId);
    long aimux_openai_new_with_base(String apiKey, String modelId, String baseUrl);
    // … all 39 symbols from aimux-ffi.h …

    Pointer aimux_generate_text(long handle, String promptJson, String optsJson);
    void aimux_stream_text(long handle, String promptJson, String optsJson,
                           Callback onPart, Callback onDone, Callback onError);
    void aimux_drop_handle(long handle);
    void aimux_free_string(Pointer ptr);
}
```

Loading: `Native.load("aimux_ffi", AimuxFFI.class)` — JNA resolves the library from `java.library.path`, `LD_LIBRARY_PATH` (tests), or the JAR's `native/` directory (packaged distribution).

### 6.2 Memory ownership

Follow the contract of [aimux-ffi.h](../aimux-ffi/aimux-ffi.h#L8):

- `char*` returned by `aimux_generate_text` (and every modality `*_generate` / `*_upload` / `aimux_embed`) **is owned by the caller** — Java reads `ptr.getString(0, "UTF-8")` then calls `aimux_free_string(ptr)` in `finally` (exact Kotlin pattern)
- `const char*` received by stream callbacks **is valid only during the callback** — copy synchronously (`ptr.getString(0, "UTF-8")` inside the callback, then enqueue)

### 6.3 Handle lifecycle

- `AtomicLong` handle; `close()` does `getAndSet(0)` + `aimux_drop_handle` — idempotent, thread-safe (Kotlin-proven)
- `finalize()` backstop for callers that forget `close()`; note in Javadoc that it is unreliable and `try-with-resources` is the primary path
- Java 9's `Cleaner` is **not** used — baseline is Java 8; revisit only if the baseline moves up

### 6.4 Callback GC safety

JNA `Callback` proxies are Java objects. They must be strongly referenced for the duration of the native call or the JVM may GC them mid-stream. Kotlin solves this with locals; Java uses local variables in the same way (JNA 5.14 also keeps `Callback` proxies referenced via the `Pointer` passed to native code, but the explicit local-hold pattern is kept for parity).

### 6.5 Concurrency

- `GenerateText` / `streamText` are synchronous blocking; they do **not** spawn threads (same contract as Kotlin/Go)
- `streamTextStream` uses a `LinkedBlockingQueue<String>` (Kotlin parity); iteration `take()`s until the `null` sentinel
- No FFI re-entry inside callbacks (documented in Javadoc; matches [aimux-ffi.h:20](../aimux-ffi/aimux-ffi.h#L20))

### 6.6 Future path: Panama FFM

When the minimum JDK moves to 21+, `java.lang.foreign` (JEP 454) can replace JNA with a hand-written linker session for the same 39 symbols — same wrapper API, lower FFI overhead, no third-party dependency. Deliberately not adopted now: Java 8/11/17 coverage is the whole point of the enterprise market. Keep the `AimuxFFI` interface as the seam so a later swap is contained.

---

## 7. Release Strategy

### 7.1 Distribution

- Gradle `java-library` + `maven-publish` + signing → Maven Central (`io.aimux:aimux-java`)
- Native library shipped as **per-platform classifier JARs** (napi-rs pattern, RFC-0001 §7.1): `aimux-java-linux-x86_64`, `aimux-java-macos-aarch64`, `aimux-java-windows-x86_64`, …; the base JAR declares JNA as a dependency and resolves the platform artifact at runtime
- Test-time loading: `LD_LIBRARY_PATH` pointing at `target/release` (exact Kotlin test convention, bindings/README.md §Kotlin)

### 7.2 CI matrix

Add a `Java binding` job to [.github/workflows/ci.yml](../.github/workflows/ci.yml) (currently no JVM job exists — this is new work):

| Platform | Steps |
|------|------|
| Linux x86_64 | `cargo build -p aimux-ffi --release` → `gradle test` (JDK 8/11/17/21 test matrix) |
| macOS / Windows | Same, consuming the ffi artifacts from the existing `aimux-ffi` matrix job |

### 7.3 Contract tests

Reuse [contract-tests/fixtures/wire-format.json](../contract-tests/fixtures/wire-format.json) via a `ContractTest.java` (JUnit + `org.json`, mirroring `run-node.ts`'s assertions and Kotlin's `org.json` usage) — ensures the Java wire format matches the other 7 languages.

### 7.4 Multimodal E2E tests

The pattern established in commit `5771ee38` (2026-08-01, "multimodal E2E tests for all 6 bindings") is a per-binding requirement, not a Go/Kotlin special: **local mock HTTP server replaying canned provider responses → real FFI call → wire-format result parsing assertions**. No real network access (every request hits 127.0.0.1).

Java ports the Kotlin suite 1:1 — [MultimodalE2ETest.kt](../bindings/kotlin/src/test/kotlin/aimux/MultimodalE2ETest.kt), which itself mirrors [Go's multimodal_withbase_test.go](../bindings/go/multimodal_withbase_test.go). The mock server is a direct port of Kotlin's `MockProviderServer` (JDK `com.sun.net.httpserver.HttpServer` — built into Java 8, zero extra dependency):

| Modality | E2E coverage | Notes |
|------|------|------|
| Embedding | Full round-trip | Canned OpenAI embeddings response → assert `embeddings` array |
| Speech (TTS) | Full round-trip | `audio/mpeg` content-type + ASCII-safe base64 body trick (Kotlin/Go parity) → assert `audio.Binary` |
| Image | Full round-trip | Canned `b64_json` → assert `images.Base64` |
| Transcription (STT) | Full round-trip | Canned `{"text":...}` → assert `text` |
| Reranking | Full round-trip | Canned ranking → assert `ranking` order + scores |
| Search | Full round-trip | Canned results → assert `results`/`answer` |
| Files | Full round-trip | Canned provider file object → assert `provider_reference` |
| **Video** | Construction + result parsing only | Google's multi-step async API (POST predict → poll → fetch) can't be driven by a single-response mock — same limitation as Go/Kotlin |

Assertions use `org.json` (already the Kotlin test dependency); typed-layer tests additionally assert Jackson deserialization of the same wire JSON.

Acceptance: `MultimodalE2ETest` green on JDK 8/11/17/21 as part of Phase 3.

### 7.5 Docs sync (same task as the binding)

- `bindings/README.md`: add Java row to the binding table (C ABI path, JNA, status) + build instructions
- Top-level `README.md`: binding count 7 → 8
- `docs/api/java.md`: new per-language guide (same structure as [kotlin.md](../docs/api/kotlin.md)), plus a Java column in the [Feature Coverage](../docs/API.md#feature-coverage) matrix and a Java row in [gaps.md](../docs/api/gaps.md)
- This RFC's status: DRAFT → implemented

---

## 8. Risks

| # | Risk | Description | Mitigation |
|---|------|------|------|
| 1 | **JNA callback GC** | Native callback into a GC'd `Callback` proxy crashes or drops parts | Strong local references for the call duration (Kotlin-proven); JNA 5.14 keeps proxies referenced from the native side |
| 2 | **Java 8 typed-layer volume** | ~1,200 lines of hand-written POJOs + builders, no `record` | Static builders + `@JsonTypeInfo` polymorphism; Lombok fallback only if maintenance cost hurts |
| 3 | **Per-platform native JARs** | `.so/.dylib/.dll` must be matched to the user's platform | Classifier JARs from the existing CI `aimux-ffi` matrix (napi-rs pattern) |
| 4 | **`finalize()` unreliability** | Handle leak if users forget `close()` | `try-with-resources` is the documented primary path; `finalize()` is a best-effort backstop identical to the Kotlin binding |
| 5 | **No JVM job in CI today** | Kotlin binding is not CI-covered; Java CI is net-new | The `Java binding` job (§7.2) covers build + tests from day one |
| 6 | **Jackson/org.json on Android** | Android ships a stripped `org.json`; typed layer uses Jackson | Java binding targets JVM servers first; Android stays the Kotlin binding's home (JNA's Android artifact works for Java too, but packaging `.aar` is out of scope) |

---

## 9. Implementation Roadmap

| Phase | Content | Status |
|------|------|------|
| **Phase 0** | This RFC passes review | ⏳ To do |
| **Phase 1** | `bindings/java/` skeleton: Gradle build (Java 8 target) + `AimuxFFI` JNA interface + `Model` raw API + `ModelTest` (port of Kotlin) | ⏳ To do |
| **Phase 2** | Typed layer: `Types.java` + `TypedModel` text/tools (Jackson) + `TypedModelTest` | ⏳ To do |
| **Phase 3** | 8-modality multimodal (Embedding/Speech/Image/Transcription/Files/Reranking/Video/Search) + factories + **MultimodalE2ETest** (mock-server E2E, port of Kotlin's suite, §7.4) | ⏳ To do |
| **Phase 4** | Contract tests (`ContractTest.java` on shared fixtures) + CI matrix job + Maven Central publish config | ⏳ To do |
| **Phase 5** | Docs sync (bindings/README.md / README.md / docs/api/java.md + Feature Coverage column + gaps.md row) + examples | ⏳ To do |

Acceptance: `gradle test` green on JDK 8/11/17/21 — including the 8-modality `MultimodalE2ETest` suite (7 full round-trips + video construction/parsing) and the contract fixtures; `cargo test --workspace` untouched and green.

---

## Revision History

| Date | Version | Description |
|------|------|------|
| 2026-08-01 | DRAFT v0.1 | Initial draft: Java promoted from RFC-0001 §5.3 third tier to the next binding after Go; JNA path (parity with the Kotlin binding) decided with zero native toolchain step; raw + typed two-layer API with 8-modality coverage (Go v0.2 parity); Java 8 baseline; Gradle build; streaming mapping JNA callback → BlockingQueue → Stream/Iterator; per-platform classifier JAR release plan |
| 2026-08-01 | DRAFT v0.1.1 | Added §7.4 multimodal E2E test plan: Java ports the 8-modality mock-server suite established in commit `5771ee38` (Kotlin `MultimodalE2ETest.kt` → `MultimodalE2ETest.java` + JDK `HttpServer`-based `MockProviderServer`); video limited to construction + result parsing (Go/Kotlin parity); Phase 3 acceptance and §7.5 docs sync extended with `docs/api/java.md` + Feature Coverage column + `gaps.md` row |
| 2026-08-01 | DRAFT v0.1.2 (implemented) | **Phases 1–5 landed**: `bindings/java/` complete — JNA interface over all **39** C ABI symbols (corrected from the draft's 36; `cohere_embedding`/`google_embedding`/`google_image` `_with_base` variants added by commit `5771ee38` were not counted), raw `Model` + typed `TypedModel`/`Types` (Jackson, Java 8 POJOs), 8-modality `Multimodal`/`MultimodalTypes`, 32 tests green (ModelTest 9 + TypedModelTest 5 + StructuredE2ETest 5 + MultimodalE2ETest 8 + ContractTest 5) on mock servers (zero real network); CI `java-binding` job (JDK 8/11/17/21 matrix) + Maven Central publish config; docs synced (bindings/README.md, README.md 7→8 bindings, docs/api/java.md, Feature Coverage column, gaps.md). Fixed a latent `StreamPartSerializer`/`ContentPartSerializer`/`GenerateContentSerializer` self-recursion (`valueToTree` re-entering the polymorphic serializer → `StackOverflowError`) by introducing an inner mapper without the externally-tagged serializers; ContractTest now does honest round-trips through the public mapper |
