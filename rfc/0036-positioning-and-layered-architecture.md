# RFC-0036: 定位与分层架构——provider 接入与治理运行时

> **Status**: Draft
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
- **L2 独立版本化**(`data_format_version`),不随 1.0 冻结。L2 的选型已定为 AI SDK V4 形态,实施设计见 docs/aisdk-architecture-alignment.md,调研 RFC 撤销（见 [docs/aisdk-architecture-alignment.md §0.7](../docs/aisdk-architecture-alignment.md#07-与-roadmap--rfc-0036-既有承诺的关系)）;原定评估指标保留作为基线升级时的核对项:
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
