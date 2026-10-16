# Changelog

All notable changes to aimux are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Breaking

- aimux's public JSON equals the AI SDK's JSON. Struct fields serialize
  camelCase (`toolCallId`, `providerOptions`, `inputTokens`); unions carry the
  AI SDK's tag and strings (`{"type":"text-delta"}`, `"tool-call"`,
  `"reasoning-file"`, Source `sourceType`, Warning/FileData/ResponseFormat/
  stream parts no longer externally tagged); bytes are base64 strings;
  optional fields are omitted instead of `null` and generate as `field?: T`;
  `AiMuxError` is tagged by `name`
  (`AI_APICallError`, `AI_RetryError`, ...). Vendor request and response types
  and the OpenAI chat-completion output keep their vendor wire formats.
  Persisted recordings and trace records change with the same rule;
  recordings stay schema 2 and still read the snake_case envelope keys
  aimux 0.5 writes (`call_id`, `recorded_at`, `model_id`, ...).

- Bytes are base64 strings in the public JSON, as the AI SDK writes a
  `Uint8Array`: image part `image`, file part `data`, `FileBytes`, speech
  `audio`, image outputs, video `binary` data and audio/image/video inputs. An
  array of numbers is no longer read. Bindings keep their byte-valued APIs
  (`Binary` variants, Python `bytes`) and encode them at the JSON boundary
  (#232 C33). Decision images (`images[].data` in a provider's options)
  follow the same rule; Python `decide` encodes `bytes` image data there
  (C33a).

- JSON `null` is kept apart from a missing value where the AI SDK's type is
  `unknown`: response and request `body` (language, speech, transcription,
  embedding, reranking and search results), `APICallError.data` and the tool
  approval request's `inputSchemaInput` read `"body": null` as a present null
  and write it back, and omit the field only when it was missing. Python
  serializes without `exclude_none` and keeps a set `None` of an `Any` field;
  Go reads these bodies as `json.RawMessage`, and `Error.Data` changes from
  `any` to `json.RawMessage` (`nil` when missing, `null` when null); Dart
  gains `hasBody` / `hasInputSchemaInput` (#232 C34).

- File content parts are the AI SDK's `FilePart`: one `file` tag whose `data`
  is a tagged `FileData` (`{type:"data"|"url"|"reference"|"text"}`), a string
  (base64 or a URL) or a bare `{provider: id}` reference. The `file-base64`,
  `file-url` and `file-reference` parts are removed; write
  `{type:"file", data:{type:"url", url}}` and so on. A `reasoning-file`
  part's `data` also takes a bare string. A `data:` URL's media type wins over
  the part's `mediaType`, and response messages carry files as bare base64
  strings. Bindings replace the three part classes with a typed `FilePartData`
  (#232 C35).

- Generated files are the AI SDK's `GeneratedFile`: `files[]` holds
  `{base64, mediaType, providerMetadata?}` instead of
  `{type:"data"|"url", ...}`, and the text results' and stream's `file` /
  `reasoning-file` parts are `{type, file: GeneratedFile, providerMetadata?}`.
  `ReasoningFileOutput` is replaced by `GeneratedFileOutput`; the
  provider-level `raw.content` keeps the V4 file
  (`LanguageModelFile`: `{mediaType, data}`). Bindings gain a `GeneratedFile`
  type with a bytes accessor (#232 C46).

- Response timestamps are written as JavaScript's `Date.toISOString()`
  (`"2025-03-06T13:50:19.000Z"`: UTC, milliseconds) instead of each
  provider's own form (`+00:00` offsets, nanoseconds, Bedrock's HTTP date,
  Open Responses' Unix seconds). Reading one requires an RFC 3339 date-time;
  other text is an error, so stored results or recordings with another form
  no longer parse. In Rust the fields are `Option<Timestamp>`
  (`aimux_core::Timestamp`) instead of `Option<String>`. Binding types stay
  strings (#232 C48).

- The text stream's `text-delta` and `reasoning-delta` parts carry their
  text under `text`, as the AI SDK's `TextStreamPart` does, instead of
  `delta`. `tool-input-delta` keeps `delta`, and so does the provider-level
  `StreamPart`. Rust matches `TextStreamPart::TextDelta { text, .. }`; the
  bindings' typed parts rename `delta` to `text` (Go `TextDeltaPayload.Text`,
  Java `getText()`, Kotlin/Swift/Dart `text`) (#229 B9).

- Provider options, provider metadata and provider references keep their
  keys in insertion order end to end, as the AI SDK's JS objects do: Rust
  holds them as `IndexMap` (see the provider options entry below) and the
  bindings no longer sort them. Go: `Tool.ProviderOptions` and
  `TranscriptionSessionOpts.ProviderOptions` are `json.RawMessage` instead of
  `map[string]json.RawMessage`, and `UploadFileResult.ProviderReference` is a
  `ProviderReference` (raw JSON, read with `Get(provider)`) instead of
  `map[string]string`. Java's `UploadFileResult` defaults to a
  `LinkedHashMap`. Swift's typed dictionaries remain unordered (#229 NP2-1).

- A tool call or source outside `content` carries its AI SDK `type` tag: the
  text results' `toolCalls[]` and an approval request's `toolCall` are
  `{"type": "tool-call", ...}` (the AI SDK's `TypedToolCall`), and their
  `sources[]` are `{"type": "source", "sourceType", ...}`. So are the repair
  context's `toolCall` and the call `apply_tool_call_repair` returns (a
  `LanguageModelV4ToolCall`). Reading accepts JSON without the tag and rejects
  a different one. Rust keeps `ToolCall` and `Source` untagged and writes the
  tag through `wire::typed` / `wire::typed_vec` / `wire::Tagged`; TypeScript
  types the fields as the new `TypedToolCall` and `TypedSource`. Every
  binding's standalone `ToolCall`, `RawToolCall` and `Source` writes the tag
  (Go through `MarshalJSON`; Java, Kotlin, Swift, Dart and Python as a fixed
  `type` field) (#229 NP2-7).

- Tool-call input parsed from the model's argument text writes an integral
  number as JavaScript does: `1e2` and `100.0` become `100`, `-0.0` becomes
  `0`, as `JSON.stringify(JSON.parse(text))` writes them, instead of
  serde_json's `100.0`. Other numbers are unchanged. Bindings that tell
  integers from floats (Python `int`, Go raw JSON, Jackson `IntNode`) now see
  integers (#229).

- The text results' usage, steps and typed tool outputs follow the same JSON
  rules. `LanguageModelUsage` (`usage`, `totalUsage`, each step's and the
  `finish` / `finish-step` parts' usage) is camelCase: `inputTokens`,
  `inputTokenDetails {noCacheTokens, cacheReadTokens, cacheWriteTokens}`,
  `outputTokens`, `outputTokenDetails {textTokens, reasoningTokens}`,
  `totalTokens`, `raw?`; an unknown count is omitted instead of `null`, the
  two detail objects are always written. `steps[]` are camelCase (`callId`,
  `stepNumber`, `model {provider, modelId}`, `rawFinishReason?`,
  `performance {stepTimeMs, responseTimeMs, timeToFirstOutputMs?, ...}`,
  `response {id, timestamp, modelId, headers?, body?, messages}`). Tool
  results, tool errors, `tool-output-denied` and `tool-approval-response`
  parts write `toolCallId`, `toolName`, `providerExecuted`, `approvalId`; an
  approval response's `toolCall` carries `type: "tool-call"`, and the
  results' `toolResults[]` are `{"type": "tool-result", ...}` (the AI SDK's
  `TypedToolResult`; TypeScript `TaggedToolResult`). Recordings write the
  outcome's usage the same way. Go, Java, Kotlin, Swift and Dart read and
  write the camelCase keys with unchanged field names; Python dicts and Node
  objects carry the new keys. Kotlin, Swift and Dart keep a parsed JSON
  `null` output apart from no output (Kotlin `JsonNull`, Swift `.null`, Dart
  `hasOutput`).

- `Output` (the `output` option of `generateText` / `streamText` and
  `generate_object`) is tagged by `type`, as the AI SDK names its output
  modes: `{"type": "text"}`, `{"type": "object", "schema", "name?",
  "description?"}`, `{"type": "array", "element", "minItems?", "maxItems?",
  ...}`, `{"type": "choice", "options"}`, `{"type": "json"}`. The externally
  tagged `{"Object": {...}}` form is no longer read.

- Decisions follow the same JSON rules. `DecideOptions` and
  `DecisionCallOptions` write `maxRetries` and `providerOptions`;
  `DecisionResult` and `DecideResult` write `rounding {probabilityDecimals,
  scoreDecimals}`, `providerMetadata` and `usage {inputTokens, outputTokens,
  totalTokens}`, an unknown count omitted instead of `null`
  (`decision-result.ts:27-31`). The model description of
  `aimux_decision_capabilities` and every binding's `describe` is
  `{specificationVersion, provider, modelId, supportedQuestionTypes}`. The
  options and metadata aimux defines in a provider's namespace are camelCase
  too: `images[].mediaType`, OpenAI `choiceValues` and `safetyIdentifier`,
  candidate scoring's `modelRevision`, `tokenizerRevision`,
  `promptFormatVersion` and `encoded.<id>.{promptTokenIds, labelTokenIds}`,
  and the metadata's `probabilitySource` and `latencyMs`; options a runtime
  receives as given keep the runtime's names (`keep_alive`,
  `chat_template_kwargs`, ...). Python's `decide` takes the JSON keys as
  keyword arguments (`maxRetries=0`). Decision recordings write the options
  and result this way, and replay refuses recorded options with snake_case
  keys instead of ignoring them.

- Request options reject keys they do not have (RFC-0042 §7.2):
  `GenerateTextOptions` and its `timeout`, the options of the other
  operations (embedding, speech, image, transcription, reranking, video and
  its `poll`, search, file upload, `decide`), the MoA config, and the router
  and transcription-session configs of the C ABI, Node and Python. aimux
  0.5's snake_case keys (`max_retries`, `timeout.total_ms`, `fail_mode`,
  `provider_name`) and typos fail naming the key instead of falling back to
  the default. The Node and Python router and transcription-session configs
  are camelCase, as the C ABI's are.

- Middleware wraps generation and streaming with callbacks for both operations,
  parameter transforms and provider/model/supported-URL overrides. Explicit
  model and provider ids take precedence over middleware overrides.
  `NoSuchProvider` carries `provider_id`, `model_id`, `model_type` and
  `available_providers`. Node and Python OpenAI constructors use Responses
  by default.

- Vendor packages follow their upstream counterparts more closely. Visible
  changes: Bedrock percent-encodes the model id in request URLs and writes
  provider metadata under both `amazonBedrock` and `bedrock`; OpenAI embeddings
  request `encoding_format: "float"` and no longer decode base64 vectors; the
  Google provider result no longer carries a model id unless the API returns
  one; an Anthropic mid-stream `error` event is forwarded as an error part
  without a synthetic finish; each package sends its own `user-agent` suffix;
  Mistral has no `transform_request_body` setting; the deprecated
  `text_embedding` and `text_embedding_model` provider methods are removed.

- `SigV4Fetch::new` takes a required `SigV4Signing`. `Aws4Fetch` signs like
  aws4fetch 1.0.20, the signer of the AI SDK's AWS packages: `user-agent`,
  `content-type` and its other `UNSIGNABLE_HEADERS` are sent but not signed,
  and no `x-amz-content-sha256` is sent. `AllHeaders` is the previous rule.
  Bedrock and Anthropic-AWS sign with `Aws4Fetch`, Polly with `AllHeaders`.
  Bedrock, Bedrock Mantle and Anthropic-AWS send the user-agents of
  `@ai-sdk/amazon-bedrock` 5.0.100 and `@ai-sdk/anthropic-aws` 2.0.63 (the
  package part is appended again by the transport).

- The `ai-sdk-provider-utils/<v>` part of a pinned package's user-agent names
  the provider-utils that the package's upstream `package.json` depends on, as
  the AI SDK sends it. That is 5.0.51 for Azure, Bedrock, Cohere, DeepSeek,
  Groq, Mistral, xAI and Google Vertex, and 5.0.53 for OpenAI,
  OpenAI-compatible, Open Responses, Anthropic, Anthropic-AWS and Google. A
  per-call `user-agent` keeps the package's part. The Vertex MaaS and xAI
  sub-providers send `ai-sdk-openai-compatible/3.0.59`, the version
  google-vertex depends on. Hugging Face sends `ai-sdk-provider-utils/5.0.53`
  and no package part, as `@ai-sdk/huggingface` 2.0.62 does (now pinned in
  `VERSIONS.json`). Other packages send `ai-sdk-provider-utils/5.0.53`.

- `to_chat_completion` reads `choices[].logprobs` from the calling
  provider's metadata namespace (Azure's `azure`, DeepSeek's `deepseek`)
  and returns it in the Chat Completions wire shape, `{ "content": [...],
  "refusal": null }`. It used to return the provider's raw token list. A
  Responses result's per-part lists become one list, and a token without
  `bytes` gets `"bytes": null`.

- Security: OpenAI image edits download URL files (`files` and `mask`) with
  `download_blob`, as `@ai-sdk/openai` does with `downloadBlob`. The download
  goes through the download guard, so a private-network, loopback or
  link-local URL is refused before any request, and it sends no headers. A
  failed download is a status-code error rather than a parsed OpenAI error.

- OpenAI chat `providerMetadata.openai.logprobs` holds the entries as the AI
  SDK's chat schema parses them, `{token, logprob, top_logprobs: [{token,
  logprob}]}`: `bytes` is dropped, so `to_chat_completion` returns it as
  `null`. A `null` `content` sets no `logprobs`.

- Anthropic-AWS follows `@ai-sdk/anthropic-aws` 2.0.63 settings: the region
  comes from the setting or `AWS_REGION` with no `us-east-1` default; the
  workspace id is required, from the setting or `ANTHROPIC_AWS_WORKSPACE_ID`;
  with no `auth`, a request uses `ANTHROPIC_AWS_API_KEY` when it is set and
  otherwise SigV4 with `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` and
  `AWS_SESSION_TOKEN`. All of them are read when a request is made.

- Anthropic gains the computer toolset of `@ai-sdk/anthropic` 4.0.71
  (`anthropic.computer_toolset_20260801`). A member call in a response (for
  example `left_click`) comes back as one call of the caller's toolset tool,
  with the member name as `action` in its input and
  `providerMetadata.anthropic.toolsetName`. Sent back as history, the call
  keeps its input as it is, `action` included; a call without a string
  `action` is still sent, named after the toolset, with a warning; the
  toolset name is read from the call's tools or from
  `providerOptions.anthropic.toolsetName` only.

- Anthropic checks the args of the provider-defined tools that
  `@ai-sdk/anthropic` 4.0.71 `prepareTools` validates (computer toolset,
  advisor, `text_editor_20250728`, the three `web_fetch` and three
  `web_search` versions): args that do not match fail the call with
  `InvalidArgument` before any request, on Anthropic, Vertex and Bedrock
  alike. `anthropic::prepare_tools::{prepare_tools,
  prepare_tools_with_provider}` return a `Result`.

- Public surface (`aimux-core`, `aimux-provider-utils`): removed
  `recording::init_recording_from_env`; made private the retry preparation
  helpers and default constants, `composite::{add_usage,
  build_aggregator_prompt, extract_text}`, `recording::new_call_id`,
  `util::rfc3339_now`, two session helpers, `fetch_error_to_ai_mux_error`,
  `extract_response_header_pairs`, the SigV4 signing internals, the logging
  internals and `DEFAULT_MAX_JSON_RESPONSE_SIZE`.

- Proxy settings belong to a transport instance, not to the process
  (`aimux-provider-utils`). `init_proxy` and its set-once global are
  removed; `ProxyConfig` moves to `fetch`. `ReqwestFetch::new(proxy)` is a
  transport with its own proxy and connection pool, given to a provider as
  its `fetch` setting, as the AI SDK leaves routing to `fetch`; the
  WebSocket counterpart is `ws::TungsteniteConnector::new(proxy)`, given as
  `web_socket`. `set_default_transport` replaces the default that providers
  without `fetch`, built-in WebSocket connects and download hops read per
  request. `PinnedFetch::new` takes the hop's `ProxyConfig`.
  `aimux_init_proxy` (C ABI unchanged) and the Node and Python `initProxy` /
  `init_proxy` now replace the default transport for later requests, every
  time they are called, instead of taking effect only before the first
  request.

- Removed items the AI SDK has no counterpart for: `aimux_core::ModelId`
  (and `model_id`, the Node `ModelId` type), the hidden
  `aimux_provider_utils::download_guard` re-export (use
  `aimux_core::download_guard`; `same_origin` stays re-exported), the
  deprecated panicking converters `convert_prompt_to_anthropic`,
  `convert_prompt_to_anthropic_full`, `convert_prompt_to_openai_messages`
  and `convert_prompt_to_openai_messages_with_mode` (use the `_fallible`
  forms), and the `ResponsesSystemMessageMode` alias (use
  `SystemMessageMode`).

- `aimux_provider_utils::create_standard_json_error_response_handler` is
  removed. Failed-response handlers live in the provider modules, as the AI
  SDK keeps `groqFailedResponseHandler` and DeepSeek's error schema in their
  packages; build one with `create_json_error_response_handler`. The Groq
  handler now reads only `error.type` as the provider code, and DeepSeek's
  prefers `error.code` (a string or a number) over `error.type`.

- `LanguageModel::supported_urls` and
  `LanguageModelMiddleware::override_supported_urls` are async, as the AI
  SDK's `supportedUrls` and `overrideSupportedUrls` may be a `PromiseLike`.
  A wrapped model reads the override, else the inner model, when asked
  instead of once at wrap time. The OpenAI-compatible `SupportedUrlsFn`
  setting returns a `BoxFuture<'static, SupportedUrls>`.

- The user operations download as the AI SDK's do, through a transport the
  caller can replace (`aimux-core`, new public `download` module).
  `generate_text` and `stream_text` read the model's `supported_urls()`
  before the call and download each prompt file URL it does not cover (user
  `FileUrl` parts and `file` items of tool outputs; not `data:` URLs), which
  then reaches the provider as inline data; they used to pass every URL
  through. The Cohere chat, Open Responses and xAI Responses models declare
  the URLs their packages declare (HTTP(S) images; for xAI also PDF and
  text), so those still reach the API as URLs. Codex models declare the
  OpenAI Responses model's URLs (HTTP(S) images and PDFs) on both channels,
  and the Hugging Face Responses model declares HTTP(S) images, as
  `@ai-sdk/huggingface` does.
  `GenerateTextOptions::download` takes a `DownloadFunction` (the AI SDK's
  `experimental_download`) instead. `AudioInput::Url` is new:
  `transcribe` downloads it with `TranscriptionCallOptions::download`, and a
  transcription model given one returns `AudioInput::url_error()`.
  `generate_video` downloads every video returned as a URL with
  `VideoCallOptions::download` and returns it as `VideoData::Binary` (it
  returned the URL before). Without a download each operation uses
  `create_download(None)`, the guarded download (public HTTP(S) URLs only,
  pinned hops, no provider credentials, 2 GiB limit), so a Vertex `gs://`
  video result needs an injected download.

- Provider results carry request and response information as the AI SDK does
  (`aimux-core`). `GenerateResult` has `request: Option<RequestInfo>` and
  `response: Option<ResponseInfo>`; `ResponseInfo` gains `id` and holds the
  timestamp, model id, headers and body. `StreamResult` has `request` and
  `response: Option<StreamResponseInfo>`. The flat `request_body` and
  `response_headers` fields are removed and `response` is optional; the call
  layer fills a missing id, timestamp and model id. `GenerateTextResult`
  exposes `request: RequestInfo` and `response: ResponseInfo`, including
  headers and body.

- The prompt a provider receives is modelled by role (`aimux-core`), as the AI
  SDK's `LanguageModelV4Message`. `LanguageModelPrompt` is
  `Vec<LanguageModelMessage>`: `System { content: String }`,
  `User { content: Vec<UserPart> }`, `Assistant { content: Vec<AssistantPart> }`,
  `Tool { content: Vec<ToolPart> }`. The parts are `TextPart`, `FilePart`,
  `ReasoningPart`, `ToolCallPart`, `ToolResultPart`. A provider sees one file
  part, `FilePart { data: FileData, media_type, filename }`; the five
  user-facing file variants are folded into it by
  `convert_to_language_model_prompt`, which now returns `Result` and rejects a
  part its role does not allow. `LanguageModelPromptMessage` is removed.
  Recordings store the new prompt JSON: a system message's `content` is a
  string and every file input is a `file` part. In TypeScript the provider
  `ToolResultPart` / `ToolResultOutput` are `LanguageModelToolResultPart` /
  `LanguageModelToolResultOutput`; the plain names are the user-facing types.

- The user-facing `ModelMessage` is a union by role, as the AI SDK's
  (`aimux-core`): `System { content: String }`, `User { content: UserContent }`,
  `Assistant { content: AssistantContent }`, `Tool { content: ToolContent }`,
  each with message-level `provider_options`, which now reach the provider
  prompt (they were dropped). Each role has its own part union:
  `UserContentPart` (text, image and the file variants),
  `AssistantContentPart` (text, custom, the file variants, reasoning,
  reasoning file, tool call, tool result, tool approval request) and
  `ToolContentPart` (tool result, tool approval response). `ContentPart`,
  `MessageContent` and `Role` are removed; `ModelMessage::role()` returns the
  role name. A tool result is `ToolResultPart { tool_call_id, tool_name,
  output: ToolResultOutput, provider_options }`, the AI SDK's typed output
  (`text`, `json`, `execution-denied`, `error-text`, `error-json`, `content`);
  `preliminary` and `dynamic` are gone from it. A tool message can carry a
  `tool_approval_response { approval_id, approved, reason, provider_executed }`,
  which reaches the provider only for a provider-executed tool, as upstream;
  a tool message left empty is dropped. Wire format: the role and part tags
  are unchanged; a tool result carries only `output`, an AI SDK output
  object, and the earlier `result` / `is_error` fields are no longer read or
  written; a part its role does not allow (an `image` in an assistant message, a
  non-text system message, a string or text part in a tool message) is now
  rejected when the prompt is parsed instead of when it is converted.

- Provider options and provider metadata have one typed shape, the AI SDK's
  `Record<string, JSONObject>`: `SharedProviderOptions` and
  `SharedProviderMetadata` (`ProviderMetadata` is an alias) are one type,
  `NamespaceMap` = `IndexMap<String, JsonObject>`, namespace to JSON object.
  Like the AI SDK's JS objects they keep insertion order: a value serializes
  with its keys in the order it was built or parsed.
  `SharedProviderReference` is `IndexMap<String, String>` for the same
  reason. They replace `serde_json::Value`, `HashMap<String, Value>` and the
  per-part `Option<Value>`. Every `provider_options` / `provider_metadata` field of
  content parts, messages, call options, tools, results and stream parts uses
  them. `{"namespace": {"key": value}}` serializes as before; a value that is
  not namespace -> object (a number, string or array under a namespace, or a
  non-object at the top) is rejected when deserialized. Build one with
  `provider_namespace(ns, json!({..}))`.

- `ContentPart::Reasoning` has no `signature` field any more, as the AI SDK's
  `ReasoningPart` (`text` and `provider_options` only). A reasoning signature
  travels in its provider's namespace (`provider_options.anthropic.signature`,
  `provider_options.bedrock.signature`), which is where the providers already
  read it; response messages no longer copy it to a top-level field. A
  `signature` key in input JSON is ignored.

- `convert_to_language_model_prompt` takes the called provider's id as a third
  argument (`aimux-core`), the part of `LanguageModel::provider()` before the
  first `.`, as the AI SDK passes it. A `content` tool output accepts the AI
  SDK's deprecated items `file-data`, `file-url`, `file-id`,
  `file-reference`, `image-data`, `image-url`, `image-file-id` and
  `image-file-reference` (`ToolResultContentPart`; a `file_id` is a string or
  a provider-to-id map, `ToolResultFileId`), and the conversion turns each
  into a `file` part as upstream does: a `file-url` without a media type gets
  one from the URL's extension, a URL keeps its given spelling as
  `original_url` when normalizing changes it, and a string `file_id` becomes
  a reference for the called provider (an error when there is none). These
  items and a user `image` part log upstream's deprecation warning through
  `tracing`.

- `generate_text` and `stream_text` check the prompt and the response as the
  AI SDK does (`aimux-core`). An empty message list is an `InvalidPrompt`
  error (`messages must not be empty`). The prompt conversion combines
  consecutive tool messages into one, merging the earlier message's
  `provider_options` into its last part, and fails with the new
  `AiMuxError::MissingToolResults { tool_call_ids }` when a tool call the
  provider did not execute has no tool result or approval response before
  the next user or system message or by the end of the prompt. A response
  that breaks a `required` or `tool` tool choice fails `generate_text` with
  the new `AiMuxError::ToolChoiceViolation(ToolChoiceViolationError {
  tool_choice, finish_reason, provider, model_id, content })`; `stream_text`
  ends with that error as an `Error` part followed by a `Finish` part whose
  unified finish reason is `error`. Error codes: the C ABI reports
  `MissingToolResults` as `AIMUX_E_INVALID_PROMPT` and `ToolChoiceViolation`
  as `AIMUX_E_INVALID_RESPONSE_DATA`; Node and Python raise them as
  `InvalidPromptError` and `InvalidResponseDataError`.

- Text results carry the AI SDK's core `ContentPart` (`aimux-core`), eleven
  kinds, instead of the provider's `GenerateContent`: `StepResult.content`
  (and so `GenerateTextResult.content`) is `Vec<ContentPart>`
  (`TextContent` is removed; Node exports `ContentPart`, Python keeps the
  `TextContent` model name). New kinds: `ToolError(TypedToolError)` and
  `ToolApprovalResponse(ToolApprovalResponseOutput)`. A provider tool result
  is `ToolResult(TypedToolResult { tool_call_id, tool_name, input, output,
  provider_executed, provider_metadata, dynamic, preliminary })`, with the
  input of the call it answers, or `ToolError(TypedToolError { .., error,
  .. })` when the provider marks it as an error. A tool call that could not
  be parsed is followed by a `ToolError` holding its error message, which
  the response messages replay in a tool message as error text. Wire
  format: `{"File": {"file": GeneratedFile, "provider_metadata": ..}}` (was
  the bare `GeneratedFile`), `{"ToolResult": {.., "input", "output",
  "provider_executed"}}` (was `result` / `is_error`), and the new
  `ToolError` / `ToolApprovalResponse` tags. `GenerateContent` is no longer
  generic.

- The custom content `kind` is a `types::NamespacedKind` at every layer
  (`AssistantContentPart::Custom`, the provider `CustomPart`,
  `GenerateContent::Custom`, `ContentPart::Custom`, `StreamPart::Custom`),
  the AI SDK's `${string}.${string}`: a kind without a `.` (for example
  `"compaction"` instead of `"openai.compaction"`) no longer deserializes.
  The JSON is still a plain string; the TypeScript type is `NamespacedKind`.

