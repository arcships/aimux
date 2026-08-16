# aimux 架构审计与代码审计计划（Round 4）

> **日期**: 2026-08-14
> **状态**: 计划待执行
> **前置**: 需先 commit 或 stash 工作区中未提交的 RFC-0028 transcription streaming 改动
> **产出位置**: `docs/quality-audit/round4/`

---

## 0. 现状摸底结论（计划依据）

本轮审计的定位是**增量审计**：项目于 2026-08-06 已完成一轮 9-agent 并行质量审计（`docs/quality-audit/` 下 P0–P6 共 14 份报告），本计划在其基础上开拓新战场，避免重复劳动。

### 0.1 代码规模与结构

| 维度 | 现状 |
|---|---|
| Rust 核心 | 8 个 workspace crate：providers 51k / core 16k / ffi 3.7k / provider-utils 3.5k LOC + stream + 2 个 tools CLI |
| 语言绑定 | 7 套：node（napi-rs 原生）/ python（PyO3 原生）/ go / swift / java / kotlin（走 aimux-ffi）/ flutter-dart |
| Provider | 325 个 = 10 个原生协议目录 + 251 条注册表（`provider_registry.json`）+ 其余独立/模态实现 |
| 测试 | providers 121 个集成测试文件、294 个内联测试、32 组 cassette、contract-tests 跨绑定 parity 套件、wiremock E2E |
| CI | fmt --check + clippy -D warnings + workspace test + 8 绑定矩阵 + contract tests；**无 coverage job** |
| 设计文档 | 29 个 RFC（`rfc/`），最新 2026-08-14（RFC-0028） |

### 0.2 上轮（2026-08-06）审计已覆盖、本轮不重复的内容

- FFI soundness 细节（p1b，含 H1 高危：回调 panic 跨 `extern "C"` 边界）
- google/utils.rs unwrap（p1a，含 H3：`set_nested_value` 7 处 expect）
- convert.rs 结构与重复模式（p1c，含 H2：静默吞错返回 `body: Null`）
- 325 provider 抽象一致性（p2a）、async Send/Sync 正确性（p2b）
- thiserror/anyhow 错误分层（p2c）、Rust 2024 安全公告（p3a）、依赖版本（p3b）
- v0.2.1 release head review（p4）、native providers round 2（p5）、concurrency round 3（p6a）、RFC 一致性 round 3（p6b）

### 0.3 本轮审计的空白与增量

| 发现 | 对本轮的影响 |
|---|---|
| **覆盖率工具为零**（无 llvm-cov/nextest/codecov，CI 无 coverage job） | 目标 2（E2E 覆盖率）是最大空白，需先建基础设施再审计 |
| **注释质量从未审过** | 目标 4 为全新战场 |
| **防御性代码冗余角度从未审过**（上轮只审了"错误处理分层"） | 目标 5 增加三层重复校验盘点 |
| release profile 为 `panic="abort"` —— 任何 panic 直接终止宿主进程，FFI 场景致命 | 错误处理审计的优先级放大器 |
| 基线漂移信号：上轮生产 unwrap 22 处 → 现在 core 非测试 unwrap ~26 处（recording.rs 15 / session.rs 10） | 需固定基线 + 复查漂移 |
| 工作区有未提交的 RFC-0028 WIP（6 个文件） | 新代码单独立"首审"通道，不混入全量基线 |
| `aimux-ffi.h` 手工维护，对应 94 个 `extern "C" fn` | 头文件漂移需机器检测 |

---

## 1. 审计范围与基线

- **审计基线**: `master @ 3a7255c`。执行前先 commit 或 stash 当前 RFC-0028 WIP 改动。
- **WIP 增量首审**: 未提交的 transcription streaming diff（约 6 个文件）单独走一遍"新代码首审"清单（见 4e），不混入全量基线。
- **范围**:
  - 8 个 workspace crate（aimux-core / aimux-providers / aimux-stream / aimux-ffi / aimux-provider-utils / tools/aimux-cli / tools/aimux-replay / scripts/fix_tool）
  - `bindings/`（node / python / go / swift / java / kotlin / flutter）
  - `contract-tests/` 与 `tests/ui-parity/`
  - `rfc/` 仅作架构规则来源，不做内容审计（上轮 p6b 已做）
