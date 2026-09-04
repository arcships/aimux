# Adding a provider

## The four layers

Adding a provider means touching exactly one of these. Find your layer first;
the rest is someone else's job or a generator's.

| Layer | What it is | Who changes it | Gate |
|-------|------------|----------------|------|
| **L0** protocol | Hand-written Rust under `aimux-providers/src/<protocol>/` — one directory per wire protocol (openai, anthropic, google, …) | a human, rarely | `cargo test` + conformance cases |
| **L1** identity | `aimux-providers/src/provider_registry.json` — one row per vendor: `name`, `display`, `tier`, `base_url`, `env_var`, `profile`, optional `note` / `status` | a human, by hand — **never regenerated** | `registry_entries_are_valid` and the other registry tests in `provider.rs` |
| **L2** model knowledge | `aimux-providers/data/models/<name>.json` — context windows, pricing, modalities, from models.dev / anya2a | `scripts/gen_models.py`; corrections go in `data/models.overrides.json` | `gen_models.py --check` in CI |
| **L3** derived artifacts | `provider_name.rs` and its 7 language twins, `docs/api/providers.md` | a generator, never by hand | `gen_provider_names.py --check`, `gen_providers_doc.py --check` |

**The generator rule.** Every derived artifact carries a do-not-edit header
and has a `--check` mode wired into CI. A generator without one is dead code
and gets deleted — that is the test for whether a generator is still alive.
If you hand-edit an L3 file, CI fails and your edit is lost on the next run.

**Tiers.** `verified` — a cassette in `aimux-providers/tests/cassettes/<name>/`
replays this provider, so the base URL and wire shape are known-good.
`listed` — an upstream catalogue knows the name, but aimux has no recording.
`unverified` — neither; the row is a lead from vendor docs and may be wrong.
Adding a cassette is what promotes a row to `verified`.

## Case 1 — an OpenAI-compatible vendor (the common case)

No Rust. One L1 row, then let the generators and tests catch up.

1. Add one row to `aimux-providers/src/provider_registry.json`, keys in this
   order: `name` (snake_case), `display`, `tier` (`unverified` until you have
   a cassette), `base_url` (including the `/v1` the vendor actually serves),
   `env_var`, `profile` (`{}` unless the vendor drops a capability — see
   `ProviderProfile` for the flags). Add a `note` if you deliberately
   disagree with upstream; that exempts the row from the weekly sync report.
2. `python3 scripts/gen_provider_names.py` — regenerates the name constants
   for Rust and the 7 bindings.
3. `python3 scripts/gen_providers_doc.py` — regenerates `docs/api/providers.md`.
4. `python3 scripts/gen_models.py` — only if models.dev or anya2a carries the
   vendor; skip it otherwise, and never hand-write a `data/models/` file.
5. Derive a cassette with `scripts/generate_thin_wrapper_cassettes.py` if the
   vendor is a byte-for-byte OpenAI wrapper, or record a real one. Then flip
   `tier` to `verified`.
6. Add the five conformance cases (`aimux-providers/tests/conformance_test.rs`):
   streaming, tool calls, abort, an empty message, and context overflow.

## Case 2 — a new wire protocol

Only when the vendor is not OpenAI-compatible and no existing L0 protocol fits.

1. New directory `aimux-providers/src/<protocol>/` implementing the protocol
   against `aimux-core`'s `Provider` / `LanguageModel` traits, wired into
   `lib.rs` under a section comment (`gen_providers_doc.py` reads those).
2. One L1 row per vendor speaking that protocol.
3. Conformance cases as in case 1, plus cassettes for the protocol itself.

Expect review on the protocol, not on the vendor rows.

## Case 3 — a single-modality vendor (image, speech, transcription, video)

Do **not** hand-write a `do_generate` with its own headers, URL assembly and
polling loop. Use the executor pattern from
[RFC-0033 §8.2](../../rfc/0033-code-convergence-plan.md): a
`XxxConfig { api_key, base_url }` plus two transform functions, with
`call_json` / `submit_and_poll` in `aimux-provider-utils` owning the HTTP,
retries and polling. `submit_and_poll` retries only the poll, which is what
keeps a retry from submitting a billable job twice.

## Maintenance (you do not run these; the clock does)

`scripts/sync_registry.py --report` diffs L1 against upstream weekly and
comments on issue #170. `scripts/probe_registry.py` probes every `base_url`
monthly and reports what no longer answers. Neither one edits the registry —
a human reads the report and changes the row.
