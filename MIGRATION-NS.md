# Namespace types migration recipe (provider options / provider metadata)

Do not commit this file.

## What changed

Both are now `HashMap<String, JsonObject>` (namespace -> JSON object), the AI SDK `Record<string, JSONObject>`:

```rust
// aimux_core::shared  (also re-exported from aimux_core::prelude)
pub type JsonObject = serde_json::Map<String, serde_json::Value>;
pub type SharedProviderOptions = HashMap<String, JsonObject>;
pub type SharedProviderMetadata = HashMap<String, JsonObject>;
pub fn provider_namespace(namespace: &str, object: serde_json::Value) -> SharedProviderMetadata;
// aimux_core::types
pub type ProviderMetadata = SharedProviderMetadata;   // was serde_json::Value
```

Every `provider_options` field (content parts, messages, CallOptions, GenerateTextOptions, FunctionTool, every
`*CallOptions`) is `Option<SharedProviderOptions>` (the media/embedding call options that were already non-Option stay
non-Option). Every `provider_metadata` field (GenerateContent, StreamPart, ToolCall/ToolResult/RawToolCall, Source,
ReasoningOutput, GenerateResult, ...) is `Option<ProviderMetadata>`. Not changed: `recording::ProviderRecord.provider_options`
(provider construction options such as org/project, flat, not namespaced).

Imports:

```rust
use aimux_core::shared::{provider_namespace, JsonObject, SharedProviderMetadata, SharedProviderOptions};
use aimux_core::types::ProviderMetadata;
```

Wire JSON is unchanged: `{"openai": {"k": v}}`. A non-object namespace value (`{"openai": 1}`, `[]`, `"x"`, `null`) or a
non-object top level is now a deserialization error.

## Patterns

### 1. Build metadata / options, one namespace (the common case)

```rust
// before
provider_metadata: Some(json!({ "xai": { "itemId": item_id } })),
// after
provider_metadata: Some(provider_namespace("xai", json!({ "itemId": item_id }))),
```

Same for `provider_options: Some(...)` (identical type). The helper is `#[must_use]`; a non-object second argument yields
an empty namespace, so pass an object literal, or build a `JsonObject` yourself (pattern 3).

Functions that returned `Value` metadata now return `ProviderMetadata`:

```rust
// before
fn server_tool_metadata(...) -> Value { ...; json!({ "google": payload }) }
// after
fn server_tool_metadata(...) -> ProviderMetadata { ...; provider_namespace("google", payload) }
```

### 2. Build metadata with several namespaces

```rust
// before
Some(json!({
    "googleVertex": { "thoughtSignature": "a" },
    "vertex": { "thoughtSignature": "b" },
}))
// after (tests / fixtures): deserialize the literal, shape is checked by the type
Some(serde_json::from_value(json!({
    "googleVertex": { "thoughtSignature": "a" },
    "vertex": { "thoughtSignature": "b" },
})).unwrap())
// after (non-test code): extend
let mut m = provider_namespace("bedrock", json!({ "signature": sig }));
m.extend(provider_namespace("amazonBedrock", json!({ "signature": sig })));
Some(m)
```

### 3. Build from a `Map` you filled imperatively

The `Value::Object(..)` wrapper goes away.

```rust
// before
let mut metadata = HashMap::new();
let mut prodia_meta = Map::new();
prodia_meta.insert("images".into(), json!([job_result]));
metadata.insert("prodia".into(), Value::Object(prodia_meta));
// after
metadata.insert("prodia".into(), prodia_meta);        // prodia_meta: JsonObject (= Map<String, Value>)

// before
std::iter::once(("google".to_string(), Value::Object(metadata))).collect()
// after
HashMap::from([("google".to_string(), metadata)])     // or provider_namespace("google", Value::Object(metadata))
```

`Map::new()` is the same type as `JsonObject::new()`; `use aimux_core::shared::JsonObject` if you want the alias.

### 4. Merge into existing metadata

```rust
// before (Value)
meta["xai"]["reasoningEncryptedContent"] = json!(ec);
if let Some(o) = meta.as_object_mut() { o.insert(...) }
// after
meta.entry("xai".into()).or_default().insert("reasoningEncryptedContent".into(), json!(ec));
// merge a whole map one level deep
for (ns, obj) in other { meta.entry(ns).or_default().extend(obj); }
```

Building the inner object first and wrapping once is also fine:

```rust
let mut inner = json!({ "itemId": part_id });             // before: json! then index-assign
inner["reasoningEncryptedContent"] = json!(ec);           // still works on a Value
provider_namespace("xai", inner)                          // wrap at the end
```

