## Unreleased

- Breaking: the typed layer and the maps `Model` returns follow the AI SDK
  JSON of the Rust core: camelCase fields, `type`-tagged unions with
  kebab-case values (`text-delta`, `tool-call`, ...), optional fields absent
  instead of `null`, `FileBytes` untagged, errors keyed by `name`.
- Breaking: `ContentPartToolResult` carries `toolCallId`, `toolName` and a
  typed `ToolResultOutput`; `result`, `isError`, `preliminary`, `dynamic` are
  gone from the message part (provider-executed `ToolResult` keeps them).
- Breaking: `StreamPart` models the stream's `TextStreamPart` (adds
  `finish-step`; `StreamPartToolCall` / `StreamPartToolResult` / `StreamPartFile`
  wrap `ToolCall` / `ToolResult` / `GeneratedFile`; no `response-metadata`).
- Breaking: `registerProviders` is removed together with
  `aimux_register_providers`; `config_json` keys are camelCase (`baseUrl`) and
  unknown keys are rejected.
- `types.dart` no longer uses `json_serializable` except for the OpenAI Chat
  Completions family; `types.g.dart` is regenerated for it.

## 0.5.0

- Breaking release aligning the whole aimux family — see the
  [main CHANGELOG](https://github.com/arcships/aimux/blob/master/CHANGELOG.md)
  for the complete list (stable cross-language error model, request
  pipeline retries/timeouts, Core-boundary tool-input validation, host-side
  `repairToolCall`, WS proxy, SSRF hardening).
- Flutter binding: typed error model with retry context, host-side
  `repairToolCall`, `VideoPollOptions`.
- The embedded iOS `aimux_ffi.xcframework` no longer carries LTO bitcode /
  debug symbols: each slice shrinks from ~128MB to ~15MB (the 0.3.0
  archives had grown past pub.dev's 256MB package limit).

## 0.2.1

- First pub.dev release under publisher `arcships.ai`.
- Flutter plugin conversion: Android `libaimux_ffi.so` per ABI and iOS
  `aimux_ffi.xcframework` embedded in the package; Dart-only plugin
  (`dartPluginClass`), no platform channels.
- iOS integration via CocoaPods (`aimux.podspec`): vendored static
  framework slices staged by `script_phase`, symbols force-loaded into the
  app binary (`user_target_xcconfig`) — verified by CI symbol scan
  (issue #25). SwiftPM is not used (Flutter 3.44 cannot link plugin binary
  targets); privacy manifest (`PrivacyInfo.xcprivacy`) bundled.
- `example/` app demonstrating the API; CI builds it for iOS and Android
  to validate native integration.
