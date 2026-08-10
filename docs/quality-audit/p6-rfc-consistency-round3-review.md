# P6：第三轮 Review — RFC 状态/实施一致性全量核对 + 生成文件漂移 + 第二轮独立复核

> **快照声明**: 本报告是 **2026-08-07** 的时间点快照，基于 v0.2.1 tag 到当前 HEAD（`4dac8ce`）的代码与文档状态。
> 代码演进后部分发现可能过期；本报告作为诊断参考归档。
>
> **范围与方法**: 第三轮独立新地面——①RFC 状态头与实现事实全量核对（28 个物理文件 / 27 个 RFC 编号，0027 有两份）；
> ②生成文件漂移（provider_registry.json / gen_provider_names / node index.js·d.ts / ts-rs types / CI 门禁）；
> ③第二轮 5 组结论独立复核（B3/R2/R5·R10/M2b/R7·R8）；④docs/plan 状态与代码事实一致性。
> 方法：`ls rfc/` + 读状态头 + `git diff v0.2.1..HEAD --stat -- rfc docs/plan` + grep 关键 API + `python3 scripts/gen_* --check`。
> 只读源码审计；可选 `cargo check -p aimux-core -p aimux-providers` 通过（0.30s，无错误）。
>
> **日期**: 2026-08-07
> **关联**: [p4-release-v021-head-review.md](p4-release-v021-head-review.md)、[p5-native-providers-round2-review.md](p5-native-providers-round2-review.md)

---

## 1. RFC 状态矩阵（全量 28 文件）

> 状态头来源：每个 RFC 文件前 8 行。实现事实来源：grep 关键 API + 读相关源码。
> `git diff v0.2.1..HEAD --stat -- rfc` 显示本轮改动 RFC：0006/0008/0009/0010/0012（仅 2 行小改，疑格式）+ 0015/0016/0017/0020/0021/0022/0023/0024/0025/0026/0027×2（主体新增/重写，共 +3390 行）。

