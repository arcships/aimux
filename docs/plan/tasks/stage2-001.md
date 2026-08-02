---
id: stage2-001
scope: aimux-providers/src/openai
status: pending
depends-on: []
---

# stage2-001: 退役 RequestBodyOverride + max_tokens_key + warning

## objective

一个交付物："厂商映射全部移出代码，max_tokens_key 修复，warning 不静默"。用户 API 零变化。包含：

1. **退役**（mod.rs + convert.rs）：
   - 删除 `RequestBodyOverride` 枚举、`request_body_override` profile 字段
   - 删除 `apply_deepseek_override` 函数（convert.rs ~1485-1552）
   - 删除 reasoning_effort 归一化（xhigh→max/minimal→low + warning），改为**直接透传** 7 档
   - `OpenAICompatProfile::deepseek()` 回归 `full()`（函数保留，薄封装结构不变）
2. **max_tokens_key**（mod.rs + convert.rs）：内部字段 + 分支改造（`"max_tokens"` / `"max_completion_tokens"` / None=现状推断）
3. **warning 删除**（convert.rs ~1332-1344）：不可达死代码（直传语义下 `is_custom_reasoning=true` 时 resolved 必然 Some），删除——v3 无"未翻译"状态

## context

- 设计：[docs/plan/analysis/stage2-reasoning-map.md](../analysis/stage2-reasoning-map.md) §2.1-2.4、§4 I1-I3
- RFC：[rfc/0017-provider-config-dx.md](../../../rfc/0017-provider-config-dx.md) §2.3、阶段 2
- 现有：[mod.rs](D:\code\aimux\aimux-providers/src/openai/mod.rs)（RequestBodyOverride ~53-56、deepseek() profile ~82-91）、[convert.rs](D:\code\aimux\aimux-providers/src/openai/convert.rs)（build_request_body ~1098-1427、apply_deepseek_override 1485-1552）

## path

- `aimux-providers/src/openai/mod.rs`
- `aimux-providers/src/openai/convert.rs`
- `aimux-providers/src/openai_compat_registry.rs`（deepseek/groq 行若引用特化则同步）

## verification

1. `cargo check -p aimux-providers`
2. 文件内单测：
   - `reasoning:'none'` → 请求体含 `reasoning_effort:"none"`（透传），**不含** thinking 注入
   - `reasoning:'xhigh'` → 请求体含 `reasoning_effort:"xhigh"`（直传，无归一化）
   - 无 warning 断言（直传语义下不该有"未翻译"warning——防未来误加）
   - `max_tokens_key=Some("max_tokens")` + 推理模型 → 发 `max_tokens` 不含 `max_completion_tokens`
   - `max_tokens_key=Some("max_completion_tokens")` → 发 mct（非推理分支也发 mct）
3. `cargo test -p aimux-providers --tests`（deepseek 相关旧测试按退役语义更新后全绿——在 stage2-002 处理测试改造,本任务保证编译与基础测试绿）
