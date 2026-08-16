# Round 4 Phase 0.2：上轮审计发现核对

> **核对基线**：master @ `cf2cea5`（/tmp/aimux-audit-master worktree），含 RFC-0028 transcription streaming 合入（FFI 层新增 `aimux-ffi/src/transcription_session.rs`）。
> **对照来源**：`docs/quality-audit/SUMMARY.md`（2026-08-06 快照）及 p1-ffi-soundness / p1-convert-structure / p1-google-unwrap 三份子报告。
> **核对方式**：只读代码验证（Read/Grep），未运行任何 cargo 命令。
> **核对日期**：2026-08-14。

---

## 汇总统计

**11 fixed / 1 unfixed / 1 partial / 0 cannot-verify**（共核对 13 条：H×3 + M×10；M8 已在上轮报告中被自行排除，未列入核对）。

| 编号 | 发现 | 状态 |
|---|---|---|
| H1 | FFI 回调 panic 跨 `extern "C"` 边界 UB | ✅ fixed |
| H2 | openai convert 静默吞转换错误 | ✅ fixed |
| H3 | google `set_nested_value` 7 处 `expect()` | ✅ fixed |
| M1 | `into_cstring_raw` null 返回破坏契约 | ✅ fixed |
| M2 | 文档把重入 panic 误述为 deadlock | ✅ fixed |
| M3 | `Timeout` 不在 `is_retryable()` | ⬜ unfixed（有意决策，已文档化） |
| M4 | `status_code()` 靠解析 "HTTP " 前缀 | ✅ fixed |
| M5 | `from_slice` 错误误标 `AiMuxError::Http` | ✅ fixed |
| M6 | 429 响应 body 被丢弃 | ✅ fixed |
| M7 | FFI 重入无运行时防护 | ✅ fixed |
| M9 | `Provider` trait 强制 `language_model()` | ✅ fixed |
| M10 | convert.rs 大量逐字重复 | 🟡 partial |
| M11 | 3 个 400+ 行超大函数 | ✅ fixed |

多个修复点在代码注释中显式回引了上轮编号（"issue H2"、"issue M1"、"issue M7"、"issue M9"、M10），说明上轮报告被正式采纳为工作项。

---

## H1：FFI 回调 panic 跨 `extern "C"` 边界导致 UB

- **原文摘要**：流式函数的 on_part/on_done/on_error 回调 panic 时 unwind 跨 FFI 边界，是 UB；涉及 lib.rs ~30+ 调用点（原 lib.rs:1157-1158 等）。
- **状态**：✅ **fixed**
- **证据**：
  - `aimux-ffi/src/lib.rs:475-489`：新增 `invoke_stream_callback`，用 `std::panic::catch_unwind(AssertUnwindSafe(f))` 包裹回调，panic 被转换为 `AiMuxError::Other("stream callback '{name}' panicked: {msg}")` 返回。
  - 全部 4 个回调调用点均走该包装：lib.rs:1632-1634（as_openai on_part）、1640-1642（on_done）、1730-1732（stream_text on_part）、1738-1740（on_done），均 `invoke_stream_callback("on_part", || { on_part(...); })?`。
  - grep `on_error(` / `fire_error` 在 lib.rs 中已无调用——on_error 回调整体被移除，错误改为 `CAimuxError` out-struct 返回（lib.rs:333-341 + `fail_ai`）。
  - 测试：lib.rs:3210-3310 共 5 个用例（`stream_callback_catches_str_panic` / `_string_panic` / `_non_string_panic` / `_on_done_panic` / `ok_passthrough`），注释标注 "issue #64: FFI panic guard"。
  - 兜底：根 `Cargo.toml:30` `[profile.release] panic = "abort"`。
  - 新增的 `transcription_session.rs`（RFC-0028）采用 pull 模型（`next_part`），不含任何 extern "C" 回调，未引入新的未防护回调点。
- **备注**：注释明确说明 dev/debug 构建仍会 unwind，因此 catch_unwind 是必要防护而非仅靠 panic=abort。

## H2：`build_request_body_with_warnings` 静默吞掉所有转换错误