- Tool-call and tool-result types are unified, one per protocol layer
  (`aimux-core`). A provider emits `tool::RawToolCall` (`input` is the raw
  argument text) in `GenerateContent::ToolCall(..)` and
  `StreamPart::ToolCall(..)`, and `tool::ToolResult` in the `ToolResult`
  variants; these variants are newtype variants now. A provider's `do_stream`
  yields `StreamPart` with raw tool-call input. `stream_text` and everything
  after it yield `TextStreamPart` with parsed input and `invalid` / `error`.
  Call-layer streams consume provider response-metadata events internally;
  `TextStreamPart` has no `ResponseMetadata` variant. `RawToolCall` moved from `parse_tool_call` to `tool`. Wire format:
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

- A repaired tool call that still fails to parse or validate is reported as
  the AI SDK reports it: the original call stays invalid and carries the
  replacement's own `InvalidToolInput` / `NoSuchTool` error, no longer wrapped
  in `ToolCallRepair` (only a failing repair function is wrapped). This holds
  for the `repair_tool_call` closure and for a host's `repaired` reply in every
  binding.

- `CallOptions.tool_choice` is the V4 object union
  `LanguageModelV4ToolChoice` (`{"type":"auto"}`, `{"type":"none"}`,
  `{"type":"required"}`, `{"type":"tool","toolName":..}`); the user-level
  `ToolChoice` (`"auto"` | `"none"` | `"required"` | `{type:"tool",toolName}`)
  stays on `GenerateTextOptions`. `prepare_tool_choice` converts one into the
  other as the AI SDK's `prepareToolChoice` does, and `generate_text` /
  `stream_text` now send `{"type":"auto"}` when no choice is given, with or
  without tools (providers still drop it when there are no tools, except
  open-responses, which sends it as upstream does). Provider authors match
  on `LanguageModelV4ToolChoice`. Recordings store the object form, so an
  `ExactMatcher` no longer matches recordings made before this change.

