# Changelog

All notable changes to aimux are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Breaking

- Public surface (`aimux-core`, `aimux-provider-utils`): removed
  `recording::init_recording_from_env`; made private the retry preparation
  helpers and default constants, `composite::{add_usage,
  build_aggregator_prompt, extract_text}`, `recording::new_call_id`,
  `util::rfc3339_now`, two session helpers, `fetch_error_to_ai_mux_error`,
  `extract_response_header_pairs`, the SigV4 signing internals, the logging
  internals, `DEFAULT_MAX_JSON_RESPONSE_SIZE` and `TungsteniteConnector`.

- Provider results carry request and response information as the AI SDK does
  (`aimux-core`). `GenerateResult` has `request: Option<RequestInfo>` and
  `response: Option<ResponseInfo>`; `ResponseInfo` gains `id` and holds the
  timestamp, model id, headers and body. `StreamResult` has `request` and
  `response: Option<StreamResponseInfo>`. The flat `request_body` and
  `response_headers` fields are removed and `response` is optional; the call
  layer fills a missing id, timestamp and model id. The user-facing results and
  their JSON are unchanged.

- The prompt a provider receives is modelled by role (`aimux-core`), as the AI
  SDK's `LanguageModelV4Message`. `LanguageModelPrompt` is
  `Vec<LanguageModelMessage>`: `System { content: String }`,
  `User { content: Vec<UserPart> }`, `Assistant { content: Vec<AssistantPart> }`,
  `Tool { content: Vec<ToolPart> }`. The parts are `TextPart`, `FilePart`,
  `ReasoningPart`, `ToolCallPart`, `ToolResultPart`. A provider sees one file
  part, `FilePart { data: FileData, media_type, filename }`; the five
  user-facing file variants are folded into it by
  `convert_to_language_model_prompt`, which now returns `Result` and rejects a
  part its role does not allow. `LanguageModelPromptMessage` is removed. The
  user-facing `ModelMessage` and `ContentPart` are unchanged. Recordings store
  the new prompt JSON: a system message's `content` is a string and every file
  input is a `file` part.

- Provider options and provider metadata have one typed shape, the AI SDK's
  `Record<string, JSONObject>`: `SharedProviderOptions` and
  `SharedProviderMetadata` (`ProviderMetadata` is an alias) are
  `HashMap<String, JsonObject>`, namespace to JSON object. They replace
  `serde_json::Value`, `HashMap<String, Value>` and the per-part
  `Option<Value>`. Every `provider_options` / `provider_metadata` field of
  content parts, messages, call options, tools, results and stream parts uses
  them. `{"namespace": {"key": value}}` serializes as before; a value that is
  not namespace -> object (a number, string or array under a namespace, or a
  non-object at the top) is rejected when deserialized. Build one with
  `provider_namespace(ns, json!({..}))`.

- Tool-call and tool-result types are unified, one per protocol layer
  (`aimux-core`). A provider emits `tool::RawToolCall` (`input` is the raw
  argument text) in `GenerateContent::ToolCall(..)` and
  `StreamPart::ToolCall(..)`, and `tool::ToolResult` in the `ToolResult`
  variants; these variants are newtype variants now. `StreamPart` is generic
  over the tool-call type: a provider's `do_stream` yields `StreamPart`
  (= `StreamPart<RawToolCall>`), `stream_text` and everything after it yield
  `TextStreamPart` (= `StreamPart<ToolCall>`, parsed input with `invalid` /
  `error`). `RawToolCall` moved from `parse_tool_call` to `tool`. Wire format:
  enum variants keep their JSON shape; a provider-layer `StreamPart::ToolCall`
  has a string `input` and no `invalid` / `error`; a
  `GenerateContent::ToolCall` with a non-string `input` no longer
  deserializes; the standalone `tool::ToolResult` gains a required
  `tool_name` and optional `dynamic` / `provider_metadata`.
- Source, generated-file and reasoning types are unified, one per concept
  (`aimux-core`). `result::Source`, `result::GeneratedFile` and
  `result::ReasoningOutput` are the payloads of `GenerateContent::{Source,
  File, Reasoning}` and `StreamPart::{Source, File}` (newtype variants now) and
  the elements of `sources`, `files` and `reasoning` on `GenerateTextResult`
  and `StreamTextResultAggregated`; `StreamPart::ResponseMetadata` wraps
  `types::ResponseMetadata`. `SourcePart`, `FilePart` and `ReasoningPart` are
  removed (TypeScript: `Source`, `GeneratedFile`, `ReasoningOutput`). Wire
  format: enum variants keep their JSON shape; the top-level `sources` /
  `files` / `reasoning` arrays gain `provider_metadata` (so the reasoning
  signature is no longer dropped), and a source's `url` / `title` serialize as
  `null` when absent instead of being omitted.

- Removed the generated `ProviderName` type in every binding (Rust enum, TS
  const object, Go/Java/Kotlin consts, Swift enum, Dart consts, Python
  `Literal`) and `scripts/gen_provider_names.py`. Provider names are plain
  strings: Rust uses `create_provider("groq", PresetSettings)`; bindings retain
  `provider("groq", ...)`, including overlay-registered names. The provider list lives in
  [docs/api/providers.md](docs/api/providers.md); Rust also gains
  `provider_names()`.

**Rust (aimux-stream)**

- `SseStream` is now a thin adapter over the `sse-stream` crate (WHATWG
  event-stream parsing), mirroring how the AI SDK's `parseJsonEventStream`
  wraps `eventsource-parser`: `\n`, `\r` and `\r\n` line endings in any
  mix, a leading UTF-8 BOM is stripped, a field line without `:` counts as an
  empty value, an empty `event:` is `None`, an `id` containing U+0000 is
  ignored, `retry` must be all ASCII digits, and a block is dispatched only
  when it had a `data` line. There is no event size limit any more (as
  upstream): `with_max_event_size` and `SseError::FrameTooLarge` are gone,
  which fixes streams carrying multi-megabyte events (OpenAI Responses and
  Gemini image generation). `SseError::Stream` carries the transport error
  as its source instead of a `String`; `SseError::Utf8` holds a
  `str::Utf8Error`; `SseError::Decode` is added. Decoder errors (invalid
  UTF-8, transport failure) now end the stream after being reported.
  `SseStream` requires the body error type to implement `std::error::Error +
  Send + Sync + 'static`.
- Removed `NdjsonStream` / `NdjsonError`: nothing in the workspace used them
  and the AI SDK has no counterpart. `tokio` is now a dev-dependency only and
  the unused direct `serde` dependency is dropped.

**Rust (aimux-stream)**

