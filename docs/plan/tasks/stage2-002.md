---
id: stage2-002
scope: aimux-providers/tests
status: pending
depends-on: [stage2-001]
---

# stage2-002: 测试套件（退役回归 + max_tokens_key + warning）

## objective

一个交付物："退役行为与保留行为全部被测试锁定"。包含：

1. **现有 deepseek 测试改造**：凡是断言"reasoning:'none' → thinking:{type:disabled} 注入"的用例，改为断言"透传 reasoning_effort + 不含 thinking 注入"（退役语义）；`provider_options["deepseek"]` 相关用例按新语义处理
2. **新测试文件 `tests/reasoning_map_test.rs`**（或并入现有文件）：
   - I4：退役后 DeepSeek 请求体不含 thinking（除非用户 bodyOverrides 注入）
   - I5：用户 `body_overrides: { thinking: { type: 'disabled' } }` → 请求体含之（阶段 1 能力回归）
   - max_tokens_key 矩阵：6 家 `"max_tokens"` + groq/heroku `"max_completion_tokens"`（wiremock 请求体断言）
   - warning 两路径
   - reasoning_effort 直传：none/minimal/low/medium/high/xhigh 7 档无归一化
3. **防死代码对照**：max_tokens_key 内置条目 × 测试对照表（评审核对）

## context

- 设计：[docs/plan/analysis/stage2-reasoning-map.md](../analysis/stage2-reasoning-map.md) §4 I4-I5、§5
- 现有测试：[deepseek_test.rs](D:\code\aimux\aimux-providers/tests/deepseek_test.rs)、[openai_model_test.rs](D:\code\aimux\aimux-providers/tests/openai_model_test.rs)（mock 模式）

## path

- `aimux-providers/tests/deepseek_test.rs`（改造）
- `aimux-providers/tests/reasoning_map_test.rs`（新）
- 可能涉及 `aimux-providers/tests/openai_model_test.rs`（如 deepseek 用例在其中）

## verification

1. `cargo test -p aimux-providers --test reasoning_map_test` 全绿
2. 改造后 deepseek/openai 测试全绿
3. `cargo test -p aimux-providers --tests` 全绿
4. max_tokens_key 对照表完整
