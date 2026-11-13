# RFC-0049：数据面性能演进（返回包瘦身、去中间 `Value` 树、流式批量过桥）

> **Status**: Draft
>
> **Date**: 2026-11-12
>
> **Baseline**: `master@c5a40837`。它不是 v0.6.1：v0.6.1 tag 是 `164d7f4b`，`c5a40837` 在其后又合入了 18 个 PR，其中 #407、#408 是性能改动。文中“现行”与 §1 的数字均指 `c5a40837`。2026-11-13 在两者上复测（§1.10）：返回包字节数相同（只差 `performance` 计时字段的位数，20 B 以内）；用 `bindings/node/bench/bench_profile.cjs` 交替跑 4 轮，6 个场景的中位数差异都在轮间波动范围内。[#502](https://github.com/arcships/aimux/pull/502)（`a1be8777`，草案）是本文 §6 阶段 1 的第一批改动，未合入；涉及它的数字单独标注。
>
> **Scope**: `aimux-ops`、`aimux-core`、`aimux-provider-utils`、`aimux-providers`、`aimux-ffi`、`tools/aimux-codegen`、`bindings/*`。定义瓶颈账本、设计方向、分阶段顺序和验收口径；不改变任何 provider 行为、安全限制或回放能力。
>
> **Related**: [RFC-0039](0039-ops-bindings-contract.md)（ops 与绑定契约）、[RFC-0042](0042-v4-type-boundaries.md)（DTO 与唯一生成链）、[RFC-0043](0043-size-performance-budgets.md)（体积预算与性能门禁 P）、[RFC-0036](0036-positioning-and-layered-architecture.md)（分层与 L0 passthrough）、[RFC-0015](0015-cache-trace-audit.md)（trace 指纹）、[RFC-0023](0023-runtime-request-recording.md)（录制）。
>
> **本稿的来历**：修订自一份外部草案。草案里的每条源码断言都对照 `c5a40837` 核过，每个数字都换成了本文 §1 的实测或标为“未测”。草案中被否决或改写的条目列在 §8。

## 0. 摘要

### 0.1 定性

aimux 自己不做模型计算。一次调用里 SDK 的 CPU 只花在两件事上：搬运字节（JSON / SSE 编解码、拷贝、分配）和跨边界调度（FFI、tokio 任务切换、宿主事件循环唤醒）。§1 的采样与这个判断一致：大包场景不走网络时约 70% 的时间在结果序列化和释放上；流式每帧的成本基本等于一次跨线程往返。

### 0.2 决定

| # | 决定 | 依据 |
|---|---|---|
| D1 | 新增 `include: { requestBody, responseBody }`，与上游 AI SDK 同名同义；默认值最终为 `false`（上游默认即 `false`） | §1.3、§3.1 |
| D2 | `GenerateTextResult` 的序列化只写事实集（`steps`、`totalUsage`、`output`），其余字段由 codegen 的 7 个目标语言生成的 getter 从 `steps` 派生；C ABI 调用方自行从 `steps` 取（§3.1） | §1.3 原型实测 |
| D3 | `aimux-ops` 出向不再先建 `serde_json::Value` 树；入向随后 | §1.4 |
| D4 | 流式读取统一“取走所有已就绪的帧、不等待凑批”；成帧改为单遍有界写入 | §1.5 原型实测 |
| D5 | 性能门禁用确定性指标（返回包字节数、每请求分配次数与字节数）做硬门禁，耗时只做成对比较 | §1.1、§5 |
| D6 | 全局分配器换 mimalloc 先在真实绑定里验证，再决定 | §1.8 |
| D7 | 锁、URL 解析、响应头等归为小修，不设独立阶段 | §1.7、§1.4 |

### 0.3 明确不做

- **按包覆盖 `opt-level = 3`**：RFC-0043 §15.8 已实测否决（`"z"` 加三个热点包覆盖后 22.8 MB，仍慢 10%）。Node / Python 绑定现行已是 `"s"`。
- **去掉运行时的 `serde_json/preserve_order`**：运行时依赖键的插入顺序（`aimux-core/src/shared.rs`、`response_messages.rs` 的注释与测试；根 `Cargo.toml` 对 `indexmap` 的说明）。去掉会改变输出与请求体的键序，影响录像回放与缓存指纹。
- **`.tcp_nodelay(true)`**：reqwest 0.12.28 默认即为 `true`。
- **`.http2_adaptive_window(true)`**：工作区的 reqwest 关闭了默认 feature 且未启用 `http2`，该方法不存在；现行只走 HTTP/1.1。是否启用 HTTP/2 是 §7 的待决问题。
- **无锁分代句柄表、32 分片缓存行对齐**：§1.7 表明锁不是瓶颈。
- **以“单请求十微秒级”为目标**：§1.6 的下限不支持。

