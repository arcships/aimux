# RFC-0050：纯 Rust 调用路径的每请求与每块开销

> **Status**: Draft
>
> **Date**: 2026-11-12
>
> **Baseline**: master `c5a408370`（v0.6.1 之后）。文中 `文件:行号` 都指这个提交；上游引用 `ai@7.0.122`（`reference/aisdk-pinned/ai`）。
>
> **Scope**: Rust 服务直接依赖 `aimux-core` + `aimux-providers` 调用 `generate_text` / `stream_text` 时经过的四个 crate：`aimux-core`、`aimux-providers`、`aimux-provider-utils`、`aimux-stream`。不涉及 `aimux-ops`、`aimux-ffi` 和各语言绑定。
>
> **Related**: [RFC-0036](0036-positioning-and-layered-architecture.md)（AI SDK 对齐方向）、[RFC-0043](0043-size-performance-budgets.md) §7（基准测试方法）、[RFC-0044](0044-crate-layering-and-dependency-inversion.md)（crate 分层）、[RFC-0015](0015-cache-trace-audit.md)（请求体回显的用途）、PR [#502](https://github.com/arcships/aimux/pull/502)（单次解析的 JSON 响应处理器，未合并）、PR [#407](https://github.com/arcships/aimux/pull/407)（`Arc<HttpBody>` 与延迟脱敏，已合并）、issue [#405](https://github.com/arcships/aimux/issues/405)（并发评估 M1–M5）。
>
> **来源**：本文修订自一份外部草案。草案列出的十处开销点（下文 R-1…R-10）经逐条在源码上核对后保留；草案的方案中，与 AI SDK 对齐方向冲突、与已有实测结论矛盾、或引入并行代码路径的部分不采纳，理由见 §5。

## 1. 决策摘要

1. **请求体不再复制；step 结果默认不带请求体，对齐上游。** provider 把请求体交给 HTTP 层后再原样拿回，删除 `body.clone()`；`ai@7.0.122` 的 `generateText` 和 `streamText` 的 `include.requestBody` 默认都是 `false`，aimux 的 step 结果同样默认不带请求体（§4.1）。
2. **每个响应体、每个 SSE 事件只解析一次 JSON。** 非流式响应沿用 #502 的单次解析处理器推广到全部 provider；流式响应从 SSE 事件的字节直接反序列化成 provider 的块类型，不经过中间 `Value`（§4.2、§4.3）。
3. **工具集按引用共享。** `GenerateTextOptions::tools` 改为 `Option<Arc<[Tool]>>`，每次调用和每个 step 的 `tools.clone()` 变成引用计数加一（§4.4）。
4. **只保留一条路径。** 不新增 `HttpBody::WireJson`、内联流快路径、静态分发的 `NativeLanguageModel` 等与现有路径并存的实现。要换就整条替换；不能整条替换的，本文不做。
5. **先测后改。** 每项改动必须按 RFC-0043 §7 的方法给出成对测量结果；测不出收益的改动不合入。本文不写绝对数字门槛，原因见 §6.2。

## 2. 现状：开销点核对

下表的"位置"在基线提交上逐条核对过。"处理"一栏指向本文的章节，或标为不采纳。

| 编号 | 位置 | 现行行为 | 处理 |
|---|---|---|---|
| R-1 | `aimux-providers/src/openai/chat/chat_language_model.rs:174,340,365` | 请求体用 `json!` 拼成 `Value`，传给 HTTP 层时 `body.clone()` 一份，再存一份进 `RequestInfo.body` 回显。HTTP 层自 #407 起用 `Arc<HttpBody>` 共享，错误上下文按需生成。 | §4.1 |
| R-2 | 同文件 `:341,478,480` | 流式响应先用 `create_event_source_response_handler::<Value>()` 把每个 SSE 事件解析成 `Value`，再在 `transform` 里 `serde_json::from_value::<StreamChunk>` 转一次；`usage` 先 `clone` 再转一次。 | §4.3 |
| R-3 | `aimux-provider-utils/src/response_handler.rs:245,259`；`read_response_with_size_limit.rs:79,144` | 成功的 JSON 响应先解析成 `T`，再整体解析一遍成 `Value` 填 `raw_value`（注释写明是给 `Usage.raw` 这类调用方用的）。限长读取从 `Vec::new()` 开始，不按 `Content-Length` 预留容量。 | §4.2 |
| R-4 | `aimux-stream/src/sse.rs:66`；`response_handler.rs:475` | 每个 SSE 事件分配一个 `SseEvent { data: String }`；事件流由 `async_stream::stream!` 宏生成。 | §4.3 |
| R-5 | `chat_language_model.rs:390,392`；`aimux-core/src/stream_part.rs`；`aimux-core/src/generate.rs:1862-1863` | 文本块 ID 是常量 `"0"`、`"reasoning-0"`，但 `StreamPart` 的 `id` 字段是 `String`，每个块都要分配一次；`generate.rs` 用 `HashSet<String>` 记录活跃块。 | §4.5（待定） |
| R-6 | `generate.rs:1712-1732` | `stream_text` 总是 `tokio::spawn` 一个 pump 任务，经 `mpsc::channel(256)` 把块交给消费者。 | 不采纳，§5.1 |
| R-7 | `generate.rs:147,1051,1568` | `tools: Option<Vec<Tool>>`，入口处 `options.tools.clone()`，每个工具的 `input_schema: Value` 整树复制。 | §4.4 |
| R-8 | `aimux-provider-utils/src/headers.rs:42,58` | `combine_headers` / `normalize_headers` 在 `HashMap`、排序 `Vec`、`BTreeMap`、`Vec<(String, String)>` 之间转换，逐个小写化。 | §4.6 |
| R-9 | `aimux-provider-utils/src/fetch.rs:258,344,496` | `default_transport()` 每次取全局 `RwLock` 的写锁（只为置 `frozen = true`）；`ReqwestFetch::client()` 每次锁 `Mutex`；`pool_max_idle_per_host(10)`、`pool_idle_timeout(30s)`。 | 不采纳，§5.2 |
| R-10 | `Cargo.toml:64`；`generate.rs:59,1626,1639`；`provider_registry.rs:50`；`aimux-providers/src/stream/chat_tool_calls.rs:99` | 工作区开启 `serde_json/preserve_order`；后备响应 ID 用 `rand::thread_rng()`；录制关闭时仍生成 `call_id`；`resolve_session_id` 先于 `session_store()` 求值；`ProviderRegistry` 用 `BTreeMap`；`ChatToolCalls::default` 每个流 `Arc::new` 一个闭包。 | 不采纳，§5.3–§5.5 |

草案中另有几处说法与源码不符，已从本文删除：

- **"未开启 `tcp_nodelay`"。** reqwest 0.12.28 的 `ClientBuilder` 默认 `nodelay: true`。
- **"5 层 `Box<dyn Stream>`"。** `transform_stream.rs:133` 的 `input: Pin<Box<S>>` 中 `S` 是具体类型，不是 trait 对象，每次 poll 没有虚调用；这个 `Box` 每个流分配一次，不是每块一次。
- **"建流时克隆 3 份响应头"。** 非流式 JSON 响应的多次提取由 #502 收到错误分支里；流式处理器建流时提取一次，每个流一次，不在每块路径上。

## 3. 原则

1. **上游有的照上游。** 默认值、选项名、回显内容与 `ai@7.0.122` 一致；上游没有的（录制、会话、传输冻结）按 aimux 自己的 RFC。
2. **一条路径。** 新实现替换旧实现，同一 PR 改完全部调用方，不留过渡代码，不加开关。
3. **改动净减或持平代码。** 为省一次每流分配而手写几十行状态机的，不做。
4. **先测后改。** 依据是 RFC-0043 §7 方法下的成对测量，不是推理出的纳秒数。

## 4. 方案

### 4.1 请求体不复制，step 结果默认不带请求体（R-1）

请求体在两层被使用，要分开处理：

- **provider 层。** `LanguageModel::do_generate` / `do_stream` 返回的 `RequestInfo.body` 必须有值：`aimux-core/src/trace/layer.rs:600,633` 包在 provider 外面，用它算缓存指纹（RFC-0015）。上游也是 provider 总返回 `request: { body }`。
- **core 层。** `generate_text` 的 step 结果和 `StreamTextResult.request_body` 是给调用方看的回显。上游 `generate-text.ts` 的 `include` 有 `requestBody`、`requestMessages`、`responseBody` 三个字段，`stream-text.ts` 有 `requestBody`，默认都是 `false`，为 `false` 时 step 结果不带请求体。

改动：

1. **provider 层去复制。** 现在 provider 先 `body.clone()` 一份传给 `post_json_to_api`（`aimux-provider-utils/src/post_to_api.rs:19`，按值接收），自己留一份放进 `RequestInfo.body`。改为 `post_json_to_api` 在成功时把请求体原样还给调用方：`PreparedRequest` 里的 `Arc<HttpBody>` 在请求结束后只剩一个持有者，`Arc::try_unwrap` 取回 `Value`，没有复制。错误路径照旧由 #407 的延迟脱敏处理。
2. **流式错误上下文。** 流式 provider 在流里保留一份请求体，用于流中途出错时填 `ApiCallError`（`self.ctx.request_body.clone()`：openai chat、groq、deepseek、bedrock、openai-compatible、anthropic、google、huggingface 等）。改为流上下文持有 `Arc<Value>`，只在出错时复制。这是 provider 内部类型，不影响公共 API。
3. **core 层对齐 `include`。** `GenerateTextOptions` 增加 `include: IncludeOptions`，字段与上游同名（`request_body`、`request_messages`、`response_body`），默认全 `false`；为 `false` 时 core 在拿到 provider 结果后丢弃请求体，不放进 step 结果，`generate.rs:1853` 的 `request_body.clone()` 随之删除。trace 层在 provider 层读取，不受影响。

第 3 点是破坏性改动：依赖 `step.request.body` 或 `StreamTextResult.request_body` 默认有值的调用方，要显式打开 `include.request_body`。仓库内除测试外没有这样的调用方；绑定通过 DTO 看到的 step 结果同样默认不再带请求体。

不改的部分：请求体仍用 `Value` 拼。把每个 provider 的请求换成带生命周期的 `#[derive(Serialize)]` 结构直接写字节，代码量是现在的数倍，而且 `transform_request_body` 之类的钩子按 `Value` 工作；先看 §4.1 做完后序列化在基准里的占比，再决定是否值得。

### 4.2 非流式响应只解析一次（R-3）

1. #502 合并后，`create_json_bytes_response_handler` 推广到全部 JSON provider（openai chat、anthropic、google、bedrock、mistral、cohere、openai-compatible 等），`create_json_response_handler` 删除。
2. `Usage.raw` 不能丢。做法是让 provider 的响应类型里 `usage` 字段直接反序列化成 `Value`（或 `Box<RawValue>`，看 `Usage.raw` 的类型），一次解析同时得到强类型字段和原始 usage，不再整体解析第二遍。
3. `read_response_with_size_limit` 在有 `Content-Length` 且不超过上限时按它预留容量；只有一个分块时直接返回该分块的 `Bytes`，不复制。

### 4.3 流式响应：每个 SSE 事件解析一次（R-2、R-4）

1. **provider 侧。** `create_event_source_response_handler::<Value>()` 改为 `::<StreamChunk>()`，`transform` 里的 `serde_json::from_value` 删除；`usage` 在 `StreamChunk` 里直接是 `Value` 字段，不再 `clone` 后转换。只有 `include_raw_chunks == Some(true)` 时才另解析一份 `Value` 给 `StreamPart::Raw`，这与上游 `includeRawChunks` 一致。
2. **按协议族分 PR。** 一个 PR 改完一个协议族的全部 provider（OpenAI chat 及兼容实现、Anthropic、Google、Bedrock、Responses），不在同一协议族里留两种写法。
3. **SSE 层。** `SseEvent.data` 在事件没有跨网络分块时借用分块字节（`Bytes::slice`，引用计数，无复制），跨分块时才拼接。`response_handler.rs:475` 的 `async_stream::stream!` 换成实现 `Stream` 的结构体，前提是换完后代码不多于现在；否则只做借用字节这一步。
4. **借用字符串必须用 `Cow`。** serde_json 不能把带转义（`\n`、`\"`、`\uXXXX`）的 JSON 字符串借用成 `&str`，会直接报 `expected a borrowed string`；`Cow<'a, str>` 遇到转义回退为 `Owned`，无转义时借用。模型输出的 delta 经常含换行，所以借用型块类型的字符串字段一律用 `Cow<'a, str>`，本文不承诺"每块零分配"。

### 4.4 工具集按引用共享（R-7）

`GenerateTextOptions::tools` 从 `Option<Vec<Tool>>` 改为 `Option<Arc<[Tool]>>`。`generate.rs:1051,1568` 的 `clone()` 变成引用计数加一。提供 `From<Vec<Tool>>`，但结构体字面量 `tools: Some(vec![...])` 要改成 `Some(vec![...].into())`，属于破坏性改动，按 RFC-0036 的一次性破坏原则在同一 PR 里改完仓库内全部调用方。

不做 schema 预编码成 `RawValue`：各 provider 在发送前会改写 schema（OpenAI strict 模式、Google 的 schema 转换），预编码的字节无法直接使用。

### 4.5 流块 ID 的类型（R-5，待定）

`StreamPart` 的 `id: String` 让常量 ID 每块分配一次。可选做法：

- **A.** `id` 改为 `Cow<'static, str>`。常量 ID 零分配，动态 ID 仍是 `String`。改公共类型，绑定的 DTO 生成（RFC-0042）要跟着改。
- **B.** 不改类型，只把 `generate.rs:1862-1863` 的 `HashSet<String>` 换成线性查找的小 `Vec`，省掉哈希表分配，ID 分配保留。

`delta` 保持 `String`：delta 内容来自网络，本来就要一份自有数据；引入 `CompactString` 之类的新依赖会改公共类型，收益要先测。

本文倾向 B，A 等 §6 的基准测出 ID 分配在每块开销里的占比后再定。

### 4.6 请求头合并（R-8）

保留现有语义：值为 `None` 表示删除该头；只差大小写的头名按原名字节序处理，结果与哈希表遍历顺序无关。在此前提下把 `combine_headers` + `normalize_headers` 合成一遍，直接产出 `http::HeaderMap`，省掉中间的 `BTreeMap` 和 `Vec`。优先级低，每请求一次，不在流式路径上。

## 5. 不采纳项

### 5.1 `stream_text` 的内联快路径（R-6）

pump 不只是转发。`generate.rs:1717` 起的注释写明它同时负责：

- **终结熔断。** 收到 `Finish` 或不可恢复的错误后一定结束流；否则 provider 在 `Finish` 后不关连接时，读到流结束的消费者会挂住。
- **期限与取消。** 首块期限、逐块期限（`chunk_ms`）、abort signal。
- **到达时间。** 给每块打到达时间，用于 step 的耗时统计。

草案的内联快路径只在"没有 `chunk_ms`、没有录制"时启用，没考虑 abort signal 和首块期限，而且会让熔断逻辑存在两份。2026-11-02 的并发评估（与 #405 同一轮）实测过 pump 计时器的开销，没有可测收益，改动未提交。除非 §6 的基准显示 pump 在每块开销中占主要部分，不再改动。

### 5.2 传输锁与连接池（R-9）

- **锁。** 2026-11-02 的同一轮评估实测过把 `default_transport` 改读锁、`client()` 去 `Mutex`，没有可测收益：每请求只取一次锁，热点是缓存行争用，不是等锁。锁在每请求一次，不在每块路径上。
- **连接池。** `pool_max_idle_per_host` 和 `pool_idle_timeout` 的默认值属于产品行为，不在本文改。需要高并发调优的 Rust 服务可以自己构造 `reqwest::Client` 注入 `Fetch`。是否在 `ReqwestFetch` 上开放这两个参数，留作 §7 的开放问题。
- **`http2_adaptive_window`、`http2_keep_alive_interval`。** 没有测量数据，不做。

### 5.3 去掉 `serde_json/preserve_order`（R-10）

不做。`Cargo.toml:64` 的 `preserve_order` 与 JS 对象的插入顺序对齐，`indexmap`、`schemars` 的 `indexmap2` 特性都依赖它。去掉之后：

- 工具 schema 的 `properties` 会按字母序发给上游，影响模型行为和 prompt cache 的键；
- `provider_metadata`、请求体回显、contract fixtures 的键序都会变。

### 5.4 后备 ID 改原子计数器、`ProviderRegistry` 改 `FxHashMap`

- **后备 ID。** 响应 ID 会写进录制和会话，进程内计数器在多进程或重启后会重复；随机数生成每次调用一次，不在每块路径上。
- **注册表。** 注册的 provider 通常只有个位数，`BTreeMap` 查找在每次调用一次，换哈希表测不出差别。

### 5.5 其余

- **录制关闭时生成 `call_id`（`generate.rs:1626`）。** `StreamTextResult.call_id` 是公开字段，总要有值，这次生成是必需的。
- **`resolve_session_id` 的求值顺序（`:1639`）。** 只有通过环境变量开启会话推断时才做实际工作，默认是一次 `OnceLock` 检查。
- **`ChatToolCalls::default` 的 `Arc::new`、`PipeThrough` 的 `Box`、`TransformStreamController` 的 `VecDeque`。** 都是每流一次或容量复用，不是每块分配。
- **`NativeLanguageModel` 静态分发接口。** 与 `dyn LanguageModel` 并存，是第二条路径。
- **宿主编译配置（`target-cpu=native`、mimalloc、`panic = "abort"`）。** 属于宿主服务自己的选择，不写进 aimux 的规范；RFC-0043 §7.1 也禁止在基准里使用 `target-cpu=native`。

## 6. 基准与验收

### 6.1 场景

在 `aimux-core/benches/` 下用内存 `Fetch`（直接返回预先构造的 `Bytes` 分块，不走网络），按 RFC-0043 §7 的方法测量：固定 2 个 Tokio worker，不开 `target-cpu=native`，base 与 candidate 成对运行。

| 场景 | 内容 | 记录 |
|---|---|---|
| `gen-30k-15tools` | 20 轮对话（约 30 KB）+ 15 个工具，返回 2 KB 补全 | 每次调用耗时；堆分配次数与字节数 |
| `stream-1k-chunks` | 1,000 个短文本 SSE 事件 | 每块耗时；稳定阶段（第 2 至第 999 块）每块堆分配次数 |
| `gen-tools-loop` | 同一组 30 个工具，连续 100 次调用 | 每次调用的工具准备耗时 |

堆分配用基准内的计数分配器统计，不加新的运行时依赖。

### 6.2 门槛

- 每个实现 PR 附上该 PR 影响的场景的成对结果，并说明分配次数的变化来自哪里。
- 测不出收益（差异落在 RFC-0043 §7.4 的噪声范围内）的改动不合入。
- 不写绝对数字门槛。草案的数字不可达：例如"`gen-30k-15tools` 全链路小于 3.5 µs"——serde_json 序列化吞吐约 1–2 GB/s，单是写出 30 KB 请求体就需要 15–30 µs。绝对护栏在第一批 PR 测出 base 基线后，按 RFC-0043 §10 的流程登记。

## 7. 开放问题

1. **流块 ID 的类型。** §4.5 选 A 还是 B。
2. **连接池参数。** 是否在 `ReqwestFetch` 上开放 `pool_max_idle_per_host` 和 `pool_idle_timeout`，还是只让调用方注入自己的 `Client`。
3. **`include` 的形状。** `IncludeOptions` 作为结构体（与上游对象一致），还是三个独立的 `bool` 字段。

## 8. 实施拆分

按 2026-11-12 起的四条工作线，下列 PR 都归 provider 线（§4.1、§4.4 同时改 core）。每个 PR 改完它影响的全部调用方。

| 顺序 | PR | 依赖 | 破坏性 |
|---|---|---|---|
| 1 | 基准场景（§6.1），先测出 base | — | 否 |
| 2 | 请求体移入移出、流上下文改 `Arc`、core 的 `include` 选项（§4.1） | 1 | 是 |
| 3 | 非流式单次解析推广到全部 provider，限长读取预留容量（§4.2） | #502 合并、1 | 否 |
| 4 | 流式单次解析，按协议族各一个 PR（§4.3 第 1、2 点） | 1 | 否 |
| 5 | SSE 事件借用分块字节（§4.3 第 3 点） | 4 | 否 |
| 6 | `tools: Option<Arc<[Tool]>>`（§4.4） | 1 | 是 |
| 7 | 流块 ID（§4.5，按开放问题 1 的结论）、请求头合并（§4.6） | 1 | A 方案是 |
