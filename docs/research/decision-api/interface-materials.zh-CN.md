# 原始接口文档材料（摘录）

> 2026-10-08 更新：OpenAI 已有公开 beta `/v1/decisions` 和请求／响应 schema。本文保留 10 月 6 日的调研历史，旧的 limited-preview／schema 未公开结论已过时；当前字段映射、官方运行时清单和开发阶段见 [RFC-0038](../../../rfc/0038-decision-provider-expansion.md)。

> 来源页面和接口随服务迭代。以下为截至 2026-10-06 的一手文档要点与最小化请求/响应材料，保留字段名和路径；不是对网页的逐字转载。报告见 [report.zh-CN.md](report.zh-CN.md)。

## 1. Jev / TypeSafe 的 System One API

官方文档入口：[Introduction](https://docs.typesafe.ai/introduction)、[API reference](https://docs.typesafe.ai/api)、[Quick start](https://docs.typesafe.ai/introduction/quickstart)、[SDK 常量](https://docs.typesafe.ai/sdk/python/api/constants)。Jev 是 TypeSafe 的首个公开 System One 模型，以下仅记录官方接口。

```http
POST /v1/systemone HTTP/1.1
Host: api.typesafe.ai
Authorization: Bearer $TYPESAFE_API_KEY
Content-Type: application/json
```

请求包含 `model`、共享 `state` 和 id→question 的 `questions`。state 支持字符串、对象、数组。推荐模型为 `jev-latest`；响应 `model` 给出实际版本，例如 `jev-1.13.0`。

```json
{
  "model": "jev-latest",
  "state": "We were billed twice and need help before Friday.",
  "questions": {
    "human": {"type": "noul", "instructions": "Does this need a human reply?"},
    "queue": {
      "type": "choice",
      "instructions": "Choose the team that should own the first reply.",
      "criteria": {"billing": "Invoices and duplicate charges", "technical": null}
    },
    "urgency": {
      "type": "score",
      "instructions": "Rate the urgency from low to high.",
      "criteria": ["low", "normal", "high"]
    }
  }
}
```

以下响应为按官方 schema 构造的示例，不是线上录制：

```json
{
  "model": "jev-1.13.0",
  "answers": {
    "human": {"type": "noul", "noul": 0.89},
    "queue": {"type": "choice", "choice": "billing", "probabilities": {"billing": 0.91, "technical": 0.09}, "confidence": 0.91},
    "urgency": {"type": "score", "score": 1.72, "legend": {"0": "low", "1": "normal", "2": "high"}, "probabilities": {"0": 0.1, "1": 0.08, "2": 0.82}, "confidence": 0.82}
  },
  "usage": {"input_tokens": 503, "output_tokens": 70}
}
```

Choice 最多 255 项；描述可为字符串、对象、数组或 null。Score criteria 为有序数组，支持 2–10 级，score 是概率加权的等级位置；legend 将索引映回描述。Noul 可带 true/false 描述，noul 表示 P(true)。instructions 和 Score 描述也允许字符串、对象或数组。官方未公布题目数量、ASCII ID 或文本长度限制。

aimux 的统一请求支持字符串/对象/数组形式的 instructions、Choice 描述和 Score 等级，并暴露 Noul 的 true/false criteria。Choice 省略描述时映射为官方允许的 null。

响应要求 model、answers 和 usage；Choice/Score 包含完整分布和 confidence。HTTP 401 表示认证错误，422 表示请求校验失败，429/529 为临时限流或过载。实际重试由 aimux Core 管理，保留 HTTP 上下文和 raw response。

## 2. OpenAI Decisions API

官方来源：[DevDay 2026 recap](https://openai.com/index/devday-2026-recap/)。官方描述其将 Luna 的能力聚焦到用户定义的一组问题和有限的预定义答案；可提供文本或图像上下文，答案用于分类、路由或选择 agent 下一步动作。公告标注 limited preview，并称计划随后扩大开放。

**截至调研日未找到公开的 API Reference、endpoint、JSON schema、SDK 方法或概率字段说明。**统一适配应等待正式 contract。当前可确认的能力范围为 typed finite-answer decision task。

## 3. Vercel AI SDK Decision API

官方 provider 文档：[TypeSafe](https://ai-sdk.dev/providers/ai-sdk-providers/typesafe-ai)、[OpenAI](https://ai-sdk.dev/providers/ai-sdk-providers/openai)、[Anthropic](https://ai-sdk.dev/providers/ai-sdk-providers/anthropic)、[Google](https://ai-sdk.dev/providers/ai-sdk-providers/google)。当前文档名为 `experimental_decide` 与 provider factory `decisionModel(modelId)`。初始 GitHub 实现 PR 使用过 `experimental_evaluate` / `evaluationModel` 名称；现行使用以 provider 文档为准。

```ts
import { experimental_decide } from 'ai';
import { typeSafeAi } from '@ai-sdk/typesafe-ai';

const result = await experimental_decide({
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
    requestsRefund: {
      type: 'boolean',
      instructions: 'Is the customer requesting a refund?',
    },
  },
});
```

统一输入字段：`model`、`state`、`questions`。问题 map 的 key 是调用方指定的 ID。支持问题类型为 `choice`、`score`、`boolean`。统一结果 `answers` 按相同 ID 索引。Score 可返回有序等级上的小数值；Boolean 的 `probability` 表示 P(true)。

TypeSafe 的 `@ai-sdk/typesafe-ai` adapter 将上述请求合并为一个 `POST /v1/systemone`，将 Boolean 映射成 `noul`。该 adapter 保留 Choice/Score 完整概率分布，原生 confidence 存于 `providerMetadata.typesafe.confidence[questionId]`。文档限制：Choice 1–255 个选项；Score 2–10 个等级；TypeSafe 返回值精度为小数点后两位，结果通过 rounding metadata 验证并保留原值。

OpenAI、Anthropic、Google adapter 以各家原生 structured-output API 生成一次结构化结果。Choice/Score 不提供概率分布。Boolean probability 是 prompt 估计，文档没有校准保证。SDK 校验结果字段和概率范围；拒答、截断、缺题或非法结果会使调用失败。模型需支持相应 structured output。

当前文档列出的 provider：TypeSafe、OpenAI、Anthropic、Google。Provider model 可用实例、custom provider alias 或 registry 解析。官方 OpenAI-compatible provider 文档只记载通用语言模型等 factory，没有 `decisionModel` 文档。vLLM、SGLang 接入需要自定义 provider adapter。

### 多模态输入状态

当前官方 decision provider 页面展示文本 state 和 JSON object state。Decision request schema 未记录图片 part、文件 part、媒体 URL 或二进制字段。AI SDK 的常规 `ModelMessage` 支持 ImagePart，provider 的普通语言模型 API 也有图像能力；这些输入能力没有列入 `experimental_decide` / `decisionModel` contract。当前集成按文本/JSON决策处理，图片决策需扩展 API schema 和各 adapter。

## 4. vLLM

官方文档：[Structured reads on DiffusionGemma](https://docs.vllm.ai/en/latest/examples/features/structured_diffusion/)。这是 vLLM 官方 example，不是默认 API server 的通用 endpoint。该 example 包装 DiffusionGemma，提供 `POST /v1/systemone`，接收 Jev 风格 `model/state/questions`，`questions` 是 id→`type/instructions/criteria`。支持 `noul/choice/score`，并可附带实现扩展如 `samples`、`steps`、`think`、`ask`、`depends_on`、`ask_if`、`alone`。图片可走 multipart（`request` JSON part + image parts）或 JSON data URL array。Example 另以 OpenAI-shaped `/v1/chat/completions` 返回 JSON 内容；`/v1/raw/chat/completions` 转发普通生成。

该例从离散扩散模型的固定答案槽位读取分布。响应包含 Jev 风格 answer 和 diagnostics（采样次数、时延、引擎等）。服务由独立 HTTP wrapper 调用 vLLM engine。默认 `vllm serve` 不包含此路径。

社区 RFC：[vLLM #59365](https://github.com/vllm-project/vllm/issues/59365) 提出核心 `/v1/decisions`，并保留 `/v1/systemone` 作为 Jev 兼容层。提议 backend 包括 `logit`（常规生成模型候选 label 的 next-token logits）、`encoder`（Laya）及 `canvas`（DiffusionGemma），输出概率和校准/审计元数据。该接口处于 RFC 阶段。RFC 作者还提供[演示 patch 仓库](https://github.com/Timiku/v1-decisions-vllm)。

另一个已有的常规能力是 [Structured Outputs](https://docs.vllm.ai/en/latest/features/structured_outputs/)，通过 JSON schema/choice/grammar 限制“生成什么文本”。它保证输出形式，不天然给出每个候选项的概率分布或校准置信度，因此不等价于 Decision API。

## 5. SGLang

主仓库来源：[dLLM serving roadmap #39499](https://github.com/sgl-project/sglang/issues/39499)。路线图将 Jev-style diffusion 决策放在开发项：DiffusionGemma pinned canvas、固定 answer slots、有界 denoising、label logprobs 和 cookbook；另一处将 causal decision track 分开追踪，列有未来共享 API 的设想。此路线图不是统一决策 endpoint/schema。

SGLang 已合并 [PR #40826](https://github.com/sgl-project/sglang/pull/40826)，在 `/v1/score` / Engine scoring 增加 per-item candidate token IDs、temperature scaling 和可选 logprobs。PR 限定 single-token candidates，并以 SemIf 为例。此改动提供候选评分原语。Jev 式 `state + typed questions -> answers` wire API 尚未定义；路线图将 shared API 列为后续方向。

独立社区项目：[LLM2Jev](https://github.com/Yinsongxu/LLM2Jev) 使用 SGLang/Transformers/MLX 对本地 LLM 做候选 logit scoring，并提供 `System One` 兼容 HTTP 服务。项目 README 记录 `POST /v1/systemone`，支持 Noul/Choice/Score 与文本/图像输入。该项目自行实现协议与概率映射/归一化；校准质量受模型模板和候选 tokenization 影响。

本次查阅的 SGLang 官方资料记录了 OpenAI-compatible chat/generate、constrained decoding 和 `/v1/score` 候选评分能力。官方资料尚未定义 Jev/OpenAI 式一等决策 endpoint。

## 接口对照速查

| 来源 | 路径/表面 | request 核心 | 概率/typed answer | 状态 |
|---|---|---|---|---|
| Jev 官方 | `POST /api/v1/systemone/` | model, state, questions map | 是；noul/choice/score | hosted 公共 API 文档 |
| OpenAI | 公开 endpoint/schema 未查到 | 官方只披露 text/image + 问题 + finite answers | 公布说明未明确字段/分布 | Decisions API limited preview |
| vLLM official example | `/v1/systemone` | Jev wire + example extensions | 是；含 diagnostics | 可运行 example wrapper |
| vLLM RFC | `/v1/decisions`（提议） | typed questions、backend/options | 提议概率、audit fields | RFC，非稳定 upstream contract |
| SGLang upstream | `/v1/score` candidate token scoring | per-item token candidates + calibration/logprobs | 有底层候选打分，不是 typed decision envelope | scoring 已合并；统一 route 未定 |
| LLM2Jev | `/v1/systemone` | Jev-compatible | 是，基于 logits scoring | 独立社区项目 |

## AI SDK 官方材料清单

- [TypeSafe provider: native typed decision mapping](https://ai-sdk.dev/providers/ai-sdk-providers/typesafe-ai)
- [OpenAI provider: Responses API structured-output decision model](https://ai-sdk.dev/providers/ai-sdk-providers/openai)
- [Anthropic provider: Messages structured-output decision model](https://ai-sdk.dev/providers/ai-sdk-providers/anthropic)
- [Google provider: Gemini structured-output decision model](https://ai-sdk.dev/providers/ai-sdk-providers/google)
- [OpenAI-compatible provider capabilities](https://ai-sdk.dev/providers/openai-compatible-providers)
- [Core decision API implementation PR #20848](https://github.com/vercel/ai/pull/20848)
- [TypeSafe native provider PR #20851](https://github.com/vercel/ai/pull/20851)
- [OpenAI, Anthropic, Google adapters PR #20858](https://github.com/vercel/ai/pull/20858)
- [Decision model registry PR #20875](https://github.com/vercel/ai/pull/20875)
