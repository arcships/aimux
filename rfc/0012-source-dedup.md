# RFC-0012: Source code streamlining proposal

> **Status**: DRAFT (pending review)
> **Date**: 2026-07-31
> **Scope**: `aimux-providers`, `aimux-ffi`, and the generation mechanism of `aimux-providers/src/lib.rs`
> **Related**: [Rust architecture audit report](../docs/rust-architecture-audit-2026-07-31.md), [Provider adaptation layer improvements](0002-provider-improvements.md), [Provider development specification](0006-provider-development.md)

## 1. Goal

Eliminate source-code redundancy at the architectural level, and curb the slope of subsequent feature bloat. **Binary size is not a goal** — LTO already eliminates unreferenced code, and binary size is determined by the protocol engines actually linked in, independent of the source line count.

Core constraints:

1. **The unified-support principle is unchanged** — no crate splitting, no feature gates, no reduction in provider count.
2. **Tests are not touched** — the existing 125 test files (74,014 lines) are not modified, deleted, or merged. All acceptance is predicated on `cargo test --workspace --no-fail-fast` passing in full.
3. **The public API is unchanged** — the externally exported `XxxConfig` and `XxxProvider` type names and constructors remain unchanged, with zero perceptibility to downstream code.

## 2. Current state

| Metric | Value |
|---|---:|
| Product source | 68,362 lines / 433 files |
| Thin wrapper | 21,469 lines / 293 files |
| `lib.rs` registration statements | 650 entries (325 `pub mod` + 325 `pub use`) / 737 lines |
| FFI single file | 893 lines / 1 file |
| Responses variant duplication | ~7,400 lines / 7 files |
| Anthropic AWS duplication | ~650 lines / 1 file |

## 3. Streamlining items

### 3.1 Thin-wrapper manifest + macro generation

**Problem**: 293 files are structurally isomorphic, with the only real differences being 3 constants (`DEFAULT_BASE_URL`, `ENV_VAR`, `PROVIDER_NAME`) and the profile selection. After conservative normalization, 248 files and 16,965 lines fall into 11 groups of structural duplication.

**Solution**:

1. Create a new declarative macro in `aimux-providers/src/openai_compat.rs`:

```rust
macro_rules! declare_openai_compat_provider {
    ($name:ident, $display:literal, $base_url:literal, $env_var:literal, $profile:expr) => {
        pub struct ${concat($name, Config)}(OpenAIConfig);

        impl ${concat($name, Config)} {
            pub fn new(api_key: impl Into<String>) -> Self {
                Self(
                    OpenAIConfig::new(api_key)
                        .with_base_url($base_url)
                        .with_provider(stringify!($name))
                        .with_profile($profile),
                )
            }

            pub fn from_env() -> Result<Self, AiMuxError> {
                let key = load_api_key(None, $env_var, $display)?;
                Ok(Self::new(key))
            }

            pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
                self.0 = self.0.with_base_url(url);
                self
            }
        }

        pub struct ${concat($name, Provider)}(OpenAIProvider);

        impl ${concat($name, Provider)} {
            pub fn new(config: ${concat($name, Config)}) -> Self {
                Self(OpenAIProvider::new(config.0))
            }

            pub fn model(&self, model_id: &str) -> OpenAIModel {
                self.0.model(model_id)
            }
        }

        impl Provider for ${concat($name, Provider)} {
            fn name(&self) -> &str { stringify!($name) }
            fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
                Ok(Box::new(self.model(model_id)))
            }
        }
    };
}
```

2. In `aimux-providers/src/openai_compat_registry.rs`, use a declaration table to invoke the macro:

```rust
declare_openai_compat_provider!(ai21, "AI21 Labs", "https://api.ai21.ai/v1", "AI21_API_KEY", OpenAICompatProfile::full());
declare_openai_compat_provider!(groq, "Groq", "https://api.groq.com/openai/v1", "GROQ_API_KEY", OpenAICompatProfile::groq());
declare_openai_compat_provider!(deepseek, "DeepSeek", "https://api.deepseek.com/v1", "DEEPSEEK_API_KEY", OpenAICompatProfile::deepseek());
// ... 293 lines, one provider per line
```

3. `lib.rs` changes to:

```rust
mod openai_compat_registry;
pub use openai_compat_registry::*;  // one line replaces 518 lines of pub use
```

**Preserve the public API**: Type names and constructors such as `Ai21Config`, `Ai21Provider`, `GroqConfig`, `GroqProvider` remain unchanged.

**Expected result**:

| | Before | After |
|---|---:|---:|
| File count | 293 | 4 (macro + registry + 2 retained files) |
| Line count | 21,469 | ~1,330 (macro ~50 + registry ~1,250 + 2 retained files ~30) |
| Net reduction | | **-20,139 lines / -289 files** |

