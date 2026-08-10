# P6：v0.2.1 → HEAD 第三轮 Review — 并发/异步正确性 + 热路径性能

> **快照声明**: 2026-08-07 时间点快照，基于 v0.2.1..HEAD 的代码静态审查。
> 代码演进后部分发现可能过期；本报告作为诊断参考归档。
>
> **范围（第三轮新地面）**：v0.2.1 之后新增/修改代码的 **Send/Sync 与异步正确性、
> 死锁/竞态、热路径性能、阻塞调用进异步上下文、流式退避/取消**。
> - `aimux-core/src/{recording.rs, replay.rs, session.rs, trace/*, openai_output.rs, generate.rs, provider.rs}`
> - `aimux-provider-utils/src/{http.rs, logging.rs}`
> - `aimux-ffi/src/lib.rs`（新增部分）
> - `aimux-stream/src/sse.rs`（**经核实 v0.2.1..HEAD 零改动**，本轮不展开；P2/P3 边界问题见 p4）
>
> **方法**：`git diff v0.2.1..HEAD` 概览 + 精读关键文件，未运行 `cargo`（p4 已确认 check 通过）。
> 先读 [p4](p4-release-v021-head-review.md) 与 [p5](p5-native-providers-round2-review.md) 避免重复。
>
> **日期**: 2026-08-07

---

## 0. 总体判断

| 维度 | 判断 | 关键依据 |
|---|---|---|
| Send/Sync 与 async 正确性 | **良好** | 跨 await 边界持有的类型（`RecordCtx`/`RecordingOutcomeStream`/`Arc<dyn Recorder>`）均 Send+Sync；无 `Mutex`/`RwLock` 持锁跨 await；FFI 用 thread-local 重入守卫 + `OnceLock` runtime 正确；`RecordingOutcomeStream`/`ObservedByteStream` 均有 Drop 兜底 |
| 死锁/活锁/竞态 | **无死锁面** | 各锁获取-释放不嵌套、顺序一致；`Once`/`OnceLock` 用法正确；唯一隐患是单 `Mutex` 的**持锁面**（性能，见 §2）而非死锁 |
| **热路径性能** | **有可优化点（均 opt-in）** | 默认全关时热路径≈1 读锁+1 Arc clone，确实零成本；开启 trace/recording/session 后，每次 generate/stream 在 async 线程上做 O(body) CPU + 多次全局互斥锁串行化，且未 `spawn_blocking` |
| 阻塞调用进异步上下文 | **干净** | core/http 生产路径无 `std::fs`/`sleep`/`block_on`（replay 的 `block_on` 全在 `#[cfg(test)]`）；唯一阻塞是 sync FFI 桥（`ffi_block_on`）与录制 flush（`recv_timeout`，按设计 sync） |
| 流式退避/取消 | **abort 路径及时、无泄漏 task** | `aimux_stream_text_with_abort` 在连接阶段与逐 chunk 均用 `tokio::select!` 抢占 `signal.cancelled()`；全程 inline 无 `spawn`，abort 后 stream drop→HTTP 层与录制层 Drop 兜底。**但 trace 层在该路径丢记录**（见 F1） |

**最关键的新发现**：trace 层的流式包装器**在消费方提前 drop 流时丢失 TraceRecord**（F1，major），与录制层（有 Drop 兜底）行为不一致——这正是 abort/cancel/take(N) 这类高频场景。

---

## 1. 新 findings

> 🆕 = 前两轮（p4/p5）**未报告**的新发现；🔁 = 对已有结论的**验证/扩展**。
> 每条标注「正确性」或「性能」。

### 1.1 Blocker
无。

### 1.2 Major

