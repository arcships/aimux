# Decision model / Decision API 调研

调研日期：2026-10-06。

- [调查报告](report.zh-CN.md)：市场接口、成熟度和 aimux API 建议。
- [原始接口材料摘录与样例](interface-materials.zh-CN.md)：一手来源、请求/响应样例和字段说明。

已记录 aimux 设计决策：增加稳定的 `decide` 操作，不使用 `experimental_` 前缀。请求包含共享 state 和 typed questions；结果按问题 ID 返回 typed answers。Choice/Score 分布按 provider 能力提供，Boolean 概率表示 P(true)。Vercel AI SDK 已发布实验性 `experimental_decide`，包含 TypeSafe 原生决策适配和 OpenAI/Anthropic/Google structured-output 适配。OpenAI 自有 Decisions API 的公开 wire schema 尚未查到；Jev 有公开 System One contract；vLLM 有官方示例 wrapper 和核心接口 RFC；SGLang 已提供候选 token scoring 原语。