- **前置动作**:
  1. 拉取 GitHub Issues 开放清单，与上轮审计发现交叉核对，标记"已修 / 未修 / 部分修"——避免把已修问题重新报一遍。
  2. 通读上轮 `SUMMARY.md` 与三份高危相关子报告（p1b / p1c / p1a），形成"上轮发现 → 当前代码状态"核对表（重点验证 H1/H2/H3 是否已修复）。

---

## 2. 审计目标

1. **语言规范**：符合 Rust 及各绑定语言的编码特点与规范。
2. **E2E 覆盖率**：端到端测试对各子流程模块的覆盖率可测量、空白可定位。
3. **精简与架构**：代码精简优雅，架构约定被遵守且可机器验证。
4. **注释质量**：注释冗余度低、与代码现状同步。
5. **防御与错误处理**：防御性代码冗余度合理，错误处理路径正确且不吞错。

---

## 3. 阶段计划

### Phase 0 — 基线与工具准备（0.5 天）

| 步骤 | 内容 |
|---|---|
| 0.1 | 固定基线 commit；创建 `docs/quality-audit/round4/` 目录（子报告续用 p 系列编号或按阶段命名） |
| 0.2 | 完成前置动作（Issues 核对 + 上轮报告核对表，见第 1 节） |
| 0.3 | 安装审计工具链（仅本地，暂不动 CI）：`cargo-llvm-cov`、`cargo-nextest`、`cargo-machete`（无用依赖）、`cargo-deny`、`cargo-depgraph`、`cargo-modules`；绑定侧：`ruff`（Python）、`staticcheck`（Go）、`eslint`（TS，若仓库未配） |
| 0.4 | 跑全量基线快照并把数字写进报告头：LOC / 文件数、clippy / fmt 结果、非测试 `unwrap/expect/panic` 计数、`#[allow]` 清单、TODO/FIXME 清单。后续所有对比以此快照为准 |

### Phase 1 — 自动化体检（1 天）

机械检查全量跑，人工只看增量信号；产出物供 Phase 3 / 4 聚焦使用。

1. **语言规范（Rust）**
   - `cargo clippy --workspace --all-targets -- -W clippy::pedantic -W clippy::nursery` 一次性报告 → 人工筛选值得永久开启的子集（候选：`must_use_candidate`、`missing_errors_doc`、`cognitive_complexity`、`implicit_clone`），写入 `[workspace.lints]`
   - `cargo doc --workspace -- -D warnings`（broken intra-doc links）
   - `cargo machete` / `cargo-shear` 检测无用依赖
2. **语言规范（bindings，轻量执行）**
   - `tsc --strict` 现状、ruff、`go vet` + staticcheck、`dart analyze`、ktlint / swift-format 仅盘点不阻塞
   - 绑定层审计重心放在 API parity，风格问题低优先级
3. **FFI 头文件漂移检测**
   - 写脚本比对 `aimux-ffi.h` 声明与 `lib.rs` 的 94 个 `extern "C" fn` 签名（手维护头文件是已知风险；RFC-0028 又新增 `transcription_session.rs`）
4. **注释信号采集（供 4c 使用）**
   - TODO / FIXME / HACK / XXX 清单及所在文件最后修改时间
   - 重复注释检测（同一注释文本出现 N 处）
   - 注释中引用的 `rfc-\d{4}`、文件路径、符号名提取并验证存活性（RFC-0027 存在两个变体文件已是文档腐化信号）
5. **错误处理信号采集**
   - 非测试代码中 `unwrap` / `expect` / `panic!` / `let _ =` / `.ok()` / `unwrap_or_default` 的全量定位清单（含 文件:行号）

### Phase 2 — 测试覆盖率审计（2 天，目标 2，本轮最大增量）

1. **建覆盖率基础设施**
   - `cargo llvm-cov --workspace --lcov` 跑通，产出 per-crate / per-module 覆盖率报告
   - 注意：绑定测试（pytest / ava / go test）不在 llvm-cov 范围内，单独盘点
