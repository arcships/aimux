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
| 2. A new protocol | its own HTTP API for text generation | a `src/<name>/` module implementing `do_generate` / `do_stream` | `cohere` |
| 3. Single modality | any API, but only one non-text modality | a typed shell + one modality model | `serper` (search), `lmnt` (speech) |

If the vendor is OpenAI-compatible **and** needs quirks beyond a `profile`
flag (different auth header, non-standard error body, …), it is case 2, not
case 1.

## Case 1 — OpenAI-compatible vendor (registry row)

1. Add one row to `aimux-providers/src/provider_registry.json` (file is
   sorted by `name`; follow the flat 5-key shape):

   ```json
   { "name": "example", "display": "Example", "env_var": "EXAMPLE_API_KEY",
     "base_url": "https://api.example.com/v1", "profile": {} }
   ```

   `profile` carries the known quirks — `supports_top_k`, `supports_tools`,
   `supports_response_format`, `stream_usage_key`, `max_tokens_key` — see
   the rows that already use one (e.g. `groq`). Anything beyond body-shape
   flags rides in per-call `body_overrides` (RFC-0017); if the quirk changes
   auth or error shapes, it is case 2.

2. Regenerate the provider documentation and commit its output:

   ```sh
   python scripts/gen_providers_doc.py    # docs/api/providers.md (totals + list)
   ```

   CI runs it with `--check` in the `contract-tests` job and fails if the
   committed output is stale.

3. Derive replay cassettes: add a tuple to `PROVIDERS` in
   `scripts/generate_thin_wrapper_cassettes.py`, then run it. It derives
   `thin_wrapper_nonstream.json` / `thin_wrapper_stream.json` under
   `aimux-providers/tests/cassettes/<name>/` from **real OpenAI recordings**
   (rewriting request path and model id — not fake data), because a
   registry-backed provider's requests are byte-for-byte OpenAI shape.

4. Nothing else: `provider("example", ...)` now works in every binding, and
   env-var key loading follows `env_var` automatically.

> Roadmap note: #166 B2 turns the 33 standalone thin wrappers into registry
> rows and makes `conformance_test.rs` iterate the registry; the
> cassette-derivation step above is then replaced by that suite.

## Case 2 — a new protocol

1. New directory `aimux-providers/src/<name>/`:

   - `model.rs` — `<Name>Provider` implementing `LanguageModel`:
     `do_generate` / `do_stream` (plus other modality traits if the vendor
     offers them: embeddings, image, transcription…);
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
   pub use <name>::{<Name>Config, <Name>Provider};
   ```

3. Record real cassettes under `tests/cassettes/<name>/` (RFC-0003; no
   network, no keys at test time) and add a `<name>_conformance` module to
   `tests/conformance_test.rs` — `do_generate_returns_text` and
   `do_stream_returns_parts` against those cassettes, mirroring the existing
   20 modules.

4. Note: protocol providers are **not** name-addressable today — callers use
   the typed factories (`CohereProvider::new(...)`), and
   `provider("cohere", ...)` fails with `NoSuchProvider`. A registry row with
   a `protocol` field arrives with #166 B1 (the `Protocol` enum +
   `from_resolved`); until then, do not add protocol providers to the
   registry JSON.

## Case 3 — single-modality vendor

1. One file `aimux-providers/src/<name>.rs` (see `serper.rs` for search,
   `lmnt.rs` for speech; `cartesia/` for a vendor with two modalities):

   - `<Name>Config { api_key, base_url }` with `new` / `with_base_url` /
     `from_env`;
   - `<Name>Provider` — a shell whose only job is returning the modality
     model (`search_model()`, `speech_model()`, `image_model()`, …) and
     reporting its `name()`;
   - the modality model implementing the trait's operation
     (`SearchModel::do_search`, `SpeechModel::do_speak`, …) as: build the
     request body → send through the provider-utils HTTP helpers → map the
     response into aimux types. Implement **only** the transform functions;
     transport, retry and timeouts are not your problem (Core owns them).

2. `pub mod` + `pub use` in `lib.rs` under the modality's section comment
   (search-only, speech-only, image-only, video-only, …).

3. Cassettes + unit tests asserting the mapped result shapes.

## Generator and script rules

- **A generator may stay in `scripts/` only if its output carries a
  "GENERATED — do not edit" header and CI runs it with `--check`.** Today
  that is `gen_providers_doc.py` (the `contract-tests` job).
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
