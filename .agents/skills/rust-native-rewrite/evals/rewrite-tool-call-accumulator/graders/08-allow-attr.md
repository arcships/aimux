---
type: llm
focus: trace
weight: 1
---

ToolCall.index 上不带理由的 #[allow(dead_code)] 被去掉（字段删掉或真正用上），或改成带 reason 的 #[expect(dead_code, ...)]