### 0.4 非目标

- 不删减 provider、模态接口、录制 / 回放 / trace 能力。
- 不放宽 SSRF 防护、错误上下文脱敏、1 MiB 单帧上限、8 MiB 队列背压。
- 不承诺未经本文 §1 方法测过的数字。

## 1. 实测基线

### 1.1 环境、方法与局限

- **环境**：4 核 Linux 容器（虚拟化），Node 22，rustc 1.97。Node 绑定用 `opt-level = 3`、无 LTO 构建；RFC-0043 §15.8 测得 `"s"` + LTO 与它在噪声内。
- **负载**：小请求为 `"hi"` + 几十字节响应。大包的响应文本为 50,000 字符；prompt 有两种，**两节的返回包大小因此不同**：
  - §1.2 沿用 7 月基准（`96dba455d` 的 `bindings/node/bench/bench-constrained.ts`）的 `makeContext(200_000)`：195,973 字符，含换行，按当时的方式 `generateText(JSON.stringify(prompt))` 传入。
  - §1.3 起为单个 200,000 字符的字符串。默认模型走 OpenAI Responses 路径（`provider.model("gpt-4o")` 现行返回 Responses 模型）。
- **三种测法**：
  - Node 端到端：独立进程的本地 HTTP mock，按并发度扫描，记录吞吐、客户端进程 CPU、JS 线程占用。
  - Rust 层差分：直接调 `generate_text`，注入内存 `Fetch`（不走网络）对比走 localhost 的 reqwest。
  - 分桶采样：SIGPROF 采样（`pprof` crate），按符号名自叶向根归桶。每组约 1,500–2,600 个样本，1 个百分点以内的差别不可靠。
- **局限**：
  - 绝对值只在本机有意义，本文只使用同机比例。
  - 同一构建两轮之间吞吐可差 30%（RFC-0043 §15.8 记录过 6–40%）。
  - mock 与客户端同机抢核。
  - 32 核扩展性没有测。
  - 基准脚本尚未入库，是阶段 0 的交付物。
  - **不满足 RFC-0043 §7.1**：Node 22、rustc 1.97 只记到主版本，没有锁定 patch 版本与依赖 lock digest；4 核是虚拟化容器，绑定进程与 mock 没有固定 affinity，Tokio worker 数没有固定。因此 §1 的数字只作立项依据，不作正式基线；正式基线由阶段 0 在 §7.1 的环境中产出。

### 1.2 相对 README 数据的回归

`README.md` 与 `docs/PERF-RESULTS.md` 的数字测于 2026-07-30（`c4748178`），用的是 ops 层之前的接口，循环是串行 `await`（并发度 1，mock 在同一个 Node 进程里），所以“1 核、2 核、32 核吞吐持平”是测法决定的，不是锁。把四个时间点的 Node 绑定用同一配置编出，按当时的测法在同一台机器上各跑两轮取平均：

| 版本 | 小请求平均耗时 | 大包吞吐 | 大包返回 JSON |
|---|---:|---:|---:|
| `c4748178`（2026-07-30，README 数据对应） | 0.234 ms | 637 rps | 297,414 B |
| `97830b59`（2026-10-17，ops 层之前） | 0.377 ms | 418 rps | 846,382 B |
| `c5a40837`（现行） | 0.493 ms | 378 rps | 846,380 B |
| #502 | 0.383 ms | 395 rps | 846,393 B |

返回 JSON 一列 2026-11-13 复测为 297,438 / 846,331 / 846,331 / 846,341 B（同样的 prompt 与 50,000 字符响应；`c4748178` 默认走 Chat，用 Chat 形状的 mock，其余走 Responses），与表中只差计时字段的位数。

现行相对 7 月：小请求慢约 2.1 倍，大包吞吐低约 1.7 倍。下降主要发生在 ops 层之前（返回包变大、默认接口由 Chat 改为 Responses）。上表按当时的测法不解析结果字符串；按 0.6 TS 客户端的实际路径（`JSON.stringify` + `JSON.parse`），现行大包为 251–289 rps。

### 1.3 返回包体积

大包场景（单个 200,000 字符 prompt）一次 `text.generate` 的返回 JSON 是 853,082 B，是 50,000 字符响应文本的 17 倍（2026-11-13 复测，经 Node 绑定的 ops 接口）：

| 字段 | 字节 |
|---|---:|
| 顶层 `request`（body 200,088） | 200,097 |
| 顶层 `response`（body 50,288 + messages 50,111） | 100,661 |
| 顶层 `content` / `text` / `output` / `responseMessages` | 50,079 / 50,002 / 50,002 / 50,111 |
| `steps[0]`（`request` 200,097、`response` 100,661、`content` 50,079） | 351,519 |

