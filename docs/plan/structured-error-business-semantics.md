---
id: structured-error-business-semantics
scope: aimux-core, aimux-provider-utils, aimux-providers, RFC error semantics
status: ready
depends-on: [feat/structured-error-fields]
executor: Claude
---

# Structured error business-semantics completion plan

## 1. Objective

Finish the **business/runtime semantics** of the structured-error redesign on
`feat/structured-error-fields`.

The goal is not merely that every old enum variant has a new spelling. The
runtime must construct the correct error kind, preserve the provider facts it
actually observed, and make retry/timeout/abort behavior agree with the RFCs.

This plan deliberately does **not** spend time on backward compatibility or on
adding richer binding APIs. Binding files may receive mechanical changes needed
to compile or to reflect the final core wire type, but no compatibility shim is
required in this branch.

## 2. Authoritative decisions

Use these decisions when code, old tests, comments, and RFC acceptance wording
disagree.

### 2.1 RFC-derived rules

| Source | Required behavior |
|---|---|
| RFC-0009 §4.2 | Retry delay prefers a real provider `retry-after-ms` / `retry-after` hint; when absent, use local exponential backoff with jitter. Never store a local fallback as provider data. |
| RFC-0014 §3.3 | HTTP error handling and retry observation belong in the shared HTTP choke point, not duplicated across providers. |
| RFC-0016 §7.1 | Request deadlines surface as `AiMuxError::Timeout` and are not retried. |
| RFC-0016 §7.6 R2/R4 and §7.7 S1/S2 | User cancellation surfaces as `AiMuxError::Aborted`; abort must remain responsive during body reads, retry backoff, and provider polling waits. |
| RFC-0016 alignment goal | Retryability follows the AI SDK response policy: 408, 409, 429, and 5xx are retryable; ordinary 4xx are not. |
| RFC-0017 §3 stage 4 | Unknown registry names must identify the requested provider and direct callers to the generated `ProviderName` surface. The runtime error must not carry all 250 automatically registered names. |
| RFC-0018 §3.2 | Codex subscription 401 remains the specialized `TokenExpired` action signal. Do not fold it back into a generic 401 `ApiCall`. |

### 2.2 Final error classification table

| Observed failure | Error kind | Required fields/behavior |
|---|---|---|
| Request could not be sent, DNS/TLS/connect/body transport failed | `ApiCall` | `status_code=None`, `is_retryable=true`, transport message; no invented response fields |
| Non-2xx HTTP response | `ApiCall` | Actual status, extracted provider message/code, raw body when present, request id when present, actual retry hint when present |
| 408 / 409 / 429 / 5xx HTTP response | `ApiCall` | Same facts as above and `is_retryable=true`; shared retry loop must actually retry it |
| Other 4xx response | `ApiCall` | `is_retryable=false`; return immediately |
| Provider returns HTTP 2xx but explicitly reports a domain failure (`failed`, `cancelled`, non-zero provider code, moderation rejection, in-band error object) | `ApiCall` | Preserve the observed 2xx status, provider code/domain state when available, and raw body; do not mislabel it as malformed data |
| JSON bytes are syntactically invalid/truncated | `JsonParse` | Parser diagnostic; do not claim a provider/HTTP failure kind merely because decoding happened after HTTP |
| JSON is valid but violates the expected response shape, lacks required output, contains an invalid URL/base64 value, or otherwise cannot be converted into the documented result | `InvalidResponseData` | Validation/conversion diagnostic; do not manufacture `ApiCall(status=200)` for this category |
| Caller-configured or provider-polling deadline expires | `Timeout` | Human-readable phase/provider context; non-retryable at the outer operation level |
| Caller aborts at any stage | `Aborted` | No string wrapper and no `Other("...aborted")` |
| Codex subscription endpoint returns 401 | `TokenExpired` | Preserve RFC-0018 refresh-and-retry contract |
| Registry lookup misses a provider | `NoSuchProvider { provider_id }` | Display generated from the fact; no `available` vector and no stored `message` |

## 3. Non-goals and guardrails

1. Do not add compatibility deserializers for retired variants or old
   `NoSuchProvider` payload shapes.
2. Do not add new public fields to C/Go/Java/Kotlin/Swift/Flutter exceptions.
3. Do not redesign the C ABI or use its reserved pointer.
4. Do not add `url`, `requestBodyValues`, or `responseHeaders` to
   `ApiCallError` in this task.
5. Do not replace `TokenExpired` or change the RFC-0018 OAuth responsibility
   split.
6. Do not blanket-replace every `ApiCall(status=200)`: provider-declared
   in-band failures remain `ApiCall`; malformed response data does not.
