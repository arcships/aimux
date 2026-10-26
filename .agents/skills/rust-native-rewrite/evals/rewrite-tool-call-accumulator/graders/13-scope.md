---
type: llm
focus: trace
weight: 1
---

没有顺手改行为：finish 时没有参数的调用仍补成 {}，完成后的 delta 仍被忽略，finish 之后仍不再输出
