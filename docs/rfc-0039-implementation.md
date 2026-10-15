# RFC-0039 implementation

This document describes the implementation in this tree. The runtime manifest
contains 72 operations, 23 resource kinds and 93 referenced type entries. ABI v1
is `65536` (`1 << 16`) and ops is `1.0`. Capability registration is not evidence
that a live provider or every supported platform has been tested. See the
[conformance matrix](rfc-0039-conformance.md) and
[coverage ledger](rfc-0039-coverage-ledger.md) for evidence and its limits.

## Crate placement and dependencies

An arrow means “depends on”; the C ABI arrow is a native call boundary.

```text
C/C++, Go, Java, Swift, Dart ──C ABI──> aimux-ffi
Kotlin ──> same-version Java
aimux-ffi ────────────────┐
bindings/node (napi-rs) ──┼──> aimux-ops ──> aimux-providers
bindings/python (PyO3) ──┘         ├──────> aimux-provider-utils
                                 └──────> aimux-core
aimux-providers ──> aimux-core, aimux-provider-utils
aimux-provider-utils ──> aimux-core
tools/aimux-codegen ──> aimux-ops
```

`aimux-ops/Cargo.toml` also declares serde, serde_json, Tokio, futures and SHA-256.
Ops does not depend on FFI, napi or PyO3. Canonical business DTOs stay in core;
ops owns control DTOs, registration, handle lifting, error projection, framing
and Operation admission. Providers retain protocol conversion and factories;
core retains generation, retry, tracing, recording and replay algorithms.
Jev construction uses the existing public `JevConfig`/`JevProvider` constructors.

## Dispatch and handle ownership

```text
① adapter extracts owned inputs
   C: validate pointers/UTF-8, reset outputs, use checked abi::block_on
   Node: async napi runtime bridge; Python: release GIL around blocking work
② dispatch::{call,stream}(Ctx, HandleId, op, params)
   exact descriptor/mode → object controls → primary and secondary owner/kind checks
③ ops::<family>::handler(Ctx, Target, params)
   canonical Rust call → canonical result or business event
④ cached ErrorEnvelope or FrameStream(event*, end) → language projection
```

`Target::Global` is separate from `Target::Handle(Entry)`. Only ID 0 selects
global scope; it is not a HandleKind. IDs allocate monotonically from 1, never
reuse or wrap, and fail with HandleExhausted on exhaustion. JSON uses unsigned
decimal strings; C uses `uint64_t`. The deserializer accepts leading zeros,
while emitted IDs are normalized decimal strings.

`Owner::CProcess` is shared by C ABI consumers. Trusted adapters allocate
`Owner::Node(id)` per Env and `Owner::Python(id)` for interpreter ownership using
the process allocator; user inputs never supply owner tokens. The registry
clones strong references while locked and releases the lock before invoking or
disposing resources. A resolved model, files object, wrapper or composite can
outlive the source handles. Admitted ordinary calls can finish after drop;
dropping Abort does not signal it, while dropping Operation closes/cancels it.
`release_owner` removes entries before cleanup and signals retained Operations.

All secondary fields listed in `runtimeHandleFields`, including wildcard paths,
and the C abort argument use the same typed owner resolver. Foreign and unknown
handles have identical undisclosing errors. `handle.drop` is admitted before
lookup: 0, unknown, already-dropped and foreign IDs succeed; foreign drops do
nothing. Explicit close/drop does not flush a recorder. C drop may dispose a
resource synchronously; the finalizer helper queues it off the GC thread.

The 23 kinds are Provider, Registry, Language, Embedding, Image, Speech,
Transcription, Reranking, Video, Search, Decision, Files, Operation, Abort,
Fetch, WebSocket, Download, CredentialResolver, Middleware, TraceStore,
Recorder, SessionStore and ReplaySession.

## Complete operation table

`G` means only global handle 0. `Models` means Language, Embedding, Image,
Speech, Transcription, Reranking, Video, Search and Decision. `All` means the
23 resource kinds; handle.drop additionally admits 0/unknown IDs. Functions
below are in `aimux_ops::ops`; `dispatch` and `Operation` refer to
`aimux-ops/src/dispatch.rs` and `aimux-ops/src/operation.rs`. Modality functions
are emitted by the local `model_call!` macro. Mode is the actual descriptor
mode, not an inference from the operation name.