2. **分层定义 E2E 并逐层测绘现状**（这是"端到端对子流程模块覆盖率"的落地方式）

   | 层 | 定义 | 现状盘点点 |
   |---|---|---|
   | L1 协议 E2E | wiremock 假服务 → 完整请求/流式响应往返 | `aimux-providers/tests/e2e_test.rs`（14 个 fn）覆盖了 10 个原生协议中的几个？关键路径（generate / stream / object / tools）是否齐全 |
   | L2 转换层 | cassette replay 驱动 convert / stream 全分支 | 32 个 provider cassette 子目录对 `convert.rs` / `stream.rs` 的分支覆盖（llvm-cov 验证） |
   | L3 跨绑定 parity | contract-tests TS 套件 | `contract-tests/fixtures/` 的 fixture 面是否过窄，覆盖哪些消息形态 |
   | L4 绑定 E2E | 各语言绑定各自 E2E | node / python / go 已有 e2e；swift / java / kotlin / flutter 现状逐绑定盘点 |

3. **子流程覆盖矩阵**
   - 行 = 用户 API（`generate_text` / `stream_text` / `generateObject` / 各模态 trait）
   - 列 = 子流程模块（convert → http/retry → stream parse → aggregate → error map）
   - 用 llvm-cov 数据填"被哪条 E2E 路径执行过"，空白格即补测 backlog
4. **产出**
   - 覆盖率基线报告 + 明显盲区清单（预期盲区：`recording.rs` / `replay.rs` 等 2k LOC 大模块、tools/ 下两个 CLI、FFI 94 个导出函数的回调路径）
   - 补测优先级（按 ROI 排序）
   - 决策：coverage job 是否进 CI（建议先报告模式，后转门禁）

### Phase 3 — 架构审计（1.5 天，目标 3）

1. **规则固化**：从 `PROJECT-OVERVIEW.md` + RFC 提取成文的架构不变量，候选：
   - 依赖方向：`ffi → core → providers → provider-utils`；core 不得反向依赖；providers 互相不依赖
   - `unsafe` 只允许存在于 aimux-ffi（bindings 原生层除外）
   - 250 个 registry provider 必须纯数据驱动，不得有手写代码残留
2. **机器验证**：
   - cargo-depgraph / 解析各 Cargo.toml 验证依赖方向
   - `grep -rn "unsafe"`（排除 aimux-ffi 与 bindings）验证边界
   - registry 完整性脚本对照 `provider_registry.json` 的 251 条记录
3. **结构异味盘点**：
   - top 大文件逐个评估是否到拆分阈值：ffi/lib.rs 3484 行 / recording.rs 2246 / replay.rs 2002 / openai_output.rs 1636 / moa.rs 775 / session.rs 708
   - 大函数 / 高复杂度 top-20（用 Phase 1 的 cognitive_complexity 数据）
   - 转换层重复模式：上轮 p1c 已发现 openai convert 重复，验证整改情况并推广到其余 9 个原生协议
4. **精简度专项**：
   - 独立 provider 中是否有本可收编进 registry 的
   - `#[allow]` 压制清单逐条辩护（压制的 lint = 被掩盖的异味）
   - dead code、手写 `bindings/node/index.d.ts` 与 ts-rs 生成类型的漂移
5. **产出**：`architecture-rules.md`（长期作为贡献指南）+ 违规清单（0 违规或逐条豁免）+ top-10 重构建议（含收益/风险）

### Phase 4 — 代码深审（2–3 天，按风险面并行）

沿用上轮"并行 agent 各出独立报告"模式，每份子报告独立可读：

| 子任务 | 审查范围 | 审查重点 | 对应目标 |
|---|---|---|---|
| 4a | `aimux-core` 的 recording / replay / session（约 4.5k LOC、3 个文件） | 精简度；unwrap 密度最高（15/10 处）；注释与 RFC-0023 的一致性 | 1/3/4/5 |
| 4b | `aimux-providers` 10 个原生协议抽样（openai + 2 个最复杂协议全读，其余 convert/stream 抽查） | 转换层优雅度；防御性重复；静默吞错 | 1/3/5 |
| 4c | **注释质量专项**（全新） | Phase 1 信号 + 人工抽样：what 注释 vs why 注释比例；过期注释（引用已删除符号 / RFC）；module-level `//!` 与实际行为漂移；错误码文档与 `error.rs` 分类的一致性 | 4 |
| 4d | **防御性冗余专项**（全新角度） | 同一输入在 FFI 边界 / core / provider 三层被重复校验的清单；定义"每项校验只在一层做"的归属规则；多余 clone / 锁拷贝 / 不必要的 `into()` 链 | 5 |
| 4e | FFI + RFC-0028 WIP 首审 | 未提交 diff 走新代码清单：错误路径、回调 panic 边界（对照上轮 H1）、头文件同步、ws.rs 新增代码质量 | 1/5 |

