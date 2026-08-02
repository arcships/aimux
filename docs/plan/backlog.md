# Backlog — 非阻塞发现 / 后续项

> 按计划评审与开发过程中产生的非阻塞项归档于此。阻塞项必须当轮修复，不得入此表。

## RFC-0017 阶段 2 相关（v3：完全用户定义）

| # | 来源 | 内容 | 状态 |
|---|---|---|---|
| B1 | 调研 batch-04 | `reasoning_effort` 档位差异（kimi-k3 low/high/max、perplexity 四档）——档位不翻译（透传+厂商自决），知识进用户手册 | 已闭合（文档化） |
| B2 | 调研 batch-05 | `reasoning_format`（stepfun general/deepseek-style）硬编码 `provider=="groq"`——用户 bodyOverrides 表达，手册文档化 | 待文档化 |
| B3 | 调研 batch-06 | venice_parameters/hetzner chat_template_kwargs 等封闭字段——用户 bodyOverrides，手册文档化 | 待文档化 |
| B5 | 调研全局 | 7 处 registry base_url 确定性错误 + ~20 处存疑——P0 修复（数据层），与阶段 2 并行不冲突 | 待开工 |
| B6 | 调研 batch-02 | DeepSeek 是否接受 `max_completion_tokens` 待实测——实测后决定 DeepSeek 是否加 `max_tokens_key="max_tokens"`（v3 下它已无特化，纯通用路径，优先级降低） | 待实测 |
| B8 | 调研 batch-03 | Heroku 的 `allow_ignored_params` 机制——max_tokens_key 之外的特殊参数行为，用户 bodyOverrides + 手册文档化 | 待文档化 |
| B9 | verify stage2-001 F7 | 厂商级 `max_tokens_key` 数据接线未做（6 家 `"max_tokens"` + groq/heroku `"max_completion_tokens"`）——**须在 stage2-002/003 收尾前接线**，否则 groq/heroku 修复对真实流量不生效 | 待 stage2-002 |
| B10 | verify stage2-001 F8 | `Some("max_tokens")` + 显式 `maxCompletionTokens` 被静默丢弃——行为方向符合设计，但需在用户手册标注（用户应改用 `max_output_tokens`） | 待文档化 |

## 已闭合（v3 变更消除）

| # | 原条目 | 闭合原因 |
|---|---|---|
| B4 | 不可关模型按 model 前缀分派（kimi-k2.7-code 等） | 不内置任何映射——无需分派，用户 bodyOverrides + warning 兜底 |
| B7 | alibaba 内置 EnableThinking 对 qwen3-thinking-2507 的 by-model 误伤 | 不内置——无误伤面 |