| Operation | Mode | Primary handle kinds | Implementing function | Scope |
|---|---|---|---|---|
| `abort.create` | call | G | `handles::abort_create` | Current |
| `abort.signal` | call | Abort | `handles::abort_signal` | Current |
| `auth.codexRefresh` | call | G | `misc::codex_refresh` | Current |
| `catalogue.fetch` | call | G | `misc::catalogue_fetch` | Current |
| `composite.moa` | call | G | `composite::moa` | Current |
| `composite.router` | call | G | `composite::router` | Current |
| `credential.create` | call | G | `handles::credential_create` | Future: UnsupportedFunctionality |
| `decision.capabilities` | call | Decision | `decision::decision_capabilities` | Current |
| `decision.decide` | call | Decision | `decision::decision_decide` | Current |
| `embedding.embed` | call | Embedding | `modalities::embed` | Current |
| `embedding.embedMany` | call | Embedding | `modalities::embed` | Current |
| `files.upload` | call | Files | `files::upload` | Current |
| `handle.drop` | call | All | `dispatch::Dispatcher::call_registered (drop special case)` | Current |
| `image.generate` | call | Image | `modalities::generate_image` | Current |
| `middleware.create` | call | G | `handles::middleware_create` | Current |
| `model.info` | call | Models | `registry::info` | Current |
| `model.resolve` | call | Registry | `registry::resolve` | Current |
| `model.wrap` | call | Models | `registry::wrap` | Current |
| `operation.cancel` | call | Operation | `operation::cancel` | Current |
| `operation.inputDone` | call | Operation | `operation::input_done` | Current |
| `operation.next` | call | Operation | `operation::next` | Current |
| `operation.push` | binaryPush | Operation | `dispatch::operation_push → Operation::try_push` | Current |
| `operation.status` | call | Operation | `operation::status` | Current |
| `output.toOpenAI` | call | Language | `misc::to_openai` | Current |
| `provider.create` | call | G | `provider::create` | Current |
| `provider.custom` | call | G | `provider::custom` | Current |
| `provider.default` | call | G | `provider::default_provider` | Current |
| `provider.files` | call | Provider | `files::provider_files` | Current |
| `provider.invoke` | call | Provider | `provider::invoke` | Current |
| `provider.model` | call | Provider | `provider::model` | Current |
| `provider.wrap` | call | Provider | `provider::wrap` | Current |
| `recorder.attach` | call | Recorder | `recording::attach` | Current |
| `recorder.create` | call | G | `recording::create` | Current |
| `recorder.export` | call | Recorder | `recording::export` | Current |
| `recorder.flush` | call | Recorder | `recording::flush` | Current |
| `recorder.stats` | call | Recorder | `recording::stats` | Current |
| `recorder.stop` | call | Recorder | `recording::stop` | Current |
| `registry.create` | call | G | `registry::create` | Current |
| `registry.provider` | call | Registry | `registry::provider` | Current |
| `replay.capabilities` | call | ReplaySession | `replay::capabilities` | Future: UnsupportedFunctionality |
| `replay.create` | call | G | `replay::create` | Current |
| `replay.dryRun` | call | ReplaySession | `replay::dry_run` | Current |
| `replay.inspect` | call | ReplaySession | `replay::inspect` | Current |
| `replay.live` | call | ReplaySession | `replay::live` | Current |
| `reranking.rerank` | call | Reranking | `modalities::rerank` | Current |
| `runtime.configure` | call | G | `runtime::configure` | Current |
| `runtime.describe` | call | G | `runtime::describe` | Current |
| `search.search` | call | Search | `modalities::search` | Current |
| `sessionStore.attach` | call | SessionStore | `session::attach` | Current |
| `sessionStore.calls` | call | SessionStore | `session::calls` | Current |
| `sessionStore.create` | call | G | `session::create` | Current |
| `sessionStore.list` | call | SessionStore | `session::list` | Current |
| `speech.generate` | call | Speech | `modalities::generate_speech` | Current |
| `text.consume` | call | Language | `text::consume` | Current |
| `text.generate` | call | Language | `text::generate` | Current |
| `text.generateObject` | call | Language | `text::generate_object` | Current |
| `text.generateOpenAI` | call | Language | `text::generate_openai` | Current |
| `text.stream` | stream | Language | `text::stream` | Current |
| `text.streamOpenAI` | stream | Language | `text::stream_openai` | Current |
| `toolRepair.apply` | call | G | `misc::repair_apply` | Current |
| `toolRepair.applyResult` | call | G | `misc::repair_apply_result` | Current |
| `toolRepair.context` | call | G | `misc::repair_context` | Current |
| `trace.aggregate` | call | TraceStore | `trace::aggregate` | Current |
| `trace.clear` | call | TraceStore | `trace::clear` | Current |
| `trace.create` | call | G | `trace::create` | Current |
| `trace.export` | call | TraceStore | `trace::export` | Current |
| `trace.sessionChain` | call | TraceStore | `trace::session_chain` | Current |
| `trace.sessionTrajectory` | call | TraceStore | `trace::session_trajectory` | Current |
| `transcription.generate` | call | Transcription | `modalities::transcribe` | Current |
| `transcription.start` | call | Transcription | `operation::start` | Current |
| `transport.create` | call | G | `handles::transport_create` | Current |
| `video.generate` | call | Video | `modalities::generate_video` | Current |