原型（同一二进制，环境变量切换）只写 `steps` 与 `totalUsage`、并去掉两处 body。下表是原测量：其中“现行”的 854,637 B 比上面的复测多 1,555 B，差在 `request`（顶层与 `steps[0]` 各多 772 B）。原测量的 prompt 内容没有留存；原型代码也没有留存，下表的原型数字未复测：

| | 现行 | 原型 |
|---|---:|---:|
| 返回 JSON | 854,637 B | 101,324 B |
| 吞吐，并发 1 | 276 rps | 486 rps |
| 吞吐，峰值（并发 8） | 560 rps | 1,318 rps |
| 客户端 CPU / 请求 | 约 3.4 ms | 约 1.9 ms |

只去掉 body、保留顶层派生字段：853,082 − 2 × 200,088 − 2 × 50,288 = 352,330 B（按复测的字段大小计算，未单独实测）。原型里剩下的 101 KB 是 `steps[0].content` 与 `steps[0].response.messages` 各一份文本。

大包并发 2 以上吞吐不再上升，此时 JS 线程占用 100%：瓶颈是主线程上的 `JSON.stringify` / `JSON.parse` 与 napi 字符串进出，随返回包大小线性增长。

### 1.4 不走网络时的时间分布

Rust 层差分（现行，Responses 路径）：

| 场景 | 内存 `Fetch` | localhost | 其中结果 `to_value` + `to_string` |
|---|---:|---:|---:|
| 小请求 | 144 µs | 355 µs | 约 37 µs（结果 3.2 KB） |
| 大包 | 1,493 µs | 2,027 µs | 约 950 µs（结果 855 KB） |

分桶（内存 `Fetch`）：

| 小请求（约 154 µs） | 占比 | 大包（约 1.6 ms） | 占比 | 其中 memcpy |
|---|---:|---|---:|---:|
| 基准自身的任务调度与唤醒 | 23% | 结果 `Value` 树转字符串 | 39.9% | 15.3% |
| 结果先建 `Value` 树再转字符串 | 21% | 结果转 `Value` 树 | 22.0% | 17.2% |
| 值的释放 | 11% | 值的释放 | 8.3% | 0 |
| core 其余（步骤组装、选项、计时器） | 11% | 请求体转字节 | 7.7% | 0.7% |
| 响应 JSON 解析（两遍） | 5.5% | core 里的 prompt 转换 | 2.8% | 2.6% |
| `supported_urls()` 克隆 | 5.3% | 响应 JSON 解析（两遍） | 1.9% | 0.2% |
| URL 解析 | 3.1% | 无用的 `raw_value.to_string()` | 1.6% | 0.2% |
| 请求 / 响应头处理 | 2.8% | | | |
| 无用的 `raw_value.to_string()` | 2.6% | | | |

表中各桶的 memcpy 合计 36.2%，其中结果序列化两桶（结果 `Value` 树转字符串、结果转 `Value` 树）占 32.5 个百分点，约九成。**memcpy 一列没有复现**（§1.10）：其他平台上 memcpy 合计为 15–20%，结果序列化两桶只占其中一半左右。memcpy 的占比取决于平台的 memcpy 实现与文本内容，本文的决定不以它为依据。复现的是结果序列化两桶的合计占比（上表 61.9%，复测 63.7–67.3%）。

OpenAI Chat 路径（#502 头部）分布相同：大包结果序列化两桶合计 63.6%，响应解析两遍 2.7%（小请求 5.3%），请求体 `Value` 克隆不到 1%。

本节的基准不经过 `aimux-ops`，所以入向 `from_str → Value → from_value` 的成本没有数。

### 1.5 流式

| 测点 | 每帧 |
|---|---:|
| Node，现行（5,000 个 delta 一次写出） | 32.7–38.7 µs 墙钟，52–67 µs CPU，约 2.6–3.1 万帧 / 秒 |
| Node，原型：`next()` 取走所有已就绪帧，上限 64 | 5.7 µs 墙钟，11.9 µs CPU，约 17.5 万帧 / 秒 |
| Rust，内存 SSE，仅 core 管线 | 1.76 µs |
| Rust，同上加现行成帧（`to_value`、两次计数、一次序列化） | 2.81 µs |

原型平均每批约 16 帧。现行成帧的四遍处理单独测约 0.2 µs / 帧，放回管线约 1 µs / 帧：批量之前它不到每帧成本的 3%，批量之后接近两成。

批量只对已就绪的帧有效。单条 50–100 token / 秒的流每批只有 1 帧，行为与现行一致，也没有收益；收益在高速后端和高并发流。

### 1.6 下限

- 一次不发 HTTP 的 napi 异步往返（`model.info`）：并发 1 时 48 µs 墙钟、64 µs CPU；并发 16 时 25 µs CPU。
- HTTP 层（reqwest / hyper / loopback）：小请求约 200 µs 墙钟、约 140 µs CPU。

