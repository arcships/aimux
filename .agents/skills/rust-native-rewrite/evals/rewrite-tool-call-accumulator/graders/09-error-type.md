---
type: llm
focus: { source: file, path: src/lib.rs }
---

错误类型用 thiserror 派生，不是 String；两条错误消息仍是 "Expected 'id' to be a string." 和 "Expected 'function.name' to be a string."
