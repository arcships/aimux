# RFC-0025: aimux-cli — 缓存探测 client

> **Status**: DRAFT (pending review)
> **Date**: 2026-08-05
> **Scope**: 本 repo 内新增 `tools/aimux-cli` crate——基于 aimux 构建的独立可运行二进制(client),第一版只做缓存探测业务(审计/诊断/调试指定 provider 的缓存能力),回放/通用调试子命令后续按需加
> **Related**: [RFC-0015](0015-cache-trace-audit.md) 缓存命中探测(本 CLI 消费其数据与查询接口)、[RFC-0023](0023-runtime-request-recording.md) 录制与回放(离线数据源)、[RFC-0024](0024-session-aggregation.md) 会话聚合(session 级诊断)

---

## 1. Motivation

缓存探测拆三层(见 [RFC-0015 §1.2](0015-cache-trace-audit.md)):① 探测本身进 core(常开 infrastructure,采集/判定/存储/查询接口),② 探测业务做独立 client,③ 告警外部消费。

**为什么探测业务要做成独立 client 而非 SDK 集成**:

1. **两套逻辑**:SDK 集成是库路径——开发者在自己的应用里调 aimux,`TraceLayer` 包一层,探测数据进 `RingTraceStore`,然后自己消费。探测业务 client 是独立产物路径——直接 run 一个二进制,探测指定 provider 的缓存能力,不需要宿主应用。
2. **调试工具定位**:开发者想快速回答"这个 provider 的缓存到底靠不靠谱"——不该为此写一个应用,该有一个现成工具。
3. **不污染库二进制**:探测业务逻辑(审计/诊断/展示)进 core 会膨胀库、绑定业务判断;独立 crate 保持 core 纯净。

**为什么 client 放在本 repo 内**(而非独立 repo):aimux-cli 是 aimux 生态的调试产物,与 core 的探测接口强耦合(接口演进同步),放同一 workspace 便于协同开发与版本一致。它是 **aimux 的"产物"**(基于 aimux 构建的 client),不是 aimux 的一部分——`aimux-core` 不依赖它,它依赖 `aimux-core`。

---

## 2. Design Goals

1. **独立可运行**:`cargo run -p aimux-cli`(或安装后 `aimux`)即可用,不要求宿主应用集成。
2. **薄 client**:只做探测业务的消费逻辑,探测算法/判定/存储全在 core(RFC-0015)。
3. **两种数据源**:进程内查询(core 的 `RingTraceStore` 查询接口)+ 离线 jsonl(core `export_jsonl` 导出的 TraceRecord,或 RFC-0023 录制文件)。
4. **面向调试**:输出人可读报告(审计 summary / 链级命中演变 / prompt 结构诊断),不是机器 API。
5. **渐进式**:第一版只做缓存探测子命令,回放/通用调试后续按需加。

---

## 3. CLI 形态

### 3.1 位置与命名

```
tools/aimux-cli/
├── Cargo.toml        # package name = "aimux-cli",[[bin]] name = "aimux"
├── src/
│   ├── main.rs       # 子命令分发(clap)
│   ├── probe/        # 缓存探测子命令
│   │   ├── mod.rs
│   │   ├── online.rs   # 进程内数据源(连 core RingTraceStore 查询)
│   │   └── offline.rs  # 离线 jsonl 数据源(读 export_jsonl 输出)
│   └── report.rs     # 人可读报告渲染
```

加入 workspace members(`Cargo.toml` `members` 数组),`[workspace.dependencies]` 复用。

### 3.2 子命令(第一版:探测)

