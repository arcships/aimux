---
type: llm
focus: { source: file, path: src/lib.rs }
---

行为没有被顺手改：finish 时没有参数的调用仍补成 "{}"，完成后的 delta 仍被忽略，finish 之后 push 和 finish 都不再输出
