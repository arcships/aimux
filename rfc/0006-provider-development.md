# RFC-0006: Provider Development Specification

> **Status**: ACCEPTED (in force — required reading in CONTRIBUTING.md and applied by provider-research batches; 2026-07-28)  
> **Date**: 2026-07-28  
> **Scope**: Adding or redoing provider adapters in `aimux-providers`  
> **Related**: [Provider Inventory and Extraction Results](../provider-inventory/README.md), [Provider Adapter Layer Improvements](0002-provider-improvements.md), [Cassette Test Plan](0003-test-cassette.md), [Protocol Conversion and Adapter Layer Design](0005-protocol-conversion.md)

## 1. Positioning and Boundaries

This document specifies the minimum process, implementation contracts, and acceptance criteria required to develop a provider. The goal is to ensure that the capabilities claimed for support this time have a protocol basis, are correctly implemented, and can be deterministically verified.

Core principles:

1. Choose the implementation approach based on the provider's protocol facts, not inferred from the provider name or inventory tags.
2. Prefer reusing existing shared layers, but only reuse behaviors that the shared code actually supports.
3. Only investigate, implement, and test the current delivery scope; no prior census of all the provider's capabilities is required.
4. Options explicitly passed in by the user must be mapped, produce a warning, or return an error; they must not be silently dropped.
5. Required tests must not access the public internet or read real credentials.

This document does not define provider development priorities, inventory extraction, or statistics processes, nor does it require adding CI, generators, probes, or repository-level refactoring within provider tasks. When modifications to public contracts or shared infrastructure are genuinely needed, they should be made as independent, verifiable prerequisite changes.

## 2. Main Process for Each Provider

```text
Determine delivery scope → Verify protocol → Choose implementation path → Implement → Targeted testing → Export & check → Record implementation facts
```

### 2.1 Determining the Delivery Scope and Minimum Evidence

Before starting implementation, only confirm information directly related to this delivery:

- canonical ID, necessary aliases, and whether there is already an entry for the same provider or an aggregator;
- the capabilities implemented this time, e.g., language, embedding, speech, or image;
- the official API documentation, or the location of the corresponding protocol in the official SDK/OpenAPI;
- authentication, base URL, endpoint formula, and required environment variables;
- at least one model ID usable for test configuration;
- the request, response, and error structures for this capability;
- the optional behaviors claimed for support this time, e.g., streaming, tools, reasoning, or async tasks.

Capabilities not part of this delivery need not be investigated or filled in as "unknown". When extending capabilities later, re-verify the corresponding protocol.

[`provider-inventory/providers.json`](../provider-inventory/providers.json) is used to discover candidates, canonical IDs, aliases, and source leads; it cannot serve as a basis for protocol implementation. When evidence conflicts, adjudicate in the following order:

1. The provider's official API documentation, SDK, or OpenAPI;
2. Mature and traceable current implementations in `reference/`;
3. Records in the inventory where multiple independent sources agree;
4. A single third-party source or automated inference.

When there is insufficient evidence to confirm the request and response contract for this delivery, do not proceed to implementation.

### 2.2 Choosing the Implementation Path

| Path | Conditions for use | Main work |
|---|---|---|
| OpenAI-compatible thin wrapper | The authentication, URL, request, response, and streaming behaviors used this time can all be correctly expressed by the OpenAI shared layer | Configure base URL, name, credentials, profile, and model factory |
| OpenAI shared layer extension | Mostly compatible, but with clear, limited, and reuse-suitable differences | First extend the shared behavior and regression tests, then add the thin wrapper |
| Native protocol | Structural differences in authentication, path, messages, response, streaming state machine, or multi-step calls | Implement provider types, conversion, HTTP calls, and necessary state machines |
| Modality-specific implementation | Only integrating capabilities such as embedding, reranking, speech, transcription, image, or video | Directly implement the corresponding model trait and factory |

Selection rules:

- Use the thinnest implementation that fully preserves this protocol's semantics.
- An "OpenAI-compatible" label, a `/v1` URL, or an inventory tag alone cannot prove that a thin wrapper is usable.
- The existence of a profile field or public method does not mean the request and response code already consumes that capability.
- gateway, cloud platform, and local runtime are business categories, not implementation paths; still judge them by the table above.
- A pure-modality provider must not fake a language model for a unified appearance.

### 2.3 Implementation Contract

