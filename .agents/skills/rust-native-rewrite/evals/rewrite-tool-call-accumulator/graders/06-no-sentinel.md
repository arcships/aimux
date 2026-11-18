---
type: llm
focus: { source: file, path: src/lib.rs }
---

查找工具调用不再返回 -1 哨兵、不再用 i64 作内部下标，改成 Option<usize> 或等价写法；输入 Delta.index 上的 -1 约定可以保留
