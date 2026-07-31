# RFC-0001: Multi-language Bindings

> **Status**: v0.5.1 (all phases implemented; Flutter corrected to the dart:ffi C ABI path)
> **Author**: —
> **Date**: 2026-07-26
> **Related**: [aimux-core](../aimux-core), [aimux-providers](../aimux-providers)

---

## 1. Background and Motivation

`aimux` is currently a pure Rust workspace, benchmarked against the Vercel AI SDK (TypeScript-native). The goal is to provide core capabilities as a library to multi-language ecosystems such as **Node, Swift, Kotlin, Flutter, Python**, avoiding re-implementing provider adaptation and stream parsing in each language.

### 1.1 Where the reuse value lies

The most expensive and error-prone part of cross-language rewriting is the **provider adaptation logic**:

- HTTP request construction for OpenAI / Anthropic / Bedrock etc.
- SSE / NDJSON streaming decoding (see [aimux-stream](../aimux-stream/src/lib.rs))
- Conversion from response to `StreamPart` (see [stream_part.rs](../aimux-core/src/stream_part.rs))
- Rate limiting, retry, backoff (see [aimux-provider-utils/src/retry.rs](../aimux-provider-utils/src/retry.rs))

This part is large in volume and stable, and is the true reuse asset of the Rust core. The user-facing API (`generate_text` / `stream_text`) and the provider-facing trait (`do_generate` / `do_stream`) have already been decoupled in [generate.rs](../aimux-core/src/generate.rs) — this is the foundation for FFI-ization.

### 1.2 Strategic premise (must be answered first)

> ⚠️ This project benchmarks against the Vercel AI SDK, which is itself TypeScript-native.

- **The Node binding is the flagship battleground** — the primary goal of this project's existence is to defeat the Vercel AI SDK. Although the TS ecosystem has an official implementation, aimux's selling point is not "yet another TS SDK", but **a unified Rust core reused across languages + performance + 172-provider coverage**. As the first binding, Node directly proves the competitiveness of the Rust core on AISDK's home turf, and is the fulcrum of the entire multi-language strategy.
- **Python** has the largest AI/ML developer base; although LangChain/LlamaIndex exist, it lacks an SDK that is "lightweight, with a unified provider interface, and does not hijack the architecture". The Rust core can fill this gap — second priority.
- **Swift / Kotlin / Flutter** ecosystems have no official unified AI SDK; mobile is sensitive to binary size, performance, and offline use, where the Rust core is competitive — third tier, a value zone but not the primary battleground.

**Recommendation**: The motivation should focus on Node (flagship, benchmarked against AISDK) + Python (AI/ML native language), with mobile (Swift/Kotlin/Flutter) as a value extension.

---

## 2. Current-state analysis: how friendly is the current architecture to multi-language

After checking the core types one by one, the conclusions fall into two categories.

### 2.1 ✅ Naturally cross-language friendly (data types, serde-serializable)

- `GenerateContent` / `GenerateResult` / `StreamPart` / `Usage` / `FinishReason` / `Warning` (see [result.rs](../aimux-core/src/result.rs), [stream_part.rs](../aimux-core/src/stream_part.rs), [types.rs](../aimux-core/src/types.rs))
- `CallOptions` / `GenerateTextOptions` / `ModelMessage` / `ContentPart` (see [options.rs](../aimux-core/src/options.rs), [message.rs](../aimux-core/src/message.rs))
- `FunctionTool` / `ToolCall` / `ToolResult` (see [tool.rs](../aimux-core/src/tool.rs))
- `AiMuxError` (enum, serializable, see [error.rs](../aimux-core/src/error.rs))

### 2.2 ⛔ Stumbling blocks across the FFI boundary (Rust-specific abstractions)

