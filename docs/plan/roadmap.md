# Roadmap：0.5.0 → 1.0

> 基线：v0.5.0（2026-09-28 发布）。本文档是 issue 驱动的排期总览，
> 单项设计与差距分析仍在各 issue / RFC 内，本文不复制细节。
> 上次复核：2026-09-28（open issues #95 #166 #167 #170 #171 #174 #175 #179 #180 #181 #185）。

## 1. 依赖全景

```
#166 代码缩减总纲（25 PR，净 ≈ −44k 行，已对抗性验证）
├─ A 轨 机械清理            A1 ✅(#169)；A2–A5 未动，互相独立
├─ B 轨 protocol registry（RFC-0032 修正版，−10~12k 行）
│   ├─→ #174 Auth L1（registry auth schema + apply_auth）  ← 依赖 B1
│   │     └─→ #175 Auth L2（CredentialStore + TokenRefresher）
│   └─→ #167 transport-level replay                         ← 依赖 B1
│         └─→ #179 replay 子命令                            ← 依赖 #167
├─ C 轨 FFI ops（109 导出 → 9 JSON 导出）；C2 构造器转发是 B4/B5 前置
├─ D 轨 7 语言 binding 重写 + D8 类型镜像生成（依赖 C1；手工镜像 18.9k 行归零）
├─ E 轨 #164 内部清理 + 文档三层重组（docs/ 用户面 / rfc/ 索引 / archive/ 历史）
│
├── 独立项：#185 ToolInput 类型化 + 删 StreamingToolCallTracker
├── 独立项：#170 registry 维护自动化 → #171 triage（27 个 base_url 分歧 + 116 缺失）
└── 小项：#181 session 后续 | #180 cache probe §10 八项 | #179 online mode（需设计决策）

横切（见 §6）：产物体积（桌面 .a 117–139MB）| 协议/生态定期跟踪（扩展 #170）

常设跟踪：#95 错误体系（P1/P2 已落地于 #94/#158，基本收官，新错误相关 PR 挂此 issue）
```

## 2. 阶段划分

### 0.6.0 —— 「收缩 + 体积止血」（低风险，目标 ~3-4 周）

目标：消化全部纯减法与解锁项，发布产物体积回落，API 面不动（除两处 Rust 层 breaking）。

| 项 | 内容 | 量级 |
|---|---|---|
| A2 | 删生成式 `ProviderName` 枚举及 9 份语言副本 | −3.5k |
| A3 | `provider-inventory/` 归档 `archive/`（42.6k 行数据移出仓库树） | 归档 |
| A4 | `aimux-web` 140 个 ts-rs 类型副本指向生成集 | −3.0k |
| A5 | Flutter example 删桌面平台脚手架（iOS/Android 留守 CI 门禁） | −3.0k |
| E1 | #164 内部清理 + 文档三层重组（§1–§3/§14 保留，其余入 rfc/0031） | −1.5k |
| #185 | `ToolInput{Raw,Parsed}` 类型化 + 删 429 行死码 tracker | −0.5k |
| #170 | `sync_registry.py` 周报 + `probe_registry.py` 月探活 + scheduled Action | 新增脚本 |
| #171(前半) | 27 个 base_url 分歧逐条 triage（修错的、note 有意的） | 数据 |
| **S1** | **桌面 `.a` 切 LTO-off profile**：v0.5.0 的 `libaimux_ffi-*.a` 达 117–139MB（fat-LTO bitcode，同 iOS 根因）；`ios-release` profile 改名 `staticlib-release` 复用于所有 `.a` 目标，Linux 实测 138.4→66.5MB | 体积 |
| **S2** | **体积门禁推广**：把 iOS 的 64MiB slice 守门推广到所有发布产物（桌面 `.a`、Android `.so`、`.node`、wheel），CI 输出体积报告 | 门禁 |
| **B1** | registry 行加 `protocol` 列（纯增量，RFC-0032 修正 1–7 已写入 issue） | +行 |
| **C2** | 40 个 FFI 构造器转 `provider()` 的转发 shim | 前置项 |

B1 + C2 是本版本最重要的两项：它们解锁 #174 / #175 / #167 / #179 与 B4–B6，
纯增量零破坏，越早落地后续越顺。

版本语义：Rust 层 breaking（A2 删枚举、#185 换类型），binding wire 与 C ABI 不变。