7. Do not hide provider response bodies in human-readable messages. Raw bodies
   belong in structured fields where the selected error type supports them.
8. Do not write a broad new test suite. Prefer existing tests plus a small
   number of high-value shared-path regressions listed in §9.
9. Preserve unrelated worktree changes. This branch already contains user
   edits to `error.rs`, `provider.rs`, FFI mapping, and generated/binding files.

## 4. Work package A — finalize `NoSuchProvider`

### Files

- `aimux-core/src/error.rs`
- `aimux-providers/src/provider.rs`
- mechanical compile/wire fallout only:
  - `aimux-ffi/src/lib.rs`
  - `bindings/node/src/types/AiMuxError.ts`
  - existing binding/core golden tests that construct the variant
- `rfc/0017-provider-config-dx.md`

### Changes

1. Replace the current payload with the single fact:

   ```rust
   #[error("No such provider: {provider_id}")]
   NoSuchProvider { provider_id: String }
   ```

2. Remove both `available` and stored `message`. Display text must be derived
   from `provider_id`, so the payload cannot contain contradictory facts.
3. In `provider_handle`, construct only `provider_id`.
4. Remove the now-unused `ProviderName` import and fix the stale `provider()`
   doc comment that still promises the full provider list.
5. Update RFC-0017 stage-4 wording and acceptance item. The accepted behavior
   is: the error names the unknown provider; discoverable valid names come from
   generated `ProviderName`. The error payload does not contain 250 names.
6. Regenerate or mechanically update the checked-in TS wire type. Do not add a
   compatibility union for the old payload.

### Acceptance

- Unknown `"not-real"` displays exactly `No such provider: not-real`.
- Serialized value contains `provider_id` and no `available`/`message`.
- No production code builds a 250-name string/vector for an error.

## 5. Work package B — unify all non-2xx handling in the shared HTTP loop

### Files and functions

- `aimux-provider-utils/src/http.rs`
  - `send_with_retry_raw`
- `aimux-provider-utils/src/response.rs`
  - `error_for_status`
  - `parse_provider_error`

### Current defects

- 429 and 5xx hand-build `ApiCallError` and bypass
  `parse_provider_error`, losing extracted `provider_code` and provider message.
- A hint-less 429 stores a fabricated `retry_after_ms=1000`.
- `error_for_status` can mark a status retryable, but the HTTP loop has
  hard-coded status branches; changing the flag alone would not make 408/409
  retry.

### Target control flow

Refactor the non-success response branch into one fact-preserving path:

1. Read `status_code`.
2. Capture `x-request-id` / `request-id` before consuming the response.
3. Parse retry headers before consuming the response. Under the current core
   contract, store the distilled hint only for 429. Missing/invalid/negative
   values become `None`; never substitute 1000.
4. Capture redacted response headers for recording.
5. Read the capped error body once.
6. Call `parse_provider_error(status, body, error_structure)` for **every**
   non-2xx status, including 408/409/429/5xx.
7. If the result is `AiMuxError::ApiCall(detail)`, attach the request id and
   actual retry hint. Do not reconstruct the error and lose parsed fields.
8. Record the failed exchange once using the same final error value.
9. If `err.is_retryable()` is true, assign it to `last_error` and flow through
   the existing attempt-limit/backoff logic. Otherwise return immediately.

The resulting branch should not have separate hand-built 429 and 5xx
`ApiCallError` literals.

### Retry policy

Change `error_for_status` to:

```rust
matches!(status, 408 | 409 | 429) || status >= 500
```

Do not add 401/403/404 retry behavior. Do not retry `Timeout`, `Aborted`, or
`TokenExpired`.

### Acceptance

- JSON 429/5xx bodies preserve both extracted `message` and `provider_code`.
- Raw body and request id survive on all non-2xx paths.
- Hint-less 429 has `retry_after_ms=None` while the retry loop still backs off.
- 408 and 409 enter the retry loop; non-retryable 4xx return after one attempt.
- Recording/logging still records one failed exchange per attempt.

## 6. Work package C — central JSON decode classification

### Files

- `aimux-core/src/error.rs`
- provider response-decoding call sites under `aimux-providers/src/`

### Core rule

Make the canonical `From<serde_json::Error> for AiMuxError` classification
explicit:

```text
Category::Syntax | Category::Eof -> JsonParse
Category::Data                   -> InvalidResponseData
Category::Io                     -> JsonParse unless the call site has a
                                    concrete transport error to preserve
```

Keep the existing string payloads for this task. Do not redesign
`InvalidResponseData` into another large struct as part of this branch.

### Call-site migration

