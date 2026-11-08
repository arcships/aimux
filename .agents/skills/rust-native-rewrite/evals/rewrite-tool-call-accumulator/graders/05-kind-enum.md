---
type: llm
focus: { source: file, path: src/lib.rs }
---

StreamPart 的 type 不再是和字符串字面量比较的 String：改成了 serde rename 成 kebab-case 的 enum，或整个 StreamPart 改成按类型分变体的 enum