这两项本文的手段都压不动。能压的是 core 加结果序列化那约 137 µs。

### 1.7 并发与锁

- 小请求吞吐随并发上升：并发 1 为 2,503 rps，并发 16 为 10,874 rps，此时 4 个核已被客户端与 mock 占满。
- 把 `default_transport()` 的 `RwLock::write` 与 `HandleRegistry` 的 `Mutex<HashMap>` 单独拿出来争用：单线程 22–28 ns / 次；4–8 线程死循环争抢仍有约 400 万次 / 秒。每个请求只过几次锁。

### 1.8 分配器

独立 Rust 可执行文件（#502 头部），内存 `Fetch`，两轮平均：

| 场景 | glibc | mimalloc | 变化 |
|---|---:|---:|---:|
| 小请求 | 131 µs | 101 µs | −23% |
| 大包 | 1,427 µs | 937 µs | −34% |
| 流式每帧（含成帧） | 2.88 µs | 2.10 µs | −27% |

未测：装进 Node / Python 动态库后的效果、对产物体积的增量。它省下的分配有一部分正是 D2、D3 会消除的，收益不能与之相加。

### 1.9 #502

内存 `Fetch`，改动前后各两轮平均，只看 `generate_text` 这一段（结果序列化未改）：小请求 −18%，小 prompt + 50 KB 响应 −27%，大包 −18%。Node 端到端见 §1.2 表：小请求 0.493 → 0.383 ms（−22%），大包吞吐 378 → 395 rps（+4.5%）。这两项都只是两轮平均，仍在 §1.1 记录的同构建两轮间最多 30% 的波动范围内，本文不据此下结论。

### 1.10 复测（2026-11-13）

针对评审意见做的复测。返回包字节数是确定性的，与机器无关；耗时与采样分布只在同机比较有意义。

- **返回包字节数**（macOS arm64，Node 26.8.2，rustc 1.99.1，`opt-level = 3` 无 LTO 构建的 Node 绑定，进程内 localhost mock）：

  | 提交 | §1.2 的 prompt | 单个 200,000 字符 prompt |
  |---|---:|---:|
  | `c4748178`（Chat） | 297,438 | 300,808 |
  | `97830b59` | 846,331 | 853,068 |
  | `164d7f4b`（v0.6.1 tag） | 846,341 | 853,070 |
  | `c5a40837` | 846,331 | 853,082 |
  | #502（`a1be8777`） | 846,341 | 853,069 |

  同一提交多次运行之间相差不超过 20 B，全部来自 `steps[0].performance` 里计时数字的位数。§1.2 与 §1.3 原文的 846,380 与 854,637 不矛盾，是两种 prompt 的结果。

- **采样分布**：用 `pprof` 在内存 `Fetch` 上调 `generate_text`，再做 ops 路径的 `to_value` + `to_string`，按调用栈分桶。

  | 环境 | 大包每次 | 结果序列化两桶 | memcpy 合计 | 其中序列化两桶 |
  |---|---:|---:|---:|---:|
  | 原测量（§1.4） | 约 1.6 ms | 61.9% | 36.2% | 32.5 pp |
  | Linux arm64（glibc 2.36，rustc 1.100.0，Docker 限 4 CPU），两轮 | 435–505 µs | 63.7–64.0% | 15.1–19.5% | 7.5–10.3 pp |
  | macOS arm64，两轮 | 380–384 µs | 66.2–67.3% | 未取得（系统 memmove 的帧没进调用栈） | — |

  结论：“大包时约六成以上的时间在结果序列化”可以复现；memcpy 的比例不能复现。小请求的分布与 §1.4 也不同（复测中结果序列化占 31–38%，§1.4 为 21%），两者的基准驱动方式不同（§1.4 有 23% 落在“基准自身的任务调度与唤醒”），不能直接比较。

- **v0.6.1 tag 与 `c5a40837` 的耗时**（macOS arm64，`bench_profile.cjs`，每版 4 轮、每轮 300 次，取中位数的中位数）：`text.generate` 146 / 138 µs，`text.generate(64KB)` 451 / 417 µs，`text.stream(400 deltas)` 5,801 / 5,555 µs，各版本轮间的最小–最大范围相互重叠。这台机器上测不出 #407、#408 带来的差别。

- **仍然缺少**：符合 RFC-0043 §7.1 的环境（独占物理核、固定 affinity 与工具链 patch 版本）。本节与 §1.1–§1.9 一样，只作立项依据。

## 2. 现状核实

逐条对照 `c5a40837`。“代价”一栏引用 §1。