1. Replace response-decoding closures such as
   `map_err(|e| AiMuxError::JsonParse(e.to_string()))` with the canonical
   conversion where the operation is genuinely parsing provider response JSON.
2. For `serde_json::from_value<T>`, a failure is data/schema validation and
   should resolve to `InvalidResponseData` through the same conversion.
3. Do **not** mechanically change JSON serialization of outbound requests,
   registry/config parsing, replay fixtures, or user-authored JSON. Review the
   operation first; this package is about provider response decoding.
4. Explicit missing-field/output checks should use `InvalidResponseData`, not
   an `ApiCallError` with a success status.

### Required audit query

Run:

```bash
rg -n 'AiMuxError::JsonParse|serde_json::from_(slice|str|value)' \
  aimux-providers/src --glob '*.rs'
```

Classify each hit by operation; do not use a global textual replacement.

### Acceptance

- Truncated/malformed provider JSON returns `JsonParse`.
- Valid JSON missing required typed fields returns `InvalidResponseData`.
- Outbound serialization/config parsing behavior is not accidentally changed.

## 7. Work package D — audit all in-band 2xx error sites

The branch currently has 47 `status_code: Some(resp.status)`-style sites in 23
provider files. They are an audit inventory, not 47 automatic bugs.

### Required inventory

- `aimux-providers/src/assemblyai.rs`
- `aimux-providers/src/bedrock/image.rs`
- `aimux-providers/src/black_forest_labs.rs`
- `aimux-providers/src/codex.rs`
- `aimux-providers/src/gladia.rs`
- `aimux-providers/src/google/files.rs`
- `aimux-providers/src/google/model.rs`
- `aimux-providers/src/google/video.rs`
- `aimux-providers/src/huggingface/responses.rs`
- `aimux-providers/src/klingai.rs`
- `aimux-providers/src/luma.rs`
- `aimux-providers/src/mistral/model.rs`
- `aimux-providers/src/open_responses.rs`
- `aimux-providers/src/openai/model.rs`
- `aimux-providers/src/prodia.rs`
- `aimux-providers/src/replicate.rs`
- `aimux-providers/src/revai.rs`
- `aimux-providers/src/runwayml.rs`
- `aimux-providers/src/stability.rs`
- `aimux-providers/src/vertex/model.rs`
- `aimux-providers/src/vertex/video.rs`
- `aimux-providers/src/xai/model.rs`
- `aimux-providers/src/xai/responses/mod.rs`

Reproduce the inventory with:

```bash
rg -n 'status_code:\s*Some\((resp|poll_resp|poll_response|response)\.status\)' \
  aimux-providers/src --glob '*.rs'
```

### Keep as `ApiCall` with observed 2xx status

Keep cases where the provider deliberately reports an unsuccessful operation
inside a successful HTTP envelope, for example:

- explicit error object/string in a 2xx xAI/HuggingFace response;
- non-zero provider result code;
- asynchronous job state `failed`/`cancelled`;
- moderation/rejection reported as domain data.

Fill `provider_code` with the provider error code or stable domain state when
one exists. Keep `response_body`. Do not set `is_retryable=true` merely because
the provider task failed; only provider evidence/policy should enable retry.

### Change to `InvalidResponseData`

Change cases where the response is supposed to represent success but cannot be
converted into a usable result, including:

- missing `choices`, `candidates`, output/image, access token, job id,
  operation name, polling URL, or required result object;
- invalid provider-supplied URL or base64 payload;
- structurally invalid multipart response or missing multipart boundary;
- typed decoding failures classified as `serde_json::error::Category::Data`.

Representative current sites include OpenAI/Mistral/Google/Vertex “no
choices/candidates”, Stability missing/invalid image, Codex refresh missing
`access_token`, BFL missing/invalid polling URL, Replicate/Prodia missing job
ids, and Gladia returning a completed job without a result.

### Review result requirement

The implementation commit or PR description must include a short count:

```text
2xx audit: N kept as provider-declared ApiCall, M changed to
InvalidResponseData, K changed to Timeout/Aborted.
```

This prevents a silent blanket rewrite and makes review possible.

## 8. Work package E — provider polling timeout and abort semantics

### Known wrong timeout sites

- `aimux-providers/src/google/files.rs`
- `aimux-providers/src/black_forest_labs.rs`
- `aimux-providers/src/runwayml.rs`
- `aimux-providers/src/luma.rs`

Replace polling deadline exhaustion currently represented as bare `ApiCall`
with `AiMuxError::Timeout`, preserving provider/task/elapsed context in the
message.

### Known wrong abort site

`google/files.rs` currently returns
`Other("file upload polling aborted")`; return `AiMuxError::Aborted`.

