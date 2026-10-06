# Decision model / Decision API 调研

调研日期：2026-10-06。

aimux 支持范围限于原生决策接口和候选评分能力。不实现 structured-output
模拟 adapter 或生成式 fallback；下述 AI SDK 实现仅作为调研背景。

开发第一优先级为 TypeSafe 官方 Jev System One 接入，默认端点为
`https://api.typesafe.ai/v1/systemone`。仅使用 TypeSafe 官方 contract；
具体实现与优先级见 [RFC-0037](../../../rfc/0037-decision-api.md)。

已记录 aimux 设计决策：增加稳定的 `decide` 操作，不使用 `experimental_` 前缀。请求包含共享 state 和 typed questions；结果按问题 ID 返回 typed answers。Choice/Score 分布按 provider 能力提供，Boolean 概率表示 P(true)。Vercel AI SDK 已发布实验性 `experimental_decide`，包含 TypeSafe 原生决策适配和 OpenAI/Anthropic/Google structured-output 适配。OpenAI 自有 Decisions API 的公开 wire schema 尚未查到；Jev 有公开 System One contract；vLLM 有官方示例 wrapper 和核心接口 RFC；SGLang 已提供候选 token scoring 原语。
