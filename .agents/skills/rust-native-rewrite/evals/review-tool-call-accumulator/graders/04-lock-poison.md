---
type: llm
focus: trace
weight: 1
---

指出 record 里的 .lock().unwrap()，并按配置的项目决定建议把 poison 转成错误返回，而不是 expect 或 into_inner