| 编号 | 位置 | 现行行为（已核实） | 实测代价 |
|---|---|---|---|
| B-1 | `aimux-core/src/generate.rs`（`GenerateTextResultSnapshot`） | 顶层写一遍派生字段和最后一步的 `request` / `response`，`steps` 里再写一遍 | 返回包 17 倍于响应文本；大包 Rust 侧约 70%（§1.3、§1.4） |
| B-2 | `aimux-ops/src/ops/text.rs`、`frame.rs`、`limits.rs`，`bindings/node/src/bridge.rs`，`aimux-ffi/src/abi.rs` | 每帧 `to_value`，`check_wire_item` 计数一遍，绑定层 `frame_size` 再计数一遍，最后序列化；`seq` 在 `u64` 与字符串间往返 | 约 1 µs / 帧（§1.5） |
| B-3 | `aimux-ops/src/dispatch.rs`（`decode`、`validate_secondary`），各绑定的 `parse` | 入向 `from_str → Value → from_value`；`validate_secondary` 每次 `split('.').collect()`；`aimux-ffi` 的 `context` 用 `Value::String(abort.to_string())` 包一层再解析 | 未测（§1.4） |
| B-4 | `aimux-providers/src/openai/chat/chat_language_model.rs` | `body.clone()`；`create_json_response_handler` 对响应解析两遍，第二遍的 `Value` 同时用于 `Usage.raw` 与 `response.body` | 克隆不到 1%；两遍解析 2.7%–5.3%（§1.4） |
| B-5 | `aimux-core/src/generate.rs`（流泵），`bindings/node/src/bridge.rs`、`bindings/python/src/bridge.rs` | core 无条件 `tokio::spawn` 流泵；绑定层再起一个任务；消费者每帧一次往返。Python 的 `__next__` 每帧 `block_on` 一个带 20 ms 超时的 `recv` | Node 每帧约 39 µs（§1.5） |
| B-6 | `aimux-ffi/src/abi.rs`（`block_on`） | `aimux_call` / `aimux_stream` 阻塞调用线程到整个交互结束 | 未测 |
| B-7 | `aimux-provider-utils/src/fetch.rs`，`aimux-ops/src/handle.rs`、`dispatch.rs` | `default_transport()` 每次取写锁只为置 `frozen`；`ReqwestFetch::client()` 每次锁 `Mutex`；`HandleRegistry` 是一把全局 `Mutex<HashMap>`；按名查操作是线性扫描；`pool_max_idle_per_host(10)` | 锁不是瓶颈（§1.7）；连接池上限的代价未测 |
| B-8 | 根 `Cargo.toml`，`bindings/node`、`bindings/python` 的 `Cargo.toml` | 根 profile 为 `"z"`（`aimux-ffi` 用它）；Node / Python 绑定为 `"s"` | `"z"` 在绑定上使 64 KB 响应慢 28–35%（RFC-0043 §15.8）；`aimux-ffi` 上未测 |

各语言绑定另有已核实的局部问题，归入 §3.3 与 §3.6：

- Node：`JSON.stringify` 带 replacer。实测比不带慢 1.02–1.6 倍，200 KB 多消息请求约慢 0.15 ms。
- Go：`C.CString(string(params))` 与 `[]byte(C.GoString(s))` 各多拷一次；结果先 `validateJSON` 再解码；`streamBufferSize = 7`。
- Flutter：`StreamBridge(credits: 1)` 配合 C 侧 `pending = 1` 的停等；`execute()` 每次调用 `Isolate.run`。
- Swift：所有调用派发到 `DispatchQueue.global(qos: .utility)`。

## 3. 设计

### 3.1 返回包：`include` 与事实集

**`include`。** 在 `GenerateTextOptions` 增加 `include: Option<IncludeOptions>`，字段 `request_body`、`response_body`。上游 AI SDK 的 `generateText` 有同名选项 `include?: { requestBody?, requestMessages?, responseBody? }`，三者默认 `false`，文档说明是“默认不包含以减少内存占用”（ai-sdk.dev 的 `generateText` 参考，2026-11-12 查阅）。`requestMessages` 在 aimux 的 `StepResult` 里没有对应字段，本文不引入。

**剥离的位置。** 不能在 provider 里把 `request.body` 置空：`aimux-core/src/trace/layer.rs` 包在模型外层，读 `result.request.body` 算缓存指纹。body 在 `generate_in_operation` / `stream_in_operation` 组装 `StepResult` 时按 `include` 丢弃。

**随之可省的工作。** `response_body == false` 且没有其他消费者需要完整响应体时，响应不必解析第二遍。`Usage.raw` 只需要 `usage` 子树，可用 `Box<RawValue>` 或对该子树单独解析。这一步逐 provider 进行；Responses 路径已在 #502 完成。Chat 路径依赖本开关，因为 `response.body` 现行就是第二遍解析的产物。

