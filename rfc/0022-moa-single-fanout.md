# RFC-0022: MoA 单次扇出聚合

> **Status**: DRAFT (pending review)
> **Date**: 2026-08-05
> **Scope**: `aimux-core` 新增 `MoaModel`——实现 `LanguageModel` trait 的 Mixture-of-Agents 单次扇出聚合模型(reference models 并行跑 → 输出拼进 aggregator prompt → aggregator 跑 → 返回),不含 agent loop
> **Related**: [RFC-0021](0021-composite-model-routing.md) Composite Model 骨架(共用)、[RFC-0016](0016-align-with-aisdk.md) H4 边界(不做多步循环)

---

## 1. Motivation

**MoA(Mixture-of-Agents)是提质手段,与路由(省钱)正交。** Hermes MoA 实测:Opus aggregator + GPT-5.5 reference 比单独跑 Opus 高 6 分(HermesBench 0.8202 vs 0.7607)。机制是多个模型并行给分析,aggregator 聚合出最终答案——适合难题(架构、安全审查、难 bug),用模型协作弥补单模型盲区。

**为什么 aimux 做薄版(单次扇出)而非完整版**:
- Hermes MoA 之所以强,正因为它嵌在 agent loop 里每轮重跑(reference 在每次工具迭代后重新分析)。这个"嵌在 loop 里"是 agent 框架的活,aimux 明确不做(RFC-0005 + H4 多步工具循环已排除)。
- aimux 能做的原子能力:**单次 `generate_text`/`stream_text` 内,reference 并行扇出 + aggregator 聚合,不含 loop**。上层 agent 框架拿这个原子能力去 loop(把 MoaModel 当一个模型塞进它的循环)。
- 这与 RouterModel(RFC-0021)是兄弟——都是 composite model 实现 `LanguageModel` trait,只是一个"选 1 个"、一个"全调+聚合",可共用 `ChildModel = Arc<dyn LanguageModel>` 骨架。

**与路由的区别**:路由是"选一个模型"(降本);MoA 是"全调一遍+聚合"(提质,但更贵更慢)。两者独立,可组合(如用 RouterModel 选 reference 池,再 MoaModel 聚合)。

---

## 2. Design Goals

1. **单次调用内完成**:`do_generate`/`do_stream` 内扇出+聚合,不含 agent loop / 多步工具循环。
2. **对调用方透明**:MoaModel 实现 `LanguageModel`,`generate_text`/`stream_text` 入口零改动。
3. **自动跨 8 binding**:作为 `LanguageModel`,经现有 FFI/napi 自动可用(wire 层零改动,仅需新增构造函数)。
4. **复用 aimux 自身能力**:reference 并行调 `do_generate` 用 `futures::join_all`(aimux-core 已依赖 `futures`),**不引入 tokio/ML 库**。
5. **错误容忍**:默认 BestEffort(单 reference 失败丢弃+告警,其余继续),可配 FailFast。
6. **usage 正确累加**:reference + aggregator 的 token usage 全部累加,不丢计费信息。

---

## 3. Design

### 3.1 MoaModel 结构

```rust
// aimux-core/src/moa.rs (新增)

use std::sync::Arc;
use async_trait::async_trait;
use futures::future::join_all;
use crate::composite::ChildModel;  // = Arc<dyn LanguageModel>,与 RFC-0021 共用

/// Mixture-of-Agents 单次扇出聚合模型。
///
/// reference models 并行跑(非流式)→ 输出拼进 aggregator prompt → aggregator 跑 → 返回。
/// 单次 generate_text/stream_text 内完成,不含 agent loop。
pub struct MoaModel {
    references: Vec<ChildModel>,
    aggregator: ChildModel,
    config: MoaConfig,
}

/// reference 失败策略。
#[derive(Debug, Clone, Copy, Default)]
pub enum MoaFailMode {
    /// 尽力而为:某个 reference 失败则丢弃 + 发 Warning,其余继续。(默认)
    #[default]
    BestEffort,
    /// 任一 reference 失败立即整体失败。
    FailFast,
}

#[derive(Debug, Clone)]
pub struct MoaConfig {
    pub provider_name: String,            // 默认 "moa"
    pub model_id: String,                 // 默认 "moa"
    /// aggregator 独立 system 指令(拼在原 prompt 前)。None 用默认聚合指令。
    pub aggregator_instructions: Option<String>,
    /// reference 是否去掉 tools/tool_choice(Hermes:reference 不带 tool schema 保便宜)。默认 true。
    pub strip_reference_tools: bool,
    pub fail_mode: MoaFailMode,
}

// 手写 Default：strip_reference_tools 必须默认 true（derive(Default) 会给 false，与文档矛盾）。
impl Default for MoaConfig {
    fn default() -> Self {
        Self {
            provider_name: "moa".into(),
            model_id: "moa".into(),
            aggregator_instructions: None,
            strip_reference_tools: true,
            fail_mode: MoaFailMode::BestEffort,
        }
    }
}
```