- **原文摘要**：openai/convert.rs:1475 的 `unwrap_or_else(|_| RequestBodyResult { body: Null, warnings: vec![] })` 吞掉所有错误，请求以空 body 发出。
- **状态**：✅ **fixed**
- **证据**：
  - `aimux-providers/src/openai/convert.rs:1362-1368`：签名改为 `pub fn build_request_body_with_warnings(...) -> Result<RequestBodyResult, AiMuxError>`；doc 注释（1359-1361）："Conversion errors propagate to the caller (fail-fast, issue H2): the old behaviour of silently returning `body: null` sent empty requests upstream"。
  - convert.rs:1388-1392：`convert_prompt_to_openai_messages_with_provider(...)?` 用 `?` 传播。
  - 包装层 `build_request_body`（892-905）同样返回 `Result<Value, AiMuxError>` 并透传。
- **备注**：无残留 `unwrap_or_else(|_| ... body: Null ...)` 模式。

## H3：`set_nested_value` 的 7 处 `expect()` 可被恶意流触发 panic

- **原文摘要**：google/utils.rs:452-495 的 7 处 `expect()`（"parent must be object/array" 等）在路径与已累积树类型冲突时直接 panic，且输入是不可信的 Google 流式 partialArgs。
- **状态**：✅ **fixed**
- **证据**：
  - `aimux-providers/src/google/utils.rs:454`：签名改为 `fn set_nested_value(obj: &mut Value, segments: &[Segment], value: Value) -> Result<(), AiMuxError>`；函数体内全部 expect 已替换为 `ok_or_else(|| parent_must_be(seg))?` 等 `AiMuxError::JsonParse` 错误（如 496、505、524、531、545、549 行）。
  - `process_partial_args`（192-195）同样返回 `Result<ProcessResult, AiMuxError>`，调用点 222/231 用 `?` 传播；doc（188-191）明确 "such conflicts must not panic"。
  - 额外加固（超出原建议）：`MAX_PARTIAL_ARG_INDEX = 100_000`（440）、`MAX_PARTIAL_ARG_DEPTH = 64`（444）资源上限，且索引预校验保证错误原子性（463-476，注释引 "audit round 3, A1"）；209-211 还防御了空路径切片 panic（注释标注 "audit finding H3 residual panic path"）。
- **备注**：修复质量高于原建议（Result 化 + DoS 上限 + 残留 panic 路径清理）。

## M1：`into_cstring_raw` null 返回破坏 API 契约

- **原文摘要**：`CString::new` 失败时返回 `null_mut()`，调用者无法区分成功/失败；`fire_error` 失败时静默跳过回调。
- **状态**：✅ **fixed**
- **证据**：`aimux-ffi/src/lib.rs:292-305`：`into_cstring_raw` 现将内部 NUL 替换为 U+FFFD（`s.replace('\0', "\u{FFFD}")`）并经 `tracing::warn!` 上报，doc 声明 "Never returns null (issue M1)"；测试 lib.rs:3204-3207 验证 `"a\0b"` → `"a\u{FFFD}b"`。on_error/fire_error 路径已被 CAimuxError 结构化错误通道取代。
- **备注**：采用的正是子报告建议的方案 1（U+FFFD 替换）+ 可观测性（tracing）。

## M2：文档把 FFI 重入 panic 误述为 deadlock

- **原文摘要**：lib.rs:22-24 原文 "doing so would deadlock the runtime"，实际是 tokio block_on 嵌套 panic，跨 FFI 即 UB。
- **状态**：✅ **fixed**
- **证据**：`aimux-ffi/src/lib.rs:22-30` 模块文档改为："a nested `block_on` on the same thread makes tokio **panic** ('Cannot start a runtime from within a runtime') … under this workspace's release profile, `panic = 'abort'` … the thread-local guard in [`ffi_block_on`] does that (issue M7)"。
- **备注**：与 M7 的防护实现联动修正。

## M3：`Timeout` 不在 `is_retryable()` 中

- **原文摘要**：超时不会触发重试（aimux-core/src/error.rs:81-86）。
- **状态**：⬜ **unfixed（有意决策，已文档化）**
- **证据**：`aimux-core/src/error.rs:175-180`：`is_retryable` 仍只匹配 `AiMuxError::ApiCall(d) => d.is_retryable`，Timeout 依旧返回 false。但 171-174 行新增设计说明："`Timeout` is a spent caller-side time budget — the AI SDK treats it as part of the abort family (`isAbortError` matches `'TimeoutError'`) and does not retry it; neither do we."
- **备注**：行为未变，但已从"疑似遗漏"升级为"对齐 AI SDK 的有意决策"。若 Round 4 仍想改，需先推翻该对齐论据；建议本轮按"确认即可"处理，不再列为缺陷。