| Location | Problem | Why it's hard |
|------|------|---------|
| [result.rs:56](../aimux-core/src/result.rs#L56) `Pin<Box<dyn Stream<Item=Result<StreamPart,AiMuxError>>+Send>>` | Stream trait object | **The number-one problem**. Rust's `Stream` cannot be passed directly to JS/Swift/Kotlin; each language's async model is completely different |
| [language_model.rs:25](../aimux-core/src/language_model.rs#L25) `LanguageModel` trait + `Box<dyn LanguageModel>` | trait object dynamic dispatch | FFI cannot pass `&dyn`; needs opaque handle + registry |
| [generate.rs:154](../aimux-core/src/generate.rs#L154) `impl Into<ModelPrompt>` | Generic input parameter | FFI can only pass concrete types |

> **Note**: Providers hard-binding `reqwest` + `tokio` was originally listed in this table ("mobile will duplicate the native HTTP stack, increasing binary size"). v0.2 removed this judgment — reqwest+rustls is cross-platform enough and does not constitute an FFI obstacle; see §4.5 for details.

---

## 3. Recommended approach: layered + dual-path bindings

Don't "hard-bind one Rust library to four languages", and don't "write the providers once per language". The repository is divided into a contract layer (`aimux-core`), an engine layer (`aimux-providers` and other reusable Rust assets), and a binding layer (`bindings/*`); between the engine and the bindings there are two seams to choose from — native bindings connect directly to the engine, and C ABI bindings go through `aimux-ffi`:

```
aimux/                         # Cargo workspace only manages Rust crates
├── aimux-core/                # Contract layer: data types + trait (full serde, language-agnostic)
├── aimux-stream/              # SSE/NDJSON parsing primitives (no IO)
├── aimux-provider-utils/      # retry/backoff/key loading (pure logic)
├── aimux-providers/           # 172 provider engines (hard-bound reqwest+tokio, shared by all bindings, untouched)
├── aimux-ffi/                 # [NEW] C ABI seam: opaque handle + streaming push callback (for C ABI path)
├── Cargo.toml
└── bindings/                  # thin bindings per language, each with own build system, not in cargo workspace
    ├── python/                # PyO3 + maturin   ── directly consumes aimux-providers (native path)
    ├── node/                  # napi-rs          ── directly consumes aimux-providers (native path)
    ├── flutter/               # dart:ffi         ── calls aimux-ffi (C ABI path, pure Dart no Rust crate)
    ├── swift/                 # module.modulemap ── calls aimux-ffi (C ABI path)
    ├── kotlin/                # JNA              ── calls aimux-ffi (C ABI path)
    └── c/                     # direct link .h   ── calls aimux-ffi (C ABI path)
```

### 3.1 Core principles

The FFI boundary only carries three kinds of things, and never carries Rust's trait / generic / Stream:

1. **Serialized JSON** (data)
2. **opaque handle** (object, a `u64` integer ID)
3. **callback** (streaming callback)

Each language binding is responsible for wrapping these into an API idiomatic to that language.

### 3.2 Dual-path binding strategy (v0.5 revision: Flutter moved to the C ABI path)

Don't make all languages go through `aimux-ffi`'s C ABI. Split into two paths based on the maturity of each language ecosystem's Rust binding tooling:

| Path | Applicable languages | Dependency | Rationale |
|------|---------|------|------|
| **Native binding** | Python / Node | `aimux-core` + `aimux-providers`, **bypassing `aimux-ffi`** | PyO3 / napi-rs can directly map Rust types and async, with the best DX and one less layer of indirection |
| **C ABI binding** | Swift / Kotlin / Flutter / C/C++ | `aimux-ffi` + hand-written wrapper (dart:ffi / JNA / module.modulemap / direct linking) | These languages have no self-contained Rust native binding tool, or the tool's codegen step produces no value under aimux's JSON boundary |

> **v0.5 correction**: Flutter was originally listed on the native path (flutter_rust_bridge); after evaluation it was moved to the C ABI path. Reasons: ① flutter_rust_bridge's `StreamSink` is not publicly exported and requires codegen + the Flutter SDK to compile, unlike PyO3/napi-rs which are self-contained; ② aimux's cross-boundary protocol is JSON (§3.1), so frb's "auto-map Rust types → Dart" advantage does not hold under a JSON boundary; ③ dart:ffi directly calls aimux-ffi's 6 C functions, unifying the path with Swift/Kotlin, with zero extra toolchain. See §5.5 for the investigation notes.

**Organizational impact**: `bindings/python` and `bindings/node`'s Cargo.toml depend on `aimux-providers` and **do not depend on** `aimux-ffi`. `bindings/swift`, `bindings/kotlin`, `bindings/flutter`, and `bindings/c` call `aimux-ffi`'s C ABI. The Flutter binding is pure Dart, with no Rust crate.

---

## 4. FFI boundary design points

### 4.1 Cross-boundary abstraction of streams (the number-one problem, most worth finalizing now)

Currently [generate.rs:101](../aimux-core/src/generate.rs#L101) directly exposes `Pin<Box<dyn Stream>>`. The FFI layer needs to convert it into a form consumable by each language.

**v0.2.1 conclusion: the C ABI path only does push (callback), not pull (polling).**

- **Push mode** (callback, adopted): `register_callback(handle, on_part, on_done, on_error)`
  - The Rust side `spawn`s a task that does `.next().await` normally in tokio, and calls back to notify the foreign language each time it gets a chunk. The foreign-language side pushes the callback data into a channel/buffer, and its own AsyncSequence/Flow pulls from the buffer.
  - **Why push is necessary**: C ABI synchronous functions (`extern "C"`) cannot `.await` Rust's async stream. The pull-mode `stream_next(handle) -> Option<json>` can only get the next chunk by blocking the current thread to wait — on the Swift/Kotlin main thread this would **freeze the UI**, which is a fatal flaw. Push uses callbacks to bypass synchronous blocking.
  - Push on the transport side, pull on the consumer side, decoupled in the middle by a channel — this is the de facto standard for cross-language streams (napi-rs / PyO3 / flutter_rust_bridge all use this pattern).
- ~~**Pull mode** (polling): `stream_next(handle) -> Option<StreamPartJson>`~~ — **rejected in v0.2.1**: C ABI synchronous functions cannot await an async stream, so pull inevitably degrades into blocking wait, freezing the main-thread environment. Although the consumer side of Swift's `AsyncSequence` / Kotlin's `Flow` is pull, that just needs to connect to the push transport side via a channel/buffer; the FFI layer does not need to do pull too.

> Note: This conclusion **only affects `aimux-ffi` (the Swift/Kotlin/C path, third tier)**. Node/Python/Flutter take the native binding path; napi-rs / PyO3 / flutter_rust_bridge each come with their own Rust Stream → that language's async bridging, and do not go through this push/pull design layer. The flagship Node binding is unaffected.

`StreamPart` is already a serializable enum (see [stream_part.rs](../aimux-core/src/stream_part.rs)); converting it to tagged JSON lets it cross the boundary.

> ⚠️ This is the largest part of the workload in the entire proposal, and also the part most worth finalizing during the review period.

### 4.2 Opaque handle + registry (replacing trait object)

FFI cannot pass `Box<dyn LanguageModel>`. Maintain an integer-ID registry of `Arc<dyn LanguageModel>` in `aimux-ffi`:

```rust
// aimux-ffi draft
static REGISTRY: Mutex<HashMap<u64, Arc<dyn LanguageModel>>> = ...;

fn create_openai_model(api_key: &str, model_id: &str) -> u64;   // returns handle
fn generate_text(model_handle: u64, prompt_json: &str, opts_json: &str) -> String; // JSON result
fn drop_handle(handle: u64);                                     // destructor
```

Each language wraps the `u64` handle into an object after obtaining it, and calls `drop_handle` on destruction. JVM/.NET must use the `Closeable` / `IDisposable` pattern for explicit release to avoid native memory leaks.

### 4.3 Tool definitions: already data descriptions, no rework needed

> **v0.2.1 correction**: The original section title "change from macro to data description" was a wrong premise — after checking the code, the data description was found to **have been ready long ago**.

The core types in [tool.rs](../aimux-core/src/tool.rs) are all already language-agnostic data descriptions, and already derive serde:

- `FunctionTool` ([tool.rs:10](../aimux-core/src/tool.rs#L10)): `name` + `input_schema: Value` (JSON Schema) + already `#[derive(Serialize, Deserialize)]` ✅
- `ToolCall` / `ToolResult` ([tool.rs:96](../aimux-core/src/tool.rs#L96) / [:107](../aimux-core/src/tool.rs#L107)): already serde ✅



No rework is needed for cross-language use: foreign-language users construct a `FunctionTool` with objects and pass it in, and after receiving a `ToolCall` they **execute it themselves on the foreign-language side**, then feed the `ToolResult` back into the next round. The entire chain is already-serialized data, without touching Rust macros.

~~The only omission: `ToolChoice` ([tool.rs:117](../aimux-core/src/tool.rs#L117)) lacked `Serialize/Deserialize`; adding a derive would suffice (3 minutes) and does not count as rework.~~ — **Completed in v0.2.1**: `ToolChoice` has been given serde, and its wire format is aligned with AISDK (`"auto"|"none"|"required"|{type:"tool",toolName}`), with hand-written Serialize/Deserialize + 8 contract tests ([tool_choice_test.rs](../aimux-core/tests/tool_choice_test.rs)). This is the first landed example of cross-language wire-schema alignment.

### 4.4 Add serde to data types + fix the wire schema

Many cross-boundary types currently do not derive serde (e.g. `StreamPart` only has `Debug`). FFI going through a JSON boundary requires all cross-boundary types to be serializable. Recommendations:

- Add `#[derive(Serialize, Deserialize)]` to all cross-boundary types
- Use a version field (e.g. `"specVersion": "v4"`) to lock the contract, avoiding silently breaking other languages when the Rust side is refactored

### 4.5 ~~Isolate tokio / reqwest dependencies~~ — **deleted in v0.2**

~~Providers currently hard-bind `reqwest` + `tokio`. It was suggested to abstract an `HttpTransport` trait, allowing injection of native HTTP stacks (iOS `URLSession`, Android `OkHttp`).~~

**Reason for deletion**: reqwest + rustls is itself cross-platform and can be compiled to iOS/Android, so there is no need to abstract a separate transport layer for mobile. All bindings consume the same complete engine; the mobile binary-size issue is instead addressed by narrowing tokio features + `strip` + LTO, without touching the architecture. What actually takes up size is the tokio runtime, not reqwest. Abstracting `HttpTransport` would force all 172 providers to change their signatures — a large workload whose benefit lands only on mobile, so the cost outweighs the gain.

---

## 5. Per-language approaches

### 5.1 First tier (must do)

| Language | Tool | Key points |
|------|------|--------|
| **Node.js** | `napi-rs` | **Flagship binding, done first**. Benchmarked against the Vercel AI SDK, proving the Rust core's competitiveness on its home turf; napi-rs directly exposes Promise/AsyncIterator, with DX no worse than native TS; native binding path, bypassing `aimux-ffi` |
| **Python** | `PyO3` + `maturin` | AI/ML native language; async generator maps to Rust Stream; selling point "lightweight unified provider abstraction + performance + GIL-free stream parsing"; second priority |
| **Swift** | module.modulemap + hand-written C FFI | callback stream → `AsyncSequence`; iOS binary needs `xcframework`; C ABI path |
| **Kotlin** | JNA | Auto-maps the C ABI; Android needs `.so` + `.aar`; `Closeable` for explicit handle release; C ABI path |
| **Flutter/Dart** | hand-written `dart:ffi` | Directly calls aimux-ffi's 6 C functions; no codegen needed; C ABI path (changed from flutter_rust_bridge to dart:ffi in v0.5) |

> **v0.2 adjustment**: The first binding is set to **Node.js** (napi-rs). The strategic goal is to defeat the Vercel AI SDK; Node is the main battleground, and the first binding must prove the Rust core on AISDK's home turf. Python is demoted to second priority — the task of validating streaming cross-boundary feasibility is also taken on by Node (napi-rs's AsyncIterator can equally validate streaming consistency, and directly serves the flagship goal).

### 5.2 Second tier (almost free, done along the way)

| Language | Tool | Notes |
|------|------|------|
| **C / C++** | `cbindgen` generates `.h` | `aimux-ffi` is itself a C ABI, at almost zero cost; C++ just wraps a layer of RAII. Embedded/edge/game-engine scenarios |
| **Zig** | direct `extern "C"` | Same path as C; small ecosystem but overlaps with the Rust user base |

### 5.3 Third tier (worth doing, but each has friction)

| Language | Tool | Friction points |
|------|------|--------|
| ~~**Go**~~ → upgraded to independent RFC-0011 for execution | `cgo` + C ABI | Originally listed as "CGo has overhead / Go has no async / gRPC sidecar is too heavy" — RFC-0011 re-evaluated and resolved all three concerns (cgo overhead is negligible for network IO; aimux-ffi push-callback naturally maps to Go CSP; gRPC sidecar is not adopted). See [RFC-0011](0011-golang-bindings.md) |
| **Java / Scala** | `UniFFI` (shared with Kotlin) or hand-written JNI | Enterprise market, calling LLMs inside Spark; JVM GC needs `Closeable` for explicit release |
| **C# / .NET** | `UniFFI` or `P/Invoke` | Windows ecosystem, Unity; `IAsyncEnumerable<T>` needs a manually connected layer |

### 5.4 Not recommended

Ruby / PHP / Elixir / Perl / Lua — low share in AI scenarios; maintenance cost > benefit.

### 5.5 Dual-path summary (v0.5 revision: UniFFI not adopted after evaluation)

Don't design a separate FFI for each language. Follow the §3.2 dual-path strategy:

| Path | Toolchain | Covered languages | Dependency |
|------|--------|---------|------|
| **Native binding** | `PyO3` + maturin | Python | `aimux-providers` direct |
| **Native binding** | `napi-rs` | Node.js | `aimux-providers` direct |
| **C ABI binding** | hand-written wrapper (module.modulemap) | Swift | `aimux-ffi` |
| **C ABI binding** | hand-written wrapper (JNA) | Kotlin / Java | `aimux-ffi` |
| **C ABI binding** | hand-written wrapper (dart:ffi) | Flutter / Dart | `aimux-ffi` |
| **C ABI binding** | hand-written wrapper (direct linking .h) | C / C++ | `aimux-ffi` |

> ~~`UniFFI`~~ — **not adopted after v0.5 evaluation**. Reasons:
> 1. aimux-ffi has only 6 C functions; hand-written Swift/Kotlin/Dart wrappers are ~150 lines each, with very low maintenance cost; UniFFI's codegen leverage does not hold under such a narrow C ABI surface.
> 2. UniFFI has its own FFI layer and cannot reuse the existing aimux-ffi C ABI — it needs to regenerate FFI glue from Rust traits, effectively making aimux-ffi's work wasted.
> 3. UniFFI's async/stream support is still being improved (callback interface), and is less mature than the current push callback → channel → AsyncSequence/Sequence chain.
> 4. The native binding path already covers Python/Node; Swift/Kotlin/Flutter hand-written wrappers are thin enough. UniFFI's "define once, generate for many languages" advantage falls through under the dual-path architecture.

> ~~`flutter_rust_bridge`~~ — **not adopted after v0.5 evaluation**. Reasons:
> 1. `StreamSink` is not publicly exported and requires codegen + the Flutter SDK to compile, unlike PyO3/napi-rs which are self-contained (usable with just `cargo build`).
> 2. aimux's cross-boundary protocol is JSON (§3.1), so frb's core value "auto-map Rust types → Dart class" does not hold under a JSON boundary.
> 3. dart:ffi directly calls aimux-ffi's 6 C functions, unifying the path with Swift/Kotlin, with zero extra toolchain.
> 4. For the same reason, Rinf (an event-signal system, still needs codegen) and membrane (stream-first, designed for hardware data streams, not a general-purpose scenario) are rejected. ffigen (Dart's official FFI generator) is an alternative — it can be introduced for automatic generation when aimux-ffi expands to dozens of functions.
>
> **Conditions for reconsideration**: If aimux-ffi's C ABI functions expand from 6 to dozens (e.g. exposing full-modality FFI), the cost of hand-written wrappers rises, and only then would UniFFI have leverage.

Once the upfront architectural investment is in place, adding a language is often just the workload of "adding a binding directory + CI build". Adding a language on the native binding path only requires writing a thin wrapper + connecting to Rust async; the C ABI path requires that `aimux-ffi`'s handle/callback already covers the needed capabilities.

---

## 6. Pre-review-period refactoring items

The following changes are good design even under a single-language state and will not be wasted; they are recommended to be pushed forward during the review phase:

| # | Refactoring | Corresponding section | Priority | Status |
|---|------|---------|--------|------|
| 1 | `aimux-ffi` streaming push callback abstraction (only for the C ABI path; ~~pull + push dual mode~~ → changed to push-only in v0.2.1) | §4.1 | Low (third tier, does not block Node) | ✅ Done |
| 2 | `Box<dyn LanguageModel/Provider>` → opaque handle + registry | §4.2 | High | ✅ Done |
 ✅ Done |
| 4 | Add `#[derive(Serialize, Deserialize)]` + version fields to all cross-boundary types; also add `ts-rs` derives to auto-generate `.d.ts` for the Node binding | §4.4 / §9-7 | High | ✅ Done |

> ~~Item 5 "abstract an HttpTransport trait"~~ — **deleted in v0.2**, see §4.5.

---

## 7. Implementation roadmap

| Phase | Content | Output | Status |
|------|------|------|------|
| **Phase 0 (review period)** | Complete §6 prerequisite refactorings 1–4 | A core that is better maintained even under a single language | ✅ Done |
| **Phase 1** | Choose **Node.js** (`napi-rs`) to do the first binding PoC | Flagship binding, validating the Rust core's competitiveness + streaming cross-boundary feasibility on AISDK's home turf | ✅ Done |
| **Phase 2** | Finalize `aimux-ffi` C ABI + JSON wire schema; CI matrix builds binaries for each platform | `.so`/`.dylib`/`.dll`/`.aar`/`.framework` | ✅ Done (header file + CI matrix + C/C++ examples) |
| **Phase 3** | Python (PyO3, native binding), Flutter (native binding), Swift, Kotlin (C ABI binding) | Covers second priority + mobile | ✅ Done |
| **Phase 4** | C/C++ binding + contract tests | Covers the second tier + consistency assurance | ✅ Done (C/C++ examples + shared JSON contract test framework) |

### 7.1 CI / Release

- Each binding is released independently: `npm` / `PyPI` / SPM / Maven / `pub.dev`
- Core Rust crates are published to `crates.io`
- GitHub Actions matrix builds artifacts for each platform

### 7.2 Contract tests

Use the same set of JSON test fixtures to drive all languages, ensuring consistent provider behavior.

---

## 8. Risks

| # | Risk | Description | Mitigation |
|---|------|------|------|
| 1 | **Cross-boundary consistency of streams** | Rust Stream is lazy pull, JS/Swift is push-based async. If the conversion layer has bugs, it can drop chunks, break backpressure, or leak memory | Dedicated design + stress testing; Node PoC validates first (napi-rs AsyncIterator) |
| 2 | **tokio runtime embedding** | The Rust core needs to start a tokio runtime inside each language's process, handling lifetimes, thread safety, and cooperation with each language's event loop | napi-rs / flutter_rust_bridge have ready-made patterns; Kotlin/Swift need manual management |
| 3 | **Binary size** | Mobile is sensitive to size; the Rust core + tokio + reqwest may be too large | Narrow tokio features (`full`→on-demand) + strip + LTO; ~~HttpTransport abstraction~~ (deleted in v0.2)|
| 4 | ~~**Node's value is unclear**~~ — **overturned in v0.2**: Node is the flagship battleground, with clear value (defeating the Vercel AI SDK). The real risk is changed to the next row |
| 5 | **The flagship binding must have DX no worse than native TS** | If the Node binding's API experience, streaming feel, or type completeness is worse than the Vercel AI SDK, the "defeat" goal cannot be achieved | Align with AISDK's TS API shape; compare AsyncIterator streaming feel item by item; auto-generate type definitions from Rust serde (ts-rs / specta)|

---

## 9. Open Questions

The following need to be clarified during review and determine the direction of the proposal:

1. ~~**Motivation ranking**: Is the primary goal of multi-language support mobile, Python, or Node?~~ — **Decided in v0.2**: Node first (flagship, benchmarked against AISDK) → Python second → mobile third.
2. ~~**Whether to do Node**~~ — **Decided in v0.2**: Yes, and at the highest priority. The goal is to defeat the Vercel AI SDK; Node is the main battleground.
3. **FFI tool selection**: Should `UniFFI` unify all languages, or should each language use its best tool (PyO3 / napi-rs / flutter_rust_bridge each independent)? The former is worry-free, the latter offers better experience.
   - **Decided in v0.2**: Dual path — Python/Node/Flutter take native bindings, Swift/Kotlin/C take the C ABI (§3.2). No pursuit of unification.
   - **v0.5 supplement: UniFFI not adopted**. aimux-ffi has only 6 C functions; hand-written Swift/Kotlin wrappers are ~150 lines each, so UniFFI's codegen leverage does not hold; moreover, UniFFI has its own FFI layer and cannot reuse aimux-ffi. See §5.5 for details.
4. ~~**HTTP stack strategy**: Must mobile support injecting native HTTP~~ — **Decided in v0.2**: No abstraction; accept Rust's built-in reqwest (§4.5 deleted).
5. ~~**Streaming transport mode**: Do both pull / push dual modes, or do one first?~~ — **Decided in v0.2.1**: Push-only (callback). C ABI synchronous functions cannot await an async stream, so pull inevitably blocks (freezing the main thread). This only affects aimux-ffi (Swift/Kotlin/C); native bindings like Node are unaffected (§4.1).
> **Note: aimux-tools and aimux-macros were deleted on 2026-07-31**; the tool-related content below is outdated and kept only for the historical record.

6. ~~**Timing of tool-definition rework**: Should the `#[tool]` macro be reworked into a data description during the review period, or when the first binding lands?~~ — **Decided in v0.2.1: a non-issue, no rework needed**. Checking the code shows that `FunctionTool`/`ToolCall`/`ToolResult` are already language-agnostic data descriptions with serde (§4.3). The `#[tool]` macro + `ToolExecutor` are Rust-side convenience tools; aimux does not do an agent loop or execute tools, and the foreign-language side executes them itself. Only `ToolChoice` lacked serde; adding a derive suffices.
7. **TS type generation for the Node binding**: Use `ts-rs` / `specta` to auto-generate `.d.ts` by deriving from Rust serde, or hand-write TS types?
   - **Decided in v0.2.1**: Auto-generate. Hand-written types will sooner or later drift from the Rust core, which is a fatal flaw for the flagship binding. Choose one of `ts-rs` or `specta`, and add derives to cross-boundary types during the §6 prerequisite-refactoring phase.

---

## Revision history

| Date | Version | Notes |
|------|------|------|
| 2026-07-26 | DRAFT v0.1 | Initial draft, based on current-state code analysis and multi-language proposal discussions |
| 2026-07-28 | v0.2 | Review revision: deleted the HttpTransport abstraction (§4.5/§6-5/§9-4); established the dual-path binding strategy (§3.2/§5.5, native binding vs C ABI) |
| 2026-07-28 | v0.2.1 | Strategic revision: Node promoted to flagship binding and highest priority (goal is to defeat the Vercel AI SDK, §1.2/§5.1/§7/§8/§9); first binding changed Python→Node; Node moved from the third tier to the first tier; added risk 5 (DX no worse than native TS); added open question 7 (auto-generation of TS types); streaming C ABI rejected pull, push-only (§4.1/§6-1/§9-5, reason: C ABI synchronous functions cannot await an async stream); TS type generation set to auto-generation (§9-7 decided); tool-definition corrected to "already ready, no rework needed" (§4.3/§6-3/§9-6, a non-issue, data description was done long ago); all open questions converged |
| 2026-07-28 | v0.3 | **Phase 0 + Phase 1 landed**: all §6 prerequisite refactorings completed (87 cross-boundary types given serde+ts-rs, 80 TS type files auto-generated; aimux-ffi crate created, handle registry + push streaming callback + 6 C ABI symbols exported); Phase 1 flagship Node.js binding completed (napi-rs v3 native binding, bypassing aimux-ffi and connecting directly to aimux-providers; generateText/streamText PoC + AsyncGenerator streaming + 6 tests passing); full workspace compile + test passing |
| 2026-07-29 | v0.4 | **Phase 2 + 3(Python) + 4 landed**: Phase 2 — aimux-ffi C header file (aimux-ffi.h) + GitHub Actions CI matrix (Rust test / Node binding / Python binding / ffi build / contract tests, across Linux/macOS/Windows); Phase 3 — Python binding completed (PyO3 native path, 6/6 tests passing); Phase 4 — C/C++ binding examples (RAII wrapper) + contract test framework (13 shared JSON wire-format fixtures, Rust 9/9 + Node 16/16 dual-end validation) |
| 2026-07-29 | v0.5 | **Phase 3 mobile bindings all landed**: Swift (Swift Package + module.modulemap, C ABI path, ARC-managed handle, AsyncSequence streaming); Kotlin (JNA wrapping the C ABI, Closeable for explicit release, Sequence streaming); Flutter (flutter_rust_bridge v2 native path, handle registry + channel streaming). All compile successfully. CI matrix adds Swift/Kotlin build jobs. bindings/README.md summarizes the 6-language bindings. All RFC-0001 phases completed |
| 2026-07-29 | v0.5.1 | **Flutter path correction**: After investigation, rejected flutter_rust_bridge (StreamSink not publicly exported, needs codegen; type-mapping advantage does not hold under a JSON boundary), and rejected UniFFI/Rinf/membrane (same reasoning). Flutter changed to hand-written dart:ffi calling aimux-ffi (C ABI path), unified with Swift/Kotlin. Pure Dart, no Rust crate, zero extra toolchain. §3.2/§5.1/§5.5 updated accordingly |
| 2026-07-31 | v0.6 | **Go upgraded to independent RFC for execution**: The three concerns in §5.3's original "Go third tier (CGo friction)" were all resolved after re-evaluation in [RFC-0011](0011-golang-bindings.md) (cgo overhead is negligible for network IO; aimux-ffi push-callback naturally maps to Go CSP/goroutine+channel; gRPC sidecar not adopted). Go upgraded to the next binding for immediate execution, single binary measured at 7.5MB |
