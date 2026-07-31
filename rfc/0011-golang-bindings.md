# RFC-0011: Golang Bindings

> **Status**: v0.2 (typed API + 8-modality multimodal + DeepSeek factory, aligned with Node flagship coverage)
> **Date**: 2026-07-31
> **Related**: [RFC-0001](0001-multilang-bindings.md) multilang bindings, [aimux-ffi](../aimux-ffi/aimux-ffi.h)

---

## 1. Background and Motivation

RFC-0001 §5.3 listed Go in the "third tier (worth doing but each with friction)", with the reasoning that "CGo has cross-boundary overhead; Go has no async, requiring goroutine + channel to convert to a Rust Stream; under SDK form, gRPC sidecar is too heavy". After re-evaluation, this RFC promotes Go to the next binding for **immediate execution**.

### 1.1 Why Do Go Now

aimux currently covers 6 language bindings (Node/Python/Swift/Kotlin/Flutter/C++), **only Go is missing**. Yet Go is the native language of cloud-native backends, and the volume of scenarios for "calling LLMs in microservices" is large.

In the second half of 2025, Go's AI SDK ecosystem experienced a concentrated burst, proving the demand is real and growing:

| Framework | Source | Time |
|------|------|------|
| **Eino** | ByteDance CloudWeGo | 2025 |
| **ADK-Go** | Google | 2025-11 reached 1.0 |
| **Genkit-Go** | Firebase | 2025-11 |
| **LangChainGo** | Community | Continuously updated |
| **GoAI** | Community | 2026, "22+ LLM providers, 2 dependencies" |

Among them, GoAI's selling points ("lightweight, unified provider interface, doesn't lock in architecture") are almost identical to aimux's—this shows that aimux's positioning has a real gap in the Go ecosystem. Key signal: [Zep's research](https://blog.getzep.com/agentic-development-in-go/) points out that in Go "most teams skip frameworks"—this is exactly the positioning gap that aimux's "only unify providers, don't do orchestration" addresses.

### 1.2 Re-evaluation of the Three "Friction Points" in RFC-0001 §5.3

| Original concern | Re-evaluation | Conclusion |
|--------|---------|------|
| "CGo has cross-boundary overhead" | cgo call overhead ~200ns/call, negligible for network-IO LLM calls | ❌ Not a problem |
| "Go has no async, needs goroutine + channel to convert to Rust Stream" | aimux-ffi's `aimux_stream_text` is itself a **push-callback synchronous blocking** model (RFC-0001 §4.1), specifically designed for "languages without Rust-compatible async". Go's CSP model is its most natural mapping: goroutine calls blocking FFI, callback writes to channel, consumer does `for part := range ch` | ❌ Actually the most suitable |
| "gRPC sidecar is too heavy" | This solution does not adopt gRPC sidecar; uses cgo + static linking, single binary | ❌ Not adopted, problem disappears |

All three friction points are resolved.

---

## 2. Technical Path: cgo + Static Linking aimux-ffi

### 2.1 Path Selection

Go does not have native Rust binding tooling at the level of PyO3/napi-rs, so it can only go the C ABI path (consistent with Swift/Kotlin/Flutter/C++, see RFC-0001 §3.2):

```
Go code ──cgo──→ aimux-ffi (libaimux_ffi.a statically linked) ──→ aimux-providers
```

| Option | Assessment | Adopted |
|------|------|:----:|
| **cgo + static link `.a`** | Single binary, zero extra processes, same path as C/C++ bindings | ✅ |
| cgo + dynamic link `.so` | Breaks Go's "single binary" convention, requires distributing `.so` with the package | ❌ |
| gRPC sidecar | One extra process, complex deployment, one extra hop of latency | ❌ |
| Pure Go rewrite of providers | Violates the "reuse Rust core" strategy, abandons 172 providers | ❌ |

### 2.2 Single-binary Feasibility (empirically tested)

aimux-ffi's [Cargo.toml](../aimux-ffi/Cargo.toml#L11) already declares `crate-type = ["cdylib", "staticlib", "rlib"]`, i.e., the `libaimux_ffi.a` static library is already produced. After cgo links the `.a`, the Rust core is statically compiled into the Go binary.

**Empirical data** (after release profile optimization, see [Cargo.toml](../Cargo.toml) `[profile.release]`):

| Artifact | Size |
|------|------|
| `libaimux_ffi.a` (static library, includes all intermediate symbols, not deduplicated) | 82 MB |
| `libaimux_ffi.so` (dynamic library, stripped) | 3.5 MB |
| **Statically linked into executable (stripped, real increment)** | **7.5 MB** |

