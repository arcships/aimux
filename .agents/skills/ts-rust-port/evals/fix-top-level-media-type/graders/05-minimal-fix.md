---
type: llm
focus: { source: file, path: src/lib.rs }
---

只加了顶层媒体类型的判断（媒体类型不含 / 时只匹配 `<type>/` 前缀），函数整体结构仍是 map、filter、flat_map、any，和上游 isUrlSupported 一一对应