- `CallOptions.call_id` and `CallOptions::for_step` are removed
  (`aimux-core`); the V4 call options have no call id. The recording
  correlation id is the recording context's `call_id`, and `HttpRequest::new`
  takes it from there. Provider code that built `HttpRequest { call_id:
  options.call_id.clone(), .. }` uses `HttpRequest::new(url, headers,
  options)`. The JSON is unchanged (both were internal and never serialized).

- `CallOptions.max_retries`, `CallOptions.timeout` and
  `CallOptions.session_id` are removed (`aimux-core`), as in the V4 call
  options. They are settings of the `generate_text` / `stream_text`
  operation: `GenerateTextOptions` keeps all three, and the JSON bindings
  are unchanged. The operation runs its model calls inside
  `operation::OperationSettings::scope`. The router and MoA take the child
  retry count from `OperationSettings::current()`, and `TraceLayer` takes
  the session id from it. Code that calls `do_generate` / `do_stream` on a
  router, MoA or trace layer directly wraps the call in
  `OperationSettings { .. }.scope(..)`. Recordings keep the three keys in
  `input.options`, and replay restores them.

- A provider tool result without its call is accepted only for a provider
  tool that sets `supportsDeferredResults: true` (new optional
  `ProviderTool.supports_deferred_results`), as in the AI SDK
  (`convert-language-model-content.ts:96-103`); before, any provider tool of
  that name was enough. The flag is not sent to the provider.