```text
aimux cache-probe <SUBCOMMAND>

子命令:
  online    进程内查询:连上宿主应用导出的探测数据(经 core 查询接口)
            --provider <name> --model <id> --session <session_id> --since <ms>
  offline   离线分析:读 core export_jsonl 导出的 TraceRecord jsonl
            --file <trace.jsonl> --provider <name> [--session <session_id>]
  session   会话级诊断:读离线 jsonl,输出指定 session 的链级命中演变
            --file <trace.jsonl> --session <session_id>
  provider  探测指定 provider 的缓存能力:直接调 aimux provider 发测试请求,
            跑探测算法,输出该 provider 的缓存能力报告
            --provider <name> --model <id> --api-key <env:VAR> [--base-url <url>]
```

**第一版范围**:`offline` + `session` + `provider`(调试指定 provider 缓存的核心诉求)。`online`(连活进程)后续——aimux 是库不是服务,没有常驻进程给 CLI 连,online 形态需再设计(可能经共享内存/Unix socket 或只留 jsonl 路径)。

### 3.3 数据源(两种)

| 数据源 | 内容 | 来源 | 用途 |
|---|---|---|---|
| **TraceRecord jsonl(离线)** | 探测哈希数据(指纹/usage/verdict) | core `RingTraceStore::export_jsonl` 或外部 `TraceSink` 落盘 | `offline`/`session` 审计诊断 |
| **录制 jsonl(RFC-0023)** | 明文完整上下文(输入/配置/HTTP) | core `JsonlRecorder` | 深查可疑 verdict 时拉明文跑完整 LCP;缓存可复现性验证(多次请求回放 + 探测对比) |

两者以 `trace_id` 关联(RFC-0015 §8 协同)。CLI 的 `provider` 子命令是**在线探测**(直接调 provider 发请求),不依赖数据源。

---

## 4. 与 core 探测接口的关系

CLI **只消费** core 暴露的接口(RFC-0015 §5.3),不实现探测算法:

| core 暴露(已设计) | CLI 消费方式 |
|---|---|
| `TraceRecord` 结构(serde,jsonl 可读) | `offline` 读文件反序列化 |
| `TraceStats` / `TraceFilter` 聚合 | `offline --provider X` 输出统计 |
| `SessionChainView` / `PrefixBreak` | `session --session X` 输出链级诊断 |
| `CacheAuditor` / 判定规则(内置) | `provider` 子命令:发测试请求 → 挂 `TraceLayer` + 内置 auditor → 读 verdict |
| `LanguageModel` / `provider()`(aimux-providers) | `provider` 子命令构造指定 provider 发请求 |

**不消费**(core 不暴露的,CLI 也不做):告警阈值决策(③外部)、调优建议生成、预判决策。

---

## 5. 与现有 RFC 的关系

| RFC | 关系 |
|-----|------|
| [RFC-0015](0015-cache-trace-audit.md) | **主依赖**。本 CLI 是 ① 探测本身的 ② 业务 client,消费其查询接口与 jsonl。 |
| [RFC-0023](0023-runtime-request-recording.md) | **协同**。离线深查用录制明文;缓存可复现性验证用请求回放 + 探测组合。 |
| [RFC-0024](0024-session-aggregation.md) | **协同**。session 级诊断消费 session_id 归组(离线 TraceRecord 自带 session_id)。 |
| [RFC-0021](0021-composite-model-routing.md) | **远期闭环**。探测 verdict → 外部告警 → RouterModel 切换回放策略。CLI 只出数据,不做决策。 |

---

## 6. Non-Goals(第一版)

1. **不做 `online` 连活进程**(aimux 是库,无常驻进程;形态待定,可能仅留 jsonl 路径)。
2. **不做回放子命令**(`replay` 属于 RFC-0023 的消费,第一版只做探测;后续按需加)。
3. **不做通用调试子命令**(单次 generate_text 调用调试)。
4. **不做告警/报表**(③外部消费;CLI 只输出人可读报告,不做持续监控)。
5. **不做调优建议**(自动改 prompt 提升命中是应用层逻辑)。
6. **不实现探测算法**(判定/LCP/存储全在 core,CLI 只消费)。

---

## 7. Scope of Changes