**Providers not applicable to this solution**: In practice, only 2 thin wrappers have extra methods — `openrouter.rs` and `huggingface.rs` (both have `responses_model`). These two retain independent files and are not included in macro generation. The remaining 281 pure `model()` thin wrappers are all generated by the macro.

### 3.2 Auto-generation of `lib.rs` root registration

**Problem**: Of the 737 lines, 650 are mechanical `pub mod` + `pub use`, and each provider requires changes in two places.

**Solution**:

- Thin-wrapper part: `pub use openai_compat_registry::*;` — one line replaces 518 lines.
- Native-protocol and modality-specific providers: keep hand-written `pub mod` + `pub use`, because the number is limited (~30) and each has its own exported types.
- When adding a compatible provider, only `openai_compat_registry.rs` needs to change in one place.

**Expected result**:

| | Before | After |
|---|---:|---:|
| `lib.rs` line count | 737 | ~80 |
| Change points for adding a provider | 2 (mod + use) | 0 (one line in the registry) |

### 3.3 FFI duplicate-pattern extraction

**Problem**: [`aimux-ffi/src/lib.rs`](../aimux-ffi/src/lib.rs) has 893 lines, with 20 duplicated `cstr_to_string` two-argument unpacking sites and 10 duplicated `block_on → serialize → CString` patterns.

**Solution**:

1. Extract common helpers:

```rust
/// Constructs (key, model_id) from two C strings; returns None on failure.
///
/// # Safety
///
/// The caller must ensure that `a` and `b` are either null or point to valid
/// NUL-terminated C strings.
/// Internally the function reads the strings safely via `CStr::from_ptr`, but
/// pointer validity is the caller's responsibility.
unsafe fn parse_two_args(a: *const c_char, b: *const c_char) -> Option<(String, String)> {
    match (cstr_to_string(a), cstr_to_string(b)) {
        (Some(k), Some(m)) => Some((k, m)),
        _ => None,
    }
}

/// Runs an async operation and returns a JSON string (caller must free).
fn run_and_serialize<F, T>(model_msg: &str, f: F) -> *mut c_char
where
    F: std::future::Future<Output = Result<T, AiMuxError>>,
    T: serde::Serialize,
{
    let result = runtime().block_on(f);
    match result {
        Ok(r) => serde_json::to_string(&r)
            .map(into_cstring_raw)
            .unwrap_or_else(|e| error_json_raw(format!("serialize: {e}"))),
        Err(e) => error_json_raw(format!("{}: {e}", model_msg)),
    }
}

/// Parses the base_url argument; an empty string is treated as unset.
fn parse_base_url(base_url: *const c_char) -> Option<String> {
    cstr_to_string(base_url).filter(|url| !url.is_empty())
}
```

2. Each `extern "C"` function shrinks to a 2–4 line call.

**No ABI change**: The `#[unsafe(no_mangle)] pub extern "C" fn` signatures remain completely unchanged.

**Expected result**:

| | Before | After |
|---|---:|---:|
| `aimux-ffi/src/lib.rs` line count | 893 | ~450 |
| Net reduction | | **-443 lines** |

### 3.4 Anthropic AWS merge

**Problem**: The streaming loop in [`aimux-providers/src/anthropic_aws/model.rs`](../aimux-providers/src/anthropic_aws/model.rs) (650 lines) is almost verbatim duplicated from [`aimux-providers/src/anthropic/model.rs`](../aimux-providers/src/anthropic/model.rs). The only differences are SigV4 authentication and the `HttpBody::Bytes` sending method.

**Solution**:

1. Extract an `anthropic_stream_reducer` function in `anthropic/model.rs` that takes an `Fn(&[u8], &str, &str) -> Vec<(String, String)>` for header construction (the standard path returns a Bearer header, the AWS path returns a SigV4-signed header).
2. `anthropic_aws/model.rs` calls the same reducer, overriding only `build_headers` and body encoding.

**Expected result**:

| | Before | After |
|---|---:|---:|
| `anthropic_aws/model.rs` | 650 lines | ~200 lines |
| Net reduction | | **-450 lines** |

### 3.5 Responses API variant merge

**Problem**: 7 files each implement the Responses API conversion; the structures are highly similar but have provider-specific differences:

| File | Lines |
|---|---:|
| `open_responses.rs` | 1,290 |
| `huggingface/responses.rs` | 1,196 |
| `azure/responses.rs` | 1,106 |
| `openai/responses/mod.rs` | 969 |
| `openai/responses/convert.rs` | 1,088 |
| `xai/responses/mod.rs` | 954 |
| `xai/responses/convert.rs` | 819 |
| **Total** | **7,422** |

