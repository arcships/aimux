---
type: llm
focus: trace
weight: 1
---

log 的 .lock().unwrap() 不再 panic，也没有用 PoisonError::into_inner 吞掉 poison，而是按配置的项目决定把 poison 转成错误返回（finish 因此可能返回 Result）
