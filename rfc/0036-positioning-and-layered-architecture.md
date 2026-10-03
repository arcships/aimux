# RFC-0036: 定位与分层架构——provider 接入与治理运行时

> **Status**: Draft（§12–§14 为 2026-10-03 设计覆盖增补；原 §1–§11 的目标冲突按 §12.2 逐项处理）
> **Date**: 2026-09-29
> **Scope**: 明确 aimux 的产品定位,确立数据分层(L0–L3)、一套 ops 协议与两个入口(FFI 绑定 / stdio CLI)、proxy 扩展包的方向、治理产品线,并据此重排 0.6 → 1.0 路线。
> **Related**: [#166](https://github.com/arcships/aimux/issues/166)(代码缩减总纲)、[#197](https://github.com/arcships/aimux/pull/197)(roadmap 草案)、[#167](https://github.com/arcships/aimux/issues/167)、[#170](https://github.com/arcships/aimux/issues/170)、[RFC-0016](0016-align-with-aisdk.md)、[RFC-0023](0023-runtime-request-recording.md)、[RFC-0026](0026-openai-compatible-output.md)、RFC-0032(provider protocol registry,**尚未入库**)

---

## 1. 背景

#197 的 roadmap 以 #166 为主干,从「代码的理想形态」倒推版本,在 1.0 冻结 API。它回答了**怎么把代码变干净**,没有回答**aimux 为谁、替代什么、长期抽象押在哪一层**。

几个外部事实(2026-09 调研):

- **采用度仍处早期**:npm 约 200/月、PyPI 约 90/月、194★、两位贡献者。此时冻结 API,冻结的是猜测。
- **同构竞品已出现**:liter-llm(Rust core,14 种绑定由生成器产出,自带 OpenAI 兼容 proxy / MCP / OTel)。「Rust core + 多绑定」本身不再是差异。
- **长尾协议在收敛**:Open Responses 规范(2026 年初)已被 Hugging Face、OpenRouter、Vercel、Ollama、vLLM、LM Studio、Databricks 采用。provider 数量不再是护城河。
- **统一格式本身在漂移**:AI SDK provider spec 约一年内 V2 → V3 → V4;Open Responses 走 item 模型(reasoning 为一等 item、`encrypted_content` 跨轮回传),与 AI SDK 的 parts 模型不同构。
- **私有 API 持续涌现**:各家的私有参数、私有端点、私有协议(realtime、batch、files、缓存控制……)不会停止。

## 2. 定位

> **aimux 是 provider 接入与治理的运行时**:性能与体积是硬约束;向上为多语言栈提供一致行为;向下跟随各家演进且各异的协议;自带对 provider 的端侧测试与治理;可嵌入进程,也可作为独立进程(CLI / API)与 harness 解耦。

六条原则,按优先级:

1. **性能与体积是第一基准线**。每个产物、每个载体都有预算,回归即 CI 失败。
2. **多语言支持的目的是多语言栈的行为统一**:同一份重试、超时、错误、工具修复、录制语义,而不是「每种语言各有一个 SDK」。
3. **治理是一等能力**:对 provider 做端侧测试、漂移检测、能力验证,不只是调用它。
4. **协议会演进、私有 API 会持续存在**:架构不能以最小公约数为真相;任何私有能力第一天就应可达。
5. **与 harness 解耦**:aimux 可作为子进程(stdio CLI)运行,harness 无需链接它。
6. **统一数据格式是需求,但具体抽象(AI SDK 形态)的长期适配度待评估**,不与 1.0 绑定。

**不做**:agent loop、编排、RAG(沿用 RFC-0016 §7.5);多租户网关业务(virtual key、计费、预算、限流);OAuth 登录流程(沿用 RFC-0018)。

## 3. 数据分层:真相下沉

| 层 | 内容 | 真相? | 现状 |
|---|---|---|---|
| **L0 传输** | auth 注入、重试、超时、代理、录制;**native passthrough**:原样发送任意请求到任意 provider 端点,享受 L0 全部能力 | 是(字节) | 录制在;passthrough **缺失** |
| **L1 协议** | 每个协议的原生类型与转换,**无损** | 是(结构) | 17 个 `LanguageModel` 实现,待 B 轨收敛为 ~7 个协议 |
| **L2 统一视图** | 当前的 AI SDK 形态(`GenerateResult` / `StreamPart`)。**L1 的投影**;投影不下的进 `provider_metadata` 或 `Raw`,不得静默丢弃 | 否 | 已有;`include_raw_chunks` 在 core 定义,仅 3 处 provider 发出 `StreamPart::Raw` |
| **L3 治理** | 基于 L0 录制的一致性测试、漂移检测、能力矩阵、cache probe、回放 | — | 散落于 #167 / #170 / #180 / aimux-cli / aimux-web |

规则:

- **L0 passthrough 是私有 API 的兜底**(原则 4):新端点、新参数不必等 L1/L2 跟进即可调用,且被录制、被治理。
- **L2 独立版本化**(`data_format_version`),不随 1.0 冻结。L2 的选型(继续 AI SDK 形态 / 转向 Open Responses items / 自有模型)另立调研 RFC,评估指标:
  1. 各协议 L1 → L2 往返保真率(原生字段落入 `provider_metadata` / 丢失的比例);
  2. 过去 12 个月跟随 AI SDK spec 的变更次数与改动面;
  3. Open Responses items 对 aimux 现有 8 个模态与 reasoning / 缓存 / 引用语义的覆盖度。

## 4. ops 协议与载体

### 4.1 一套协议

**ops 协议** = 把「aimux 能做的每件事」从一个个专用导出函数,变成「操作名 + JSON 参数」的数据。

现状是一件事一个导出(`aimux-ffi` 120 余个 `extern "C"`):40 个左右的 provider 构造器(`aimux_openai_new`、`aimux_anthropic_new`……)、`aimux_generate_text` / `aimux_stream_text` / `aimux_stream_text_with_abort`、23 个 `aimux_error_*` 访问器、`aimux_trace_*` / `aimux_session_*` / `aimux_recording_*`。每加一个能力,8 种语言各写一遍声明、封装与测试。

#166 C 轨把它收敛为固定的几个入口,操作名是参数:

```c
aimux_model_new(spec_json, &handle)                        // 任何模型:{"provider":"anthropic","model":"…","kind":"language"}
aimux_call  (handle, "generate_text", request_json, &out)  // 所有一次性操作
aimux_stream(handle, abort, "stream_text", request_json, on_event, ctx)  // 所有流式操作
aimux_call  (0, "configure", {...})                        // 录制、session、代理、日志
/* + abort / drop / free_string;错误统一为 JSON 信封 */
```

调用一旦变成「op 名 + JSON 请求 → JSON 响应 / 事件流」,就**不再依赖 C 函数调用这个载体**,同一操作换传输照样成立:

```jsonc
{"id":1,"op":"model_new","params":{"provider":"anthropic","model":"claude-sonnet-4-5"}}
{"id":1,"result":{"handle":7}}
{"id":2,"op":"stream_text","handle":7,"params":{"prompt":[]}}
{"id":2,"event":{"type":"text-delta","delta":"Hel"}}
{"id":2,"event":{"type":"finish","finish_reason":"stop"}}
{"id":3,"op":"cancel","params":{"target":2}}
```

Rust 侧只有一个 `dispatch(op, handle, json)`;FFI 与 stdio 只是把消息送到它的两个入口。本 RFC 将 C 轨的设计**提升为 aimux 的一等协议**,在其基础上追加以下要求:

- **消息**:request / response / stream event(带 id)/ cancel(id) / error envelope(即 C3 的错误 JSON)。
- **op 表**:单一来源,穷举测试(每个 op × 每种 handle 类型),导出为各语言常量与 JSON Schema。
- **二进制帧**:实时转写推音频、文件上传,不能强制 base64(+33%)。帧格式 = 长度前缀 + 类型字节(JSON 帧 / 二进制帧),二进制帧以 stream id 关联。**因此不直接采用 LSP 式 JSON-RPC over stdio 的纯文本帧。**
- **版本协商**:握手时交换协议版本与 op 表版本。

### 4.2 两个入口

| 入口 | 调用方 | 说明 |
|---|---|---|
| **FFI** | 8 种语言绑定 | 全部按 #166 D 轨重写为 ops 之上的薄封装,**一种形态,不分级** |
| **stdio** | harness、治理工具 | `aimux` CLI 以子进程方式提供同一套 ops;零配置、无端口、生命周期随调用方 |

两个入口共用同一个 dispatch;一致性由构造保证,而非逐语言对齐。

**本轮不做**:UDS / named pipe daemon、HTTP 服务。协议传输无关,日后有真实需求再加,不改协议本身。

### 4.3 性能门禁

stdio 入口的往返开销与 FFI 一并测量(单请求开销、流式吞吐、200 KB 上下文序列化),进 CI 门禁。

## 5. proxy 扩展包(方向,不排期)

proxy = **别人的协议外观**(OpenAI Chat、Responses / Open Responses、Anthropic Messages……入站,任意 provider 出站),供不认识 aimux 的现成工具改 base_url 接入;也是治理第三方 harness 的入口。

仅确立方向与边界,本轮不排期:

- 独立 crate / 二进制,不进默认构建与绑定包(原则 1)。
- 同协议走 L0 passthrough(无损);跨协议经 L2,有损字段显式报告。
- 不做多租户、virtual key、计费、预算、限流。

## 6. 治理产品线

把现有资产(2,800 份 cassette、RFC-0023 录制、RFC-0024 session、RFC-0015 cache probe、#170 registry diff / 探活)收成一条线,经 CLI 暴露:

- **有证据的能力矩阵**:provider × 能力(tools / reasoning / cache / structured output / 各模态),每格由真实探测或录制支撑,而非照抄文档。
- **漂移检测**:定期重录并与 cassette 字节级 diff;协议或行为变化即告警。依赖 #167 transport-level replay。
- **CLI**:`aimux probe` / `aimux replay` / `aimux diff`,沿用 aimux-cli。

## 7. 性能与体积基线

沿用 #197 §6.1 并升为硬门禁:

- **S1** 桌面 `.a` 切 LTO-off staticlib profile(只改 staticlib job;cdylib 保持 fat-LTO)。
- **S2** 所有发布产物体积门禁,**每个产物给具体阈值**(初期可取当前值 +10%)。
- **S3** `cargo-bloat` 审计;feature gating 只作可选小体积路径,默认全量。
- **P(新增)** 性能回归门禁:单请求开销、流式吞吐、RSS 增长;§4.3 的 IPC 开销纳入。
- **新增预算**:`aimux` CLI 单二进制体积。
- 约束不变:**瘦身不改变对外 API**;需要收窄公开 API 才能换到的收益另立提案。

## 8. 与 #166 代码缩减的关系

**不放弃,重新归类**。大部分项正是新方向的地基;C1 升级,少数项降级。

| #166 项 | 处理 | 在新架构中的角色 |
|---|---|---|
| A1–A5 | 保留,0.6 | 纯减法,降低维护面(A2 为绑定源码级 breaking,需写迁移说明) |
| B1 | 保留,0.6 最高优先 | L1 协议层的入口;解锁 #174 / #175 / #167 |
| B2–B8 | 保留,0.7 | L1 协议层本体 |
| B9 | 保留,降级为可选 | 单模态 provider 外壳,收益小 |
| C1 | **保留并升级** | 成为 §4 的 ops 协议;新增要求:传输无关、二进制帧、版本协商 |
| C2 | 保留,0.6 | 旧 ABI 转发 shim,让 B 轨可在绑定迁移前推进 |
| C3 | 保留 | 错误 JSON 即协议错误信封 |
| C4 | 保留,时机后移 | 8 种绑定全部迁移后 |
| D1–D7 | 保留 | 按原计划重写为 ops 协议的 FFI 薄封装 |
| D8 | 保留 | 类型镜像生成 |
| E1 | 保留,0.6 | #164 已合入,改为 master 上的独立清理 |
| #171 后半(补 116 个 provider) | 降级为机会性 | 广度不是目标;B 轨后加一行 registry 的成本很低,按需补 |

## 9. 阶段

| 版本 | 主题 | 内容 |
|---|---|---|
| **0.6** | 门禁 + 协议地基 + 纯减法 | S1 / S2 / P 门禁;A2–A5、E1、#185;B1 + C2;**ops 协议 schema**(op 表、帧格式、版本协商,先以文档 + 测试落地);**L0 passthrough**;RFC-0032 / 0033 入库 |
| **0.7** | L1 协议层 + auth | B2–B8、#174 / #175;L2 的 `Raw` / `provider_metadata` 补齐;S3;L2 数据模型调研 RFC |
| **0.8** | ops 协议落地 | C1 dispatch 同时落地 FFI 与 stdio CLI;D 轨绑定迁移、D8;stdio 开销进性能门禁 |
| **0.9** | 治理 | #167 transport replay;#170 漂移检测;能力矩阵;`aimux probe / replay / diff` |
| **1.0** | 冻结 | 冻结 ops 协议与 L0 / L1 契约;L2 独立版本化不随之冻结;C4 |

各阶段允许交叠;A 轨与 B1 / C2 无依赖。

## 10. 开放问题

1. **绑定用生成器还是手写?** ops 协议收敛后每种语言只剩 ~4 个对象,手写成本已低;类型镜像(D8)可考虑 JSON Schema → 各语言生成。
2. **二进制帧格式选型**:自定义长度前缀帧,还是采用现成格式(如 CBOR / MessagePack 承载 JSON 语义)。
3. **L2 选型**:见 §3 的调研 RFC。

## 11. 不在范围

- 本 RFC 不修改任何代码,只确立方向;各项实现由对应 issue / RFC 承载。
- 不重新评审 #166 各项的设计细节,只调整归类与时机。
- UDS / HTTP 传输、proxy 扩展包的实现:方向已定,待有真实需求时另立 RFC。

## 12. 全节点设计覆盖与新旧决策关系（2026-10-03 草案）

### 12.1 状态、适用顺序与独立评审边界

本增补是设计覆盖与验收规范，不是第二份路线图，也不调整版本日期。核验代码为 `master@72a37b5058ecd620d75dfe66bee34ef51f89294a`；#200 当前 head 为 `d8c3d15a9b84eaa176b64fe9c5f84d678498634d`，仍未合并且被要求修订。本文不把 PR 正文的 Accepted 状态等同于已生效代码或已合并规范。

#200 §0.4 / §0.5 已记录四项目标决定：单步运行时、本地 registry、完整依赖闭包的一次性 breaking 切换、所有语言宿主自定义回调后置。它们不是重新待选的方向；但 #200 仍须解决 RFC 同号、精确替代范围及正文回调矛盾。目标设计据这些记录编写，不要求先实施随后删除的旧方案。

以下规则避免两个相反要求同时指挥实现：

1. 已发布 master 的实际行为是现状证据，不因 RFC 改字而宣称已完成迁移。
2. §12.2 标出的冲突节点停止按旧架构启动新实现，目标验收使用对应专题 RFC；#200 修订并合并、专题设计获接受，是跨架构实现的共同前置门。
3. 若 #200 合并前目标决定有修改，只重新评审受影响的行和专题；不能自动恢复旧方案，也不能把设计草案当作运行时授权。
4. S1/S2/P 的测量规范、registry 只读维护报告、无依赖的机械清理可以独立评审；不以这些独立项为由提前实施完整切换。
5. 下列专题以文档编号和文件名标识，独立 PR 均从同一 master 建立，未合并的文件不使用相对链接冒充已存在依赖。合并后索引再加入链接。0037 没有在本文被分配；#200 必须自行取得无冲突编号。

专题目录：

- RFC-0033 `0033-registry-maintenance.md`：registry 只读同步、无凭证探测、#171 前半
- RFC-0038 `0038-size-performance-budgets.md`：S1/S2/S3/P 与全发布产物预算
- RFC-0039 `0039-ops-bindings-contract.md`：C/D 轨、统一 dispatch、FFI/stdio 契约和 cutover
- RFC-0040 `0040-provider-auth.md`：provider 工厂、B 轨、auth、L0 能力边界
- RFC-0041 `0041-replay-governance.md`：transport-leaf replay、治理 CLI、能力证据与生态跟踪
- RFC-0042 `0042-v4-type-boundaries.md`：V4 四层类型、#185、投影保真与类型生成语义

这些编号在发布前必须对 master 和所有开放 RFC PR 再次查重；如被占用，整批索引和相互引用一次性更新。

### 12.2 明确替代清单

| 本文或旧 roadmap 原要求 | 目标规范 | 处理与生效门 |
|---|---|---|
| §3 L2 可选 AI SDK / Open Responses / 自有；独立 `data_format_version` | #200 §0.3–0.5、RFC-0042 的 V4 四层边界 | 目标固定 V4；不重新开展架构三选一。旧调研节点改为覆盖/损失证据核验，不许静默删字段 |
| §8 B1 `Protocol` + `from_resolved` 和通用配置重建 | #200 D10–D16、D21、D29；RFC-0040 | 以工厂/私有 model config/显式注册扩展承接；不新增通用 `from_resolved` 兼容层 |
| §8 C2 旧构造 shim、§9 分期删旧 ABI；#166 一 minor 共存 | #200 Q3/D31；RFC-0039 | C2 作为被替代节点保留记录；完整依赖闭包一次切换，不新增转发/废弃别名，不承诺旧 ABI 共存 |
| §3 L0 auth/retry 统一下沉 | #200 D11/D14/D26；RFC-0040 | 认证在包内 headers/fetch 组合；operation 唯一通用 retry owner；下载不复用带凭证 provider 栈 |
| §6 / 旧 #167 `ProviderRecord` + protocol 重建 | #200 D4/D5/D24；RFC-0041 | 用 leaf `replay_fetch` / `replay_web_socket`；离线 miss 不发网，不恢复凭证或旧录制 |
| §4 ops/stdio、§9 0.6 L0 passthrough 与 #200 不新增能力范围 | RFC-0039 / RFC-0040 独立能力验收 | 保留为后续 roadmap 设计，不夹带到结构清理与 V4 cutover；具体验收通过并明确接受后再开放 |
| §4 九导出硬指标及固定 model-only 构造 | #200 provider/registry 句柄与 RFC-0039 | 保留单一 dispatch 的目标；按实际对象/能力生成 ABI，不为凑九个符号压缩语义 |
| §8 D8 从 serde/ts-rs 独立生成镜像 | #200 §0.5/D20；RFC-0042 / RFC-0039 | descriptor → manifest → codegen 唯一来源；schema 是输出，不是第二个手工真相 |
| roadmap #185 纯删 tracker → `ToolInput{Raw,Parsed}` | RFC-0042；#204 未合并实现提案 | V4 原始 String / core 解析 JSON 分层；是否保留内部累积器按显式接受的规范核验，不能把 #204 移动重写记为纯删除完成 |
| §9 1.0 冻结 L0/L1、L2 不冻结 | RFC-0039 / RFC-0042 与 §14 发布契约清单 | 对外契约逐项列版本与承诺；V4 依赖跟踪不等于允许同主版本任意破坏，单独的旧 L2 冻结口号不作为目标规则 |

本表针对目标设计替代旧实现方式，不删除定位目标：性能/体积、多语言行为一致、治理、私有协议保真、非 agent 产品边界继续保留。原 §1–§11 保留作为合并基线记录；冲突实现要求的裁决入口为本节，#200 修订应回链此表并更新对应旧段，不能只在新 RFC 内声明替代而遗留相反指令。

### 12.3 完整节点矩阵

状态只取：**已交付**（须合并证据）、**待实现**（设计有明确承载）、**目标替代**（旧方式不再启动）、**后置**（有再次开启条件）。节点设计齐备不等于实现或发布完成。

| 阶段 / 节点 | 状态；规范位置 | 依赖 | 可验证的关闭条件 |
|---|---|---|---|
| 基线 #164 / RFC-0031 请求管线 | 已交付；`docs/ai-sdk-request-pipeline.md`；本增补 §13.2 | 已合并 #164 | 已有 retry/timeout/abort 契约保留；目标差异另由 #200 记录，迁文不算重实现 |
| 基线 A1 | 已交付；#169、#166 修订清单 | #169 | 16 个脚本/依赖实际删除；仍保留的 cassette/converter/audit 脚本不误报删除 |
| 基线 iOS staticlib 修复 | 已交付；#196；RFC-0038（预算/测量/门禁章节） | #196 | 当前 strip/64 MiB 门存在，不把 32 MB 目标或全部 S2 误记完成 |
| 0.6 RFC-0031 入库位置 | 待实现；§13.2 | 文档迁移独立 PR | 来源与原 §1–3/14 语义保留；所有链接更新；历史详述可追溯 |
| 0.6 RFC-0032 缺失规范 | 目标替代；RFC-0040 §§2–8 | #200 一致性门 | 明确由0040承接七项修正的行为要求，不另造旧 from_resolved 文档 |
| 0.6 RFC-0033 入库 | 待实现；RFC-0033 §§3–8 | 独立报告规范接受 | 不阻塞合并的报告、离线 fixtures、安全约束完整 |
| 0.6 S1 | 待实现；RFC-0038 §5 | staticlib target 清单 | 仅 staticlib 用 LTO-off；cdylib profile 不变；逐目标 .a 预算验证 |
| 0.6 S2 | 待实现；RFC-0038 §§3–4、9 | 可复现测量/基线登记 | 每个真实 release artifact 唯一清单、阈值、报告；新增未登记产物不能放行 |
| 0.6 P | 待实现；RFC-0038 §7 | 稳定 runner 与 fixtures | 单请求/吞吐/RSS 门；噪声/重测/基线更新有审核证据 |
| 0.6 A2 ProviderName | 待实现；§13.1；#203 提案 | source consumer 清单 | Rust/各绑定/web 副本与生成入口退出；字符串路径、运行期列表、版本说明受测 |
| 0.6 A3 inventory | 待实现；§13.1；RFC-0004 | 历史来源固定链接 | 不丢研究结论/来源；标历史；registry 唯一现行来源；#203 仍不算完成 |
| 0.6 A4 web 类型副本 | 待实现；§13.1；RFC-0042 §§3–8 | 类型输出路径 | 所有 type imports 有去向；Wire 独有类型有唯一生成来源，构建不写源码 |
| 0.6 A5 Flutter 脚手架 | 待实现；§13.1 | iOS/Android 保留门 | 只移除桌面 example；force-link 符号验收随最终 ABI 调整且不取消 |
| 0.6 E1 prelude/run_operation | 待实现；§13.2 | #164，目标 runtime 分层 | 各模态重试/超时/录制/session 语义逐项回归 |
| 0.6 E1 abort/重试/下载清理 | 待实现；§13.2 | 取消与资源上限测试 | 不因少写代码丢失 body reader/poll 等取消；不重复提交付费 job |
| 0.6 E1 文档三层 | 待实现；§13.3 | 受影响链接清单 | docs/绑定/rfc/历史各有归属；编号碰撞显式登记；无未授权删除结论 |
| 0.6 #185 工具输入/tracker | 目标替代；RFC-0042 §§3–8 | #200 类型与 #204 核验 | raw/parsed 边界、截断/非法输入/重复 id 例验证；不把重写称纯删 |
| 0.6 #170 sync/probe | 待实现；RFC-0033 §§3–8 | 受审查输入/allowlist | 每周 diff/月探测只报告；无凭证无私网无重定向泄漏 |
| 0.6 #171 前半端点差异 | 待实现；RFC-0033 §§3–8 | #170 当前快照 | 历史27项逐条证据/决定/cassette；数字不冒充当前事实 |
| 0.6 B1 构造入口 | 目标替代；RFC-0040 §§2–8 | #200；descriptor/registry | 真 provider/model/namespace 身份分离；无公开配置重建 |
| 0.6 C2 转发 shim | 目标替代；RFC-0039 §§0.2、11 | 全闭包切换 | 无 shim；所有旧构造调用方在同一集成快照替换，符号审计通过 |
| 0.6 ops schema | 待实现；RFC-0039 §§2–8 | op 单一来源/错误类型 | op×handle/协商/帧/取消/资源上限反例完整；先规范后入口 |
| 0.6 L0 passthrough | 后置至独立能力接受；RFC-0040 §§2–8 | 工厂/传输安全/录制 | 字节保真、认证域/重试/重定向/取消全部通过；不混入 cleanup |
| 0.7 B2 wrappers/presets | 目标替代；RFC-0040 §§2–8 | B1 目标工厂 | 各厂商协议行为不被通用 profile 抹去；无 key/base_url_env 分开验收 |
| 0.7 B3 compat 数据 | 目标替代；RFC-0040 §§2–8 | descriptor/包内配置 | owned serde 输入或生成配置不成为第二真相；禁止跨包按名字猜协议 |
| 0.7 B4 Mistral | 待实现；RFC-0040 §§2–8 | 全闭包消费者迁移 | content array/tool_choice/finish/error 等真协议 fixtures 不丢 |
| 0.7 B5 xAI | 待实现；RFC-0040 §§2–8 | 共享 helper/消费者迁移 | 目标 Responses/共享 helper 的 usage/source/reasoning 与错误受测；旧 chat 入口撤销须明示，不伪称保留 |
| 0.7 B6 Responses 家族 | 待实现；RFC-0040 §§2–8 | headers/namespace/包内请求转换 | Azure/HF/Codex/xAI 特殊语义逐项回归；不恢复通用 body_overrides |
| 0.7 B7 Vertex | 待实现；RFC-0040 §§2–8 | region/auth/metadata | global/eu/us/区域 host、googleVertex namespace 与终止行为不丢 |
| 0.7 B8 小重复 | 待实现；RFC-0040 §§2–8 | header/error 单次 helper | 保留无 Content-Type 特例；错误上下文与 AWS 凭证边界受测 |
| 0.7 #174 auth L1 | 目标替代；RFC-0040 §§2–8 | 工厂/认证组合 | none/key/token/signature 边界、每请求求值、秘密不持久化 |
| 0.7 #175 auth L2 | 目标替代；RFC-0040 §§2–8 | #174 目标语义 | G1 保留既有包求值；新 G3 协调器后置，完整并发/失败/取消规范独立接受 |
| 0.7 Raw/provider_metadata | 待实现；RFC-0042 §§3–8 | 四层转换 | 每协议字段去向表、raw 开关、保真损失明确；不能静默丢弃 |
| 0.7 S3 bloat/features | 待实现；RFC-0038 §§4.4、6 | 默认构建基线 | 大小来源审计；可选 slim 不缩默认 API；全量与 slim 均受测 |
| 0.7 L2 数据模型调研 | 目标替代；RFC-0042 §§3–8 | #200 V4 决定 | 完成 V4 覆盖/损失/上游变化证据；不另做选型 RFC |
| 0.8 C1 dispatch/生成 ABI（替代九导出硬指标） | 待实现；RFC-0039 §§2–4 | 规范与全闭包 | 一份 dispatch，provider/registry/model 有类型句柄与生成 ABI；导出数量不作验收；旧符号仅按切换门删除 |
| 0.8 C3 error JSON | 待实现；RFC-0039 §6；#95 | 统一错误序列化 | Retry variant 与派生 retry_after_ms、不明错误/诊断脱敏测试 |
| 0.8 D1 Kotlin | 待实现；RFC-0039 §9.4 | Java artifact | 保留语言级 sealed 异常/Flow；无重复 JNA；依赖图/取消受测 |
| 0.8 D2 Go | 待实现；RFC-0039 §9.2 | dispatch/生成类型 | cgo 资源/回调生命周期/错误/流合同一致 |
| 0.8 D3 Java | 待实现；RFC-0039 §9.3 | dispatch/生成类型 | JNA/资源释放、异常与非空输入语义明确 |
| 0.8 D4 Swift | 待实现；RFC-0039 §9.5 | dispatch/生成类型 | AsyncThrowingStream 取消/早退/释放/终止一致 |
| 0.8 D5 Flutter | 待实现；RFC-0039 §9.6 | 生成头/loader/finalizer | iOS process loader、finalizer 与 force-link 全保留验收 |
| 0.8 D6 Node | 待实现；RFC-0039 §9.7 | async Rust dispatch | napi 保留非阻塞；runtime 重入/回压/取消测试；无新宿主注入 |
| 0.8 D7 Python | 待实现；RFC-0039 §9.8 | async Rust dispatch | GIL 不阻塞、异常跨界和 wheel 完整性；不以未验证 ctypes 替换为必需 |
| 0.8 C 直接使用面 | 待实现；RFC-0039 §9.1 | header/ABI | C 示例可覆盖管理操作、错误、session 与裸字节，不漏第八语言 |
| 0.8 D8 镜像生成 | 目标替代；RFC-0042 与 RFC-0039（ABI/ops/绑定/验收章节） | descriptor/manifest | 全8语言和 web 输出 check；nullable/optional/bytes/error golden 一致 |
| 0.8 stdio ops | 后置至独立入口接受；RFC-0039 §§8、11.4 | dispatch/安全帧/协商 | 握手、混合帧、背压、EOF/取消、stdout 不污染；不新增 daemon |
| 0.8 stdio P | 待实现；RFC-0038 §8 | stdio 入口 | 往返与200KB上下文基准入账后才能发布入口 |
| 0.9 #167 replay | 目标替代；RFC-0041 §§4–12 | leaf/schema3 | 真协议代码运行；miss 零网络；HTTP/WS/下载边界完整 |
| 0.9 #170 漂移检测扩展 | 待实现；RFC-0041 §10.4、RFC-0033 | 安全重录/审核凭证 | 原始字节差异保留、批准的归一化分开展示；不自动覆盖 cassette |
| 0.9 能力矩阵 | 待实现；RFC-0041 §10.3 | probe/record 证据 | 每格有范围/时间/版本/证据与 unknown，不把连通性当能力 |
| 0.9 #179 replay CLI | 待实现；RFC-0041 §§4–12 | 离线 replay contract | 模式/退出码/筛选与失败确定；live 操作须显式目标与独立授权 |
| 0.9 probe/diff CLI | 待实现；RFC-0041 §§4–12 | evidence/schema | 报告可复现；机器输出稳定；无秘密泄漏/隐式网络 |
| 小项 #180 cache §10 | 待实现/部分后置；RFC-0015/0025、0041 | 已实现 core 审计 | 真未完成项逐项证据；online attach 不虚构既有通道 |
| 小项 #181 session/debug | 待实现/部分后置；RFC-0024、0041 | session/operation scope | debug 消费既有 store/导出；业务缓存/宿主持久化职责不进入模型 |
| 1.0 C4 | 目标替代；RFC-0039 §§11.1–11.3；§14 | 全依赖闭包测试/发布物 | 一次性删除旧符号/旧测试须替代合同齐备，不等一个 minor 兼容期 |
| 1.0 冻结范围 | 目标替代；§14、RFC-0039/0042 | 公共契约台账 | wire/ABI/格式各自版本、变化规则、明确后置面；不虚称未交付能力稳定 |
| 1.0 发布判据 | 待实现；§14 | 全矩阵与证据包 | 每项 delivered/approved-deferred，S2/P 连续绿，错误/管线稳定周期 |
| 常设 #95 错误体系 | 持续跟踪；RFC-0031/0039/0042 | 每个错误相关变更 | code/category/cause/retry 语义 golden，不因日志变化泄密 |
| 横切 每周 registry/月 reachability | 待实现；RFC-0033 §§3–8 | 只读 scheduled reports | 失败只报 unknown/任务状态，不成为合并门 |
| 横切 每月上游规范 diff | 待实现；RFC-0041 §10.5 | 固定参照版本/证据 | AI SDK/pi-ai/Open Responses 版本差与处置；落后>2版本升 issue |
| 横切 每季新鲜度/机制评估 | 待实现；RFC-0041 §10.5 | changelog/录制范围 | 有日期证据和做/不做决定；新增机制只立独立 RFC |
| 降级 B9 单模态 shells | 后置；RFC-0040 §§2–8 | 实际维护收益/全量测试 | 不算1.0阻塞；单独选择后执行，保留无key/特殊URL行为 |
| 降级 #171 后半116 providers | 后置；RFC-0033 §§3–8 | 实际请求/协议/cassette | 历史数字不作吞吐目标；按需接入不阻塞1.0 |
| 不排期 UDS/named pipe/HTTP | 后置；本 RFC §4.2 | 真实宿主需求/威胁模型 | 独立 RFC 获接受前不新增监听面 |
| 不排期 aimux-proxy | 后置；本 RFC §5 | 明确调用方/协议损失证据 | 独立 crate/预算/安全 RFC；不纳默认构建或绑定 |
| 不做 agent/编排/网关业务/OAuth登录 | 不在范围；§2/#200 Q1 | 新产品决定才可重开 | 本批实现不含多步、计费租户、审批执行或登录流程 |
| 新宿主回调/新模态 | 后置；#200 Q4、RFC-0039/0042 | 独立需求/线程与安全契约 | 所有语言同范围，不以 Node/Python 桥接为例外 |

## 13. 机械清理与已实现规范的补充验收

### 13.1 A2–A5：删除对象与行为保留清单

本节补足原 RFC 没有展开的清理设计，不另开一份清理架构。#203（核验 head `3521d5d`）已提出这些改动，但尚未合并，因此下列均为验收要求，不是重复写实现 PR 的指令。

**A2**：provider 名称是字符串，不再生成固定枚举及各语言副本。删除前列出 Rust、Go、Java、Kotlin、Swift、Flutter、Node、Python 和 web 全部消费者；运行时 provider 列表仍由规范 registry/preset 输入导出，不能用静态补全列表代替。未知但显式注册的合法名称可解析；不存在的名称有可诊断错误。Node 的补全若保留，必须来自唯一生成链且不限制运行时可接受字符串。绑定源码 breaking 要在 CHANGELOG 写清，不能说 ABI 未改就称完全兼容。移除旧 generator 的 CI 调用和文档引用；新 codegen 稳定后再移除其替代期输出，不能并存两种权威类型。

**A3**：`provider-inventory/` 是历史调查，不是运行配置。RFC-0004 标为 historical，资料必须可由固定 commit+path 定位，不能只链接浮动 master 已删除路径。纯生成的原始大文件可以用固定历史引用替代；人工结论保留。最终选择以 #203 接受的具体保留/删除清单为准，本批不创建另一份 inventory 或复制旧数据到新权威目录。

**A4**：web 的共享类型从唯一生成输出导入，专有 Wire 类型保持单一结构定义并生成镜像。每条 import 都需要类型检查；不能只比较文件是否相同。普通 test/build 不应改写受跟踪文件；生成是显式命令，`--check` 只比较并失败。PR #203 的 ts-rs 过渡产物是当前实现提案，#200 目标 codegen 接管时整体替换，不能将临时脚本列为永久架构。

**A5**：删除范围仅 Flutter example 的 Linux/macOS/Windows runner。保留 iOS/Android 示例、平台打包与真正的发布支持。iOS force-link 检查不能随旧 `aimux_openai_new` 消失而移除；最终 ABI 的代表性生成导出符号取代原符号，验证 `DynamicLibrary.process()`、静态链接和实际调用，而非只验证文件存在。

共同测试：按最终 tree 做 tracked-file 状态检查、全仓旧引用搜索、相关语言类型检查和示例构建。全局出现历史链接/说明不算失败，但可执行路径出现旧符号必须失败。删除 cassette 不属于上述任何清理项。

### 13.2 E1：请求管线整理不能改变操作语义

RFC-0031 已以 `docs/ai-sdk-request-pipeline.md` 存在且 #164 已合并。迁入 `rfc/0031-ai-sdk-request-pipeline.md` 时保留原始作者记录、参考版本、§1–§3 设计和 §14 结论；实现详述/完成清单放固定历史链接或保留补充文档。正式迁移必须先列原章节映射，不能用本增补替代已经完成的管线规范。

对 `run_operation`、prelude、abort、retry、body-reader 合并逐项要求：

- 一个用户操作唯一通用 retry owner。认证求值和 HTTP helper 不套第二个通用重试；composite 每个 child 的预算不因外层失败整组重跑。
- 取消覆盖等待重试、网络连接、流式 body、下载和 provider submit/poll。只因最外层 drop future 可取消部分 await，不足以删除所有内层检查；必须证实背景任务、channel、文件和传输在取消后都终止。
- 每个 operation 的 scope/session/span 建立一次，每个 attempt/child 有正确关联；telemetry 关闭不影响 recording/调用身份。共用实现不让并发 child 覆盖彼此 scope。
- `StreamTextResult::text()` 的重用必须保留消费时机、错误传播、结束状态和使用量，不只比较最终文本。
- 合并 `PreparedRetries`/`RetryConfig` 需固定单位、上限、溢出、Retry-After 优先级与 abort 语义；时间测试使用受控时钟，不睡真实退避时间。
- 下载上限不得把 2 GiB 全读入 Vec 后再检查。已知/未知长度、分块、解压后字节、超限取消/资源释放都必须在迭代读中验收；最终上限由下载合同设定，不为瘦身取消守卫。
- fal/luma/BFL/revai/gladia 的 submit+poll 是行为风险，不因 cleanup 顺带统一为再次 submit。已获 job ID 后的轮询失败只重试可重试轮询；若 provider 无幂等保证，不能自动重试提交。案例失败不得记作机械删除通过。

本期仅细化验收，不修改运行时代码。以上提取应在 #200 runtime 分层相应位置完成；与其改变同一职责的 E1 不能独立抢先引入另一套 runtime。纯文档迁移、已证明无行为变化的小整理可单独 PR。

### 13.3 文档三层、历史与索引

1. `docs/` 保留面向使用者的 API、guides 和 benchmarks；各绑定用法到 `bindings/<lang>/README.md`。公共文档及绑定 README 使用英文，RFC 使用中文；本次搬运不假称所有历史文档已翻译。
2. `rfc/README.md` 建立 number/title/status/source 索引；0005/0027 既有碰撞标出完整 filename，不重新编号破坏链接；新的 RFC 发布前查重。Implemented 必须附合并证据，Superseded 必须附替代章节，Draft 不等于未使用的历史。
3. `docs/internal/`、`docs/quality-audit/`、`docs/plan/` 的历史结论可归档；纯构建 log/lcov 不作为规范。先给每个搬移路径目标或固定 commit 链接，再删除原位置；`archive/README` 指向历史，不能留下指向已删文件的目录树。
4. `docs/api/gaps.md` 的仍开放问题只有在对应 issue 真实存在并回链后才移除，不在文档清理时自动关闭问题或创建未经审查的重复 issue。
5. 链接检查包括 root README、RFC、bindings、生成 doc、issue 中已知规范路径；外部已有引用无法批量改写时，在旧路径留下短指向页或保留可追溯固定历史说明。目标不提供运行时兼容层不等于禁止文档重定向。

验收报告列出每个文件的 move/retain/history-only/delete-generated-artifact 决定。发布 docs-only 变更前至少通过 Markdown 链接解析、重复编号审计和 `git diff --check`；涉及 generator 或 build 的实现再跑对应完整检查，不将文档检查描述成运行时测试通过。

## 14. 1.0 接受、冻结与整批发布门

### 14.1 公开契约清单

1.0 冻结必须对应已经交付的面，而不是给抽象层名字盖章。发布清单至少列：Rust 公共模型/操作 API 的 semver、生成 C ABI 版本、ops 消息/帧与 op manifest 版本、各绑定 API/package 版本、录制 schema3 与导出/CLI 机器输出版本、provider V4 参照版本。

- 同一 wire 主版本不改变字段含义/必需性/错误类别；未知可选字段的容忍规则与未知 variant 的错误规则由 RFC-0039/0042 明定。删 op、删 variant、换字节编码或必需字段变化需要新的不兼容版本。
- 外部 provider 的协议改变不能保证上游永远不变；aimux 自身的字段转换、认证边界、取消和错误合同需稳定。对上游变更发布 conformance 证据和对应版本，不借“持续跟进”静默改同版 wire。
- 录制 schema 与 wire 版本分开；schema3 不读旧录制的决定须在发布说明明确。cassette 原始测试资产只增不减；录制工具的新格式不授权删旧测试库。
- 尚未通过能力门的 L0 public passthrough、stdio ops 不得列为已稳定面；UDS/HTTP/proxy/宿主自定义回调/新模态明确不在1.0保证范围。将来纳入需独立接受与预算。

### 14.2 关闭矩阵与稳定周期

完整实现发布前，矩阵每个待实现节点附最终 commit、合并 PR、实际运行的测试与产物；目标替代节点附接受的替代决定和对应实现证据；后置节点附理由与开启条件。后置不能被计作完成，也不能用勾选总数掩盖核心路径缺失。B9/#171 后半明确不阻塞1.0；其它延期若影响承诺能力，必须调整发布范围并显式接受。

“错误/管线稳定一个周期”具体为：使用拟发布完整依赖闭包的 release candidate，覆盖所有声明支持的 provider 协议/模态与8语言，至少一个公开候选验证周期（建议14个连续自然日）；候选开始/结束时间、样本、阻塞问题、修复提交都有记录。任何公共错误/取消/重试/序列化合同的修复使受影响合同重新进入验证，不把早期旧 SHA 的绿灯计入新候选。14日是本增补提案，不伪称旧 roadmap 已确定数字；维护者接受此门后执行。

S2/P 连续绿的唯一计数与重置规则由 RFC-0038 §11 定义；本节不再增加另一份同 SHA 计数要求。最终发布候选必须具有对应完整 artifact 清单与全部基准矩阵的有效结果；每日窗口能否复用先前 SHA 的证据，按该节的合同/环境/预算变化规则判断。环境失效/缺测为 unknown，不是绿；不能每次回归后自动移动基线。错误/管线合同的稳定周期与 S2/P 可以并行积累，但两者各自都须满足。registry scheduled 报告始终只通知，不混成阻断合并的硬门。

### 14.3 文档批次的完成门

本批是 RFC 文档交付，不执行上述运行时迁移。提交独立 Draft PR 前必须满足：

- 所有 §12.3 节点都有设计承载、依赖和可验证条件；既有完整规范只补缺口，不再创造同题 RFC
- #200 已记录决定和需要修订的未合并状态都写清；各文档无旧 shim、ProviderRecord 重建、全语言回调范围等互相矛盾指令
- 对跨主题 contract 做独立审查：descriptor所有者、op/type/录制版本、身份、auth、retry、取消、错误、字节/脱敏及资源上限
- 每个专题从同一最新 master 独立分支，说明先后依赖；不把另一个未合并 PR 的文件静默叠入，也不把本地存在当成 master 已有
- 核对编号和相对链接；运行文档检查并如实报告未跑运行时测试；所有 Draft PR 只含本主题规范，不夹带代码或替代 roadmap

每个专题可独立评审接受，但实现应遵守完整依赖闭包的切换门。此批设计覆盖通过不是 #200 自动批准、运行时实现完成或1.0可发布。
