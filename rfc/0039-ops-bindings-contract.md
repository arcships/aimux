# RFC-0039：ops、FFI 与八种语言绑定契约

> **状态**：Draft；以 PR [#200](https://github.com/arcships/aimux/pull/200) §0.4 的全链路 V4、一次性 breaking 切换和全语言宿主回调后置为设计目标，不表示 #200 已合并或本 RFC 已被接受。
>
> **日期**：2026-10-03
>
> **事实基线**：`origin/master@72a37b5058ecd620d75dfe66bee34ef51f89294a`；#200 审阅版本 `d8c3d15a9b84eaa176b64fe9c5f84d678498634d`。ROADMAP 取自该 master，不把未合并的 roadmap 状态修订当作已生效路线。
>
> **范围**：#166 C1–C4、D1–D8、ops schema、stdio 与 1.0 契约冻结；补足 #200 的跨语言执行、资源、交付与验收规则。
>
> **相关**：[RFC-0036](0036-positioning-and-layered-architecture.md)、[#166](https://github.com/arcships/aimux/issues/166)、[#95](https://github.com/arcships/aimux/issues/95)、[RFC-0035](0035-host-side-tool-call-repair.md)。并行设计 RFC-0038 负责性能与体积，RFC-0040 负责 provider/registry/auth/L0，RFC-0041 负责录制回放与治理，RFC-0042 负责类型、消息分层与唯一生成链；并行稿件在各自合并前以编号引用。

## 0. 决策与旧路线的关系

### 0.1 本 RFC 的确定决策

1. 同一份操作描述、Wire DTO 和 Rust 异步分发驱动 C ABI、Node、Python，以及未来 stdio；不让语言绑定自行实现鉴权、重试、provider 选择、消息转换或 replay。
2. 按 #200 的两级 provider → model 对象模型切换。`model_new({provider,model,...})` 不作为新的规范入口，不保留旧 one-shot 构造器或 `from_resolved` 转发链。
3. op 是传输无关契约，C 导出数量不是目标。通用 call/stream 与少量必要的生命周期、二进制输入和快速轮询原语组成 ABI；生成的具名便利函数只描述新 API，不保留旧符号。
4. 八种接口为 C、Go、Java、Kotlin、Swift、Flutter/Dart、Node、Python；C++ 示例属于 C ABI 的 RAII 用法，不计作第九套实现。
5. C、Go、Java、Kotlin、Swift、Flutter 走同步 C ABI；Node 保留 napi-rs、Python 保留 PyO3，直接调用共享异步 dispatch，禁止经同步 C ABI 嵌套 `block_on`。
6. provider/model/fetch/token/telemetry 的任意宿主控制流回调在**所有八种接口**后置。结果流的搬运 callback、取消通知和 RFC-0035 已有宿主侧修复能力仍在范围内。
7. D8 是切换前提，不是最后删除手写类型之后的补救。descriptor → manifest → codegen 是唯一生成链，JSON Schema 是其产物，不能再养一条 ts-rs 或手写 Schema 真相链。
8. 当前切换只交付已有能力的清理、V4 对齐和缺陷修复。stdio 服务、新的一次性二进制上传接口、任意 native passthrough、新模态均需单独通过后续范围门禁。
9. 一次性替换整个依赖闭包；不发布旧/新 ABI 并存的 minor，不添加 deprecated alias、配置归一化 shim 或旧录制读取路径。

### 0.2 精确替代清单

下表的 `superseded-pending-#200` 表示目标已在本设计中被替换，但必须随 #200 的编号、状态、supersession 和 ROADMAP 决策一同确认，不能把原 issue checkbox 提前勾成已实现。

| 旧节点或承诺 | 处理 | 新的实施要求 |
|---|---|---|
| ROADMAP 0.6 `B1 + C2`、依赖图“C2 shim 是 B4/B5 前置” | superseded-pending-#200 | provider、FFI、绑定及工具在集成分支同批替换；原直连调用点仍须逐一消除，但不通过 shim 解锁 |
| #166 C1“109 → 9 exports”、一份 `model(spec)`、旧导出共存 | amended / superseded-pending-#200 | 保留单一 dispatch 与穷举表；撤销固定九符号与 one-shot 模型工厂；采用 provider/registry 方法分辨和本 RFC ABI |
| #166 C2“40 构造器符号不变并转发” | superseded-pending-#200 | 删除旧构造器；新的错误测试使用真实新入口，不能为了保留旧测试保留 NULL-key 构造器 |
| #166 C3“字段 getter 缩减、派生 retry hint” | amended | 保留 code/message/json 三类只读错误入口及 free；其余 payload 通过规范信封；派生 retry hint 不丢失 |
| #166 C4“八绑定迁完一个 minor 后删旧符号”及 ROADMAP 1.0 的延迟 C4 | superseded-pending-#200 | 八绑定与工具迁移、删除旧符号是同一次合并的必要条件；1.0 不再替先前版本补删 |
| #166 D1、D2–D7 逐语言独立发布 | amended | 可以分工和内部评审，发布必须来自同一全绿集成 SHA |
| #166 D5 先做旧头文件 ffigen 过渡版 | superseded-pending-#200 | 直接生成新头文件对应声明，不花一个发布周期维护旧 ABI 声明 |
| #166 D7 ctypes 原型 → PyO3 fallback | superseded-pending-#200 | 选定薄 PyO3；消除 GIL 阻塞与重复解析，不引入第二套 ctypes wheel/线程方案 |
| #166 D8 serde/ts-rs → Schema → 类型、“最后一项” | superseded-pending-#200 | RFC-0042 的规范类型/Wire DTO + descriptor → manifest → codegen；Schema 与语言代码同批生成 |
| RFC-0036 §4/§9、ROADMAP 0.8“FFI 与 stdio 同时实现” | amended | 一套协议保留；当前结构切换先完成 FFI/原生绑定。stdio 为后续独立能力里程碑，不混入 #200 清理范围 |
| ROADMAP“瘦身不改变对外 API” | 保留在纯性能工作；收窄适用范围 | RFC-0038 不能以瘦身之名删 API；本 RFC 与 #200 是明确声明的 breaking 结构替换 |
| RFC-0036 §3/§9“L2 独立版本化、不随 1.0 冻结” | amended-pending-#200 | 采用 V4 四层数据模型；ops/ABI 与录制 schema 分别管理；V4 类型兼容性归 ops 版本/能力协商，type revision 只记入 manifest；不新增独立 data_format_version 或保留旧 L2 投影实现 |
| #200 §6.2/§6.3 的具名 C 函数与结构化 getter 示例 | 本 RFC 补充并收敛 | 具名 API 由同一 op 表生成；新 ABI 的 payload getter 统一为 error JSON；不得另写第二份分发逻辑 |
| #200 §6.4 Node/Python fetch/model/telemetry callback 文字 | 以其 §0.4 Q4、范围原则为准 | 全语言后置；本 RFC 不恢复这部分相互冲突的旧段落 |

#200 目前与已存在的 RFC-0036 编号冲突；本 RFC 只引用 PR 及其章节，不假定重编号文件已存在。其编号修正与架构接受是两个独立门禁。

## 1. 源码基线与要保留的能力

### 1.1 已核对的实现

| 位置 | 事实及影响 |
|---|---|
| `aimux-ffi/src/lib.rs::HandleEntry`、`NEXT_HANDLE`、`ffi_block_on` | 当前是单调 `u64` + `HashMap` + typed enum；model/provider 八模态、Files、TranscriptionSession、Abort 在同一表；已有线程局部重入保护 |
| 同文件 `aimux_error_*`、`AiMuxFfiError` | 错误分为 AiMux、Recording、FFI；目前有 owned error 指针及众多 getter，不能只保留 AiMuxError 而丢记录器/桥接错误 |
| 同文件 `aimux_trace_*`、`aimux_session_*`、`aimux_recording_*` | trace、聚合 session、录制均是现有公开能力；删导出必须保留可达的新操作 |
| 同文件 `aimux_router_new`、`aimux_moa_new` | composite 验证全部 child 句柄并克隆 `Arc`；不能把无效 child 静默过滤掉 |
| `aimux-ffi/src/transcription_session.rs` | 音频输入 64 项、输出 256 项；push 阻塞；drop 先移除句柄再 join，join 上限 5 秒；目前输出满时 abort 可能丢终结错误 |
| `aimux-core/src/error.rs::retry_after_hint` | hint 不是 serde 字段；支持 `retry-after-ms`、秒值和 HTTP-date；超时/取消不自动等于可重试 |
| `bindings/node/src/lib.rs` | napi tokio 驱动流，当前 64 项通道，已有 AbortBridge；不能改为在 JS 线程同步调用 FFI |
| `bindings/python/src/lib.rs` | 已用 PyO3；`__next__` 释放 GIL，但 `generate_text` 等阻塞入口目前直接 `block_on`，切换时必须覆盖全部入口 |
| `bindings/java/build.gradle.kts`、`bindings/kotlin/build.gradle.kts` | Java 目标为 Java 8；Kotlin 当前独立 JNA，JVM 17；不能为删 Builder 默认改成 Java records |
| `bindings/go/aimux.go`、Swift `Aimux.swift`、Flutter `aimux.dart` | 分别已有 cgo handle/Cancel、AsyncThrowingStream、NativeFinalizer/动态库加载；要迁移线程和生命周期，不只替换函数名 |
| `scripts/gen_ts_types.py`、`contract-tests/fixtures`、`.github/workflows/ci.yml` | 当前生成门禁以 TS 为主；各语言有合同测试；新生成链必须接管完整文件集与 stale-file 检查 |

基线中的队列终结缺陷、Python 阻塞以及 Swift 异步包装的实际执行方式是待验收项目，不宣称当前代码已经满足后文契约。本 RFC 只完成设计与源码核对，不声称运行了实现测试。

### 1.2 能力保留清单

除 #200 明确撤销的旧配置、旧 API 与旧录制兼容承诺外，必须保留：八模态调用、文件上传、provider discovery/catalogue、OpenAI 输出转换、router/MoA、实时转写 push/pull、abort、日志/代理、现有 Rust 认证实现、trace/审计、session 查询、ring/JSONL 录制及显式 flush 错误、mock/live replay 和 tool-call repair。

新增对外能力不能借“生成器支持一个 variant”自动进入发行版。描述类型完整不等于多步引擎、工具执行或审批运行时已经实现。

## 2. 分发与类型归属

### 2.1 单一运行路径

```text
C / Go / Java / Kotlin / Swift / Flutter ─→ C adapter ─┐
Node napi / Python PyO3 ─────────────────────────────┤
未来 stdio adapter ─────────────────────────────────┤
                                                    ↓
                                  typed dispatch + HandleRegistry
                                                    ↓
                aimux 操作 / provider 注册条目 / devtools /纯数据工具
```

`dispatch` 不拥有第二份 provider 表或 retry 策略。它负责 Wire DTO 验证、句柄解析、调用 Rust 操作、事件降为 Wire DTO、错误分类。provider/settings/命名方法由 RFC-0040 负责；消息类型由 RFC-0042 负责；录制完整性和 replay 匹配由 RFC-0041 负责。

Rust 共享接口应返回 Future/Stream 和 typed result。C adapter 独自调用受保护的同步运行时；Node/Python adapter 按各自运行时契约 await。禁止把 `aimux_ffi::dispatch` 实现成“共享一个会 block_on 的 C 函数”。共享模块可作为独立内部 crate 或 rlib 模块，依赖方向不得令规范类型依赖 FFI。

### 2.2 操作描述

manifest 的每条 op 至少包含：

```text
name, sinceOpsVersion, mode(call|stream|binaryPush|poll), scope(current|future),
allowedHandleKinds, requestType, resultType, eventType?, terminalType?,
runtimeHandleFields, requiredCapabilities, cancellation, sideEffects,
resultOwnership, requestBytePolicy, retryOwner, sourceSymbolInventory
```

`requestType` 等引用 RFC-0042 的稳定 type ID。不得在 op 表中复制同名类型定义。所有 op 名精确匹配且大小写敏感；无 alias；未知字段在控制对象中拒绝，providerOptions 和 JSON 值按其开放字典规则保留。

## 3. 句柄与完整 op × handle 规则

### 3.1 句柄种类

`0` 仅表示进程/连接的控制作用域 `G`，不是“自动猜一个 provider”。非零句柄至少具有以下互斥 kind：

- `Provider`：叶子注册条目，保留扩展、source 与 descriptor
- `Registry`：显式 registry；不能擦除成只有 Provider trait 的对象
- `Language`、`Embedding`、`Image`、`Speech`、`Transcription`、`Reranking`、`Video`、`Search`
- `Files`、`Operation`、`Abort`
- `Fetch`、`WebSocket`、`Download`、`CredentialResolver`、`Middleware`：只指向 Rust 内建能力
- `TraceStore`、`Recorder`、`SessionStore`、`ReplaySession`

`Operation` 有 `subkind`、`state` 和 `capabilities`；当前外部可创建的双向 Operation 为实时转写。未来不能因为 kind 相同而向文本流写音频。Trace middleware 与 trace store 分开，聚合 session 字符串与实时转写 Operation 也分开。

### 3.2 规范合法矩阵

下表列出**全部允许组合**。除 `handle.drop` 的显式规则外，任何未列出的 op × kind 组合都返回 `InvalidHandle`，并携带 expected/actual；没有通配 fallback。每行列出多 op 时，生成器必须展开为独立条目和测试。各模态缩写只在本表使用。

| op | 允许的主 handle | 参数中的其他句柄/约束 | 模式与结果 |
|---|---|---|---|
| `runtime.describe` | G | 无凭证 manifest | call → 版本、能力和描述摘要 |
| `runtime.configure` | G | 仅日志、启动期 proxy、现有运行时默认值；未知 key 拒绝 | call → 生效配置摘要；已冻结项变更返回错误 |
| `provider.create`, `provider.default` | G | package descriptor；settings 中 runtime slot 分别校验 Fetch/WS/CredentialResolver；不接受 callback JSON | call → Provider |
| `registry.create` | G | entries 引用 Provider；拒绝重复 key | call → Registry |
| `registry.provider` | Registry | key | call → Provider，保留注册 source/扩展 |
| `provider.model` | Provider | method、modelId；默认方法来自 descriptor | call → 精确模态 Model；无此方法返回 UnsupportedFunctionality |
| `provider.invoke` | Provider | method 必须登记；当前只开放已有 discovery/厂商能力 | call → descriptor 规定结果；非任意方法 RPC |
| `provider.wrap` | Provider | 内建 Middleware；逐能力检查 | call → Provider，保留可用扩展/source |
| `provider.custom` | G | 显式现有模型映射/默认模型/内建包装能力 | call → Provider；禁止宿主函数对象 |
| `model.resolve` | Registry | modelRef 的 namespace/key/method/modelId | call → Model；不得去 Vercel Gateway |
| `model.info` | 所有八类 Model | 无 | call → provider/modelId/modality、能力与 source 摘要；不返回 settings/key |
| `model.wrap` | 所有八类 Model | 类型相容的内建 Middleware | call → 同模态 Model |
| `composite.router` | G | 非空 Language children，逐 child retry 预算与显式策略 | call → Language；clone 全部引用 |
| `composite.moa` | G | Language references 与必需 aggregator | call → Language；不把 child 列表序列化成可重建身份 |
| `text.generate`, `text.generateObject`, `text.consume`, `text.generateOpenAI` | Language | Abort 可选；操作 options、registry/modelRef 在 DTO lift 时解析 | call → 对应结果 |
| `text.stream`, `text.streamOpenAI` | Language | Abort 可选 | stream → 对应 data events + 唯一 terminal |
| `embedding.embed`, `embedding.embedMany` | Embedding | Abort 可选；多输入不得只处理第一项 | call → embedding 结果 |
| `image.generate` | Image | Abort 可选 | call → image 结果 |
| `speech.generate` | Speech | Abort 可选 | call → speech 结果 |
| `transcription.generate` | Transcription | Abort 可选；一次性字节采用 V4 wire base64 | call → transcription 结果 |
| `reranking.rerank` | Reranking | Abort 可选 | call → reranking 结果 |
| `video.generate` | Video | Abort 可选；提交/轮询重试边界不在 adapter 实现 | call → video 结果 |
| `search.search` | Search | Abort 可选 | call → Search 扩展结果 |
| `provider.files` | Provider | descriptor 标明已有 files 能力 | call → Files |
| `files.upload` | Files | 一次性数据沿用规范 base64 DTO；不自动重试全上传 | call → file 结果 |
| `transcription.start` | Transcription | 必须具备 doStream；Abort 可选 | call → Operation(transcription)；连接错误通过 operation 终结交付 |
| `operation.push` | Operation(transcription) | input-open；原始 bytes | binaryPush → 已接收；不是 provider 已处理确认 |
| `operation.inputDone` | Operation(transcription) | 开始关闭输入；幂等 | call → 空结果 |
| `operation.next` | Operation(transcription) | 单消费者；timeoutMs | poll → part / ended / timeout；终结错误见 §5 |
| `operation.status` | Operation | 无 | call → 状态及保存的 terminal 摘要 |
| `operation.cancel` | Operation | 无 | call → 首次/已请求/已终结状态 |
| `abort.create` | G | 无 | call → Abort |
| `abort.signal` | Abort | 无；可跨线程、幂等 | call → 空结果 |
| `transport.create` | G | kind=fetch/ws/download，仅内建实现/已存在设置 | call → 相应 typed 能力 |
| `credential.create` | G | 仅 RFC-0040 已有内建 resolver | call → CredentialResolver；不导出秘密 |
| `middleware.create` | G | 内建能力；trace variant 需 TraceStore | call → Middleware |
| `trace.create` | G | 容量、审计模式 | call → TraceStore |
| `trace.aggregate`, `trace.sessionChain`, `trace.sessionTrajectory`, `trace.export`, `trace.clear` | TraceStore | filters/sessionId 是数据，不是句柄 | call → 各自查询/清理结果 |
| `recorder.create` | G | ring/JSONL 与明确 capture policy | call → Recorder |
| `recorder.attach`, `recorder.stop`, `recorder.flush`, `recorder.stats`, `recorder.export` | Recorder | attach 为显式 runtime 配置；flush 报告实际持久化失败 | call → 相应结果；drop 不冒充 flush |
| `sessionStore.create` | G | 明确 infer 开关 | call → SessionStore |
| `sessionStore.attach`, `sessionStore.list`, `sessionStore.calls` | SessionStore | sessionId 为字符串 | call → 结果；未知 session 保留各查询既有语义 |
| `replay.create` | G | RFC-0041 schema 与规则 | call → ReplaySession |
| `replay.capabilities` | ReplaySession | 无 | call → 内建 Fetch/WS/Download 等能力句柄 |
| `replay.inspect`, `replay.dryRun` | ReplaySession | 无副作用 | call → 验证/计划 |
| `replay.live` | ReplaySession | 明确 Registry/Model 目标及当前凭证环境 | call → operation 结果；禁止恢复旧配置 |
| `catalogue.fetch` | G | 已有 source 规则与下载守卫 | call → catalogue DTO |
| `toolRepair.context`, `toolRepair.apply`, `toolRepair.applyResult` | G | 纯 DTO，无宿主 callback 句柄 | call → repair DTO/结果 |
| `output.toOpenAI` | Language | metadata selector 明确；不猜 openai namespace | call → 输出 DTO |
| `auth.codexRefresh` | G | 既有 refresh 输入；不记录 token、不自动重试 | call → token 结果；不得进入通用日志 |
| `handle.drop` | 所有非零 kind；0/未知值也可 | 无 | 幂等；先移除 registry，再取消/回收 |

这里的 provider/transport/devtools 生命周期 op 是 #200 对现有行为的显式对象化，不授权本期实现 descriptor 中尚未具备的 skills、Evaluation、Batch、SpeechTranslation 等能力。方法列在 descriptor 中但不属于 current capability 集时，返回 `UnsupportedFunctionality`，不能返回伪造的成功空值。

Rust runtime 的字符串模型入口先通过显式 registry 解析成模型再进入相应行；因此 C/stdio 的主 handle 合法矩阵不会因 `model` 参数另一种写法而变得含糊。

### 3.3 生命周期、并发与身份

- ID 由进程全局 allocator 从 1 单调递增，不回绕、不复用；分配耗尽失败，不覆盖旧条目。JSON 中所有句柄、requestId、seq 用十进制字符串表示，避免 JS 的 53 位精度限制；C 数值为 `uint64_t`。
- 全局 registry 条目同时记录 owner；Node 使用当前 Env、Python 使用当前 interpreter、stdio 使用连接 ID 作为 owner，由 adapter 向私有 dispatch 传入不可伪造的 owner context。公开 C primitives 没有 context 参数，因此明确采用单一 C-process owner，所有 C-ABI 绑定在同一进程共享该 owner，不宣称它们彼此隔离。全局 ID 不碰撞，lookup 同时验证 owner；外来 owner 一律 InvalidHandle，不泄露其他 owner 的对象详情。句柄不得跨 Node realm、Python interpreter、stdio 连接或 C/native adapter 边界裸传；显式跨 owner 转移不在本期能力内。
- lookup 在锁内验证并 clone 强引用，随后释放锁再 await/调用 callback。禁止持 registry 锁执行 I/O、join、序列化或调用宿主代码。
- drop 使后续 lookup 失败，已成功取得强引用的一次性调用可继续；取消进行中的请求须显式 abort。Operation drop 则取消其任务；两个语义不能混淆。
- Provider/Registry/Model/Files/composite 持有依赖的强引用。用户先 drop provider、registry 或 child handle，已创建的 model/composite 仍可工作。
- Runtime 能力注入在构造时获得强引用；resolver 的关闭或 provider 重建规则由其真实生命周期定义，不以悬空整数替代。
- Operation 单一 reader，允许另一个线程 push 和一个线程 cancel；第二个 reader 返回 `ConcurrentConsumer`，不隐式分流。inputDone 与 push 通过 admission 序号线性化：关闭前已接纳的 push 完成，关闭后拒绝。
- wrapper 的 Close/Dispose 幂等且原子取走句柄；finalizer 是防漏手段，不是 flush 或正常终结协议。

## 4. C ABI：所有权和执行边界

### 4.1 规范原语

接口布局由 manifest 生成，以下是冻结候选而非当前已存在符号：

```c
uint32_t aimux_abi_version(void);
/* 版本 = (major << 16) | minor；新契约 major=1，与无版本旧 ABI 不兼容 */
aimux_error_t *aimux_manifest_json(char **out_json);
aimux_error_t *aimux_call(uint64_t handle, const char *op,
                         const char *params_json, uint64_t abort, char **out_json);
aimux_error_t *aimux_stream(uint64_t handle, const char *op,
                           const char *params_json, uint64_t abort,
                           aimux_event_cb on_event, void *context);
aimux_error_t *aimux_operation_push(uint64_t operation,
                                   const uint8_t *data, size_t length);
aimux_error_t *aimux_operation_next_part(uint64_t operation, int64_t timeout_ms,
                                        char **out_part, int32_t *out_state);
void aimux_abort(uint64_t abort);
void aimux_drop(uint64_t handle);
void aimux_free_string(char *string);
void aimux_error_free(aimux_error_t *error);
int32_t aimux_error_code(const aimux_error_t *error);
char *aimux_error_message(const aimux_error_t *error);
char *aimux_error_json(const aimux_error_t *error);
```

结果流 callback 为 `void (*aimux_event_cb)(const char *frame_json, void *context)`。不新增反向 fetch/token/model callback。provider.create/model/registry 等具名便利函数由 op descriptor 生成在头文件或语言 facade 中，所有逻辑仍调用上述 primitives；它们是新 API 的代码生成，不能宣称旧 ABI 转发兼容。

`aimux_abort` 仅触发 signal，不 block_on，允许在结果 callback 中调用；需要报告无效句柄时调用 `abort.signal`。`aimux_drop` 和 `aimux_free_string` 对 0/NULL 安全。无效 owned 指针、double free、未终止的 C 字符串仍属 C 调用方违反内存契约，不能承诺运行时捕获。

### 4.2 输入输出规则

- JSON 字符串为 NUL 结尾 UTF-8；完整 JSON 内部的 NUL 用转义表示。空 C 指针不自动等于 `{}`，可选值由 DTO 定义；不保留旧的 NULL/空串/`null` 配置归一化。
- 所有 fallible API 成功返回 NULL；失败返回 owned `aimux_error_t*`。调用前将输出 pointer 写为 NULL、数值输出写为 0；错误不得留下上一调用值。
- out JSON、manifest JSON、错误 getter 字符串都 owned，统一 `aimux_free_string`；错误本身 `aimux_error_free`。本版不返回 borrowed static manifest，避免两个所有权形式混用。
- callback JSON 只在 callback 返回前有效；绑定必须同步复制需要保留的字节。context 的 lifetime 至 `aimux_stream` 返回；返回后不得再收到 callback。
- push 数据同步复制或在函数返回前全部消费，函数返回后调用方可复用 buffer；`data=NULL && len=0` 合法空输入，NULL 且 len>0 为 NullPointer；长度算术必须 checked。
- `operation.next` 快速状态在 C 中保持 `PART=1 / ENDED=2 / TIMEOUT=3` 的 out_state，不为 TIMEOUT/ENDED 分配 JSON；其他载体映射成同一 typed PollResult。名字和实现是新 ABI，不保留旧 `aimux_transcription_next_part` 符号。
- poll 超时仅表示本次等待无数据，不是操作 Timeout；错误时 out_part=NULL、out_state=0。真正 operation timeout 通过终结错误交付。

### 4.3 同步与重入

C adapter 同步，阻塞工作不能在 GUI/JS/UI isolate 主线程运行。进入任何可能 block_on 的路径前检查重入；错误为 ReentrantCall，不能依赖 `catch_unwind` 在 `panic=abort` 的发行构建兜底。

绑定须在 callback 边界捕获自身语言异常，在 wrapper 的 first-failure latch 保存异常，触发 abort；随后的 data/end callback 只做必要清理，不再投递给已失败消费者。等 native 函数返回并退出全部 callback 后，wrapper 在安全栈帧抛出唯一 CallbackFailure。void C callback 无法把失败返回 native；native 只观察 Abort/cancelled，不能宣称其自行识别宿主异常。C 调用者不得抛出 C++ 异常穿越 `extern C`。不能保证任意不返回的 callback 可被强制取消；文档必须要求 callback 有界、只搬运结果。

所有后台 join 在 registry 锁之外；5 秒为现有实时转写清理上限的初始值，超时要记资源清理失败和取消状态，不能记录“正常结束”。finalizer 仅发取消并排队回收，不阻塞语言 GC/finalizer 线程。

## 5. 流、取消、背压和错误交付

### 5.1 流状态机

```text
created → accepted → running → terminal(success | error | cancelled)
                     │
                     └→ inputClosed（仅双向转写，输出可继续）
```

- accepted 前的 Wire 校验/句柄/能力/启动失败：同步 API 返回 error，不发 callback。stdio 返回 request error，不发 accepted。
- accepted 后的 operation 只产生一次 terminal；provider 首次连接失败也属此路径。终结是控制信息，不是 V4 data part。
- data 中的 `finish` 或 `error` 与控制终结分离。V4 可恢复错误 part 必须保留为事件，不能一律抛异常关闭；仅实际 runtime terminal failure 终结 operation。
- C stream 每次 callback 交付一个 `event` 或 `end` frame。最后一个 callback 必须是 `end`；取消/失败也要有 end。取消先到则 end=cancelled，成功已终结则晚到取消不重写结果。
- end 已交付时 `aimux_stream` 返回 NULL，operation 错误只存在 end.error，避免被 callback 和返回值重复抛出。accepted 前错误只在返回值。callback 的语言搬运失败按 §4.3 的 wrapper-local latch + abort 处理：native 正常投递 end(cancelled) 并返回 NULL，wrapper 优先交付其保存的唯一 CallbackFailure，不再向用户交付取消终结。原始 C callback 的异常不得跨 ABI，native 不凭 void 返回值猜测失败。
- 不发送旧的 `on_done`，也不以 NULL JSON 充当成功终结；#166 的 NULL-sentinel 草案由显式 EndFrame 替换，防止“EOF 就是成功”。
- pull operation 把 terminal 独立保存于状态槽，不挤进普通 part 队列。消费完已接纳事件后交付终结错误一次，后续 next=ENDED；`operation.status` 仍能查询真实 outcome。不能把满队列时丢失 Aborted 视为成功。

### 5.2 取消与重试

Abort 是一次性、可共享、幂等的取消信号。operation 持有其 clone；释放 abort handle 不等于 signal。Future.cancel、Task cancellation、iterator.return/close 必须触发底层 abort，仅停止 UI 消费不是完成取消。

取消必须打断：等待 provider 建连、retry 退避、HTTP body/WS 接收、等待输入、等待输出队列容量、push 等待。已完成收费提交不能被撤回；取消只停止后续工作，不能宣称 provider 已撤销计费。

流首个语义输出之后，adapter 不得自动重新生成、切换模型或重发已发事件。所有 retry 归 aimux 操作层与明确安全的 poll/download 阶段；bindings/stdio 不再加一层重试。网络断连后的请求执行结果可能未知，不能在重连后凭相同 id 自动重放。

### 5.3 有界背压

当前切换默认预算：音频输入最多 64 项且最多 8 MiB；输出 data 队列最多 256 项且最多 8 MiB；单个事件/音频块最大 1 MiB；先达到任何限制即背压。Node/Python 可保留更小的 64 项桥接队列，但总字节上限与终结规则一致。允许显式降低限额，不允许“无限”配置。

这些是协议资源限额，不是性能成果；数值须由 RFC-0038 压测核定才能进入稳定冻结。超大单项在分配前返回 ResourceLimit；禁止通过 item count 为 256 宣称大对象内存已受限。

流队列 full 时停止向上游拉取，不丢 delta、不覆盖最老事件、不预先收集全流。abort/drop 通路不依赖获取队列容量；terminal 独立槽确保可读消费者最终看到真实结果。消费者不再读取且显式关闭时可放弃未交付事件，状态必须是 cancelled/consumerClosed。

Swift AsyncThrowingStream 的 drop-oldest/drop-newest 策略不能用来实现无损流；必须采用可 await 的有界 channel/自定义 AsyncSequence 或阻塞后台生产桥。Flutter 跨 isolate 通道必须有 credit/ack，不能把无界 SendPort 当背压。

## 6. 错误信封与 retry hint

### 6.1 规范结构

```json
{
  "kind": "aimux",
  "code": 11,
  "message": "API call failed",
  "error": {"type": "api-call", "message": "rate limited", "statusCode": 429},
  "retryable": true,
  "retryAfterMs": 1500
}
```

示例仅展示投影字段；`error` 完整联合定义归 RFC-0042。JSON 字段采用 camelCase；旧讨论中的 `retry_after_ms` 表示派生概念，新 wire 只有 `retryAfterMs`，不同时提供 snake_case alias。

- `kind=aimux|recording|binding|protocol`，每种都有 discriminator 明确的 payload。Node/Python 也用 binding 域表达参数/序列化错误，不伪装 provider API 失败。
- 保留既有有意义的错误码区间与 golden 含义：AiMux 1–17、Recording 100–105、桥接 200–206。新增类型由 manifest 分配未占用值，禁止重用历史 code=4 等保留槽；ops 协议错误使用独立登记区间。
- LoadApiKey/LoadSetting、NoSuchProvider/NoSuchModel、UnsupportedFunctionality、Retry history、ToolCallRepair 的 original/cause 链、RecordingWrite 与 FFI 参数错误均不得压成字符串 Other。
- HTTP 状态只在实际观察到响应时提供；不能给缺 key/解析失败/认证未联网错误补一个 401/500。token-expired 保留自身语义。
- absent 省略、JSON null 保留，具体 nullable 按 schema；缺少 hint 与 `0` 毫秒不同。未知错误 variant 保留原始对象和 code，返回 UnknownAimuxError；不要丢 provider data，也不要猜一个已知异常类。
- 同一个 error 在 C、Node、Python、stdio 使用同一次规范投影。不得通过多次 getter 各自重新算 HTTP-date hint。

### 6.2 派生 retryAfterMs

1. 在错误首次跨共享 dispatch 边界时记录一次可注入时钟 `now`；相同 error 的各 adapter 共用该投影。
2. 仅对 ApiCall payload 使用其规范 response headers；先尝试 `retry-after-ms` 数值，再尝试 `retry-after` 秒值，再尝试 HTTP-date 减 now。
3. 接受有限且非负的数值；毫秒向零截断，checked 范围；NaN、Infinity、超界、过去日期不产生 hint，不把无效值钳成一个巨大有效延迟。负 ms 优先项按上游语义判定无 hint，golden 固定规则，不由语言另定。
4. Retry 聚合错误保留每次尝试的错误和 hint；顶层 retryable=false、无顶层 hint，除非未来规范显式改变。不能偷偷从最后一次错误复制一个 hint，诱导宿主重试整个已经耗尽预算的操作。
5. hint 是服务端建议，不构成自动重试授权；超时/Abort 不重试。操作层仍检查剩余预算、重试次数、是否已产生语义数据、是否可能重复提交收费任务。

现实现有 `retry_after_hint()` 可作行为基线，但其浮点/溢出边界须在重构的统一 helper 内规范化；不得复制八个解析器。测试用固定时钟覆盖 ms、秒小数、HTTP-date、过去日期、大小写规范化、无 header、错误格式、零、极大值、Retry nesting。

### 6.3 边界与可观察交付

| 出错阶段 | call | stream/pull | 必须记录 |
|---|---|---|---|
| 参数/版本/句柄拒绝 | 唯一 error | accepted 前唯一 error | 未执行 provider |
| provider/操作运行失败 | 唯一 error | 唯一 terminal error，已有 data 保留 | 原分类、attempt 信息 |
| 可恢复 provider data error | DTO 定义 | 普通 data event，不改 terminal | 错误 part 和后续事件 |
| 序列化/桥接失败 | binding error | 取消并唯一 bridge failure | 不能输出 `{}` 或假成功 |
| 用户取消 | Aborted/取消投影 | terminal=cancelled；语言 cancellation 带源错误 | 不重发 |
| stdio 断连/写失败 | 调用方见 transport failure，结果可能未知 | EOF 不是成功；未读结果不能补送 | cancel 未终结操作，录制 incomplete 原因 |

默认错误打印、trace、录制、stdio 日志均须脱敏；错误原始结构的捕获策略遵循 RFC-0041。refresh token 与 credential payload 禁止通用 debug dump。

## 7. Schema 与版本

### 7.1 版本与产物标识

| 字段 | 管理的变化 | 规则 |
|---|---|---|
| `abiVersion` | C 符号、布局、调用约定、所有权 | major 不同立即拒绝加载；minor 只追加，不能改变已有布局 |
| `opsVersion` | 信封、op 语义、handle kind、流终结、V4 类型兼容性与控制规则 | `major.minor`；major 必须相同，选择双方最高公共 minor；每版本明确支持的 type revision/能力 |
| `manifestRevision` / `manifestDigest` | 本次 op/type/package/capability 实例、schema/type revision | 修订标识 + canonical JSON SHA-256；不是独立握手协议版本；digest 不同可继续按 ops/requiredOps/类型能力协商，不伪称二进制相同 |
| `recordingSchema` | 录制格式 | 由 RFC-0041 定义；本期只支持新格式，不加载旧录制；它不是 live ops 握手的附加版本 |

RFC-0042 的 schema/type revision 与固定 SDK 基线记录在 manifest，由 ops 兼容矩阵解释；不额外引入 dataFormatVersion/data_format_version 握手。冻结的是明示版本，不是把某一 SDK npm patch 号当永久 ABI。V4 新 variant 是否为 additive 必须由数据规范和兼容测试决定；不能因为 JSON 能解析就认为有语义兼容。

### 7.2 握手

未来 stdio 第一帧必须为 hello，请求 id 为十进制字符串。示例中的版本值为候选版本，并不表示已有可运行 stdio：

```json
{"type":"hello","id":"1","opsVersions":["1.0"],"requiredTypeCapabilities":["v4-core"],"requiredOps":["provider.create","text.stream"],"limits":{"maxJsonBytes":1048576,"maxBinaryBytes":1048576,"maxInFlight":128}}
```

响应列出选定 ops 版本及其支持的类型能力、manifest digest、supportedOps、feature flags 和取双方最小值的 limits。协商选中的 minor 确定请求、结果、事件控制字段集，服务端必须投影到该 schema；additionalProperties:false 与此保持一致，不通过忽略未知控制字段提供兼容。无共同版本、requiredOp 缺失或客户端要求未提供 capability 时拒绝并关闭，不能自动降成旧协议。握手完成前禁止创建 provider/执行请求。

FFI 不提供协商 hello，也没有隐式“客户端选择 minor”状态。binding 加载库时先读 ABI 版本，再获取 manifest：要求库的 currentOpsVersion 与 binding 编译时 expectedOpsVersion 精确一致，所消费的 type schema revision/hash 也必须与生成产物一致；不匹配就拒绝加载，不承诺 C 客户端的旧 minor 自动投影。raw C 用户须采用同一初始化检查，生成的入口统一执行该检查。其他 package/catalogue 元数据改变可以不影响所消费 type hash。Node/Python 内置库同样检查生成产物一致性并暴露版本/digest，不能因同包分发免去漂移检查。只有带显式 hello 的 stdio 路径提供上述 minor 协商与精确投影。

### 7.3 JSON Schema

使用 JSON Schema 2020-12，输出为 manifest 的派生发布产物；`$id`、type ID 与引用稳定、相互解析，`oneOf` 必须具有不重叠 discriminator。协议控制对象 `additionalProperties:false`；允许开放的 providerOptions/metadata/JSON value 明确标注，不能全局关闭未知字段而损坏 provider 私有信息。

请求/结果/事件/end/错误、所有 handle 引用、options 的 missing/null、整数范围、base64 字节、时间字符串、模态都必须有 schema 和正反 vectors。Schema 验证不替代语义校验：活句柄、capability、状态机、重入、重复 id 由运行时验证。

版本兼容测试包括：新服务端按协商旧 minor 投影精确控制 schema，不发送该 minor 未定义字段；requiredOps 不满足失败；未知业务 union variant 按 RFC-0042 明确 decode 失败、不变空内容；错误信封的 UnknownAimuxError 可保留原值但不能执行未知行为；不同 major、不支持的类型能力、同名 op 语义变更必须失败。

## 8. 后续 stdio：帧、调度和退出

本节是 RFC-0036 方向的完整设计候选，**不属于 #200 当前清理交付**。必须先完成 §11 的 S 门禁才可实施/发布 `aimux ops --stdio`；现有 probe/replay CLI 不因此被重命名。

### 8.1 二进制 framing

每帧：`u32be length | u8 frameType | payload`。length 计入 type 字节、不含自身四字节。协商的 maxJsonBytes 只计 JSON payload，maxBinaryBytes 只计数据块；因此 JSON 的 length 上限为 1+maxJsonBytes，BINARY 上限为 1+26+maxBinaryBytes，所有加法 checked。reader 先以两者较大值检查 length，再读取 type 并应用精确上限，之后才分配 payload。length=0、超过相应上限、未知 type、截断 frame 都是致命 framing error，不尝试按换行恢复。

- `0x01 JSON`：payload 为一个 UTF-8 JSON object，无尾部换行要求，长度上限默认 1 MiB。
- `0x02 BINARY`：payload 为 `u64be requestId | u64be operationHandle | u64be sequence | u8 channel | u8 flags | bytes`；固定头 26 字节；channel=1 音频输入，其他值保留；flags bit0=FIN，其余位必须为 0。requestId 是本次 push 的全新单调请求 ID，不是已经完成的 transcription.start 请求 ID；operationHandle 使用 start response 返回的句柄。每个 binary push 用其 requestId 接收恰好一个 JSON response(result/error)，result 仅确认入口队列接纳，不代表 provider 已处理。sequence 与 credit 以 `(operationHandle,channel)` 为键，错误投递给本次 push ID；operation 自身终结仍由 operation.next/status 观察。
- binary 最多 1 MiB 数据块；sequence 在每个 operation/channel 从 0 严格递增，不重复/跳跃，只有成功接纳才推进。JSON 的十进制 ID 与二进制 u64 值等价，不接受负数、指数或溢出。
- FIN 表示该输入通道关闭，可以携带最后一块；只发送一次；其余 inputDone 重复视为幂等，但 FIN 后新 bytes 为 InvalidState。
- 一次性 file upload 继续 base64 DTO。若后续批准 raw file upload，先登记新 channel、op、总长/摘要与取消规则，不把文件塞进音频 channel。

文本 DTO 保持 #200 的 base64 字节规范；二进制传输只为显式流通道服务，两者不是两个互相猜测的数据格式。

### 8.2 消息

```json
{"type":"request","id":"2","op":"text.stream","handle":"7","params":{"prompt":"hello"}}
{"type":"accepted","id":"2"}
{"type":"event","id":"2","seq":"0","event":{"type":"text-delta","id":"text-0","text":"hello"}}
{"type":"end","id":"2","seq":"1","outcome":"success"}
{"type":"cancel","id":"3","targetId":"2"}
{"type":"response","id":"3","result":{"state":"alreadyTerminal"}}
```

一次性请求返回一个 response（result 或 error，二者互斥），无 accepted/event/end。流使用 accepted → event* → end；end 包含 error/cancelled 时使用同一 ErrorEnvelope。所有客户端 request/control id 严格递增且在连接内不复用；服务端只保存活动请求与最近 1,024 个终结摘要，更早的 target 返回 UnknownRequest，不承诺无限历史查询。序列号逐请求单调，不能把全局执行顺序当事件顺序。

cancel 是独立 id 的控制请求。其 response 只确认取消状态变更，不替代 target 的终结。取消未知 target 返回 UnknownRequest；尚在终结摘要窗口内的已结束 target 返回 alreadyTerminal；同时发生 success/cancel 时由单一 terminal CAS 决定，禁止两个 end。

### 8.3 公平调度与 credit

输入解码、控制处理、provider 执行和 stdout writer 必须分离。reader 不能因某个音频队列满而停止读取所有 cancel；不在 reader 内 await `push_audio`。

协商后服务端通过 `credit` 控制消息（含 operationHandle、channel、grantedBytes，非 request response）按 operation/channel 给出可发送的 byte credit；客户端不能超发。每次发 credit 前，服务端从连接全局预算中预留 grantedBytes；未消费的已授权字节不能当空闲容量再次授予另一 operation。收到块时把相应 reserved credit 转为实际 queued input 占用，不能重复计数；消费后才释放预算并按公平策略重新授予。取消/关闭通道回收其未用 credit，随后到达的旧通道帧拒绝。额度不单凭每通道缓冲大小计算：必须始终满足 queuedInputBytes + queuedOutputBytes + outstandingGrantedInputBytes + otherWaitingPayloadBytes ≤8 MiB。至少1 MiB保留给输出进展，输入credit最多占7 MiB，避免客户端闲置inputcredit阻塞服务端输出。控制/终结槽不借用payloadcredit。超发是协议错误，取消该 operation；连续违例关闭连接。这样等待音频的生产者不会堵塞 cancel、inputDone 与其他请求。

输出端按请求公平轮转，一次写完整帧不可交错。每请求有界队列；一个慢流只能阻塞该流，连接 writer 的下游管道不可写时施加全连接背压并继续处理 stdin 的取消。默认 maxInFlight=128（支持 RFC-0038 的 100 并发验收），额外新请求返回 Busy，不静默排入无界队列。

stdio 全连接所有适配器等待队列共享 8 MiB payload 总预算，计入全部 operation 的输入/输出排队数据，不是每个 operation 各 8 MiB；另允许最多一个正在解析/写出的合法 frame，占用不超过协商单帧上限。本版单帧的数据 payload 最大 1 MiB，实际 frame 另有 type/固定头/长度前缀（binary 最多1 MiB+31字节）；若将来提高，须按“一个在途大帧 +8 MiB 等待 payload”重新登记 peak 门禁。控制/terminal 状态使用受限独立槽，不能被 data 占满。100 条流暂停读取后恢复再取消的 ack P95 ≤100 ms 归 RFC-0038 验收。

断管/EPIPE 立即停止产出、取消所有 operation，退出时资源回收不再试图输出 terminal；客户端按 transport failure 处理。无法读 stdout 的客户端不能要求服务器无限内存保留所有结果。

### 8.4 进程与日志

stdout 只允许 framing；stderr 为脱敏日志。help、panic/debug println、进度条都不得混入 stdout。CLI 参数错误在进入协议前退出；协议后所有可报告错误走结构化帧。

stdin EOF：停止接纳请求、取消存量 operation、关闭 producer、尝试有界 recorder flush，再退出。正常显式 shutdown 先拒绝新请求、按参数 drain 或 cancel，返回 shutdown ack 后关闭。stdio 默认总清理期限 2 秒（涵盖取消、join、flush，优先于单个 FFI Operation 的 5 秒防护上限），超期强制退出并标记未完成录制，不能把未刷盘数据报告为 durable。flush 的详细完整性归 RFC-0041。

不自动重启子进程、不恢复旧句柄、不自动重放未确认请求；不增加 daemon、UDS、HTTP、认证服务或多租户隔离。

## 9. 八种语言映射与专项验收

共同对象为 Provider、Registry、typed Model、Operation/stream、Abort、devtools handle 与结构化错误；不强迫所有对象都叫 Model。业务 JSON 可以直接使用规范 DTO；typed 层必须与 raw JSON 层行为等价。

### 9.1 C / C++

- 生成 `aimux.h`、op 常量、opaque error 与所有权注释；C++ 例子用 RAII 包装 owned string/error/handle，无异常跨 ABI。
- 具名 provider/model API 是新契约便利入口；不保留 `aimux_openai_new` 等旧符号。发布者提供精确导出 allowlist。
- 测试：C11/C++ 编译链接、每种错误 free、NULL out 参数 sentinel、无效 UTF-8、超界长度、callback 生命周期、drop 竞争、wrong-kind 全矩阵、ASan/LSan；无法执行某 target 的测试必须区别于仅交叉编译。

### 9.2 Go（D2）

- cgo 保留；`Close` + `runtime.KeepAlive`；context cancellation 映射到 Abort；`cgo.Handle` 只保存 callback context，不能把 Go 指针长期交给 Rust。
- 流使用有界 channel，producer 是唯一关闭者；Err 仅在 channel 关闭后读；从 callback 往满队列发送时 select cancellation，不能阻塞关闭。
- generated DTO 保留缺失/null 与 uint64 十进制 wire；error 用结构化类型及 `errors.As`，保留完整 envelope。
- 测试：race detector、Close/Generate 并发、context 在启动前/满队列/终结竞争时取消、无人消费后 Cancel、cgo handle 归零、child drop 后 composite 可调用、所有模态 typed round trip。

### 9.3 Java（D3）

- JNA 声明只生成一次；Java 8 仍支持，生成普通 final class/constructor/accessor，不以 records 删除 Builder。业务 DTO 可以移除冗余 Builder，但须作为本次 breaking 文档说明。
- AutoCloseable 主导资源释放；提供同步方法及显式 Executor 驱动的 Future；取消 Future 必须触发 Abort；callback context 在 native call 返回前强引用保活。
- 不在持 model 读锁的 callback 内等待 `close()` 写锁；在调用 admission 后保持强引用，将关闭与取消操作分开。
- 测试：现有 JDK 矩阵、JNA 32/64 位签名、异常 subtype、UTF-8、Future.cancel、slow Consumer、callback 抛异常、重复 close、Java 8 编译、native library 版本不匹配早失败。

### 9.4 Kotlin（D1）

- Kotlin 依赖同版本 Java artifact，删除第二套 JNA 与 native loader；保留 Kotlin sealed AimuxException 的穷举类型外观，由 Java envelope 映射，不复制错误字段解码。
- Coroutine/Flow 使用 IO dispatcher；Flow collector 取消必须 abort；有界 channel，不用无界 callbackFlow 缓冲。
- Kotlin typed DTO 可以由同一 manifest 生成其惯用类型；不得成为第二套 wire 权威。Java public 基础对象与 Kotlin sealed 映射的依赖边在 Gradle/CI 明确。
- 测试：Java artifact 依赖实际解析、sealed when 编译、Flow.take(1)/取消/异常、调度器不占 UI、Java/Kotlin 互传 provider/model、相同 error fixture subtype 和 unknown fallback。

### 9.5 Swift（D4）

- UInt64 handle wrapper、明确 close；后台执行器运行阻塞 C 调用；AsyncSequence 使用真实有界背压。
- `onTermination`/Task cancellation 触发 Abort；Unmanaged context 只释放一次且晚于最后 callback；Swift Error 转换不能从 C callback 抛出。
- Swift Sendable 声明必须有同步依据；不能只加 `@unchecked Sendable` 而无 close/admission 竞争控制。
- 测试：主 actor 心跳不被阻塞、提前退出 for-await、取消时 full queue、并发 close、context 释放计数、iOS simulator link、macOS Swift contract vectors、未知 error variant 保真。

### 9.6 Flutter / Dart（D5）

- 从新生成头文件生成 FFI 声明；保留平台 loader，iOS 用 DynamicLibrary.process，加载版本与 manifest 必验。
- 同步 native 操作在后台 isolate；Dart 回调只能在符合其线程约束的 isolate 上运行。不得从 Rust 任意工作线程直接调用主 isolate 回调；跨 isolate 传输使用受控桥与 credit。
- NativeFinalizer 回调必须采用正确 `void(void*)` ABI 的独立非阻塞 trampoline/token，不能继续把 `void(uint64_t)` drop 随意 reinterpret 为 finalizer 签名；显式 close 解绑 finalizer，防双重释放。
- 测试：主 isolate UI 心跳、暂停/恢复 Stream、取消时 credit 耗尽、finalizer 与 close 竞争、Android ABI、iOS 静态强制链接。原 nm 查 `aimux_openai_new` 改为新 ABI 必需符号，不能直接删掉该 regression gate。

### 9.7 Node（D6）

- 保留 napi-rs，异步调用 Promise、流 AsyncIterator/ReadableStream；共享 typed dispatch，不通过阻塞 C adapter；runtime 只能在有文档的 tokio context 驱动 future。
- AbortSignal 连接到底层；`return()`/reader.cancel/环境关闭都 abort；realm-local handle table 和 JS 对象生命周期隔离。
- 允许已有搬运结果的 TSFN/Promise machinery；不公开 JS fetch/model/token/telemetry 注入。不能以“napi 已支持回调”为由恢复 #200 §0.4 已后置能力。
- 测试：event-loop timer 延迟、流消费慢/中止/未消费、worker_threads realm 交叉句柄拒绝、环境关闭无悬挂、error properties 完整、raw/typed DTO 一致、provider/trace/replay 与 C 路径同 vectors。

### 9.8 Python（D7）

- 保留薄 PyO3；所有可能阻塞的生成、构造时 I/O、discovery、poll、flush、join 入口均先提取 owned 参数，再释放 GIL；只在构造/访问 Python 对象时持 GIL。
- 同步迭代器使用有界 Rust channel；close/上下文退出/取消触发 Abort，StopIteration 只用于成功耗尽，终结错误抛相应 AimuxError subtype。
- 不新增任意 Python callable fetch/credential/model；不在本期承诺新的 asyncio 回调桥。已有对外异步外观若保留，须用该语言自己的执行桥，不伪称普通阻塞迭代器是 async generator。
- wheel 继续 PyO3 原生打包；不新增 ctypes 动态库查找/双套 wheel 方案。
- 测试：并发 Python 心跳验证每个阻塞入口释放 GIL、iterator.close 满队列、KeyboardInterrupt 与显式取消、GC 防漏、错误 cause/Retry history、wheel 安装后离线 smoke、所有模态 typed round trip。

## 10. D8 生成责任与合同测试

### 10.1 责任界面

RFC-0042 拥有四层类型定义、Wire DTO、type IDs、错误 variants、nullable/bytes/time/JSON value 规则及 codegen；本 RFC 拥有 op metadata、handle kind、调用模式、所有权、生命周期、控制信封和语言桥接语义。RFC-0040 拥有 package descriptor/settings/命名方法/认证 slot；RFC-0041 拥有 Recording schema 和 replay DTO。

这些输入在同一 manifest 汇总并一次生成：Rust dispatch registration、C header、各语言 op 常量与类型、provider 工厂外观、JSON Schema、文档参考和 contract vectors。语言 loader、executor、finalizer、stream pump、错误投影外观可以手写；provider 行为、wire 字段表和 per-provider 构造列表不能手写。

provider 输出的 tool-call input 是 V4 原文 String，core 工具输入是解析后的 JSON；不得在此重新引入 `ToolInput` 无标签联合混淆两层。stream 事件类型归 RFC-0042；本 RFC EndFrame 是传输控制类型，不成为第五套业务 StreamPart。

### 10.2 可执行门禁

1. 新 `aimux-codegen --check`（命令名为实现要求）在临时目录重生成，比较完整文件集合；漏文件、stale 文件、格式、manifest digest、schema ref 和文档表漂移都失败。禁止 build.rs 暗中生成不同产物。
2. 每个 op × 每个 handle kind × 错误状态生成全矩阵：允许组合成功进入对应 stub；错误组合在 provider 被调用前失败；覆盖 0、未知、已释放、不同 runtime、耗尽。
3. 每个 DTO vectors 经过 Rust encode → 八语言 decode/encode → Rust decode；object key 顺序无关，数组、数值精度、缺失/null、metadata、byte 内容及 unknown variant 原值保留。
4. 共享 scenario vectors 覆盖 accepted、preflight failure、finish、recoverable error、terminal error、cancel、late cancel、reader close、duplicate finish、serialization failure、full queue。各 binding 与 dispatch 得到相同语义记录。
5. C 导出 allowlist 与生成头文件双向比较；旧构造器/错误 payload getter/旧头文件引用 grep 必须归零。Node/Python Rust 依赖树不得重新引入已退役 provider 入口。
6. 每语言专项测试见 §9；共同门禁不能被“只运行本语言 CI”替代。`cargo test --workspace`、clippy、rustdoc、contract-tests、全部 binding jobs 以及发行 packaging smoke 必须指向同一个集成 SHA。
7. 字节级 cassette 门禁继续保留，不为使新 API 通过而删已有 fixture；涉及录制 schema 的新 fixture/re-record 规则归 RFC-0041，历史输入作为明确不兼容 negative tests 或保留的历史资产，不能伪装读旧数据成功。
8. 性能与体积测试、FFI/Node/Python overhead、流 RSS 及未来 stdio RTT 阈值归 RFC-0038。本 RFC 只提供 workload 与生命周期断言，不重复设一组相互矛盾的性能阈值。

测试文件计划至少包含：`ops_matrix`、`handle_lifecycle`、`stream_terminal`、`error_envelope_golden`、`manifest_contract`、各语言生成 vectors；stdio 另加 frame parser fuzz、truncation、split/coalesced read、Windows binary mode、partial write、credit violation、EOF/EPIPE、shutdown。不得把平台未运行标为通过。

## 11. 实施、C4 切换与 1.0 冻结

### 11.1 当前清理集成里程碑

| 门禁 | 工作 | 完成证据 |
|---|---|---|
| G0 设计接受 | #200 编号/状态/supersession、ROADMAP、#166 清单与本 RFC 冲突消解 | 精确变更表；所有“旧 ABI 共存/C2 shim”依赖已撤销或改写 |
| G1 规范准备 | RFC-0042 类型/Wire DTO + RFC-0040 descriptor + 本 RFC op 表 | manifest/schema/vectors 可生成；无双重类型权威 |
| G2 完整依赖闭包 | provider/运行时/devtools、C adapter、八绑定、CLI/Web/replay 工具同步更新 | workspace 与语言编译全部通过；旧直连路径清单归零 |
| G3 行为验收 | 错误/终结/取消/背压/所有权/每模态/trace/session/recording/replay | 同 SHA 全门禁与性能报告；已知失败有明确修复，不以 skip 隐藏 |
| G4 C4 原子切换 | 删除旧导出、旧头文件、旧 error getters/构造器与仅测旧接口的测试 | 新功能覆盖对照、symbol diff、包安装 smoke、发行资产 manifest |
| G5 发布 | 同 SHA 生成所有语言包/二进制、breaking 指南 | 新版本原子关联，逐资产 checksum/ABI/ops/type revision/manifest 信息可核对 |

G1–G4 的分工可以分别提交到同一个集成分支，但不逐语言发布兼容过渡版本；providers 与 binding 调用点应在该分支内协调，主线相关区域的并行改动先合并/重放再重新跑全门禁。

### 11.2 C4 必备证据包

- **覆盖账本**：每个旧公开能力映射到新 op、类型、绑定 facade 和测试；provider 专用构造符号可以删除，trace/session/recording 等能力不能因旧家族符号删除而消失。
- **删除账本**：旧 symbols/header/imports/dependency edges 的自动扫描结果；删除旧测试前指出替代测试，不按“删 2,073 行”为成功标准。
- **发行矩阵**：当前支持的 OS/架构/库形式/语言包逐格标明 build、test、package、install。原始 C/C++ 示例、Swift modulemap、Flutter iOS force-link、JNA packaged native、Go linker、Node prebuild 与 Python wheel 都覆盖。
- **版本闭合**：每个资产携带同一 source SHA、ABI/ops/type revision/recording schema 与 manifest digest；Kotlin → Java dependency 锁同一发行系列，拒绝随机加载旧 native library。
- **回归证据**：共享 vectors、wrong-kind 表、终结/full-queue cancel、GIL/event-loop/UI 心跳、资源计数以及 RFC-0038 门禁报告。
- **breaking 说明**：明确删除 one-shot 工厂/旧设置/别名/旧 wire/旧录制读取；给新 provider→model 示例与升级后的调用方式。这是面向用户的升级说明，不是库内兼容适配器或旧数据迁移实现。

任一必需语言尚未迁移、一个平台只 build 未 test、某 native asset 没有验证，都不得称 C4 完成；可明确缩减支持矩阵的产品决策必须另行接受，不能自动跳过。

### 11.3 回退与故障处置

合并前失败：修复集成分支或整体撤回该分支，主线保持原版本；不向主线塞临时 alias 维持半切换。

发布后失败：暂停新发行推广、撤回/标记有问题的发行资产（按仓库发布流程），用户可整体 pin 上一个已发布版本。不得让新 binding 自动加载旧 ABI，或把新录制静默当旧格式读取。没有兼容读器意味着新版本产生的数据回退后可能不可读，发布说明必须直说并要求保留原始文件；不得承诺自动转换。

修复发布必须从新的全绿 SHA 重新形成完整资产集合。不能只替换某平台同名二进制而让 manifest/source 不一致；任何包管理器无法撤销的版本保持可追踪并发布修复版本。

### 11.4 后续能力门禁 S

stdio、原始文件二进制入口及 RFC-0040 的 native passthrough 分别立实施工作项；它们不阻塞当前清理切换。stdio 至少要求：

1. G4 完成且共享 dispatch/资源契约稳定；
2. 本节范围经确认，CLI 命令与子进程安全边界明确；
3. §8 parser/fuzz、版本拒绝、binary channel、credit、取消、EOF、shutdown 与 FFI 等价 vectors 通过；
4. RFC-0038 建立 CLI 体积及 stdio RTT/RSS 基线；
5. 当前能力支持表明确，无“声明 op 但未实现”造成握手假成功。

这是将已经提出的新入口分开排期，不是撤销 RFC-0036 的传输无关方向。未通过 S 时文档/manifest 不得宣称已支持 stdio。

### 11.5 1.0 冻结清单

冻结前必须逐项签署，而不是简单宣称“稳定一个周期”：

- ops v1 的操作语义、合法 handle 矩阵、所有权、线程/重入、错误域/code、取消/终结/背压与版本协商已有 golden。
- ABI v1 头文件/导出/平台 calling convention 与 loader mismatch 测试完成；所有公开支持平台都有安装后验证。
- V4 DTO 的 schema/type revision 由 RFC-0042 登记在 manifest，类型兼容性由 ops 版本/能力协商承载；后续 V4/V5 对齐不借 ops minor 偷换既有业务数据形态。录制 schema 独立，升级影响明确。
- 若 1.0 对外承诺 stdio，则 S 门禁必须先完成；若尚未完成，修改 1.0 支持范围的决策须先入库，不可借主线已存在 schema 文件声称入口完成。
- L0/L1 的具体稳定边界由 RFC-0040 与 #200 决策登记；本 RFC 不冻结尚未批准的 passthrough 或宿主能力注入。
- C4 已在原子切换时完成；没有旧 ABI 影子出口、兼容 alias 或生成器双轨。
- #95 错误体系、请求运行时、recording/replay 完整性与全部语言 contract tests 在同一候选版本稳定；性能门禁连续绿的测量周期和统计准则由 RFC-0038 给出。
- 重试与收费副作用、drop/abort 差异、callback 不可强制终止、未知结果、数据不兼容回退等限制在公开英文 binding 文档中准确可见。

## 12. 被排除的方案

- 固定九个导出但把 registry、能力句柄、二进制/poll、版本、错误所有权藏进无类型 JSON：会掩盖对象模型和高频路径，不采用。
- 每个 provider/模态/错误字段一个手写导出与八份绑定声明：重新制造当前重复，不采用。
- 旧 ABI 共存一 minor、C2 forwarding shim、旧 wire alias：与 #200 一次性切换冲突，不采用。
- Python ctypes 重写或 Node 同步 C 调用：增加线程/打包分叉或阻塞事件循环，不采用。
- Schema、ts-rs、manifest 三套都可改：无法判定真相，不采用。
- callback 缺失视为取消、队列关闭视为成功、StreamPart::Error 一律终结：混淆数据与控制，不采用。
- 以清理之名实现全语言 fetch/telemetry/model/credential callback、异步 vtable、daemon、HTTP、Batch 或多步工具执行：均后置，不能由本 RFC 的接口预留自动获得实施授权。