- `StreamingToolCallTracker`, `ToolCallStreamPart` and `TrackerError` are no
  longer exported from `aimux-stream`; nothing in the workspace used them and
  the AI SDK keeps the tracker in `@ai-sdk/provider-utils`, next to the
  provider code that drives it. See `aimux-provider-utils` below.

**Rust (aimux-provider-utils)**

- Gains `StreamingToolCallTracker` (plus `StreamingToolCallDelta`,
  `StreamingToolCallFunction`, `TypeValidation`, `TrackerError`,
  `StreamingToolCallArgumentState` and `starts_with_structured_value`),
  a port of the current `@ai-sdk/provider-utils` tracker. The previous
  `aimux-stream` version was a port of an older, index-only tracker. Deltas
  are correlated by wire `id`, `index` and function name (ambiguous deltas
  are dropped), ids are de-duplicated with bounded suffixes, blank function
  names are ignored, and `flush` orders calls by index only when every call
  has one. It emits `aimux_core::StreamPart` tool-input parts directly
  (`ToolInputStart` / `ToolInputDelta` / `ToolInputEnd` / `ToolCall`); there
  is no separate `ToolCallStreamPart` event type. `TrackerError` converts
  into `AiMuxError::InvalidResponseData`. Metadata hooks are typed with
  `serde_json::Value` / `ProviderMetadata`, and the builder closures must be
  `Send + Sync`.

**Rust (aimux-providers)**

- The OpenAI chat-completions stream (`openai/model.rs`, which serves every
  registry-backed provider) correlates `tool_calls` deltas with the tracker
  instead of by `index` alone: deltas are matched by wire id, index and
  function name; a continuation without an id follows its call; indices
  reused across parallel calls stay distinct; ambiguous deltas are dropped;
  a call whose delta carries no id gets a generated `tool-call` /
  `tool-call-N` id instead of an empty string; a new call without a function
  name ends the stream with `InvalidResponseData` (previously it started a
  call with an empty name). `DeltaToolCall.index` is now `Option<usize>`.

**Rust (provider factory, RFC-0036)**

- `OpenAIProvider::language_model(id)` (and so `create_provider("openai", ..)`
  and the registry id `openai:<model>`) returns the **Responses** model, as
  `@ai-sdk/openai` does. It returned the Chat Completions model. Call
  `.chat(id)` for Chat Completions, for example against an endpoint that has
  no `/responses` route.
  The bindings' `openai(...)` constructors and the C ABI's `aimux_openai_new*`
  are unchanged: they still create the Chat Completions model.

- Removed crate-root exports `provider()`, `provider_handle`,
  `provider_from_env`, `provider_discovery`, `provider_registry_entry`,
  `ProviderOptions`, `ProviderProfile`, `register_provider` and
  `load_providers_from_json`. Binding compatibility lives in
  `aimux_providers::provider`; binding APIs are unchanged. Rust by-name
  creation is `create_provider(name, PresetSettings)`; `provider_names()`
  lists names, and `default_providers()` supplies the map for
  `aimux_core::provider_registry::create_provider_registry(providers, options)`
  with `provider:model` ids by default.

- Redirects are handled in the request helper layer, not in the transport.
  An ordinary API call follows a redirect only while it stays on the same
  origin; a cross-origin `3xx` is returned as a non-2xx response, so no
  credential header (`x-api-key`, `x-goog-api-key`, `api-key`,
  `x-amz-security-token`, ...) and nothing a transport decorator adds reaches
  another origin. Every followed hop goes through the request's transport
  again, so SigV4 signs the URL it sends. A `Fetch` never follows a redirect
  itself: `RedirectPolicy`, `FetchRequest.redirect` and
  `PinnedFetch::unpinned` are removed. Validated downloads keep their
  hop-by-hop guard.
- `Provider` follows the AI SDK's `ProviderV4`: `language_model`,
  `embedding_model` and `image_model` are required and return
  `NoSuchModel { model_id, model_type }` when a vendor has no such modality;
  `transcription_model`, `speech_model`, `reranking_model` and `files` are
  optional (`None` = not offered); video and search stay aimux extensions.
  Every constructor returns an `Arc<dyn …>`. Removed: `Provider::name()`,
  `specification_version()` (from `Provider` and the nine model traits),
  `LanguageModel::config_snapshot()` and `list_models` on `Provider`.
  Discovery is the separate `ProviderDiscovery` trait, accessed through
  `Provider::discovery()`; `supported_urls()` is added to `LanguageModel`.
- Every `XxxConfig`, config builder, `from_env()`, `with_*` method and
  `XxxProvider::new(...)` is gone, in every provider package. Each package
  has package-specific `XxxProviderSettings`, `create_xxx(settings)`
  (validating package-specific settings, including base URLs, conflicting
  credentials and template parameters) and a default instance `xxx()` where
  the package provides one.
  Models read a crate-private per-model config; there are no getters and no
  snapshot. `Fetch` (HTTP) and package-specific WebSocket settings are
  transport injection points. OpenAI, Anthropic and Google use fixed string
  credentials and header maps, without a body-transform setting. The
  OpenAI-compatible package exposes its upstream `transform_request_body` hook.
- For packages using the API-key loader, environment fallbacks are evaluated
  on every request: `api_key: None` reads the package's environment variable, `Some("")` is
  sent verbatim and never falls back to the environment, and a missing key
  fails the call (not the factory) with `AiMuxError::LoadApiKey { env_var,
  description }`. A missing required setting (AWS region, Azure resource name,
  Vertex project, …) fails the call with `AiMuxError::LoadSetting { env_var,
  name }`. Credentials and custom headers otherwise follow each package's
  settings; the generic compatible factory sends no auth for a missing or empty
  key. Headers merge case-insensitively in package-specific order; custom and
  call headers can override credential headers.
- Retry is a call-level concern only. `RetryConfig`, `retry_config()`,
  `with_retry_config`, the `max_retries` fields on every provider config and
  `ProviderRecord.max_retries` are deleted; the nine core operations call
  `prepare_retries(max_retries, abort)` with the constant defaults 2 retries,
  2000 ms initial delay, factor 2 (jitter, `Retry-After` and the error history
  are unchanged). Providers no longer retry anything themselves: `list_models`
  and files uploads are single exchanges, and job-creating requests are sent
  exactly once.
- `body_overrides` is removed everywhere (provider configs, builders,
  `CallOptions`, `ProviderOptions`, `config_json`, binding `ProviderConfig`).
  OpenAI-compatible provider-level body rewrites use its upstream
  `transform_request_body` closure on the finished JSON body. `ProviderOptions`,
  the C ABI `config_json` and the Node `ProviderConfig` that carry `max_retries` or
  `body_overrides` now fail with `InvalidArgument` instead of ignoring them
  (`reject_removed_provider_options` for other shapes).