`provider.create` accepts strict settings (`apiKey`, `baseUrl`, `headers`,
`organization`, `project`, `params`, `fetch`, `webSocket`, `credentialResolver`,
`endpoint`, `probabilitySource`); accepted keys do not imply support by every
factory. WebSocket/credential slots are resolved then explicitly rejected when
unsupported. `provider.default` takes a package and default settings. Supply an
explicit model method for vendor packages without default-method metadata.
`provider.invoke` only accepts `listModels`. `model.resolve` takes an explicit
`{modelRef:{namespace,key,method,modelId}}`; it never falls back to a gateway.
`model.wrap` supports existing Language/Image wrappers, not invented wrappers
for every model kind. Trace middleware uses the canonical `TraceLayer` hook.

Runtime logging/proxy configuration shares native initialization authorities;
identical settings are idempotent and changed/frozen settings fail InvalidState.
An empty inspection request does not freeze proxy state. Logging and proxy
updates are separate settings, not a transaction. Summaries omit secret values.
Recording requires explicit `capturePolicy: "full"`; there is no audit-only
recording format. Trace stores support off/shared/strict audit policy; trace
middleware inherits it unless disabled/shared/strict is supplied explicitly.

## C ABI: fourteen exports

The generated authority is [aimux.h](../aimux-ffi/include/aimux.h); implementations
are in `aimux-ffi/src/abi.rs`. All fallible functions return NULL on success or
an owned `aimux_error_t*` on failure. Outputs are reset before validation.

| Export | Behavior and ownership |
|---|---|
| `aimux_abi_version()` | Returns packed ABI major/minor. |
| `aimux_manifest_json(char **out_json)` | Allocates manifest JSON. |
| `aimux_call(uint64_t, const char*, const char*, uint64_t, char**)` | Blocking call; last handle is optional Abort (0 = none). |
| `aimux_stream(uint64_t, const char*, const char*, uint64_t, aimux_event_cb, void*)` | Blocking stream; callback receives borrowed frame JSON and context. |
| `aimux_operation_push(uint64_t, const uint8_t*, size_t)` | Validates and copies/consumes input before return; NULL is valid only with length 0. |
| `aimux_operation_next_part(uint64_t, int64_t, char**, int32_t*)` | Nonnegative timeout; PART=1 allocates JSON; ENDED=2/TIMEOUT=3 allocate none; errors clear both outputs. |
| `aimux_abort(uint64_t)` | Callback-safe signal without block_on; invalid handles are ignored. |
| `aimux_drop(uint64_t)` | Idempotent C-owner removal; 0/unknown are harmless. |
| `aimux_handle_finalizer(void*)` | Pointer value encodes the handle; never dereferences it; queues drop using the initialized runtime. |
| `aimux_free_string(char*)` | Frees an owned returned string; NULL-safe. |
| `aimux_error_free(aimux_error_t*)` | Frees an owned error; NULL-safe. |
| `aimux_error_code(const aimux_error_t*)` | Reads saved code; NULL gives 0. |
| `aimux_error_message(const aimux_error_t*)` | Allocates saved message; embedded NUL is replaced in this display accessor. |
| `aimux_error_json(const aimux_error_t*)` | Allocates saved full envelope; JSON preserves escaped NUL and cached hint. |

