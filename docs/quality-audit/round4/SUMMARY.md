# aimux 质量审计总报告 — Round 4

> **快照声明**: 本报告是 **2026-08-14** 的时间点快照，基线 master @ `cf2cea5`（含 RFC-0028 transcription streaming 合入）。整改追踪以 backlog/GitHub Issues 为准，本报告作为诊断参考归档。
> **方法**: 按 [audit-round4-plan.md](../../docs/plan/analysis/audit-round4-plan.md) 执行 6 阶段：自动化体检（fmt/clippy pedantic+nursery/rustdoc/deny/machete/FFI 头漂移/信号脚本）→ 覆盖率基础设施（首次引入 cargo-llvm-cov）→ 架构机械验证 → 5 路并行 agent 深审（4a–4e）→ 汇总。基线通过 git worktree 固定，主工作区零改动。
> **上轮**: 2026-08-06 Round 1–3（P0–P6）。本轮为增量审计，聚焦五大目标：语言规范 / E2E 覆盖率 / 精简与架构 / 注释质量 / 防御冗余。

---

## 1. 体检硬数据

| 检查项 | 结果 |
|---|---|
| `cargo fmt --check` | ✅ 通过 |
| clippy 默认 lint（CI 基线） | ✅ 0 warning |
| clippy pedantic + nursery | 🟡 4,850 warning / 0 error（校验修正：初报 4,979 含 cargo 汇总行；推荐子集见 p7） |
| `cargo doc -D warnings` | 🔴 47 error 横跨 4 crate（校验修正：初报 5 为 aimux-core 即止的部分计数；CI 未覆盖此项） |
| `cargo deny advisories` | ✅ ok |
| cargo-machete | 🟡 5 处未使用/错置依赖 |
| FFI 头文件漂移（94 导出） | ✅ 0 缺失 |
| 覆盖率（首次测量） | 78.5%（36,847 instrumented 行；ffi 44.8% / providers 77.8% / core 89.7%） |
| 生产 unwrap/expect（排除 test） | 58 unwrap + 32 expect；三路独立定性：**无外部输入可触发者**，全部为 lock().unwrap()/不变量守卫 |
| TODO/FIXME | 1 处 |
| `#[allow]` | 28 处（dead_code 11） |

## 2. 上轮整改核对（Phase 0.2）

13 条核对（3H+10M）：**11 fixed / 1 unfixed / 1 partial**。
- ✅ H1（FFI 回调 catch_unwind，含 5 个 panic 测试）、H2（convert 静默吞错改 fail-fast）、H3（google Result 化 + 上限）全部修复
- ⚪ M3（Timeout 不可重试）unfixed：已文档化为对齐 AI SDK 的有意决策
- 🔶 M10 partial：模型能力重复已收敛 convert_common.rs，但 `top_level_media_type` 实测散落 **9 处**（上轮记 5 处）

## 3. 本轮发现（按严重度，详情见各子报告）

### 🔴 高风险（8 H）

| # | 发现 | 来源 | 位置 |
|---|---|---|---|
| H1 | recording writer 磁盘 I/O 错误全静默（ENOSPC 时 try_flush 仍 Ok，违反落盘契约） | 4a | core/recording.rs:829-834 |
| H2 | completion barrier 缺 input 条件 + drop-newest，可写出 complete=true 但 input 空的失真取证记录（RFC-0023 偏差） | 4a | core/recording.rs:94-98 |
| H3 | google 用 tool_call_id 冒充 functionResponse.name（忽略 tool_name），非流式多轮工具调用几乎必然 400（校验：流式 id 配对可能幸免） | 4b | providers/google/convert.rs:233-234 |
| H4 | anthropic 流错误提前 return 不发 Finish part，违反 "Finish=Final chunk" 契约（校验：后果为 finish_reason/usage 失真而非流悬挂，已降级 medium） | 4b | providers/anthropic/stream.rs:481-503 |
| H5 | JSON 线格式解析器在 FFI/Python/Node 三份逐字等价，"空/null=默认"约定全仓 20 份拷贝（4d 共 11 组重复实例之一） | 4d | aimux-ffi + bindings |
| H6 | http.rs:177 Client 构建 expect 受限环境可杀宿主（panic=abort）；registry mutex "中毒 DoS" 经校验不成立（锁窗口无 panic 源），降为统一性改进 | 4d | provider-utils/http.rs、ffi/lib.rs |
| H7 | bedrock 注释称 ReasoningDelta 无 provider_metadata 故丢弃 signature——与代码矛盾（该字段存在） | 4c | providers/bedrock/model.rs:388 |
| H8 | Kotlin nextPart 超时哨兵异常死代码（throwFromC 返回 Nothing），retryable 哨兵 API 契约破坏 | 4e | bindings/kotlin Multimodal.kt:303 |