- `CallOptions.recording_context` and `DecisionCallOptions.recording_context`
  are removed (`aimux-core`): the V4 call options carry no recording context.
  `generate_text`, `stream_text` and `decide` run the model call in
  `recording::RecordingContext::scope`, and the stream in
  `RecordingContext::scope_stream`; the router and MoA scope each child step
  in a child context. `HttpRequest::new` and the trace layer read
  `RecordingContext::current()`. Code that set the field on call options
  wraps the call in `RecordingContext::scope(Some(context), ..)`. The JSON is
  unchanged (the field was never serialized).

- `ProviderTool.id` is a `types::NamespacedKind`, the V4
  `${string}.${string}` identifier: an id without a `.` (for example
  `"web_search"` instead of `"openai.web_search"`) no longer deserializes,
  and `NamespacedKind::new` / `parse` check it when built in Rust. The JSON
  is still a plain string.

- A provider-executed `tool::ToolResult` whose `result` is `null` no longer
  deserializes: the V4 `result` is `NonNullable<JSONValue>`.

- Removed the generated `ProviderName` type in every binding (Rust enum, TS
  const object, Go/Java/Kotlin consts, Swift enum, Dart consts, Python
  `Literal`) and `scripts/gen_provider_names.py`. Provider names are plain
  strings: Rust uses `create_provider("groq", PresetSettings)`; bindings retain
  `provider("groq", ...)`, including overlay-registered names. The provider list lives in
  [docs/api/providers.md](docs/api/providers.md); Rust also gains
  `provider_names()`.

