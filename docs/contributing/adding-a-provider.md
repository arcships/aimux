# Adding a provider

Three cases cover every provider in aimux. Find yours, follow the checklist,
and run the generators and boundary scripts before opening a PR.

Related: [CONTRIBUTING.md](../../CONTRIBUTING.md) (workspace setup & build),
[RFC-0046](../../rfc/0046-provider-protocol-abstraction.md) (module layout,
internal boundaries & `layout.toml`),
[RFC-0045](../../rfc/0045-upstream-test-parity.md) (upstream test parity),
[RFC-0017](../../rfc/0017-provider-config-dx.md) (preset registry),
[RFC-0003](../../rfc/0003-test-cassette.md) (cassettes),
[RFC-0006](../../rfc/0006-provider-development.md) (minimum acceptance).

## Which case am I in?

| Case | The vendor speaks… | You add | Example |
|---|---|---|---|
| 1. OpenAI-compatible preset | OpenAI Chat Completions wire format, own base URL | one row in `aimux-providers/src/provider_registry.json` + derived cassettes | `deepinfra` |
| 2. Protocol or multi-modality package | its own HTTP API (or wraps a shared protocol via `::internal`) | `aimux-providers/src/<vendor>/` (`mod.rs`, `provider.rs`, model/helper files) | `cohere`, `mistral`, `codex` |
| 3. Single-modality (or modality-only) vendor | any non-text API (search, image, video, speech, transcription, reranking) | `aimux-providers/src/<vendor>/{mod.rs, provider.rs, <modality>_model.rs}` | `serper` (search), `lmnt` (speech), `replicate` (image + video) |

If an OpenAI-compatible vendor needs behavior beyond what a registry row
expresses (a custom auth scheme, a non-standard error body, or vendor-specific
message/option conversion), use Case 2 and create a directory next to `groq/`
and `deepseek/`. The registry has no `family` field.

## Case 1 — OpenAI-compatible vendor (registry row)

1. Add one row to `aimux-providers/src/provider_registry.json` (sorted
   alphabetically by `name`):

   ```json
   {
     "name": "example",
     "display": "Example",
     "env_var": "EXAMPLE_API_KEY",
     "base_url": "https://api.example.com/v1",
     "profile": {}
   }
   ```

   Optional fields (validated at runtime by `aimux-providers/src/preset/mod.rs`):
   - `auth: "none"` for local servers without a required key (never injects a
     placeholder key; optional `env_var` is sent only when set and non-empty),
     often paired with `base_url_env`;
   - `params` for a templated `base_url` with `{param}` placeholders (each with
     optional `env`, `default`, or `derive: { from, map, otherwise }`);
   - `profile.max_tokens_key` (`"max_tokens"` or `"max_completion_tokens"`);
   - `profile.convert_usage` (`"alibaba"` or `"moonshotai"`; add new usage
     mappers in `aimux-providers/src/preset/mod.rs`);
   - `profile.structured_output_models` (exact model IDs or `*`-suffixed
     prefixes; `[]` disables native `json_schema` and falls back to
     `json_object`; absent enables all models);
   - `profile.user_agent` (defaults to `openai-compatible`; e.g.
     `"amazon-bedrock"` for Bedrock Mantle).

   Presets also inherit `PresetSettings::transform_request_body` via
   `openai_compatible::internal`.

2. Regenerate the provider reference page and commit the result:

   ```sh
   python3 scripts/gen_providers_doc.py    # updates docs/api/providers.md
   ```

   `preset/mod.rs` embeds `provider_registry.json` once via
   `include_str!("../provider_registry.json")` and validates every row in its
   unit tests. CI verifies `docs/api/providers.md` with `--check`.

3. Derive replay cassettes if needed: add the vendor to `PROVIDERS` in
   `scripts/generate_thin_wrapper_cassettes.py` and run it to derive cassettes
   under `aimux-providers/tests/cassettes/<name>/` from real OpenAI recordings.

4. `create_provider("example", PresetSettings::default())` and
   `preset::create("example", ...)` now work in Rust and all bindings. Keys are
   loaded from `env_var` on every request; `provider_names()` lists all names.

## Case 2 — Protocol or multi-modality package