### 🟡 中风险（44 M，各报告全文）

代表性条目：
- **4a（9M）**：JsonlRecorder::new 降级致 AIMUX_RECORD=1 静默不录；流式 tool_call 缺 index 静默并入 0 号；trace/store.rs aggregate O(n²)；注释漂移 3 处（replay.rs:130/172 与 recording 现行脱敏行为相反）
- **4b（15M）**：top_level_media_type 9 处 + Chat 族 SSE 循环 4 份（Gemini 族 2 份已漂移）；unwrap_or_default 5 处吞关键信号（anthropic base64/usage）；漏映射 stop_sequence；bedrock 用户可控 enum 的 unreachable 兜底
- **4c（8M）**：error.rs 三变体文档语义零构造点；generate.rs //! 漏 RFC-0023/0024；17 份 thin-wrapper 模块文档逐字重复
- **4d（5M）**：validate_base_url 零调用者（死防御）而活路径 base_url 无校验、空 prompt 直通远端 400
- **4e（7M）**：FFI drop 类型盲（误传句柄静默销毁）；user-abort 链接任务泄漏；非 JSON 帧静默丢弃；rate_limits 未按 RFC 映射；Go 缺 timeout；Python 缺 abort；新代码错误路径零测试
- **Phase 1–3**：rustdoc 47 error（4 crate）；5 处依赖错置；RFC-0028 新模块覆盖 61–64%；L1 统一 e2e 仅 2/11 协议；4 个测试二进制插桩下失败（校验修正：真因是 SigV4 签名 host 头缺端口 + 环境代理，非时间敏感，见 #125）

### ⚪ 低风险（36 L）+ 机会项

依赖清理 5 处、dead_code 11 处、15 个零覆盖本地 provider（registry 化候选）、swift 无 e2e、contract-tests 仅 1 fixture。

## 4. 与上轮（2026-08-06）漂移对比

| 维度 | 上轮 | 本轮 | 判读 |
|---|---|---|---|
| 生产 unwrap | 22 | 58 | 口径差异 + RFC-0023/0024 功能扩张；定性未恶化（全 lock 类），密度趋势需跟踪 |
| 高危整改 | 3H | 0 旧账 + 8 新 H | 旧账清偿良好；新 H 集中在取证子系统与协议边角 |
| 转换重复 | M10 记 5 处 | 9 处 | 🔶 恶化，需下轮收敛（SSE 下沉） |
| 测试 | 74k 行无量化 | 78.5% 基线建立 | 首次可量化，RFC-0028 是洼地 |

## 5. 五大目标验收（对照计划 §4）

| 目标 | 验收 | 结论 |
|---|---|---|
| 1 语言规范 | lint 提升建议清单 + 头文件 0 漂移 | ✅ p7 §1.1 给出 5 lint 子集（≈1,700 处机械修复）；rustdoc 5 error 待修后可入 CI |
| 2 E2E 覆盖率 | per-crate 数字 + 空白矩阵 + ROI backlog | ✅ coverage.md：78.5% 基线、L1–L4 现状、8 条 ROI 排序 |
| 3 精简与架构 | 规则文档 + 违规清单 + top 重构 | ✅ architecture-rules.md：R1–R6 全过（无硬违规），软超标=大文件+SSE 重复 |
| 4 注释质量 | 三档清单 | ✅ p4c：抽样 60 条 why 55%/what 45%/noise 0，45 引用仅 2 失准；H1/M8/L6 |
| 5 防御与错误 | panic 点定性 + 归属规则 | ✅ 无外部可触发 panic；4d 校验归属规则表（FFI 管指针、wire 层管格式、core 管语义、utils 管传输） |

