---
type: llm
focus: trace
weight: 1
---

record(&Vec<StreamPart>) 改成接受切片，finish(&Option<String>) 改成 Option<&str>、Option<String> 或等价写法