- `RetryError`'s text names the last attempt's message as the AI SDK's
  `getErrorMessage` does: `Failed after 3 attempts. Last error: Too Many
  Requests`, not `... Last error: API call error: HTTP 429: Too Many
  Requests`. The HTTP status stays a field of the attempt error.

- Video poll settings belong to the operation, as the AI SDK's
  `generateVideo` `poll` option (`aimux-core`, `aimux-providers`).
  `VideoModel::poll_config()` and `VideoPollConfig` are removed; unset
  `VideoCallOptions::poll` fields use the AI SDK defaults (5000 ms interval,
  600000 ms timeout). RunwayML no longer paces its own polling (it used
  2 s / 300 s, first as settings, then as package constants); pass `poll` per
  call instead. Each status check, its retries included, now races
  the time left in the poll budget and a hanging check fails with `Timeout`
  "Video generation timed out after {timeoutMs}ms." as upstream.

- Call-level usage and finish reasons follow the AI SDK (`aimux-core`). The
  results of `generate_text` / `stream_text` / `generate_object` and the
  `TextStreamPart` finish parts report usage as `LanguageModelUsage`
  (`input_tokens`, `input_token_details { no_cache_tokens,
  cache_read_tokens, cache_write_tokens }`, `output_tokens`,
  `output_token_details { text_tokens, reasoning_tokens }`, `total_tokens`,
  `raw`) and the finish reason as the unified string (`"stop"`,
  `"tool-calls"`, ...) with `raw_finish_reason` beside it. Usage is summed
  over steps with `add_language_model_usage`. Provider results
  (`GenerateResult`, `StreamPart`) keep `Usage` and `FinishReason`.
  Recordings store the call-level usage for both the generate and the
  stream path.

- `GenerateTextResult` is built from steps, as the AI SDK's
  (`aimux-core`). It holds `steps: Vec<StepResult>`, the summed usage and the
  parsed output; everything else is an accessor: `final_step()`, `content()`,
  `text()`, `files()`, `sources()`, `tool_calls()`, `static_tool_calls()`,
  `dynamic_tool_calls()`, `tool_results()` (and the static/dynamic
  variants), `finish_reason()`, `raw_finish_reason()`, `usage()`,
  `warnings()`, `response_messages()`, `output()`, and the deprecated
  `reasoning()`, `reasoning_text()`, `total_usage()`, `request()`,
  `response()`, `provider_metadata()` that the AI SDK also deprecates.
  `StepResult` carries the step's call id, model, content, finish reason,
  usage, performance, warnings, request, response (with its messages) and
  provider metadata. The `raw` provider result is removed, and
  `reasoning_text` is `None` when there is no reasoning. The JSON form is a
  snapshot: it writes the derived values next to `steps`, `total_usage` and
  `output`, and only those three are read back.

- Structured output is the `output` option, as the AI SDK's `Output`
  (`aimux-core`). `GenerateTextOptions::response_format` is replaced by
  `output: Option<Output>` (`Text`, `Object`, `Array`, `Choice`, `Json`); the
  parsed and schema-checked value is `GenerateTextResult::output()`.
  `generate_object` is deprecated, as upstream, and is a wrapper over
  `generate_text` with `Output::Object`; its `GenerateObjectResult` has
  `object`, `reasoning`, `finish_reason`, `usage`, `warnings`, `request`,
  `response` (`GenerateObjectResponse`: id, timestamp, model id, headers,
  body) and `provider_metadata`, and no `raw`. It no longer repairs
  malformed JSON before parsing.

- `TextStreamPart` is the AI SDK's call-level stream part, its own enum and
  no longer `StreamPart<ToolCall>` (`aimux-core`). The stream opens with
  `Start` and `StartStep { request, warnings }` and closes with
  `FinishStep { response, usage, performance, finish_reason,
  raw_finish_reason, provider_metadata }` and
  `Finish { finish_reason, raw_finish_reason, total_usage }`; the provider's
  `StreamStart` and `ResponseMetadata` parts are not forwarded. `TextDelta`
  and `ReasoningDelta` carry `text` (was `delta`). Aborting the call ends the
  stream with `Abort { reason }`. `StreamTextResult::consume()` returns a
  `GenerateTextResult`; `StreamTextResultAggregated` is removed.

- `stream_text` drops an empty `TextDelta` that carries no provider
  metadata, as the AI SDK does (`aimux-core`); providers emit one, e.g. the
  opening `"content": ""` chunk of OpenAI chat.

- `TextStreamPart` carries the AI SDK's tool parts (`aimux-core`,
  stream-language-model-call.ts:760-792). `ToolResult` is a
  `TypedToolResult` (`tool_call_id`, `tool_name`, `input?` from the call it
  answers, `output`, `provider_executed`, `provider_metadata?`, `dynamic?`,
  `preliminary?`) instead of the provider's `ToolResult`; a provider result
  marked `is_error` arrives as `ToolError` (`TypedToolError`, with
  `error`), and an invalid client tool call is followed by a `ToolError`
  holding its error message and `dynamic: true`
  (stream-language-model-call.ts:709-725). `ToolOutputDenied` and
  `ToolApprovalResponse` are added to complete the AI SDK's set; aimux's
  stream does not produce them yet. Unlike upstream, the provider's `preliminary` flag is kept, so
  a preliminary result stays out of the step content, as on the generated
  path. A consumed stream's step content records the tool results, tool
  errors and approval responses as they arrive (stream-text.ts:1529-1538,
  :1648-1650), so an invalid call's tool error is inline after its call
  there, where `generate_text` puts it after the content, as upstream does.

- `generate_text` fails with `Tool call {id} not found.` when the model
  returns a provider tool result whose call is not in the response and no
  provider tool of that name was passed (`aimux-core`,
  convert-language-model-content.ts:87-104). A result for a passed provider
  tool is kept as a deferred result. The stream passes such results through,
  without `input`, as upstream does.

- The response messages write a tool approval request's
  `input_schema_input` only when it differs from the call's input under the
  AI SDK's `isDeepEqualData` (`aimux-core`, to-response-messages.ts:133-146).

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
  The response handler propagates terminal SSE decoding failures as
  non-retryable `ApiCall` errors, preventing a normal finish or tool-call
  finalization after truncated input. Invalid JSON inside an otherwise valid
  SSE event remains a recoverable `JsonParse` error.
  `SseStream` requires the body error type to implement `std::error::Error +
  Send + Sync + 'static`.
- Removed `NdjsonStream` / `NdjsonError`: nothing in the workspace used them
  and the AI SDK has no counterpart. `tokio` is now a dev-dependency only and
  the unused direct `serde` dependency is dropped.
- `StreamingToolCallTracker`, `ToolCallStreamPart` and `TrackerError` are no
  longer exported from `aimux-stream`; nothing in the workspace used them and
  the AI SDK keeps the tracker in `@ai-sdk/provider-utils`, next to the
  provider code that drives it. See `aimux-provider-utils` below.

**Rust (aimux-provider-utils)**

- Gains `StreamingToolCallTracker` (plus `StreamingToolCallDelta`,
  `TypeValidation` and `TrackerError`), a port of the current
  `@ai-sdk/provider-utils` tracker, exported from the crate root only: the
  `streaming_tool_call_tracker` and `streaming_tool_call_argument_state`
  modules are private, and the argument state is internal, as upstream
  exports only the tracker from its index. The previous `aimux-stream` version was
  a port of an older, index-only tracker. Deltas are correlated by wire
  `id`, `index` and function name (ambiguous deltas are dropped), ids are
  de-duplicated with `-N` suffixes, generated ids come from the new
  `generate_id` (upstream's default `generateId`; `tool-call` only when a
  generator returns a blank id), blank function names are ignored, and
  `finish` orders calls by index only when every call has one. It emits
  `aimux_core::StreamPart` tool-input parts directly (`ToolInputStart` /
  `ToolInputDelta` / `ToolInputEnd` / `ToolCall`); there is no separate
  `ToolCallStreamPart` event type. `StreamingToolCallDelta` borrows from the
  provider's wire type and flattens upstream's `function` object;
  `process` and `finish` append to a caller-owned `Vec<StreamPart>`, and
  `finish` consumes the tracker. The upstream `extractMetadata` /
  `buildToolCallProviderMetadata` hooks are replaced by
  `StreamingToolCallDelta::provider_metadata`, read from the delta that
  starts a call. `TrackerError` converts into
  `AiMuxError::InvalidResponseData`.