**事实集。** `generate.rs` 已有只含 `steps`、`total_usage`、`output` 的 `GenerateTextResultFacts`，它是反序列化用的私有结构。序列化改为只写这三项：

- `output` 在默认纯文本模式下等于文本，不再写出。
- 派生字段（`text`、`content`、`toolCalls`、`finishReason`、`usage`、`request`、`response`、`responseMessages` 等）由 `tools/aimux-codegen` 生成 getter。codegen 现有 7 个目标（Python、Go、Java、Kotlin、Swift、Flutter、Node，见 `tools/aimux-codegen/generated-files.json`），绑定共 8 个：
  - **C**：没有生成类型，调用方直接读 JSON。D2 之后 C 调用方拿不到顶层派生字段，只能从 `steps` 自行取；升级说明要给出每个派生字段与 `steps` 的对应关系。
  - **Node**：只生成类型声明（见下一条），getter 需要另外生成一段运行时代码挂到结果对象上。
- Node 没有编解码器（`scripts/check_dto_vectors.py` 的 `DECLARATION_ONLY = {"node"}`），结果是 `JSON.parse` 出的普通对象。派生字段用不可枚举的 getter 挂上，避免用户 `JSON.stringify` 结果时重新展开。副作用：对象展开与 `structuredClone` 拿不到这些字段，需写进升级说明。
- `steps[i].content` 与 `steps[i].response.messages` 的重复保留。去掉它要把响应消息的构造逻辑搬进 7 个 codegen 目标和 C 调用方，不在本文范围。

### 3.2 `aimux-ops`：出向与入向

**出向（先做）。** `CallFuture` 的输出由 `Value` 改为已序列化的字节；各操作的 `json()` 直接 `to_writer`。绑定层依赖结果 `Value` 的地方需要改，例如 Node 的 `remember()` 用它判断结果是否是句柄。

**入向（后做）。** handler 接收字节并一步反序列化为请求结构。`validate_secondary` 现在靠遍历 `Value` 找次级句柄，改为从请求结构取，或用只读取句柄字段、其余 `IgnoredAny` 的探针结构。`aimux-ffi` 已经以 `u64` 传入 `abort`，直接查表，不再包成 `Value::String`。

`TextRequest` 里 `options` 用了 `#[serde(flatten)]`。flatten 的缓冲只作用于 `options` 对象，prompt 不在其中，不作为本文的改动理由。

### 3.3 流式：批量过桥与单遍成帧

**规则。** 消费者请求下一帧时，若队列为空则等第一帧；拿到后用 `try_recv` 取走此刻所有已就绪的帧，到上限为止，立即返回。不等待凑批。

- **Node**：`FrameIterator::next()` 返回 `Vec<String>`；`native.ts` / `index.ts` 的迭代器从本地数组逐个吐出，对外仍是逐帧的 `AsyncIterableIterator`，`./raw` 导出的逐帧字符串不变。
- **Python**：`__next__` 先 `try_recv`，队列空才进运行时等待；去掉每帧一次的 `block_on` + 定时器。
- **Go / Swift / Java / Kotlin / Flutter**：C ABI 的回调是逐帧的。先做不改 ABI 的部分：Go 去掉重复扫描（注意 `Decoder.Decode` 不拒绝尾部多余内容，尾部检查要保留）、缓冲按字节记账；Flutter 把 `credits: 1` 与 C 侧停等一起改成窗口。批量回调属于 §3.5。
- **终结帧**：`End` 帧仍走独立于数据队列的槽位，批量不得丢弃或改写它。

**单遍成帧。** `StreamFrame` 的事件由 `Value` 改为可直接序列化的类型；`seq` 用 `u64`，序列化时写成十进制字符串；用一个在写入过程中累计字节并在超过 1 MiB 时立即报错的 writer，一次 `to_writer` 同时完成编码与限额检查，取代 `check_wire_item` 与 `frame_size`。

**流泵不动。** `generate.rs` 的注释写明流泵是有意无条件启动的：它按帧“到达时间”计超时与性能统计，也是流的终止保险。内联会把口径变成“消费者轮询时间”。本文不改它。

### 3.4 Provider 层

- 单遍解析按 §3.1 推进，顺序为 OpenAI Chat、Anthropic、Google、Bedrock。
- `body.clone()` 不单独处理：收益不到 1%，而去掉它要改 `post_json_to_api` 的返回方式，涉及约 86 个调用点。
- SSE：`aimux-stream` 已启用 `sse-stream` 的 `memchr`；core 整条流式管线每帧 1.76 µs。零拷贝解析器与借用型 chunk 结构排在批量之后，做之前先用内存 SSE 基准量出它在 1.76 µs 里的份额。
- `create_event_source_response_handler` 里响应头的三份拷贝每条流只发生一次，不处理。