## 6. 整改优先级（已写入 docs/plan/backlog.md）

1. **P0 正确性**：H1–H8（recording 落盘契约、google functionResponse.name、anthropic Finish 契约、Kotlin 哨兵、FFI drop 类型盲等）
2. **P1 防退化资产**：coverage CI job、rustdoc 修复+入 CI、lint 子集、SigV4 稳定性
3. **P1 补测**：RFC-0028 错误路径（ws/transcription）、L1 扩协议、FFI 冒烟遍历
4. **P2 结构**：SSE 下沉 provider-utils（净删 ~1,000 行 + 修 vertex 漂移）、ffi/lib.rs 拆分、依赖/dead_code 清理、15 个本地 provider registry 化

## 7. 产出物索引

| 文件 | 内容 |
|---|---|
| [baseline.md](baseline.md) | 基线仪表盘 + panic/注释信号 + 上轮核对结论 |
| [p0-prior-findings-status.md](p0-prior-findings-status.md) | 上轮 13 条发现逐条核对 |
| [p7-automated-audit.md](p7-automated-audit.md) | clippy/rustdoc/deny/machete/FFI 漂移 |
| [coverage.md](coverage.md) | 覆盖率基线 + 子流程矩阵 + L1–L4 + ROI |
| [architecture-rules.md](architecture-rules.md) | 架构不变量 R1–R6 + 精简度 + 重构建议 |
| [p4a-core-recording-replay-session.md](p4a-core-recording-replay-session.md) | core 大模块深审（H2/M9/L3） |
| [p4b-native-protocols.md](p4b-native-protocols.md) | 原生协议转换层（H2/M15/L14） |
| [p4c-comments.md](p4c-comments.md) | 注释质量专项（H1/M8/L6） |
| [p4d-defensive-redundancy.md](p4d-defensive-redundancy.md) | 防御冗余专项（H2/M5/L4） |
| [p4e-rfc0028-first-pass.md](p4e-rfc0028-first-pass.md) | RFC-0028 合入首审（H1/M7/L9） |
| signals-*.txt / *.log / lcov.info | 原始数据 |

**执行偏差记录**：基线 worktree 位于 /tmp/aimux-audit-master（用后可删）；cargo doc 参数与 llvm-cov 参数各修正一次重跑；machete 需 --locked 安装（rustc 1.85 限制）；4 个测试二进制在插桩下失败已定位（SigV4 host 端口 + 环境代理，计入发现，未阻塞报告生成）。

## 8. 独立校验与 Issue 化（2026-08-14）

全部 16 个 backlog 项已建为 GitHub Issues **#110–#125**，并逐项派独立 agent 复核（与原发现不同 agent，重新对代码取证）。

| 判定 | 数量 | Issues |
|---|---|---|
| ✅ 完全确认 | 4 | #110（recording H1/H2）、#114（wire 解析器三份）、#122（大文件）、#124（本地 provider 零覆盖） |
| 🔶 部分确认（数字/措辞/范围修正） | 11 | #111、#112、#113、#115、#116、#117、#118、#119、#120、#121、#123 |
| 机理推翻 | 1 | #125（非时间敏感；真因 sigv4.rs host 头缺端口 + 环境代理劫持回环请求——暴露真实签名缺陷） |

**校验带来的实质性修正**（issue 正文已全部更新）：
- #117 rustdoc：5 → **47 error**（4 crate）
- #112 降级 medium（后果为 finish_reason/usage 失真，非流悬挂）；#121 升级 medium（vertex 漂移确认为功能缺失：工具可启用但流式结果静默丢弃）
- #113 范围扩大：anthropic 流式同样丢 reasoning signature（非 bedrock 独有）
- #115 "registry 锁中毒级联 DoS" 不成立（锁窗口无 panic 源、panic=abort 下不可中毒）
- #119 swift "无 e2e" 不成立（有功能等价的未命名 e2e）；真实缺口 = c 绑定零测试 + 统一套件化
- #118 零覆盖清单修正（首块超时/会话中 abort 已有测试）
- #120 数字修正（总量 4,850；needless_lifetimes 实为 0 处移出子集）；#123 bytes 应整体移除、store.rs:73 属过期属性非 dead
