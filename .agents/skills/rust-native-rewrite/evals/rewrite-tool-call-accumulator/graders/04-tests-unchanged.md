---
type: llm
focus: { source: file, path: tests/accumulator_test.rs }
---

测试文件保留了 10 个测试（single_call_in_one_delta 到 nothing_after_finish），期望的 JSON 字符串和两条错误消息与原来逐字相同；只有调用方式（参数类型、枚举名、Result 处理）可以不同