### 3.5 C ABI 异步原语

`aimux-ffi` 现行 14 个导出全部同步。新增非阻塞的 `call` / `stream` 提交与批量帧回调，作为 ABI 的增量符号（旧符号保留），让 Go / JVM / Swift / Dart 不必为每个在途调用占一个 OS 线程。

本文没有 C ABI 侧的并发基准。这一项的设计细节与验收在有了 Go 侧基准之后另行补充，本文只登记方向。

### 3.6 传输、编译与小修

- **连接池**：`pool_max_idle_per_host(10)` 在只有 HTTP/1.1 的现行配置下意味着超过 10 条的空闲连接被关闭。调高或做成可配置；用“并发 64 → 停顿 → 并发 64”的突发场景量化（稳态并发测不出，loopback 明文还会低估重建 TLS 的代价）。
- **`aimux-ffi` 的 `opt-level`**：RFC-0043 §15.3 只量了 `"z"` 改 `"s"` 的体积（+3.8%），没量性能。补测后决定。
- **mimalloc**：按 §1.8 在 Node / Python 绑定与 `aimux-ffi` 里验证效果与体积，再决定。
- **小修**（每项独立、可单独合入）：
  - `default_transport()` 改成读路径加原子标志。
  - `jsonschema` 校验器现行每次工具调用与结构化输出都重新编译；工具定义在绑定层每次调用都重新反序列化，所以用按 schema 哈希的全局有界缓存，而不是挂在 `FunctionTool` 上。
  - 按名查操作改二分；`validate_secondary` 的路径预切分。
  - Node 的 `client.manifest` 与 Python 导入时的 manifest 解析改为惰性。
  - Swift 改为继承调用方 QoS。
  - Flutter 的常驻 worker isolate 先测再定。
- **PGO**：未测，不排期。

### 3.7 远期方向（未测，不承诺）

- **同协议直通**：`text.streamOpenAI` 在上游本身是 OpenAI Chat 协议、且没有中间件、工具修复、结构化输出、trace、录制、严格流校验时，跳过统一模型的拆装。与 ROADMAP 的 L0 passthrough 一并评审。
- **共享字节视图**：以 `Buffer` / `memoryview` 进出，省掉 UTF-16 与 UTF-8 之间的转码。它不省宿主侧的 `JSON.parse` / `JSON.stringify`，而 §1.3 表明那才是 Node 的瓶颈。Node 的外部内存挂载在部分运行时可能不可用，未验证。
- **二进制协议**：只在多模态大附件上考虑，目的是绕开 base64。

## 4. 不变量

- 写入超过 1,048,576 字节（即写到第 1,048,577 字节）时立即中止并返回 `ResourceLimit`，不允许先分配再检查；恰好 1,048,576 字节的帧合法，与现行 `check_wire_item`（`aimux-ops/src/limits.rs`，`total <= maximum`）一致。
- 错误上下文里的请求值，无论从 `Value` 还是从字节生成，都经过同一套脱敏与截断，`contract-tests/dto/AiMuxError/*` 字节级不变。
- `End` 帧独立于数据队列；取消与上游错误不因批量而丢失或乱序。
- trace 指纹与录制看到的请求体与现行一致（§3.1 的剥离位置）。
- 输出与请求体的键序不变（§0.3）。
- 超时与 `StepResultPerformance` 的计时口径不变（§3.3）。

## 5. 门禁

沿用 RFC-0043 §7 的成对运行规则，新增三组基准并入库：Node 并发扫描（含突发场景）、流式帧率、内存 `Fetch` 的层差分。

- **硬门禁（确定性）**：固定负载下的返回包字节数；内存 `Fetch` 下每请求的堆分配次数与字节数（计数分配器）；流式稳态每帧分配次数。
- **成对比较（有噪声）**：每请求 CPU、吞吐、帧率。只判定基线与候选在同一台机器上交替运行的差。
- 分配次数现行没有数，阶段 0 登记。

## 6. 阶段与验收

| 阶段 | 版本边界 | 内容 | 验收 |
|---|---|---|---|
| 0 | 0.6.x | 三组基准入库；登记返回包字节数、分配次数、§1.2 的四版本对比 | 基线可由 CI 复现 |
| 1 | 0.6.x，不破坏类型与默认行为 | #502；Node / Python 流式批量与单遍成帧；ops 出向直写；`include` 选项（此阶段默认仍回传）；§3.6 小修 | Node 突发流帧率不低于 150,000 帧 / 秒（原型 175,000，现行约 26,000）；单流首帧与逐帧延迟不回退；`include` 全关时大包返回 JSON 不超过 360,000 B |
| 2 | 0.7.0，破坏性窗口 | `include` 默认改为 `false`；序列化只写事实集，7 个 codegen 目标生成 getter，C 调用方按升级说明从 `steps` 取；ops 入向；Go / Flutter 流式改造；mimalloc、HTTP/2、`aimux-ffi` profile 三项按实测定案 | 大包返回 JSON 不超过 110,000 B（原型 101,324）；目标（不作门禁）：大包吞吐不低于 `c4748178`，按 RFC-0043 §7.4 的配对 bootstrap 登记结果；小请求耗时登记并说明与 `c4748178` 的差距构成 |
| 3 | 0.7.x 之后 | C ABI 异步原语与批量回调；§3.7 中经实测立项的部分 | 另行补充 |