## M4：`status_code()` 靠解析 "HTTP " 字符串前缀还原状态码

- **原文摘要**：Auth/RateLimited/ModelNotFound 等解析不出状态码，FFI 信封 status_code 多为 null。
- **状态**：✅ **fixed**（随错误枚举重构整体解决）
- **证据**：`aimux-core/src/error.rs:209-215`：`status_code()` 现直接读结构化字段 `ApiCallError::status_code`（`AiMuxError::TokenExpired(_) => Some(401)` 为变体契约）；测试 `status_code_reads_the_field_not_the_message`（267 行起）钉死"never from parsing the message"。全仓 grep `"HTTP "` 前缀解析无残留；HTTP 层在 `parse_provider_error`/`send_with_retry_raw` 构造时填入观察到的状态码。
- **备注**：错误枚举已重构（Auth/RateLimited/ModelNotFound 变体消失，统一为带字段的 `ApiCall{ApiCallError}`），M4 的根因（字符串解析）不复存在。

## M5：10 处 `serde_json::from_slice` 错误误标 `AiMuxError::Http`

- **原文摘要**：bedrock/google/vertex 的 JSON 反序列化错误应归类 `Json` 而非 `Http`。
- **状态**：✅ **fixed**
- **证据**：`AiMuxError::Http` 变体已不存在（aimux-providers / aimux-core / aimux-provider-utils 全仓 grep 0 命中）。统一入口为 `impl From<serde_json::Error> for AiMuxError`（`aimux-core/src/error.rs:156-164`）：`Category::Data => InvalidResponseData`，`Syntax | Eof | Io => JsonParse`。原 10 处调用点已全部改为 `?` 或 `.map_err(AiMuxError::from)`，如 `bedrock/model.rs:171`、`bedrock/embedding.rs:217`、`google/embedding.rs:143`、`vertex/embedding.rs:162`。
- **备注**：比原建议（改标 Json）更进一步——按 serde Category 做了语法/语义二级分类。

## M6：429 响应 body 被丢弃

- **原文摘要**：`RateLimited` 无 provider message（aimux-provider-utils/src/http.rs:750）。
- **状态**：✅ **fixed**
- **证据**：`aimux-provider-utils/src/http.rs:1325-1334`：非 2xx 路径先 `read_error_body`，再 `parse_provider_error(status_code, &body, ...)`，注释明确 "extracted message/provider_code/raw body survive"。`aimux-provider-utils/src/response.rs:26-75`：429 与其他状态同路径处理——message 从 body 的配置路径提取（提不出则逐字用 body，44-51），且 `response_body` 在 body 非空时逐字保留（74 行 `let response_body = (!body.is_empty()).then(|| body.to_string());`），存入 `ApiCallError.response_body`（85-104）。retry-after 头另存 `retry_after_ms`。
- **备注**：原 L2（parse_provider_error 429 死分支）随重构一并消失——现在所有状态码走同一分支。

## M7：FFI 重入死锁/panic 风险，无运行时防护

- **原文摘要**：回调中再调 FFI 会触发 block_on 嵌套 panic（跨 FFI 即 UB），无防护。
- **状态**：✅ **fixed**
- **证据**：`aimux-ffi/src/lib.rs:195-236`：新增 `thread_local! { static IN_FFI_BLOCK_ON: Cell<bool> }` + `ffi_block_on`——重入时返回 `Err(AiMuxError::Other("re-entrant FFI call from within a callback is not allowed"))`，并用 `Reset` guard（Drop 置回 false）保证 panic 时也释放。全部 FFI 入口（7 处 `ffi_block_on(` 调用，含 `run_and_serialize`:527、两个 stream 函数:1597/1695）走该包装；全 crate 唯一直接 `runtime().block_on` 在 `ffi_block_on` 内部（234 行）。新 `transcription_session.rs:268` 的 `terminate` 也经 `ffi_block_on` 做有界 join 并处理重入 detach。
- **备注**：guard 本身无专门单元测试（回调 panic 有 5 个测试，重入路径未见对应用例），建议补一条"回调内再调 FFI 返回错误而非崩溃"的测试。

