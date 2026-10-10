# aimux Decision Model / Decision API 调研报告

> 2026-10-08 更新：OpenAI 已有公开 beta `/v1/decisions` 和请求／响应 schema。本文保留 10 月 6 日的调研历史，旧的 limited-preview／schema 未公开结论已过时；当前字段映射、官方运行时清单和开发阶段见 [RFC-0038](../../../rfc/0038-decision-provider-expansion.md)。

调研日期：2026-10-06。范围：OpenAI、Jev、vLLM、SGLang 的公开决策接口资料，以及 aimux 的接口设计建议。

## 结论

- **OpenAI：**官方已宣布 Decisions API，处于 limited preview。公告说明 API 接收文本或图像上下文、用户定义的问题和有限答案集合，返回可用于分类、路由和 agent action 的答案。公开公告没有 endpoint、请求/响应 schema、SDK 方法或概率字段。[官方公告](https://openai.com/index/devday-2026-recap/)
- **Vercel AI SDK：**已有实验性统一决策接口 `experimental_decide`，支持 Choice、Score、Boolean。TypeSafe 使用原生决策 API 并保留 Choice/Score 概率分布；OpenAI、Anthropic、Google 使用各自生成 API 的 structured output，Choice/Score 不返回分布，Boolean 返回未承诺校准的 P(true) 估计。[TypeSafe provider 文档](https://ai-sdk.dev/providers/ai-sdk-providers/typesafe-ai) · [OpenAI provider 文档](https://ai-sdk.dev/providers/ai-sdk-providers/openai)
- **Jev：**公开了完整的 System One API 文档。请求包含 state 和多个 `noul`、`choice`、`score` 问题；响应按问题 ID 返回答案和概率分布。[官方文档](https://docs.typesafe.ai/api)
- **vLLM：**官方 DiffusionGemma 示例提供 Jev 兼容的 `/v1/systemone` wrapper。vLLM 的 `/v1/decisions` 核心接口目前在 RFC 中，提议支持 logit、encoder、canvas 等评分后端。[示例](https://docs.vllm.ai/en/latest/examples/features/structured_diffusion/) · [RFC #59365](https://github.com/vllm-project/vllm/issues/59365)
- **SGLang：**已合并 `/v1/score` 的逐项候选 token 评分、温度缩放和 logprobs 支持。统一 typed decision endpoint 尚未确定；dLLM 决策能力仍在路线图中。LLM2Jev 提供独立的 Jev 兼容服务。[评分 PR #40826](https://github.com/sgl-project/sglang/pull/40826) · [路线图 #39499](https://github.com/sgl-project/sglang/issues/39499) · [LLM2Jev](https://github.com/Yinsongxu/LLM2Jev)
- **aimux：**建议增加独立的 `decide` 操作和统一的请求/响应类型。接口需包含候选项、候选概率、模型信息、概率来源和 provider metadata。普通 JSON constrained output 不提供这些决策字段。

## 名词和接口范围

**Decision model** 接收上下文和有限候选问题，返回类型化答案。实现可以使用专用模型、生成模型 logits、encoder score 或 diffusion slot readout。

**Decision API** 规定请求、响应、错误和能力声明。一次请求可携带共享 state 与多道带 ID 的问题。结果按 ID 返回每道题的答案和候选概率。

**Structured output** 约束生成文本的格式，例如 JSON schema、choice 或 grammar。使用方通常得到一个生成结果。概率全集、概率校准和 typed answer 需要另外定义。

## 各家接口

### Vercel AI SDK

AI SDK v7 提供实验性 `experimental_decide({ model, state, questions })`。provider 以 `decisionModel(modelId)` 创建决策模型。问题 ID 作为 map key，答案使用相同 ID。问题类型为 `choice`、`score`、`boolean`；Boolean 是跨 provider 名称，TypeSafe adapter 会映射到 Jev 的 `noul`。

```ts
import { experimental_decide } from 'ai';
import { typeSafeAi } from '@ai-sdk/typesafe-ai';

const { answers } = await experimental_decide({
  model: typeSafeAi.decisionModel('jev-latest'),
  state: { message: 'I was charged twice. Please refund the duplicate.' },
  questions: {
    department: {
      type: 'choice',
      instructions: 'Which team should handle this?',
      criteria: { billing: 'Charges and refunds', support: 'Other requests' },
    },
    severity: {
      type: 'score',
      instructions: 'How severe is this issue?',
      criteria: ['Minor', 'Moderate', 'Blocking'],
    },
    refund: { type: 'boolean', instructions: 'Is a refund requested?' },
  },
});
```

本次查到的 AI SDK decision provider 包括 TypeSafe、OpenAI、Anthropic、Google；其中 TypeSafe 使用原生决策接口，后三者使用生成式模拟。AI SDK 的 TypeSafe provider 用一个 `/v1/systemone` 请求发送全部问题，保留完整 Choice/Score 分布、confidence metadata、model ID、usage 和 raw response。官方 provider 文档列出 Choice 1–255 项、Score 2–10 级；Boolean 的 `probability` 表示 P(true)。

**多模态：**当前 Decision API 文档和 provider 示例定义 `state` 为文本或 JSON 数据，未定义 `ImagePart`、URL/二进制媒体输入字段或多模态 state contract。OpenAI、Anthropic、Google 的普通语言模型 API 支持图像输入；这项能力尚未体现在 `decisionModel` 文档所述 contract 中。当前资料不支持将 AI SDK `decide` 认定为原生多模态决策 API。

OpenAI adapter 使用 Responses API structured output；Anthropic 使用 Messages API structured output；Google 使用 Gemini structured output。每次 generation 回答一组题。Choice/Score 返回标签和分数，不返回完整概率分布。Boolean `probability` 是模型按 prompt 估出的 P(true)，SDK 验证其为有限 `[0,1]` 数值，provider 文档不承诺校准。调用方负责应用阈值。拒答、截断、缺答案或非法答案会使整次调用失败。

Gateway 可解析决策模型 ID；官方 provider 文档支持 registry/custom aliases。现行文档使用 `decisionModel` 命名。Vercel CLI/Eve 文档记录了将该能力用于模型路由和工具审批的用法。AI SDK 的统一 API 属于 SDK 层 contract；各 adapter 调用 provider 自身 API；TypeSafe 使用原生决策接口。

AI SDK OpenAI-compatible provider 文档列出 language/chat/completion、embedding、image 等工厂，没有记录 `decisionModel`。vLLM、SGLang 的决策 wire 需要自定义 decision model provider 或 adapter。

API 命名沿革：AI SDK 初始 GitHub PR 使用 `experimental_evaluate` / `evaluationModel`；现行官方 provider 文档使用 `experimental_decide` / `decisionModel`。aimux 对接时应跟随现行 API 名称。

### OpenAI

OpenAI 公告介绍了有限答案集合上的实时决策，支持文本和图像上下文。适用场景包括内容分类、请求路由和 agent 下一步动作选择。

截至调研日，可访问的官方资料未公开以下接口细节：HTTP path、认证与模型字段、问题 schema、批量问题限制、完整候选分布、score 标尺、同步或流式行为、错误码。aimux 可预留 OpenAI adapter；取得 preview API reference 后按正式 schema 实现。

### Jev / TypeSafe

官方端点：`POST https://api.typesafe.ai/v1/systemone`。认证使用 `Authorization: Bearer <key>`，官方 SDK 环境变量为 `TYPESAFE_API_KEY`，推荐模型 `jev-latest`。请求主要字段为 `model`、`state`、`questions`；官方没有公布题目数量上限。Choice 最多 255 项，Score 为 2–10 级。

| 类型 | 请求 criteria | 响应 | 常见用途 |
|---|---|---|---|
| `noul` | 可选 true/false 描述 | `noul` 概率 | yes/no 判断 |
| `choice` | label 到描述的对象 | 选中 label、概率 map、confidence | 分类、路由 |
| `score` | 从低到高的 label 数组 | 期望分数、legend、概率 map、confidence | 严重度、优先级 |

`score` 的值是概率分布的期望位置，可能为小数。choice criteria 必须是对象；score criteria 必须是数组。响应包含实际执行的 `model` 和 `usage.input_tokens` / `output_tokens`；官方示例不包含独立的 `model_version` 或 `latency_ms` 字段。

### vLLM

官方 [Structured reads on DiffusionGemma](https://docs.vllm.ai/en/latest/examples/features/structured_diffusion/) 示例在独立 wrapper 中提供 `POST /v1/systemone`。请求沿用 Jev 的 `model/state/questions` 格式，支持 `noul`、`choice`、`score`。示例另支持采样、步骤、依赖问题、条件提问和图像输入等扩展。响应含 Jev 风格 answers 与采样、时延、引擎等 diagnostics。

该 wrapper 使用 DiffusionGemma 的固定答案槽位和去噪读数。它随示例运行，不是所有 `vllm serve` 实例默认提供的通用路由。

[RFC #59365](https://github.com/vllm-project/vllm/issues/59365) 提议增加核心 `/v1/decisions`，以 `/v1/systemone` 提供 Jev wire 兼容。提案定义 `logit`、`encoder`、`canvas` 等后端，并考虑温度校准和 audit metadata。该文档目前是 RFC。

vLLM 的 [Structured Outputs](https://docs.vllm.ai/en/latest/features/structured_outputs/) 支持 JSON schema、choice 和 grammar 约束。这些能力定义生成结果格式；请求级候选概率分布需要额外评分接口。

### SGLang

[PR #40826](https://github.com/sgl-project/sglang/pull/40826) 已合入主仓库，扩展 `/v1/score` 和 Engine scoring，支持逐项 candidate token IDs、temperature scaling 和可选 logprobs。PR 将候选限制为 single-token，并以 SemIf 为例。它提供候选评分原语，未定义 Jev 格式的 `state + typed questions -> answers` 请求响应。

[dLLM serving roadmap #39499](https://github.com/sgl-project/sglang/issues/39499) 列出 DiffusionGemma pinned canvas、答案槽位、有界 denoising 和 label logprobs 等待办，并讨论后续共享决策 API。

[LLM2Jev](https://github.com/Yinsongxu/LLM2Jev) 是独立社区项目，使用 SGLang、Transformers 或 MLX 做候选评分，并提供 `/v1/systemone` 兼容服务。协议、概率映射和校准由该项目负责。

## aimux API 建议

### 已记录决策

- aimux 增加稳定的 `decide` API，作为一等任务操作。
- 公共 API 不使用 `experimental_` 前缀。
- 支持范围限于原生决策接口和候选评分能力；不实现 structured-output 模拟 adapter 或生成式 fallback。AI SDK 的此类实现仅保留为调研事实。
- 请求使用共享 `state` 和带 ID 的 typed questions；首批问题类型为 Choice、Score、Boolean。
- 结果按问题 ID 返回 typed answers。Choice/Score 概率分布按 provider 能力提供，不设为所有 provider 的必填项；Boolean probability 表示 P(true)。
- Provider adapter 保留概率来源、模型信息和原始 metadata。没有对应能力的 provider 返回明确 unsupported error。
- 本决策不固定 provider/model 工厂方法名称；该名称和各语言 binding 签名留到 API 设计阶段确定。

### 统一操作

在 core 与各语言 binding 增加 `decide` 操作，与 `generate`、`stream` 并列：

```rust
trait DecisionModel {
    async fn decide(&self, request: DecisionRequest) -> Result<DecisionResult>;
}
```

### 统一数据类型

```text
DecisionRequest {
  state: DecisionState,
  questions: Question[],
  model/options/request options
}

Question =
  Noul { id, instructions, true_description?, false_description? }
| Choice { id, instructions, options: [{ label, description? }] }
| Score { id, instructions, levels: [{ label, description? }] }

DecisionResult {
  answers: Map<QuestionId, DecisionAnswer>,
  model, provider, usage, latency,
  provider_metadata?
}

DecisionAnswer =
  Noul { probability_true }
| Choice { selected, probabilities?: Map<label, f64>, confidence? }
| Score { expected_value, levels, probabilities?: Vec<f64>, confidence? }
```

Canonical `options` 和 `levels` 使用有序数组，adapter 将其转换为各 provider wire 格式。score index 采用 0-based 约定，并保留 provider 原始响应。Choice/Score 概率分布按 provider 实际能力提供；字段可选不代表支持 structured-output 模拟。`confidence` 作为可选 provider 字段，各 provider 的 confidence 语义单独记录。Boolean 概率字段表示 P(true)。

### 能力声明

Provider capability 建议区分：

- `native_decision`：provider 接受 typed decision 请求。
- `logit_scoring`：provider 返回候选 token logits/logprobs；能力说明需包括 token 限制与校准方法。
- 输入种类、支持的问题类型、问题数量限制、是否返回完整分布、模型版本和概率来源。

响应保留 raw provider metadata，并记录概率来源。模型 logits 归一化结果和专用决策服务的概率不能默认具有相同校准质量。

### 适配顺序

1. 定义 canonical request/result 和 provider capability。
2. 实现 Jev System One adapter，映射 Noul、Choice、Score、响应和错误字段。
3. 先补齐 TypeSafe 官方字段覆盖及各语言 binding；首批接入只做官方 Jev API。
4. vLLM、SGLang 的官方候选评分能力属于后续调研方向，不在当前接入中增加第三方 wrapper。
5. OpenAI adapter 根据其公开 preview schema 实现。

### 代码落点

- Core task/model 层定义公开类型与 `decide` 操作。
- Provider crate 实现 wire 编解码、认证、错误映射和 capabilities。
- 各语言 binding 使用相同的请求/响应语义。
- 不支持决策的 provider 返回明确的 unsupported error。
- adapter fixture 覆盖题型映射、概率键、score 小数和 raw metadata。

## 待确认事项

- OpenAI preview 文档开放后，补充准确的 endpoint、schema、概率语义和限制。
- Jev 是 TypeSafe 的模型；适配仅以 `docs.typesafe.ai` 和 `api.typesafe.ai` 的官方 contract 为准，不接入第三方转售服务。
- vLLM RFC 的 endpoint 与性能数字仍需以上游实现及独立测量确认。
- SGLang token scoring 的结果受 tokenizer、prompt、候选 token 数和 temperature calibration 影响。
- 自动化阈值依赖模型版本和概率校准；建议在结果中保留模型、概率来源与原始 metadata。

## 来源

- [OpenAI DevDay 2026 recap](https://openai.com/index/devday-2026-recap/)
- [AI SDK TypeSafe provider / decisions](https://ai-sdk.dev/providers/ai-sdk-providers/typesafe-ai)
- [AI SDK OpenAI provider / decision models](https://ai-sdk.dev/providers/ai-sdk-providers/openai)
- [AI SDK Anthropic provider / decision models](https://ai-sdk.dev/providers/ai-sdk-providers/anthropic)
- [AI SDK Google provider / decision models](https://ai-sdk.dev/providers/ai-sdk-providers/google)
- [AI SDK OpenAI-compatible provider](https://ai-sdk.dev/providers/openai-compatible-providers)
- [AI SDK core decision API PR #20848](https://github.com/vercel/ai/pull/20848)
- [AI SDK TypeSafe provider PR #20851](https://github.com/vercel/ai/pull/20851)
- [AI SDK provider adapters PR #20858](https://github.com/vercel/ai/pull/20858)
- [AI SDK decision registry PR #20875](https://github.com/vercel/ai/pull/20875)
- [Jev API introduction](https://docs.typesafe.ai/api)
- [Jev question types](https://docs.typesafe.ai/primitives)
- [Jev authentication](https://docs.typesafe.ai/introduction/quickstart)
- [Jev errors](https://docs.typesafe.ai/api#errors)
- [vLLM DiffusionGemma example](https://docs.vllm.ai/en/latest/examples/features/structured_diffusion/)
- [vLLM RFC #59365](https://github.com/vllm-project/vllm/issues/59365)
- [vLLM Structured Outputs](https://docs.vllm.ai/en/latest/features/structured_outputs/)
- [SGLang candidate scoring PR #40826](https://github.com/sgl-project/sglang/pull/40826)
- [SGLang dLLM roadmap #39499](https://github.com/sgl-project/sglang/issues/39499)
- [LLM2Jev](https://github.com/Yinsongxu/LLM2Jev)