| RFC | 状态头 | 实现事实 | 是否一致 |
|---|---|---|---|
| 0001-multilang-bindings | `v0.5.1 (all phases implemented)` | bindings/{node,python,go,swift,kotlin,java,flutter,c} 全存在 | ✅ 一致 |
| 0002-provider-improvements | **无状态头** | 早期改进提案，已被 0005/0017 取代 | — 无状态头（历史文档） |
| 0003-test-cassette | **无状态头** | cassette 测试机制存在 | — 无状态头 |
| 0004-provider-inventory | **无状态头**（标题即"Implementation Status"） | provider 清单文档 | — 无状态头 |
| 0005-protocol-conversion | **无状态头** | 协议转换设计文档 | — 无状态头 |
| 0005-rename-to-aimux | `Decided plan, pending execution` | 仓库已改名 `aimux`、错误类型 `AiMuxError` 已落地 | ⚠️ **不一致**：状态称"待执行"，实际已执行完毕 |
| 0006-provider-development | `ACCEPTED (in force)` | CONTRIBUTING 引用、provider-research 应用 | ✅ 一致 |
| 0007-search-model-trait | `ACCEPTED` | `SearchModel` trait 存在 | ✅ 一致 |
| 0008-multimodal-bindings | `IMPLEMENTED (2026-08-01)` | 全 binding multimodal 表面存在 | ✅ 一致 |
| 0009-request-resilience | `IMPLEMENTED (2026-07-30)` | shared client + PoolConfig + jitter + timeout 存在 | ✅ 一致 |
| 0010-perf-benchmark-vs-aisdk | `IMPLEMENTED (2026-07-30)` | bench 套件 + PERF-RESULTS.md 存在 | ✅ 一致 |
| 0011-golang-bindings | `v0.2 (typed API + 8-modality)` | Go binding 存在 | ✅ 一致 |
| 0012-source-dedup | `SUPERSEDED (2026-08-02，由 RFC-0017 阶段4 达成)` | 宏生成路线未采用，统一 `provider(name)` 入口已落地 | ✅ 一致 |
| 0013-java-bindings | `IMPLEMENTED (2026-08-01)` | Java binding 存在 | ✅ 一致 |
| 0014-logging | `IMPLEMENTED (2026-08-03)` | logging.rs + http 埋点 + generate span 存在 | ✅ 一致 |
| 0015-cache-trace-audit | `IMPLEMENTED (2026-08-05)` | TraceLayer/RingTraceStore/查询 API/FFI+Node+Python+Go 透传存在 | 🟡 **基本一致**：核心探测已落地；但 [a-line-cache-probe.md:30](../plan/a-line-cache-probe.md#L30) 称 #36 P3 待依赖 RFC-0023，而 0023 现已实施 → P3 应可推进，状态未更新 |
| 0016-align-with-aisdk | `DRAFT (pending review) + §7 实施追踪` | §7 诚实记录已落地（H1/H2/H3/M1/M2/M3/M9/M10）与未落地（M6/M7/M8/M11/M12/M13/L1/L3/L5-L7）项 | ✅ 一致：DRAFT 状态合理，§7.4 自承"§2 表格滞后、以本节为准"，诚实追踪 |
| 0017-provider-config-dx | `IMPLEMENTED (2026-08-02)` | `provider(name)` 工厂 + body_overrides + max_tokens_key 存在 | ✅ 一致 |
| 0018-codex-subscription | `IMPLEMENTED (2026-08-03)` | codex Path A/B + codex_refresh + C ABI 存在 | ✅ 一致 |
| 0019-session-affinity | `ACCEPTED (2026-08-03，代码零改动)` | 纯文档化方案，无代码改动 | ✅ 一致 |
| 0020-external-provider-config | `DRAFT (pending review)` | 全仓 `.rs` 无 `external_provider`/`ExternalProviderConfig`/`register_external` | ✅ 一致：DRAFT，未实施 |
| 0021-composite-model-routing | `DRAFT (pending review)` | 全仓 `.rs` 无 `RouterModel`/`CompositeModel` | ✅ 一致：DRAFT，未实施 |
| 0022-moa-single-fanout | `DRAFT (pending review)` | 全仓 `.rs` 无 `MoaModel` | ✅ 一致：DRAFT，未实施 |
| 0023-runtime-request-recording | `DRAFT (pending review)` | Recorder/RingRecorder/FFI 入口/MockReplayModel/`replay_with_model` 全部已落地；[plan tracker 进度表](../plan/rfc0023-recording.md#L90) P1-P6 + config_snapshot 全 ✅ 已实施/已合并 | 🔴 **不一致**：状态称 DRAFT，实际 P1-P6 已实施（R7 确认） |
| 0024-session-aggregation | `IMPLEMENTED (P1/P2/P5；P3/P4 待依赖 RFC-0023/0015)` | session.rs SessionStore/查询 API 存在 | 🟡 **基本一致**：P3/P4 待依赖，而依赖的 0023 现已实施 → 可推进，状态未更新 |
| 0025-aimux-cli-cache-probe | `IMPLEMENTED (P1/P2/P3；P4/P5 待)` | aimux-cli offline/session/provider 三子命令存在 | ✅ 一致 |
| 0026-openai-compatible-output | `状态:草案 (DRAFT)` | `generate_text_as_openai`/`stream_text_as_openai` 已公开（[generate.rs](../../aimux-core/src/generate.rs) + lib.rs:1190 + 全 binding） | 🔴 **不一致**：状态称草案，实际已实施（R8 确认） |
| 0027-model-catalogue-and-list-api | `状态:草案 (DRAFT)` | `Provider::list_models` trait + `RuntimeModel` + `get_model_specs`(anya2a fetch) 存在；但 RFC 承诺的 `ResolvedModel` 合并类型未实现（[model_catalogue.rs:9-11](../../aimux-core/src/model_catalogue.rs#L9) 明确"deliberately not merged"）、catalogue 缓存/TTL/offline 未实现、CLI `catalogue sync` 未实现 | 🟡 **部分一致**：核心 API 已落地，但 RFC 承诺的 ResolvedModel 合并/缓存/CLI sync 未实现（C1 同源） |
| 0027-list-models-coverage | **无状态头**（覆盖追踪表） | 基线 P1=251 与 provider_registry.json 251 条一致 | 🟡 数据一致，但**缺状态头**（应标 Implemented/P1 阶段） |

**矩阵小结**：
- ✅ 一致：17 个（0001/0006/0007/0008/0009/0010/0011/0012/0013/0014/0016/0017/0018/0019/0020/0021/0022）
- 🟡 基本/部分一致：4 个（0015/0024/0027-model-catalogue/0027-coverage）
- 🔴 明显不一致：3 个（**0023/0026 状态滞后**、0005-rename 状态滞后）
- — 无状态头：4 个（0002/0003/0004/0005-protocol-conversion，历史文档，可接受）

**核心问题**：本轮最严重的一致性缺陷是 **0023/0026 两份已实施 RFC 仍标 DRAFT/草案**（与第二轮 R7/R8 一致，本轮独立复核确认）。此外发现一个**第三轮新问题**——[docs/plan/rfc0023-recording.md](../plan/rfc0023-recording.md) 自身自相矛盾。

---

## 2. 生成文件漂移检查

| 检查项 | 命令 | 结果 | CI 是否门禁 |
|---|---|---|---|
| provider 名称生成 | `python3 scripts/gen_provider_names.py --check` | ✅ **PASS**：8 files up to date (251 names) | ✅ [ci.yml:366](../../.github/workflows/ci.yml#L366) |
| ts-rs 类型生成 | `python3 scripts/gen_ts_types.py --check` | ✅ **PASS**：123 files up to date（122 export 测试绿） | ✅ [ci.yml:375](../../.github/workflows/ci.yml#L375) |
| provider_registry.json vs 0027-coverage | 计数对比 | ✅ **一致**：registry 251 条 = 0027-coverage P1=251 | — |
| node index.js / index.d.ts vs napi 生成 | 工作树 git status | ✅ **干净**（无未提交修改）；CI 用 `git diff --exit-code -- index.js index.d.ts`（napi build 后）门禁 | ✅ [ci.yml:130](../../.github/workflows/ci.yml#L130) |
| bindings/node/src/types/*.ts vs core（ts-rs） | 同 ts-rs --check | ✅ PASS | ✅ ci.yml:375 |
| trailing whitespace（R13） | `git diff --check v0.2.1..HEAD -- bindings/node/src/types/*.ts` | 🔴 **118 行 trailing whitespace**（CallOptions/Fingerprint/GenerateTextOptions 等） | ❌ CI 无 whitespace 门禁（`cargo fmt` 仅覆盖 Rust） |

**结论**：生成文件本身**无漂移**（三套生成器输出与 committed 一致）。唯一漂移风险是 node types 的 trailing whitespace（R13，第二轮已报，本轮确认 118 行，CI 无门禁）。

**CI 实际跑的 --check**（contract-tests job，[ci.yml:360-384](../../.github/workflows/ci.yml#L360)）：
1. `gen_provider_names.py --check` ✅
2. `cargo test --test contract_test -p aimux-core`（注：P1 指出仅验证 JSON 可解析，弱测试）
3. `gen_ts_types.py --check` ✅
4. node `napi build` + `run-node.ts` contract
另外 node-binding job 跑 `git diff --exit-code -- index.js index.d.ts` ✅

**local-ci.sh vs GitHub CI 差距**（E7 精细化确认）：
- [local-ci.sh:76-78](../../scripts/local-ci.sh#L76) contract 阶段只跑 `gen_provider_names.py --check` + `cargo test contract_test` + `run-node.ts`
- **缺** `gen_ts_types.py --check` 与 node `index.js/index.d.ts` 漂移检查
- 即本地 gate 弱于远端 CI，开发者本地 `--quick` 通过不代表 CI 通过

---

## 3. 第二轮结论独立复核表

| 编号 | 结论 | 依据 |
|---|---|---|
| **B3**（版本仍 0.2.1 + [Unreleased] 空） | **属实（且更严重）** | [Cargo.toml:15](../../Cargo.toml#L15) `version = "0.2.1"`；[CHANGELOG.md:8](../../CHANGELOG.md#L8) `[Unreleased]` 段为空（紧接 `[0.2.0]`）；**且整个 CHANGELOG 无 `[0.2.1]` 段**（v0.2.1 tag 存在却无对应条目）；compare link `[Unreleased]: .../compare/v0.1.0...HEAD` 仍从 v0.1.0 起（应 v0.2.1...HEAD） |
| **R2**（RateLimited 变体改形） | **属实** | v0.2.1：`RateLimited { retry_after_ms: u64 }`（error.rs:33）；HEAD：`RateLimited { retry_after_ms: u64, message: String }`（[error.rs:42-46](../../aimux-core/src/error.rs#L42)，`#[serde(default)]` 仅反序列化兼容旧 JSON）；序列化形状破坏；有 `rate_limited_serde_back_compat` 测试（error.rs:231）验证旧 JSON 可反序列化 |
| **R5**（provider 手册声称 listModels 合并 anya2a + 缓存/TTL） | **属实（且范围更广）** | [provider-config-manual.md:119](../../docs/provider-config-manual.md#L119) 称"用 anya2a 补充配置"；:127-129 示例 `models[i].spec` 字段；:153-158 §7.5 描述缓存目录/`AIMUX_CATALOGUE_DIR`/TTL 24h/`AIMUX_CATALOGUE_OFFLINE`。**代码事实**：[model_catalogue.rs:9-11](../../aimux-core/src/model_catalogue.rs#L9) 明确"deliberately not merged, host fetches both"；[catalogue.rs:8-11](../../aimux-providers/src/catalogue.rs#L8) 明确"does no caching, no FS writes, no TTL"。手册承诺的 `.spec` 合并与缓存/TTL/offline 均不存在 |
| **R10**（C header 声称 list_models 返回 ResolvedModel[]） | **属实** | [aimux-ffi.h:231](../../aimux-ffi/aimux-ffi.h#L231) `Returns a JSON array of ResolvedModel`；代码 [provider.rs:44-46](../../aimux-core/src/provider.rs#L44) 返回 `Vec<RuntimeModel>`；`ResolvedModel` 非真实 struct（仅 [model_catalogue.rs:25](../../aimux-core/src/model_catalogue.rs#L25) 注释引用） |
| **M2b**（config_snapshot 缺 8 个 LanguageModel） | **部分属实（主论断属实，一处子论断不准确）** | 主论断属实：bedrock/anthropic_aws/vertex/vertex-anthropic/xai/xai-responses/open_responses/huggingface_responses 8 个 `impl LanguageModel` 均**无 `fn config_snapshot`**（grep 仅命中 codex/mistral/cohere/azure×2/openai×2/anthropic/google 9 处）。plan tracker [rfc0023-recording.md:98](../plan/rfc0023-recording.md#L98) 自承仅覆盖 6 原生族，证实此 8 为已知范围外缺口。**但第二轮子论断"open_responses/huggingface_responses 的 list_models 已构造 OpenAIConfig 却不复用 snapshot helper"不准确**：二者**无 `fn list_models`**（用 trait 默认 Unsupported），不存在"已构造 OpenAIConfig"；该子论断仅对 **xai** 属实（[xai/mod.rs:119](../../aimux-providers/src/xai/mod.rs#L119) list_models 构造 OpenAIConfig 却无 config_snapshot） |
| **R7**（RFC-0023 状态未同步） | **属实** | [rfc/0023:3](../../rfc/0023-runtime-request-recording.md#L3) `Status: DRAFT (pending review)`；但 plan tracker 进度表 [rfc0023-recording.md:90-99](../plan/rfc0023-recording.md#L90) 显示 P1-P6 + config_snapshot 全 ✅ 已实施/已合并 |
| **R8**（RFC-0026 状态未同步） | **属实** | [rfc/0026:3](../../rfc/0026-openai-compatible-output.md#L3) `状态:草案`；`generate_text_as_openai`/`stream_text_as_openai` 已公开（generate.rs + lib.rs:1190 + 全 binding） |

**误报审查**：第二轮 5 组结论无完全误报；**M2b 有一处子论断需修正**（open_responses/huggingface_responses 无 list_models，非"已构造 OpenAIConfig"）。其余 B3/R2/R5/R10/R7/R8 全部属实。

---

## 4. 新 Findings（第三轮）

### 🔴 Major

#### N3-1. [major] docs/plan/rfc0023-recording.md 自相矛盾：顶部称"待实施"但进度表显示 P1-P6 全部已实施
- **位置**: [docs/plan/rfc0023-recording.md:5-6](../plan/rfc0023-recording.md#L5) vs [:90-99](../plan/rfc0023-recording.md#L90)
- **问题**: 文件头（L5）"状态:2026-08-06 对齐完成 + 双模型评审完成，**待修订 RFC 定稿后实施**"；L6"（P1-P3 曾实施后回滚，代码在 backup 分支）"。但同文件"实施进度"表（L92-98）显示 P1 ✅已合并(PR #85)、P2-P6 + config_snapshot 全 ✅已实施。**同一文件内部自相矛盾**——头部描述的是回滚前的旧状态，进度表是最新事实，头部未更新。
- **影响**: 维护者读头部会误判 RFC-0023 尚未实施，可能重复实施或误判 API 未公开。与 R7 同源但属独立的新发现（plan tracker 自身矛盾，非 RFC 正文）。

#### N3-2. [major] provider-config-manual.md §7 整段描述与代码事实不符（R5 扩展确认）
- **位置**: [docs/provider-config-manual.md:119-158](../../docs/provider-config-manual.md#L119)
- **问题**: §7.1-7.4 声称 `listModels()` 返回带 `.spec`（ModelSpec）合并的对象；§7.5 描述 catalogue 缓存目录 `~/.cache/aimux/catalogue/`、`AIMUX_CATALOGUE_DIR`、TTL 24h、`AIMUX_CATALOGUE_OFFLINE=1`。代码 [model_catalogue.rs:9-11](../../aimux-core/src/model_catalogue.rs#L9) 与 [catalogue.rs:8-11](../../aimux-providers/src/catalogue.rs#L8) 明确"不合并、不缓存、不落盘、无 TTL"。用户照手册访问 `models[i].spec` 会得到 `undefined`，设置 `AIMUX_CATALOGUE_OFFLINE` 无效。

### 🟠 Minor

#### N3-3. [minor] aimux-ffi.h 取消能力声明自相矛盾（R6 独立复核确认）
- **位置**: [aimux-ffi.h:23-26](../../aimux-ffi/aimux-ffi.h#L23) vs [:284-326](../../aimux-ffi/aimux-ffi.h#L284)
- **问题**: L23-26"there is no abort/cancel entry point over the C ABI"；同文件 L284-326 公开 `aimux_abort_signal_new`/`aimux_abort_signal_abort`/`aimux_abort_signal_drop`/`aimux_stream_text_with_abort`。代码 [lib.rs:1118](../../aimux-ffi/src/lib.rs#L1118) `aimux_abort_signal_new` 已实现。属第二轮 R6，本轮复核确认仍存在。

#### N3-4. [minor] RFC-0027 P3 承诺的 "CLI aimux-cli catalogue sync 预拉" 未实现
- **位置**: [rfc/0027-model-catalogue-and-list-api.md:276](../../rfc/0027-model-catalogue-and-list-api.md#L276)（P3 阶段） vs tools/aimux-cli/src/
- **问题**: RFC-0027 P3 列"native provider list_models + CLI `aimux-cli catalogue sync` 预拉 + manual 更新"。grep tools/aimux-cli/src/ 无 `catalogue`/`sync` 子命令（仅有 offline/session/provider 三 probe 子命令）。CLI 未提供 catalogue 预拉。与 0027 DRAFT 状态部分一致（未实施项本应在 DRAFT 范围），但 manual §7.5 已向用户描述该能力（N3-2），形成文档承诺与代码双重缺口。

#### N3-5. [minor] local-ci.sh 弱于 GitHub CI（E7 精细化确认）
- **位置**: [scripts/local-ci.sh:76-78](../../scripts/local-ci.sh#L76)
- **问题**: contract 阶段仅跑 `gen_provider_names.py --check` + `cargo test contract_test` + `run-node.ts`；**缺** GitHub CI 有的 `gen_ts_types.py --check`（[ci.yml:375](../../.github/workflows/ci.yml#L375)）与 node `index.js/index.d.ts` 漂移检查（[ci.yml:130](../../.github/workflows/ci.yml#L130)）。开发者本地 `--quick`/`--only=contract` 通过不保证 CI 通过。

### 🟡 Nit

#### N3-6. [nit] rfc/0027-list-models-coverage.md 缺状态头
- **位置**: [rfc/0027-list-models-coverage.md:1-12](../../rfc/0027-list-models-coverage.md#L1)
- **问题**: 全文无 `Status:` 行（仅有"目标/基线"陈述），与其余 RFC 格式不统一。其 P1=251 数据与 registry 一致，但缺状态标记（应标如 "Implemented (P1)"）。

#### N3-7. [nit] 0005-rename-to-aimux.md 状态滞后
- **位置**: [rfc/0005-rename-to-aimux.md:3](../../rfc/0005-rename-to-aimux.md#L3)
- **问题**: `Status: Decided plan, pending execution`，但改名已完成（repo=aimux、`AiMuxError` 已落地）。应标 Implemented/Done。

#### N3-8. [nit] RFC-0015/0024 头部"待依赖 RFC-0023"现已可推进但未更新
- **位置**: [a-line-cache-probe.md:30](../plan/a-line-cache-probe.md#L30)（#36 P3 待 RFC-0023）、[rfc/0024:3](../../rfc/0024-session-aggregation.md#L3)（P3/P4 待）
- **问题**: 0015 P3 与 0024 P3/P4 均标注"待依赖 RFC-0023"，而 0023 现已 P1-P6 全部实施（见 N3-1），依赖已满足但状态未刷新。

---

## 5. docs/plan 状态与代码事实一致性

| 文件 | 内容 | 与代码一致性 |
|---|---|---|
| [docs/plan/README.md](../plan/README.md) | 计划总索引，**仅跟踪 RFC-0017 阶段 2**（4 个 stage 全 ✅ done） | ⚠️ **滞后**：未纳入本轮新 RFC（0015/0023/0024/0025/0026/0027）的计划跟踪；README §7.4 自承"backlog.md 仅跟踪 RFC-0017 阶段 2 项" |
| [docs/plan/a-line-cache-probe.md](../plan/a-line-cache-probe.md) | A 线追踪（RFC-0015/0024/0025），#49/#36/#52 全 ✅ 完成 | ✅ 一致（与三 RFC 状态头吻合）；仅 P3 依赖 0023 的标注现已过期（见 N3-8） |
| [docs/plan/rfc0023-recording.md](../plan/rfc0023-recording.md) | #48 RFC-0023 对齐追踪 | 🔴 **自相矛盾**（见 N3-1）：头部"待实施" vs 进度表"P1-P6 全实施" |
| [docs/plan/backlog.md](../plan/backlog.md) | 非阻塞项归档，**仅含 RFC-0017 阶段 2 相关**（B1-B10，多数已闭合） | ⚠️ **范围窄**：未归档本轮新 RFC 的非阻塞发现（如 0023 漂移项、0027 未实现承诺） |

**结论**：A 线追踪（0015/0024/0025）与代码一致；但 **plan 总索引 README 与 backlog 未覆盖本轮新 RFC 的实施跟踪**，且 rfc0023-recording.md 内部矛盾。RFC 完成度在 docs/plan 层面呈现碎片化、非单一事实源。

---

## 6. Top 5 优先修复建议

1. **同步 RFC-0023/0026 状态头 + 修 rfc0023-recording.md 矛盾**（R7/R8 + N3-1）：将 [rfc/0023:3](../../rfc/0023-runtime-request-recording.md#L3) 与 [rfc/0026:3](../../rfc/0026-openai-compatible-output.md#L3) 由 DRAFT/草案 改为 Implemented（标注 P1-P6 落地）；同步刷新 [rfc0023-recording.md:5-6](../plan/rfc0023-recording.md#L5) 头部使其与进度表一致。这是本轮最显眼的一致性缺陷，零代码风险。

2. **修齐 provider-config-manual.md §7 + aimux-ffi.h list_models 描述**（R5/R10 + N3-2/N3-4）：删除手册中 `models[i].spec` 合并、缓存/TTL/offline 的虚假描述，改为"返回裸 `RuntimeModel`，宿主自行调 `get_model_specs` 合并；catalogue 无内置缓存"；[aimux-ffi.h:231](../../aimux-ffi/aimux-ffi.h#L231) 将 `ResolvedModel` 改为 `RuntimeModel`；补/删 RFC-0027 P3 未实现的 `catalogue sync` 承诺。

3. **删 aimux-ffi.h 取消能力矛盾声明**（R6/N3-3）：[aimux-ffi.h:23-26](../../aimux-ffi/aimux-ffi.h#L23) 删除"no abort/cancel entry point"表述，改为正确描述 `aimux_abort_signal_*` + `aimux_stream_text_with_abort`。

4. **B3 版本与 CHANGELOG 策略**（B3 确认）：升 workspace 版本至 0.3.0；填充 `[Unreleased]`（breaking: RateLimited 加 message 字段 / CallOptions·GenerateTextOptions·StreamTextResult 加字段 / C header 删 `aimux_last_error`；additive: recording/replay/session/trace/list_models）；修 compare link 为 `v0.2.1...HEAD`；补 `[0.2.1]` 段。

5. **补 config_snapshot 覆盖 + 修正 M2b 子论断**（M2b）：至少为 xai 复用 `config_snapshot_from_config`（其 list_models 已构造 OpenAIConfig，零新增抽象）；open_responses/huggingface_responses 若要覆盖需先补 `list_models`（当前用 trait 默认 Unsupported）；bedrock/vertex/anthropic_aws 至少补 base_url + api_key_source 以支持回放重建。注意 plan tracker 已将此 8 列为"6 原生族之外"的已知范围，应显式标为 backlog 缺口而非默认覆盖。

---

## 7. 剩余不确定性

- **node index.js/index.d.ts 漂移**：本地未跑 `npx napi build`（耗时），仅验证工作树干净 + CI 门禁存在（[ci.yml:130](../../.github/workflows/ci.yml#L130)）。若 committed 副本与 napi 输出不一致，CI 会捕获但本地未独立复核。
- **M2b 子论断修正的边界**：本轮确认 open_responses/huggingface_responses 无 `fn list_models`，但未逐一读其完整 `impl LanguageModel` 块确认是否经宏/其他方式提供 list_models（grep `fn list_models` 无命中，可信度高）。
- **RFC-0015 P3 / RFC-0024 P3/P4 是否真已可推进**：依赖 RFC-0023 已实施（plan tracker 进度表事实），但 0023 状态仍 DRAFT、N3-1 显示 plan tracker 头部称"待定稿后实施"——若维护者以"RFC 未定稿"为由认为 0023 实施不算数，则 0015/0024 依赖仍未解除。状态定义层面存在歧义。
- **`cargo check` 仅覆盖 core + providers**：未跑 workspace 全量 / clippy / test（遵循只读审计 + p4 已验证基线）。
