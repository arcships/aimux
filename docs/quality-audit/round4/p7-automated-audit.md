# P7 — 自动化体检报告（Round 4 Phase 1）

> **基线**: master @ cf2cea5 · **日期**: 2026-08-14 · 对应审计计划目标 1 / 4 / 5 的机械部分

## 1. clippy pedantic + nursery 全量扫描

`cargo clippy --workspace --all-targets -- -W clippy::pedantic -W clippy::nursery`
结果：**4,850 warning / 0 error**（独立校验修正：初报 4,979 含 129 行 cargo 汇总行）。默认 lint 基线（CI 强制）保持 0 warning。

### 1.1 Top lint 频率（完整分布见 clippy-lint-frequency.txt）

| lint | 数量 | 建议 |
|---|---|---|
| doc_markdown（missing backticks） | 1,229 | 不开启。信噪比低（含大量测试宏内 doc），修复量大收益小 |
| uninlined_format_args | 563 | **建议开启**。机械可修（`clippy --fix`），可读性收益明确 |
| must_use_candidate | 487 | **建议开启**。API 误用防护 |
| redundant_closure_for_method_calls | 355 | **建议开启**。机械可修 |
| return_self_not_must_use | 170 | **建议开启**（与 must_use 同族） |
| unreadable_literal | 292 | 可选。数字字面量加分隔符，风格偏好 |
| missing_const_for_fn (nursery) | 227 | 不开启。const fn 语义变化需逐案评估 |
| missing_errors_doc | 146 | **建议开启**。与注释审计（4c）发现互补——核心 API 有示例但缺 `# Errors` 段（纯手工，无自动修复） |
| cast_possible_truncation | 146 | 不开启。需逐处判断截断是否有害 |
| cognitive_complexity / too_many_lines | 低频（个位数） | 复杂度控制良好，无需 lint 强制 |

**推荐永久子集**（校验修正后）：`uninlined_format_args` + `must_use_candidate` + `return_self_not_must_use` + `redundant_closure_for_method_calls` + `missing_errors_doc` ≈ 1,721 处修复量（日志自带自动修复建议 3,299 处；missing_errors_doc 146 处纯手工）。注：初版子集中的 needless_lifetimes 经校验实为 **0 处**（原 114 为 unnecessary_literal_bound 误计），已移除。落地后 pedantic 噪声降至 doc 类为主。

## 2. rustdoc 基线（`RUSTDOCFLAGS="-D warnings"`）

🔴 **失败：47 error 横跨 4 个 crate**（独立校验修正：初报 5 为首次运行在 aimux-core 即止的部分计数）：

- aimux-core **7**：`generate.rs:952` 失效链接、`recording.rs:176` 未转义 `[REDACTED]`、`recording.rs:541` 失效链接、`replay.rs:47/49` 链到私有项、`util.rs:5/7` 失效链接
- aimux-providers **36**、aimux-provider-utils **2**、aimux-ffi **2**

修复以转义与链接目标修正为主，属机械劳动。建议修完后把 `cargo doc -D warnings` 加入 CI（当前 CI 无 doc 检查）。

## 3. 依赖与安全

- `cargo deny check advisories`：✅ ok，无已知漏洞依赖
- cargo-machete：最新版要求 rustc 1.91（基线工具链 1.85），`--locked` 安装重试中；若失败降级为手工核对（workspace 依赖全部在代码中有引用，未见明显冗余，置信中等）

## 4. FFI 头文件漂移检测

方法：脚本比对 `aimux-ffi/src/*.rs` 的 `pub extern "C" fn` 与 `aimux-ffi.h` 中 `aimux_*` token。
结果：**94 个导出函数全部在头文件中声明，0 缺失**；头文件多出 3 个 token 为 typedef/回调类型名，属正常。RFC-0028 合入时头文件同步良好。

## 5. 绑定语言静态检查（轻量盘点）

- CI 已跑：tsc（node）、pytest（python）、gradle test（java）、go test、flutter analyze（见 ci.yml 各 binding job）
- 缺口：无 eslint/prettier（node）、ruff（python）、staticcheck（go）类独立静态检查 job——绑定层风格问题依赖各语言测试顺带暴露。属低优先级（绑定层审计重心在 API parity，见 4e）

## 6. 错误处理信号汇总（定性结论归口）

机械清单见 baseline.md §3 与 signals-panic.txt。本轮最重要的定性结论：

1. **panic 面总体受控**：无外部输入可触发的 unwrap/expect（4a/4b/4d 三路独立验证）；两个例外见 4d（registry mutex 中毒级联、http.rs:177 Client 构建 expect）。
2. **真正的风险是静默吞信号**：recording.rs 磁盘 I/O 错误静默（4a-H1）、anthropic/google 流解析 5 处 `unwrap_or_default` 吞关键信号（4b）、非 JSON 帧静默丢弃（4e）。
3. providers `panic!` 2 处 + bedrock `unreachable!` 1 处（用户可控 enum 兜底）需逐条复核（见 4b 报告）。

## 7. 与计划的偏差记录

- cargo doc 首次运行参数写法错误（`--` 透传不被 cargo doc 支持），改用 `RUSTDOCFLAGS` 重跑，结果有效。
- llvm-cov 首次运行参数名错误（`--ignore-failures` → `--ignore-run-fail`），已修正重跑。
- machete 受工具链版本限制（见 §3）。