**为什么用 `ChildModel = Arc<dyn LanguageModel>`**(与 RFC-0021 一致,验证结论):
- `Arc` 可 clone,与 FFI/Node 形状一致;child handle drop 后子模型仍由 Arc 保活。
- `Box<dyn LanguageModel>` 不可 Clone,不适合 composite 共享。

### 3.2 Prompt 拼接格式(方案 A)

reference 输出拼成一条新 `Role::User` 消息,append 到 aggregator 的 prompt 末尾(等价 Hermes 的 "private context injection")。

```
[原始 prompt messages ...(原样)]
+ Role::User 消息:
    {aggregation_instruction}

    # Reference model responses

    ## {model_id_1}
    {a₁}

    ## {model_id_2}
    {a₂}
    ...
```

**为什么不用 `provider_options`**(方案 B 否决):`provider_options` 是给单个 provider 的 opaque 桶(`HashMap<String, Value>`,key 是 provider 名),无法表达"多模型输出"这种结构化语义,aggregator provider 也不会去读它。

**reference 输出只取文本**:`extract_text` 从 `GenerateContent::Text` 提取,丢弃 `Reasoning`/`ToolCall`/`Source`(薄版)。若未来要保留 reference 的 reasoning,可另开配置,但 aggregator 会把它当历史 reasoning,语义有歧义,薄版不建议。

### 3.3 do_generate 流程

```rust
#[async_trait]
impl LanguageModel for MoaModel {
    fn provider(&self) -> &str { &self.config.provider_name }
    fn model_id(&self) -> &str { &self.config.model_id }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        // 1. 扇出 reference(并行,非流式)。join_all 等待全部完成(不短路)。
        let results = if self.references.is_empty() {
            Vec::new()
        } else {
            let ref_opts = self.reference_options(options);  // clone + 去掉 tools
            join_all(self.references.iter().map(|m| m.do_generate(&ref_opts))).await
        };

        // 2. 收集成功输出 + 累加 reference usage + 收集失败 warning。
        let mut ref_usage = Usage::default();
        let mut warnings = Vec::new();
        let mut texts: Vec<(String, String)> = Vec::new();  // (model_id, text)
        for (i, r) in results.into_iter().enumerate() {
            match r {
                Ok(res) => {
                    ref_usage = add_usage(ref_usage, &res.usage);
                    let mid = res.response.model_id.clone().unwrap_or_else(|| format!("ref-{i}"));
                    texts.push((mid, extract_text(&res.content)));
                }
                Err(e) => {
                    if matches!(self.config.fail_mode, MoaFailMode::FailFast) { return Err(e); }
                    warnings.push(Warning::Other { message: format!("moa reference {i} failed: {e}") });
                }
            }
        }
        // 全部失败(且配置了 reference)→ 无法聚合。
        if !self.references.is_empty() && texts.is_empty() {
            return Err(AiMuxError::Other("moa: all reference models failed".into()));
        }

        // 3. 拼 aggregator prompt + aggregator CallOptions。
        let agg_prompt = build_aggregator_prompt(&options.prompt, self.config.aggregator_instructions.as_deref(), &texts);
        let mut agg_opts = options.clone();
        agg_opts.prompt = agg_prompt;

        // 4. 调 aggregator。
        let mut agg = self.aggregator.do_generate(&agg_opts).await?;

        // 5. 合并:usage 加上 reference 部分;warnings 追加 reference 失败告警。
        agg.usage = add_usage(agg.usage, &ref_usage);
        agg.warnings.extend(warnings);
        Ok(agg)
    }
    // do_stream 见 §3.4
}
```

**并行执行要点**(已验证):
- `join_all`(非 `try_join_all`)——后者遇首个 Err 短路,而 BestEffort 需"等全部完成再分区成败"。
- 所有 reference 共享同一个 `&ref_opts`(`do_generate` 只读借用)。
- `AbortSignal` 是 `Clone` 且共享同一 `CancellationToken`——`options.clone()` clone 出共享信号,abort 一次取消所有 reference + aggregator。