### Abortable polling waits

Every explicit provider polling sleep must remain responsive to the supplied
`AbortSignal`. Audit these current sleep sites:

- Runway, Replicate, Black Forest Labs, Prodia, Fal, Rev.ai, Luma,
  Google Video, Gladia, KlingAI, Google Files, Vertex Video, AssemblyAI.

Prefer one small shared async helper in `aimux-provider-utils` if it avoids
copying `tokio::select!` into every provider. The helper contract should be:

```rust
sleep_or_abort(duration, Option<&AbortSignal>) -> Result<(), AiMuxError>
```

- already-aborted signals fail immediately;
- abort wins over the sleep when both are ready (`biased` select);
- no signal behaves exactly like `tokio::time::sleep`;
- returned cancellation is exactly `AiMuxError::Aborted`.

Do not add automatic retries around whole polling operations.

## 9. Verification strategy — deliberately small

Do not build a large new suite. Add or adjust only the following high-value
coverage:

1. **Shared non-2xx parameterized regression** in
   `aimux-provider-utils/tests/structured_error_fields.rs`:
   - 429 and 500 JSON extract message/code/body/request id;
   - a hint-less 429 retains `retry_after_ms=None`;
   - 408/409 are retryable while a representative 400 is not.
2. **One retry-loop integration case** in the existing HTTP send tests proving
   408 or 409 is retried and a later success is returned. One case is enough;
   status-policy unit coverage handles the other.
3. **Existing timeout/abort tests** should be extended only where necessary to
   cover the shared polling-wait helper. Do not add one test per provider.
4. Update existing `NoSuchProvider` golden/mapping assertions mechanically.

Run, in order:

```bash
cargo fmt --all -- --check
cargo check -p aimux-core -p aimux-provider-utils -p aimux-providers -p aimux-ffi
cargo test -p aimux-provider-utils --test structured_error_fields
cargo test -p aimux-provider-utils --test http_send_test
cargo test -p aimux-providers --test provider_error_test
cargo test -p aimux-providers --test codex_test
git diff --check
```

If a named test target has changed in the repository, use the nearest existing
focused target. Do not default to a full workspace test run unless the focused
checks reveal cross-crate fallout.

## 10. Documentation and debt cleanup in scope

Clean only residue directly invalidated by this work:

- stale `provider.rs` documentation promising the full provider list;
- RFC-0017 unknown-provider wording/acceptance contradiction;
- comments in `error.rs`, `response.rs`, and `http.rs` that still claim only
  429/5xx are retryable after 408/409 alignment;
- `scripts/gen_responses_convert.py` if it still emits a removed
  `AiMuxError::Provider` constructor;
- stale Rustdoc references to `AiMuxError::Unsupported` / `AiMuxError::Json`;
- the unused `ProviderName` import created by removing the available list.

Do not turn this into a general documentation sweep.

## 11. Suggested implementation order

1. Finalize the core taxonomy and `NoSuchProvider` shape.
2. Refactor shared non-2xx construction and retry branching.
3. Add the canonical serde error classification.
4. Audit and reclassify the 23 provider files.
5. Fix polling timeout/abort behavior and make polling sleeps abortable.
6. Apply mechanical FFI/generated-type fallout.
7. Update RFC/comments and remove directly related residue.
8. Run the focused verification list and report the 2xx audit counts.

Do not mix these into one mechanical search/replace commit. Recommended commit
boundaries are:

1. `fix(core,provider-utils): finalize structured error policy`
2. `fix(providers): classify 2xx response and polling failures`
3. `docs: reconcile structured error behavior with RFCs`

## 12. Definition of done

- [ ] `NoSuchProvider` stores only `provider_id` and never carries 250 names.
- [ ] Every non-2xx HTTP response goes through the provider error parser.
- [ ] 429/5xx retain parsed provider code/message, raw body, and request id.
- [ ] Missing retry headers never become fabricated structured hints.
- [ ] 408/409/429/5xx retry in the shared loop; ordinary 4xx do not.
- [ ] Provider-response JSON syntax and data/schema failures are distinct.
- [ ] All 47 identified in-band sites have been intentionally classified.
- [ ] Provider-declared 2xx failures remain `ApiCall` with evidence.
- [ ] Malformed success data is `InvalidResponseData`.
- [ ] Polling deadlines are `Timeout`; cancellation is `Aborted`.
- [ ] Explicit polling sleeps wake promptly on abort.
- [ ] Codex subscription 401 still maps to `TokenExpired`.
- [ ] No binding compatibility work or unrelated API expansion was added.
- [ ] Focused checks in §9 pass and `git diff --check` is clean.