### 0.7.0 —— 「protocol registry + auth」（主菜，目标 ~5-6 周）

目标：provider 构造从代码变成数据，auth 从 12 处手写收敛为一处；顺带完成体积审计（S3，见 §6.1）。

| 项 | 内容 |
|---|---|
| B2→B9 | protocol registry 全量：17 个 LanguageModel 实现 → 7 个协议；33 个 wrapper 退役；responses 家族合并（注意 #166 修正 3：xai/mistral 非纯 fork，节流为 profile hooks；修正 6：五个 `MistralProvider::model()` 直连点先迁 `provider()`） |
| #174 | registry `auth{kind,env,header}` schema + `apply_auth()` 单点；`none` kind 退役 33 处 `PLACEHOLDER_API_KEY` |
| #175 | `Credential` 解析序（explicit → env → store）+ `CredentialStore` + 通用 `TokenRefresher`（codex_refresh 泛化；vertex 未装 refresher 时照旧抛 `TokenExpired`） |
| #171(后半) | 116 个缺失 provider 按 auth/protocol schema 一行一个补齐 + cassette |

版本语义：Rust API 大 breaking（33 wrapper 退役）；C ABI 经 C2 转发不受影响。

0.7.0 期间体积跟进（S3）：`cargo-bloat` 审计 0.5.0 动态库增重来源（Android `.so`
13.4→21MB，+57%：jsonschema / rustls+webpki-roots(ws) / replay / repair 等新依赖），
评估 per-modality 或 per-protocol feature gating 对 binding 包的收益——产出仅为
**可选的 feature 组合与裁剪文档**，默认 feature 全量、对外 API 不变（§6.1 约束）。
node workspace 默认全量的问题需一并解决。

### 0.8.0 —— 「FFI / binding 手术」（~4-5 周）

| 项 | 内容 |
|---|---|
| C1→C3→C4 | 109 个 FFI 导出收敛为 9 个 JSON ops（recording/mock/session 走 `configure` op） |
| D1–D7 | 七语言 binding 按同一 ops 面重写（Kotlin 先行——纯 Java artifact；Node 先测 runtime 重入；Python 先做 ctypes 原型） |
| D8 | 类型镜像生成：serde 类型 → JSON Schema → 六语言，`gen_ts_types.py --check` 扩到全部输出 |

版本语义：C ABI breaking，八个 binding 全量重发。

### 1.0 —— 「replay 完成 + API 冻结」锚点

| 项 | 内容 |
|---|---|
| #167 | transport-level replay：mock 挂在 HTTP 层跑真协议代码，覆盖全协议/全模态；删 OpenAI-only 解码器（~1.5k 行）；`ProviderRecord` = registry 行 + protocol；exchange schema 与 RFC-0003 cassette 统一；Router/MoA 记录实际 child；`PassthroughOnMiss` |
| #179 | `aimux-cli replay` 子命令（依赖 #167）；`online` mode 需先做设计决策（dump-on-signal / sidecar / FFI export，单独 RFC） |
| #181 / #180 | session partial-match 决策、debug CLI（并入 aimux-cli）、cache probe §10 验证项清尾 |
| — | 1.0 发布判据：#166 ledger 全勾 + 错误模型 #95 / 请求管线 #164 双稳定一个周期 |

时机理由：错误模型（#158）、请求管线（#164）刚在 0.5.0 定型；#166 全量落地后
API 面才真正定形，1.0 语义干净。粗估 3.5–5 个月后。

## 3. 贯穿原则（沿用 #166 ground rules）

- **一个方向一个 PR**；providers 先于 bindings；最高确定性的删除先落。
- **Cassettes 是字节级回归门禁**：2,799 份录音只增不减，后续 PR 依赖更重。
- **文档重组而非删除**：`git mv` + 链接修复；`archive/` 只留 README 指向 git 历史。
- **Scheduled registry 报告永不阻塞合并**（#170 验收条件）。
- 语言规则：`docs/` 与 `bindings/*/README` 英文，`rfc/` 中文。

## 4. 近期第一步（本周可并行开）