The linker performs symbol resolution + dead code elimination on the `.a`; the real increment is only 7.5MB (not the `.a`'s 82MB). Estimated final Go binary ≈ 12-13MB (Rust core 7.5MB + Go itself ~5MB), within the normal range for the Go ecosystem.

`ldd` verification: the statically linked executable **only depends on base libraries shipped with the OS**:

```
linux-vdso.so.1
libm.so.6          ← bundled with glibc
libc.so.6          ← bundled with glibc
libgcc_s.so.1      ← gcc runtime, bundled with system
/ld-linux-x86-64.so.2
```

**No need to distribute any `.so/.dll/.dylib` with the package**, single binary holds.

### 2.3 Cross-platform

| Platform | Static linking | Result |
|------|---------|------|
| **Linux** | Link `.a`, with `x86_64-unknown-linux-musl` + musl-gcc | Fully static ELF, `ldd` shows `not a dynamic executable` |
| **macOS** | Link `.a` | One binary + depends on system-bundled libSystem, effectively single file |
| **Windows** | Link `.a` | One .exe + depends on system-bundled ucrt, effectively single file |

The only build-time requirement: a C toolchain (cgo needs this anyway) + rustls's ring (includes assembly). This is a build-environment requirement and does not affect the distributed artifact.

---

## 3. Streaming Mapping: push callback → channel

RFC-0001 §4.1 already finalized that the C ABI path only does **push (callback)**, not pull. Go's CSP model is the most natural consumer of this design:

```
[aimux-ffi]           [cgo]                                           [Go user code]
aimux_stream_text  →  C callback on_part(json)  →  channel<- part  →  for part := range ch
   (blocking)         (sync, in cgo thread)        (goroutine)        (consumer)
```

- **Producer side (push)**: cgo calls the blocking `C.aimux_stream_text` in a separate goroutine; the C callback writes each StreamPart JSON to the Go channel
- **Consumer side (pull)**: Go users consume with `for part := range ch`, conforming to Go conventions
- A buffered channel decouples the two in between, avoiding backpressure

Compared to the streaming wrappers of other C ABI bindings:

| Binding | Transport side | Consumer side |
|------|--------|--------|
| Kotlin | JNA callback → `LinkedBlockingQueue` | `Sequence` |
| Swift | C callback → `AsyncStream` continuation | `AsyncSequence` |
| Flutter | dart:ffi callback → `StreamController` | `Stream` |
| **Go** | **cgo callback → channel** | **`for range`** |

Go's implementation is instead the most concise—channels are a first-class language citizen, requiring no `LinkedBlockingQueue` or `AsyncStream` wrapper layer.

---

## 4. API Shape

Align with other C ABI bindings (Kotlin/Swift/Flutter), keeping the API shape consistent:

```go
package aimux

// OpenAI model, statically links the Rust core
model := aimux.OpenAI("sk-...", "gpt-4o")
defer model.Close()

// Non-streaming generation
result := model.GenerateText(`"What is Rust?"`)
// result is a JSON string: {"text":"...","usage":{...}}

// Custom base URL (Ollama / OpenRouter / local proxy)
model := aimux.OpenAIWithBase("sk-...", "gpt-4o", "http://localhost:11434")

// Streaming generation
stream := model.StreamText(`"Write a haiku"`)
for part := range stream {
    // part is a StreamPart JSON: {"TextDelta":{"delta":"..."}}
    fmt.Println(part)
}
// stream auto-closes on end; on error, stream.Err() returns it
```

- `Model` implements `io.Closer` (mirrors Kotlin's `Closeable` / Swift's `deinit`)
- `GenerateText` / `StreamText` inputs/outputs are all JSON strings (wire format is consistent with other bindings, see [aimux-ffi.h](../aimux-ffi/aimux-ffi.h#L24))
- Streaming returns a `chan string` (or a custom `Stream` type that wraps error propagation)

---

## 5. Directory Structure

Align with the organization of other C ABI bindings:

```
bindings/go/
├── go.mod                    # module github.com/aimux/aimux-go
├── aimux.go                  # cgo declarations + Model type + Generate/Stream
├── stream.go                 # streaming channel wrapper
├── types.go                  # JSON wire types (optional, see Kotlin Types.kt)
├── aimux_test.go             # unit tests
├── stream_test.go
├── examples/
│   └── generate/main.go      # minimal example
└── README.md                 # build/usage notes (or reuse bindings/README.md)
```

Estimated code volume: wrapper ~250 lines (same order of magnitude as Kotlin's [Model.kt](../bindings/kotlin/src/main/kotlin/aimux/Model.kt) 201 lines, with slightly more cgo declarations).

---

## 6. cgo Implementation Points

### 6.1 Static Linking `.a`

cgo links `libaimux_ffi.a` via `CFLAGS` / `LDFLAGS`:

```go
// #cgo CFLAGS: -I${SRCDIR}/../../aimux-ffi
// #cgo LDFLAGS: -L${SRCDIR}/../../target/release -laimux_ffi -lpthread -ldl -lm
// #include "aimux-ffi.h"
import "C"
```

Build prerequisite: `cargo build -p aimux-ffi --release` has already produced `target/release/libaimux_ffi.a`.

### 6.2 Memory Ownership

Follow the contract of [aimux-ffi.h](../aimux-ffi/aimux-ffi.h#L10):

- The `char*` returned by `aimux_generate_text` **is freed by the caller**—the Go side calls `C.aimux_free_string`, using `defer` to guarantee release
- The `const char*` received by the `aimux_stream_text` callback **is only valid during the callback**—inside the callback you must synchronously copy it out with `C.GoString`, then write to the channel

### 6.3 Concurrency

- [aimux-ffi.h:20](../aimux-ffi/aimux-ffi.h#L20) is explicit: all FFI functions are synchronous blocking, callbacks execute on the same thread, **re-entering FFI inside the callback is not allowed**
- Go side: `GenerateText` / `StreamText` call the blocking FFI in a separate goroutine, not blocking the caller's goroutine (`StreamText` returns the channel immediately)
- The tokio runtime is managed internally by aimux-ffi; Go is unaware of it

---

## 7. Release Strategy

### 7.1 Single-binary Distribution

The biggest advantage of the Go binding: **directly `go build` produces a single binary**, users need not install the Rust toolchain.

- Library form: released as a Go module (`go.mod`), but requires a pre-compiled `libaimux_ffi.a` (needed at cgo compile time)
- Application form: users `go build` directly produces a single binary containing the Rust core

### 7.2 Cross-platform Build

GitHub Actions matrix (aligned with existing [CI](../.github)):

| Platform | target | Linking method |
|------|--------|---------|
| Linux x86_64 | `x86_64-unknown-linux-musl` | Fully static |
| Linux aarch64 | `aarch64-unknown-linux-musl` | Fully static |
| macOS x86_64 | `x86_64-apple-darwin` | Dynamically links libSystem |
| macOS aarch64 | `aarch64-apple-darwin` | Dynamically links libSystem |
| Windows x86_64 | `x86_64-pc-windows-msvc` | Dynamically links ucrt |

### 7.3 Contract Tests

Reuse the shared JSON fixture [contract-tests/fixtures/wire-format.json](../contract-tests/fixtures/wire-format.json) to ensure Go's wire format is consistent with the other 6 languages.

---

## 8. Risks

| # | Risk | Description | Mitigation |
|---|------|------|------|
| 1 | **cgo requires `.a` at compile time** | If the user has no local `libaimux_ffi.a` when running `go get`, compilation fails | CI pre-compiles `.a` for each platform and distributes it with the module; or provide a `go generate` script to auto-run `cargo build` |
| 2 | **cgo call overhead** | ~200ns/call | Negligible for network-IO LLM calls |
| 3 | **Callback thread safety** | cgo callback executes on a C thread; writing to a Go channel requires `runtime.cgocall` | The Go runtime guarantees channel safety across goroutines; the callback only does `C.GoString` + channel send |
| 4 | **Binary size** | Rust core 7.5MB | Already optimized with release profile (LTO + panic=abort + strip + opt-level=z), 12MB → 7.5MB; within normal range for the Go ecosystem |

---

## 9. Implementation Roadmap

| Phase | Content | Status |
|------|------|------|
| **Phase 0** | release profile optimization (LTO/panic=abort/strip/opt-level=z) | ✅ Done (commit c167273a, 12MB→7.5MB) |
| **Phase 1** | This RFC passes review | ✅ Done |
| **Phase 2** | `bindings/go/` PoC: cgo declarations + Model + GenerateText + StreamText | ✅ Done |
| **Phase 3** | Contract tests (shared wire-format fixture) | ✅ Done |
| **Phase 4** | CI matrix (cross-platform `.a` build + Go test) | ⏳ To do |
| **Phase 5** | Docs sync (bindings/README.md / main README.md / docs/API.md) | ✅ Done |

---

## Revision History

| Date | Version | Description |
|------|------|------|
| 2026-07-31 | DRAFT v0.1 | Initial draft: Go binding design, promoting RFC-0001 §5.3's third tier to immediate execution; cgo + static linking path; push callback → channel streaming mapping; single-binary empirical verification (7.5MB) |
| 2026-07-31 | v0.1 | **PoC landed**: `bindings/go/` complete implementation (aimux.go cgo declarations + Model + Generate/Stream; types.go typed JSON types; 19 tests all passing: 7 unit + 6 E2E + 6 contract subtests). Single binary empirically 8.7MB (after strip), statically linked `libaimux_ffi.a`, zero extra file dependencies |
| 2026-07-31 | v0.1.1 | **Review fixes**: RWMutex handle lifecycle, stream fallback closeParts, error envelope JSON parsing, constructors return error, ParseStreamPart validation, numeric types aligned with Rust (uint32/uint64/float64), mock server io.ReadAll, contract default→Fatal |
| 2026-07-31 | v0.2 | **Aligned with Node flagship coverage**: typed text API (Generate/Stream accept string\|[]ModelMessage + typed options, return *GenerateTextResult / typed StreamPart channel); 8-modality multimodal (Embedding/Speech/Image/Transcription/Files/Reranking/Video/Search) + factory functions + typed result types (aligned with ts-rs wire format); DeepSeek factory; 55 tests all passing (including -race) |
