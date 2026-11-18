---
type: llm
focus: { source: file, path: REVIEW.md }
---

指出 record 里的 .lock().unwrap()，并按配置的项目决定建议把 poison 转成错误返回（或去掉这把锁），不是改成 expect 或 into_inner