The pointer-encoded finalizer token is limited to uintptr_t width. Dart uses
its own allocated uint64 token and native queue trampoline on 32-bit targets.

Inputs are borrowed NUL-terminated UTF-8. Do not pass invalid owned pointers,
double-free, or omit terminators: these are caller violations. Free every owned
JSON/getter string with aimux_free_string and every owned error with
aimux_error_free; never free callback JSON. Copy callback bytes before return
and retain context until aimux_stream returns. Blocking reentry, including from
a Tokio caller, returns ReentrantCall. Abort is safe inside a callback. Host
exceptions must be latched by adapters and raised after native return; a
callback that never returns cannot be forcibly interrupted.

## Error envelope and retries

```json
{"kind":"protocol","code":300,"message":"invalid op: missing.op","error":{"name":"InvalidOp","message":"missing.op"}}
```

The four domains are aimux, recording, binding and protocol. `error` preserves
the canonical payload, nested causes, Retry attempt history, open fields and
missing/null distinctions. A payload name discriminates variants sharing a
code. AI_APICallError uses code 11, not 1; no HTTP status is invented for local
configuration/parse failures. Unknown payloads remain available to consumers.

`OpsError::Projected(Box<ProjectedError>)` caches the envelope at the first
dispatch crossing with `Ctx.now`. C getters, stream terminals and Operation
status reuse it. Only a direct ApiCall error derives `retryAfterMs`; a Retry
aggregate retains its attempts without copying a hint to the outer object.
Absent hint differs from zero. The sole core parser checks case-insensitive
retry-after-ms, retry-after seconds and HTTP dates with checked finite,
nonnegative conversion and truncation toward zero. Parsed invalid preferred
numeric values stop fallback; malformed text permits the next header. The
core legacy delegate rejects values above i64::MAX. Hints never authorize a
retry; generation policy stays in core and bindings add no retry loop.

| Protocol payload | Code |
|---|---:|
| InvalidOp | 300 |
| InvalidParams | 301 |
| UnsupportedOpsVersion | 302 |
| ConcurrentConsumer | 303 |
| InvalidState | 304 |
| ResourceLimit | 305 |
| HandleExhausted | 306 |

AiMux uses 1–20 with 4 reserved; Recording uses 100–105 and Binding 200–206.
Code 3 aliases InvalidResponseData, ToolCallNotFoundForApproval and
ToolChoiceViolation; code 6 aliases InvalidPrompt and MissingToolResults.

## Streams, Operation and limits

Preflight errors emit no frames. Accepted streams emit zero-based decimal
`seq` values, `event` frames and exactly one `end` with success/error/cancelled.
Business finish/recoverable error parts remain events. After delivering end,
the C stream returns NULL rather than returning a second error. Wrapper failure
latches once, aborts native work and is reported after callbacks finish.

`transcription.start` checks doStream before acceptance and returns a decimal
Operation ID. Connection failure after acceptance becomes durable terminal
state. Binary push admission is ordered against idempotent inputDone; acceptance
does not mean provider processing. Only one poll reader is admitted; another
gets ConcurrentConsumer. Buffered parts precede the terminal error, delivered
once; later polls return Ended and status retains the outcome. Poll Timeout
does not terminate the operation. Cancel reports first/alreadyRequested/
alreadyTerminal and cannot overwrite completed success.

| Bound | Implemented value |
|---|---|
| Realtime input | 64 admitted items and 8 MiB |
| Output | 256 admitted items and 8 MiB |
| Single binary chunk / serialized stream frame | 1 MiB, including frame overhead |
| Node/Python bridge | 64 frames and 8 MiB; terminal separate |
| Realtime cleanup | Five seconds total; cleanup failure separate from outcome |

QueueLimits/OperationLimits can be lowered, never made zero or increased beyond
defaults. Checked wire-size counters and binary admission run before copying;
terminal storage does not need data capacity. Full bounded queues stop their
producer; consumer close can abandon queued data and cancels the operation.
Synchronous binary admission reports ResourceLimit on a full input queue.
One-shot JSON requests have no byte cap here. These are admitted-payload bounds,
not a bound on all engine allocations (G-g), and remain proposals for freeze.
Recorder I/O uses spawn_blocking, and registry locks do not cross await/join.

## Manifest and generation chain