1. **A2–A5**：四个独立 PR，纯删码/归档。
2. **#185**：先删 `StreamingToolCallTracker`（纯删），再 `ToolInput` 枚举（17 处机械替换）。
3. **#170**：sync/probe 脚本 + Action，吸收两个 LiteLLM survey 脚本。
4. **S1**：桌面 `.a` 切 staticlib profile——机制已在 iOS 修复（PR #196）中验证，
   改 profile 名 + `ffi-build` job 换 profile + S2 门禁，一个小 PR。
5. **B1 + C2**：解锁项，RFC-0032 修正版按 issue 内 7 条修正执行。

## 5. 版本节奏预估

| 版本 | 主题 | 预估 |
|---|---|---|
| 0.6.0 | 收缩 + 解锁 + 体积止血 | ~3-4 周 |
| 0.7.0 | protocol registry + auth + 体积审计 | ~5-6 周 |
| 0.8.0 | FFI ops + bindings | ~4-5 周 |
| 1.0 | replay + 冻结 | ~2-3 周 |

产出节奏参照 0.3.0→0.5.0（6 周 13 PR，含 3 个特大）；25 个 #166 PR
按 2-3 PR/周推进。各阶段允许交叠：A/E 轨与 B1/C2 无依赖可同批。

## 6. 横切需求

### 6.1 产物体积（binary-size budget）

v0.5.0 现状与目标：

| 产物 | v0.5.0 | 目标 | 手段 |
|---|---|---|---|
| 桌面 `.a` ×5 | 117–139 MB | ≤ 70 MB | S1：staticlib LTO-off profile（已验证 138.4→66.5MB） |
| iOS slice | ~40 MB/片 | ≤ 32 MB | 已落 strip+守门（#196）；S3 审计后视情况再压 |
| Android `.so` ×3 | 21 MB 合计 | 默认构建依赖瘦身回落；裁剪构建 ≤ 15 MB | S3：cargo-bloat 找增重；gating 仅作可选路径（§6.1 约束） |
| `.node` / wheel | 15–22 MB | 报告化 | S2 门禁先跟踪，S3 评估可选裁剪收益 |
| pub 包压缩 | 31.7 MB | ≤ 25 MB | 跟随 iOS/Android 改善（Flutter 包固定全量） |

原则：体积是发布门禁不是事后优化——每个产物有预算线（S2），回归即 CI 失败，
与 iOS 的 64MiB 守门同机制。体积问题不新增单独版本，S1/S2 随 0.6.0，S3 随 0.7.0。

**约束：瘦身不改变对外 API。** 所有体积手段（profile、strip、gating）以默认
构建的对外 API 面不变为前提：S1/S2 是构建管线变化，代码零改动；S3 的 feature
gating 若落地，只新增「可选的小体积构建路径」（按需关闭某模态/某协议的
编译），默认 feature 集全量、binding 的完整 API 不变——凡需要删除或收窄
公开 API 才能换到的体积收益，不在此需求范围内，需另立提案讨论。

### 6.2 协议与生态定期跟踪（ecosystem tracking）

registry 数据层已由 #170 覆盖（weekly models.dev diff + monthly reachability）。
协议行为层与上游演进在此扩展为固定节奏：

| 周期 | 内容 | 产出 |
|---|---|---|
| 每周 | models.dev registry diff（#170） | 报告贴固定 issue，不阻塞合并 |
| 每月 | reachability probe（#170） | 同上 |
| 每月 | **上游参照系 diff**：Vercel AI SDK release notes + pi-ai 变更，标注 aimux 未跟进的能力 | 差异清单贴 tracking issue，超出 2 个版本未跟进的项升 issue |
| 每季度 | **协议新鲜度审计**：对照各 provider 官方 changelog（OpenAI Responses 演进、Anthropic thinking/computer-use 参数、Gemini API 版本等），过时 cassette 标记 | 审计报告 + 需更新的协议转换 issue |
| 每季度 | **新机制评估**：新模态/新协议特性（MCP、computer use、audio output…）→ 开 DRAFT RFC 或明确记录不做 | RFC 决策记录 |

落地方式：扩展 #170 的 scheduled Action 为三条流水（registry-diff / probe /
upstream-diff），quarterly 审计人工触发。#167（transport replay）落地后，
「重新录制真实 provider cassette」可脚本化，协议新鲜度从文档对照升级为
字节级验证——这是把 #167 排在 1.0 前的另一个理由。

关联：#95（错误体系参照 AI SDK）是此节奏在错误域的常设形式；#171 的 triage
是 registry 数据层的一次性清偿。