### Phase 5 — 汇总与整改（0.5 天）

- 汇总报告 `docs/quality-audit/round4/SUMMARY.md`：
  - 与 2026-08-06 基线的漂移对比表（unwrap 计数、大文件、依赖等）
  - 按严重度排序的发现（沿用 H/M/L 分级）
- 整改 backlog 写入 `docs/plan/backlog.md` 并建 GitHub Issues（上轮经验：Issues 是追踪源，报告是快照）
- 可选固化（作为本轮审计留下的长期资产）：
  - pedantic lint 子集进 `[workspace.lints]`
  - coverage CI job（报告模式）
  - FFI 头文件漂移检测脚本进 `scripts/`

---

## 4. 五大目标 → 方法与验收映射

| # | 目标 | 主要方法 | 验收标准 |
|---|---|---|---|
| 1 | 语言规范 | pedantic / nursery lint 扫描；machete；`cargo doc -D warnings`；各绑定静态检查；FFI 头漂移脚本 | lint 基线提升建议清单（每条含保留/放弃理由）；头文件 0 漂移 |
| 2 | E2E 覆盖率 | llvm-cov 基线；L1–L4 分层测绘；子流程覆盖矩阵 | 覆盖率数字 per crate；空白格矩阵；补测 backlog 按 ROI 排序 |
| 3 | 精简与架构 | 架构规则文档化 + 机器验证；大文件 / 复杂度 / 重复度盘点 | `architecture-rules.md`；违规 = 0 或逐条豁免；top-10 重构建议 |
| 4 | 注释质量 | 引用存活性脚本；重复注释检测；人工抽样分级 | 过期 / 冗余 / 风格三档清单，过期项全部有修复指向 |
| 5 | 防御与错误 | panic 面清单（`panic=abort` 放大器）；三层重复校验盘点；吞错点扫描 | 非测试 panic 点 = 0 或逐条 justify；每项校验有唯一归属层 |

---

## 5. 风险面优先级（深审资源分配）

- **P0**：`aimux-ffi`（94 导出 + 手维护头文件 + unsafe 密度最高 + panic=abort）；recording / replay / session（体积最大 + unwrap 最密）；RFC-0028 WIP diff
- **P1**：10 个原生协议的 convert / stream 层；覆盖率基础设施（本身是交付物）
- **P2**：250 个 registry provider（纯数据驱动，抽查即可）；tools/ 下 CLI；绑定风格类问题

---

## 6. 产出物清单

| 产出物 | 位置 |
|---|---|
| 基线快照（数字仪表盘） | `docs/quality-audit/round4/baseline.md` |
| 自动化体检报告（lint / 依赖 / 头漂移 / 信号清单） | `docs/quality-audit/round4/p7-automated-baseline.md`（编号可调） |
| 覆盖率报告 + 补测 backlog | `docs/quality-audit/round4/coverage.md` |
| 架构规则 + 违规清单 + 重构建议 | `docs/quality-audit/round4/architecture-rules.md` |
| 深审子报告 ×5（4a–4e） | `docs/quality-audit/round4/` 下独立文件 |
| 汇总报告 | `docs/quality-audit/round4/SUMMARY.md` |
| 整改追踪 | `docs/plan/backlog.md` + GitHub Issues |
| 长期资产（可选） | lint 子集、coverage CI job、头漂移检测脚本 |

## 7. 排期估算

| 阶段 | 估算 | 累计 |
|---|---|---|
| Phase 0 基线与工具 | 0.5 天 | 0.5 天 |
| Phase 1 自动化体检 | 1 天 | 1.5 天 |
| Phase 2 覆盖率审计 | 2 天 | 3.5 天 |
| Phase 3 架构审计 | 1.5 天 | 5 天 |
| Phase 4 代码深审（并行） | 2–3 天 | 7–8 天 |
| Phase 5 汇总与整改 | 0.5 天 | **7.5–8.5 天** |

> 执行说明：Phase 4 的 5 个子任务相互独立，可并行分派（沿用上轮多 agent 模式）；Phase 2 的覆盖率基础设施建议最先做——它既是本轮审计工具，也是审计结束后留在仓库里的长期资产。