### 3.4 do_stream 策略

**reference 全部非流式跑完(`do_generate`),再流式调 aggregator,aggregator 流透传给用户。**

理由:
- reference 是辅助上下文,没必要把它们的 token 流给用户(用户只关心最终聚合结果)。
- 若 reference 也流式,首 token 延迟 = max(reference 流式) + aggregator,且要先把所有 reference 流收齐才能开始 aggregator,实现复杂、收益为零。

```rust
async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
    // 1. reference 非流式扇出(同 do_generate 前半段)。
    let (texts, ref_usage, drop_warnings) = self.run_references_nonstream(options).await?;
    //    (全失败则 return Err,同 do_generate)

    // 2. aggregator prompt + options。
    let agg_prompt = build_aggregator_prompt(&options.prompt, self.config.aggregator_instructions.as_deref(), &texts);
    let mut agg_opts = options.clone();
    agg_opts.prompt = agg_prompt;

    // 3. aggregator 流式(先 await 拿到 StreamResult)。
    let agg = self.aggregator.do_stream(&agg_opts).await?;
    let mut agg_stream = agg.stream;

    // 4. 包装:发自己的 StreamStart(带 reference 失败告警),转发其余,
    //    吞 aggregator 的 StreamStart(已发过),Finish 时把 reference usage 累加。
    let stream = async_stream::stream! {
        yield Ok(StreamPart::StreamStart { warnings: drop_warnings });
        while let Some(part) = agg_stream.next().await {
            match part {
                Ok(StreamPart::StreamStart { .. }) => { /* 吞掉 */ }
                Ok(StreamPart::Finish { finish_reason, usage, provider_metadata }) => {
                    yield Ok(StreamPart::Finish {
                        finish_reason,
                        usage: add_usage(usage, &ref_usage),
                        provider_metadata,
                    });
                }
                Ok(other) => yield Ok(other),
                // 传输层 Err 是终止性的——转发后立即 break,不继续转发后续
                // part(违反协议的 aggregator 可能在 Err 后继续吐 token)。
                Err(e) => { yield Err(e); break; }
            }
        }
    };

    Ok(StreamResult { stream: Box::pin(stream), request_body: None, response_headers: None })
}
```

**do_stream 阻塞语义**:`do_stream` 内先 await reference 扇出再返回 stream,意味着 `stream_text(model, ..).await?` 在拿 stream 句柄前要等 reference 全跑完。这是 MoA 固有延迟,可接受。若想"调用立即返回、首 poll 时才扇出",可把扇出挪进 `async_stream` 体内(发 StreamStart 后),代价是 reference 全失败的错误只能以 `StreamPart::Error` 出现在流里而非 `do_stream` 返回 `Err`。**建议保持当前草案**(返回 `Err` 更清晰)。

### 3.5 与 CallOptions 的关系

| 字段 | reference | aggregator | 依据 |
|---|---|---|---|
| `prompt` | 原样 | 原 prompt + reference 上下文 | §3.2 |
| `tools` / `tool_choice` | **去掉**(`tools=None`, `tool_choice=Auto`) | 原样继承 | Hermes:reference 不带 tool schema 保便宜;aggregator 才产出最终 tool call |
| `temperature`/`max_output_tokens`/`top_p`/… | 原样继承 | 原样继承 | 薄版不做 per-reference 调参 |
| `abort_signal` | 继承(共享同一 token) | 继承 | abort 一次全取消 |
| `headers`/`provider_options`/`body_overrides`/`max_retries`/`timeout` | 原样继承 | 原样继承 | 透传底层 provider |

`strip_reference_tools` 默认 true(可关)。某些 MoA 变体想让 reference 也规划 tool call 再由 aggregator 汇总,但已超出薄版范围。

### 3.6 错误处理

| 场景 | 策略 |
|---|---|
| 单个 reference 失败 | BestEffort(默认):丢弃 + `Warning::Other`;FailFast:直接 `return Err` |
| 部分 reference 失败 | 同上,继续用成功的输出聚合 |
| 全部 reference 失败 | `AiMuxError::Other("moa: all reference models failed")` |
| aggregator 失败 | 透传其 `Err` |
| reference 配置为空(0 个) | 跳过扇出,aggregator 用原 prompt 跑(退化为单模型,不报错) |

默认 BestEffort——MoA 的价值在于"多路冗余",单点失败不应击穿。

### 3.7 usage 累加