**Solution**:

1. Under `aimux-providers/src/openai/responses/`, extract a shared `responses_convert.rs` containing the common logic for request-body construction, streaming-event parsing, and usage extraction.
2. Each provider retains only the difference overrides: endpoint concatenation, model-id mapping, and provider-specific fields.
3. Do not force a merge into a single function — each provider's responses implementation has real protocol differences; only extract the shared framework.

**Expected result**:

| | Before | After |
|---|---:|---:|
| Responses variants | ~7,400 lines / 7 files | ~4,000 lines / 4 files |
| Net reduction | | **-3,400 lines / -3 files** (estimated; a line-by-line similarity audit must be done first to confirm before implementation) |

## 4. Things not to do

| Not doing | Reason |
|---|---|
| Split the `aimux-providers` crate | Unified-support principle |
| Introduce Cargo feature gates | Unified-support principle |
| Modify any test files | Explicit constraint |
| Merge native protocol engines (openai/anthropic/google/bedrock etc.) | Each provider's protocol differences are real complexity, not redundancy |
| Merge modality-specific providers (TTS/STT/image/video) | Each provider's API differences are large and cannot be shared |
| Modify public API type names or constructors | Zero perceptibility to downstream |
| Pursue binary-size optimization | This proposal does not target size |

## 5. Acceptance criteria

### 5.1 Functional acceptance

- [ ] `cargo test --workspace --no-fail-fast` passes in full, 0 failures.
- [ ] The number of test files under `tests/` is unchanged (125), and the number of test lines is unchanged (74,014).
- [ ] Adding inline `#[cfg(test)]` assertions inside `src/` files (e.g. to verify that macro-generated types exist) is allowed and is not counted toward the 125 files above.
- [ ] All `XxxConfig` and `XxxProvider` types can still be imported from `aimux_providers`, with unchanged constructor signatures.
- [ ] All `#[unsafe(no_mangle)] pub extern "C" fn aimux_*` symbols still exist, with unchanged signatures.

### 5.2 Scale acceptance

- [ ] The number of `.rs` files under `aimux-providers/src/` drops from 388 to ~100.
- [ ] The line count of `aimux-providers/src/lib.rs` drops from 737 to ~80.
- [ ] The line count of `aimux-ffi/src/lib.rs` drops from 893 to ~450.
- [ ] The number of files matching the complete thin-wrapper skeleton drops from 293 to 0 (all generated by the macro).

### 5.3 Quality acceptance

- [ ] `cargo check --workspace --all-targets` 0 errors.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` 0 errors.
- [ ] `cargo fmt --all -- --check` 0 diffs.

### 5.4 Convergence acceptance

- [ ] Adding an OpenAI-compatible provider only requires adding 1 line in `openai_compat_registry.rs`, with no new file and no change to `lib.rs`.
- [ ] No new single-file thin wrappers appear under `aimux-providers/src/`.

## 6. Implementation order

```text
3.1 thin-wrapper manifest + macro  →  3.2 lib.rs auto-generation  →  3.3 FFI helper extraction  →  3.4 Anthropic AWS merge  →  3.5 Responses merge
```

Each step is independently verifiable. 3.1 and 3.2 must be completed consecutively (3.2 depends on 3.1's registry). 3.3, 3.4, and 3.5 are independent of each other and can be done in parallel.

## 7. Expected results

| Metric | Current | Target | Net reduction |
|---|---:|---:|---:|
| Product source line count | 68,362 | ~43,243 | -25,119 (-37%) |
| Product source file count | 433 | ~140 | -293 |
| `lib.rs` line count | 737 | ~80 | -657 |
| FFI line count | 893 | ~450 | -443 |
| Cost of adding a compatible provider | 1 file / ~65 lines | 1 line of manifest | -99.5% |
| Test line count | 74,014 | 74,014 (unchanged) | 0 |

## 8. Risks

| Risk | Mitigation |
|---|---|
| Macro-generated type names inconsistent with hand-written ones | Use compile tests to assert that every exported type exists |
| The `${concat}` macro unavailable on older Rust versions | The project MSRV is 1.85, which supports `${concat}`; CI is pinned to stable |
| Some thin wrappers have hidden differences (e.g. extra methods) | Providers not applicable to the macro retain independent files |
| Behavior drift after the Anthropic AWS merge | Cassette replay tests cover streaming behavior |
| Loss of provider differences after the Responses merge | Retain difference overrides per provider + individual tests |

## 9. Changelog

| Date | Notes |
|---|---|
| 2026-07-31 | Initial version, formulated from the redundancy data in the architecture audit report |