- Recording: `RECORDING_SCHEMA = 3`. `ProviderRecord` holds identity only
  (`provider_id`, `provider`, `model_id`); base URL, key source, profile,
  options and retry settings are no longer recorded and schema-2 files are
  rejected on read. `rebuild_provider(record, registry)` resolves
  `provider_id` through the caller-supplied registry, retaining its settings.
  If the default model method differs from the recorded one, pass the model
  to `replay_with_model`. Recorded provider strings change with the next item.
- `model.provider()` is `"{name}.{method}"`, where `name` is
  `settings.name` (default: the package name). Defaults: `openai.chat` /
  `openai.responses` / `openai.embedding` / `openai.image` / `openai.speech` /
  `openai.transcription` / `openai.files`; `anthropic.messages` (a custom name
  is used verbatim; `{name}.files` derives from it), `anthropic-aws`,
  `googleVertex.anthropic.messages`; `google.generative-ai` (+ `.video`,
  `.files`), `google.vertex` (+ `.video`, `.transcription`),
  `amazon-bedrock`; `azure.chat` / `.responses` / `.embeddings` / `.image` /
  `.transcription` / `.speech`; `xai.responses`, `huggingface.responses`,
  `mistral.chat` / `.embedding`, `cohere.chat` / `.textEmbedding` /
  `.reranking`, `codex.responses`, `{name}.responses` (open_responses),
  `voyage.embedding` / `.reranking`, `elevenlabs.speech` / `.transcription`,
  `groq.chat`, `deepseek.chat`; compat and preset providers
  `{name}.chat` / `.embedding` / `.image`; single-modality packages
  `luma.image`, `deepgram.transcription`, `tavily.search`, `amazon-polly.speech`,
  `blackForestLabs.image`, `cartesia.transcription`, … . Consumers that
  compared `provider` to `"openai"` must match the prefix (the CLI and web
  probes filter by `model.provider()`).
- providerOptions namespaces are canonical only: `googleVertex` and
  `amazonBedrock` (the legacy `vertex` / `bedrock` keys are neither read nor
  written; Bedrock still reads `anthropic` for its Anthropic models). The
  Anthropic package reads `anthropic` merged with the first segment of a
  custom `name` (custom wins) and writes metadata under the custom key;
  compat providers no longer read the `openai` key; `providerOptions.deepseek`
  is honoured (`reasoningEffort`, `thinking`).
- Presets: `provider_registry.json` has 281 rows; Groq and DeepSeek are
  vendor packages instead of preset rows. Former thin wrapper types are
  deleted. The JSON is embedded and parsed once into a runtime descriptor
  table in `preset.rs`; there is no `family` field, generated presets source
  or per-name Rust function. Create presets or vendor packages by name via
  `create_provider(name, PresetSettings)`.
  Rows carry `auth: api_key | none` (`none` sends no
  `Authorization`; `PLACEHOLDER_API_KEY` is gone), `base_url_env` and
  template `params` (env, default, derived host maps). Only declared
  parameters are accepted, host parameters reject `/ @ : ?`, and an
  unexpanded placeholder is an error. Unknown names are `NoSuchProvider`;
  nothing falls back to OpenAI. `litellm_proxy` reads `LITELLM_PROXY_BASE_URL`
  (the old wrapper read its API-key variable as a URL). Registry validation
  runs in the table test, and
  `scripts/check_provider_boundaries.sh` runs in CI.
- Groq and DeepSeek are standalone packages (`create_groq`, `create_deepseek`)
  with their own chat models; the native OpenAI package no longer knows any
  other vendor. Compat providers expose chat, embedding and image only (the
  upstream set); the native OpenAI package warns on and drops `top_k`;
  `OpenAICompatProfile` and `with_profile` are deleted in favour of dialect
  hooks.
- Anthropic: the base URL includes `/v1` (endpoint `{base}/messages`; only the
  bare `https://api.anthropic.com` is rewritten); `ANTHROPIC_BASE_URL` and
  `OPENAI_BASE_URL` supply the base URL when the corresponding setting is
  absent; `stream` is omitted on non-streaming calls; `providerOptions.anthropic.metadata.userId` maps to `metadata.user_id`; `anthropic-beta` is
  sent on every host. Result-level `providerMetadata` is now populated
  (`usage`, `stopSequence`, `iterations`, `container`, `contextManagement`,
  under `anthropic` and the custom name), `usage.raw` is the provider's own
  usage object, and `usage.iterations` now feeds the token totals. Vertex-
  Anthropic and `anthropic_aws` run on the Anthropic core.
- Google: request bodies follow the AI SDK (`generationConfig` is always
  sent, function tools use `parametersJsonSchema`, `providerOptions.google`
  maps `thinkingConfig`, `responseModalities`, `safetySettings`,
  `cachedContent`, …; response `modelId` comes from `modelVersion`).
  Vertex: Gemini uses `v1beta1`, Anthropic-on-Vertex `v1`; a tuned model in
  express mode now fails at request time instead of at construction.
  Bedrock signs the final request bytes through `SigV4Fetch`;
  `list_models` now calls the control-plane host
  `bedrock.{region}.amazonaws.com` (it used `….api.amazonaws.com`).
- Azure defaults to `api_version = "v1"` with `/v1{path}`; a dated
  `api_version` selects the deployment URL form. xAI, Hugging Face and Azure
  default to the Responses API. **xAI and Hugging Face no longer have a Chat
  Completions model**: the AI SDK packages serve Responses only, so `XaiModel`
  and the Hugging Face chat model are removed (no `chat` / `chat_completions`
  method). To keep calling those endpoints, use
  `create_openai_compatible` with `https://api.x.ai/v1` or
  `https://router.huggingface.co/v1`; the xAI-specific chat response
  handling (citations, search parameters, 200-status errors) is not available
  on that path. The xAI Responses model now sends `top_k` and warns for
  `frequencyPenalty` and `presencePenalty`, as upstream does. The OpenAI default `language_model` and
  `call` use Responses, matching upstream; `chat` remains an explicit accessor. Codex is `codex.responses`, its ChatGPT-account base URL
  defaults to `https://chatgpt.com/backend-api/codex` (it was missing
  `/codex`), and `store: false` is a package rule. `open_responses`: `url` →
  `base_url`. The Azure deepseek / completion / MAI models and the Foundry item
  type are not ported.
- Single-modality vendors (28 packages) run async jobs on package constants:
  a fixed interval and attempt cap (previously unbounded loops now stop after
  6000 × 100 ms), a transient poll error spends one attempt, and the interval
  is overridden through the package namespace (`pollIntervalMs`, luma
  `pollIntervalMillis`) which replaces the old per-provider poll settings.
  `cartesia` `version` and `runwayml` `poll_interval` / `timeout` settings are
  removed (headers / constants); `google_pse` `cx` resolves settings, then
  providerOptions, then `GOOGLE_CSE_ID` per request; DataForSEO without
  credentials fails with `LoadApiKey`; searxng accepts an optional bearer
  token; the user-agent suffix is gone. `VideoModel::poll_config()` stays as
  the package-constant source.
