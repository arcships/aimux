# RFC-0050：纯 Rust 调用路径的补充项

> **Status**: Draft
>
> **Date**: 2026-11-12
>
> **Baseline**: master `c5a408370`（v0.6.1 之后）。文中 `文件:行号` 都指这个提交；上游引用 `ai@7.0.122`、`@ai-sdk/openai@4.0.80`（`reference/aisdk-pinned`）。
>
> **Scope**: Rust 服务直接依赖 `aimux-core` + `aimux-providers` 调用 `generate_text` / `stream_text` 时，[RFC-0049](https://github.com/arcships/aimux/blob/docs/rfc-0049-data-plane-performance/rfc/0049-data-plane-performance.md)（PR [#503](https://github.com/arcships/aimux/pull/503)，尚未合入；本分支上没有 `rfc/0049-*.md`，链接指向 #503 的分支）没有覆盖的部分。涉及 `aimux-core`、`aimux-providers`、`aimux-provider-utils`、`aimux-stream`。§3.1 的类型改动会传到 ops DTO（`GenerateTextOptions` 是 DTO，见 `aimux-ops/src/dto_vectors.rs:107`），线格式不变。
>
> **Related**: [RFC-0049](https://github.com/arcships/aimux/blob/docs/rfc-0049-data-plane-performance/rfc/0049-data-plane-performance.md)（数据面性能，含实测基线，PR #503）、[RFC-0036](0036-positioning-and-layered-architecture.md)、[RFC-0042](0042-v4-type-boundaries.md)（DTO 与 codegen）、[RFC-0043](0043-size-performance-budgets.md) §7（基准方法）、PR [#502](https://github.com/arcships/aimux/pull/502)、PR [#407](https://github.com/arcships/aimux/pull/407)。
>
> **来源**：修订自一份外部草案（"纯 Rust SDK 零开销引擎"）。草案列出的十处开销点（下文 R-1…R-10）逐条在源码上核对过；它与同一来源的 RFC-0049 草案大面积重叠，重叠部分以 RFC-0049 为准，本文只保留纯 Rust 调用方特有、RFC-0049 未处理的条目。

## 1. 决策摘要

1. **与 RFC-0049 重叠的条目以 RFC-0049 为准，本文不重新决定。** 包括 `include` 选项及默认值切换时点、单遍解析的推进顺序、`body.clone()` 不单独处理、连接池、`default_transport()` 读路径小修、不去掉 `preserve_order`、不做内联流泵、门禁口径（§2 表中逐条标出）。
2. **工具集按引用共享。** `GenerateTextOptions::tools` 改为 `Option<Arc<[Tool]>>`，每次调用和每个 step 的 `tools.clone()` 变成引用计数加一。线格式不变，Rust 源码层（含 `CallOptions` 与 `LanguageModel` 实现方）是破坏性改动，只能放进 RFC-0049 阶段 2 的 0.7.0 破坏性窗口，不在 0.6.x 提前（RFC-0049 阶段 1 要求不破坏类型）（§3.1）。
3. **借用型流块的字符串字段用 `Cow<'a, str>`。** 这是对 RFC-0049 §3.4"借用型 chunk 结构"的约束：serde_json 不能把带转义的字符串借用成 `&str`（§3.2）。
4. **流块 ID 与活跃块集合**、**请求头合并**两项按 §3.3、§3.4 处理，先测后定。
5. **只保留一条路径。** 不新增 `HttpBody::WireJson`、内联流快路径、静态分发的 `NativeLanguageModel` 等与现有实现并存的路径（§4）。

## 2. 开销点核对与归属

| 编号 | 位置 | 现行行为 | 处理 |
|---|---|---|---|
| R-1 | `aimux-providers/src/openai/chat/chat_language_model.rs:174,340,365` | 请求体用 `json!` 拼成 `Value`，`body.clone()` 一份传给 `post_json_to_api`（`aimux-provider-utils/src/post_to_api.rs:19`，按值接收），自己留一份放进 `RequestInfo.body`。HTTP 层自 #407 起用 `Arc<HttpBody>` 共享。 | RFC-0049 §3.4：不单独处理（实测占比不到 1%，约 86 个调用点）。step 结果的回显由 RFC-0049 §3.1 的 `include` 处理。 |
| R-2 | 同文件 `:341,474-480` | 每个 SSE 事件先解析成 `Value`，再 `from_value::<StreamChunk>`；`usage` 子树 `clone` 后再转一次。 | RFC-0049 §3.4 定推进顺序；本文 §3.2 补充约束。 |
| R-3 | `aimux-provider-utils/src/response_handler.rs:245,259`；`read_response_with_size_limit.rs:79,144` | 成功响应解析成 `T` 后再整体解析一遍 `Value`；限长读取从 `Vec::new()` 开始。 | RFC-0049 §3.1、§3.4。 |
| R-4 | `aimux-stream/src/sse.rs:66`；`response_handler.rs:475` | 每个 SSE 事件分配 `SseEvent { data: String }`；事件流由 `async_stream::stream!` 生成。 | RFC-0049 §3.4：先量出它在每帧成本中的份额。 |
| R-5 | `chat_language_model.rs:390,392`；`aimux-core/src/stream_part.rs`；`aimux-core/src/generate.rs:1862-1863` | 常量 ID `"0"`、`"reasoning-0"` 写进 `id: String`，每块分配；`generate.rs` 用 `HashSet<String>` 记活跃块。 | 本文 §3.3。 |
| R-6 | `generate.rs:1712-1732` | `stream_text` 总是 `tokio::spawn` 一个 pump，经 `mpsc::channel(256)` 交给消费者。 | RFC-0049 §8：不做内联流泵。理由补充见 §4.1。 |
| R-7 | `generate.rs:147,1051,1568` | `tools: Option<Vec<Tool>>`，入口 `options.tools.clone()`，每个工具的 `input_schema` 整树复制。 | 本文 §3.1。 |
| R-8 | `aimux-provider-utils/src/headers.rs:42,58` | `combine_headers` / `normalize_headers` 在 `HashMap`、排序 `Vec`、`BTreeMap`、`Vec<(String, String)>` 之间转换，逐个小写化。 | 本文 §3.4。 |
| R-9 | `aimux-provider-utils/src/fetch.rs:258,344,496` | `default_transport()` 每次取写锁置 `frozen`；`client()` 每次锁 `Mutex`；`pool_max_idle_per_host(10)`。 | RFC-0049 §1.7、§3.6：锁改读路径加原子标志列为小修；连接池调高或可配置。 |
| R-10 | `Cargo.toml:64`；`generate.rs:59,1626,1639`；`provider_registry.rs:50`；`aimux-providers/src/stream/chat_tool_calls.rs:99` | 开启 `serde_json/preserve_order`；后备响应 ID 用 `rand::thread_rng()`；录制关闭时生成 `call_id`；`resolve_session_id` 先于 `session_store()` 求值；注册表用 `BTreeMap`；`ChatToolCalls::default` 每流 `Arc::new` 一个闭包。 | `preserve_order` 见 RFC-0049 §0.3；其余见本文 §4.3。 |

### 2.1 对草案的事实更正

- **`tcp_nodelay`。** reqwest 0.12.28 的 `ClientBuilder` 默认 `nodelay: true`，草案说"未开启"不对（RFC-0049 §0.3 同）。
- **录制关闭时生成 `call_id`。** 草案说得对，但这次调用是必需的：`generate.rs:1624-1626` 的 `step_call_id = call_id.clone().unwrap_or_else(crate::recording::new_call_id)` 在录制关闭时也会调用，因为 `StreamTextResult::consume()` 把它写进公开字段 `StepResult.call_id`（`aimux-core/src/step_result.rs:223`），总要有值；`StreamTextResult.call_id`（`:2050`）本身是私有字段，没有公开访问器。不处理。RFC-0049 §8 的对应条目已按此更正（#503）。
- **未核实的草案说法。** "单条流穿过 5 层 `Box<dyn Stream>`"、"建流时克隆 3 份响应头"本文没有逐层核对。后者 RFC-0049 §3.4 已定为每流一次、不处理；前者在 §5 的基准里按每块耗时判断，不按层数判断。

## 3. 方案

### 3.1 工具集按引用共享（R-7）

`GenerateTextOptions::tools` 从 `Option<Vec<Tool>>` 改为 `Option<Arc<[Tool]>>`：

- `generate.rs:1051,1568` 的 `clone()` 变成引用计数加一；Agent 循环里反复使用同一组工具时，不再每次复制所有 `input_schema`。
- `GenerateTextOptions` 派生 `Serialize`、`Deserialize`、`JsonSchema`，是 ops DTO。`Arc<[T]>` 的序列化要打开 serde 的 `rc` 特性（工作区 `Cargo.toml:63` 现在只开了 `derive`）；JSON 形状仍是数组，DTO schema 与线格式不变，要用 `aimux-ops` 的 DTO 往返测试确认。
- **`CallOptions::tools` 一起改。** `into_call_options`（`generate.rs:243`）把 `self.tools` 按值移进 `CallOptions::tools`（`aimux-core/src/options.rs:136`，`Option<Vec<Tool>>`）。只改 `GenerateTextOptions` 的话，这里要 `to_vec()` 一次，所有 `input_schema` 照样整树复制，省下的只是 `generate.rs:1051,1568` 那一份。所以 `CallOptions::tools` 也改成 `Option<Arc<[Tool]>>`。`CallOptions` 是 `LanguageModel` trait 的入参，这对仓库外自己实现 provider 的调用方同样是破坏性改动。
- **改写 `call.tools` 的地方。** Bedrock（`aimux-providers/src/bedrock/chat_language_model.rs:1358`）`call.tools.clone().unwrap_or_default()` 后追加 provider 工具，改为 `to_vec()` 后追加，只在这条路径上复制，与现行相同；Google 图像模型（`aimux-providers/src/google/image_model.rs:195`）整体赋值 `call.tools = Some(vec![...])`，改为 `.into()`。`ToolCallRepairContext::tools`（`aimux-core/src/parse_tool_call.rs:30`，`Vec<Tool>`）只在工具修复时构造，不在本文范围。
- Rust 源码层是破坏性改动：结构体字面量 `tools: Some(vec![...])` 要改成 `Some(vec![...].into())`。提供 `From<Vec<Tool>>`，按 RFC-0036 的一次性破坏原则在同一 PR 里改完仓库内全部调用方，排在 RFC-0049 阶段 2 的 0.7.0 窗口。

不做 schema 预编码成 `RawValue`：各 provider 发送前会改写 schema（OpenAI strict 模式、Google 的 schema 转换），预编码的字节不能直接使用。工具 schema 校验器的缓存归 RFC-0049 §3.6（按 schema 哈希的全局有界缓存）。

### 3.2 借用型流块的字符串字段（R-2、R-4 的补充约束）

RFC-0049 §3.4 把"零拷贝解析器与借用型 chunk 结构"排在批量之后。本文补充一条约束：**借用字段一律用 `Cow<'a, str>`，不用 `&'a str`。**

serde_json 只能在 JSON 字符串没有转义时借用。带 `\n`、`\"`、`\uXXXX` 的字符串反序列化成 `&str` 会直接报错 `invalid type: string "a\nb", expected a borrowed string`；`#[serde(borrow)] Cow<'a, str>` 无转义时借用，有转义时回退为 `Owned`（2026-11-12 用 serde_json 1 实测）。模型输出的 delta 经常含换行，用 `&str` 会让普通的流式响应解析失败，所以借用型结构不能承诺"每块零分配"。

单遍解析时 `usage` 保持现在的宽松语义：`chat_language_model.rs:476-478` 对 `usage` 的解析失败只得到 `None`，不让整个块失败。单遍版本的 `usage` 字段要用 `Option<Value>`（或 `Box<RawValue>`）接收原始对象，再单独宽松地转成 `UsageResponse`，不能直接声明成强类型字段。

### 3.3 流块 ID 与活跃块集合（R-5，待定）

`StreamPart` 的 `id: String` 让常量 ID 每块分配一次。两个做法：

- **A.** `id` 改为 `Cow<'static, str>`。常量 ID 不分配，动态 ID 仍是 `String`。`StreamPart::into_text_stream_part`（`aimux-core/src/stream_part.rs:461`）把 `id` 原样移进 `TextStreamPart`（`stream_part.rs:189`，`id: String`），只改 `StreamPart` 的话这里要 `into_owned()`，常量 ID 照样分配一次，所以 `TextStreamPart` 的 `id` 也要改成 `Cow<'static, str>`。两者都是公共类型，DTO 生成（RFC-0042）跟着改，线格式不变。
- **B.** 不改类型，只把 `generate.rs:1862-1863` 的 `HashSet<String>` 换成线性查找的小 `Vec`。同一时刻活跃的文本块、推理块通常各一个，省掉哈希表分配与哈希计算，ID 分配保留。

`delta` 保持 `String`：内容来自网络，本来就要一份自有数据；换成 `CompactString` 之类会引入新依赖并改公共类型。

倾向 B。A 等 §5 的 `stream-1k-chunks` 测出 ID 分配在每块成本（RFC-0049 §1.5：core 管线约 1.76 µs / 帧）中的占比后再定。

### 3.4 请求头合并（R-8）

保留现有语义：值为 `None` 表示删除该头；只差大小写的头名按原名字节序处理，结果与哈希表遍历顺序无关。在此前提下新增一个内部函数，把合并与规范化合成一遍，直接产出 `http::HeaderMap`，省掉中间的 `BTreeMap` 与 `Vec`，HTTP 层改用它。`combine_headers` / `normalize_headers` 是 `aimux-provider-utils` 的公开函数（`lib.rs:71-72` 重导出），保留原签名与语义作为包装，因此不破坏（§7 表中标为“否”）。每请求一次，不在流式路径上，排在最后，测不出收益就不做。

## 4. 不采纳项

RFC-0049 已否决的（`preserve_order`、内联流泵、按包 `opt-level`、无锁句柄表等）不在这里重复；下面只列纯 Rust 草案特有的条目，以及对 RFC-0049 理由的补充。

### 4.1 内联流泵：补充理由（R-6）

除 RFC-0049 §3.3 的理由外，pump 还承担这些职责（`generate.rs:1712-1722` 的注释）：

- **终结熔断。** 收到 `Finish` 或不可恢复的错误后一定结束流；否则 provider 在 `Finish` 后不关连接时，读到流结束的消费者会挂住。
- **期限与取消。** 首块期限、逐块期限（`chunk_ms`）、abort signal。
- **到达时间。** 给每块打到达时间，用于 step 的耗时统计。

草案的快路径只在"没有 `chunk_ms`、没有录制"时启用，没考虑 abort signal 和首块期限，而且熔断逻辑会存在两份。pump 的 spawn 与 channel 在每块成本里占多少没有单独测过；如果 §5 的基准显示它是主要部分，再另行提出不分叉的改法。

### 4.2 并行路径

- **`HttpBody::WireJson` 与 `Json` 并存。** 把各 provider 的请求换成带生命周期的 `#[derive(Serialize)]` 结构，代码量是现在的数倍，而且 `transform_request_body` 等钩子按 `Value` 工作。RFC-0049 已决定 `aimux-ops` 出向直写，provider 请求体仍用 `Value`。
- **`NativeLanguageModel` 静态分发接口。** 与 `dyn LanguageModel` 并存，是第二条路径。

### 4.3 其余

- **后备响应 ID 改进程内计数器（`generate.rs:59`）。** 它填的是公开的 `response.id`；计数器在多进程或重启后会重复。每次调用一次，不在每块路径上。
- **`ProviderRegistry` 改 `FxHashMap`。** 注册的 provider 通常个位数，查找每次调用一次。
- **`resolve_session_id` 的求值顺序（`:1639`）。** 没开会话推断时只做一次 `OnceLock` 检查；开了推断时推断器（`session.rs:367` 起，带锁）有自己的状态，先判断 store 再推断会跳过无 store 时的推断调用，需要先确认推断器是否依赖每次调用都被调用。
- **`ChatToolCalls::default` 的 `Arc::new`、`PipeThrough` 的 `Box`、`TransformStreamController` 的 `VecDeque`。** 每流一次，或容量复用，不在每块分配路径上。
- **宿主编译配置（`target-cpu=native`、mimalloc、`panic = "abort"`）。** 宿主服务自己选择；mimalloc 对 aimux 自身产物的影响归 RFC-0049 §1.8、§3.6。RFC-0043 §7.1 禁止在基准里使用 `target-cpu=native`。

## 5. 基准与验收

门禁口径以 RFC-0049 §5 为准（确定性指标做硬门禁，耗时只做成对比较），测量方法按 RFC-0043 §7。本文在 RFC-0049 的三组基准之外补两个纯 Rust 场景，都用内存 `Fetch`：

| 场景 | 内容 | 记录 | 对应 |
|---|---|---|---|
| `gen-tools-loop` | 同一组 30 个工具，连续 100 次 `generate_text` | 每次调用的分配次数与字节数 | §3.1 |
| `stream-1k-chunks` | 1,000 个短文本 SSE 事件 | 稳定阶段（第 2 至第 999 块）每块分配次数；每块耗时 | §3.2、§3.3 |

每个实现 PR 附上受影响场景的成对结果，说明分配次数的变化来自哪里；测不出收益的不合入。

草案的绝对数字门槛全部不采用。例如"20 轮 30 KB 对话 + 15 个工具的 `generate_text` 全链路小于 3.5 µs"：按 serde_json 序列化吞吐约 1–2 GB/s 估算（未测），单是写出 30 KB 请求体就要 15–30 µs；RFC-0049 §1.6 实测的 core 段也在百微秒量级。

## 6. 开放问题

1. **流块 ID。** §3.3 选 A 还是 B。

## 7. 实施拆分

都归 provider 线，排在 RFC-0049 的对应阶段之后或之中：

| 顺序 | PR | 依赖 | 破坏性 |
|---|---|---|---|
| 1 | §5 的两个基准场景，先测 base | RFC-0049 阶段 0 | 否 |
| 2 | 活跃块集合改小 `Vec`（§3.3 B） | 1 | 否 |
| 3 | `GenerateTextOptions` 与 `CallOptions` 的 `tools: Option<Arc<[Tool]>>`，开 serde `rc`（§3.1），0.7.0 | 1 | 是（Rust 源码） |
| 4 | 借用型流块（§3.2），随 RFC-0049 §3.4 的零拷贝解析器一起做 | RFC-0049 阶段 1 的批量完成 | 否 |
| 5 | 请求头合并（§3.4） | 1 | 否 |