| 位置 | 改动 | 工作量 |
|------|------|--------|
| `Cargo.toml` | workspace members 加 `"tools/aimux-cli"` | 1 行 |
| `tools/aimux-cli/Cargo.toml` | package + bin + deps(clap/tokio/aimux-core/aimux-providers/aimux-provider-utils/serde_json) | ~30 行 |
| `tools/aimux-cli/src/main.rs` | clap 子命令分发 | ~60 行 |
| `tools/aimux-cli/src/probe/offline.rs` | 读 TraceRecord jsonl → 审计统计 + 报告 | ~200 行 |
| `tools/aimux-cli/src/probe/session.rs` | 读 jsonl → session 链级诊断 + 报告 | ~150 行 |
| `tools/aimux-cli/src/probe/provider.rs` | 构造 provider → 发测试请求 → 挂 TraceLayer + auditor → 报告 | ~200 行 |
| `tools/aimux-cli/src/report.rs` | 人可读报告渲染(表格式) | ~100 行 |
| 测试 | 离线解析 + 报告 + provider 探测(用 cassette 或 mock) | ~200 行 |

**合计:~700-900 行(第一版)。依赖 RFC-0015 的探测接口落地(core 先做,P0 改动)。**

---

## 8. Risks

| 风险 | 等级 | 对策 |
|------|------|------|
| **依赖 RFC-0015 接口未落地** | 高 | CLI 第一版与 RFC-0015 的 core 实现协同开发;先做离线(jsonl)路径(只依赖 TraceRecord serde,依赖面最小) |
| **`provider` 子命令消耗真实 API 费用** | 中 | 默认用最短测试 prompt(单轮,~100 token);`--max-requests N` 限制;文档警示 |
| **`online` 形态不明**(库无常驻进程) | 中 | 第一版不做 online;只做 offline(jsonl)+ provider(直接调)。online 待设计(共享内存/Unix socket/仅 jsonl) |
| **CLI 与 core 版本漂移** | 低 | 同 workspace,版本一致;TraceRecord serde 是契约,改格式需同步 |

---

## 9. Open Questions

1. **二进制命名**:`aimux` 还是 `aimux-cli`?建议 `aimux`(命令短,与库 crate 名区分),但可能与将来别的 aimux 命令冲突——待定。
2. **`provider` 子命令的测试 prompt 设计**:测缓存能力需要"两次相同前缀请求"验证命中(第一次写,第二次读)。用固定模板 prompt?还是可配?建议固定模板 + `--prompt` 可覆盖。
3. **`online` 形态**:aimux 是库不是服务,CLI 怎么连进程内 RingTraceStore?候选:Unix socket / 共享内存 / 只留 jsonl(应用自己 export)。建议第一版只留 jsonl,online 取消或远期。
4. **报告格式**:文本表格 vs JSON 输出?建议 `--format text|json`(text 给人,json 给脚本)。

---

## 10. Implementation Order

| 阶段 | 内容 | 依赖 | 状态 |
|------|------|------|------|
| **P1** | `offline` 子命令:读 TraceRecord jsonl → 审计统计 + 报告 | RFC-0015 P0(core TraceRecord serde + export_jsonl) | 待实施 |
| **P2** | `session` 子命令:会话级诊断 + 报告 | RFC-0015 P1(session_chain),RFC-0024(session_id) | 待实施 |
| **P3** | `provider` 子命令:直接调 provider 探测缓存能力 | RFC-0015 P1(TraceLayer + auditor 落地) | 待实施 |
| **P4**(后续) | `replay` 子命令(RFC-0023 消费)+ 通用调试 | RFC-0023 落地 | 不做(第一版) |
| **P5**(待定) | `online` 形态(Unix socket / 共享内存 / 仅 jsonl) | 设计先行 | 待定 |

**建议顺序**:P1(离线审计,依赖面最小)→ P3(在线探测 provider)→ P2(会话诊断)。第一版交付 = P1 + P2 + P3,即 `offline`/`session`/`provider` 三个子命令。
