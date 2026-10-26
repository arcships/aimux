---
type: llm
focus: trace
weight: 1
---

StreamPart 的 type 字段从 String 改成了 enum（serde rename 成原来的 kebab-case 字符串），或整个 StreamPart 改成按类型分变体的 enum；输出的 JSON 和原来逐字一致
