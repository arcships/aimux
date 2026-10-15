# aimux-ops

Transport-independent operation dispatch for aimux's C and language adapters.
The crate owns operation registration, control decoding, runtime handle lifting,
resource ownership, error projection and stream framing. Generation, retries,
provider protocols, trace recording and replay algorithms remain in the existing
Rust engine crates.

Dependencies flow from adapters into `aimux-ops`, then into `aimux-core`,
`aimux-providers` and `aimux-provider-utils`. Canonical business DTOs contain no
binding handle IDs or adapter types. See the [operation table](../docs/api/ops.md)
for registered names and the runtime manifest for authoritative modes, handle
kinds, type paths, cancellation, capabilities and byte policies.

## Dispatch and ownership

`dispatch::call` and `dispatch::stream` use the shared registry and operation
table. `Dispatcher::new` supports an isolated registry and registration table.
Handlers receive `Target::Global` or `Target::Handle(Entry)`; handle `0` denotes
global scope. Global scope is never a resource kind.

Handles are decimal strings on JSON boundaries and never reused. Node/Python
adapters allocate owner tokens in trusted code with `Owner::new_node()` and
`Owner::new_python()`; C callers share `Owner::CProcess`. Primary and secondary
handles are checked against the same owner and declared kinds. Foreign handles
produce the same error as unknown IDs, without disclosing their kinds.
`handle.drop` succeeds for zero, unknown, dropped and foreign handles. Resources
are removed before cancellation and cleanup; cloned dependencies survive parent
handle removal. Owner release removes all owned entries before disposal.

Errors cross dispatch once, capturing one clock snapshot and a cached
`ErrorEnvelope`. Adapters and operation status reuse that projection. Retry hints
use `aimux_core::error::retry_after_ms_from_headers`; the ops re-export adds no
parser or retry policy. Core owns generation retries, while uploads and provider
control calls do not add retries.

## Runtime configuration and tracing

`runtime.configure` accepts strict `logging` and `proxy` objects; `{}` inspects
state without freezing proxy configuration. Logger initialization uses the same
process guard as native and environment initialization and honors existing
host/environment precedence. A different requested level after initialization
fails with InvalidState; an identical programmatic request is idempotent.
The summary reports the requested level, configured flags and freeze flags,
without returning proxy URLs, credentials or environment directives.

The startup proxy freezes when the default transport is first acquired by HTTP,
WebSocket or guarded download code. Repeating its active configuration is
idempotent. The explicit native `set_default_transport` replacement API retains
its existing behavior; replacing a transport does not unfreeze startup settings.
Logging and proxy updates are separate process settings, not a transactional pair.

`trace.create` retains `auditMode: off|shared|strict`. `middleware.create` with
`kind: trace` retains its TraceStore and uses canonical `TraceLayer` for language
models reached through `model.wrap`, `provider.wrap` and `provider.custom`.
Omitting middleware auditMode inherits the store policy; explicit
`disabled|shared|strict` overrides it. `scopeName` isolates fingerprint history.
Shared and strict use the existing rules auditor. Trace storage retains hashes
and telemetry. Recording is separate: `recorder.create` supports only explicit
`capturePolicy: full` with the existing redaction policy; audit-only recordings
are not an exposed capability.

## Streams and limits

`transcription.start` returns an Operation handle. Native
`dispatch::operation_push` admits binary bytes and `dispatch::operation_next`
returns Part, Ended or Timeout. JSON `operation.next` uses the same polling
service; generic JSON calls to `operation.push` are InvalidOp. Input closure,
cancellation, consumer closure and the terminal result share one operation state.

Stream events and binary chunks are limited to 1 MiB. Default operation queues
hold 64 input and 256 output items, with 8 MiB budgets each; terminal delivery
has a separate first-writer slot. Admission reserves capacity before copying
binary payloads. These limits cover wire bytes and admitted queue payloads,
not every upstream provider allocation. One-shot JSON requests have no byte cap
in this layer. The existing core text pump uses an unbounded internal channel.
Realtime cleanup has a five-second bound and reports cleanup failure separately
from the terminal outcome. Blocking recorder I/O uses `spawn_blocking`; adapter
finalizers enqueue cleanup.

## Manifest and registered differences

`manifest::manifest()` exports all 72 operations, including two Decision
operations beyond RFC-0039's original 70, 23 handle kinds, error codes and a
`types` catalogue. Ops controls and result wrappers carry schemas; canonical
business fields retain explicit type references. Codegen publishes the C header,
language constants and schemas. Its `--check` detects missing, stale and changed
outputs. The digest sorts object keys and operation names and excludes itself.

Registered differences from RFC-0039 include the Decision kind and operations,
BinaryPush-only JSON rejection for operation.push, and the native finalizer
primitive `aimux_handle_finalizer`. Error envelopes use the canonical error name
and payload with no outer retryable field. Existing numeric code aliases remain:
code 3 covers three variants, code 6 covers two, and code 4 is reserved. Protocol
codes use 300+. Stdio, host control-flow callbacks and skills/Batch/Evaluation
operations are outside the exposed inventory. A parsed negative, non-finite
or out-of-range preferred retry
value blocks fallback; malformed nonnumeric text allows fallback.

Registered gaps:

- **G-a:** recorder, session store and inference capture stay process-wide.
  Handle ownership does not provide per-owner capture isolation.
- **G-b:** recording format migration and removal of schema-1 reading belong to
  RFC-0041. Current canonical readers retain their existing schema compatibility.
- **G-c:** credential resolver creation and replay transport capabilities return
  UnsupportedFunctionality and have Future scope. Replay uses existing model-level
  language/Decision replay. Provider WebSocket/credential settings unsupported by
  its native factory also fail explicitly.
- **G-d:** canonical business schemas and language DTO generation belong to
  RFC-0042. `scripts/gen_ts_types.py` remains until its full outputs are covered.
- **G-e:** release ordering/header packaging does not establish installation of
  every published asset on every platform.

Model source summaries are object-level owner-scoped metadata; complete
per-handle/package provenance and authoritative defaults for every provider
are not exposed.
