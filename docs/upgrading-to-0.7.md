# Upgrading to aimux 0.7

> **0.7.0 is a breaking release for the Rust API and for two binding
> types.** The wire protocol, the ops manifest (ABI 1 / ops 1.0) and the C ABI
> stay compatible with 0.6; `aimux_call_async` / `aimux_stream_async` are
> additive exports. If you only use Node, Python, Go, Java or Kotlin through
> the generated DTOs, nothing below applies except the `provider.create`
> settings validation.

This guide lists what changed and how to move a 0.6 integration. The exhaustive
list is the `Breaking` section of the [0.7.0 changelog](../CHANGELOG.md#070---2026-11-24).

## Upgrade all layers together

Same rule as 0.6: a 0.7 runtime requires the matching header, generated
metadata and language packages. Do not mix 0.6 and 0.7 artifacts in one process;
the manifest digest changes even though the ops version does not.

| Layer | 0.7 requirement |
|---|---|
| Rust crates | `0.7.0` (workspace version) |
| C ABI | ABI major `1` (`AIMUX_ABI_VERSION == 65536`), now 16 exports |
| Ops protocol | `1.0`, 76 operations (72 in 0.6) |
| C header | `aimux-ffi/include/aimux.h` generated from the same commit |
| Manifest | the runtime's manifest `digest` must equal the generated `MANIFEST_DIGEST` |
| Bindings | Node/Python/Go/Java/Kotlin/Swift/Flutter/C from the same release |
| iOS | `aimux_ffi.xcframework` now ships a **dynamic** framework; the podspec no longer uses `-force_load` |

## Shared: `provider.create` validates settings

`settings` is now checked against the package's `PackageDescriptor`. The
visible change: `settings.organization` is accepted only by `openai`, and
`settings.project` only by `openai` and `google_vertex` (where it sets the GCP
Vertex project rather than sending an `OpenAI-Project` header). On any other
package those two fields are rejected with `InvalidArgument` instead of being
silently turned into headers. Pass custom headers explicitly:

```json
{ "package": "ollama",
  "settings": { "headers": { "OpenAI-Organization": "org-1", "OpenAI-Project": "proj-1" } } }
```

`settings.params` is likewise accepted only by preset packages that declare
`templateParams`. Use the new `provider.descriptors` op (optionally with a
`package`) to see exactly which settings, language APIs and template params a
package accepts.

## Rust

### `chrono` → `jiff`

`aimux_core::Timestamp` is backed by `jiff` (#523, `RUSTSEC-2026-0543`):

```rust
// 0.6
let ts = Timestamp::from_datetime(dt);
let dt: chrono::DateTime<Utc> = ts.to_datetime();

// 0.7
let ts = Timestamp::from_timestamp(jiff::Timestamp::now()).unwrap();
let ts: Timestamp = jiff_ts.try_into()?;           // jiff spans ±9999 years; Timestamp is 0000..=9999
let jiff_ts: jiff::Timestamp = ts.to_timestamp();   // or ts.into()
```

`SigV4Fetch::with_clock` takes `fn() -> jiff::Timestamp` instead of a
`chrono::DateTime<Utc>`.

### `tools` is shared

`GenerateTextOptions.tools` and `CallOptions.tools` are `Option<Arc<[Tool]>>`
so repeated calls can share one schema set. The wire field is still a JSON
array.

```rust
let tools: Arc<[Tool]> = vec![weather, search].into();
options.tools = Some(tools.clone());
```

### `HttpRequest` has a new required field

If you construct `aimux_provider_utils::HttpRequest` with a struct literal, set
`kind` (or use `..Default::default()`). For synchronous, billable image /
speech generations build the request with `.generation_for(&options)` (it
consumes the request and takes `&ImageCallOptions` / `&SpeechCallOptions`) so
the exchange gets the 10-minute guard and is **not** replayed after the
request was sent.

### `aimux_ops` handler signature

`Handler::Call` is now `for<'a> fn(Ctx, Target, Params<'a>) -> CallFuture<'a>`
and returns serialized JSON (`String`); a new `Handler::TypedStream` variant
exists. `OpDescriptor.runtime_handle_fields` is `&[HandleField]`
(`(&["settings", "fetch"], &[HandleKind::Fetch])`) instead of dotted strings.
`decode(params)` accepts `Params<'_>` directly.

### Removed module paths

`aimux_providers::voyage::embedding` / `::reranking` are gone; import
`VoyageEmbeddingModel` / `VoyageRerankingModel` from `aimux_providers::voyage`.

### New: direct language APIs and declarative configuration

- `Provider::language_model_with_api(LanguageApi::Chat | Responses | Completion | Messages, id)`
  lets one provider serve several protocol families; `CustomProvider` gains
  `with_language_api_models(api, map)` / `with_language_api_model(api, id, model)`.
- `aimux_providers::{provider_from_config_with_transport, providers_from_config_with_policy,
  EnvLookupPolicy}` build providers from a `ProviderConfigDocument` with
  `{ "env": "VAR" }` (`StringOrEnv`) credentials.
- `ProviderRegistry` is in-place mutable (`register`, `unregister`,
  `extend_providers`, `replace_providers`, `update_providers`) and derivable
  (`with_updated_providers`); `provider()` returns `Arc<dyn Provider>`,
  `provider_ids()` returns `Vec<String>`.
- `aimux_provider_utils::passthrough` exposes the L0 native endpoint
  (`NativeEndpoint`, `NativeRequest`, `NativeContext`, `NativeResponse`,
  `NativeBodyStream`, `NativeExchangeObserver`).

## Bindings

### Node.js / Python / Go / Java / Kotlin

No source changes. New operations are available through the generated ops
facades:

- `provider.descriptors`, `registry.fromConfig`, `registry.list`,
  `registry.update`
- `provider.model` / `model.resolve` / `provider.custom` accept `method:
  "chat" | "responses" | "completion" | "messages"` (and `chatModels`,
  `responsesModels`, … maps on `provider.custom`)
- `provider.create` accepts the `openai_compatible` package with `name` +
  `baseUrl`, `{ "env": "VAR" }` credentials and an optional `envAllowlist`.
  `provider.create` and `registry.fromConfig` are **trusted-caller** ops: the
  allowlist gates explicit env interpolation only and does not suppress
  implicit per-provider env fallbacks. Do not expose them to untrusted input.
- Go uses the new async C ABI (`aimux_call_async` / `aimux_stream_async`);
  streams deliver batches of ready frames followed by a terminal frame.
- Node streams drain ready frames in batches internally; the public iterator
  still yields one frame at a time.

### Swift and Flutter: typed `cause`

`AimuxError.cause` (Swift) and `OpsException.cause` (Flutter) are now the
generated `ErrorCause?` model instead of untyped JSON, matching the other
bindings:

```swift
switch error.cause {
case .aiMuxError(let inner): …
case .sourceError(let src): …   // {message, cause?}
case .none: …
}
let raw = error["cause"]        // untyped access stays available
```

### C and C++

`aimux.h` has two new exports, `aimux_call_async` and `aimux_stream_async`
(16 exports total). A completion callback may run before the submitting call
returns, may run on a runtime thread, and must return promptly; re-entering
the dispatcher from inside a callback fails with `204`. Submission copies its
inputs, so `op` / `params` may be freed as soon as the call returns; a
rejected submission invokes no callback.

### iOS / Flutter

`aimux_ffi.xcframework` now contains a dynamic `aimux_ffi.framework` instead of
merged static archives. The CocoaPods podspec drops `-force_load`; apps that
linked the static library manually should link the framework instead. Bitcode
is not shipped (zero-bitcode is enforced at build time).

## Migration checklist

- [ ] Rust: replace `Timestamp::from_datetime` / `to_datetime`, and any
      `chrono::DateTime<Utc>` passed to `SigV4Fetch::with_clock`.
- [ ] Rust: `tools` is `Option<Arc<[Tool]>>`; convert with `Some(v.into())`.
- [ ] Rust: set `HttpRequest.kind` (or `..Default::default()`); use
      `.generation_for(&options)` for synchronous image / speech exchanges.
- [ ] Rust: update custom `aimux_ops` handlers to the `Params<'_>` / `String`
      signature and the `TypedStream` variant.
- [ ] Rust: `voyage::{embedding, reranking}` paths → `aimux_providers::voyage`.
- [ ] Swift / Flutter: `cause` is `ErrorCause?`; read `error["cause"]` for raw JSON.
- [ ] All: `provider.create` rejects `organization` / `project` on packages
      other than `openai` / `google_vertex`; pass headers explicitly.
- [ ] All: upgrade every layer (crates, native library, header, bindings) to
      0.7.0 together; the manifest digest changed.
- [ ] iOS: link the dynamic framework; remove any `-force_load` workaround.

## Rollback

0.7.0 does not change the ops protocol or the public JSON shapes, so a 0.6
binding keeps working against a 0.7 native library for the operations it
already used — but the manifest digest differs, and a 0.7 binding rejects a
0.6 native library at load. To roll back, roll back every layer together to
0.6.1.