- Removed from `aimux-providers`: `body_merge`, `openai_legacy`, `AzureAuth`,
  `TokenProvider`, `VertexAuth`, `BedrockAuth`, `StaticBearerConfig` and the
  `hmac` dependency (`sha2` / `hex` are dev-dependencies).
- Bedrock regions must be a single DNS label; invalid values fail the call
  with `InvalidArgument` before a request is sent.
- `ExternalProviderEntry` debug output reports only whether `api_key` is
  present, keeping literal keys out of debug logs.

**C ABI**

- New error codes `AIMUX_E_LOAD_API_KEY = 18` and `AIMUX_E_LOAD_SETTING = 19`
  (appended; retired 4 is not reused). `aimux_error_provider_code` returns the
  environment variable that was consulted and `aimux_error_provider_message`
  the key's description (`"OpenAI"`) or the setting name (`"region"`); no new
  symbol. `aimux_error_model_type` carries `NoSuchModel.model_type`. These two
  failures used to surface as `AIMUX_E_INVALID_ARGUMENT`.
- `aimux_provider_new`, `aimux_provider_handle_new` and `config_json` resolve
  names through the binding compatibility module (overlays, vendor packages
  and presets); `config_json` accepts
  `base_url`, `headers`, `organization`, `project` and `params`, and rejects
  `max_retries` and `body_overrides` with `AIMUX_E_INVALID_ARGUMENT`. A
  provider handle is provider + discovery. `aimux_register_providers`
  entries reject the same two keys.

**Node / Python**

- Node `ProviderConfig`: `maxRetries` and `bodyOverrides` are kept in the type
  only so that passing them throws `InvalidArgumentError`, for every native
  constructor (`openai`, `anthropic`,
  `google`, `cohere`, `mistral`, `xai`, `deepseek`, `bedrock`, `vertex`,
  `anthropicAws`, `azure`, `provider`, `createProvider`), not only OpenAI.
  `headers` now applies to all of them. New `params: Record<string, string>`
  fills a preset's template parameters. Python `config` /
  `config_json` accept `params` and reject `max_retries` / `body_overrides`
  the same way.
- New error classes `LoadAPIKeyError` (`envVar`, `description`) and
  `LoadSettingError` (`envVar`, `settingName`) in Node; `LoadAPIKeyError`
  (`env_var`, `description`) and `LoadSettingError` (`env_var`,
  `setting_name`) in Python. Both extend `AimuxError`, no longer
  `InvalidArgumentError`.
- Recorded and traced provider strings follow `"{name}.{method}"`
  (`openai.chat`, not `openai`).

**Go / Java / Kotlin / Swift / Dart**

- New error codes 18 / 19 (`CodeLoadAPIKey` / `CodeLoadSetting`,
  `LoadAPIKeyError` / `LoadSettingError`, `.loadApiKey` / `.loadSetting`),
  with the consulted environment variable as `EnvVar` / `envVar`.
- The call-level `body_overrides` / `bodyOverrides` field is deleted from
  `GenerateTextOptions` (the core no longer accepts the key; leaving it would
  have been silently ignored). The provider `config_json` helpers
  (`ProviderConfig` in Go and Dart) drop `max_retries` / `body_overrides` and
  gain `params`; call-level `max_retries` is unchanged.

## [0.5.0] - 2026-09-27

**Breaking release.** 13 PRs since 0.3.0: the cross-language error model
stabilized behind one opaque C-ABI error pointer, the request pipeline
aligned with the AI SDK (core-owned retries, timeouts, stream lifecycle,
video start/status split), tool inputs parsed and validated at the Core
boundary with host-side repair in every binding, WebSocket proxy support
plus ElevenLabs realtime transcription, and SSRF hardening for
provider-supplied download URLs.

### Breaking

**Rust (aimux-core / aimux-provider-utils)**