#### Config and Authentication

- Fail fast when required credentials are missing or empty; services without authentication must not fake user-credential semantics.
- The base URL, version segment, and endpoint concatenation rules must be explicit, avoiding duplicate or omitted `/`.
- Only configuration items that the request code will actually read should be exposed externally.
- By default, call-level or config-level header overrides for authentication, signing, and other protocol-required headers are forbidden; overrides are only allowed when a public API or the provider protocol explicitly permits them, and conflict behavior must be tested.
- Credentials must not appear in logs, errors, `Debug`, test snapshots, or cassettes.

#### Provider and Model Factory

- Only provide model factories that the provider actually supports and that are implemented this time.
- When supporting a language model, implement [`Provider`](../aimux-core/src/provider.rs); pure-modality implementations directly provide the corresponding model factory.
- Multiple capabilities of the same provider share Config, authentication, and the HTTP client; do not duplicate security-sensitive logic.
- The public provider name, model provider name, and model ID must be stable; add assertions when there are non-obvious differences.

#### Request, Response, and Error

- For explicit options that the model can accept this time, perform one of "map, warn, error".
- `provider_options` only reads this provider's stable namespace and validates field types and values.
- Request conversion should remain a pure function with no network side effects.
- Responses preserve the text, reasoning, tool calls, usage, finish reason, and provider metadata that the public result type can express.
- Unknown but legitimate enum values degrade safely and retain the raw value where possible; unknown responses must not panic.
- When the provider's error structure differs from the shared structure, add dedicated parsing instead of relying on default error mapping.
- Public fields or behaviors not yet defined in core must not be faked inside the provider.

#### Code Organization and Exports

- Thin wrappers with no protocol differences should preferably use a single file.
- Native implementations only split into `types`, `convert`, `model`, or modality files when complexity requires it; do not create empty placeholder modules.
- Export the Config, Provider, and model types that callers need in [`aimux-providers/src/lib.rs`](../aimux-providers/src/lib.rs); do not export wire types or parsing state.

### 2.4 Minimum Testing Requirements

All required tests use local fixtures, wiremock, or already-desensitized cassettes; they do not access real services or read real credentials. The test scope follows this implementation and its differences; do not create placeholder tests for unimplemented capabilities.

| Change type | Must-test content |
|---|---|
| All providers | URL/path, minimal request, and minimal response; when there is authentication, required config, provider-specific errors, or non-obvious identity differences, also test the corresponding behavior |
| No-difference thin wrapper | Use shared smoke tests to verify URL, credentials, and module identity; when the profile has differences, also test its behavior |
| Profile or shared layer extension | Direct behavior tests for the new differences, plus non-regression of the default OpenAI path |
| Native protocol | Pure conversion, provider errors, and provider-specific protocol behavior involved this time; for streaming, only test state ordering and mid-stream errors when supported |
| Custom headers | When this capability is exposed, test ordinary header merging and required-header conflict behavior |
| Unsupported options | When this delivery's public options have fields that upstream does not support, test that explicit input produces the expected warning or error |
| Modality implementation | Test the inputs, outputs, and limits for this delivery's modality per Section 3.4, plus one failure path |

Generic status-code behavior already covered by the shared HTTP or error layer does not need to be retested for each provider. The provider only tests its own structural or mapping differences. Thin wrappers can plug into the shared tests in [`openai_compatible_test.rs`](../aimux-providers/tests/openai_compatible_test.rs) but need not re-prove unchanged shared implementation details.

### 2.5 Completion Checks

At minimum, run the checks directly related to the change:

```bash
cargo fmt --check
cargo test -p aimux-providers --test <provider_test_target>
cargo clippy -p aimux-providers --lib -- -D warnings
```

When modifying the OpenAI shared layer, public utilities, or core contracts, also run the full tests of the affected crate; a plain thin wrapper should not bear unrelated repository-wide verification due to process requirements.

After implementation is complete, record the implementation status of the corresponding canonical ID and the new protocol evidence added this time.

## 3. Conditional Rules

This section is only executed when this implementation hits the corresponding capability; it is not a fixed checklist for every provider.

### 3.1 OpenAI Shared Layer Extension

Only extend the shared layer when the difference can be expressed as an explicit profile field, a closed enum, or a general conversion rule.

Extensions must satisfy:

1. Request construction or response parsing actually reads the new config;
2. The default OpenAI behavior remains unchanged, with regression tests;
3. No conditional branches scattered in the shared state machine keyed on provider name;
4. If the difference only serves one complex provider and significantly increases shared-layer complexity, use a native implementation instead.

### 3.2 Streaming Output

Only when streaming support is declared do you need to verify and test:

- Use the upstream's actual transport protocol, e.g., SSE, NDJSON, WebSocket, or event stream;
- Public events satisfy the ordering contract of start, delta, end, and finish;
- text, reasoning, and tool input use stable IDs;
- Tool argument fragments produce the final tool call only after forming valid JSON;
- When the upstream provides final usage, finish reason, or end metadata, preserve it correctly in the public result;
- Upstream errors, malformed data, or connection interruptions do not panic, nor do they fabricate a successful `Finish`.

Non-SSE protocols use the corresponding parser or an independent state machine; do not alter protocol semantics just to reuse an interface.

### 3.3 Tools, Reasoning, and Structured Output

Only when support for the corresponding capability is declared do you verify its request and response:

- tools: definitions, choice, call arguments, results, and streaming fragments;
- reasoning: request options, content fields, and usage;
- structured output: schema, mode restrictions, and unsupported behaviors.

### 3.4 Non-language Models

Only execute the rules corresponding to this delivery's modality:

| Capability | Contract that must be preserved |
|---|---|
| Embedding | Output order matches input order; specify batch and dimension limits |
| Reranking | Preserve original document index, score, and `top_n` semantics |
| Speech / Transcription | Correctly handle media types, binary or URL results, and supported formats |
| Image / Video | Correctly handle quantity, size or aspect ratio, input files, and result form |

When upstream limits are exceeded, an error should be reported or batching should be done according to rules explicitly allowed by the public trait; silent truncation is not allowed.

### 3.5 Async Task API

Only when the call chain includes submission and polling do you define and test:

- The submission result and task ID;
- Polling interval and in-progress status;
- Success, failure, and timeout termination conditions;
- Cancellation behavior when the public interface actually supports it.

Do not poll indefinitely, and do not treat in-progress status as a successful result.

### 3.6 Cassette

When deterministic tests can precisely cover requests and parsing, a cassette is not a required gate. A desensitized cassette may be added in the following cases:

- Native or complex streaming protocols lack stable fixtures;
- Real responses have known differences from official documentation;
- The implementation depends on provider-specific fields or event combinations.

A no-difference thin wrapper does not need to record a cassette merely for process completeness. Recording, desensitization, and source requirements are in [RFC-0003](0003-test-cassette.md).

### 3.7 Public Contract Changes

If correct integration depends on new core fields, options, trait methods, or shared infrastructure capabilities, that public contract should be reviewed separately first, and regression tests should prove that existing providers are unaffected. The provider implementation may depend on that change, but must not create an inconsistent alternative interface in local types.

## 4. Change-record Requirements

A simple thin wrapper only needs to provide, in the issue or PR, official protocol evidence, shared-layer reuse basis, verified differences, and corresponding tests; no separate design table is required.

Only shared-layer extensions, native protocols, custom streaming state machines, signature authentication, async tasks, or core changes require a separate design record. The record should cover protocol evidence, key conversions, error boundaries, scope of change, and known limitations; the format is unrestricted.

## 5. Definition of Done

- [ ] The capability scope, provider identity, and official protocol evidence for this delivery are clear.
- [ ] The implementation path conforms to Section 2.2, with no dependence on shared capabilities that have not taken effect.
- [ ] Config, authentication, request, response, and error satisfy Section 2.3.
- [ ] The conditional capabilities that are hit satisfy Section 3; conditional capabilities that are not hit are not forcibly implemented or tested.
- [ ] The deterministic tests required by Section 2.4 pass, and the tests do not use the public internet or real credentials.
- [ ] Public exports are complete, and the relevant formatting, test, and lint checks have been run.
- [ ] The implementation facts for the canonical ID and the new protocol evidence have been recorded.
- [ ] Any shared-layer or core change has been independently verified not to regress the default path.

## 6. Implementation Entry Points

- Public model contract: [`aimux-core/src/`](../aimux-core/src/)
- OpenAI shared layer: [`aimux-providers/src/openai/`](../aimux-providers/src/openai/)
- Provider tests: [`aimux-providers/tests/`](../aimux-providers/tests/)
