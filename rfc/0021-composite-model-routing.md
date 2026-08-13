# RFC-0021: Composite Model 与 Model 路由

> **Status**: DRAFT (pending review)
> **Date**: 2026-08-05
> **Scope**: `aimux-core` 新增 composite model 基础设施(实现 `LanguageModel` trait 的组合模型)+ `RouterModel`(规则路由 / fallback / 可插拔策略)+ 内置策略(RuleRouter / LLM 分类器 / 视觉分流)
> **Related**: [RFC-0022](0022-moa-single-fanout.md) MoA(共用 composite 骨架)、[RFC-0016](0016-align-with-aisdk.md) AISDK 对齐、[RFC-0005](0005-protocol-conversion.md) 定位边界

---

## 1. Motivation

aimux 当前**无任何模型级路由 / fallback / 负载均衡基础设施**(全仓 grep `route|router|fallback|load_balance|shard` 零命中,retry 仅在 HTTP 层单 provider 内)。三个真实需求:

1. **省钱**:多个模型配在一起,按价格/延迟选;简单的走小模型,难的走大模型。RouteLLM 实测可砍 85% 成本保 95% 质量。
2. **容灾**:一个 provider 挂了自动切下一个,提升可用性。这是 HTTP retry 的自然延伸(跨模型而非跨重试)。
3. **降本分流**:纯文本别走多模态模型(更贵),含图才走多模态。

这些需求的共同点:**在单次 `generate_text`/`stream_text` 调用内,从多个模型中选一个(或组合)执行,不引入 agent loop / 多步工具循环**。这正是访问层的合理延伸边界——aimux 已有 HTTP 层 jitter backoff retry(RFC-0009),跨模型 fallback 是其同类。

**边界判定**:只要不引入多步循环/状态机/RAG/agent loop,且决策是"单次选模型",就还在访问层边缘内。学习型路由(ML 推理)的"重"不在决策本身,而在其运行时依赖——故不进核心,走可插拔 trait 注入。

---

## 2. Design Goals

1. **对调用方透明**:RouterModel 实现 `LanguageModel` trait,`generate_text`/`stream_text` 入口零改动(只依赖 `&dyn LanguageModel`)。
2. **自动跨 8 binding**:RouterModel 作为 `LanguageModel`,经现有 FFI/napi 自动可用,wire 层零改动(仅需新增构造函数)。
3. **可插拔策略**:路由决策走 `Router` trait,内置规则/LLM 分类器/视觉策略,用户可注入自定义(含学习型——自带 ort/candle)。
4. **零新硬依赖**:规则路由 + LLM 分类器路由不需要任何 ML 库(LLM 分类器复用自身 `LanguageModel` 调小模型);学习型由用户自带。
5. **不演化成内置 agent loop**:流式 fallback 不做(已发 StreamStart 后无法回滚);只做单次组合。

---

## 3. Design

### 3.1 Composite Model 基础设施(与 RFC-0022 共用)

已验证(独立 crate 编译通过):`Vec<Arc<dyn LanguageModel>>` 持有 + 自身实现 `LanguageModel` 完全可行。

```rust
// aimux-core/src/composite.rs (新增,与 moa.rs 共用基础)

use std::sync::Arc;
use async_trait::async_trait;
use crate::language_model::LanguageModel;

/// composite model 共用的子模型持有形状。
/// 用 Arc(非 Box):与 FFI/Node 的 Arc<dyn LanguageModel> 形状一致,
/// 可 clone、可跨 composite 共享、child handle drop 后仍由 Arc 保活。
pub type ChildModel = Arc<dyn LanguageModel>;
```