#### F1. 🆕【正确性】TraceLayer 流式包装器在提前 drop 流时丢失 TraceRecord
- **模块**: aimux-core/src/trace/layer.rs
- **问题**: `TraceLayer::do_stream` 用 `async_stream::stream!` 内联生成器包装流，`rec_ctx.record(...)`
  是该流 TraceRecord 落库的**唯一**调用点，且位于 `while let Some(item) = s.next().await` 循环**之后**
  （[layer.rs:475-489](../../aimux-core/src/trace/layer.rs#L475)）。
  `yield item` 挂起生成器；当消费方在流自然结束前 drop 该流（abort、`take(N)`、错误提前返回、外层超时），
  生成器在 `yield` 挂起点被直接丢弃，**循环之后的 `rec_ctx.record` 永不执行** → 该次调用的
  TraceRecord 从未入 sink。生成器状态机只跑其中所持有值的 `Drop`，而 `rec_ctx` 是 `Arc<RecordCtx>`、
  `RecordCtx` 无 `Drop` impl，故无任何兜底调用 `record`。
- **后果**: trace 子系统在「流被提前放弃」这一高频场景下静默丢记录——聚合统计缺数、
  会话链视图断链、LCP 索引缺该次指纹（后续同前缀调用查不到匹配，缓存判定退化）。
  这恰好是 abort/cancel 路径（用户最想看「发生了什么」时）丢的。
- **对照（设计不一致）**: 同仓库录制层 `RecordingOutcomeStream` 有 `Drop` impl，在未终结 drop 时
  记录 `OutcomeStatus::Cancelled`（[recording.rs:976-988](../../aimux-core/src/recording.rs#L976)）；
  HTTP 层 `ObservedByteStream` 也有 `Drop` 兜底补全 exchange（[http.rs:540](../../aimux-provider-utils/src/http.rs#L540)）。
  唯独 trace 层用了内联 `async_stream!` 而无 Drop 守卫。
- **触发链佐证**: `aimux_stream_text_with_abort` abort 时 `return` 退出 async 块
  （[lib.rs:1465-1468](../../aimux-ffi/src/lib.rs#L1465)），外层 `RecordingOutcomeStream` 的 Drop 触发
  （记录 Cancelled），但内层 trace 生成器被 drop 于挂起点 → trace 记录丢失。即 abort 路径「录制捕获了 Cancelled，trace 丢了记录」。
- **修复方向**: 把 trace 流包装从内联 `async_stream!` 改为一个有 `Drop` impl 的 struct（仿 `RecordingOutcomeStream`），
  在 Drop 里以 `Outcome::Incomplete`/error 兜底 `record`；或在生成器尾部之外用 Drop guard 包裹 `rec_ctx`。
- **位置**: [layer.rs:475-489](../../aimux-core/src/trace/layer.rs#L475)（record 在 :488）
- **与前两轮关系**: 🆕 全新。p4 B4 是「未实现两阶段占位记录、在途调用互不可见」（并发可视性简化），**本条是「已终结但被 drop 的调用记录彻底丢失」**，是不同的缺口。

### 1.3 Minor

#### F2. 🆕【正确性/可观测性】`aimux_recording_flush` 恒返回 0，30s 超时后调用方无法察觉落盘失败
- **模块**: aimux-ffi/src/lib.rs ↔ aimux-core/src/recording.rs
- **问题**: `aimux_recording_flush` 调 `rec.flush()` 后**无条件返回 0**
  （[lib.rs:2334-2339](../../aimux-ffi/src/lib.rs#L2334)）。而 `JsonlRecorder::flush` 用
  `ack_rx.recv_timeout(Duration::from_secs(30))` 并 `let _ =` 丢弃结果
  （[recording.rs:513-522](../../aimux-core/src/recording.rs#L513)）——超时（writer 卡死/磁盘满/已退出）
  时数据**并未落盘**，但 FFI 仍回 0（成功）。调用方无法区分「已落盘」与「30s 后放弃」。
- **理由**: p4 A9 报的是「JsonlRecorder 初始化/写入失败被静默吞掉」，本条是 flush 路径特有的
  「超时即成功」语义——同一类「失败静默」但在 flush 这一对用户最关键的同步点上。
- **修复方向**: flush 返回 `Result`/区分码；FFI 把 `recv_timeout` 的 `Err` 映射为非 0 退出 + error 信封。
- **位置**: [recording.rs:513-522](../../aimux-core/src/recording.rs#L513)、[lib.rs:2334-2339](../../aimux-ffi/src/lib.rs#L2334)

#### F3. 🔁 扩展【性能】RingTraceStore `lookup` 是 `&mut self` → LCP 查询与插入抢同一把排他锁
- **模块**: aimux-core/src/trace/store.rs
- **问题**: `TraceStore::lookup` 签名是 `&mut self`（它递增 `tombstone_hits`、并在墓碑率>25% 时
  `maybe_rebuild_index` 重建索引，[store.rs:169](../../aimux-core/src/trace/store.rs#L169)/[:223](../../aimux-core/src/trace/store.rs#L223)）。
  `RingTraceStore::lookup` 因此必须 `inner.lock()`（**排他**，[store.rs:516-522](../../aimux-core/src/trace/store.rs#L516)），
  与 `record`/`append`（写）抢同一把锁。结果：**所有并发调用（含异 scope）的「LCP 查询」与「记录插入」
  全部串行化在一把 Mutex 上**，且查查询强制排他（无法并发只读）。
- **理由**: p4 B3 只说「单 Mutex 替代 16 片分片锁，异 scope 不并行」；本条补强根因——
  `lookup` 的可变性（墓碑计数+索引重建）使其不可能降级为读锁，即使分片也仍是排他。
  这是 trace 热路径串行化的真正机制点。
- **修复方向**: 墓碑计数用 `AtomicU64`、`maybe_rebuild_index` 移出 lookup 路径（惰性触发于 insert），
  使 lookup 可降级为读锁；或按 scope 分片。
- **位置**: [store.rs:169](../../aimux-core/src/trace/store.rs#L169)、[:223-254](../../aimux-core/src/trace/store.rs#L223)、[:516-522](../../aimux-core/src/trace/store.rs#L516)

#### F4. 🆕【性能】查询 API（aggregate/session_chain/export_jsonl）持全局 Mutex 期间做全量遍历/IO，可阻塞热路径
- **模块**: aimux-core/src/trace/store.rs
- **问题**: `export_jsonl` 在 `inner.lock()` 期间遍历**全部** record 逐条 `serde_json::to_string` + `writeln!`
  到调用方传入的 `impl Write`（[store.rs:469-476](../../aimux-core/src/trace/store.rs#L469)）——若该 writer 是慢 IO
  （文件/套接字），整段持锁，所有并发 `record`/`lookup`（async 线程）被阻塞。
  `aggregate` 持锁全量遍历 + 每组再全量 filter（O(groups×records)，p4 B9 已报），
  `session_chain` 持锁期间对每个 call_id 做 `records.iter().find`（O(records×session_len)，见 F5）。
- **理由**: 这些是查询/导出 API，但与热路径共享同一把锁；高并发录制期间一次导出即可拖停 trace 写入。
  p4 B3/B9 提了单锁与 aggregate 复杂度，但未点出「查询持锁做 IO 直接阻塞热路径」这一跨关注点耦合。
- **修复方向**: 查询先在锁内 clone 出 `Vec<Arc<TraceRecord>>` 快照，释放锁后再序列化/写 IO；
  或读写锁分离（配合 F3）。
- **位置**: [store.rs:369](../../aimux-core/src/trace/store.rs#L369)（aggregate）、[:424-466](../../aimux-core/src/trace/store.rs#L424)（session_chain）、[:469-476](../../aimux-core/src/trace/store.rs#L469)（export_jsonl）

#### F5. 🆕【性能】`session_chain` 对每个 call_id 做 `records.iter().find`，O(records × session_len)
- **模块**: aimux-core/src/trace/store.rs
- **问题**: `session_chain` 先从 `by_session` 取 call_id 列表，再对**每个** id 在
  `inner.records`（VecDeque）上线性 `find`（[store.rs:430-434](../../aimux-core/src/trace/store.rs#L430)）。
  全部在持锁期间。n=总记录数、k=该会话记录数时复杂度 O(n×k)。
- **修复方向**: 用 `HashMap<call_id, usize>`（records 的索引）避免线性查找；或查询前快照后释放锁。
- **位置**: [store.rs:430-434](../../aimux-core/src/trace/store.rs#L430)
- **与前两轮关系**: 🆕（p4 B9 是 aggregate 的 O(groups×records)，本条是 session_chain 的 O(n×k)，不同函数）

#### F6. 🆕【性能】TraceLayer 在 async 线程上做 O(body) CPU 且未 spawn_blocking，每次调用 2 次全局排他锁
- **模块**: aimux-core/src/trace/layer.rs
- **问题**: `record()` 在 `do_generate().await` 返回后（[layer.rs:422](../../aimux-core/src/trace/layer.rs#L422)）
  / 流结束后（[:488](../../aimux-core/src/trace/layer.rs#L488)）**同步**于 async runtime 线程执行：
  ① `denoise(body)` 对整棵请求 JSON 做**深拷贝**（[fingerprint.rs:92-118](../../aimux-core/src/trace/fingerprint.rs#L92)）；
  ② `serde_json::to_vec(&denoised)` 再序列化整段（[layer.rs:194](../../aimux-core/src/trace/layer.rs#L194)）；
  ③ `fp.compute` 对全 body 分块哈希（O(body)，hash 本身是非加密 splitmix64，[hash.rs:23-45](../../aimux-core/src/trace/hash.rs#L23)）；
  ④ `lookup_lcp`（一次排他锁，见 F3）+ `session_stats_for`（session_tracker Mutex）+ `sink.record`（又一次排他锁）。
  另 `do_generate`/`do_stream` 里 `result.request_body.clone()`（[:420](../../aimux-core/src/trace/layer.rs#L420)/[:445](../../aimux-core/src/trace/layer.rs#L445)）又克隆一次大请求体。
  全程未 `spawn_blocking`，绑死 async worker。
- **理由**: 默认（TraceLayer 未启用）热路径零成本；但启用后，长对话历史（100KB+）的 denoise 深拷贝+序列化+哈希
  会给单线程 runtime 整体加尾延迟，多线程 runtime 占满 worker。两把排他锁进一步串行化并发。
  p4 提了「trace 额外开销」泛指但未落到「denoise 深拷贝+未 offload+双锁」这一具体机制。
- **修复方向**: 记录与指纹计算 `spawn_blocking`；或避免 denoise 深拷贝（流式哈希/原地脱敏）；
  request_body 用 `Arc<Value>` 共享而非 clone。
- **位置**: [layer.rs:192-195](../../aimux-core/src/trace/layer.rs#L192)、[:257-397](../../aimux-core/src/trace/layer.rs#L257)、[:420](../../aimux-core/src/trace/layer.rs#L420)
- **与前两轮关系**: 🆕（p4 B6 是 system_tokens 恒 0、B5 是自定义 sink LCP 退化，均非此性能机制）

#### F7. 🆕【性能】SessionStore LRU `touch` 为 O(sessions)，每次 generate 都在全局锁内执行
- **模块**: aimux-core/src/session.rs
- **问题**: `SessionStore::append`（generate_text/stream_text 入口每调用一次，[generate.rs:250](../../aimux-core/src/generate.rs#L250)/[:399](../../aimux-core/src/generate.rs#L399)）
  取全局 `Mutex<Inner>` 后调 `touch`：`self.lru.iter().position(...)` 线性扫描 + `VecDeque::remove(pos)` 线性移动
  （[session.rs:216-221](../../aimux-core/src/session.rs#L216)）。默认上限 256，单次开销小，但**所有并发调用在
  generate 入口串行化于此锁**。`session_calls`（已知会话）也 `touch`。
- **修复方向**: 用 `LinkedHashMap`/`IndexMap` + 双向索引使 touch O(1)；或 touch 移出热路径。
- **位置**: [session.rs:216-221](../../aimux-core/src/session.rs#L216)、[:127-169](../../aimux-core/src/session.rs#L127)
- **与前两轮关系**: 🆕（p4 未涉及 session LRU 的 touch 复杂度；A12 是 step 单调性、A13 是 call_id 双来源）

#### F8. 🆕【性能】`to_http_record` 先物化整段请求体再截断到 1 MiB，每条 exchange 在 async 线程上
- **模块**: aimux-provider-utils/src/http.rs
- **问题**: `to_http_record` 对 `HttpBody::Json(v)` 做 `serde_json::to_string(v)`（**整段**序列化，无上限，
  [http.rs:975](../../aimux-provider-utils/src/http.rs#L975)），对 `Bytes(b,_)` 做 `String::from_utf8_lossy(b).into_owned()`
  （**整段**拷贝，[:976](../../aimux-provider-utils/src/http.rs#L976)），**之后**才 `truncate_utf8(&body, 1MiB).to_string()`
  （再分配 1 MiB，[:982](../../aimux-provider-utils/src/http.rs#L982)）。对大请求体（如多模态/长历史），
  等于先分配 N 字节、再分配 1 MiB、再丢 N。每次录制 exchange（含每次重试失败 attempt）都如此，于 async 线程。
- **理由**: 仅录制开启时触发（opt-in），但与 F6 叠加时放大 async 线程分配压力。
- **修复方向**: Json 分支直接 `to_writer` 进有上限的缓冲；Bytes 分支先 `take(cap)` 再转 lossy。
- **位置**: [http.rs:961-990](../../aimux-provider-utils/src/http.rs#L961)
- **与前两轮关系**: 🆕（B1 是 URL query 凭据、A6 是 chunk 级 lossy decode、N5 是 body 不做 JSON 脱敏，均非此「整段物化再截断」浪费）

### 1.4 Nit

#### F9. 🆕【性能】`RingTraceStore::append` 在全局锁内对每个 block 做 hex→u128 解析
- **模块**: aimux-core/src/trace/store.rs
- **问题**: append 持锁期间把 `fingerprint.block_hashes`（已是 hex 字符串）逐个
  `u128::from_str_radix(h, 16).unwrap_or(0)` 解析回 u128（[store.rs:487-493](../../aimux-core/src/trace/store.rs#L487)）。
  解析失败静默归 0（会让多块 hash 落到同一索引键 0，潜在伪碰撞——但指纹由内部生成，正常不触发）。
  耗时不大但属于「持锁做可前置的 CPU 工作」。
- **修复方向**: `TraceRecord` 直接带 `Vec<u128>`（或 `Chain` 不转 hex），省去往返转换；解析在入锁前做。
- **位置**: [store.rs:487-493](../../aimux-core/src/trace/store.rs#L487)

#### F10. 🆕【性能/一致性】`new_call_id()` 在 recording.rs 与 session.rs 各有一份、各自独立 `AtomicU64` 计数器
- **模块**: aimux-core/src/{recording.rs, session.rs}
- **问题**: 两个模块各定义 `static CALL_SEQ: AtomicU64` + `fn new_call_id()`（[recording.rs:325-334](../../aimux-core/src/recording.rs#L325)、
  [session.rs:397-406](../../aimux-core/src/session.rs#L397)），逻辑相同但计数器分离。session 与 recording 各自生成的
  call_id 序列号可能相同（仅靠纳秒时间戳前缀区分），增加跨系统对账时的混淆。
- **理由**: 与 p4 A13（call_id 双来源）同源但角度不同——A13 是「HttpRequest.call_id vs recording_context.call_id」，
  本条是「两个 new_call_id 工厂各自独立」。
- **修复方向**: 收敛为单一 `new_call_id()` 工厂。
- **位置**: [recording.rs:325-334](../../aimux-core/src/recording.rs#L325)、[session.rs:397-406](../../aimux-core/src/session.rs#L397)

---

## 2. 已有结论验证（确认仍成立，未变）

| 已有 finding | 本轮验证 | 证据 |
|---|---|---|
| p4 D1 — FFI `list_models`/`get_model_specs` 绕过 `ffi_block_on`/共享 runtime/重入守卫 | 🔁 **仍成立**，代码未变 | [lib.rs:948-980](../../aimux-ffi/src/lib.rs#L948)、[:1015-1033](../../aimux-ffi/src/lib.rs#L1015) 仍走 `Handle::try_current()` + 自建/瞬态 runtime `block_on`；不走 `ffi_block_on` 故不置 thread-local 重入守卫；`Err` 分支创建瞬态 `Runtime::new()`→`.handle().clone()` 后该 Runtime 即被 drop，再对已 drop runtime 的 handle 调 `block_on`（行为仍存疑）。宿主已跑 tokio 时 `Handle::try_current()` 取到宿主 handle → 嵌套 `block_on` → `panic=abort` 终止进程 |
| p4 A4 — JsonlRecorder 无界 channel | 🔁 仍成立 | [recording.rs:417](../../aimux-core/src/recording.rs#L417) `std::sync::mpsc::channel`（无上限/丢弃策略） |
| p4 N4 — `JsonlRecorder::new` `.expect()` 在 FFI 可达路径上 abort 宿主 | 🔁 仍成立 | [recording.rs:420-423](../../aimux-core/src/recording.rs#L420) spawn `.expect()`；`aimux_init_recording` 直接 `JsonlRecorder::new(dir)`（[lib.rs:2300-2307](../../aimux-ffi/src/lib.rs#L2300)） |
| p4 A6 — 流式录制按网络 chunk 逐个 lossy decode | 🔁 仍成立 | [http.rs:506](../../aimux-provider-utils/src/http.rs#L506) `String::from_utf8_lossy(bytes)` 逐 chunk，多字节字符跨边界成 U+FFFD |
| p4 B3 — RingTraceStore 单 Mutex | 🔁 仍成立并扩展 | [store.rs:323](../../aimux-core/src/trace/store.rs#L323) 单 `Mutex<Inner>`；见 F3/F4/F9 的持锁面细化 |
| p4 N3 — recording/logging 脱敏规则分叉 | 🔁 仍成立 | logging.rs v0.2.1..HEAD 零改动，分叉仍在 |

---

## 3. 正面发现（流式退避/取消，维度 e）

- **abort 路径及时、无泄漏 task**：`aimux_stream_text_with_signal` 在 `ffi_block_on` 内全程 inline 驱动
  （[lib.rs:1446-1497](../../aimux-ffi/src/lib.rs#L1446)），无 `tokio::spawn`。连接阶段
  `tokio::select!{ signal.cancelled(), stream_text(...) }`（[:1449-1454](../../aimux-ffi/src/lib.rs#L1449)），
  逐 chunk `tokio::select!{ signal.cancelled(), stream.next() }`（[:1463-1471](../../aimux-ffi/src/lib.rs#L1463)），
  biased 抢占，abort 后 `return` 退出 async 块、stream 被丢弃、reqwest 连接随之释放。**无悬挂 task**。
- **三层 Drop 兜底链**（abort/放弃时）：`ObservedByteStream::Drop` 补全 exchange（[http.rs:540](../../aimux-provider-utils/src/http.rs#L540)）→
  `RecordingOutcomeStream::Drop` 记 `Cancelled`（[recording.rs:976-988](../../aimux-core/src/recording.rs#L976)）。
  录制语义在 cancel 路径完整。
- **FFI 重入守卫正确**：`IN_FFI_BLOCK_ON` thread-local + `Reset` Drop guard（[lib.rs:201-230](../../aimux-ffi/src/lib.rs#L201)），
  panic 时也能复位；`runtime()` 用 `OnceLock`（[:182-187](../../aimux-ffi/src/lib.rs#L182)）线程安全。
  M7 在走 `ffi_block_on` 的路径上确实生效。
- **阻塞进异步 = 干净**：core/http 生产路径无 `std::fs`/`sleep`/`block_on`（replay 的 `block_on` 全在 `#[cfg(test)]`）；
  录制走 channel send（非阻塞）+ 短临界区 Mutex，不在 async 线程做阻塞 IO。
- **默认全关 = 零成本**：`recorder()`/`session_store()`/`session_inferer()` 关闭时各 1 读锁+1 Arc clone 返回 None 即短路，
  `TraceLayer` 未包裹时无开销。claim 成立。

> 注：abort 路径唯一缺口是 F1——trace 层在该路径丢记录（录制层不丢）。

---

## 4. Top 5 优先修复建议

1. **F1 — 给 TraceLayer 流式包装加 Drop 兜底**（major，正确性）：把 `do_stream` 的内联 `async_stream!`
   换成带 `Drop` 的 struct（仿 `RecordingOutcomeStream`），未终结 drop 时以 Incomplete/error 兜底 `record`。
   这是 trace 子系统在 cancel/abort/take(N) 场景下「丢记录」的唯一根因，且与录制层行为不一致，优先级最高。
2. **F2 — flush 失败可观测**：`aimux_recording_flush` 在 `recv_timeout` 超时时返回非 0 / error 信封，
   别让「30s 后放弃」伪装成成功。
3. **F3 + F4 — 缩小 RingTraceStore 持锁面**：墓碑计数改 `AtomicU64`、索引重建移出 lookup 使其可降级为读锁；
   查询/导出 API 先在锁内 clone 快照、释放后再序列化/写 IO，避免导出阻塞热路径写入。
4. **F6 — trace 记录 offload**：`record()` 的 denoise 深拷贝+序列化+哈希移到 `spawn_blocking`，
   或用 `Arc<Value>` 共享 request_body 避免 clone，降低 async worker 尾延迟（与 F3 叠加收益最大）。
5. **F5/F7/F8 — 消除热路径上的 O(n) 与多余分配**：`session_chain` 用 call_id 索引避免线性 find；
   SessionStore LRU `touch` 换 O(1) 结构；`to_http_record` 直接 `take(cap)` 而非先物化整段 body。

---

## 5. 剩余不确定性

- 未运行 `cargo check`/`cargo test`（遵循 p4 已验证基线 + 避免与并发 agent 冲突）；以上为纯静态阅读结论。
- **F1 的实际触发频率**取决于宿主是否常在流自然结束前 drop（abort/超时/take(N)）。若宿主约定「必消费到 EOF」则不触发；
  但 abort 信号是公开 ABI（`aimux_stream_text_with_abort`），取消是支持的操作，故按「会触发」报告。
- **FFI `block_on` 的线程假设**：`ffi_block_on` 走 FFI 自有 `OnceLock<Runtime>`，其正确性依赖「宿主从非 tokio worker 线程调用」
  （napi/PyO3 常规桥接满足）。若宿主从自身 tokio worker 线程调用任意 FFI 入口（含 generate/stream），
  `runtime().block_on` 仍可能嵌套 panic——此为 FFI 同步契约的既定假设，非本轮新发现，未深挖。
- **瞬态 `Runtime` drop 后持 handle 调 `block_on` 的确切行为**（p4 D1「存疑」项）依赖 tokio 版本内部细节，
  本轮未构造可执行用例验证是否真的挂起/可用，仅确认代码形态未变。
- sse.rs 与 logging.rs 经核实 v0.2.1..HEAD **零改动**，p4 P2/P3（纯 CR 分隔符、无冒号字段）与 N3（脱敏分叉）结论不变，
  按范围未展开。