## M9：`Provider` trait 强制 `language_model()`

- **原文摘要**：30+ 非语言模型 provider 被迫返回 `Unsupported`。
- **状态**：✅ **fixed**
- **证据**：`aimux-core/src/provider.rs:23-28`：`language_model` 改为带默认实现的 provided method（默认返回 `UnsupportedFunctionality`），doc 注明 "Non-language-model providers … get the default `Unsupported` error. Only providers that actually expose a language model override it (issue M9)"；`list_models`（44-53）同为默认实现模式。
- **备注**：非 LM provider 的 impl 块不再需要手写样板。

## M10：convert.rs 大量逐字重复

- **原文摘要**：`is_custom_reasoning` 5 处；`get_gpt_version`/`get_o_series_version`/`get_model_capabilities` ~140 行在 openai chat/responses 间 100% 重复。
- **状态**：🟡 **partial**
- **证据（已修复部分）**：
  - 新增 `aimux-providers/src/openai/convert_common.rs`：单一来源定义 `get_gpt_version`(17)、`get_o_series_version`(71)、`ModelCapabilities`(84)、`SystemMessageMode`(94)、`get_model_capabilities`(100)。
  - `openai/responses/convert.rs:31`：`use crate::openai::convert_common::{ModelCapabilities, SystemMessageMode, get_model_capabilities};`；`openai/convert.rs:16-17` 同样从 `super::convert_common` 导入，注释标注 "(M10)"。
  - `is_custom_reasoning` 在 aimux-providers 全目录 grep `fn is_custom_reasoning` 为 0 命中——该函数已被 `convert_common` 的 `resolve_reasoning_effort` 等机制取代，5 处重复清零。
- **证据（残留部分）**：`top_level_media_type`/`get_top_level_media_type` 仍有 5 处定义（`openai/convert.rs:288`、`anthropic/convert.rs:281`、`xai/convert.rs:222`、`open_responses.rs:1318`、`huggingface/responses.rs:811`）；`resolve_full_media_type` 3-4 处（openai:295 / xai:226 / anthropic:717 / huggingface:834）；`resolve_provider_reference` 2 处（openai:320 / xai:249）。
- **备注**：SUMMARY 点名的两项（is_custom_reasoning ×5、~140 行模型能力检测）已完全解决；子报告 P1 清单中的 media-type 系列小工具仍未收敛到共享层。

## M11：3 个 400+ 行超大函数

- **原文摘要**：openai/convert.rs（~430 行）、anthropic/convert.rs（~430 行）、openai/responses/convert.rs（~375 行）的 build_request_body 系列。
- **状态**：✅ **fixed**
- **证据**（按函数起止行测量）：
  - `openai/convert.rs:1362-1469`：`build_request_body_with_warnings` 现 ~107 行，原 ~430 行职责已拆出 `resolve_reasoning_effort`、`resolve_is_reasoning_model`、`resolve_system_message_mode`、`apply_max_tokens`、`strip_sampling_params`、`insert_sampling_params`、`apply_tools` 等辅助函数。
  - `anthropic/convert.rs:1593-1698`：现 ~105 行。
  - `openai/responses/convert.rs:832-949`：`build_responses_request_body` 现 ~117 行（前置 `apply_responses_reasoning_block`:793 等）。
- **备注**：三个点名函数全部降到 ~110 行量级。非点名的 `convert_message_to_openai`（446-693，~247 行）与 `convert_to_xai_responses_input`（284-523，~239 行）体量基本未变，可作后续观察项。

## M8（已排除项，未核对）

上轮 SUMMARY 已自行核实 anyhow 无成员使用、Cargo.lock 无条目，RUSTSEC-2026-0190 不构成风险，仅留下 L1（根 Cargo.toml 死声明清理）作为低风险后续项。本轮不重复核对。

---

## 观察：RFC-0028 合入对 FFI 结论的影响

新增 `aimux-ffi/src/transcription_session.rs`（276 行）采用 session handle + push/next_part 拉模型，绕开了 push-callback 模式；其 `terminate` 经 `ffi_block_on`（268 行）与重入防护兼容，有界 join（JOIN_TIMEOUT=5s）+ detach 兜底。未引入新的回调 panic 或重入缺口，H1/M7 结论不受影响。
