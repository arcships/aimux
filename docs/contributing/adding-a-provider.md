# Adding a provider

Three cases cover every provider in aimux today. Find yours, follow the
checklist, and CI verifies the generated parts automatically.

Related: [CONTRIBUTING.md](../../CONTRIBUTING.md) (dev setup),
[RFC-0017](../../rfc/0017-provider-config-dx.md) (the registry),
[RFC-0003](../../rfc/0003-test-cassette.md) (cassettes),
[#166](https://github.com/arcships/aimux/issues/166) (the roadmap that will
simplify some steps below — noted per step).

## Which case am I in?

| Case | The vendor speaks… | You add | Example |
|---|---|---|---|
| 1. OpenAI-compatible | OpenAI Chat Completions wire format, own base URL | one registry row + derived cassettes | `deepinfra` |
| 2. A new protocol | its own HTTP API for text generation | a `src/<name>/` package: `XxxProviderSettings` + `create_xxx` + a model implementing `do_generate` / `do_stream` | `cohere` |
| 3. Single modality | any API, but only one non-text modality | a `XxxProviderSettings` + `create_xxx` factory + one modality model | `serper` (search), `lmnt` (speech) |

If the vendor is OpenAI-compatible **and** needs quirks beyond what a row can
say (a different auth header, a non-standard error body, message conversion of
its own, ...), it is case 2: add its own package next to `groq/` and
`deepseek/`, which have their own chat models. The registry has no `family` field.

## Case 1 — OpenAI-compatible vendor (registry row)

1. Add one row to `aimux-providers/src/provider_registry.json` (file is
   sorted by `name`):

   ```json
   { "name": "example", "display": "Example", "env_var": "EXAMPLE_API_KEY",
     "base_url": "https://api.example.com/v1", "profile": {} }
   ```

   Optional keys: `auth: "none"` (a local server: no key variable, no
   `Authorization` header, no placeholder key) with `base_url_env` (the
   variable holding the base URL); `params` for a templated `base_url`
   (`{param}` placeholders, each declared with an optional `env` list and
   `default`; a derived parameter takes `derive: {from, map, otherwise}`);
   `profile.max_tokens_key` (`"max_tokens"` or `"max_completion_tokens"`: the
   only max-token key the vendor accepts). Anything beyond that rides in a
   `transform_request_body` closure on the factory; if the quirk changes auth
   or error shapes, it is case 2.

2. Regenerate what is generated from the registry and commit its output:

   ```sh
   python scripts/gen_providers_doc.py    # docs/api/providers.md (totals + list)
   ```

   The registry is embedded and parsed once into a runtime descriptor table.
   Create presets with `create_provider(name, PresetSettings)`; there are no
   per-name Rust functions or generated presets source. The table test validates
   all rows and creates each with default settings. CI checks the generated documentation with `--check`
   in the `contract-tests` job.

3. Derive replay cassettes: add a tuple to `PROVIDERS` in
   `scripts/generate_thin_wrapper_cassettes.py`, then run it. It derives
   `thin_wrapper_nonstream.json` / `thin_wrapper_stream.json` under
   `aimux-providers/tests/cassettes/<name>/` from **real OpenAI recordings**
   (rewriting request path and model id - not fake data), because a
   registry-backed provider's requests are byte-for-byte OpenAI shape.

4. `create_provider("example", PresetSettings::default())` now works in Rust;
   bindings retain `provider("example", ...)`. Keys are loaded from `env_var`
   when a request is made. `provider_names()` lists accepted names.

> Roadmap note: #166 B2 makes `conformance_test.rs` iterate the registry; the
> cassette-derivation step above is then replaced by that suite.

## Case 2 — a new protocol

1. New directory `aimux-providers/src/<name>/`:

   - `mod.rs` — the factory, modelled on the vendor's `@ai-sdk/<name>`
     package: `<Name>ProviderSettings` (every field optional: `base_url`,
     `api_key: Option<Resolvable<String>>`, `headers`, `name`, `fetch`,
     `transform_request_body`), `create_<name>(settings)` (fails only for an
     unusable `base_url`), an infallible default instance `<name>()`, and
     `<Name>Provider` with one method per model (`chat(id)`, `embedding(id)`,
     …) plus the `Provider` trait impl. The key is resolved **on every request**
     (`None` reads the environment variable, `Some("")` is sent verbatim, a
     missing key fails the call with `AiMuxError::LoadApiKey`); a required
     setting that is missing fails the call with `LoadSetting`. Each model
     reports `provider()` as `"{name}.{method}"` with `name` defaulting to the
     package name. There is no `Config` type, no `from_env`, no `with_*`
     builder, and no retry setting: the model reads a private config built by
     the factory, and retry belongs to the caller (`max_retries` on the call);
   - `model.rs` — the model implementing `LanguageModel`:
     `do_generate` / `do_stream` (plus other modality traits if the vendor
     offers them: embeddings, image, transcription…);
   - `options.rs` — the providerOptions / providerMetadata namespace keys,
     spelled once;
   - `convert.rs` — unified message format ⇄ vendor wire format, both
     directions, including the error-body shape;
   - `types.rs` — vendor request/response structs (`#[serde(default)]` on
     optional fields).

   Reference implementations, smallest to largest: `mistral/`, `cohere/`,
   `openai/`, `anthropic/`, `google/`, `bedrock/` (event stream + SigV4).