```text
① family descriptors + register → dispatch::registrations (sorted by name)
② manifest::manifest → descriptors + kind/code/type catalogue + control schemas
③ manifest_digest → compact JSON, recursively sorted object keys, SHA-256
④ aimux-codegen → C header + seven language constant files + schema/ops/*.json
                    + tools/aimux-codegen/generated-files.json
⑤ language loaders/bridges + handwritten facades consume shared dispatch/ABI
```

The digest excludes its own digest field; array order is preserved. Each type
entry has an ID and Rust path; ops-owned controls/wrappers carry schemas and
embedded business fields have x-aimux-type references. Schema IDs use JSON Schema
2020-12 with local resolvable refs. Strict controls reject unknown keys while
providerOptions/metadata/explicit JSON stay open. `--check` builds temporary
outputs and compares complete managed file sets and bytes without rewriting.
The generator does not generate business DTOs, language facades, the operation
registration code, this document or the complete eight-language vector corpus.
Those stronger RFC generation goals remain G-d/G-i rather than implicit claims.
Separately, `bindings/flutter/tool/generate_ffi.py` consumes the C header to generate its
FFI declarations and packaged header; Java's JNA declarations are handwritten
and tested against the primitive signatures, not emitted by aimux-codegen.

The generated file set is checked against the current manifest by the commands below;
this documentation audit does not claim a successful generator run.
Python pins its generated digest at import; Node checks ABI/ops versions and
exposes the runtime digest without comparing the generated digest (G-h).
Other adapters have their implemented ABI/ops checks. No adapter can claim per-consumed-business-type hash validation
until that metadata exists. Schema presence does not validate live handles,
owner isolation, capabilities or state transitions.

## Verification gates

These commands are reproducible gates, not a claim that all ran for this
documentation change or that their existence establishes release readiness.

| Gate | Existing command / evidence |
|---|---|
| Dispatch, ownership, stream, error and manifest | `cargo test -p aimux-ops` (including ops_matrix, handle_lifecycle, stream_terminal, error_envelope_golden, manifest_contract) |
| Native ABI and deletion | `cargo build -p aimux-ffi` then `cargo test -p aimux-ffi`; exports_smoke_test compares actual cdylib symbols |
| Generated outputs and schemas | `cargo test -p aimux-codegen`; `cargo run -p aimux-codegen -- --check` |
| C/C++ | `bash scripts/test-c-abi.sh --run`; separate `--asan` mode; accepted live-stream branch needs configured provider parameters |
| Go | `bindings/go/test-abi.sh` and `bindings/go/test-native.sh` (fixture and real-library results separate) |
| Java / Kotlin | Gradle tests for each binding; cached-classpath local runners are separate fallback evidence |
| Swift | Binding SwiftPM tests or its standalone runners; macOS execution does not establish simulator linkage |
| Dart | Binding analyzer, native runtime tests and bridge tests; Android/iOS packaging and symbol gates remain separate |
| Node / Python | Native and facade/bridge suites; socket-dependent provider cases and installed packages require their own results |
| Workspace | `cargo fmt --all -- --check`; `cargo clippy --workspace --all-targets -- -D warnings`; workspace tests and docs |

The manifest matrix uses probe handlers to test routing/admission across all
72 names and 23 kinds plus global/invalid states; it does not execute every
business operation. Family suites test real dispatch with existing engine
traits. stream_terminal runs all eleven native shared scenarios; its external
driver hook remains ignored unless explicitly enabled. Cross-language complete
vectors, performance/size gates (RFC-0038) and per-platform build/test/package/
install evidence are additional obligations. Release order/header packaging
are configured in `.github/workflows/release.yml` but do not establish G-e closure.

## Registered differences from RFC-0039

These register implementation differences against the RFC examples and original
inventory; the canonical business error union remains owned by RFC-0042.