`Usage`/`TokenUsage`([types.rs:31-62](../aimux-core/src/types.rs#L31))各含 `total/no_cache/cache_read/cache_write/text/reasoning`,逐项 `Option` 相加。`Usage.raw`(provider 专属 opaque)跨 provider 累加无意义,丢弃。若要保留各 reference 的原始 usage,可塞进 `provider_metadata` 的 `moa` key(待 Open Question 定)。

---

## 4. Integration Approach

### 4.1 FFI / binding 透明性(与 RFC-0021 同,已验证)

- **C ABI**:`aimux_generate_text(handle, ...)` 只依赖 trait,MoaModel 注册 handle 后直接可用,wire 层零改动。
- **Node**:`Model { inner: Arc<dyn LanguageModel> }`,`generateText`/`streamText` 零改动。
- **新增构造函数**(纯新增):
  - C ABI:`aimux_moa_new(reference_handles: *const u64, ref_len: usize, aggregator_handle: u64, config_json: *const c_char) -> *mut c_char`。
  - Node:`moa(references: Array<&Model>, aggregator: &Model, opts?) -> Model` napi factory。

### 4.2 落点

- **模块**:`aimux-core/src/moa.rs`。纯 `LanguageModel` trait 组合,无 HTTP/provider 专属代码,与 trait 同属 core 最自然。
- **依赖**:`aimux-core/Cargo.toml` 加 `async-stream = { workspace = true }`(workspace 已有,与 RFC-0021 共用)。**不需要 tokio**——`join_all` 来自 `futures`,async future 由调用方运行时驱动。
- **prelude**:`lib.rs` 加 `pub mod moa;` + re-export `MoaModel`/`MoaConfig`/`MoaFailMode`。

---

## 5. Relationship with Existing RFCs

| RFC | 关系 |
|-----|------|
| [RFC-0021](0021-composite-model-routing.md) | **共用 composite 骨架**(`ChildModel = Arc<dyn LanguageModel>`)。RouterModel 选 1 个,MoaModel 全调+聚合。两者可组合:RouterModel 选 reference 池 → MoaModel 聚合。 |
| [RFC-0016](0016-align-with-aisdk.md) | **边界确认**。H4 多步工具循环明确不做;MoaModel 是单次组合,不含 loop。Hermes MoA 的"嵌在 loop 里每轮重跑"是上层 agent 框架的活,aimux 只提供原子能力。 |
| [RFC-0005](0005-protocol-conversion.md) | **正交**。MoA 不碰协议转换,各 reference/aggregator 走各自 provider 协议。 |

---

## 6. Non-Goals

1. **不含 agent loop / 多步工具循环**(H4,RFC-0016 §7.5)。MoaModel 是单次 `generate_text`/`stream_text` 内完成。上层 agent 框架把 MoaModel 当一个模型塞进它的循环去实现"每轮重跑"。
2. **不做多轮聚合(多 layer)**。本方案是单层(reference→aggregator)。Hermes 原版支持多轮(layer N 输出喂 layer N+1)。薄版不做;若要,可把 MoaModel 自身作为 reference 嵌套进外层 MoaModel(天然支持,因 `MoaModel: LanguageModel`),但 usage/warning 累加会嵌套,需测。
3. **不做 fanout per_iteration**(Hermes 的"每次工具迭代重跑 advisor")。那是 agent loop 内的语义,aimux 不做 loop。
4. **不内置 reference 的 reasoning 透传**。薄版只取 reference 的文本输出。保留 reasoning 另开配置。
5. **不做 reference 自适应选择**(按 query 难度选 reference 池)。那是路由(RFC-0021)的活,与 MoA 正交,可组合。

---

## 7. Scope of Changes

| 位置 | 改动 | 工作量 |
|------|------|--------|
| `aimux-core/src/moa.rs` | 新增:`MoaModel` + `MoaConfig` + `MoaFailMode` + `build_aggregator_prompt` + `extract_text` + `add_usage` + impl `LanguageModel` | ~300 行 |
| `aimux-core/src/composite.rs` | 新增:`ChildModel` + `add_usage`(与 RFC-0021 共用) | ~50 行(RFC-0021 已计) |
| `aimux-core/src/lib.rs` | `pub mod moa;` + prelude | ~5 行 |
| `aimux-core/Cargo.toml` | `async-stream`(与 RFC-0021 共用) | 1 行 |
| `aimux-ffi/src/lib.rs` | 新增 `aimux_moa_new` C ABI | ~30 行 |
| `bindings/node/src/lib.rs` | 新增 `moa(references, aggregator, opts?)` napi factory | ~30 行 |
| `bindings/{python,go,...}` | 薄透传 | 每语言 ~25 行 |
| 测试 | 扇出正确性 + BestEffort/FailFast + usage 累加 + stream 包装 + object-safety | ~200 行 |

**合计:~350-450 行(含与 RFC-0021 共用的 composite.rs)。无 trait 改动、无破坏性变更、入口零改动。**

---

## 8. Risks

| 风险 | 等级 | 对策 |
|------|------|------|
| **延迟倍增**(reference 串行等待) | 中 | reference 并行(`join_all`),延迟 = max(reference) + aggregator,非累加;文档标注 MoA 固有延迟 |
| **成本倍增**(N 个 reference 各消耗 token) | 中 | 默认关闭,用户显式构造 MoaModel;文档建议仅难题用;`strip_reference_tools` 保 reference 便宜 |
| **`provider()`/`model_id()` 返回 "moa" 丢失下游** | 低 | `provider_metadata` 补 `aggregator_provider`/`aggregator_model` + 各 `reference_*` |
| **do_stream 阻塞至 reference 完成** | 低 | MoA 固有延迟,文档明确;可接受(难题场景不在意首 token 延迟) |
| **usage 累加口径** | 低 | reference + aggregator 逐项相加;`raw` 丢弃(跨 provider 无意义);可选塞 `provider_metadata` |
| **aggregator prompt 膨胀**(N 个 reference 输出) | 低 | 文档建议 reference 用 `max_output_tokens` 限制(Hermes `reference_max_tokens`);薄版不做内置 cap |

---

## 9. Resolved Questions (2026-08-13)

1. **`provider()`/`model_id()` 返回值**:返回固定 `"moa"`,`provider_metadata` 补 aggregator + reference 信息。
2. **reference 输出是否带 reasoning**:默认只取 `Text`，丢弃 `Reasoning`/`ToolCall`/`Source`。
3. **多轮聚合(多 layer)**:不做。可嵌套 MoaModel（天然支持），但 usage/warning 嵌套累加需测。
4. **reference 数量上限**:无硬上限，文档建议 2-4 个 reference（Hermes 默认 2 个）。
5. **`reference_max_tokens`**:薄版不做内置 cap，用户可 per-reference 设 `max_output_tokens`。
6. **usage 结构化拆分**:顶层 `usage` 是所有 reference + aggregator 的累加总和。明细拆分放 `provider_metadata.composite_usage`，**统一结构**为 `{ "participants": [{ "role": "ref-0"|"ref-1"|...|"aggregator", "model": "model-id", "usage": Usage }], "total": Usage }`，与 RFC-0021 的 RouterModel 场景共用同一 schema（role 不同但结构一致）。
7. **`config_snapshot` 聚合策略（修订）**:同 RFC-0021 Resolved Question 6。MoaModel **不伪造自己的 config_snapshot**，P1 使用默认 `ProviderRecord::minimal("moa", "moa")`。子模型级别的 nested 录制是 RFC-0023 的增强（见 RFC-0021 §9 "RFC-0023 录制层的 nested 设计"），P1 不实现。
8. **`run_references_nonstream` helper**:RFC 草案里 `do_stream` 调用了此函数但未定义签名。P1 实施时提取为 `fn run_references_nonstream(&self, options: &CallOptions) -> Result<(Vec<(String, String)>, Usage, Vec<Warning>), AiMuxError>`，`do_generate` 和 `do_stream` 共用。返回 `(reference_texts, accumulated_usage, warnings)`。

---

## 10. Implementation Order

| 阶段 | 内容 | 依赖 | 状态 |
|------|------|------|------|
| **P1** | `composite.rs` 共用骨架(与 RFC-0021 协同)+ `moa.rs`:`MoaModel` + `do_generate` + 单测 | RFC-0021 P1 | 待实施 |
| **P2** | `do_stream`(reference 非流式 + aggregator 流透传 + StreamStart/Finish 包装)+ 单测 | P1 | 待实施 |
| **P3** | FFI(`aimux_moa_new`)+ Node(`moa` factory)+ 其他 binding 透传 | P1 | 待实施 |
| **P4**(可选) | 多 layer 嵌套测试 + `reference_max_tokens` 内置 cap | P1 | 待评估 |

**建议与 RFC-0021 协同实施**:先做 composite 骨架(共用),再分别做 RouterModel(0021)和 MoaModel(0022)。两者可并行开发,共用 `composite.rs` + `ChildModel` + `add_usage`。
