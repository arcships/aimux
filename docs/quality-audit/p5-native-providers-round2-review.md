# P5：原生 provider 实现第二轮 Review（找第一轮漏掉的新问题）

> **快照声明**: 本报告是 **2026-08-07** 的时间点快照。基于当前 HEAD（v0.2.1..HEAD）代码审查。
> 代码演进后部分发现可能过期；本报告作为诊断参考归档。
>
> **范围**: `aimux-providers/src` 中第一轮（p4）未细看的**原生 provider 实现**——
> `{google, anthropic, cohere, mistral, xai, bedrock, vertex, azure, anthropic_aws, codex}` 原生协议层 +
> `{tinyfish, searxng, jina_ai, dataforseo, you_com, linkup, parallel_ai, tavily}` 等搜索/模态 provider。
> 第一轮已审的 openai 子目录、provider-utils/http.rs、aimux-ffi、bindings、CLI 不在本轮范围。
>
> **方法**: 只读源码审计（`read`/`grep`/`git diff --stat`），未运行 `cargo`（p4 已确认
> `cargo check -p aimux-providers` 通过、`list_models_test` 22/22 通过）。
> 先读 [p4-release-v021-head-review.md](p4-release-v021-head-review.md) 与
> [p1-google-unwrap-review.md](p1-google-unwrap-review.md)、
> [p1-convert-structure-review.md](p1-convert-structure-review.md)、
> [p2-provider-abstraction-audit.md](p2-provider-abstraction-audit.md) 的结论，避免重复并验证其是否仍成立。
>
> **日期**: 2026-08-07

---

## 0. 总体四维判断

| 维度 | 判断 | 关键依据 |
|---|---|---|
| 架构状况 | **中等偏好，但有身份/重试两类系统性缝隙** | 原生协议层分层清晰（每 provider 独立 Config+Provider+Model），bedrock SigV4 拆层（anthropic/stream.rs 共享 core、event_stream.rs 自实现二进制解码）合理；但 config_snapshot 覆盖矩阵不完整、`max_retries` 跨 provider 被静默忽略，属于横切一致性缺陷 |
| 代码整洁度 | **中等** | 命名/模块拆分清晰，生产路径 panic 可控；但 `list_models` 在 6 个原生 provider 中近乎逐字复制（~300 行重复），`is_custom_reasoning`/`get_model_capabilities` 等辅助函数仍多处重复（p1 结论仍成立） |
| 边界遵守情况 | **良好** | 不可信外部响应处理普遍稳健：SSE/JSON 解析用 `serde` + `map_err`/`unwrap_or_default`，无索引越界；bedrock event_stream 二进制解码全程边界校验 + CRC + `from_utf8_lossy`；搜索/模态 provider 零 unwrap/expect/panic |
| 抽象正确度 | **核心扎实，接口有漂移** | `convert_*`/stream 状态机抽象扎实；但 `RuntimeModel.owned_by` 语义被 3 个 provider 破坏（display_name 充当 owner）；`max_retries` 契约在 5 个 provider 上漂移；config_snapshot 在 8 个 LanguageModel 上退化为 minimal |

---

## 1. 新 findings 列表

> 下表标 🆕 为第一轮（p4/p1/p2）**未报告**的新发现；🔁 为对已有结论的**验证/扩展**。

### 1.1 Blocker
无（本轮未发现 blocker 级新问题）。

### 1.2 Major

#### M1. 🆕 `CallOptions.max_retries`（RFC-0017 per-call override）在 5 个原生 provider 上被静默忽略
- **模块**: google / cohere / mistral / azure / bedrock
- **问题**: 这 5 个 provider 的 `do_generate`/`do_stream`（及 `list_models`）全部硬编码
  `RetryConfig::default()` 传入 `send_timed`/`send`，既不读 `self.config.retry_config`，
  也不调 `resolve_retry_config(.., options.max_retries)`。其 Config 结构体（`GoogleConfig`/
  `CohereConfig`/`MistralConfig`/`AzureConfig`）甚至**没有** `retry_config` 字段。
  后果：调用方传 `max_retries: Some(0)` 想禁用重试，仍按默认重试策略执行——契约违反、用户可见。
