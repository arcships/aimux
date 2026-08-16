# Round 4 审计基线快照

> **基线**: master @ `cf2cea5`（feat: transcription streaming — all RFC-0028 phases, 2026-08-14 合入）
> **方法**: git worktree `/tmp/aimux-audit-master` 检出基线，主工作区零改动。计划编写时基线为 3a7255c，执行时 master 已推进至 cf2cea5，原 4e"WIP 首审"相应变更为对该合入 commit 的首审。
> **日期**: 2026-08-14

## 1. 规模仪表盘

| 组 | 文件数 | LOC (src) |
|---|---|---|
| aimux-core | 41 | 16,178 |
| aimux-providers | 146 | 51,356 |
| aimux-stream | 5 | 868 |
| aimux-ffi | 2 | 4,045 |
| aimux-provider-utils | 10 | 3,489 |
| tools/aimux-cli | 6 | 925 |
| tools/aimux-replay | 1 | 166 |
| bindings/node (Rust) | 3 | 2,733 |
| bindings/python (Rust) | 3 | 2,018 |

## 2. 工具链体检结果

| 检查项 | 结果 | 明细 |
|---|---|---|
| `cargo fmt --all -- --check` | ✅ 通过 | — |
| clippy 默认 lint（CI 基线） | ✅ 0 warning | CI rust-test job 强制 `-D warnings`，master 绿 |
| clippy pedantic + nursery | 🟡 4,850 warning / 0 error（校验修正计数） | 详见 [p7-automated-audit.md](p7-automated-audit.md) |
| `cargo doc -D warnings` | 🔴 FAIL：47 error 横跨 4 crate（校验修正；初报 5 为部分计数） | 未转义 `[REDACTED]`、失效 intra-doc 链接、链到私有项 |
| `cargo deny check advisories` | ✅ ok | 无已知漏洞依赖 |
| FFI 头文件漂移 | ✅ 0 缺失 | 94 个 `extern "C" fn` 全部在 aimux-ffi.h；header 多出 3 个 token 为 typedef/回调名，正常 |
| cargo-machete（无用依赖） | ⚠️ 工具链受限 | 最新版需 rustc 1.91 > 基线 1.85，`--locked` 重试中；失败则降级为手工核对 |

## 3. panic 面 / 吞错信号（生产代码，排除 `#[cfg(test)]`）

| 组 | unwrap | expect | panic! | unreachable! | let _ = | .ok() | unwrap_or_default |
|---|---|---|---|---|---|---|---|
| aimux-core | 37 | 5 | 0 | 0 | 9 | 10 | 35 |
| aimux-providers | 20 | 6 | 2 | 1 | 9 | 19 | 84 |
| aimux-stream | 0 | 0 | 0 | 0 | 0 | 1 | 1 |
| aimux-ffi | 0 | 17 | 0 | 0 | 6 | 1 | 1 |
| aimux-provider-utils | 1 | 1 | 0 | 0 | 2 | 6 | 5 |
| bindings/node | 0 | 1 | 0 | 0 | 10 | 1 | 1 |
| bindings/python | 0 | 2 | 0 | 0 | 6 | 1 | 1 |

unwrap 文件集中度：recording.rs 15 / session.rs 10 / trace/store.rs 9 / google/utils.rs 7 / provider.rs 3。

**定性结论**（来自 4a/4b/4d 深审，详见各自报告）：
- core 34 处 unwrap 全部为 `lock().unwrap()`（锁中毒类），无一处可被外部数据触发；replay.rs 生产 0 unwrap。
- providers 20 处 unwrap 全部为不变量守卫，当前安全；但 bedrock 一处 `unreachable!` 是用户可控 enum 兜底。
- ffi 17 处 expect 无一可被绑定侧触发；例外：registry mutex 中毒会级联成永久 DoS、http.rs:177 Client 构建 expect 在受限环境可触发。
- 真正的风险在**静默吞信号**而非 panic：providers `unwrap_or_default` 84 处中约 5 处吞关键信号；core recording.rs 磁盘 I/O 错误全静默（4a-H1）。

**与上轮（2026-08-06）漂移**：上轮生产 unwrap 22 处 → 本轮 58 处（5 个核心 crate 口径 58，含 bindings 为 58+0+0）。除计数口径差异外，主要增量来自 recording/session/trace-store 随 RFC-0023/0024 功能扩张；定性上仍以 lock().unwrap() 为主，风险未升级，但密度趋势值得在 backlog 中跟踪。

## 4. 注释信号

- TODO/FIXME/HACK/XXX：仅 1 处（显著优于常态）
- `#[allow]`：28 处，分布：`dead_code` 11、`clippy::*` 9、`unused*` 7、`non_snake_case` 1。11 处 `allow(dead_code)` 位置见 signals-comments.txt（精简度信号：被压制而非删除的死代码）
- 重复 `//` 注释（≥3 次出现）：54 组，4c 定性约 20 组为分隔线风格、6 组为合理协议差异说明，其余为 copy-paste 传播候选
- 注释中的 RFC 引用死链：0（脚本初报 3339/1123 为 IETF RFC 误报）

## 5. 上轮审计核对（Phase 0.2）

详见 [p0-prior-findings-status.md](p0-prior-findings-status.md)：13 条核对（3H+10M）→ **11 fixed / 1 unfixed / 1 partial**。H1（FFI 回调 catch_unwind）、H2（convert 静默吞错）、H3（google expect 链）全部修复。unfixed = M3（Timeout 不可重试，已文档化为对齐 AI SDK 的有意决策）；partial = M10（转换重复收敛到 convert_common.rs，但 top_level_media_type 实测仍散落 9 处）。

## 6. GitHub Issues

开放 issue 仅 1 个：#95「错误处理 — 参考并对齐 AI SDK 的错误体系(持续跟踪)」。上轮整改均已通过 PR 落地，无滞留 issue。

## 7. 原始数据文件

`signals-panic.txt`、`signals-comments.txt`、`signals-ffi-drift.txt`、`clippy-pedantic.log`、`clippy-lint-frequency.txt`、`cargo-doc.log`、`github-issues.json`、`lcov.info`（覆盖率，Phase 2 产出）。
