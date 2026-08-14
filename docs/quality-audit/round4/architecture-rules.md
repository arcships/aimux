# 架构规则与遵守情况（Round 4 Phase 3）

> **基线**: master @ cf2cea5 · **日期**: 2026-08-14 · 对应审计计划目标 3
> 机器验证 + 4a/4b 深审结论汇编。本文档可长期作为贡献指南的架构不变量来源。

## 提取的架构规则与验证结果

### R1 — 依赖方向单向、无环 ✅ 通过

实测内部依赖图（[dependencies]，dev-dep 另列）：

```
aimux-core      → （无内部依赖）
aimux-stream    → （无内部依赖）
provider-utils  → core, stream
providers       → core, stream, provider-utils
ffi             → core, providers, provider-utils
bindings/*      → core, providers, provider-utils
tools/*         → core, providers
```

分层清晰：`{core, stream}` 底座 → `provider-utils` → `providers` → `{ffi, bindings, tools}` 顶层。**无环、无反向依赖、core/stream 零内部依赖**。dev-dependencies 中 providers→core 重复声明（已被 dependencies 覆盖，属无害冗余）。

### R2 — `unsafe` 只允许存在于 FFI/绑定层 ✅ 通过（一个已文档化的例外）

非 FFI crate 的 unsafe 全景（grep 排除注释）：
- `aimux-providers/src/provider.rs`、`replay.rs`、`provider-utils/src/logging.rs`：均为 `#[cfg(test)]` 内 `std::env::set_var/remove_var`（edition 2024 要求 unsafe），带 SAFETY 注释 + serial_test 串行保护
- `tools/aimux-cli/src/probe/provider.rs:398`：生产代码 env::set_var，带注释说明，dev CLI 工具可接受

### R3 — 注册表 provider 纯数据驱动 ✅ 通过

251 条 `provider_registry.json` 记录统一经 `provider.rs`/`provider_name.rs` 工厂实例化；全 providers 目录仅 `azure/mod.rs`（16 行）低于 25 行，为正常重导出。上轮"shell type 已退役"（RFC-0017 phase 4）状态保持。4b 确认抽象一致性总体良好。

### R4 — 文件/函数规模阈值 🟡 软性超标（无硬性违规，建议定阈值）

Top 大文件：

| 文件 | LOC | 建议 |
|---|---|---|
| aimux-ffi/src/lib.rs | 3,767 | 按 modality/section 拆分（transcription 已独立成 transcription_session.rs，模式可复制） |
| aimux-core/src/recording.rs | 2,246 | 拆 writer/屏障/脱敏三块（4a：writer I/O 错误处理需先修） |
| provider-utils/src/http.rs | 2,005 | 拆请求构建/发送/重试/响应四块 |
| aimux-core/src/replay.rs | 2,002 | 状态良好（0 unwrap），可暂缓 |
| providers/anthropic/convert.rs | 1,750 | 与 openai/convert.rs(1,500)、google/convert.rs(1,099) 一并看 R5 |
| core/openai_output.rs 1,636、open_responses.rs 1,386、huggingface/responses.rs 1,219 | — | 观察名单 |

函数级：400 行转换函数已在上轮整改中拆分（4b 确认）；残留 266 行 `rebuild_stream_result`（4a-M）。cognitive_complexity lint 命中为个位数，复杂度控制良好。
**建议阈值**（写入贡献指南）：文件 ≤1,500 行、函数 ≤150 行，超出需在 PR 说明理由。

### R5 — 转换层公共逻辑下沉 🟡 集中整改机会（本轮最重要架构发现）

- `top_level_media_type` 类工具函数实测散落 **9 处**（上轮 M10 记 5 处，仍在中期增长）
- Chat 族 SSE 解析循环 **4 份**（openai/xai/mistral/cohere，各约 300–420 行）；Gemini 族 2 份且已漂移（vertex 缺 code-execution/grounding 处理）
- 建议：上移 provider-utils 建共享 convert/stream 基建（对应 4b M-12/M-13，预估净删 800–1,200 行）

### R6 — 手写声明与生成物同步 ✅/🟡

- `aimux-ffi.h` vs 94 个导出：**零漂移**（脚本验证）
- `bindings/node/index.d.ts`（手写，56 个 export）vs ts-rs 生成类型（`bindings/node/types/`，测试时生成）：机制上存在漂移风险，建议 CI 增加"跑测试后 diff 生成物"步骤

## 精简度专项发现

1. **未使用依赖 5 处**（cargo-machete + 逐项人工验证）：
   - 移除：`tools/aimux-cli` 的 serde、`aimux-stream` 的 tokio-stream、`aimux-providers` 的 bytes（校验：src 与 tests 均无引用）、`aimux-providers` 的 dirs、`aimux-providers` 的 thiserror（providers 已不定义本地错误类型）
2. **`#[allow(dead_code)]`**（校验修正计数）：item 级 10 处——core/openai_output.rs:471、trace/store.rs:66、providers/{assemblyai.rs:99, aws_polly.rs:477/490, elevenlabs.rs:516, google/model.rs:698, bedrock/event_stream.rs:56, openai/responses/responses_convert.rs:293}；另有模块级 `#![allow(dead_code)]` 9 处（六家 types.rs 等）与 5 个测试文件。注意 trace/store.rs:73 的 `matched_blocks` 被 layer.rs:263 生产代码读取——**属性过期非 dead**，直接删属性即可
3. 复杂度信号健康：clippy cognitive_complexity/too_many_lines 命中个位数

## Top 重构建议（收益/风险排序）

| # | 建议 | 收益 | 风险 |
|---|---|---|---|
| 1 | SSE 解析循环下沉 provider-utils（R5） | 净删 ~1,000 行、修复 vertex 漂移 | 中：需回归各家 cassette |
| 2 | ffi/lib.rs 拆分（R4） | 可维护性、审查粒度 | 低：纯移动 |
| 3 | recording.rs 三拆（R4，先修 4a-H1 静默 I/O） | 取证子系统可维护性 | 中：涉磁盘契约测试 |
| 4 | 依赖清理 5 处 + dead_code 11 处 | 即时、零风险 | 低 |