- **对照**: `OpenAIModel`（[openai/model.rs:219](../../aimux-providers/src/openai/model.rs#L219)）
  用 `resolve_retry_config(&self.config.retry_config, options.max_retries)`；
  `AnthropicModel`（[anthropic/model.rs:109](../../aimux-providers/src/anthropic/model.rs#L109)）
  用 `resolve_anthropic_retry(&self.config.retry_config, options.max_retries)`——两者都正确合并。
- **理由**: p4 C4 仅把此问题框定为"**list_models** retry 策略分叉（minor）"；实际 do_generate/do_stream
  同样硬编码，且影响面是**用户可见的重试行为**，应升为 major。
- **位置**:
  - google do_generate [google/model.rs:139](../../aimux-providers/src/google/model.rs#L139)、do_stream [:218](../../aimux-providers/src/google/model.rs#L218)、list_models [google/mod.rs:160](../../aimux-providers/src/google/mod.rs#L160)
  - cohere do_generate [cohere/model.rs:142](../../aimux-providers/src/cohere/model.rs#L142)、do_stream [:268](../../aimux-providers/src/cohere/model.rs#L268)、list_models [cohere/mod.rs:122](../../aimux-providers/src/cohere/mod.rs#L122)
  - mistral do_generate [mistral/model.rs:245](../../aimux-providers/src/mistral/model.rs#L245)、do_stream [:336](../../aimux-providers/src/mistral/model.rs#L336)、list_models [mistral/mod.rs:116](../../aimux-providers/src/mistral/mod.rs#L116)
  - azure do_generate [azure/model.rs:463](../../aimux-providers/src/azure/model.rs#L463)、do_stream [:477](../../aimux-providers/src/azure/model.rs#L477)、list_models [:295](../../aimux-providers/src/azure/model.rs#L295)
  - bedrock do_generate [bedrock/model.rs:127](../../aimux-providers/src/bedrock/model.rs#L127)、do_stream [:197](../../aimux-providers/src/bedrock/model.rs#L197)、list_models [bedrock/mod.rs:262](../../aimux-providers/src/bedrock/mod.rs#L262)
- **与已有结论关系**: 🔁 扩展 p4 C4（minor → major，范围 list_models → 全路径）

#### M2. 🆕 config_snapshot 覆盖矩阵有 8 个 LanguageModel 退化为 minimal（base_url=None、source="unknown"）
- **模块**: bedrock / anthropic_aws / vertex / vertex(anthropic_model) / xai / xai(responses) / open_responses / huggingface(responses)
- **问题**: 这 8 个 `LanguageModel` 实现均**未覆盖** `config_snapshot()`，回落到 trait 默认
  `ProviderRecord::minimal(provider, model_id)`——即 `base_url: None`、`api_key_source: "unknown"`、
  `profile: None`、`provider_options: None`（见 [recording.rs:129](../../aimux-core/src/recording.rs#L129)）。
  它们都有 `from_env()`，录制后回放无法凭快照重建（缺 base_url、来源不明）。
  p4/RFC 计划（[rfc0023-recording.md:98](../plan/rfc0023-recording.md#L98)）仅声称覆盖
  "Anthropic/Google/Azure/Codex/Mistral/Cohere 原生族"——上述 8 个确属未覆盖范围，但 p4 未将其列为缺口。
- **特别指出**: 其中 `XaiModel`/`OpenResponsesModel`/`HuggingFaceResponsesModel` 是 OpenAI 兼容封装，
  其 `list_models` 已会构造 `OpenAIConfig` 并复用 `config_snapshot_from_config` 同款 helper
  （见 [xai/mod.rs:119](../../aimux-providers/src/xai/mod.rs#L119)），却唯独 `config_snapshot` 不复用——
  同一 provider 内 list_models 与 config_snapshot 抽象层级不一致。
- **位置**:（均无 `fn config_snapshot` 覆盖）
  - bedrock [bedrock/model.rs:101](../../aimux-providers/src/bedrock/model.rs#L101)
  - anthropic_aws [anthropic_aws/model.rs:117](../../aimux-providers/src/anthropic_aws/model.rs#L117)
  - vertex [vertex/model.rs:120](../../aimux-providers/src/vertex/model.rs#L120)、vertex anthropic [vertex/anthropic_model.rs:140](../../aimux-providers/src/vertex/anthropic_model.rs#L140)
  - xai [xai/model.rs:87](../../aimux-providers/src/xai/model.rs#L87)、xai responses [xai/responses/mod.rs:84](../../aimux-providers/src/xai/responses/mod.rs#L84)
  - open_responses [open_responses.rs:178](../../aimux-providers/src/open_responses.rs#L178)
  - huggingface responses [huggingface/responses.rs:83](../../aimux-providers/src/huggingface/responses.rs#L83)
- **理由**: p4 C3 只点了 4 个 provider 的 explicit 误记，未点出这 8 个 LanguageModel 完全无快照。
- **与已有结论关系**: 🆕 新发现（补 p4 覆盖矩阵缺口）

### 1.3 Minor

#### m1. 🔁 C2 在 OpenAI 族内部不一致：Chat 路径硬编码 "openai"，Responses 路径已用 config.provider
- **模块**: openai
- **问题**: `OpenAIModel.config_snapshot` 写死 `"openai"`
  （[openai/model.rs:214](../../aimux-providers/src/openai/model.rs#L214)），而
  `OpenAIResponsesModel.config_snapshot` 用 `&self.config.provider`
  （[openai/responses/mod.rs:116](../../aimux-providers/src/openai/responses/mod.rs#L116)）。
  `provider()` 同理：Chat 返回字面量 `"openai"`，Responses 返回 `&self.config.provider`。
  说明 p4 C2（兼容族丢失真实 provider 身份）的修复**只落到 Responses 路径**，Chat 路径仍丢失。
- **理由**: p4 C2 描述了现象但未指出"修复半完成"——Chat 与 Responses 同族却行为分叉。
- **与已有结论关系**: 🔁 验证 + 精细化 p4 C2

#### m2. 🔁 C5 扩展：Vertex list_models 也将 display_name 塞进 owned_by
- **模块**: vertex
- **问题**: `VertexModel.list_models` 把 `e.display_name` 写入 `RuntimeModel.owned_by`
  （[vertex/mod.rs:355](../../aimux-providers/src/vertex/mod.rs#L355)），与 google/anthropic 同病。
  对比 Cohere 正确写 `Some("cohere")`、Mistral 用响应 `owned_by` 字段、OpenAI 用响应 `owned_by`。
- **与已有结论关系**: 🔁 扩展 p4 C5（p4 仅列 anthropic + google）

#### m3. 🔁 C3 扩展：Azure 与 Codex 也无条件写 explicit（Codex 的 OAuth 模式被错记）
- **模块**: azure / codex
- **问题**: `AzureModel`/`AzureResponsesModel`（[azure/model.rs:444](../../aimux-providers/src/azure/model.rs#L444)、
  [azure/responses.rs:250](../../aimux-providers/src/azure/responses.rs#L250)）与
  `CodexModel`（[codex.rs:416](../../aimux-providers/src/codex.rs#L416)）同样硬编码 `"explicit"`。
  其中 Codex 有 `CodexMode::Subscription`（OAuth token，既非 env 也非显式 API key），快照丢失该模式信息；
  base_url 也只记 `openai.base_url`，未记 mode——回放无法区分 ApiKey/Subscription 通道。
- **与已有结论关系**: 🔁 扩展 p4 C3（p4 仅列 google/anthropic/mistral/cohere）

#### m4. 🔁 C4 扩展：Bedrock list_models 用 `send`（非 `send_timed`），与同类不一致
- **模块**: bedrock
- **问题**: 其余原生 provider 的 `list_models` 用 `send_timed`（可传 timeout），
  Bedrock 用 `send`（[bedrock/mod.rs:252](../../aimux-providers/src/bedrock/mod.rs#L252)）。
  `send_timed` 内 `timeout.unwrap_or_default()`（[http.rs:567](../../aimux-provider-utils/src/http.rs#L567)）
  保证 None 也有默认超时，故不会无限挂起；但 API 形状分叉增加维护负担。
- **与已有结论关系**: 🔁 扩展 p4 C4

#### m5. 🆕 list_models 在 6 个原生 provider 中近乎逐字复制（~300 行重复）
- **模块**: google / cohere / mistral / anthropic / vertex / bedrock
- **问题**: 这 6 处 `list_models` 结构完全相同：clone config → 拼 `{base}/models` URL →
  建 auth+content-type headers → `send_timed`/`send` + `RetryConfig::default()` →
  本地定义 `Resp{data/models/summaries}` + `Entry` → `from_slice` + map 到 `RuntimeModel`。
  仅有字段名（`models`/`data`/`modelSummaries`、`name`/`id`/`modelId`）、owned_by 取值、
  是否 strip `models/` 前缀等差异。OpenAI 已有可复用的 `execute_list_models`
  （[openai/model.rs:874](../../aimux-providers/src/openai/model.rs#L874)），xAI/Codex 已复用，
  但上述 6 个 provider 各自重写。
- **与已有结论关系**: 🆕 新发现（p1-convert-structure-review 只覆盖了 convert.rs 重复，未含 list_models）

### 1.4 Nit

#### n1. 🔁 google/utils.rs 的 unwrap/expect 仍在，GoogleJsonAccumulator 仍未接入生产流式
- **模块**: google/utils
- **问题**: p1 报告的 8 处 `.unwrap()` + `set_nested_value` 7 处 `.expect()` 仍存在
  （[google/utils.rs:102/313/325/333/358](../../aimux-providers/src/google/utils.rs#L102) 等，行号因重排略移）。
  全仓 grep `GoogleJsonAccumulator`/`process_partial_args` 仅在 utils.rs 自身 + 测试命中，**仍无生产调用点**。
- **与已有结论关系**: 🔁 验证 p1 结论"潜伏风险、未接入前加固"仍成立、未修复。

#### n2. 🔁 p1 的 anthropic panicking 包装函数实际为 test-only，生产 panic 风险低于 p1 框定
- **模块**: anthropic/convert
- **问题**: `convert_prompt_to_anthropic_full`（[:243](../../aimux-providers/src/anthropic/convert.rs#L243) `.expect`）
  与 `convert_prompt_to_anthropic`（[:259](../../aimux-providers/src/anthropic/convert.rs#L259) `panic!`）
  全仓 grep 仅在 `tests/anthropic_convert_*test.rs` 被调用；生产路径走
  `convert_prompt_to_anthropic_full_fallible`（返回 `Result`）。
- **与已有结论关系**: 🔁 精细化 p1-convert-structure-review §4.1（生产 panic 风险实为 0，应降级为 nit/死代码）。

#### n3. 🆕 anthropic usage 转换对不可信 token 计数做 `as u32` 截断
- **模块**: anthropic/usage
- **问题**: `usage_from_anthropic` 把 u64 token 计数 `as u32`
  （[anthropic/usage.rs:150-160](../../aimux-providers/src/anthropic/usage.rs#L150)）。
  解析本身稳健（`unwrap_or_default` + 全字段 `#[serde(default)]`），但对畸形/超大值静默截断。
  与 core 的 u32 token 模型一致，优先级低；同 p4 A11（replay token 截断）同源。
- **与已有结论关系**: 🆕（p4 A11 在 replay 侧，本处在 provider usage 转换侧）

#### n4. 🆕 bedrock event_stream 遇首个坏帧即 `break`，静默丢弃其后所有有效帧
- **模块**: bedrock/event_stream
- **问题**: `decode_messages` 在 prelude CRC / message CRC 校验失败时 `break`
  （[event_stream.rs:161](../../aimux-providers/src/bedrock/event_stream.rs#L161)、[:244](../../aimux-providers/src/bedrock/event_stream.rs#L244)），
  测试 `valid_frame_before_corrupt_one_is_kept` 也确认"坏帧后停止"。属刻意安全选择（不信任坏帧后的数据），
  但对中途单帧损坏的流会静默截断剩余内容。可考虑 `continue` + 计数告警。

---

## 2. 与已有审计文档结论的对照

| 已有结论 | 本轮验证结果 |
|---|---|
| p4 C2（OpenAI 兼容族丢失 provider 身份） | 🔁 仍成立；进一步发现修复半完成——Responses 路径已修、Chat 路径未修（见 m1） |
| p4 C3（env key 误记 explicit：google/anthropic/mistral/cohere） | 🔁 仍成立（4 处仍是字面量 `"explicit"`）；扩展：azure×2 与 codex 也硬编码 explicit，codex 还丢 OAuth 模式（见 m3） |
| p4 C4（list_models retry 分叉，minor） | 🔁 升级为 M1（major）：影响扩到 do_generate/do_stream，且是 `max_retries` 契约违反而非仅 list_models 风格分叉；bedrock 用 send 非 send_timed（m4） |
| p4 C5（owned_by=display_name：anthropic/google） | 🔁 仍成立；扩展 vertex 同病（m2） |
| p4 C6（anthropic config_snapshot 序列化裸 header map，漏 api_version/retry_config/body_overrides） | 🔁 仍成立（[anthropic/model.rs:100](../../aimux-providers/src/anthropic/model.rs#L100) `serde_json::to_value(&self.config.headers).ok()`，未含 api_version 等） |
| p1 google/utils.rs unwrap（8+7 处） | 🔁 仍成立、未修；GoogleJsonAccumulator 仍无生产调用点（n1） |
| p1-convert-structure-review §4.1（anthropic panicking 包装） | 🔁 精细化：生产无调用方，仅 test 用，panic 风险实为 0（n2） |
| p1-convert-structure-review 重复（is_custom_reasoning/get_model_capabilities 等） | 🔁 结构性重复仍在（本轮未逐一重核，diff 显示 convert.rs 仍在改动）；新增 list_models 重复（m5） |
| p2 Provider trait 强制 language_model（架构异味） | 未在本轮重核，结论应仍成立（trait 未变） |

---

## 3. 不可信外部响应处理专项（维度 b 结论）

**总体良好**——这是本轮最正面的发现：

- **流式 SSE 解析**：google（[google/model.rs:267-290](../../aimux-providers/src/google/model.rs#L267) 先 `from_str`→`Value`→`from_value`，错则 `StreamPart::Error`+break）、mistral（[mistral/model.rs:381-409](../../aimux-providers/src/mistral/model.rs#L381) 同款 + `[DONE]` + in-stream error 对象探测）、anthropic（[anthropic/stream.rs:277](../../aimux-providers/src/anthropic/stream.rs#L277) `from_str`→`StreamEvent`，`Ok(_)|Err(_)=>{}` 跳过未知/坏事件而非 panic）均无裸 unwrap。
- **非流式 JSON 解析**：普遍 `serde_json::from_slice(..).map_err(..)` + `#[serde(default)]` + `.into_iter().next().ok_or_else(..)`（如 [google/model.rs:151-155](../../aimux-providers/src/google/model.rs#L151)）。
- **usage 转换**：anthropic `convert_anthropic_usage` 用 `unwrap_or_default` + 全字段 `#[serde(default)]`（[anthropic/usage.rs:80](../../aimux-providers/src/anthropic/usage.rs#L80)），null/缺失字段归零而非 panic。
- **bedrock 二进制流**：[event_stream.rs](../../aimux-providers/src/bedrock/event_stream.rs) 全程边界校验（`offset+12<=len`、`total_length<16`、`header_end>data.len()`→break、`h_offset+name_len>header_end`→break）、CRC 双校验、`String::from_utf8_lossy`，无索引越界。
- **搜索/模态 provider**：`tavily/jina_ai/searxng/dataforseo/you_com/linkup/parallel_ai/tinyfish/google_pse/exa_ai/firecrawl/serper` 及 TTS/STT/image/video provider 全部 **0 处** unwrap/expect/panic。
- **唯一 guard 依赖型 unwrap**：`xai/convert.rs:481` `options.reasoning.unwrap()`（被 `is_some_and(ReasoningEffort::is_custom)` 守卫）、`bedrock/convert.rs:54` `blocks.last_mut().unwrap()`（被 `blocks.last()==Some(b)` 守卫）、`bedrock/image.rs:148` `options.files.as_ref().unwrap()`（被 `if has_files` 守卫）——当前安全但脆弱，p1 已点名同类模式。

---

## 4. 并发/超时专项（维度 e 结论）

- **超时**：`do_generate`/`do_stream` 均传 `options.timeout.map(Into::into)`；`list_models` 传 `None` 但
  `send_timed` 内 `unwrap_or_default()`（[http.rs:567](../../aimux-provider-utils/src/http.rs#L567)）兜底默认超时，**无无限挂起风险**。
- **共享状态**：全 `aimux-providers/src` 无 `Mutex`/`RwLock`/`RefCell`；provider 不持 HTTP client
  （复用 `http::shared_client()`，reqwest Client 设计可共享）；xAI 的 `SOURCE_ID_COUNTER` 为 `AtomicU64`，线程安全。无跨 await 持锁死锁面。

---

## 5. Top 5 优先修复建议

1. **M1 — 统一 `max_retries` 契约**：为 google/cohere/mistral/azure/bedrock 的 do_generate/do_stream/list_models
   引入与 OpenAI 同款的 `resolve_retry_config(config_retry, options.max_retries)`（或在各自 Config 补 `retry_config` 字段）。
   这是唯一 user-visible 的契约违反，优先级最高。
2. **M2 — 补齐 config_snapshot 覆盖**：至少为 3 个 OpenAI 兼容封装（xai/open_responses/huggingface_responses）
   复用 `config_snapshot_from_config`（它们 list_models 已能构造 OpenAIConfig，零新增抽象成本）；
   bedrock/vertex/anthropic_aws 至少补 `base_url` + `api_key_source`（env/explicit）以便回放重建。
3. **m1/m3 — 修齐 OpenAI Chat 身份 + 原生 explicit 误记**：`OpenAIModel.config_snapshot` 改用 `&self.config.provider`
   （对齐 Responses 路径）；google/anthropic/mistral/cohere/azure/codex 引入 `api_key_source` 追踪（复用 OpenAIConfig 模式）。
4. **m2 — owned_by 语义**：google/anthropic/vertex 的 `list_models` 改返回真实 owner（如 "Google"/"Anthropic"），
   display_name 应放进 `RuntimeModel` 的扩展字段而非 `owned_by`。
5. **m5 — 抽取 list_models 公共 helper**：把 6 处重复实现收敛为一个参数化 helper
   （接收 base_url/headers/retry_config + 字段映射闭包），消除 ~300 行复制分叉。

---

## 6. 剩余不确定性

- 未运行 `cargo check`/`cargo test`（遵循 p4 已验证基线 + 避免与并发 agent 冲突）；以上为纯静态阅读结论。
- `max_retries` 忽略是否被某上层（FFI/CLI）文档化为"原生 provider 不支持 retry 配置"——未在 RFC 中找到此类声明，
  且 OpenAI/Anthropic 的 `resolve_*` helper 表明意图是支持；若有文档豁免则 M1 可降级。
- `CodexMode::Subscription` 的 OAuth 回放重建策略（是否走 D1 "explicit 补 key" 还是 "传实例"）取决于 RFC-0023 定稿，
  本轮未深入 RFC-0018 OAuth 侧；m3 的"丢失 mode"为基于源码事实的推断。
- bedrock/vertex 等无 config_snapshot 是否"有意不在本期覆盖"——RFC 计划 [rfc0023-recording.md:98](../plan/rfc0023-recording.md#L98)
  仅列 6 个原生族，可能是有意范围，但 p4 未显式声明这些为已知缺口，故 M2 按缺口报告。
