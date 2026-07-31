# RFC 0005: Rename to aimux

> **Status**: Decided plan, pending execution
> **Decision**: Error type `AiMuxError` · repository name `aimux` · brand positioning "inspired by Vercel AI SDK" · script batch execution + verification

## 1. Background and Motivation

The current project name is `aimux` / `aimux`. There are two motivations for the rename:

1. **Brand identity**: The concatenated form `aimux` easily leads people to associate it with the identifier of Vercel AI SDK. Although strictly speaking Vercel's official name is "AI SDK", the npm package is `ai`, and the repository is `vercel/ai`, `aimux` is not its registered identifier, but the conceptual association is too strong, which is detrimental to establishing an independent brand.
2. **Rust naming conventions**: The convention for Rust crates is kebab-case word separation (`ai-sdk-core`); the current `aimux-core` glues ai and sdk together, which is non-standard.

## 2. Rationale for the Name Selection

After multiple rounds of screening (see session records), **`aimux`** was finally selected:

- **Most accurate architectural semantics**: mux (multiplexer) = switching among multiple signal sources behind a single input interface, which literally corresponds to this project's architecture of "a unified `LanguageModel` trait fanning out to multiple providers".
- **Short**: 5 letters, the `aimux::` prefix is clean.
- **Contains the AI keyword**: The domain is recognizable at a glance.
- **Available on crates.io**: The primary name `aimux` and the sub-crate namespaces `aimux-core`/`aimux-providers` etc. are all available (verified one by one).
- **Scarcity in the space**: The straightforward candidates `llm-mux`/`modelmux`/`llm-bridge`/`llmkit`/`aikit`/`llm-rs`/`modelforge` are all occupied by similar competing products; `aimux` is one of the few survivors that is both short, accurate, and available.