- **D1 — Error envelope (§6.1 example):** The implementation follows RFC-0042: the outer object is `kind`, `code`, `message`, `error` and optional `retryAfterMs`; `error` is AI SDK error JSON with `name` and, where defined, `isRetryable`, with no top-level `retryable` or `type`, because RFC-0042 owns the canonical error union. The RFC-0039 §6.1 example still shows `type`; it is to be corrected to `name` when this branch is merged up with master, since the RFC file is not on this branch's base.
- **D2 — Decision (§3.1–§3.2):** `Decision`, `decision.decide` and `decision.capabilities` extend the original eight-model, 70-operation table to nine model kinds and 72 operations because the source baseline already exposes Decision and its Jev constructors.
- **D3 — Error registrations (§6.1):** Protocol errors use fixed codes 300..306, AiMux codes extend through 20, code 4 stays reserved, code 3 aliases three variants and code 6 aliases two variants, because existing numeric meanings must survive the ABI replacement.
- **D4 — Finalizer ABI (§4.1):** `aimux_handle_finalizer(void *token)` is the fourteenth export because GC cleanup needs a nonblocking pointer-signature entry rather than a cast of `aimux_drop(uint64_t)`.
- **D5 — Resource budgets (§5.3):** The implemented 64/256-item, 8 MiB queue, 1 MiB event/chunk and five-second cleanup values remain proposals for stable freeze because RFC-0038 measurement and platform evidence are still required.
- **D6 — Operation polling (§3.2):** `operation.next` is a JSON `call` returning a tagged part/ended/timeout result, alongside native polling, because both paths must use the same Operation service without a separate poll-mode dispatcher.
- **D7 — Binary input (§3.2):** `operation.push` is BinaryPush-only and the synchronous native admission path returns ResourceLimit when full, because it must reserve input capacity before copying without waiting on an async queue; Rust `Operation::push` retains asynchronous backpressure.
- **D8 — Foreign handles (§3.3):** Foreign-owner lookup returns InvalidHandle with no `actual` kind and the same message as an unknown ID, while foreign `handle.drop` succeeds without effect, because the owner boundary must not disclose resources.
- **D9 — Retry-hint parser (§6.2):** The single parser lives in `aimux_core::error::retry_after_ms_from_headers` and is re-exported by ops because core retry policy cannot depend on an adapter-layer crate; a parsed invalid preferred number blocks fallback, while malformed text permits it.
- **D10 — Current capability scope (§11.4):** Stdio, host fetch/token/model/telemetry callbacks, raw one-shot binary upload, native passthrough and skills/Batch/Evaluation operations are not exposed because they require separate capabilities beyond the existing engine.

## Registered gaps

- **G-a — Process-wide capture:** Recorder, session-store attachment and inference state remain process-wide because owner-local capture requires a core runtime-context change.
- **G-b — Recording compatibility:** Writers use schema 2 and readers accept schemas 1 and 2, as implemented in `aimux-core/src/recording.rs`, because migration to schema 3 and removal of old-schema reading are planned as the final step of this series under RFC-0041.
- **G-c — Future capabilities:** `credential.create` and `replay.capabilities` have Future scope and return UnsupportedFunctionality because no credential-construction or Fetch/WebSocket/Download replay engine is implemented; replay retains existing Language/Decision model replay.
- **G-d — Business schemas and DTO generation:** Business types have stable Rust type references but no complete generated schemas or language DTO chain because RFC-0042 owns that work; `scripts/gen_ts_types.py` remains for its existing outputs.
- **G-e — Release installation evidence:** Release publishing orders ops before FFI and packages the header, but installation and execution of every platform asset are not established here because build and packaging definitions alone do not establish release closure.
- **G-f — Provider metadata:** Complete package provenance, authoritative default methods for every package, general named vendor-method dispatch and registry/string-model lifting in text requests are absent because entries retain canonical Arcs and current factories/control DTOs do not supply all that metadata or lifting; owner-scoped reference metadata exists for registry-resolved models, and `provider.invoke` accepts only `listModels`.
- **G-g — End-to-end memory bound:** Queue and wire limits do not bound every provider allocation or the existing unbounded core text pump because this layer controls admitted payloads rather than rewriting core streaming internals.
- **G-h — Generated-product consistency:** Per-business-type revision/hash checks are unavailable and Node currently checks ABI/ops versions without comparing its generated digest because consumed-type metadata and uniform loader consistency enforcement are incomplete.
- **G-i — Cross-language and platform conformance:** Exhaustive eight-language business-vector execution, same-source release evidence, installed-wheel checks and all platform sanitizer/link/install results are not established here because narrower local and fixture suites do not prove the full matrix; Dart Windows streaming is explicitly unsupported.
- **G-k — Offline mock-model migration:** The old MockReplayModel constructor has no exposed replacement for offline text generation because replay.create returns ReplaySession, text operations admit only Language, and replay.live requires a separate caller-supplied target.