2. Wire it up in `aimux-providers/src/lib.rs` **under the right section
   comment** — the comment is what `gen_providers_doc.py` reads as the
   category (and the totals table):

   ```rust
   pub mod <name>;
   pub use <name>::{<Name>Provider, <Name>ProviderSettings, <name>, create_<name>};
   ```

3. Record real cassettes under `tests/cassettes/<name>/` (RFC-0003; no
   network, no keys at test time) and add a `<name>_conformance` module to
   `tests/conformance_test.rs` — `do_generate_returns_text` and
   `do_stream_returns_parts` against those cassettes, mirroring the existing
   20 modules.

4. Add the package name and factory dispatch to `src/default_providers.rs`.
   Rust callers can then use `create_provider("cohere", PresetSettings::default())`
   or the typed factory `create_cohere(CohereProviderSettings {..})` / default
   instance `cohere()`. `default_providers()` supplies the map for
   `aimux_core::provider_registry::create_provider_registry(providers, options)`;
   model ids use `provider:model` by default. Vendor packages do not need rows
   in the compatible preset JSON. Model listing is exposed through
   `Provider::discovery()`.

## Case 3 — single-modality vendor

1. One file `aimux-providers/src/<name>.rs` (see `serper.rs` for search,
   `lmnt.rs` for speech; `cartesia/` for a vendor with two modalities):

   - `<Name>ProviderSettings { api_key, base_url, headers, name, fetch, .. }`
     (all optional), `create_<name>(settings)` and an infallible default
     instance `<name>()` — the key is resolved per request, as in case 2;
   - `<Name>Provider` — returns the modality model (`search_model()`,
     `speech_model()`, `image_model()`, …); the model reports `provider()` as
     `"{name}.{method}"` (`luma.image`, `tavily.search`);
   - the modality model implementing the trait's operation
     (`SearchModel::do_search`, `SpeechModel::do_speak`, …) as: build the
     request body → send through the provider-utils HTTP helpers → map the
     response into aimux types. Implement **only** the transform functions;
     transport, retry and timeouts are not your problem (Core owns them).

2. `pub mod` + `pub use` (`<Name>ProviderSettings`, `create_<name>`, `<name>`) in
   `lib.rs` under the modality's section comment (search-only, speech-only,
   image-only, video-only, …). Async jobs poll on package constants through
   `shared/poll.rs`; `scripts/check_provider_boundaries.sh` keeps retry,
   `options.max_retries` reads and builder-era names out of the package.

3. Cassettes + unit tests asserting the mapped result shapes.

## Generator and script rules

- **A generator may stay in `scripts/` only if its output carries a
  "GENERATED — do not edit" header and CI runs it with `--check`.** Today
  that is `gen_providers_doc.py` (the `contract-tests`
  job).
  `gen_ts_types.py` regenerates the ts-rs TypeScript
  types through `cargo test -p aimux-core --lib export` (the export tests
  are the gate).
- `docs/api/providers.md` is the single source of truth for provider counts
  (#177). Never hand-write a provider count anywhere else — link to the
  page (the README states the total once, with a date, next to that link).
- One-off scripts carry their retirement condition in the roadmap (#166):
  `generate_thin_wrapper_cassettes.py` is deleted once conformance tests
  iterate the registry (A1/B2); `convert_cassettes.py`,
  `convert_pydantic_ai.py`, `extract_litellm_bases.py`,
  `scan_litellm_urls.py` are import/audit one-offs that can be deleted after
  their data lands; `responses_similarity_audit.py` served RFC-0012 §3.5.

## Before opening the PR

- [ ] Generators run, output committed (CI `--check` green).
- [ ] Cassettes committed; `cargo test -p aimux-providers` passes offline.
- [ ] Case 2: conformance module added. Case 3: unit tests for the mapped shapes.
- [ ] `docs/api/providers.md` regenerated — the totals table reflects your addition.
- [ ] No provider-count literals introduced anywhere else.