Excluded candidates and reasons:
- `duplex` (duplex): The semantics emphasize two-way (2-way) communication, which does not match the 1→N fan-out architecture; moreover, the bare `duplex` is already taken (sunfishcode's IO trait crate, 62k downloads).
- `aireq`/`aifetch`/`airun`/`modelrun` (action verbs): They only point to a single action, missing the core selling point of "unified multiple providers + the full text/object/tool/streaming bundle".
- `oxide-ai`/`prism-ai` (metaphor category): Available but do not contain the AI keyword.

## 3. Complete Name Mapping Table

| Aspect | Current | Target |
|------|------|------|
| Repository name / root directory | `aimux` | `aimux` |
| crate names (6) | `aimux-core` etc. | `aimux-core` / `aimux-stream` / `aimux-providers` / `aimux-provider-utils` |
| directory names (6) | `aimux-core/` etc. | `aimux-core/` etc. |
| code identifiers (underscores) | `aimux_core` / `aimux_providers` / `aimux_stream` / `aimux_provider_utils` | `aimux_core` / `aimux_providers` / `aimux_stream` / `aimux_provider_utils` |
| core error type | `AiMuxError` (spread across 25+ files) | `AiMuxError` |
| test environment variable | `AISDK_TEST_LOAD_API_KEY_VAR` | `AIMUX_TEST_LOAD_API_KEY_VAR` |
| brand positioning | "Benchmark against Vercel AI SDK / high-performance alternative" | "Rust implementation inspired by Vercel AI SDK" |

## 4. Impact Scope Layering

### Layer 1: Directory rename (6)

```
aimux-core/          → aimux-core/
aimux-stream/        → aimux-stream/
aimux-providers/     → aimux-providers/
aimux-provider-utils/→ aimux-provider-utils/
```

Use `git mv` to preserve history.

### Layer 2: Cargo.toml (7 files)

- **Root `Cargo.toml`**:
  - `members` list: 6 entries
  - 4 `aimux-*` references in `workspace.dependencies`
  - `repository` URL (`github.com/yourusername/aimux`)
  - `description`
- **`Cargo.toml` of the 6 sub-crates**:
  - `name = "aimux-xxx"` → `aimux-xxx`
  - `description` strings
  - `aimux-*` references in `dependencies` / `dev-dependencies`

### Layer 3: Rust source code (100+ files)

- `use aimux_core::` → `use aimux_core::` (including the full-path form `aimux_core::error::AiMuxError`)
- `aimux_providers::` / `aimux_tools::` / `aimux_macros::` likewise
- `AiMuxError` → `AiMuxError` (core change, 25+ files, including the enum definition, impl, all uses and references)
> **Note: aimux-tools and aimux-macros were deleted on 2026-07-31**; the tool-related content below is outdated and retained only as a historical record.

- The **generated code** inside the `quote!` macro in `aimux-macros/src/lib.rs` references `aimux_tools::ToolFn` and `aimux_core::error::AiMuxError` — these must be changed in sync, otherwise users will fail to compile after the `#[tool]` macro expands.
- The crate doc comments at the top of each lib.rs, e.g. `//! # aimux-core`

### Layer 4: Documentation (10+ files)

- `README.md`: title, directory tree, code examples, comparison tables, brand positioning statements
- `HANDOFF.md` / `QUALITY_REVIEW.md` / `REMEDIATION.md` / `TEST_AUDIT.md` / `TRACKING.md`: file path references, `AiMuxError` mentions
- `docs/01~14` + `docs/README.md`: a few mentions
- `rfc/0001-multilang-bindings.md` / `rfc/0004-provider-inventory.md`: relative path links `../aimux-core` etc. (links become invalid after the directory rename and must be changed in sync)

### Layer 5: Scripts

- `scripts/convert_cassettes.py:13`: `OUT_DIR = Path("aimux-providers/tests/cassettes")` → `aimux-providers/tests/cassettes`

## 5. Explicitly Untouched Items (Boundaries)

| Item | Reason |
|----|------|
| the entire `reference/` directory | Third-party reference projects (TokenHub / traceloop-hub / uni-api / unia / rig cassettes), not this project's code |
| `AISDKError` in `docs/07-kernel-infrastructure.md` | Describes the Vercel AI SDK native class `@ai-sdk/provider.AISDKError`; this is an upstream fact, not a symbol of this project |
| `"Versal AISDK"` in `aimux-providers/tests/fal_transcription_test.rs` | Mock provider return text (a fictitious service name), unrelated to the project rename |
| `Cargo.lock` | Auto-regenerated by `cargo check` after directory/crate rename |
| `.githooks/` (pre-commit / pre-push) | Does not contain the aimux string |
| `rust-toolchain.toml` / `rustfmt.toml` / `.gitattributes` / `.gitignore` | Unrelated to naming |

## 6. Key Risks

1. **Macro-generated code** (the `quote!` in `aimux-macros/src/lib.rs`): The replacement must be synchronized in both `aimux_tools` and `aimux_core::error::AiMuxError`, otherwise `#[tool]` macro users will fail to compile. The current README marks the macros subsystem as "pending rewrite, not yet closed", but the rename must still keep references consistent.
2. **Relative path links**: The `../aimux-core` links in `rfc/` become invalid after the directory rename.
3. **Collateral damage from script batch replacement**: The script must **exclude `reference/`**, otherwise it will also change the description of the Vercel native `AISDKError` and the mock data.
4. **Case sensitivity**: The three forms `aimux` (lowercase) / `AiMux` (camelCase) / `AISDK` (all uppercase) must be handled separately; a single regex cannot cover all. Specific replacement rules:
   - `aimux` → `aimux`
   - `AiMux` → `AiMux`
   - `AISDK` → `AIMUX`
   - `aimux` → `aimux` (the repository name drops -rs; note this replacement must be done before `aimux`→`aimux` or handled separately, to avoid double replacement)

## 7. Execution Steps (Script Batch Approach)

### Step 1: Directory rename

```powershell
git mv aimux-core aimux-core
git mv aimux-stream aimux-stream
git mv aimux-tools aimux-tools
git mv aimux-macros aimux-macros
git mv aimux-providers aimux-providers
git mv aimux-provider-utils aimux-provider-utils
```

### Step 2: Batch text replacement (PowerShell script)

The script must satisfy:
- Scope: `Cargo.toml`, `*.rs`, `*.md`, `scripts/*.py` under the root directory
- **Exclude the `reference/` directory**
- **Exclude the `target/` directory**
- Case-sensitive, performing three groups of replacements: `aimux`→`aimux`, `AiMux`→`AiMux`, `AISDK`→`AIMUX`
- `aimux` (repository name) must be handled separately as `aimux`; note the order: first replace `aimux`→`aimux`, then replace `aimux`→`aimux`, to avoid `aimux` being split into `aimux-rs`

Pseudocode:
```powershell
$root = "C:\Users\eric8\Desktop\code\aimux"  # Note: the root directory itself is not renamed for now; the user handles it later via git remote/rename
$files = Get-ChildItem -Recurse -File $root -Include *.rs,*.toml,*.md,*.py |
  Where-Object { $_.FullName -notmatch '\\reference\\' -and $_.FullName -notmatch '\\target\\' -and $_.FullName -notmatch '\\.git\\' }

foreach ($f in $files) {
    $content = Get-Content $f.FullName -Raw -Encoding UTF8
    # Handle repository name first (note the order)
    $content = $content -replace 'aimux', 'aimux'
    # Three case forms
    $content = $content -replace 'aimux', 'aimux'
    $content = $content -replace 'AiMux', 'AiMux'
    $content = $content -replace 'AISDK', 'AIMUX'
    Set-Content $f.FullName -Value $content -NoNewline -Encoding UTF8
}
```

### Step 3: Manually check the boundaries

- Confirm the `reference/` directory was not touched
- Confirm that `AISDKError` (the description of the Vercel native class) in `docs/07` was not mistakenly changed — **this item must be manually grep-confirmed after the script**, because `AISDK`→`AIMUX` will cause collateral damage. Handling: after the script runs, manually change `AIMUXError` (which describes the Vercel native class) in `docs/07` back to `AISDKError`.
- Confirm that the mock text `"Versal AISDK"` in `fal_transcription_test.rs` was not mistakenly changed — same as above, `AISDK`→`AIMUX` will cause collateral damage and must be manually changed back to `"Versal AISDK"`.

### Step 4: Verification

```powershell
cargo check --workspace --all-targets
cargo test --workspace
```

`.githooks/pre-commit` will automatically run `cargo fmt --all --check` + `cargo check --workspace` as a fallback.

### Step 5: Root directory / repository rename (handled by the user)

When the scripts within the scope of this RFC are executed, the root directory name remains `aimux`. The repository rename (`git remote set-url`, GitHub repo rename, local directory rename) is performed by the user after the script verification passes, and is not within the scope of automation.

## 8. Estimated Workload

- Script execution + manual boundary check: 5-10 minutes
- `cargo check` first compilation verification: 2-5 minutes
- `cargo test`: depends on the test suite size, 5-15 minutes
- Total: about 15-30 minutes

## 9. Rollback

If verification fails:
- `git status` to view changes
- `git restore --staged .` + `git checkout .` to revert (revert directory renames with `git mv`)
- Or directly `git reset --hard HEAD` (note this will lose other uncommitted changes)
