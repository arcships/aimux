# RFC-0041：传输回放、可验证 fixture 与治理 CLI

> **Status**: Draft
>
> **Date**: 2026-10-03
>
> **代码事实基线**: master `72a37b5058ecd620d75dfe66bee34ef51f89294a`。本文是设计及验收规范，不是实施报告；没有运行真实 provider 探测或声称下述测试通过。
>
> **设计方向基线**: [PR #200](https://github.com/arcships/aimux/pull/200) 的 `d8c3d15a9b84eaa176b64fe9c5f84d678498634d`，第一部分 §4.3–4.6。该 PR 记录的决策作为本稿目标，不等于已合并实现。其 RFC-0036 编号冲突由索引维护处理，本文用 PR 与 SHA 标识来源。
>
> **Tracks**: [#167](https://github.com/arcships/aimux/issues/167)、[#179](https://github.com/arcships/aimux/issues/179)、[#180](https://github.com/arcships/aimux/issues/180)、[#181](https://github.com/arcships/aimux/issues/181)
>
> **Amends**: RFC-0023 的配置快照重建、mock 解码与格式；RFC-0025 的 CLI 后续范围；RFC-0015 §10 的验收状态。保留 RFC-0024 已落地的显式会话优先、opt-in strong-prefix 推断。

## 1. 问题与范围

现有 recording、session、cache trace 和 CLI 均有已落地 MVP。缺口是把传输重放变成协议测试的共同边界，以及把治理结论变成可核验的证据。不能通过新增一套 OpenAI 解码器、自动重建配置或把所有 CLI 子命令标成完成来填补这些缺口。

本稿确定：

1. 在 `transport_leaf` 录制和重放 HTTP/WS，真实 provider 转换器仍执行。
2. 单一 schema 3 和匹配引擎供 runtime、fixture 测试、CLI/Web 使用；TraceRecord 保持无明文审计投影，不强行合并成完整 wire 录制。
3. 离线回放必须网络封闭；凭证由离线组装显式替换，鉴权实现不识别 replay 类型。
4. 定义流、时钟、取消、队列丢失、错误及 fixture 审核契约。
5. 在既有 CLI 上完成 probe/replay/diff/session query；区分主动付费探测、live replay、已有进程 attach。
6. 把 #180 未核实的 cache 场景与 #181 partial-match 决策保留为可执行的证据任务。

不在本稿重新设计 provider/registry、ops wire、绑定异步 ABI 或性能门禁。registry 来源同步与端点维护 drift 由 RFC-0033 管理；模型能力声明来自 RFC-0040 descriptor 经 RFC-0042 生成的 manifest，此处消费该声明并验证证据。命令存在不表示能力已验证。

## 2. 当前事实与替代关系

| 项目 | master 已有 | 本稿增量与验收边界 |
|---|---|---|
| RFC-0023 | recording 输入/配置/HTTP、Ring/JSONL、flush、MVP replay | 保留生命周期能力，替换数据来源与 replay 边界；不重新实现旧 OpenAI parser |
| #167 | core mock 按 OpenAI chat wire 解码；providers rebuild allowlist | 删除 parser/allowlist/config snapshot 重建链，目标由宿主解析 |
| RFC-0024 | `CallOptions.session_id`、SessionStore、strong-prefix inferer | 迁到 operation scope 的消费位置，保留语义；增调试查询和来源解释 |
| RFC-0025 | `cache-probe offline/session/provider` | 增统一命令面；provider 是真实请求，不是 attach |
| RFC-0015 | session_id 接线、TTFT 观测、若干缓存回归测试 | 不重复立项；补 spans、meta cap 与逐项证据 |
| #180 | response-cache header、Anthropic 三字段、Bedrock equality、部分聚合测试 | 不把这些测试泛化成 20-block/quota/完整 A1–A5/并发均完成 |

#167 原建议的「ProviderRecord = registry row + protocol」不再是目标；不保存 settings、key 来源，不从录制 URL/child 列表反推 provider 或组合策略。原 `PassthroughOnMiss` 与离线保证冲突，本期不提供：所有 miss 为错误。若日后需要混合运行，必须另设清楚标为联网的功能和授权契约，不能成为离线模式的开关。

旧 schema 不自动读取、不提供旧 API alias 或数据迁移。新的 reader 遇旧格式返回 `UnsupportedRecordingSchema`，提示重新采集。旧 fixture 在切换工作中由维护者审核后重新制作为 schema 3；不把历史真实流量偷偷转存。删除旧消费方与新入口切换属于同一集成闭包，Draft 文档合入不触发删除。

## 3. 分层、所有权与 API

```text
operation / child / attempt events ────┐
                                     ├→ aimux-devtools Recorder → schema 3
provider-utils transport diagnostics ┘

ReplaySession → replay_fetch / replay_web_socket / offline download capabilities
            → 显式离线 provider 组装 → 真实 provider 转换与解析
            → operation 结果、trace、session、diff
```

- `aimux-provider-utils` 拥有 Fetch/WS、字节流及 leaf diagnostics；不得依赖 devtools。
- `aimux` 拥有 operation scope、目标解析、retry、child 生命周期与 SessionInferer 注册接口。
- `aimux-devtools` 拥有 schema 3、capture policy、ReplaySession、匹配及 diff；订阅前两者，不读 model 私有配置。
- provider 包/descriptor 提供已验证的离线构造配方；CLI 组装 registry，不把 registry 工厂搬进 matcher。
- FFI/绑定/CLI/Web 消费同一 Rust 引擎。跨语言只传序列化 options 与已定义 runtime 句柄；不凭本稿新增任意 C 异步认证回调。

建议内部入口（名字是目标 API，不声称已存在）：

```rust
ReplaySession::load(recordings, ReplayOptions) -> Result<ReplaySession, ReplayError>;
ReplaySession::fetch() -> Fetch;
ReplaySession::web_socket() -> WebSocketConnector;
ReplaySession::verify_consumed() -> Result<ReplaySummary, ReplayError>;
ReplaySession::cancel();
ReplaySession::close(); // 幂等，释放 buffers/timers/任务
```

`ReplayOptions` 包括 `scenario_id`、matcher、pacing、资源上限和 normalizer 版本。离线构造必须先验证 descriptor 的所有网络能力可替换；失败发生在 operation 启动前。一个 ReplaySession 默认只运行一个 scenario；并发测试为每个 scenario 分配独立会话，不能复用全局消费游标。

## 4. 唯一录制格式与完整性

沿用 #200 的 `schema: 3`，不定义「CLI schema」「cassette schema」分叉。每个 JSONL 行为一个 operation 的 Recording；小型 fixture 也采用该 envelope。schema 3 的新增必需字段在 schema 3 首次实施前冻结；实施后不兼容变更必须升版本。

### 4.1 结构约束

本文 snake_case 字段是 Rust/概念标识，不是第二套 JSON 命名规范。实际 schema 3 JSON 全部由 RFC-0042 的共享 DTO/codec/manifest 生成，字段 camelCase、联合 `type` 值 kebab-case。例如概念上的 `call_id/model_calls/exchange_id/transport_closed/offset_us` 分别编码为 `callId/modelCalls/exchangeId/transportClosed/offsetUs`；`schema` 仍为数字 3。读者不得同时接受 snake_case alias。以下局部示例只展示命名与联合，不是完整可加载的 Recording：

```json
{"schema":3,"callId":"call-1","modelCalls":[],"transportClosed":true,"input":{"type":"forbidden","reason":"capture-policy"},"exchanges":[{"exchangeId":"exchange-1","timing":{"terminalOffsetUs":1200}}]}
```

- 顶层：`schema`、`call_id`、`recorded_at`、`operation`、`target`、可选 `session_id/function_id`、`capture_policy`、`input`、`model_calls`、`exchanges`、`ws_sessions`、`outcome`、`complete`、`transport_closed`、`replayability`。
- `target.model_ref` 表示原 operation 的 registry key/模型 ID/命名方法；直接 model 可以无可寻址引用。`target.model` 只是身份描述。两者不能由 endpoint 倒推。
- `model_calls` 的 `(seq, attempt)` 在 operation 内唯一；parent 引用必须存在且无环。Router/MoA 的每个 child 各有 scope，aggregator 与 references 分开记；retry 不分配新的逻辑 seq。
- exchange 用 `exchange_id` 唯一标识，`exchange_index` 在其因果分区内严格递增；包含 `phase=prepare|model|auxiliary`、可选 `model_call`、`boundary=transport_leaf`、request、可选 response、timing、outcome、truncated。
- 响应 status 与 headers 到达即登记；随后记录有序 bytes chunks 与 terminal。HTTP 没有响应与收到错误 status 必须区分。
- bytes 使用共享 wire codec 的 base64 表达，保留原始长度；不把二进制转成有损 UTF-8。headers 使用有序 pair 列表，保留重复项；大小写规范化仅作用于匹配视图。
- WS 按连接记录握手、方向、frame 序号、text/binary/ping/pong/close/error 与相对时间。close code/reason 经策略处理，不伪造成 HTTP SSE。
- 每个 exchange 以 transport leaf 接受该请求为单调时间原点 0；`timing.response_head_offset_us`（收到 headers 时可选）、每个 chunk 的 `offset_us`、必需 `timing.terminal_offset_us` 共用此原点。无 response 的 reset/timeout 也必须有 terminal offset。WS 以 connector 开始为原点，握手、双向 frame、terminal 使用同样相对偏移。记录可以另带 operation-relative 开始偏移以解释并发，不能跨 exchange 强加全局顺序。每条因果流内部 head ≤ chunk offsets ≤ terminal；允许相等，不允许逆序。JSON 对应 `responseHeadOffsetUs/terminalOffsetUs/offsetUs`。`recorded_at` 仅作来源时间，不参与确定性匹配。

`Captured<T>` 的规范 JSON 为共享 codec 中的显式 `type` 标签联合：`{"type":"captured","value":T}`、`{"type":"forbidden","reason":"..."}`、`{"type":"missing","reason":"..."}`。不能以 null 同时表示未请求、被策略禁止和丢失。截断另记录 original length、captured length、算法/摘要可用性，不用一段合法 JSON 假装完整 body。

### 4.2 completion barrier

operation 终结后仍等全部 producer/child/pump/WS/body guard 关闭；所有已登记 exchange 必须终结且无必需控制事件丢失，才可写 `complete=true`。`do_stream` 返回、收到 Finish、用户不再 poll 三者都不能独自替代 barrier。

终态分为 `success/error/cancelled/incomplete`；`complete` 描述记录是否完整，不描述调用是否成功。完整的 HTTP 429 或取消场景可以是有效 fixture；丢失 chunk、未知终态或 recorder shutdown 超时必须 incomplete。导出进行中的 Ring 快照使用 `incomplete` 与原因，不凭空结束正在执行的操作。

事件队列有界，默认建议 4096 个事件、64 MiB 在途 bytes；录制不得反压正常模型请求。满载 drop-newest 并增加 dropped counters，将相关 operation 标为 inconsistent。控制平面预留 terminal/drop 标志容量；若不能逐 call 归因，将该 recorder generation 所有受影响在途记录标为 incomplete。writer flush 回执报告 persisted/incomplete/dropped，而非只报「成功」。Ring 默认保留 2048 个完成记录并有 64 MiB 总预算；超限逐条淘汰最旧记录并计数，单条上限 8 MiB。以上为本稿建议默认资源限额，需实现压测验证，不是性能实测结果。

### 4.3 replayability

`replayability.mock/live` 为独立布尔值并附机器可读 reasons。完整不等于可重放：只录 digest、正文被脱敏、缺输入、未替换的网络能力、未记录的辅助下载、未知 normalizer、truncated、missing terminal 都可使回放不可用。reader 必须重算安全约束，不盲信文件宣称的布尔值。

live 要求完整的原 operation 输入和宿主可解析目标；不能把 V4 prompt 有损转回旧用户消息。mock 的纯 transport harness 可以不需要 operation 输入，但 CLI operation replay 需要输入或调用方显式提供的完整输入。支持范围在报告中分开列。

## 5. 敏感录制与 fixture 发布策略

录制默认关闭。启用采用显式 policy：

- `metadata`：仅结构和允许的诊断元数据，不保存正文，默认不可 replay。
- `payload`：用户明确启用后可保存请求/响应 bytes；必须展示可能含 prompt、文件、工具参数及模型输出，配置保留/容量限制。不能因开启 trace 顺带开启 payload。
- `fixture`：由维护者在审核过的合成输入或获准录制上导出；导出默认本地，不自动提交、上传或推送。

所有模式在事件进入 writer/queue 之前处理 secrets：Authorization、Proxy-Authorization、Cookie/Set-Cookie、API key、签名、安全 token 和声明为 sensitive 的 query/header/body 路径永不明文保存。未知自定义 header 默认只留名称，allowlist 才留值；URL userinfo 删除。嵌套 body 凭证路径由包登记；通用 secret 扫描是附加检测，不能承诺识别任意秘密。

流式 capture 在释放任何 bytes 到 recorder queue 前必须做资格判定。只有 media type/包策略已经登记并验证「不包含敏感路径」的 payload，才允许直接 tee 原始 chunks。包含声明的敏感路径、未知媒体格式或无法确定敏感性的 body，必须使用经过验证的有界增量 sanitizer；没有该能力则整段 body/chunks 为 `forbidden`、mock replay unavailable，正常 transport 消费照常进行。sanitizer 的输入只存在独立的有界临时内存，不进入日志/queue/落盘，默认预算 64 KiB；只释放已被解析器证明安全的单位，不释放未分类的前缀。路径尚未确定、语法错误、状态不确定或超预算立即停止该 body 的捕获、清除未分类 buffer，并将整段 body 标为 forbidden（已安全输出的片段不组成可回放正文）。禁止静默保留部分 bytes 再宣称 replayable，也不使用未设计密钥管理的加密落盘来绕过此规则。live stream 不等待 sanitizer、不得为此整体预收集或反压；sanitizer 不能跟上时同样停止捕获。需要覆盖禁止捕获的路径时制作审查过的合成 fixture。fixture 测试必须将秘密 canary 在每个字节边界拆分，并测试父路径稍后才可判定、畸形 JSON、超预算和慢 sanitizer，assert queue/files/stderr 从未出现未分类秘密。

payload mode 也不能关闭凭证脱敏。脱敏若改变解析所需语义，记录不可 replay 原因；需要覆盖此路径时制作独立的合成 fixture，不在运行时恢复真实 secret。错误 message、WS close reason、文件名、diff 和 CLI stderr 同样走策略。不得输出认证值参与的普通无盐 hash；低熵敏感输入指纹使用运行期 HMAC，key 不进入录制，不能据此跨运行识别个人数据。

fixture 导出清单含 schema/normalizer 版本、source=`synthetic|approved_capture`、采集时间/实现 commit、协议/方法/模态、脱敏 policy ID、内容摘要、审核状态和适用断言。真实服务名称可记录，真实 account/project/tenant/path 应替换或禁止导出。仓库中只允许通过 secret scan 和人工 payload 审核的 fixtures；既有录制没有审核元数据时不得自动纳入。

JSONL/fixture 视为不可信输入：默认拒绝超过 8 MiB 的单条、超过 64 MiB 的加载集、过深 JSON、无效 base64、重复 ID、越界长度、父引用环、负数/逆序时间。CLI 可显式提高资源限额，但不能关闭无网络保证或读取 fixture 内引用的任意本地路径。格式不含可执行代码，也不动态加载文件指定的 normalizer。

## 6. 离线认证与网络封闭

ReplaySession 在最内层替换网络 leaf；正常签名/headers 组合在它之前照常执行。

| 原鉴权路径 | 离线组装 |
|---|---|
| API key / 无鉴权 | 显式固定占位 key / auth=none，禁止 env fallback |
| Bedrock SigV4 | 固定假 credentials、region、测试时钟；仍运行签名并断言必要签名结构 |
| Azure token resolver | 固定 fake token resolver；不变更 token/API-key 模式 |
| Vertex ADC | 包提供固定 token/headers 注入；不访问 ADC、metadata service 或浏览器，不切到 express |
| 原本的 Vertex express | 占位 key，保留原 endpoint 与模式 |
| 其他动态认证 | descriptor 配方提供封闭 resolver；无配方则 `OfflineConstructionUnavailable` |

签名值不做正常 request 等价匹配，但单独的鉴权测试必须校验 canonical request、region/service、签名顺序和不可修改的 body。排除签名字段不能掩盖鉴权回归。

必须封闭 provider HTTP/WS、输入下载、输出下载、poll 和认证获取器。采用进程级无外网测试运行环境加 panic-on-network/计数的 transport 双重断言；DNS、metadata service 和默认 client 的任何访问都使测试失败。自定义 provider 绕过能力注入点不在离线保证内，加载时拒绝将其标为已验证离线。任何 miss 不能 fallback、读取真实 key 或访问录制 URL。

## 7. 匹配、并发与确定性

先选 scenario 再消费其全部请求；禁止从不同录制拼装一次 retry/poll/MoA 流程。scenario 显式 ID 最可靠；自动选择限定目标分区、模态、方法、路由及模型。registry alias 不要求等于 `model.provider()`。

| matcher | 语义 |
|---|---|
| Exact（CI 默认） | 规范化语义请求完全相同 |
| Sequential | scenario 内因果分区顺序消费，仍校验方法、路由与请求类别；body 断言可按 fixture 显式指定，不作为协议覆盖的默认 |
| Score | 仅支持已登记对话方言；完整消息前缀与文本 LCP 评分，零相关性 miss，平分按稳定 scenario ID 排序 |
| Prefix | 已录 prompt 必须是当前 prompt 的完整消息前缀，取最长；平分按稳定 ID |

Score/Prefix 用于演示或继续对话 mock；结果标 `approximate=true`，不能用于宣称协议 Exact 回归通过。非对话/未知方言只支持 Exact/Sequential。

规范化是白名单、版本化的只读匹配视图，不改实际返回 bytes：JSON 对象键排序，数组顺序/重复值/null/字段缺失/数字值保留；不支持无损解析的数值不得经 f64 舍入后相等。multipart 解析 MIME、忽略 boundary，以字段名、重复项顺序、filename、media type、内容比较；二进制摘要一致后可比原 bytes。query 只排除声明的认证参数，其余重复项与顺序默认保留。tool-call ID、previous-response ID、业务时间戳默认保留；动态 ID 仅通过登记字段的双射符号映射保持引用，不能全局删 `id/timestamp`。

Exact 的 header 比较采用 descriptor 登记的三类集合：authentication（Authorization、Proxy-Authorization、Cookie、各包 API-key/签名/security-token 等全部排除值匹配）、semantic（如 API-version、Content-Type 的语义参数、功能 beta header）、proven-nonsemantic（有版本化依据可忽略）。名字 ASCII case-insensitive；同名值保留出现顺序及重复次数，仅按已登记规则规范化空白，不默认逗号拼接。任何 semantic header 值因隐私策略不可用，则 Exact 不可用，返回 `ReplayNotPossible`，不能把两个 redacted 占位符判为相等。未知 header 只留名称也按不可用处理，除非包登记其确为 nonsemantic。authentication 排除只用于内容匹配，§6 的独立认证 fixture 仍验证签名与 headers 构造。策略版本属于 normalizer 版本，变更会使既有 Exact 证据过期。

并发 child 用 parent/逻辑 child 标识及 attempt 分区；不以不同运行时的完成顺序或新 call_id 匹配。每个分区维护游标和已消费集合，原子 claim 防止同一 exchange 被两个请求拿走。不可判定的相同并发请求要求显式关联/fixture 顺序约束，否则 `ReplayAmbiguous`，不随机选择。结束必须 `verify_consumed()`，未消费的必需 exchange 报 `ReplayUnusedExchange`。consumer 取消时只检查取消点之前的已声明前缀，不能将正常 remainder 误报成独立 miss。

## 8. 流、时间、取消与错误

### 8.1 字节流契约

FetchResponse 到达后逐 chunk 拉取；保持 chunk bytes 和顺序，包括 UTF-8/SSE/JSON 跨 chunk 截断，不把一个 chunk 当一个语义事件。不能为录制先 collect 全流，破坏 backpressure 或 TTFT。nonstream body 可复用同一 reader。只有运行 fixture 中显式定义的 EOF/error/close，不能自动补 `[DONE]` 或 Finish。

WS 除服务端帧回放外还校验客户端发送序列、握手和 close；乱序/额外 send 明确报错。缺乏目标 WS 协议 fixture 时标未验证，不因 HTTP 回放已实现就宣称 WS 覆盖完成。

### 8.2 pacing 与时钟

- `Immediate` 默认：不等待已录 chunk 间隔，但保持异步 pull 和取消检查。
- `Recorded`：按 §4.1 exchange-local 原点后的绝对偏移调度 response head、每个 chunk/frame 和 terminal（包括无响应失败）；consumer 慢时不追加已错过的等待，不积累计时漂移。
- `Scaled(factor)`：factor > 0，所有 head/chunk/frame/terminal 偏移统一除以 factor 调度；禁止 NaN/Infinity/零/负值。Immediate 同时取消这些事件的等待但保留因果顺序，不能仅加速 chunks 而仍等待旧的 headers/error 时延。

重放时间戳属于证据，runtime 新测量属于另一组指标。Immediate 不绕过 operation retry/poll backoff；CI 用注入的虚拟时钟共同驱动 retry、poll、timeout、签名与 pacing。记录并区分 wire TTFB、模型首语义片段、用户首输出；模型 TTFT 从 `do_stream` 调用前起算，不把 SSE 预读耗时丢掉。

### 8.3 生命周期与失败

状态为 `Loaded → Running → Completed|Failed|Cancelled → Closed`。取消在匹配前、等待 headers、chunk sleep、body pull、WS send/receive 均可生效；关闭 timers、停止生产、唤醒阻塞 reader，终态只发布一次。drop 是幂等资源释放，不发新请求。已输出语义数据之后不得重试整个模型调用，也不 fallback 到其他 scenario/网络。

错误使用公共结构化错误封装并保留 devtools code：

- 输入/资格：`UnsupportedRecordingSchema`、`InvalidFixture`、`ReplayNotPossible`、`OfflineConstructionUnavailable`
- 匹配：`ReplayMiss`、`ReplayAmbiguous`、`ReplayOutOfOrder`、`ReplayUnusedExchange`
- 运行：`ReplayCancelled`、`ReplayResourceLimit`，以及录制的 transport timeout/reset/EOF

诊断只含 scenario/exchange/路径和脱敏差异摘要，不把整个 prompt、key 或原始错误 body 放进 message。录制的 429/5xx 是普通 HTTP response，走真实 provider 错误解析和 operation retry；不改成 replay engine 错误。`Retry-After` 保留并经测试时钟验证。fixture 缺失/损坏为不可重试的 harness 错误，不能消耗 API retry 后伪装为 provider 失败。

## 9. CLI 及在线边界

在 `tools/aimux-cli` 扩展，算法留在 devtools。以下是目标命令，不是当前已存在命令：

```text
aimux probe offline --file traces.jsonl --format json
aimux probe provider --model registry-key:model --method chat --max-requests 4 --allow-network
aimux session list --file sessions.jsonl
aimux session show --file sessions.jsonl --session ID --explain
aimux replay inspect --file recording.jsonl
aimux replay run --file recording.jsonl --scenario ID --mode offline --matcher exact
aimux replay run --file recording.jsonl --scenario ID --mode live --registry config.json --dry-run
aimux replay run --file recording.jsonl --scenario ID --mode live --registry config.json --allow-network
aimux diff --left result-a.json --right result-b.json --format json
```

命令切换时移除旧入口的同时更新文档/脚本/测试，不维护两套 probe 逻辑或旧命令 alias。每个命令支持 `--format text|json`，JSON 含 `report_schema`、输入摘要、构建版本、policy/normalizer 版本、result/status、warnings、证据来源；stdout 仅报告，stderr 是脱敏诊断。退出码：0 成功/无差异，1 diff 发现差异或请求判定失败，2 参数/格式/资格错误，3 replay mismatch，4 网络/远端错误，130 用户取消；完整错误 code 在 JSON 中，不靠退出码表示每个子类。

### 9.1 offline、live 与 probe

offline 永不解析真实凭证；无 `--allow-network` 时 live/probe 在构造凭证前拒绝。live dry-run 不联网、不运行凭证 resolver，只报告 operation、显式目标引用及脱敏输入概况。live 使用当前宿主 registry 和新 call_id，保存 source_call_id；直接 model/组合目标不可恢复时要求显式目标。录制 URL 只作展示，不能成为自动带 key 的 endpoint；base URL 改动要重新组装连接及认证，不继承原 key。

主动 provider probe 有 max-requests 与总超时上限，重试也计入请求预算；消耗费用必须明确显示。此上限不代表费用上限，供应商计费不可从请求次数保证。没有显式选择目标与联网时不得自动运行。真实探测结果不能写入离线 deterministic golden。

### 9.2 diff 的定义

diff 不触发网络。默认比较相同 operation 的规范结果、错误分类、finish reason、usage 及保留的 provider metadata；流式逐事件次序/内容、raw wire bytes 分别是可选 comparison 层，不混在同一个相等结论中。diff report 列出 compared/skipped/unavailable paths 与规则版本，缺失 capture 不能算相等。只忽略登记的 call_id/采集时间等观测值，不默认忽略 usage、tool IDs、业务时间戳或 metadata。浮点容差必须按路径显式传入，默认精确；TTFT 只报告 delta，不声称环境不同的时间可直接证明回归。

### 9.3 online attach 的本期决策

不交付任意现有进程 attach；stdio ops 是新启动进程的调用入口，不是进程发现、权限或附着协议。本期支持宿主主动导出 Ring/SessionStore 快照，再由 CLI 离线查看。导出与 recorder flush 共用完整性语义，不能用信号处理函数直接分配、加锁或序列化。

#179 的 online 子项保持「设计后置」，不能因 replay CLI 完成而整项关闭。重新开启需提供具体宿主、所需新鲜度、认证/租户隔离、数据最小化、背压和故障模型，再比较宿主控制面 export、dump 请求、sidecar；未有这些需求前不加入通用 HTTP/UDS daemon。

## 10. session 与 cache 治理收尾

### 10.1 SessionStore 调试

session query 使用独立、版本化的 SessionStore 导出，不把 TraceRecord JSONL 冒充完整 session index。query 输出 explicit/inferred 来源、目标引用、scope、成员 call IDs、时间区间、记录是否被淘汰/截断；`--explain` 给 strong-prefix 所匹配的 prior call 与完整消息数，不暴露正文。调试接口只读，不通过 UI 手动改写真实会话身份。

显式 session_id 最高优先，推断 opt-in、按宿主 runtime/租户隔离；推断不能改变模型、凭证、package 或 route。session affinity 若使用同一 ID，仍是独立签名前 middleware，关闭 telemetry 不关闭 affinity。

#181 partial-match 本期决定「保留 strong-prefix，不实现宽松自动合并」。依据是缺少误合并可接受性的实测证据；相同 system/tools 模板不是会话身份。评估交付物为获准脱敏/合成的带真实归组标签数据集、strong-prefix 与候选 loose matcher 的 confusion matrix、跨租户/相似模板/历史压缩/fork/重试场景、错误合并示例与内存成本。先固定 holdout，再公开精确率、召回率和样本量；阈值需维护者在看 holdout 结果前确认。当前无可引用的结果，不填虚构阈值或指标。即便实验改善也需要独立决策，不能直接开启默认合并。此决定不影响 mock Score/Prefix，因为 mock 相似度与 session 身份不同。

### 10.2 #180 逐项退出条件

| 项目 | 状态与所需证据 |
|---|---|
| `CallOptions.session_id` | 已落地；只保留显式优先/default fallback 回归 |
| TTFT | 已有观测与测试；迁移后覆盖预读、取消、empty stream，不重复声称新增基础功能 |
| span 树 | 待验收：operation → child → attempt → exchange，通过 IDs 连接 cache observation；一个 ID 存在不等于 span tree 已正确 |
| Anthropic 三字段 / response cache header | 已有部分测试；保留 golden 并补真实边界组合，不整组重写 |
| Anthropic 20-block、Bedrock quota、gateway stripping、完整 A1–A5、并发 shard locks | 未逐项验证；每项须有 fixture/test ID、预期 verdict、失败反例与运行日志后才勾选 |
| 外部 cache_read / Gemini TTL / OpenAI retention / vLLM null | UNVERIFIED；官方文档与带日期/模型/版本的获准 probe 是证据，合成 fixture 只证本地算法 |
| Azure 128-quantization、OpenAI byte-exact、byte-proxy 偏差 | UNVERIFIED；分别记录 audit family、算法版本和真实响应来源，不能以 provider 列表数量代替 |
| prototype 回补 | 建议随 #166 E1 归档旧原型，取消重复维护任务；需明确范围接受记录，不标成已实现 |
| routed/cluster | 无 affinity 证据时保留 note/suppression；零命中不能证明厂商造假。缺 route/node/cache shard 身份时显示证据不足 |

`meta_cap_bytes` 是观测导出预算，不得裁剪真正 provider 请求或运行结果。建议默认 64 KiB，0 表示不导出可选 meta payload；按 UTF-8 序列化后 bytes 计量，在进入 FFI 前执行。保持 IDs、错误 code、计数、截断声明这些固定控制字段；大 request/response meta 替换为 `{omitted:true, original_bytes, reason:"meta_cap"}`，不切半个 JSON/UTF-8 字符。固定 envelope 超限返回 `MetaEnvelopeTooLarge`，不静默扩大限制。Rust/FFI/各绑定得到同一 bounded DTO；report 明确 unavailable 而非输出空对象冒充内容为空。该上限与 recording payload 限制分别配置，不能把 cap 当作秘密脱敏。

### 10.3 能力矩阵与证据有效期

ROADMAP 0.9 已要求 provider × 能力矩阵；当前源码/文档不构成本节机制已实现的证明。本节给出目标数据契约，不填未经测量的通过率。矩阵由 RFC-0040 package descriptor 经 RFC-0042 唯一生成链输出的 manifest 快照列出行，本节的 evidence ledger 决定每格实际验证状态；descriptor 声明的 `supported` 不等于 probe `pass`。RFC-0033 仅提供 registry/端点维护观察，不声明模型能力。

每个格子的键为 `{packageId, registryEntryId?, method, modality, capabilityId, targetProfileId}`。capabilityId 使用版本化目录，例如 streaming、tool-input、structured-output、cache-observation；语义定义与断言集 hash 一起入库。targetProfileId 指获准的测试端点类别/区域/模型版本及配置摘要，不含 credentials/account ID。多端点、不同 API 版本和匿名 alias 不合并为一个远端事实。

每条不可变 evidence 记录至少包含：

```text
evidenceId, cellKey, status: pass|fail|unknown,
evidenceKind: synthetic-fixture|approved-capture|live-probe|official-document,
aimuxCommit, descriptorHash, assertionSetHash, normalizerVersion,
target: {modelId, modelVersion?, apiVersion?, region?, endpointClass},
observedAt, expiresAt, fixtureHash?, rawArtifactHash?, reportHash,
sourceUrls[], runId?, failureCode?, unknownReason?, supersedesEvidenceId?
```

全部字段按共同 JSON codec 命名。fixtureHash 为经过审核的不可变 fixture 内容摘要，原始敏感 capture 的摘要/路径不得公开。公开 ledger 只引用允许发布的 artifact；私有获准 capture 只能提供脱敏报告与受限证据引用。`unknown` 原因明确区分未测、凭证缺失、目标不可用、证据过期、脱敏导致不可断言和断言不适用。产品明确不支持的格子另带 `support=unsupported` 与 descriptor 理由，不把未实现伪装成 probe fail。

状态是断言结果，不是人工改颜色：pass 必须满足断言且无 unavailable 必需输入；fail 必须可定位失败断言；无充分证据则 unknown。最新证据不删除旧记录；聚合先按完全相同 cellKey/断言/版本分组，冲突结果保留冲突标记并显示 unknown，不能挑最后一个成功覆盖失败。同组失败只有经解释的新运行或目标变更后的新组才能解除。

建议有效期政策：live-probe/approved-capture 的远端能力声明 30 天，official-document 最多 90 天并带 lastReviewedAt；synthetic-fixture 不证明远端能力，只在完全固定的实现/断言/fixture/normalizer 组合下证明本地回归。到期保留历史 pass/fail，但 currentStatus=unknown、stale=true。descriptor、模型/API 版本、normalizer、断言集改变时立即失效相关 current 状态，fixture 本体不改写。未能解析远端滚动模型版本时记录 unknown version 并降低证据范围，不声称跨版本稳定。

矩阵生成器只读取 ledger，输出人类报告与机器 JSON；CI 检查证据引用/摘要存在、日期与键合法、所有 supported 格子的覆盖缺口显式列出。静态离线契约 tests 是合并门禁；远端能力过期或定时 probe 失败产生治理告警，不自动阻塞无关 PR。升级发布的具体必需格子由发布 manifest 显式冻结，不能把随时变化的「所有 provider」当成验收集合。

### 10.4 有预算的定期重录与漂移检测

本节是 ROADMAP 0.9 的 authenticated protocol drift，和 RFC-0033 每周 models.dev diff、每月无 key reachability probe 分开。建议每月对显式获准 target profiles 重录固定合成场景；可手动同配置重跑。默认任务禁用，仓库中只提供 workflow/配置模板；本文没有创建任何定时任务、网络调用或授权。

每个获准计划必须持久化：owner、目标 provider/方法/模型/端点、获准输入集合 hash、credential secret 引用、最大请求数（包括 retry/poll）、输出 token/字节上限、总运行时、每次与每月金额上限/币种、频率、结束/复审日期、报告目的地与 artifacts 保留策略。凭证只从受保护执行环境注入，不出现在参数/命令行/log/artifact；不在 fork PR 暴露 secret，发布报告的 token 单独授予最小权限。用户/维护者需显式批准数据、目标和预算；不能把 read-only registry 探测授权扩张成付费重录。

调度器以计划 ID+周期键加锁，预算账本原子预留最坏费用后才请求，失败和不确定计费也保守计入；不重试已提交的不幂等操作。价格/输出上限不能得出可信费用上界、费率变动、余额不足、凭证失效或审批过期时整次跳过并报告 unknown，不通过「4 次请求」假装保证费用。API 预算/组织账单硬限制可作为额外防线，但不能替代本地预留。每个请求之前检查剩余预算、取消与时间窗，终止后停止新请求并回收未消费预留，已发送请求不假称零费用。任务默认不自动扩大额度、不更换 provider，也不自动更新 golden。

重录生成新 capture 与报告，不覆盖 cassette。只有符合 §5 保留/脱敏规定的 bytes 才进入不可变 artifact；录制禁止的原文既不保存也不计算公开裸摘要。比较分三层并分别报告：

1. **byte diff**：对安全可保留的原始 exchange bytes、status、headers、chunk 序列逐字节比较并固定两侧摘要，不应用 JSON 排序或 ID 忽略；被脱敏/禁录的区段标 unavailable，不能声称其原始字节相同。
2. **normalized request diff**：按固定 normalizer 比较语义请求；明确列出排除字段与规则，不能覆盖 byte diff。
3. **decoded contract diff**：用当前及指定基线 converter 解码相同安全 fixtures，比较结构、错误、usage、metadata、流终态。随机生成文本不同与协议破坏分别分类，不靠「字节不同」自动认定 bug。

报告包括旧/新 target 版本、fixture/capture hashes、执行 commit、策略、变更路径、unavailable 区段、潜在影响和复现命令。没有安全 bytes 时只报告不可比较，不绕过 capture policy。漂移分类为 expected-nondeterminism、documented-upstream-change、suspected-regression、insufficient-evidence；后两者进入固定跟踪项，重复报告按 cellKey+变化摘要去重。自动报告不提交 fixture，不改转换器、不替维护者确认原因；更新 golden 必须单独 PR，展示 byte/normalized/contract 三层证据及审核结果。

验收用本地 fake provider 测试计划禁用/授权过期/secret 不泄漏、并发预算竞争、费用不确定、请求前取消、一次漂移重复上报、新旧 fixture 不被改写。真实付费计划启用是后续授权动作，不是本 RFC 的文档交付。

### 10.5 生态月度与季度治理

ROADMAP §6.2 已列出以下节奏；本稿具体化输入、输出和升级规则，不报告它们已经运行。RFC-0033 继续拥有每周 registry 和每月 reachability，本节不新增重复作业。

**每月参照系 diff**：固定 AI SDK、pi-ai、Open Responses 的当前接受基线版本/commit/spec revision，保存官方 release/tag/spec 来源和抓取时刻；与上次 snapshot 比较公开 API、消息/流标签、provider 行为、错误、工具/模态新增及删除。版本不按网页「latest」永久漂移；依赖锁与审计报告中同时记录旧新版本。输出逐变化清单：upstream reference、affected component、适用/不适用理由、已有 fixture、缺失证据、负责人/候选跟踪项。未知源码或无法访问上游标 unknown，不推测已对齐。

「超过 2 个版本未跟进」对有序 release/tag 项目解释为：当前接受基线之后已有至少 3 个适用稳定发布，而对应变化仍无完成或明确不采用的决策记录。预发行默认不计，安全修复单独即时升级，不等待第三个版本。Open Responses 等无稳定语义版本的规范使用发布方具名 revision；无法定义版本序列时显示版本滞后 unknown，并在连续两次月报仍未判定时请求维护者决策，不能编造版本差。达到阈值在现有固定治理 issue 中去重链接/创建有范围的子 issue；自动发布必须有预先授权的仓库、接收范围与内容类别，否则仅生成待发布报告。

**每季度协议新鲜度审计**：逐个已承诺 provider 检查官方 changelog/弃用公告/API 版本通知，保存 URL、发布日期和复核时间；关联矩阵 cell、converter 和 fixture。把已弃用 endpoint、超过证据期限、API 版本不明和缺少公开 changelog 分开标记。过时 fixture 仍保留历史内容，标 stale/replacedBy，不能删除历史失败或把日期刷新当重新验证。输出审计报告及需要更新协议转换器的具体 issue；定期重录可增强证据，不能替代阅读弃用期限。

**每季度新机制评估**：对新模态/协议机制制作统一决策项，包含用户场景、上游证据、与现有分层兼容性、auth/stream/cancel/绑定/成本/测试完整闭包、替代方案。结果必须是带依据的 `accepted-for-rfc`、`rejected` 或 `deferred`，后两者附重新开启条件。accepted-for-rfc 只授权创建 Draft 设计讨论，不表示功能已实现或 RFC 已 Accepted；真正的 Accepted/Rejected 状态由维护者决策链接记录。每项追踪其 RFC/决策链接和最后复核时间，防止每季重复重开相同已拒方案。

这些报告共用只读收集器、不可变 snapshot 与 report schema，按 runId/输入摘要可重现；采集失败保留上次报告并标本轮失败，不能写空差异。每月/季度 owner 复核所需 issue/RFC 决策和逾期项；没有授权的外部写入只生成本地/CI artifact。验收用冻结上游 snapshots 验证零变化、跨 3 发布升级、预发行排除、无版本规范、changelog 缺失、重复去重、Accepted/Rejected/Deferred 三种记录及过期状态，不需要宣称实时生态已全量核验。

## 11. 测试矩阵与门禁

fixture manifest 从 RFC-0040 的 package descriptor 经 RFC-0042 唯一生成链输出的 manifest 枚举已承诺能力；RFC-0033 只提供 registry/端点维护观察，不声明模型能力。覆盖集合以 package × method × modality × streaming 为键；标为 unsupported 的格子需要理由，标 supported 的格子必须有对应证据。不要求每个别名重复协议 fixture，但需要验证别名选择的目标/配置；共享 converter 不能证明所有包的认证/endpoint 一致。

必须交付的确定性测试：

1. schema 严格校验、版本拒绝、bytes/headers 往返、policy forbidden 与事件 missing 的区别。
2. JSON/multipart/重复 query/二进制 Exact 正反例；未知方言拒绝 Score；稳定 tie；ID 双射和被禁止的泛化忽略。
3. 首响应前失败、429→成功 retry、Retry-After、无响应 reset、headers 后流错误、正常 EOF/异常 EOF、取消前/中/后及 drop，全部 assert 一次 terminal 和无泄漏。
4. 拆开的 UTF-8/SSE 帧、空 chunk、多事件同 chunk、慢消费者、Immediate/Recorded/Scaled 与虚拟时钟；不得以 sleep 的墙钟精确值作 golden。
5. WS 双向帧、close/error/取消（仅已声明支持协议）；未覆盖项在 manifest 中显式列出。
6. Router 串行 fallback、MoA 并发 child 与 aggregator、child retry、重复请求竞争；验证因果消费，不复跑已完成 references。
7. 输入/输出下载和 poll 的多请求 scenario，认证 fake 不外网、miss 不外网、默认 env/ADC 不被调用；运行结束检查所有必需 exchanges 已消费。
8. queue/Ring 溢出、writer 超时、flush/try_flush/shutdown、incomplete export；逐个验证 dropped/inconsistent 统计与 replayability。
9. 敏感 canary 在 URL/header/body/errors/WS/diff 中均不出现在序列化文件和 stderr；低熵敏感值无裸 hash；不可信 fixture 限额/路径/解析攻击。
10. CLI text/JSON/退出码、dry-run 零联网、live 显式目标、schema mismatch、diff skipped、meta cap 多字节边界；所有绑定共享同一 DTO 和 Rust replay 引擎。
11. #180/#181 单独 manifest 保留未核实项；合成回归通过不覆盖真实行为待证。

CI 离线协议任务禁止网络和真实 secrets，使用固定 seed、时钟、normalizer、输入及 fixture 摘要。更新 golden 的 PR 必须展示原差异、理由和审核；不得「重新生成后全绿」作为正确性证明。真实 provider probes 为单独 opt-in job，保存脱敏报告与日期，不能成为无凭证 PR 的必须联网步骤。跨绑定验证只要求其已声明的 replay/recording 表面，不把尚未定义的 C 宿主异步回调记为通过。

## 12. 实施切片、依赖与关闭标准

| 切片 | 依赖 | 可独立评审的结果 |
|---|---|---|
| G1 schema/policy | #200 transport/runtime 目标契约、公共 wire codec | schema 3、校验器、capture policy、completion barrier 规范测试 |
| G2 leaf/session | Fetch/WS 注入、scope/child 传播、fake auth 配方 | ReplaySession、Exact/Sequential、网络封闭、stream/cancel/时钟回归 |
| G3 消费闭包 | G1/G2、provider 转换器、统一绑定 manifest | CLI/Web/测试使用同引擎；移除旧 decoder/rebuild/config snapshot，不留半切换 |
| G4 治理命令 | G3、SessionStore query、trace/descriptor 投影 | probe/replay/diff/session 报告与退出码、live 目标隔离、meta cap |
| G5 证据收尾 | G1–G4、RFC-0040/0042 descriptor/manifest 与 §10.3 证据 ledger | §10.3 能力证据矩阵、§10.4 定期重录/三层 diff、§10.5 月度/季度报告与升级门、#180 残余证据、#181 决策记录，全部未验证项可检索 |

G1/G2 可以在集成分支分别评审；主线切换必须满足消费闭包，不能通过旧 schema shim 把未完成依赖藏起来。版本排期仍由 ROADMAP 管理，本稿不承诺日历工期。

- #167：schema、传输 replay、child 因果录制、各公开消费方及网络封闭都验收；旧 ProviderRecord/Passthrough 提案的替代决定同步到 issue 后再判断关闭。
- #179：replay 子项可独立完成；online attach 保留后置状态与重新开启条件，不整项误关。
- #180：逐行证据清单更新；UNVERIFIED 外部行为不因实现 merge 变已验证。旧原型维护是否取消由范围决策确认。
- #181：debug query 可完成；partial-match 以独立决策和依据收尾，不要求必然实现，也不能以 mock matcher 代替该决策。

合入实施前的 review 必须核对：不存在隐式联网；secret capture 无旁路；新旧架构替代关系明确；完整性和可回放性不混淆；每个覆盖声明都有 fixture/test/report；未测结果未被写成通过。

## 13. 参考与证据边界

- [master ROADMAP](https://github.com/arcships/aimux/blob/72a37b5058ecd620d75dfe66bee34ef51f89294a/ROADMAP.md)
- [#200 固定版本设计](https://github.com/arcships/aimux/blob/d8c3d15a9b84eaa176b64fe9c5f84d678498634d/rfc/0036-aisdk-architecture-alignment.md)：schema 3、replay_fetch、离线 auth、目标引用、无旧录制读取
- [RFC-0023](0023-runtime-request-recording.md)、[RFC-0024](0024-session-aggregation.md)、[RFC-0025](0025-aimux-cli-cache-probe.md)、[RFC-0015](0015-cache-trace-audit.md)
- [现有 CLI](https://github.com/arcships/aimux/blob/72a37b5058ecd620d75dfe66bee34ef51f89294a/tools/aimux-cli/src/main.rs)、[strong-prefix 实现](https://github.com/arcships/aimux/blob/72a37b5058ecd620d75dfe66bee34ef51f89294a/aimux-core/src/session.rs)、[现有 cache tests](https://github.com/arcships/aimux/blob/72a37b5058ecd620d75dfe66bee34ef51f89294a/aimux-core/tests/cache_probe_test.rs)

本文核对的是以上源码和设计文本；没有运行新实现测试、性能基准、真实 provider 探测或统计当前全供应商覆盖率。
