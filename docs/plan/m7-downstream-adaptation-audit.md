# M7 Downstream Adaptation Audit (RFC-0016)

**Date**: 2026-08-12
**Scope**: PR #98 (`feat/m7-top-level-result-aggregation`)
**Status**: Audit — tracks which language bindings need adaptation for the 5
new `GenerateTextResult` fields.

## New fields (added in `aimux-core/src/generate.rs`)

| Field | Rust type | Wire (JSON) type |
|---|---|---|
| `reasoning` | `Vec<ReasoningPart>` | `[{ "text": string }]` |
| `reasoning_text` | `String` | `string` |
| `sources` | `Vec<SourcePart>` | `[{ "id", "source_type", "url"?, "title"? }]` |
| `files` | `Vec<FilePart>` | `[{ "data": FileData, "media_type" }]` |
| `response_messages` | `Vec<ModelMessage>` | `[{ "role", "content" }]` |

All new fields use `#[serde(default)]` → absent in old JSON → empty/zero values.

## Adaptation status

### Auto-benefit (0 changes)

| Language | Why |
|---|---|
| **Node** | `generate_text` returns `serde_json::to_string` result directly (JSON string). ts-rs auto-generates `GenerateTextResult.ts` with all 5 fields + `ReasoningPart.ts`/`SourcePart.ts`/`FilePart.ts` re-exported via `types.ts`. |
| **Python (raw dict path)** | `generate_text` returns JSON string; `__init__.py:249` does `json.loads(result)` — raw dict, no field filtering. |
| **Python (typed wrapper path)** | `wrapper.py:870` `GenerateTextResult` Pydantic model adapted with 5 new fields (weak type `Dict` for reasoning/sources/files, `ModelMessage` for response_messages). |
| **Swift (dict path)** | `Aimux.swift:776` `generate()` returns `[String: Any]` — auto. (Only the typed `generateText()` path needs adaptation.) |
| **C ABI (FFI wire)** | `aimux_generate_text` returns JSON string via `run_async_json` → `serde_json::to_string`. All 5 fields present in the JSON payload. |

### Need adaptation (typed struct)

#### Go — `bindings/go/types.go:142`

Current `GenerateTextResult` has 6 fields. Add 5:

```go
Reasoning        []json.RawMessage `json:"reasoning,omitempty"`
ReasoningText    string            `json:"reasoning_text,omitempty"`
Sources          []json.RawMessage `json:"sources,omitempty"`
Files            []json.RawMessage `json:"files,omitempty"`
ResponseMessages []ModelMessage    `json:"response_messages,omitempty"`
```

- `reasoning`/`sources`/`files` use `json.RawMessage` (weak type, same style as `Warnings []json.RawMessage`).
- `response_messages` reuses existing `ModelMessage` (`types.go:167`).
- **No new types needed.**

#### Swift — `bindings/swift/Sources/Aimux/Types.swift:882`

Current `GenerateTextResult` Codable struct has 6 fields + `CodingKeys` (6 cases) + `init` (6 params). Add 5 properties, 5 CodingKeys cases, 5 init params.

- `reasoning`/`sources`/`files` use `[JSONValue]` or `[[String: Any]]` — check if project has a `JSONValue` helper; otherwise define a minimal `AnyCodable` or use weak `[String: Any]` with custom init.
- `response_messages` reuses existing `ModelMessage` (`Types.swift:583`).
- CodingKeys: `reasoning`, `reasoning_text`, `sources`, `files`, `response_messages`.

#### Java — `bindings/java/.../Types.java:2618`

Current `GenerateTextResult` POJO + Builder + equals/hashCode, 6 fields. Add 5 `@JsonProperty` fields, extend private constructor, extend Builder, extend equals/hashCode.

- `reasoning`/`sources`/`files` use `List<JsonNode>` (same style as existing `warnings`).
- `response_messages` uses `List<ModelMessage>` (`Types.java:1527`).

#### Kotlin — `bindings/kotlin/.../Types.kt:909`

Current `data class GenerateTextResult` has 6 `@SerialName` fields. Add 5.

- `reasoning`/`sources`/`files` use `List<JsonElement>` (same style as `raw: JsonElement`).
- `response_messages` uses `List<ModelMessage>` (`Types.kt:526`).

#### Flutter — `bindings/flutter/lib/types.dart:580`

Current `@JsonSerializable() class GenerateTextResult` has 6 fields. Add 5 fields + regenerate `types.g.dart`.

- `reasoning`/`sources`/`files` use `List<Map<String, dynamic>>`.
- `response_messages` uses `List<ModelMessage>` (`types.dart:758`).
- Must run `dart run build_runner build` to regenerate `.g.dart`.

## Design decision: weak types for reasoning/sources/files

`ReasoningPart`/`SourcePart`/`FilePart` are intentionally NOT given strong types
in the 5 C-ABI languages (only in Rust core + Node TS). Rationale:

1. These are **convenience aggregation fields** — the canonical typed data
   lives in `raw.content` (which is already strongly typed as `ContentPart[]`
   in every language).
2. Each language's existing `Warnings` field already uses a weak type
   (`json.RawMessage` / `JsonNode` / `JsonElement` / `[String:Any]` /
   `Map<String,dynamic>`). Matching that style is consistent.
3. Power users who need full typing read `raw.content`; the top-level fields
   are for quick access.

`response_messages` is strongly typed (reuses `ModelMessage`) because its
primary use case — appending to the next-turn prompt — requires type
compatibility with the prompt API.