- The tracker follows `@ai-sdk/provider-utils` 5.0.55 (pinned in
  `fixtures/aisdk/VERSIONS.json`): a named delta with an unseen id and empty
  arguments continues an incomplete matching call, but starts a new call
  once that call's structured arguments are complete; a continuation without
  labels goes to the only matching call whose structured arguments are not
  complete. Parallel same-name calls whose ids are missing or reused are no
  longer merged into the first call.

**Rust (aimux-providers)**

- The OpenAI chat-completions stream (`openai/model.rs`, which serves every
  registry-backed provider) correlates `tool_calls` deltas with the tracker
  instead of by `index` alone: deltas are matched by wire id, index and
  function name; a continuation without an id follows its call; indices
  reused across parallel calls stay distinct; ambiguous deltas are dropped;
  a call whose delta carries no id gets a generated id, as upstream, instead
  of an empty string; a new call without a function name ends the stream with
  `InvalidResponseData` (previously it started a call with an empty name).
  `DeltaToolCall.index` is now `Option<usize>`.
- Once compatible servers moved to `openai_compatible`, the OpenAI (and
  Azure) chat stream drives the tracker with `@ai-sdk/openai`'s options:
  `type` is validated when present, ids missing from the wire come from the
  provider's `generate_id`, and `DeltaToolCall.index` is required again, so a
  chunk without it fails to parse.

**Rust (provider factory, RFC-0036)**

- The native Decision/Jev extension from master is retained. Its typed
  `JevConfig` / `JevProvider::new` entry point and `Provider::decision_model`
  remain available. Decision replay now takes the caller's registry;
  recordings contain provider identity only and no capability snapshot
  (see the DecisionModelV4 entry). Decision retries use the core call-level
  policy.

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
  An ordinary API call follows every redirect, as the AI SDK's `postToApi`
  does under fetch's default `redirect: "follow"`. A hop that leaves the
  request's origin keeps only `User-Agent`, the rule the AI SDK's
  `fetchWithValidatedRedirects` applies on the server, so no credential
  header (`authorization`, `x-api-key`, `x-goog-api-key`, `api-key`, ...)
  reaches another origin, and dropped headers stay dropped if a later hop
  returns. `FetchRequest.credentials` (`RequestCredentials::Include` /
  `Omit`, after the fetch `credentials` member) is `Omit` on such a hop, and
  `SigV4Fetch` then sends it unsigned, without `x-amz-security-token`. Every
  followed hop goes through the request's transport again, so SigV4 signs
  the URL it sends. A `Fetch` never follows a redirect itself:
  `RedirectPolicy`, `FetchRequest.redirect` and `PinnedFetch::unpinned` are
  removed. Validated downloads keep their hop-by-hop guard.
- `Provider` follows the AI SDK's `ProviderV4`: `language_model`,
  `embedding_model` and `image_model` are required and return
  `NoSuchModel { model_id, model_type }` when a vendor has no such modality;
  `transcription_model`, `speech_model`, `reranking_model` and `files` are
  optional (`None` = not offered); video and search stay aimux extensions.
  `Provider` gains `specification_version()`, returning `"v4"` like
  `ProviderV4` and the model traits.
  AI SDK model constructors return an `Arc<dyn …>`. Removed: `Provider::name()`,
  `LanguageModel::config_snapshot()` and `list_models` on `Provider`.
  Discovery is the separate `ProviderDiscovery` trait, accessed through
  `Provider::discovery()`; `supported_urls()` is added to `LanguageModel`.
- AI SDK-aligned packages remove `XxxConfig`, config builders, `from_env()`,
  `with_*` methods and `XxxProvider::new(...)`. Each package
  has package-specific `XxxProviderSettings`, `create_xxx(settings)`
  (validating package-specific settings, including base URLs, conflicting
  credentials and template parameters) and a default instance `xxx()` where
  the package provides one.
  Models read a crate-private per-model config; there are no getters and no
  snapshot. `Fetch` (HTTP) and package-specific WebSocket settings are
  transport injection points. OpenAI, Anthropic and Google use fixed string
  credentials and header maps, without a body-transform setting. The
  OpenAI-compatible package exposes its upstream `transform_request_body`,
  `supported_urls` and `convert_usage` hooks. Cohere and Mistral expose
  `generate_id` settings. Package settings omit `name` and body transforms
  when the upstream package does not offer them. Every request carries
  `ai-sdk-<package>/<version>` in its user-agent.
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
- Recording: `RECORDING_SCHEMA` stays 2, and recordings written by aimux 0.5
  and earlier (schemas 1 and 2) still load; schema 3 and above are refused.
  `ProviderRecord` holds identity only (`provider_id`, `provider`,
  `model_id`); base URL, key source, profile, options and retry settings are
  no longer recorded, and when read from an older recording they are
  ignored. What this version cannot replay from an older recording fails at
  replay time with an error that names it: its empty `provider_id`
  (`ProviderRecord::registry_id`) for a rebuild from a registry, a prompt in
  the older format (`InputRecord::unreadable_prompt`, loaded as empty),
  older call options (snake_case keys the camelCase options would ignore are
  refused), or the pre-V4 decision options and results.
  `rebuild_provider(record, registry)` resolves `provider_id` through the
  caller-supplied registry, retaining its settings.
  If the default model method differs from the recorded one, pass the model
  to `replay_with_model`. Recorded provider strings change with the next item.
- `model.provider()` follows the package's fixed identity or, where offered,
  its `settings.name` override. Defaults: `openai.chat` /
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
- providerOptions namespaces: `googleVertex` and `amazonBedrock` are the
  canonical keys; the legacy `vertex` / `bedrock` keys are still read, and
  provider metadata is written under both, as the upstream packages do
  (Bedrock also reads `anthropic` for its Anthropic models). The
  Anthropic package reads `anthropic` merged with the first segment of a
  custom `name` (custom wins) and writes metadata under the custom key;
  compat providers no longer read the `openai` key; `providerOptions.deepseek`
  is honoured (`reasoningEffort`, `thinking`).
- Presets: `provider_registry.json` has 282 rows; Groq and DeepSeek are
  vendor packages instead of preset rows. Former thin wrapper types are
  deleted. The JSON is embedded and parsed once into a runtime descriptor
  table in `preset.rs`, held in a `OnceLock` without leaking memory:
  `PresetDescriptor`, `ParamSpec` and `DeriveSpec` own their data (`String`,
  `Vec`) instead of `&'static` slices, and lose `Copy`; there is no `family`
  field, generated presets source or per-name Rust function. Create presets
  or vendor packages by name via `create_provider(name, PresetSettings)`.
  Rows carry `auth: api_key | none` (`none` sends no
  `Authorization` unless the row's optional key variable, such as
  `VLLM_API_KEY`, is set; `PLACEHOLDER_API_KEY` is gone), `base_url_env` and
  template `params` (env, default, derived host maps). Only declared
  parameters are accepted, host parameters reject `/ @ : ?`, and an
  unexpanded placeholder is an error. Unknown names are `NoSuchProvider`;
  nothing falls back to OpenAI. `litellm_proxy` reads `LITELLM_PROXY_BASE_URL`
  (the old wrapper read its API-key variable as a URL). Registry validation
  runs in the table test, and
  `scripts/check_provider_boundaries.sh` runs in CI.
