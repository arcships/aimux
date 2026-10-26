---
type: llm
focus: trace
weight: 1
---

find_call 的 -1 返回值和 i64 下标被去掉，改成 Option<usize> 或等价写法；输入 Delta.index 上的 -1 约定可以保留，但内部不再用 -1 做哨兵