说明：

- “不低于 `c4748178`”是推算出的目标，不是门禁：原型把单并发吞吐提到 1.76 倍，与现行落后 7 月版的 1.7 倍相当，且原型的返回包（101 KB）小于 7 月版（297 KB）。没有与 7 月版直接对测过原型。
- 小请求能否回到 7 月水平没有证据。差的约 0.25 ms 里，已知的是 core 约 0.1 ms、结果序列化约 0.04 ms。
- 体积：各阶段的产物都在 RFC-0043 §4.1 的上限内（cdylib 24,200,000、staticlib 70,000,000、Android 单片 9,000,000）。

## 7. 待决问题

1. **`include` 默认值的切换时点。** 本文建议 0.6.x 加选项、0.7.0 翻转默认。也可以在 0.6.x 直接默认 `false`（与上游一致，但改变现有调用方拿到的字段）。
2. **事实集属于哪个契约版本。** 类型化客户端经 getter 不受影响；直接读 JSON 字段的调用方会受影响。0.7.0 是否可以承载，还是要等 ops 2.0。
3. **manifest 摘要的口径。** 新增 `include` 会改变摘要。0.6.1 的更新日志已有“ABI 1 / ops 1.0 不变但摘要变化”的先例，本文按此处理，需维护者确认。
4. **是否启用 HTTP/2。** 影响高并发扇出的连接数，也影响体积；两者都未测。
5. **`aimux-ffi` 由 `"z"` 改 `"s"`。** 体积 +3.8% 已知，性能未测。
6. **README 的性能数字。** 它们测于 7 月的旧接口（§1.2），是否先加说明。

## 8. 相对外部草案的改动

| 草案的条目 | 处理 | 原因 |
|---|---|---|
| 阶段 1 返回包“小于 120 KB” | 改为 360,000 B，110,000 B 移到阶段 2 | 只关 body 仍有 7 份文本；101 KB 是事实集的结果 |
| 返回包终极“小于 55 KB” | 删除 | 需要在 8 门语言里重建响应消息 |
| “纯核 10 µs、端到端桥接小于 25 µs” | 删除 | §1.6 |
| 流式 30 万 / 100 万 / 200 万帧每秒 | 改为 Node 150,000，其余删除 | §1.5；草案的阶段 1 不含批量却承诺 30 万 |
| 阶段 1“CPU 降 60%”“32 核线性度大于 0.8”“8 线程一万路流” | 删除 | 无基线 |
| 按包 `opt-level = 3`，“体积增量小于 300–400 KB” | 不做 | RFC-0043 §15.8 |
| 剥离运行时 `preserve_order` | 不做 | §0.3 |
| `.tcp_nodelay(true)`、`.http2_adaptive_window(true)` | 不做 / 转为待决 | §0.3 |
| 锁导致“严重自旋”，32 分片句柄表 | 降为小修 | §1.7 |
| 内联流泵、去掉逐帧 `Instant::now()` | 不做 | §3.3；`Instant::now()` 走 vDSO，不是系统调用 |
| “未开录制仍调用 `new_call_id()`” | 不做 | 属实但必需：`generate.rs:1605` 只在有 recorder 时调用，`:1624-1626` 的 `step_call_id` 在未开录制时仍调用，因为 `StreamTextResult::consume()` 把它写进公开字段 `StepResult.call_id`（`aimux-core/src/step_result.rs:223`），总要有值；`StreamTextResult.call_id`（`:2050`）本身是私有字段 |
| “`StepResult` 里已标记 deprecated 的冗余字段” | 删除 | 不存在；deprecated 的是 `GenerateTextResult` 的顶层访问器 |
| 密码学随机数生成后备 ID | 不处理 | 属实，但每次约百纳秒量级，未单独测 |
| Chat 路径 `usage` 改 `RawValue` 即可单遍解析 | 改为依赖 `include` | `response.body` 由第二遍解析填充 |
| 默认关 body 只需避开 recorder | 补充 trace 层 | §3.1 |
| mimalloc 放在阶段 2 | 提前到验证项 | §1.8 |
| 流式批量放在阶段 2 | 提前到阶段 1 | 改动最小，不改对外接口 |