1. Create a package directory `aimux-providers/src/<vendor>/` (RFC-0046 §4.1):

   - **`mod.rs`** — declarations and re-exports only (mirrors upstream
     `index.ts`): `mod` declarations (keep implementation submodules private;
     only `internal` when exposing an upstream `/internal` surface), `pub use`
     for the public provider/settings/factory/model types, and
     `pub(crate) use provider::{API_KEY_ENV_VAR, DEFAULT_BASE_URL};` when used
     by `default_providers.rs`. End with `#[cfg(test)] mod tests;` when
     unit tests live in `src/<vendor>/tests/`.
   - **`provider.rs`** — `<Vendor>ProviderSettings`, `create_<vendor>(settings)`,
     optional `<vendor>()` default instance, and `<Vendor>Provider` implementing
     `Provider` (and `ProviderDiscovery` when listing models). Load API keys per
     request (`None` reads the env var on every request and fails with
     `AiMuxError::LoadApiKey`; `Some("")` is sent verbatim). Validate explicit
     `base_url` via `validate_base_url` at provider creation (or per request for
     dynamic endpoints). There is no builder-era `*Config` type, `from_env`,
     `with_*` setter, or retry config.
   - **Model & helper files**:
     - **With an upstream `@ai-sdk/<vendor>` package (RFC-0046 §4.2):** match
       upstream filenames and subdirectories in `snake_case`, stripping the
       redundant vendor prefix (e.g. `mistral-chat-language-model.ts` →
       `mistral/chat_language_model.rs`, `map-mistral-finish-reason.ts` →
       `mistral/map_finish_reason.rs`, `convert-to-mistral-chat-messages.ts` →
       `mistral/convert_to_chat_messages.rs`, `src/chat/` → `chat/`,
       `src/internal/index.ts` → `internal/mod.rs`). Put response schemas in
       `api_types.rs` when split out. Do not create `version.ts` (pins live in
       `aimux-providers/src/shared/aisdk_pins.rs`). Do not add stub files for
       unimplemented upstream modules; record them in
       `aimux-providers/layout.toml` as `status = "missing"` with an `issue`.
       Aimux-only extensions sit alongside upstream-mapped files in the same
       directory and are recorded in `layout.toml` as `status = "aimux"` with a
       `reason`.
     - **Without an upstream package (RFC-0046 §4.3):** name files by modality:
       `provider.rs`, `<modality>_model.rs` (e.g. `responses_model.rs`,
       `embedding.rs`, `reranking.rs`), `<modality>_model_options.rs`,
       `api_types.rs`; omit empty files.

   Reference implementations: `mistral/`, `cohere/`, `deepseek/`, `groq/`,
   `openai/`, `anthropic/`, `google/`, `bedrock/`, `codex/`, `jev/`.

2. Follow the protocol and boundary rules (enforced by
   `scripts/check_provider_boundaries.sh`):
   - Spell `providerOptions` / `providerMetadata` namespace keys once, in
     `options.rs`, `*_language_model_options.rs`, or a `NAMESPACE` constant
     read via `crate::shared::namespaced_options`.
   - Never compare provider names inside shared protocol packages
     (`openai/`, `openai_compatible/`, `anthropic/`, `google/`, `bedrock/`,
     `cohere/`).
   - Never read `options.max_retries`, `timeout`, `session_id`, or `call_id`
     outside `HttpRequest::new(` (and the two documented WebSocket realtime
     sites), and never call `prepare_retries` / `aimux_core::retry` inside
     `aimux-providers/src/`.
   - Cross-vendor reuse is strictly one-way through `crate::<vendor>::internal`
     (`azure`/`codex`/`huggingface` → `openai::internal`,
     `anthropic_aws`/`bedrock` → `anthropic::internal`,
     `vertex` → `{google,anthropic,openai_compatible}::internal`,
     `preset` → `openai_compatible::internal`,
     `decision` → `jev::internal`). Shared layers (`shared/`,
     `openai_compatible/`, `stream/`) never import vendor packages.

3. Wire the module in `aimux-providers/src/lib.rs` **under the appropriate
   section comment** (which `gen_providers_doc.py` uses for the category and
   totals table) and add dispatch + env/base-URL entries in
   `aimux-providers/src/default_providers.rs`:

   ```rust
   pub mod <vendor>;
   pub use <vendor>::{<Vendor>Provider, <Vendor>ProviderSettings, <vendor>, create_<vendor>};
   ```