- Decisions (#226) come from the providers: `OpenAIProvider::decision` and
  `Provider::decision_model` on OpenAI, and `decision_model` on the official
  runtime presets (ollama, llamacpp, localai, laya, vllm, sglang,
  cloudflare). `laya` is a registry row. Decision recordings store the
  provider identity only, and `rebuild_decision_provider` takes the
  caller's `ProviderRegistry` instead of an API key.
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
  token; the user-agent suffix is gone.
- Removed from `aimux-providers`: `body_merge`, `openai_legacy`, `AzureAuth`,
  `TokenProvider`, `VertexAuth`, `BedrockAuth`, `StaticBearerConfig` and the
  `hmac` dependency (`sha2` / `hex` are dev-dependencies).
- Bedrock regions must be a single DNS label; invalid values fail the call
  with `InvalidArgument` before a request is sent.
- `ExternalProviderEntry` debug output reports only whether `api_key` is
  present, keeping literal keys out of debug logs.
- `TranscriptionModel::do_stream` is optional, as the AI SDK's `doStream?`:
  it returns `Option<BoxFuture<…>>` and is `None` by default, instead of a
  default method that fails with `UnsupportedFunctionality`. Implementors
  return `Some(Box::pin(…))`. `stream_transcribe` reports a model without the
  method as `UnsupportedFunctionality` before any I/O
  (`The {provider} model "{id}" does not support streaming transcription.`).
- `SpeechResult` has `usage: Option<JsonObject>`, the provider's usage in its
  native shape, as the AI SDK's `SpeechModelV4Result.usage`. Google speech
  passes its `usageMetadata` through; other vendors report `None`. Speech
  model implementors set the field.
- Decision recordings no longer store a capability snapshot:
  `InputRecord::decision_capabilities` is removed, and
  `InputRecord::from_decision_options` and `Recorder::record_decision_input`
  no longer take capabilities. `MockDecisionReplayModel` does not read the
  field; replay rebuilds the model from the host registry. The recording
  schema stays 2, and older recordings with the field still load.
- Decisions follow the AI SDK's `DecisionModelV4` (`@ai-sdk/provider@4.0.22`)
  and `decide` (`ai@7.0.128`). Questions are a map keyed by id; Choice and
  Score questions take `criteria` (option names to descriptions, ordered
  levels). Answers are `{choice, probabilities?}`, `{score, probabilities?}`
  (keyed `"0"`, `"1"`, …) and `{probability}`. `decide` takes `DecideOptions`
  (the call options plus `max_retries`, default 2, and `timeout`) and returns
  `DecideResult` (answers, usage with `totalTokens`, warnings, rounding,
  provider metadata, response). It validates the input, rejects unsupported
  question types before any I/O and validates the answers once after the
  retries, with the upstream messages. The model call runs inside
  `operation::OperationSettings::scope` with the decision's `max_retries` and
  `timeout`, as `generate_text`'s does.
- `DecisionModel` has `specification_version`, `provider`, `model_id`,
  `supported_question_types` and `do_decide(&DecisionCallOptions)`;
  `capabilities()` and `DecisionCapabilities` are removed, and provider limits
  are checked in `do_decide` before HTTP. Images and OpenAI typed choice
  values move to `providerOptions.<provider>`; the probability source,
  confidence, OpenAI typed values, Laya abstentions and `latencyMs` move to
  `providerMetadata.<provider>`. An OpenAI refusal or a vLLM skipped question
  fails the decision instead of returning a partial answer. Decision
  recordings hold the `DecideOptions` and the `DecideResult`; recordings made
  before this change load but do not replay.
- `Provider::decision_model` returns `Option<Result<Arc<dyn DecisionModel>, _>>`,
  `None` by default; the registry and `custom_provider` report `None` as
  `NoSuchModel` with `model_type = "decisionModel"`. `Provider::evaluation_model`
  and the `evaluation_model` module are removed: Google's `decision_model` is
  the language-model adapter (`google.evaluation`), OpenAI's stays the native
  Decisions API and `OpenAIProvider::evaluation` builds the Responses adapter.
  `EvaluationLanguageModel` is renamed `DecisionLanguageModel`; its default
  provider is `"{language provider}.decision"`.

**C ABI**

- New error codes `AIMUX_E_LOAD_API_KEY = 19` and `AIMUX_E_LOAD_SETTING = 20`
  (appended; retired 4 is not reused). `aimux_error_provider_code` returns the
  environment variable that was consulted and `aimux_error_provider_message`
  the key's description (`"OpenAI"`) or the setting name (`"region"`); no new
  symbol. `aimux_error_model_type` carries `NoSuchModel.model_type`. These two
  failures used to surface as `AIMUX_E_INVALID_ARGUMENT`.
- `aimux_decide` takes `DecideOptions` JSON and returns `DecideResult` JSON.
  `aimux_decision_capabilities` keeps its name and signature but returns
  `{specificationVersion, provider, modelId, supportedQuestionTypes}`.
  `aimux_provider_decision_model` on a provider without decision models
  returns `AIMUX_E_NO_SUCH_MODEL` (was `AIMUX_E_UNSUPPORTED_FUNCTIONALITY`).
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
- Decisions use the `DecideOptions` / `DecideResult` JSON. Node
  `decisionCapabilities(model)` is replaced by `describeDecisionModel(model)`
  and Python `decision_capabilities` by `describe_decision_model`; the native
  `capabilities()` method is `describe()`. Python
  `decide(model, state, questions, **options)` takes the questions as a dict
  keyed by id.

**Go / Java / Kotlin / Swift / Dart**

- New error codes 19 / 20 (`CodeLoadAPIKey` / `CodeLoadSetting`,
  `LoadAPIKeyError` / `LoadSettingError`, `.loadApiKey` / `.loadSetting`),
  with the consulted environment variable as `EnvVar` / `envVar`.
- The call-level `body_overrides` / `bodyOverrides` field is deleted from
  `GenerateTextOptions` (the core no longer accepts the key; leaving it would
  have been silently ignored). The provider `config_json` helpers
  (`ProviderConfig` in Go and Dart) drop `max_retries` / `body_overrides` and
  gain `params`; call-level `max_retries` is unchanged.
- Decisions use the `DecideOptions` / `DecideResult` JSON; the decision model's
  `Capabilities()` / `capabilities()` is `Describe()` / `describe()` and
  returns the model's V4 properties.

**All bindings**

- Typed provider tools in Java, Kotlin, Swift, Python and Flutter carry
  optional `supportsDeferredResults`, omitted from JSON when absent
  (provider-utils@5.0.55 src/types/tool.ts:316-326).

- The typed results and stream parts follow the core's AI SDK shapes
  (Go, Java, Kotlin, Swift, Flutter, Node, Python). `GenerateTextResult`
  gains `steps`, `tool_results` and `output` and loses `raw`; its `content`
  holds the final step's parsed parts; `usage` / `total_usage` are
  `LanguageModelUsage` and `finish_reason` the unified string.
  `GenerateObjectResult` loses `raw` and `raw_finish_reason`.
  `consume_stream_text` returns a `GenerateTextResult`
  (`StreamTextResultAggregated` is removed). The options' `response_format`
  is replaced by `output`. Stream parts: `TextDelta` / `ReasoningDelta` carry
  `text`; `StreamStart` and `ResponseMetadata` give way to `Start`,
  `StartStep`, `FinishStep`, `Finish { finish_reason, raw_finish_reason,
  total_usage }` and `Abort`; `ToolResult` carries `output` and the call's
  `input` (was `result`, `is_error`), and `ToolError`, `ToolOutputDenied` and
  `ToolApprovalResponse` are decoded. Tool-call
  repair reads the top-level `tool_calls` only.

### Added

- Vertex AI MaaS and xAI sub-providers, as `@ai-sdk/google-vertex` 5.0.98's
  `/maas` and `/xai` paths: `create_google_vertex_maas` / `google_vertex_maas`
  (an OpenAI-compatible provider named `vertex.maas`) and
  `create_google_vertex_xai` / `google_vertex_xai` (Grok chat models named
  `googleVertex.xai`), over the project/location `endpoints/openapi` URL with
  a Google OAuth token. As in the Node wrappers, the provider headers and the
  token go over the per-call headers, and a provider `user-agent` header is
  sent as the whole user-agent.
- Open Responses requests carry `ai-sdk-open-responses/2.0.58`, the package
  part of `@ai-sdk/open-responses` (now pinned in `VERSIONS.json`).
  `OpenResponsesProviderSettings::user_agent_suffix` replaces it, as the AI
  SDK's `userAgentSuffix` does.
- `aimux_provider_utils::download_blob`, the AI SDK's `downloadBlob`: a GET
  of an untrusted URL through the download guard with no headers, so no
  `user-agent`. OpenAI-compatible image edits download URL files with it, as
  upstream does, and no longer send `ai-sdk-provider-utils/5.0.53` there.
- `HttpRequest::omit_provider_utils_user_agent` (`aimux-provider-utils`):
  sends the `user-agent` as given, without the `ai-sdk-provider-utils/<v>`
  part, for an exchange the AI SDK makes with a plain `fetch`. A struct
  literal of `HttpRequest` must now set it or use `..Default::default()`.
  The Google Files upload uses it: the init request sends the provider
  headers (`ai-sdk-google/4.0.87`) and the byte upload sends no
  `user-agent`, as `@ai-sdk/google`'s `fetch` calls do.
- `OpenAIProvider::tools` and `AzureOpenAIProvider::tools`: the provider tools
  on the provider instance, as the AI SDK's `openai.tools` and `azure.tools`
  (`openai().tools.web_search(…)`). Azure's holds its upstream subset.
- `custom_provider` forwards model discovery as it forwards files and
  skills: `CustomProviderOptions::discovery`, else the fallback provider's
  `discovery()`.
- `aimux_core::wrap_provider`, the AI SDK's `wrapProvider`: wraps every
  language model of a provider with language-model middleware and, when
  given, every image model with image-model middleware; all other members
  are the provider's own.
- C ABI: `void aimux_drop_handle_ptr(void *handle)`, `aimux_drop_handle` with
  the handle passed as a pointer-sized value (NULL is a no-op), for a
  finalizer that can only register a `void (*)(void *)` callback.
- `aimux_providers::provider_api_key_env(name)`: the environment variable a
  built-in provider reads its key from when `create_provider` gets none, and
  `provider_default_base_url(name)`, the endpoint it calls when given no base
  URL (`None` when that is built from settings).

### Fixed

- `default_providers()`, `openai()` and `anthropic()` no longer panic on an
  invalid `OPENAI_BASE_URL` / `ANTHROPIC_BASE_URL`. The variable is still
  read when the provider is created, as the AI SDK does; an invalid value now
  fails each call of that provider with `InvalidArgument` naming the
  variable. An invalid explicit `base_url` still fails the factory.
- Anthropic Messages streams honor `include_raw_chunks`: each SSE chunk is
  sent as `StreamPart::Raw` before the parts made from it, a chunk that is
  not JSON as `Raw` with `null`, as the AI SDK does.
- OpenAI Responses: reasoning parts of one assistant message that share an
  `itemId` become one input item, as the AI SDK does: one `item_reference`
  with `store`, otherwise one `reasoning` item whose summary collects the
  parts and whose `encrypted_content` is the last one given. Each part used
  to become its own item, which the API rejects as a duplicate id.
- Recordings and logs redact Prodia's `X-Prodia-Key` and any header ending in
  `subscription-key` (Azure's `Ocp-Apim-Subscription-Key`), with or without
  separators; both were stored in clear text.
- Flutter: the `NativeFinalizer` that releases a garbage-collected `Model` or
  `ProviderHandle` calls `aimux_drop_handle_ptr`. It called
  `aimux_drop_handle(uint64_t)` through a `void (*)(void *)` pointer, which on
  32-bit ARM (armeabi-v7a) passes a garbage high word and leaks the handle.
- Python: every blocking native call (`generate_text`, `generate_object`,
  `consume_stream_text`, `generate_text_as_openai`,
  `ProviderHandle.list_models`, `get_model_specs`, the multimodal calls and
  the recording flushes) releases the GIL while it waits, so other Python
  threads keep running.
- aimux-web: an `env:VAR` key reference is accepted only for the variable the
  provider's key is registered under (for example `env:OPENAI_API_KEY` for
  `openai`); any other name is refused before it is read, on the calls,
  cache-probe and replay paths. It read any variable of the console process.
- aimux-web: a call or cache probe whose `base_url` is not the provider's
  default endpoint gets only a key carried in the request (plaintext, loopback
  binds only): the Settings key and the environment are not read, `env:` is
  refused, and with no key the call goes out without one. Providers that read
  other credentials from the environment (Bedrock, Vertex, Codex, Polly,
  DataForSEO, Anthropic on AWS) refuse a custom `base_url`. A client could
  point `base_url` at its own host and receive the console's key.
- Recordings: a call that lost a recording event (the JSONL recorder's queue
  was full or its writer gone) or was marked inconsistent (a duplicate
  attempt, an exchange update that matches no exchange) is written with
  `complete: false`, by the JSONL and the ring recorder, whether its last
  event or a flush writes it. Such calls used to be marked complete.
- `aimux-replay` replays a decision recording live for any provider with a
  decision model (OpenAI, the runtime presets, Jev), rebuilt from the same
  registry as language recordings: the built-in providers, the recorded one
  with `--api-key` when given. It accepted only Jev.
- aimux-web: a call at a custom `base_url` gets its own cache-probe trace
  scope, named after the provider and that endpoint, so two endpoints of one
  provider and model no longer match each other's prefix history. The trace
  scope isolates by provider name and model id; `TraceLayer::with_scope_name`
  names a scope for a host that calls one provider at several endpoints.

### Removed

- The cross-language contract fixtures are retired (#249):
  `contract-tests/fixtures/wire-format.json` (a record of Rust's own serde
  output, not of the AI SDK), `tool-call-repair.json`, the Node runner
  `contract-tests/run-node.ts`, `aimux-core/tests/contract_test.rs` and the
  bindings' contract and repair tests are deleted. Wire behaviour is pinned
  instead by fixtures recorded from the pinned AI SDK packages under
  `fixtures/aisdk/` (versions and sha512 integrity in
  `fixtures/aisdk/VERSIONS.json`, fetched by `scripts/fetch-aisdk.sh`) and by
  tests ported from upstream. The two decision fixtures stay in
  `contract-tests/fixtures/`.

### RFCs

- `docs/aisdk-architecture-alignment.md`: the V4 migration now lands on
  master layer by layer, each layer CI-green with all its consumers updated,
  instead of one atomic cutover through an integration branch (§0.4 Q3, D31,
  §9.1). The new C ABI, codegen, the full recording schema 3 and the binding
  migration come last; until then the existing C constructors keep their
  signatures and call the new factories, and ts-rs stays as the interim
  generator. No release is cut from master until every layer is merged;
  fixes for released versions ship from a maintenance branch. `ROADMAP.md`
  defers the ops protocol schema and L0 passthrough and no longer fixes C1
  at nine exports.
- `docs/aisdk-architecture-alignment.md` and RFC-0031
  (`docs/ai-sdk-request-pipeline.md`), against ai 7.0.127: object generation
  is the deprecated wrapper over `output` (no separate object spans), async
  jobs follow `generateVideo`'s start/status retries and poll budget, the
  result convenience fields are marked deprecated as upstream marks them,
  the response-message rules gain the empty-reasoning, invalid-input,
  `inputSchemaInput`, tool-message order and tool-error encoding rows, and
  the remaining differences are listed: stream retries, the first-chunk
  timeout's start, `additionalRetryableError` and per-batch idempotency
  keys.

- RFC-0035 shows the repair context, reply and result patch in the final
  JSON (`toolCall` tagged `tool-call`, errors tagged by `name`, camelCase
  keys). It says that a repaired call failing re-validation is returned as an
  invalid call, not wrapped in `AI_ToolCallRepairError`, as upstream does, and
  registers the pre-evaluated `inputSchema` and Swift's delayed abort as
  differences from upstream.

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