### 5. Read an option from a namespace

`options.provider_options.get("x")` now returns `Option<&JsonObject>`, not `Option<&Value>`. Method chains that call
`.get("k")`, `.and_then(|o| o.get("k"))`, `.as_str()` on the *inner values* are unchanged.

```rust
// before and after (compiles unchanged)
let v = opts.provider_options.get("openai").and_then(|o| o.get("user")).and_then(|v| v.as_str());
// per-part options (Option<Value> before, Option<SharedProviderOptions> now)
let cc = provider_options.as_ref().and_then(|o| o.get("anthropic")).and_then(|a| a.get("cacheControl"));
```

What breaks:

```rust
// before: object-ness check on the namespace value
let o = provider_opts.as_object()?;           // E0599: no method as_object on &Map
// after: it already is the object
let o = provider_opts;
// before: .and_then(|v| v.as_object())   after: delete the and_then (the value is already &JsonObject)
// before: opts["k"] (Value index, Null on miss)   after: Map index PANICS on a missing key; use opts.get("k")
// before: Value::is_object / is_null on the namespace   after: gone (always an object)
// need a Value again (e.g. to feed serde_json::from_value or json!): Value::Object(obj.clone())
```

Looping a string-literal slice: `for key in &["bedrock", "amazonBedrock"] { po.get(key) }` fails with
`String: Borrow<&str>` (E0277); iterate by value (`for key in ["bedrock", "amazonBedrock"]`) or write `po.get(*key)`.

### 6. Pass through

Fields and function parameters that merely carry a value need no change except the declared type:

```rust
// before
fn f(meta: Option<Value>) -> StreamPart { StreamPart::TextEnd { id, provider_metadata: meta } }
// after
fn f(meta: Option<ProviderMetadata>) -> StreamPart { ... }
```

Local variables typed `Option<Value>` / `HashMap<String, Value>` that end up in these fields: retype them.
Intermediate scratch values that are not themselves stored (e.g. `StreamingToolCallTracker::with_extract_metadata`
closures, which still return `Option<Value>`) stay `Value`; `with_build_provider_metadata` still takes `Option<&Value>`
and returns `Option<ProviderMetadata>`, so build with `provider_namespace("google", json!({...}))`.

### 7. Tests comparing against `json!`

```rust
// before
assert_eq!(part.provider_metadata, Some(json!({ "anthropic": { "signature": "s" } })));
// after, option 1: compare typed
assert_eq!(part.provider_metadata, Some(provider_namespace("anthropic", json!({ "signature": "s" }))));
// after, option 2: compare the serialized shape (keeps the json! literal as-is)
assert_eq!(serde_json::to_value(&part.provider_metadata).unwrap(), json!({ "anthropic": { "signature": "s" } }));
// indexing keeps working: HashMap<String, Map> -> m["anthropic"]["signature"] == json!("s")
// but a whole namespace is a Map, not a Value: Value::Object(m["anthropic"].clone()) == json!({ ... })
```

### 8. `Option<Value>` per-part / per-message options

`ContentPart::*.provider_options`, `LanguageModelPromptMessage.provider_options` were `Option<Value>`; now
`Option<SharedProviderOptions>`. Constructing sites become `Some(provider_namespace(..))` (pattern 1); `None` unchanged.
A flat (non-namespaced) object such as `json!({ "organization": "o" })` is no longer representable there; if a test
or code passed one, it was relying on the loose shape: wrap it in a namespace or drop it.

### 9. `Value` results holding provider metadata

`GenerateTextResult.provider_metadata`, `GenerateObjectResult.provider_metadata`, `ReasoningOutput.provider_metadata` and
the stream `Finish` part are `Option<ProviderMetadata>`. `ResponseMessageBuilder` is `pub(crate)` (core only).

## Contract-test fixtures / tests that fail because they pin a non-object namespace value

Scanned every `*.json` / `*.jsonl` under contract-tests, aimux-core, aimux-provider-utils, aimux-providers/tests
(fixtures and cassettes), bindings and tools for `provider_options|provider_metadata|providerOptions|providerMetadata`
keys whose value is not `null` or an object of objects: none found. In particular
`contract-tests/fixtures/wire-format.json` (15 occurrences) already uses the namespace -> object shape, and
`aimux-core/tests/contract_test.rs` passes. Tests inside `aimux-providers/tests/*.rs` that build these values with
`json!` do not compile yet and are handled per vendor directory with the recipe above; any that pin a non-object
namespace value (for example `json!({ "openai": 1 })` or a flat `{ "organization": ... }`) must be reported with
file:line rather than kept loose.
