# Roadmap：0.5.0 → 1.0

> 基线：v0.5.0（2026-09-27 发布）。方向依据 [RFC-0036](rfc/0036-positioning-and-layered-architecture.md)
> （定位与分层架构）；本文档只做排期，单项设计与差距分析在各 issue / RFC 内。
> 上次复核：2026-10-03；代码基线：`72a37b5`（v0.5.0 发布后的 master）。阶段划分沿用已合并的 #197 / #198；完成状态以合并记录和源码为准，未合并提案单独列出。
>
> 位置说明：放在仓库根目录而非 `docs/plan/`——后者计划在 E1 文档重组时整体归档，目前仍保留；`docs/` 的英文约定不适用于本文。

## 当前状态（2026-10-03）

- **已合并的路线基线**：[#198](https://github.com/arcships/aimux/pull/198)（`cddfee7`）加入定位与分层架构 RFC，[#197](https://github.com/arcships/aimux/pull/197)（`9ce501a`）加入本文的 0.6 → 1.0 分期。RFC 文件仍标 Draft，设计状态需与决策记录同步。
- **已完成的前置工作**：#164 请求管线、[#169](https://github.com/arcships/aimux/pull/169) A1 脚本与依赖清理、[#196](https://github.com/arcships/aimux/pull/196) iOS staticlib 体积修复已合并。A1 以 #169 的实际保留/删除清单为准；#196 不代表桌面 S1 或全产物 S2 已完成。
- **仍待实施**：RFC-0032 / 0033 入库、B1 / C2、桌面 S1、全产物 S2、性能门禁 P、ops 协议与 L0 passthrough。RFC-0031 当前位于 [docs/ai-sdk-request-pipeline.md](docs/ai-sdk-request-pipeline.md)，迁入 `rfc/` 属文档整理，不是重新实现请求管线。
- **未合并提案**：[#200](https://github.com/arcships/aimux/pull/200) 提出全链路 AI SDK 对齐，新增文件与现有 RFC-0036 同号。它在切换方式、兼容策略、auth/replay 结构及 L2 定位上与当前路线有差异，需明确编号、与现行基线的继承或替代关系，并核对已有决策记录后同步本文及相关 issue；本轮状态整理不裁定这些架构选择。

阶段中的版本和工期为规划估算，不是已承诺发布日期。完成项附合并 PR；未合并提案不计入完成进度。

## 0. 方向（RFC-0036 摘要）

aimux 是 **provider 接入与治理的运行时**：

1. 性能与体积是第一基准线——每个产物有预算，回归即 CI 失败。
2. 多语言绑定的目的是多语言栈的**行为统一**。
3. 治理（端侧测试、漂移检测、能力验证）是一等能力。
4. 协议持续演进、私有 API 持续存在——数据真相下沉到 L0 传输 / L1 协议，L2（AI SDK 形态）是版本化投影，不随 1.0 冻结。L2 的选型已定为 AI SDK V4 形态，不再另立调研 RFC（调整：见 [docs/aisdk-architecture-alignment.md §0.7](docs/aisdk-architecture-alignment.md#07-与-roadmap--rfc-0036-既有承诺的关系)）。
5. 与 harness 解耦——同一套 **ops 协议**两个入口：FFI（8 种绑定）与 stdio CLI。
6. 不做 agent / 编排 / 多租户网关业务。

## 1. 依赖全景

```
#166 代码缩减总纲（25 PR，净 ≈ −44k 行）——保留，按 RFC-0036 §8 重新归类
├─ A 轨 机械清理            A1 ✅(#169)；A2–A5 互相独立
├─ B 轨 protocol registry = L1 协议层（RFC-0032，需先入库）
│   ├─→ #174 Auth L1 ──→ #175 Auth L2          ← 依赖 B1
│   └─→ #167 transport-level replay ──→ #179 replay 子命令   ← 依赖 B1
├─ C 轨 FFI（现 123 个 extern "C"，#166 基线 109）→ ops 协议（C1 升级为传输无关协议：op 表 / 二进制帧 / 版本协商）
│   └─ C2 构造器转发 shim 是 B4/B5 前置
├─ D 轨 8 种绑定重写为 ops 薄封装（依赖 C1）+ D8 类型镜像生成
├─ E 轨 内部清理（#164 已合入，改为 master 上独立 PR）+ 文档三层重组
│
├── 新增（RFC-0036）：L0 native passthrough | ops 协议 schema | stdio CLI | 性能门禁 P
├── 独立项：#185 ToolInput 类型化 + tracker 对齐上游并移入 provider-utils（#204）；调整：见 [docs/aisdk-architecture-alignment.md §0.7](docs/aisdk-architecture-alignment.md#07-与-roadmap--rfc-0036-既有承诺的关系)
├── 独立项：#170 registry 维护自动化 → #171 triage（前半：27 个 base_url 分歧）
└── 小项（随时插空）：#181 session 后续 | #180 cache probe §10

常设跟踪：#95 错误体系（新错误相关 PR 挂此 issue）
```

## 2. 阶段划分

### 0.6.0 ——「门禁 + 协议地基 + 纯减法」（~4-6 周）

| 项 | 内容 |
|---|---|
| RFC | RFC-0031 / 0032（写入 #166 的 7 条修正）/ 0033 入库——B1、#170、#174、#175 均引用它们 |
| **S1** | 桌面 `.a` 切 LTO-off staticlib profile（`ios-release` 改名 `staticlib-release`）。**只改 staticlib job**：`aimux-ffi` 同时产出 cdylib，`.so/.dylib/.dll` 保持 fat-LTO。实测 138.4 → 66.5 MB |
| **S2** | 体积门禁推广到所有发布产物，**每个产物给具体阈值**（初期取当前值 +10%），CI 输出体积报告 |
| **P** | 性能回归门禁：单请求开销、流式吞吐、RSS 增长 |
| A2–A5 | 删 `ProviderName` 枚举及各语言副本；归档 `provider-inventory/`；aimux-web 类型副本；Flutter example 桌面脚手架 |
| E1 | 内部清理 + 文档三层重组 |
| #185 | `ToolInput{Raw,Parsed}`；tracker 不删，改为对齐 `@ai-sdk/provider-utils` 并移入 aimux-provider-utils（#204）；调整：见 [docs/aisdk-architecture-alignment.md §0.7](docs/aisdk-architecture-alignment.md#07-与-roadmap--rfc-0036-既有承诺的关系) |
| #170 / #171 前半 | sync/probe 脚本 + scheduled Action；27 个 base_url 分歧 triage |
| **B1 + C2** | 解锁项：protocol 列 + `from_resolved`；40 个 FFI 构造器转发 shim（B1 形式由 descriptor 数据源 + `create_xxx` 工厂替代，C2 shim 已取消；调整：见 [docs/aisdk-architecture-alignment.md §0.7](docs/aisdk-architecture-alignment.md#07-与-roadmap--rfc-0036-既有承诺的关系)） |
| **ops 协议 schema** | op 表、消息与错误信封、二进制帧、版本协商——先以文档 + 测试落地（RFC-0036 §4） |
| **L0 passthrough** | 原样调用任意 provider 端点，享受 auth / 重试 / 录制 |

版本语义：
- **Rust 层 breaking**：A2 删枚举、#185 换类型。
- **绑定源码级 breaking**：A2 删除 Go / Java / Kotlin / Swift / Flutter / Node / Python 的 `ProviderName` 类型化常量，改为字符串（Node 由 `gen_ts_types.py` 生成 string-literal union 保留补全）；CHANGELOG 需给迁移说明。
- binding wire 与 C ABI 不变。

### 0.7.0 ——「L1 协议层 + auth」（~5-6 周）

| 项 | 内容 |
|---|---|
| B2–B8 | 17 个 LanguageModel 实现 → ~7 个协议；33 个 wrapper 退役；responses 家族合并（遵循 #166 修正 3/6） |
| #174 / #175 | registry `auth` schema + `apply_auth()`；Credential 解析序 + CredentialStore + TokenRefresher |
| L2 补齐 | `StreamPart::Raw` / `provider_metadata` 在各协议接线——投影不下的不得静默丢弃 |
| **S3** | `cargo-bloat` 审计 0.5.0 增重（Android `.so` 13.4 → 21 MB）；feature gating 仅作可选小体积路径，默认全量 |
| ~~调研 RFC~~ | ~~L2 数据模型选型（AI SDK 形态 / Open Responses items / 自有），指标见 RFC-0036 §3~~ 撤销：选型已定为 AI SDK V4 形态（调整：见 [docs/aisdk-architecture-alignment.md §0.7](docs/aisdk-architecture-alignment.md#07-与-roadmap--rfc-0036-既有承诺的关系)） |

版本语义：Rust API 大 breaking（33 wrapper 退役）；C ABI 经 C2 转发不受影响。

### 0.8.0 ——「ops 协议落地」（~4-5 周）

| 项 | 内容 |
|---|---|
| C1 | 9 个导出 + `dispatch`，**同一 dispatch 同时服务 FFI 与 stdio CLI**；旧导出保留共存 |
| C3 | `aimux_error_*` 访问器 → 错误 JSON 信封（含派生 `retry_after_ms`） |
| D1–D7 | 8 种绑定迁到 ops 薄封装。D1 Kotlin（只依赖 Java artifact）与 D5 Flutter ffigen 过渡版不依赖 C1，可在 0.6/0.7 提前做 |
| D8 | 类型镜像生成：serde → JSON Schema → 各语言，`--check` 门禁扩到全部输出 |
| P | stdio 入口往返开销纳入性能门禁 |

版本语义：新 ABI 加入，旧导出并存（#166 要求共存至少一个 minor 版本）；绑定按各自节奏迁移。“共存至少一个 minor”不再作为承诺：全链路对齐切换本身没有共存期，之后 ops ABI 引入时的旧符号也按切换门一次替换；C2 转发 shim 与 C4 旧导出清理随之取消（调整：见 [docs/aisdk-architecture-alignment.md §0.7](docs/aisdk-architecture-alignment.md#07-与-roadmap--rfc-0036-既有承诺的关系)）。

### 0.9.0 ——「治理」（~3-4 周）

| 项 | 内容 |
|---|---|
| #167 | transport-level replay：mock 挂 HTTP 层跑真协议代码，覆盖全协议 / 全模态；`ProviderRecord` = registry 行 + protocol |
| 漂移检测 | #170 扩展：定期重录，与 cassette 字节级 diff |
| 能力矩阵 | provider × 能力，每格由探测或录制支撑 |
| CLI | `aimux probe / replay / diff`（#179 replay 子命令并入）；#181 debug CLI 并入 |

### 1.0 ——「冻结」（~1-2 周）

| 项 | 内容 |
|---|---|
| C4 | 8 种绑定全部迁移后删除旧导出、旧头文件、旧 FFI 测试 |
| 冻结范围 | ops 协议与 L0 / L1 契约。**L2 独立版本化，不随 1.0 冻结** |
| 发布判据 | #166 ledger 全勾（B9 / #171 后半除外）；错误模型 #95 与请求管线 #164 稳定一个周期；各门禁（S2 / P）连续绿 |

## 3. 贯穿原则（沿用 #166 ground rules）

- **一个方向一个 PR**；providers 先于 bindings；最高确定性的删除先落。
- **Cassettes 是字节级回归门禁**：2,799 份录音只增不减。
- **文档重组而非删除**：`git mv` + 链接修复；`archive/` 只留 README 指向 git 历史。
- **Scheduled registry 报告永不阻塞合并**（#170 验收条件）。
- **瘦身不改变对外 API**（§6.1）。
- 语言规则：`docs/` 与 `bindings/*/README` 英文，`rfc/` 中文。

## 4. 近期入口与依赖门

1. **状态与方向对齐**：#166 A1 按 #169 的实际交付补勾，E1 改为 master 上独立 PR；#200 先厘清 RFC 同号、已有决策记录及与现行路线的关系。不在本轮文档维护中决定 ABI 切换方式或 #185 的最终类型架构。
2. **RFC-0032 / 0033 入库**：B1 与 #174 的规范前提。纳入 #166 已记录的修正；不能以未合并分支中的文档替代 master 的规范。
3. **S1 / S2 / P**：S1 只改 staticlib job，cdylib 保持现有 profile；S2 给出各发布产物的基线与阈值；P 明确测量方法、回归阈值与基线更新规则，再作为发布门禁。
4. **A2–A5 与 #185**：按现有独立 PR 边界推进；涉及公开类型、绑定生成或 provider/core 边界的部分，先核对 #200 的决策影响，避免重复实施。A2 的源码迁移说明保留。
5. **B1 + C2**：后续 provider 删除的解锁项；B4/B5 还需完成直接构造调用点迁移。#174 依赖 B1，#175 依赖 #174。
6. **ops schema / L0 passthrough**：分别明确跟踪项和验收条件。ops 至少覆盖 op×handle、错误信封、二进制帧、版本协商与取消；passthrough 覆盖字节保真及 auth / 重试 / 录制的共享路径。

每项记录依赖、验收条件、关联 PR 和复核日期；方向变更确认后再调整版本归属。

## 5. 版本节奏预估

| 版本 | 主题 | 预估 |
|---|---|---|
| 0.6.0 | 门禁 + 协议地基 + 纯减法 | ~4-6 周 |
| 0.7.0 | L1 协议层 + auth | ~5-6 周 |
| 0.8.0 | ops 协议落地（FFI + stdio） | ~4-5 周 |
| 0.9.0 | 治理 | ~3-4 周 |
| 1.0 | 冻结 | ~1-2 周 |

合计约 4-5.5 个月。产出节奏参照 0.3.0 → 0.5.0（6 周 13 PR），按 2-3 PR/周推进；各阶段允许交叠，A/E 轨与 B1/C2 无依赖可同批，#180 / #181 随时插空。

**不排期**（RFC-0036 已定方向）：UDS / HTTP 传输、`aimux-proxy` 扩展包——有真实需求时另立 RFC。**降级**：B9、#171 后半（补 116 个 provider），机会性推进。

## 6. 横切需求

### 6.1 产物体积与性能（硬门禁）

| 产物 | v0.5.0 | 目标 | 手段 |
|---|---|---|---|
| 桌面 `.a` ×5 | 117–139 MB | ≤ 70 MB | S1（只改 staticlib job） |
| iOS slice | ~40 MB/片 | ≤ 32 MB | 已落 strip + 64 MiB 守门（#196）；S3 后视情况再压 |
| Android `.so` ×3 | 合计 21 MB | 默认构建回落；可选裁剪构建 ≤ 15 MB | S3 |
| `.node` / wheel | 15–22 MB | S2 阈值（当前 +10%） | S2 门禁，S3 评估 |
| pub 包压缩 | 31.7 MB | ≤ 25 MB | 跟随 iOS / Android |
| `aimux` CLI | — | 0.8 设定预算 | 新增 |

性能门禁 P：单请求开销、流式吞吐、RSS 增长；0.8 起加入 stdio 往返。

**约束：瘦身不改变对外 API。** profile、strip 是构建管线变化；feature gating 只新增可选的小体积构建路径，默认 feature 全量。需要删除或收窄公开 API 才能换到的收益，另立提案。

### 6.2 协议与生态定期跟踪

| 周期 | 内容 | 产出 |
|---|---|---|
| 每周 | models.dev registry diff（#170） | 报告贴固定 issue，不阻塞合并 |
| 每月 | reachability probe（#170） | 同上 |
| 每月 | 上游参照系 diff：AI SDK、pi-ai、Open Responses 规范变更 | 差异清单；超过 2 个版本未跟进的升 issue |
| 每季度 | 协议新鲜度审计：各 provider 官方 changelog 对照，过时 cassette 标记 | 审计报告 + 协议转换 issue |
| 每季度 | 新机制评估（新模态 / 新协议特性）→ DRAFT RFC 或明确记录不做 | RFC 决策记录 |

0.9 的漂移检测落地后，协议新鲜度从文档对照升级为字节级验证（#167）。