**为什么 Arc 而非 Box**(验证结论):
- `Box<dyn LanguageModel>` 不可 `Clone`(trait object 无 DynClone);`Arc` 可 clone。
- FFI registry([aimux-ffi/src/lib.rs:79](../aimux-ffi/src/lib.rs#L79))、Node `Model`([bindings/node/src/lib.rs:31](../bindings/node/src/lib.rs#L31))都用 `Arc<dyn LanguageModel>`——composite 用同形状,子模型可同时被 registry 与 composite 持有。
- child handle drop 后,子模型仍由 composite 的 Arc 保活,引用计数自动管理。

### 3.2 Router trait(可插拔策略)

```rust
// aimux-core/src/router.rs (新增)

use crate::error::AiMuxError;
use crate::language_model_message::LanguageModelPrompt;
use crate::composite::ChildModel;

/// 路由策略 trait。决策"选哪个子模型",不含执行。
///
/// 内置实现:RuleRouter / ComplexityRouter / LlmClassifierRouter / ModalityRouter。
/// 用户可实现此 trait 注入学习型分类器(如调外部 RouteLLM 服务、用 ort 加载 ONNX)。
pub trait Router: Send + Sync {
    /// 根据提示选择子模型索引。
    /// 返回 Err 表示无法路由(如全部子模型不满足能力要求)。
    fn route(&self, prompt: &LanguageModelPrompt, models: &[ChildModel]) -> Result<usize, AiMuxError>;
}
```

**设计要点**:route 是纯决策(看 prompt + models 元信息),不含执行——执行由 RouterModel 做。这让策略可独立测试、可组合(如"先按能力过滤,再按成本排序")。

### 3.3 RouterModel

```rust
/// 路由模型:实现 LanguageModel,内部按策略选一个子模型执行 + 可选 fallback。
pub struct RouterModel {
    models: Vec<ChildModel>,
    router: Box<dyn Router>,
    fallback: FallbackPolicy,
    config: RouterConfig,
}

#[derive(Debug, Clone, Copy, Default)]
pub enum FallbackPolicy {
    /// 执行失败时按序尝试其余模型(默认)。
    #[default]
    OnError,
    /// 不 fallback,选中的模型失败即失败。
    None,
}

#[derive(Debug, Clone, Default)]
pub struct RouterConfig {
    pub provider_name: String,   // 默认 "router"
    pub model_id: String,        // 默认 "router"
}

#[async_trait]
impl LanguageModel for RouterModel {
    fn provider(&self) -> &str { &self.config.provider_name }
    fn model_id(&self) -> &str { &self.config.model_id }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let idx = self.router.route(&options.prompt, &self.models)?;
        match self.models[idx].do_generate(options).await {
            Ok(r) => Ok(r),
            Err(e) if self.fallback == FallbackPolicy::OnError => {
                // 按序尝试其余模型
                self.fallback_generate(idx, options).await
            }
            Err(e) => Err(e),
        }
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let idx = self.router.route(&options.prompt, &self.models)?;
        // 流式:路由在前,委托在后。流中不 fallback(已发 StreamStart 无法回滚)。
        self.models[idx].do_stream(options).await
    }
}

impl RouterModel {
    /// fallback:按序尝试 idx 之外的所有模型。
    async fn fallback_generate(&self, exclude: usize, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let mut last_err = None;
        for (i, m) in self.models.iter().enumerate() {
            if i == exclude { continue; }
            match m.do_generate(options).await {
                Ok(r) => return Ok(r),
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.unwrap_or_else(|| AiMuxError::Other("router: all models failed".into())))
    }
}
```

**流式 fallback 不做的理由**:流式 `do_stream` 返回后已向用户吐 `StreamStart`,此时若上游断了,重试会重复吐 token。Hermes/LiteLLM 同此策略(路由在前,委托后,流中不 fallback)。

### 3.4 内置策略

#### 3.4.1 RuleRouter(规则路由 + 静态优先级)

```rust
/// 静态优先级路由:总是选第 0 个模型,fallback 时按序往下。
/// 最简形态,等价于"主模型 + 备用"。
pub struct RuleRouter;

impl Router for RuleRouter {
    fn route(&self, _prompt: &LanguageModelPrompt, _models: &[ChildModel]) -> Result<usize, AiMuxError> {
        Ok(0)
    }
}
```

更复杂的规则路由(按 cost/latency 排序)由 `WeightedRouter` 表达:

```rust
/// 按 weight 排序选模型(weight 可编码 cost/latency/优先级)。
pub struct WeightedRouter { weights: Vec<f64> }
impl Router for WeightedRouter {
    fn route(&self, _prompt: &LanguageModelPrompt, models: &[ChildModel]) -> Result<usize, AiMuxError> {
        // 选 weight 最高(或最低,看语义)的
        self.weights.iter().enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
            .map(|(i, _)| i)
            .ok_or_else(|| AiMuxError::Other("router: no models".into()))
    }
}
```

#### 3.4.2 ComplexityRouter(LLM 分类器路由)

复用 aimux 自身 `LanguageModel` 调小模型判断难度——**零外部依赖**,这是 aimux 统一 290+ provider 后的独有甜区。

```rust
/// LLM 分类器路由:调一个小模型判断 prompt 复杂度,分到 SIMPLE/COMPLEX tier。
pub struct LlmClassifierRouter {
    classifier: ChildModel,      // 小模型(如 gpt-4o-mini / haiku)
    /// tier → models 索引映射。如 SIMPLE→0(小模型),COMPLEX→1(大模型)。
    tier_mapping: HashMap<ComplexityTier, usize>,
    /// 分类 prompt(可定制)。
    classify_prompt: String,
}

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq)]
pub enum ComplexityTier { Simple, Medium, Complex, Reasoning }

impl Router for LlmClassifierRouter {
    fn route(&self, prompt: &LanguageModelPrompt, _models: &[ChildModel]) -> Result<usize, AiMuxError> {
        // 1. 用 classifier 调小模型,结构化输出判断 tier
        let tier = self.classify(prompt)?;
        // 2. 查映射选模型
        self.tier_mapping.get(&tier).copied()
            .ok_or_else(|| AiMuxError::Other(format!("router: no model for tier {tier:?}")))
    }
}
```

**延迟开销**:~100-500ms(一次小模型 inference)。成本远低于"什么都走大模型"。LiteLLM Auto Routing 同此模式(其 `classifier_type: llm`)。

**fallback**:分类器自身失败时,降级到规则路由(选第一个 / 默认大模型)。这在 `LlmClassifierRouter` 内部处理,不依赖 RouterModel 的 fallback。

#### 3.4.3 ModalityRouter(视觉/文本分流)

这是"视觉路由"的正确形态——**不独立做,作为通用 Router 的一个策略谓词**(~50 行)。

```rust
/// 模态分流:prompt 含图像 → 多模态模型索引;纯文本 → 文本模型索引。
pub struct ModalityRouter {
    vision_model_idx: usize,   // 支持图像输入的模型
    text_model_idx: usize,     // 纯文本模型(更便宜)
}

impl Router for ModalityRouter {
    fn route(&self, prompt: &LanguageModelPrompt, _models: &[ChildModel]) -> Result<usize, AiMuxError> {
        // 扫 content parts 有无 Image
        let has_image = prompt.iter().flat_map(|m| m.content.iter())
            .any(|part| matches!(part, ContentPart::Image { .. }));
        Ok(if has_image { self.vision_model_idx } else { self.text_model_idx })
    }
}
```

**为什么视觉路由不独立做**(调研结论):
- 视觉输入管道已完备(`ContentPart::Image` 三大协议都支持转换),路由要解决的是"选哪个模型"。
- "按内容含不含图分流"是通用路由的一个 ~50 行策略谓词,单独立项必然被通用路由吞掉。
- "智能选最优视觉模型"(场景 B)本质就是智能路由,视觉只是能力维度之一,归 LLM 分类器/能力声明统一处理。

### 3.5 可插拔学习型路由(不进核心)

学习型路由(RouteLLM 风格 ML 分类器)**不内置**,但通过 `Router` trait 注入:

```rust
// 用户侧示例(不进 aimux)
struct RoutellmRouter { /* ort::Session 加载 ONNX 模型 */ }
impl Router for RoutellmRouter {
    fn route(&self, prompt: &LanguageModelPrompt, _models: &[ChildModel]) -> Result<usize, AiMuxError> {
        // 1. prompt → embedding
        // 2. ONNX 推理 → strong/weak 分数
        // 3. 分数 > threshold → 大模型,否则小模型
    }
}
```

理由:学习型需训练数据 + 模型权重 + 推理运行时(`ort`/`candle`/`burn`),引入 ~数十 MB 依赖,与 release profile(`opt-level="z"` + `strip` + `lto`,[Cargo.toml:25-30](../Cargo.toml#L25))最小二进制定位冲突。RouteLLM 泛化性好,直接复用其 Python 服务更优。aimux 只提供 trait 接口。

---

## 4. Integration Approach

### 4.1 FFI / binding 透明性(已验证)

- **C ABI**:`aimux_generate_text(handle, ...)` 只做 `get_model(handle) → Arc<dyn LanguageModel>` → `generate_text(&*model, ...)`,只依赖 trait。RouterModel 注册 handle 后直接可用,wire 层零改动。
- **Node**:`Model { inner: Arc<dyn LanguageModel> }`,`generateText`/`streamText` 只依赖 trait,零改动。
- **新增构造函数**(纯新增符号,不改现有 FFI):
  - C ABI:`aimux_router_new(handles: *const u64, len: usize, config_json: *const c_char) -> *mut c_char`——内部对每个 child handle 调 `get_model`,组装 `RouterModel`,再 `intern_model`。
  - Node:`router(models: Array<&Model>, opts) -> Model` napi factory。

### 4.2 generate_text / stream_text 入口零改动

[generate.rs:178](../aimux-core/src/generate.rs#L178) `generate_text(model: &dyn LanguageModel, ...)` 只接受 `&dyn LanguageModel`,RouterModel 传入即可工作。`provider()`/`model_id()` 返回 `"router"` 让现有 tracing span 正常打点。

### 4.3 子模型共享

同一子模型可被多个 RouterModel 持有(用 `Arc` clone),也可同时被 registry 与 composite 持有。child handle drop 后,子模型仍由 composite 的 Arc 保活。

---

## 5. Relationship with Existing RFCs

| RFC | 关系 |
|-----|------|
| [RFC-0022](0022-moa-single-fanout.md) | **共用 composite 骨架**。RouterModel 选 1 个,MoaModel 全调+聚合。两者都是 composite model 实现 trait,可共用 `ChildModel = Arc<dyn LanguageModel>`。 |
| [RFC-0016](0016-align-with-aisdk.md) | **正交**。路由是访问层组合,AISDK 对齐是单 provider 能力补齐。H4 多步工具循环明确不做,与路由的单次组合定位一致。 |
| [RFC-0005](0005-protocol-conversion.md) | **边界确认**。RFC-0005 结论"不做跨协议转换(网关的活)"。路由是"跨模型选一个(SDK 的活)",不碰协议转换。 |
| [RFC-0009](0009-request-resilience.md) | **同类延伸**。HTTP retry 是单 provider 内重试,RouterModel fallback 是跨模型容灾,同一职责层。 |

---

## 6. Non-Goals

1. **不内置学习型路由**(RouteLLM 风格 ML 分类器)。需训练 + 推理运行时,与最小二进制定位冲突。走 `Router` trait 注入,用户自带 `ort`/`candle`。
2. **不做流中 fallback**。流式 `do_stream` 返回后已吐 `StreamStart`,重试会重复 token。路由在前,委托后,流中不 fallback(Hermes/LiteLLM 同策略)。
3. **不做多步工具循环 / agent loop**(H4,RFC-0016 §7.5 明确不做)。RouterModel/MoaModel 都是单次 `generate_text`/`stream_text` 内完成。
4. **不做能力声明系统**(provider 声明 supports_vision/tools/reasoning)。薄版路由靠用户配置"哪个模型支持什么";能力数据工程(从 litellm `supports_vision` 补全 inventory)是后续独立工作,不在本 RFC。
5. **不做负载均衡 / 多 key 池 / 限流**。那是网关职责(RFC-0005 边界)。RouterModel 的 fallback 是容灾,不是负载均衡。

---

## 7. Scope of Changes

| 位置 | 改动 | 工作量 |
|------|------|--------|
| `aimux-core/src/composite.rs` | 新增:`ChildModel` 类型别名 + 共用辅助(usage 累加等) | ~50 行 |
| `aimux-core/src/router.rs` | 新增:`Router` trait + `RouterModel` + `FallbackPolicy` + `RouterConfig` | ~200 行 |
| `aimux-core/src/router_strategies.rs` | 新增:`RuleRouter` / `WeightedRouter` / `LlmClassifierRouter` / `ModalityRouter` | ~300 行 |
| `aimux-core/src/lib.rs` | `pub mod composite; pub mod router; pub mod router_strategies;` + prelude | ~10 行 |
| `aimux-core/Cargo.toml` | `async-stream`(workspace 已有,与 RFC-0022 共用) | 1 行 |
| `aimux-ffi/src/lib.rs` | 新增 `aimux_router_new` C ABI | ~30 行 |
| `bindings/node/src/lib.rs` | 新增 `router(models, opts)` napi factory | ~30 行 |
| `bindings/{python,go,...}` | 薄透传 | 每语言 ~25 行 |
| 测试 | 每个策略单测 + RouterModel fallback 测 + object-safety 测 | ~250 行 |

**合计:~600-800 行。无 trait 改动、无破坏性变更、入口零改动。**

---

## 8. Risks

| 风险 | 等级 | 对策 |
|------|------|------|
| **LLM 分类器路由增加延迟**(100-500ms) | 中 | 默认关闭,用户显式选 `LlmClassifierRouter`;分类器失败降级规则路由;文档标注延迟开销 |
| **流式无 fallback 用户体验** | 低 | 文档明确"流式仅路由不 fallback";非流式有完整 fallback |
| **学习型路由用户误以为内置** | 低 | 文档明确"不内置,走 trait 注入";给 `RoutellmRouter` 示例代码 |
| **provider()/model_id() 返回 "router" 丢失下游信息** | 低 | 在 `provider_metadata` 补 `selected_provider`/`selected_model`;tracing span 可读 |
| **fallback 顺序与用户预期不符** | 低 | `RouterConfig` 可配 fallback 顺序;默认按 models 数组序 |

---

## 9. Resolved Questions (2026-08-13)

1. **`provider()`/`model_id()` 返回值**:返回固定 `"router"`,`provider_metadata` 补被选模型信息。
2. **LLM 分类器的成本归属**:分类器消耗的 token **累加进总 usage**（用 `add_usage` helper）。顶层 `usage` 是累加总和；明细拆分放 `provider_metadata.composite_usage`，**统一结构**为 `{ "participants": [{ "role": "classifier"|"selected", "model": "model-id", "usage": Usage }], "total": Usage }`，与 RFC-0022 的 MoA 场景共用同一 schema（role 不同但结构一致）。
3. **能力声明系统**:不做，留后续。
4. **fallback 与 retry 的交互**:每个子模型的 `max_retries` 仍生效（单 provider 内 retry 先跑完，再 fallback 下一个）。
5. **`WeightedRouter` 方向语义**:**weight 高 = 优先级高 = 选 max**。用户想按成本选（低成本优先）传倒数或负数即可。
6. **`config_snapshot` 聚合策略（修订）**:Composite model **不伪造自己的 config_snapshot**。P1 使用默认 `ProviderRecord::minimal("router", "router")`。真正的录制/回放对 composite 的支持是 **RFC-0023 录制层的 nested 设计**（见下），不是 composite 自身要解决的。

### RFC-0023 录制层的 nested 设计（follow-up note）

Composite model 的子模型是真正发 HTTP 请求的实体。录制应该记的是**每个子模型的实际调用**，而非 composite 的虚拟快照。设计方向：

- **`TraceLayer` 装饰位置**：对 RouterModel，外层录制足够（它只调一个子模型，委托是透明的）。对 MoaModel，若需要看每个 reference 的调用详情，应在**子模型级别**装饰 TraceLayer，用同一 `call_id`/`trace_id` 关联。
- **trace 关联**：composite 的 `do_generate`/`do_stream` 持有 `CallOptions.call_id`（已有字段）。子模型调用复用同一 `call_id`（或加 `step` 字段区分 reference 0/1/2/aggregator），录制层按 `call_id` 聚合成树形调用链。
- **不扩展 `ProviderRecord`**：不加 `children` 字段。每个子模型的录制是独立的 `TraceRecord`，通过 `call_id` 关联，不是嵌套结构。这与现有录制架构（扁平 jsonl + call_id 关联）一致。
- **P1 不实现**：nested 录制是 RFC-0023 的增强，不阻塞 composite model 的 P1。P1 落地后 composite 调用会被录制（外层 trace），但子模型级别的明细录制需要后续在 TraceLayer 侧补。

---

## 10. Implementation Order

| 阶段 | 内容 | 依赖 | 状态 |
|------|------|------|------|
| **P1** | composite 骨架(`composite.rs`)+ `Router` trait + `RouterModel` + `RuleRouter`/`WeightedRouter` + 单测 | 无 | 待实施 |
| **P2** | `ModalityRouter`(视觉分流策略)+ `LlmClassifierRouter` + 单测 | P1 | 待实施 |
| **P3** | FFI(`aimux_router_new`)+ Node(`router` factory)+ 其他 binding 透传 | P1 | 待实施 |
| **P4**(可选) | 能力声明数据工程(从 litellm 补全)+ 基于能力的自动路由 | 独立 | 待评估 |
| **P5**(不进核心) | `RoutellmRouter` 示例(文档/示例仓库,用 ort 加载 ONNX) | P1 | 文档示例 |

**建议先做 P1**:骨架 + 规则路由 + fallback 立即可用(容灾 + 成本优化)。P2 加策略。P3 同步 binding。