4. Add cassettes under `aimux-providers/tests/cassettes/<vendor>/` (RFC-0003)
   and conformance / unit tests.

## Case 3 — Single-modality (or modality-only) vendor

1. Create `aimux-providers/src/<vendor>/` with:
   - `mod.rs` — module doc, private `mod` declarations (`provider`,
     `<modality>_model`, etc.), `pub(crate) use provider::{API_KEY_ENV_VAR, DEFAULT_BASE_URL};`
     (when fixed), and `pub use` re-exports of the provider, settings, factory,
     singleton, and model struct(s).
   - `provider.rs` — for standard 5-field providers (`base_url`, `api_key`,
     `headers`, `name`, `fetch`), invoke `crate::shared::define_vendor_provider!`
     from `aimux-providers/src/shared/factory.rs`, then wire `Provider` with
     `crate::impl_single_modality_provider!` (or
     `crate::impl_modality_provider!` when offering two modalities such as
     `image_model` + `video_model` or `speech_model` + `transcription_model`).
     Vendors with non-standard auth or settings (e.g. `aws_polly/`,
     `dataforseo/`, `google_pse/`, `prodia/`, `replicate/`, `searxng/`) keep
     explicit settings and provider structs in `provider.rs`.
   - `<modality>_model.rs` — e.g. `search_model.rs`, `image_model.rs`,
     `video_model.rs`, `speech_model.rs`, `transcription_model.rs`, or
     `reranking_model.rs`. Reuse the shared helpers in
     `aimux-providers/src/shared/` (`search.rs`, `poll.rs`, `error_body.rs`,
     `results.rs`, `media.rs`, `provider_options.rs`, `image_size.rs`). Async
     polling uses `shared::poll_until` / `retry_download` bounded by package
     constants.

2. Add `pub mod <vendor>;` and `pub use <vendor>::{...};` in
   `aimux-providers/src/lib.rs` under the modality's section comment, and wire
   dispatch in `aimux-providers/src/default_providers.rs`.

3. Add cassettes and unit tests asserting the mapped request/response shapes.

## Testing & `layout.toml`

- **Per-vendor unit tests (RFC-0046 §5):** packages aligned with an upstream
  `@ai-sdk/*` package place unit tests in `aimux-providers/src/<vendor>/tests/`
  (declared via `#[cfg(test)] mod tests;` in `src/<vendor>/mod.rs`), one file
  per upstream `*.test.ts` file, using shared `crate::test_support::mock_fetch`
  (`--features test-support` for external tests). Keep `aimux-providers/tests/`
  for cross-vendor suites (conformance, exhaustive cassette playback, e2e).
- **`aimux-providers/layout.toml` (RFC-0046 §4.4):** every upstream-backed
  vendor has a `[<vendor>]` table listing each upstream source file (`mapped`,
  `missing` with `issue`, `elsewhere`, or `not-applicable`), plus any
  aimux-only source files under that directory as `status = "aimux"` with
  `reason`. Run `python3 scripts/check_layout.py` to verify all `rust` paths
  exist and no non-`mod.rs` source file in a covered vendor directory is
  unlisted.

## Generators and checks

CI checks all generated artifacts and boundary rules:

```sh
python3 scripts/gen_providers_doc.py --check   # docs/api/providers.md
bash scripts/check_provider_boundaries.sh      # boundaries, internal surfaces & retry rules
python3 scripts/check_layout.py                # aimux-providers/layout.toml vs src/
cargo run -p aimux-codegen -- --check          # generated DTOs, schemas, constants
python3 scripts/check_dto_vectors.py           # language DTO round-trips
```

- **A generator stays in `scripts/` only if its output carries a
  `"GENERATED — do not edit"` header and CI runs it with `--check`.**
  `docs/api/providers.md` is the single source of truth for provider counts;
  never hardcode provider totals elsewhere.

## Before opening the PR

- [ ] Generators run and committed (`python3 scripts/gen_providers_doc.py --check` passes).
- [ ] Boundary and layout checks pass (`bash scripts/check_provider_boundaries.sh` and `python3 scripts/check_layout.py`).
- [ ] `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `RUSTDOCFLAGS="-D warnings" cargo doc -p aimux-providers --no-deps` pass.
- [ ] Cassettes committed; `cargo test -p aimux-providers` passes offline.
