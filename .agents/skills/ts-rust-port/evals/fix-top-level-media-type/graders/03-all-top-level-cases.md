---
type: llm
focus: { source: file, path: tests/is_url_supported_test.rs }
---

加了上游 top-level-only 那一组的四个用例：image 匹配 image/* 键、image 匹配 * 键、image 不匹配 image/png 键、image 不匹配 audio/* 键；场景和期望值和上游一致
