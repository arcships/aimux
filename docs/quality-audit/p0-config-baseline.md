# P0 quality configuration baseline

Date: 2026-08-06

## Judgment

- `rustfmt.toml` now declares Rust 2024, matching the workspace package edition.
- The root manifest defines `[workspace.lints.clippy] all = "warn"`, establishing the currently clean Clippy baseline without changing CI behavior.
- `unsafe_code = "warn"` was **not** retained in the workspace baseline. A command-line validation found 107 `unsafe_code` warnings across workspace code and test targets (primarily Rust 2024-required `std::env::{set_var, remove_var}` test setup). Adding it would make CI's `-D warnings` fail. A crate-wide `aimux-ffi` allowance would also fail to address non-FFI occurrences and would be too broad.
- `clippy::pedantic` is not enabled. It produces 3,669 diagnostic headers in the full `--all-targets` run, including duplicate compilation-target diagnostics; enabling it would immediately break CI.

## Final configuration

```toml
# rustfmt.toml
edition = "2024"

# Cargo.toml
[workspace.lints.clippy]
all = "warn"
```

Workspace lint tables are inherited only by member manifests that opt in using:

```toml
[lints]
workspace = true
```

The present member manifests do not yet contain that opt-in, so Cargo does not apply the table to current packages. CI equivalence is preserved by its authoritative command, `cargo clippy --workspace --all-targets -- -D warnings`, which passed below. Introducing the per-member inheritance stanzas was outside this task's allowed file scope.

## Verification

All commands ran from the repository root and completed successfully unless noted.

| Command | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed, no output. |
| `cargo clippy --workspace --all-targets` | Passed, zero raw warnings. |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed, matching CI's deny-warnings gate. |
| `cargo clippy --workspace --all-targets -- -W unsafe_code` | Completed with 107 warnings; configuration intentionally omitted. |
| `cargo clippy --workspace --all-targets -- -W clippy::pedantic` | Completed with 3,669 diagnostic headers; configuration intentionally omitted. |

`cargo fmt --all -- --check` was run after the formatter edition edit. The two standard Clippy commands were run after adding the final lint configuration. No source formatting changes were required.

## Pedantic assessment and rollout proposal

The 3,669 headers include duplicate diagnostics from compiling the same library for test targets. The run reported 53 distinct pedantic lint kinds. Largest categories were:

- `doc_markdown`: 997
- `uninlined_format_args`: 576
- `must_use_candidate`: 411
- `redundant_closure_for_method_calls`: 298
- `unreadable_literal`: 276
- `return_self_not_must_use`: 147
- `missing_errors_doc`: 122
- `cast_possible_truncation`: 100

The provider crate accounts for the majority (1,906 warnings for its normal library compilation; 1,910 for its lib-test compilation). Therefore, do not set `pedantic = "warn"` at workspace scope yet.

Suggested staged adoption:

1. Fix mechanical, low-risk cases automatically or in focused batches: `uninlined_format_args`, `doc_markdown`, redundant closures, and `must_use` annotations after API review.
2. Review semantic/API-facing lints (`missing_errors_doc`, casts, `too_many_lines`, boolean-parameter and naming lints) crate by crate; use narrowly scoped documented `allow`s only where the design is intentional.
3. Add a non-blocking dedicated pedantic CI job and reduce its clean library-target baseline before considering workspace-level enforcement. Keep test targets as a separate later phase to avoid generated/macro-heavy test noise blocking production code work.

## Post-task follow-up: member opt-in (completed)

The member manifests have since been updated with `[lints] workspace = true`:

- `aimux-core/Cargo.toml`
- `aimux-providers/Cargo.toml`
- `aimux-stream/Cargo.toml`
- `aimux-ffi/Cargo.toml`
- `aimux-provider-utils/Cargo.toml`
- `scripts/fix_tool/Cargo.toml`

Re-verification after opt-in:

| Command | Result |
| --- | --- |
| `cargo clippy --workspace --all-targets` | Passed, zero raw warnings. |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed, matching CI's deny-warnings gate. |
| `cargo fmt --all -- --check` | Passed. |

The workspace lint baseline is now active for all member crates.

## Remaining uncertainty

- The 107 `unsafe_code` warnings should be triaged separately. They are not limited to the FFI boundary, so adding only `#![allow(unsafe_code)]` to `aimux-ffi` would not achieve a warning-free workspace. A future policy should distinguish intentional FFI pointer operations from synchronized environment-variable mutation in tests.
