# Protocol Conversion and Adapter Layer Design

> Scanned 104 projects under reference/, recording each project's protocol-conversion logic and provider adapter layer design.
> The focus is not the provider list (see [0004-provider-inventory.md](0004-provider-inventory.md)), but **how to unify different providers' protocols**.

## 1. aimux's Current State

aimux's adapter layer is in [aimux-providers/src/openai/](aimux-providers/src/openai/):
- `model.rs`'s `execute_generate`/`execute_stream` are the shared entry points (free functions); Azure already reuses them.
- `convert.rs` converts the unified message format into the OpenAI request body and parses the response back into `GenerateResult`/`StreamPart`.
- 13 thin wrappers (groq/deepseek/...) only change the URL and add no customization.
- Native protocols (anthropic/google/bedrock/cohere/mistral) each have their own model+convert and do not share OpenAI logic.
- **There is no cross-protocol conversion** (no OpenAI↔Anthropic conversion; each provider only handles its own protocol's request construction and response parsing).

## 2. Adapter Layer Design Patterns in Reference Projects

### Pattern One: OpenAI-Compatible Shared Layer (rig / rust-genai / llm-connector / litellm)

Extract the common logic of OpenAI Chat Completions—request construction, SSE parsing, tool-call aggregation, etc.—and let compatible providers fill in only config differences.

**rig** (Rust, not object-safe):
- The `OpenAICompatibleProvider` trait exposes const capability switches (`SUPPORTS_TOOLS`, `SUPPORTS_RESPONSE_FORMAT`, `STREAM_INCLUDE_USAGE`) + hook methods.
- The `CompatibleStreamProfile` trait + generic driver function `send_compatible_streaming_request<T,P>` share the streaming state machine.
- 16 compatible providers share it, each filling in its consts and hooks.
- Native protocols (Anthropic/Gemini/Bedrock) have parallel traits and do not share the OpenAI layer.

**rust-genai** (Rust):
- The `Adapter` trait has all-static methods + the `dispatch_adapter!` macro for compile-time dispatch.
- `adapter_shared.rs` is the OpenAI common request-construction entry point.
- The `impl_pass_through_adapter!` macro generates delegation implementations (e.g., MiniMax delegates to Anthropic).
- `unsupported: [embeddings]` declares unsupported capabilities.

**llm-connector** (Rust):
- The `Protocol` trait + associated types (`type Request`/`type Response`).
- The `OpenAICompatibleCapabilities` capability-bit struct (`content_block_mode`/`supports_tool_choice`/`reasoning_request_strategy`, etc.).
- Stateless shared functions (`build_openai_compatible_request_parts`), not inheritance.

**litellm** (Python):
- The `BaseConfig` base class defines the `transform_request`/`transform_response` abstract methods + capability hooks (`should_fake_stream`/`sign_request`/`get_complete_url`).
- `OpenAIGPTConfig` is subclassed by ~30 compatible providers.
- Native protocols (Anthropic/Cohere/Gemini) directly inherit `BaseConfig`.

> **Implication for aimux**: the "config-description struct" proposed in RFC-0002 is this pattern—but aimux must stay object-safe and cannot use rig's generic trait. litellm's `BaseConfig` inheritance pattern is closest to what aimux can use: a struct describes the differences, and shared functions read it to decide behavior.

### Pattern Two: Delegating to External SDKs (opencode / aider / mastra)

Don't write protocol conversion yourself; directly use others' provider layers.

**opencode** (190k stars): delegates to the Vercel AI SDK (`@ai-sdk/*`), doing only quirk patches itself (`normalizeMessages`/`sdkKey`/model-specific handling in `transform.ts`).

**aider**: fully delegates to litellm, with zero protocol code of its own.

**mastra**: reuses the Vercel AI SDK provider layer.

> aimux takes the self-built route and does not delegate. But opencode's "quirk patch" idea is worth referencing—the config description of a thin wrapper is essentially a quirk patch.

### Pattern Three: Self-implemented Protocol Adapters (pi / continue / Roo-Code / opencodex)

Write each provider's protocol adaptation yourself, without relying on external SDKs.

**pi** (79k stars, TypeScript):
- 10 API adapters (`api/anthropic-messages.ts`, `api/openai-completions.ts`, `api/openai-responses.ts`, `api/bedrock-converse-stream.ts`, `api/google-generative-ai.ts`, etc.).
- One provider can mount multiple APIs (e.g., github-copilot supports anthropic-messages/openai-completions/openai-responses at the same time).
- 37 built-in providers.

**continue** (35k stars, TypeScript):
- Standalone package `@continuedev/openai-adapters`, one adapter file per provider.
- 66 provider classes.
- `openaiToVercelMessages.ts`/`convertToolsToVercel.ts` does conversion to/from the Vercel AI SDK.

**Roo-Code** (24k stars, TypeScript):
- Internal canonical format = **Anthropic Messages** (not OpenAI).
- The `transform/` directory does Anthropic→each provider: `openai-format.ts`, `gemini-format.ts`, `bedrock-converse-format.ts`, `mistral-format.ts`, `minimax-format.ts`, `zai-format.ts`.
- `reasoning.rs` does cross-provider reasoning format conversion.

> **Key difference**: aimux's canonical format is its own `LanguageModelPrompt`; each provider converts it into its own format. Roo-Code chooses Anthropic as the canonical format; pi chooses its own `pi-messages`. aimux's choice is similar to pi's—an in-house intermediate format.

## 3. Protocol Conversion in Gateway Projects (Cross-protocol Conversion)

Gateway projects do what SDKs do not: **let users send requests in protocol A while the backend calls the provider in protocol B**. For example, the user sends OpenAI format and the backend calls Anthropic.

### Conversion Architectures Fall into Three Categories

**Fully connected mesh (new-api)**:
- Defines the `RelayFormat` enum: openai/claude/gemini/openai_responses.
- A registry + multi-step chained conversion: A→C can automatically go A→B→C.
- Converters are organized as `{from}_to_{to}` files (`oai_chat/to_claude_messages_req.go`, etc.).
- Quality grading: good/fair/discouraged.
- **The most complete protocol-conversion implementation**, with arbitrary pairwise conversion among 4 formats.

**Hub-and-spoke, OpenAI-centric (one-api / portkey / aiproxy / simple-one-api / ferro)**:
- Inbound is unified to OpenAI format; each provider's adaptor converts OpenAI to its native format.
- one-api does not support Anthropic/Gemini as inbound formats.
- portkey supports multiple inbound endpoints (OpenAI Chat / Anthropic Messages / Responses), but internally unifies them into an OpenAI intermediate representation.

**N×M matrix (envoy-ai-gateway / axonhub)**:
- envoy: an inbound-protocol × backend-schema combination selects a translator; the file name is the conversion pair (`openai_awsbedrock.go`, `anthropic_openai.go`).
- axonhub: Inbound transformer (client→unified IR) × Outbound transformer (unified IR→provider), converting via the unified `llm.Request`/`llm.Response`.

### Support for Key Conversion Pairs

| Conversion pair | new-api | bifrost | envoy | axonhub | higress | portkey | opencodex |
|--------|:---:|:---:|:---:|:---:|:---:|:---:|:---:|
| OpenAI Chat ↔ Anthropic Messages | ✅ direct | ✅ | ✅ | ✅ | ✅(46KB) | ✅ | ✅ (via internal Responses) |
| OpenAI Chat ↔ Gemini | ✅ direct | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| OpenAI Responses ↔ Chat | ✅ bidirectional | ✅ native | ✅ inbound | ✅ complete | ❌ | ✅ inbound | ✅ |
| Anthropic Messages → Gemini | ✅ direct | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Bedrock event stream → SSE | aws channel | ✅ eventstream | ✅ eventstream | ✅ | ✅(56KB) | bedrock | — |

### Streaming Conversion

All projects use **stateful per-chunk conversion via a state machine**:
- new-api: `StreamState` three-stage (`NewStreamState`/`ConvertStreamChunk`/`FinalizeStream`).
- higress: `ClaudeToOpenAIConverter` maintains `messageStartSent`/`thinkingBlockIndex`/`toolBlockIndex`/`toolCallStates`.
- portkey: `AnthropicStreamState`.
- bifrost: `chan *BifrostStreamChunk`.
- envoy: extproc translates the streaming body chunk by chunk.

> **This is the common difficulty of cross-protocol streaming**: you cannot simply translate SSE events one by one; you must maintain state (which content block you are currently in, tool-call fragment aggregation, thinking-block start/end). aimux's `StreamPart` already has a three-stage form (Start/Delta/End); if cross-protocol conversion is to be done in the future, this state-machine design is the foundation.

### Tool-calling Conversion

Generally handled within each provider's request/response conversion functions, with no independent unified layer (except new-api's `shared/claude/tool_choice.go` and axonhub's `tools.go`/`tool_blocks.go`).

Conversion pairs:
- OpenAI `tool_calls` (`function.arguments` is a JSON string) ↔ Anthropic `tool_use` (`input` is a JSON object) ↔ Gemini `functionCall` (`args` is a JSON object)
- Argument type difference: OpenAI is string, Anthropic/Gemini is object, requiring `serde_json::from_str`/`to_string` conversion.

## 4. Protocol Conversion in Coding Agents and Forwarding Services

### True Protocol-conversion Proxies

**opencodex** (5.2k stars, the forwarding service with the most complete protocol conversion):
- Internal intermediate representation + bidirectional adapter architecture.
- Inbound: Codex (OpenAI Responses) → directly into the internal representation; Claude Code (Anthropic Messages) → `claude/inbound.ts` converts to an internal Responses body.
- Outbound adapters (`adapters/`): anthropic.ts, openai-chat.ts, openai-responses.ts, google.ts, azure.ts, kiro.ts, cursor.ts (protobuf).
- Return bridge: `bridge.ts` converts back to Responses SSE (for Codex) or Anthropic Messages SSE (for Claude Code).
- 60 provider entries.
- OAuth support: Codex subscription, Claude subscription, GitHub Copilot, Grok Build, Kiro, Antigravity, Cursor.

**claude-worker-proxy** (Cloudflare Worker):
- Anthropic Messages → OpenAI Chat / Gemini / OpenAI Responses, bidirectional.
- Lightweight, single file, no account pool.

**ccswitch-deepseek**:
- OpenAI Responses → DeepSeek Chat Completions, one-way.
- Includes DeepSeek thinking-mode multi-turn reasoning recovery (`recover.js`).

### Config Switchers (No Protocol Conversion)

**cc-switch** (122k stars, Rust+Tauri):
- Rewrites each app's config files (`ANTHROPIC_BASE_URL`/`AUTH_TOKEN`/`ANTHROPIC_MODEL`).
- An optional proxy only does model-name mapping (haiku/sonnet/opus → provider model names), without altering the message body.
- 80+ provider presets.
- Supports Claude/Codex/Copilot/Grok Build subscription OAuth.

**claude-code-router** (36k stars):
- Forwards the client's protocol as-is to a provider that declares that protocol capability.
- The core does not do Anthropic↔OpenAI message-body conversion; that is left to the user writing a route script.
- Credential pool (multi-key rotation/cooldown/rate-limiting).

**CCSwitcher** (macOS):
- Claude Code OAuth account-pool switching (Keychain-managed).
- No protocol conversion.

## 5. Differences Between aimux and Reference Projects

| Dimension | aimux | rig | litellm | new-api | opencodex |
|------|---------|-----|---------|---------|-----------|
| Positioning | Unified service access layer | Rust LLM framework | Python gateway + library | Go gateway | Forwarding proxy |
| Canonical format | Own `LanguageModelPrompt` | Own `CompletionRequest` | Own `BaseConfig` | OpenAI/Anthropic/Gemini multi-format | Own internal Responses |
| OpenAI-compatible sharing | `execute_generate` free function | `OpenAICompatibleProvider` trait | `OpenAIGPTConfig` subclassing | relayconvert registry | adapters/openai-chat.ts |
| Cross-protocol conversion | ❌ None | ❌ None | ❌ None | ✅ Full mesh | ✅ Via internal intermediate layer |
| object-safe | ✅ | ❌ | — | — | — |
| Streaming state machine | `StreamPart` three-stage | `StreamingCompletionResponse` built-in aggregation | Each provider implements its own | `StreamState` three-stage | bridge return bridging |

**Core conclusion**: aimux does not do cross-protocol conversion (a gateway's job); it only does "call each provider through a unified interface" (an SDK's job). The adapter layer's improvement direction is RFC-0002's config-description struct—letting thin wrappers express differences while staying object-safe.

## 6. Data Sources

This document comes from the following scan (2026-07-28):

- **SDK protocol conversion**: rig, rust-genai, llm-connector, edgequake-llm, litellm, pydantic-ai, instructor, eino, langchaingo, langchain4j
- **Gateway protocol conversion**: new-api, one-api, portkey-gateway, bifrost, higress, axonhub, ferro-ai-gateway, APIPark, envoy-ai-gateway, uni-api, simple-one-api, aiproxy
- **Coding agent provider**: codex, opencode, pi, gemini-cli, cline, aider, continue, Roo-Code, opencode-ai
- **Forwarding services**: opencodex, claude-code-router, cc-switch, claude-worker-proxy, ccswitch-deepseek, ccs-nicremo, CCSwitcher, oh-my-opencode-slim, agent-of-empires, pinchbench