- Request pipeline aligned with the AI SDK (#164): retries, timeouts and
  the stream lifecycle moved from the HTTP layer into the user operations.
  `RetryError { maxRetriesExceeded | errorNotRetryable }` preserves the
  full attempt history; Full Jitter drives the exponential branch; server
  `retry-after` hints are honored exactly. Providers declare policy
  through `retry_config()`; Core executes it around
  `do_generate`/`do_stream`. `stream_text` now requires a tokio runtime
  (its pump task spawns unconditionally).
- `VideoModel` splits into `do_start`/`do_status` with a Core-owned poll
  loop; `generate_video` validates `n` (0 is `InvalidArgument` before any
  network call), batches by `max_videos_per_call` concurrently, and mints
  one idempotency key per batch — billable starts replay safely.
- Tool inputs are parsed and validated at the Core boundary (#165):
  providers deliver raw argument text; Core owns JSON parsing and
  JSON-Schema validation per the AI SDK `parseToolCall` contract. Invalid
  calls return as tool calls marked `invalid: true` with a typed error
  (`NoSuchTool` / `InvalidToolInput` / `ToolCallRepair`; 16-variant
  contract, compile-time exhaustive). `GenerateContent::ToolCall.input`
  is `String` (was `serde_json::Value`); a compatibility deserializer
  accepts the legacy object shape.
- The deferred-tool-call stream mode is removed
  (`to_chat_completion_stream_with_deferred_tool_calls`, the
  `defer_tool_calls` flag): the OpenAI stream always forwards provider
  tool-input deltas and does not reflect repair (#192).
- aimux-provider-utils: `send` / `send_timed` / `send_stream_timed` /
  `send_with_retry_raw` and the JSON-path `ErrorStructure` are replaced
  by single-exchange helpers (`post_json_to_api`, `post_form_data_to_api`,
  `post_to_api`, `get_from_api`) dispatching to typed response handlers —
  exactly one fetch attempt and one recorded exchange per call; retry,
  timeouts and backoff belong to Core (#164).

**C ABI**

- Error transport switched to one opaque `aimux_error_t *` (#158): every
  fallible function returns NULL on success (result in a trailing
  out-parameter) or one owned error, released exactly once with
  `aimux_error_free()`. The caller-allocated `AimuxError` struct,
  `aimux_error_clear` and the `error_value` projection are removed. One
  unified code space: `AiMuxError` 1–17 (4 retired slots, 14 = `Retry`),
  `RecordingError` 100–105, C-boundary failures 200–206; a non-NULL error
  never reports code 0.
- New error-context getters carry the pipeline's retry/timeout state
  (retryable flag, retry-ms hint, provider code/message, response body,
  sanitized url/request, response headers, provider data);
  `aimux_error_request_id` is removed — request ids ride in response
  headers (#164).
- Stateless tool-call repair (RFC-0035) (#192): three pure
  JSON-in/JSON-out entry points (`aimux_tool_call_repair_context`,
  `aimux_apply_tool_call_repair`,
  `aimux_apply_tool_call_repair_to_result`) let any host repair an
  invalid call in its own language; safe to call from inside a stream
  callback.

**Node / Python (native bindings)**

- Errors are thrown as native runtime exceptions (napi-rs canonical JS
  constructors / PyO3 exception classes), preserving `instanceof` across
  sync throws, promise rejections, streams and workers; the serialized
  `errorValue`/`error_value` companion is removed (#158).

**All eight bindings**

- Typed error context from the pipeline (retryable, retry_ms, provider
  code/message, request id, response body) and the `VideoPollOptions`
  surface (#164); recoverable stream-frame errors keep the stream alive
  (forwarded as data on the plain path, skipped on the chunk-typed
  OpenAI path).
- Host-side `repairToolCall` (RFC-0035) (#192): the binding runs the
  user's repair function in the host language and re-validates through
  the pure core functions; non-streaming results are patched before
  decoding, OpenAI-format outputs repaired then converted.

**Go**

- `Close` is no longer a join (#157): every native owner is an atomic
  handle; `Close` never waits for an in-flight C call, and owners must
  not be copied after first use (`go vet` flags it). Router/Moa
  constructors reject any invalid member handle instead of silently
  dropping it.

### Added

- **WebSocket proxy support** (RFC-0034 P1) (#183) — `ws_connect` honors
  the global `ProxyConfig` (HTTP CONNECT tunnel with Basic proxy auth,
  `no_proxy` suffix matching); SOCKS and https-scheme proxies fail loudly
  as `UnsupportedFunctionality`; wss tunnels run rustls with webpki-roots.
  Proxy credentials never reach error strings; IPv6 proxy hosts work.
- **ElevenLabs `scribe_v2_realtime` streaming transcription**
  (RFC-0034 P2) (#184) — realtime WS sessions with commit-on-last-chunk
  semantics, partial/committed transcript mapping, timestamped finals,
  and 14 documented error events classified retry/terminal; non-pcm/ulaw
  formats fail fast without connecting.
- **SSRF hardening for provider-supplied URLs** (#163) — every fetch of a
  URL taken from a response body or header (Black Forest Labs, Gladia,
  Luma, Recraft, Replicate, fal downloads, Google Files upload) is
  validated against AI SDK blocklists (http/https only, localhost/.local
  rejected, IPv4-embedded IPv6 forms caught), DNS-pinned to validated
  answers (defeats TTL-0 rebinding), follows redirects manually
  hop-by-hop, and sends caller headers only to strictly same-origin
  targets.

### Changed

- Provider error mappers match each provider's documented error format
  instead of assuming OpenAI's `{error:{message,code}}` (ElevenLabs
  FastAPI, AssemblyAI, Deepgram, fal, Hume) (#164).
- The shared HTTP client is keyed by tokio runtime with a capped cache —
  a finished runtime no longer strands dead pooled connections (#164).
- Recording distinguishes operation attempts from exchange indices,
  records Router/MoA children as steps, and widens sensitive-key
  redaction to camelCase/kebab-case tokens (#164).
- Stream timeouts are measured at the producer: a pump task owns the
  first-chunk/chunk deadlines, so a slow consumer no longer eats the
  provider's output budget (#164).
- Docs: the provider count has a single source of truth (generated
  totals in providers.md — 251 registry + 76 typed = 327), RFC statuses
  swept to match reality, and a contributing checklist for adding a
  provider (#193).

### Fixed

- **Go lifecycle deadlocks** (#157): read locks held across blocking C
  calls formed real wait cycles (`TranscriptionSession.Close` vs
  `NextPart(-1)`, a backpressured stream vs `Model.Close`); FFI
  Router/MoA silently dropped dead member handles mid-construction.
- **Release artifacts**: the console-build matrix entries sat at the
  wrong YAML level (GitHub silently dropped the jobs), so v0.3.0 shipped
  without aimux-web / aimux-cli / aimux-replay binaries; aimux-cli is now
  staged under its release name (`aimux`) (#155/#156).
- Invalid tool calls keep the provider's verbatim argument text on every
  path (failed repair, unknown tool, schema-rejected), and blank
  arguments render as `{}` on the OpenAI wire instead of invalid JSON
  (#165).
- Cohere streaming no longer parses tool-call arguments itself — a
  malformed streamed call reaches Core as an `invalid: true` call instead
  of aborting the stream (#165).
- xAI Responses streams end on a terminal in-stream error (previously
  hung against a server holding the connection open); negative
  `Retry-After` HTTP-date hints no longer read as "absent" through the
  C ABI (#164).
- Node/Python `transcribe` / `rerank` / `search` accept options
  (`max_retries`, `timeout` were unreachable), and
  `search("cats", {query: "dogs"})` no longer silently searches for dogs
  (#164).
- Java/Kotlin repair hooks run on their own thread with the stream's
  read hold lent, so a queued fair-lock `close()` no longer deadlocks the
  three threads (#192).
- Benchmark scripts run on machines other than the one they were written
  on (napi artifact resolved from platform/arch, SDKs from npm) (#160).
- **Flutter package size**: the iOS static-lib slices carried fat-LTO
  LLVM bitcode (~80% of each slice) plus debug symbols because a
  `staticlib` never goes through cargo's link step — the 0.5.0 xcframework
  hit pub.dev's 256MiB uncompressed package limit and was rejected. The
  iOS build now uses an LTO-off profile and strips debug/local symbols:
  each slice drops from ~128MB to ~15MB.

### Removed

- Sixteen one-shot migration scripts, the `fix_tool` crate, and unused
  workspace deps (schemars, proc-macro2, syn, quote) (#169).
- `aimux_error_request_id` (C ABI) — request ids ride in response
  headers (#164).

## [0.3.0] - 2026-08-17

**Breaking release.** 196 commits since 0.2.1: observability primitives
(recording / replay / sessions / tracing), composite models, streaming
transcription, a reworked error model across the C ABI and all eight
bindings, a browser console, and a large provider-correctness sweep.

### Breaking

**Rust (aimux-core / aimux-provider-utils)**

- `AiMuxError::RateLimited` gained `message: String` (now
  `RateLimited { retry_after_ms, message }`); `#[serde(default)]` keeps
  old payloads deserializable, but the generated TypeScript marks
  `message` required — update exhaustive destructuring.
- `GenerateTextOptions` / `CallOptions` / `StreamTextResult` gained public
  fields (`session_id`, recording/trace controls, stream-result metadata).
  Use struct-update (`..Default::default()`) instead of exhaustive
  initializers.
- `shared_client()` / `shared_streaming_client()` (aimux-provider-utils)
  now return `Result<&'static Client, AiMuxError>`: client-build failures
  (TLS backend, resource exhaustion) surface as a sticky, non-retryable
  `ApiCall` instead of aborting the host process (#147).
- Panicking convert wrappers are deprecated in favor of fallible variants.

**C ABI & the six FFI bindings (Kotlin / Java / Swift / Go / Flutter / C)**

- Error transport switched to an `AimuxError` out-parameter with typed
  code + HTTP status + retry hint; the JSON error envelope, the streaming
  `on_error` callback, and `aimux_last_error()` are removed.
- All six bindings restructured to the typed error model.
- **Kotlin**: `topK` is now `Double` (was `Long` — 40.5 truncated
  silently); published artifacts require **JDK 17** (`jvmToolchain(17)`).
- **Java**: `topK` likewise typed as double-valued (#106).
- `init_recording_ring(0)` now throws instead of a silent no-op
  (consistent across all seven languages).

**Python / Node (native bindings)**

- Streaming-transcription sessions surface in-stream errors by raising
  (`next_part`) / rejecting (`nextPart`) the typed hierarchy, and part
  payloads are no longer wrapped in a `{"Ok": ...}` envelope — both now
  match the C-ABI session shape and the other six bindings (#145/#150).
  Code that parsed the envelope manually must catch the exception.

### Added

- **Request recording & replay** (RFC-0023) — `Recorder` /
  `JsonlRecorder` plus a bounded in-memory `RingRecorder` with drop
  counting; layer-B HTTP choke-point recording (per-attempt, streaming,
  credential redaction); mock & request replay with matchers and the
  `aimux-replay` CLI; cross-binding exposure via C ABI, Python, Node, Go,
  Swift, Kotlin, Java, Flutter; `config_snapshot()` captures the minimal
  provider/model identity. `aimux_recording_try_flush` FFI export reports
  write failures (sticky first error) across the ABI (#133/#137).
- **Session grouping** (RFC-0024) — `session_id` groups related calls;
  `SessionStore` + `SessionInferer` with query APIs in all bindings;
  session-cache trajectory export.
- **Cache-hit tracing** (RFC-0015) — `TraceLayer`, verdict engine, and
  `RingTraceStore` detect prompt-cache hits from provider headers
  (vLLM / SGLang / LMCache behaviours), with cluster/route-aware gating;
  exposed in every binding.
- **`aimux-cli` cache-probe client** (RFC-0025) — offline / session /
  provider probing over the trace store.
- **OpenAI-compatible output** (RFC-0026) — `generateOpenAIOutput`
  across all eight bindings.
- **Model catalogue & listing** (RFC-0027) — `Provider::list_models` and
  `get_model_specs` with reasoning/capability metadata.
- **External provider config overlay** (RFC-0020) — register or override
  OpenAI-compatible providers from JSON at runtime.
- **Composite models** — drop-in `LanguageModel` wrappers, usable from every
  binding with zero call-site changes:
  - `RouterModel` (RFC-0021) routes each call to one child model through a
    pluggable `Router` strategy (built-ins: `RuleRouter`, `WeightedRouter`)
    with automatic fallback to the remaining children on failure.
  - `MoaModel` (RFC-0022) implements mixture-of-agents in a single call:
    reference models run in parallel, their outputs are spliced into an
    aggregator prompt, and the aggregated answer is returned — no agent
    loop involved.
- **Core API growth** — `streamText` aggregation, `generateObject`,
  top-level result aggregation (reasoning / sources / files /
  responseMessages), proxy configuration, `rawFinishReason`, logprobs,
  `usage.raw`, streaming warnings, `includeRawChunks`,
  `ResponseMetadata.timestamp`.
- **Streaming transcription** (RFC-0028) — realtime WebSocket sessions
  with push-audio / next-part / abort / first-chunk and idle timeouts,
  an FFI session API, and first-class support in all eight bindings.
- **`aimux-web` console** (RFC-0029) — browser-based model-call testing
  and trace visualization, shipped as release artifacts.
- **In-page API key settings for the console** (RFC-0029 revision) —
  set provider keys in the browser (Settings page): in-memory by default,
  opt-in disk persistence at `0600`, masked hints only, and plaintext
  entry is loopback-gated (non-loopback binds fall back to `env:VAR`
  references).
- **FFI/binding ergonomics** — default-capacity ring init, cancellable Go
  streams, `ProviderWithConfig` (Go), full `ProviderOptions` for
  `provider()` (Python), optional recording-ring capacity in every
  language.

### Fixed

**Provider correctness**

- Gemini: `functionResponse.name` now uses the tool name instead of the
  opaque call id (multi-turn tool calls no longer 400) (#127).
- Anthropic: in-stream errors now emit the terminal `Finish` part
  (contract parity with OpenAI/Google) (#128); assistant reasoning —
  including its signature — is echoed back when thinking is enabled
  (#131/#138).
- Bedrock & Anthropic: streaming no longer drops reasoning signatures;
  extended-thinking multi-turn keeps its context (#131).
- Vertex: grounding / url-context / code-execution / server-tool results
  are no longer silently dropped from streams; finish metadata restored
  (#141/#143).
- AWS SigV4 signs the host header with non-default ports; local gateways
  and proxied environments no longer fail (#125/#129).
- Six providers (openai/bedrock/cohere/mistral/huggingface/anthropic)
  stop discarding response fields they had already parsed (#101/#139).
- Recording: writer I/O failures (e.g. ENOSPC) surface through
  `try_flush` instead of a silent Ok, and the completion barrier requires
  the input record before finalizing (#110/#133).
- Structured `ApiCall`/`NoSuchProvider` fields with unified retry
  classification (#94); retry config honored for vertex/anthropic-aws;
  credential-source accuracy for xai/open-responses; real provider
  identity surfaced on the OpenAI chat path.
- Kotlin: the documented retryable timeout sentinel for `nextPart` is
  actually reachable (#116/#144); close-race read/write locks for handle
  types in Kotlin/Java/Flutter; Python exposes recording / mock-replay /
  `get_model_specs` with typed exceptions.

**Tests & fixtures**

- Cassette bodies are no longer blanked for every streaming response —
  157 recovered recordings across 640 files (#102).
- Contract fixtures now type-check (field *values*, not just names)
  across Rust and all eight bindings; the `top_k` drift that hid for
  months is regression-locked (#106).

### Engineering

- Quality gates: workspace fmt, clippy baseline plus a permanent
  5-lint subset (1,521 fixes, incl. 146 hand-written `# Errors` docs),
  `rustdoc -D warnings` in CI, ts-rs type-drift check, ProviderName
  generator-drift check.
- Coverage infrastructure (cargo-llvm-cov) with a 78.5% workspace
  baseline; unified e2e suite extended to six protocols; a 96-export FFI
  smoke harness; RFC-0028 error-path coverage in Rust, Python and Node.
- Round 4 quality audit: 16 verified findings, full reports under
  `docs/quality-audit/round4/`; unused-dependency sweep; rustdoc errors
  47 → 0.
- Release pipeline hardened from the 0.2.1 post-mortem (JVM Central
  Portal routing, napi rebuild before publish, Flutter xcframework
  embedding) plus a troubleshooting handbook.

### Removed

- `aimux_last_error()` (C ABI) — replaced by the `AimuxError` out-param.
- The C ABI JSON error envelope and the streaming `on_error` callback.

## [0.2.1] - 2026-08-04

Patch release following 0.2.0. First Maven Central (Java + Kotlin) and pub.dev
(Flutter) release; Rust / Node / Python re-released to stay aligned with the
new bindings.

## [0.2.0] - 2026-08-03

**Breaking release.** This version replaces the 250 per-provider shell types
with a single registry-backed `provider(name, ...)` factory, and adds request
cancellation + timeout control. See [Removed](#removed) for migration.

### Added

- **Unified `provider(name, ...)` factory** (RFC-0017 phase 4) — every one of
  the 250 OpenAI-compatible providers is now described in one
  `provider_registry.json` (base URL, API-key env var, per-vendor quirks) and
  constructed through a single entry point in **every binding**: Rust, Node,
  Python, Go, Kotlin, Swift, Java, Flutter, C.

  ```rust
  // before: remember a class per provider
  // let model = GroqProvider::new(GroqConfig::new(key)).model("llama-3.3-70b");

  // after: one factory — typed name, explicit key
  let model = provider(ProviderName::Groq, Some(key), "llama-3.3-70b", None)?;
  // or plain string; key falls back to the provider's env var
  let model = provider_from_env("groq", "llama-3.3-70b", None)?;
  ```

  ```ts
  // Node: same factory, typed ProviderName (lowercase keys)
  const model = await provider(ProviderName.groq, apiKey, 'llama-3.3-70b')
  ```

- **Typed `ProviderName` in 8 languages** — enum/union/const in Rust,
  TypeScript, Python, Go, Java, Kotlin, Swift, Flutter. Gives autocomplete and
  compile-time checking; plain strings still work everywhere.
- **Request cancellation** — Node: pass a standard `AbortSignal` as the 4th
  argument of `generateText` / `streamText`; Rust: `abort_signal` on
  `GenerateTextOptions`. Aborting cancels the in-flight HTTP request.
- **Timeout control** — new `timeout` option on `GenerateTextOptions` /
  `CallOptions` (works in all bindings): `total_ms` (whole call),
  `first_chunk_ms` (time to first token), `chunk_ms` (idle gap between stream
  chunks). Timeouts surface as `AiMuxError::Timeout` and are not retried.
- **Reasoning effort passes through verbatim** — the old `minimal→low` /
  `xhigh→max` normalization is gone; all 7 levels are sent as documented (e.g.
  Groq's effort values). Setting `reasoning` without an effort value now
  produces a warning instead of silently doing nothing.
- **Native protocol constructors in every binding** — Bedrock, Vertex,
  Azure, Cohere, Mistral, xAI and Anthropic-AWS now ship LLM constructors
  across the C ABI and all 8 bindings (previously Rust-only). Python's
  `google()` factory is now exported.
- **Correct `max_tokens` field per vendor** — providers that expect
  `max_completion_tokens` (Groq, Heroku) or `max_tokens` (Perplexity,
  SiliconFlow, StepFun, …) get the right key automatically.
- New design docs: RFC-0014 (logging), RFC-0015 (cache-hit audit & request
  tracing), RFC-0018 (Codex subscription channel), RFC-0019 (session affinity).

### Removed (breaking)

- **250 per-provider shell types retired** (`GroqConfig`, `DeepSeekProvider`,
  …) in Rust and all bindings — migrate to `provider(name, ...)` /
  `ProviderName` (examples above). The 10 native protocol providers (OpenAI,
  Anthropic, Google, Bedrock, Vertex, Azure, Cohere, Mistral, xAI,
  Anthropic-AWS) keep their existing types; DeepSeek is registry-backed.
- **`RequestBodyOverride`** and the `request_body_override` profile field
  removed — use `body_overrides` instead.
- **Reasoning-effort normalization** removed (values now pass through).

### Fixed

- **Streaming timeouts could silently never fire** — a pending deadline was
  dropped before it could trigger; streaming now enforces `total_ms` /
  `first_chunk_ms` / `chunk_ms` reliably.
- **7 wrong `base_url` entries in the provider registry** corrected (they
  would have failed at request time).
- Provider factory missing from some bindings' public surface (Python
  exports, Node npm package, Go DeepSeek).
- CI/test hygiene: formatting drift, clippy warnings, and tests that no
  longer depend on ambient environment variables.

### Changed

- All provider tests now go through the unified `provider()` entry; new test
  coverage for timeouts, cancellation, and registry wiring.

## [0.1.5] - 2026-08-01

### Added
- **Six desktop native targets for Node binding** — expanded from 2 to 6
  platform-specific Node-API packages: Windows x64/ARM64 (MSVC), macOS
  x64/ARM64, GNU/Linux x64/ARM64. The root `@arcships/aimux` package
  auto-selects the matching platform package at install time. Targets
  Node-API 8 (compatible with Node.js and Electron without rebuild).
  - Linux built against glibc 2.17 baseline (napi-cross).
  - Windows statically links MSVC CRT (no runtime dependency).
  - macOS deployment target set to 10.13.

### Changed
- CI/release matrices updated for all six Node binding targets.
- Node.js 24 + `npm ci` in binding workflows.
- `package-lock.json` tracked, `@napi-rs/cli` pinned to 3.8.2.
- AVA worker threads disabled (avoids napi-rs/Tokio teardown panics).

## [0.1.3] - 2026-08-01

### Added
- **`bodyOverrides` (JSON deep-merge)** — per-call and provider-level request
  body overrides. Objects merge recursively, scalars overwrite, `null` deletes
  keys. Applied after built-in vendor overrides; per-call overrides
  provider-level. Lets users inject vendor-specific fields (e.g.
  `enable_thinking`, `thinking_budget`) without closure bridging — critical
  for aimux's multi-language C ABI architecture where closures can't cross the
  JSON string boundary. (RFC-0017)
- **`maxRetries` (per-call)** — override the provider's retry count. `Some(0)`
  disables retries. Available on both `GenerateTextOptions` (per-call) and
  provider factory config (provider-level, Node only).
- **Provider factory config object (Node)** — `openai()`/`anthropic()`/
  `deepseek()` 3rd param now accepts `string | ProviderConfig` (backward
  compatible). `ProviderConfig` exposes `baseUrl`, `headers`, `organization`,
  `project`, `maxRetries`, `bodyOverrides`.
- **All 7 language typed wrappers** now expose `body_overrides` + `max_retries`
  on `GenerateTextOptions`: Node, Python, Go, Java, Kotlin, Swift, Flutter.
- RFC-0016 (Vercel AI SDK gap analysis) and RFC-0017 (provider config DX
  design).

### Fixed
- **`build_headers` now reads `config.headers` and `config.project`** —
  previously `OpenAIConfig.with_headers()` / `with_project()` set fields that
  `build_headers` silently ignored. Provider-level headers and project ID now
  reach the wire.
- **Anthropic factory now applies `bodyOverrides`** — was missing in the
  initial implementation (OpenAI/DeepSeek had it, Anthropic didn't).
- **`openai_compat` macro** — added `with_headers`/`with_retry_config`/
  `with_body_overrides` pass-through methods to all 251 OpenAI-compatible
  thin-wrapper providers (previously only `with_base_url` was exposed).

## [0.1.2] - 2026-08-01

### Fixed
- **`tool`-role `ContentPart[]` with the legacy `output` field is now accepted.**
  `ContentPart::ToolResult` renamed `output` → `result` in 0.1.1, but
  deserialization only accepted `result`, so multi-part `tool` messages built
  with `output` (the Vercel AI SDK / 0.1.0 TypeScript shape) were rejected with
  "data did not match any variant of untagged enum ModelPrompt". `result` now
  accepts `output` as a serde alias, so both shapes round-trip and
  `tool_call_id` reaches the OpenAI wire format. Serialization still emits
  `result`.
- **`reasoning` ContentPart is now replayed as `reasoning_content` on the
  request side.** Thinking models (e.g. DeepSeek `deepseek-v4-flash`) require
  prior assistant `reasoning_content` to be passed back on later turns,
  including tool-call turns; the OpenAI message converter previously dropped
  `ContentPart::Reasoning` parts, producing "The `reasoning_content` in the
  thinking mode must be passed back to the API." Reasoning parts are now lifted
  to a top-level `reasoning_content` string on assistant messages (mirroring
  the Vercel AI SDK `openai-compatible` assistant conversion), for both the
  tool-call and text paths. Groq's `reasoning` field name is unchanged.
- Regenerated the stale TypeScript bindings (`ToolResult.ts`, `ContentPart.ts`,
  `GenerateContent.ts`, `StreamPart.ts`, `ToolCall.ts`, `Usage.ts`) so the npm
  copy matches the Rust source of truth (`result` field, added fields).
- Made the `release.yml` crates.io idempotency checks read the version
  dynamically from `Cargo.toml` instead of a hardcoded `0.1.0` (which would
  have skipped the 0.1.2 publish).

### Changed
- Rewrote the top-level `README.md` in English with badges, architecture
  overview, and curated quickstart.
- Translated the public docs (`docs/`, `bindings/README.md`) and all RFCs to
  English.
- Moved internal research, audit, and handoff notes into `docs/internal/`.
- Removed committed Windows build artifacts (`.exe`/`.pdb`) and gitignored them.

### Fixed (prior)
- Corrected the `repository` URL in `Cargo.toml` (`yourusername` → `arcships`).
- Fixed the CI workflow trigger branch (`main` → `master`) so CI now runs on
  the actual default branch.

### Added
- `LICENSE` (MIT), `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, `SECURITY.md`.
- GitHub issue templates and a pull request template.

## [0.1.0] - 2026-07-31

### Added
- Core abstractions: `LanguageModel` trait (object-safe, `Box<dyn>` across
  providers), `Provider`, `Message`, `StreamPart`.
- 172 provider modules: 11 native protocol implementations (OpenAI, Anthropic,
  Google, Bedrock, Vertex, Azure, Cohere, Mistral, xAI, DeepSeek,
  Anthropic-AWS) + 145 OpenAI-compatible thin wrappers + 15 modality-specific
  + 1 generic Responses API wrapper.
- 8 modality traits: text, embedding, image, video, speech, transcription,
  reranking, search.
- `OpenAICompatProfile` descriptor capturing per-provider differences
  (top_k, tools, response_format, streaming usage, request-body post-processing).
- Streaming via SSE / NDJSON parsing with safe cancellation (`AbortSignal`).
- Request resilience: shared HTTP client, Full-Jitter backoff, timeout, error
  mapping with `error_type` + `status_code` passthrough.
- 2,650 cassette tests replaying real API responses — no network or keys needed.
- 7 language bindings sharing one Rust core:
  - Node.js (native, napi-rs v3)
  - Python (native, PyO3 + maturin)
  - Swift (C ABI, Swift Package)
  - Kotlin (C ABI, JNA)
  - Flutter (C ABI, dart:ffi)
  - Go (C ABI, cgo, static link, single binary)
  - C / C++ (C ABI, direct link)
- TypeScript type definitions auto-generated from Rust via `ts-rs` (79 types).
- Release profile optimized for binary size (`lto`, `codegen-units=1`,
  `panic="abort"`, `strip`, `opt-level="z"`).

### RFCs
- RFC-0001 Multi-language bindings
- RFC-0002 Provider improvements (config descriptor + thin wrappers)
- RFC-0003 Test cassette scheme
- RFC-0004 Provider inventory (172 providers)
- RFC-0005 Protocol conversion & adaptation layer
- RFC-0006 Provider development: minimum acceptance, core contract, tests
- RFC-0007 Search model trait
- RFC-0008 Multimodal bindings
- RFC-0009 Request resilience (shared client / jitter / timeout)
- RFC-0010 Performance benchmark vs Vercel AI SDK
- RFC-0011 Go bindings (cgo static link + push callback → channel streaming)
- RFC-0012 Source dedup (product source 68K → 51K lines, −25%)

[Unreleased]: https://github.com/arcships/aimux/compare/v0.5.0...HEAD
[0.5.0]: https://github.com/arcships/aimux/compare/v0.3.0...v0.5.0
[0.3.0]: https://github.com/arcships/aimux/compare/v0.2.1...v0.3.0
[0.2.1]: https://github.com/arcships/aimux/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/arcships/aimux/compare/v0.1.5...v0.2.0
[0.1.0]: https://github.com/arcships/aimux/releases/tag/v0.1.0
