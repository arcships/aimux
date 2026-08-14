# Round 4 / 4e — RFC-0028 transcription streaming 首审（commit cf2cea5）

## 概述

对象：commit `cf2cea5` "feat: transcription streaming — all RFC-0028 phases (WS + FFI session + 8 bindings)"（#43/#108，2026-08-14 合入，当前 HEAD）。范围 29 文件 / +3214 −71：`aimux-provider-utils/src/ws.rs`（全新）、`aimux-providers/src/openai/transcription.rs`（do_stream）、`aimux-ffi/src/transcription_session.rs` + lib.rs 会话 wire、8 语言绑定、6 个流式集成测试 + 5 个 FFI 会话测试、RFC §8/§10 状态同步。

该 commit 经过了 R1–R3 三轮前置 review，多数高危问题（超时死代码、close 无界、Go 死锁、Node 死类、Python 未注册 pyclass）已在合入前修复并验证。本次首审在 R3 之外新发现 **1H / 7M / 9L**。总体评价：核心 WS 层与 FFI 会话的错误传播设计质量高（每个 await 点有 abort/timeout 覆盖、drop 顺序防死锁有据），剩余问题集中在**绑定层一致性**（Kotlin 哨兵异常不可达、Go 缺 timeout 字段、Python 缺 abort）与若干**静默丢弃路径**（非法 JSON 帧、rate_limits 事件）。

与本轮 H1 修复模式的关系：会话 API 是纯拉取式（无宿主回调），不经过 `invoke_stream_callback`；新增代码无回调调用点，H1 的 catch_unwind 模式在此 wire 形态下不适用（确认无遗漏）。头文件与 94 导出的零漂移结论此前已独立验证，本次未重跑。

## 方法

- `git show cf2cea5 --stat` 定范围 → 逐文件读 diff 与全量新文件（HEAD 即该 commit，工作区干净，直接读工作区文件等价于 commit 内容）。
- 对照 `rfc/0028-transcription-streaming.md`（含 §10 实现偏差清单 D1–D5）核对接口签名、事件枚举映射、错误语义。
- 重点清单：ws.rs 每个 await 点的错误传播（`.ok()/let _ =/unwrap` 清点）；transcription.rs 协议事件→核心事件映射的静默丢弃；FFI 句柄生命周期（drop 顺序/double-free/泄漏）与并发竞态；8 绑定 API 风格与错误冒泡；测试覆盖缺口。
- 只读审计：未运行任何 cargo 命令；未修改任何源码。

## 发现列表

### H（1 项）

**F1. Kotlin `nextPart` 超时哨兵异常不可达（公开 API 契约破坏）**
- 位置：`/Users/eric8810/Code/aimux/bindings/kotlin/src/main/kotlin/ai/arcships/aimux/Multimodal.kt:299-305`
- 证据：
  ```kotlin
  if (err.code == AIMUX_E_TIMEOUT) {
      // throwFromC consumes (frees) the message strings, then we swap
      // in the retryable sentinel.
      throwFromC(err)                                    // :303
      throw AimuxTranscriptionTimeoutException()         // :304 — 死代码
  }
  ```
  `throwFromC` 声明为 `internal fun throwFromC(err: AimuxCError): Nothing`（`Model.kt:196-201`），**必然抛出** `AimuxException`。第 304 行永不执行：超时永远以泛型 `AimuxException` 冒出，文档（KDoc `@throws AimuxTranscriptionTimeoutException ... retryable`，即轮询循环的推荐捕获类型）承诺的哨兵永不出现。对比其余 4 个 C-ABI 绑定：Java `AimuxException.fromC(...)` 返回值式（语句丢弃后再 throw 哨兵，正确）、Swift `AimuxError.fromC` / Go `errorFromC` / Flutter `AimuxException.fromC` 均为值式，唯 Kotlin 的 helper 是 throw 式。R1"S6 修复"只修了内存释放，没发现哨兵被吞。
- 影响：Kotlin 用户按文档写 `catch (e: AimuxTranscriptionTimeoutException) { continue }` 的轮询循环会直接漏出异常。
- 建议：新增一个不抛出的释放函数（如 `internal fun freeCError(err: AimuxCError)`），timeout 分支先释放再 `throw AimuxTranscriptionTimeoutException()`；并补一个绑定层单测断言哨兵类型。

### M（7 项）

**F2. `aimux_transcription_session_drop` 对句柄类型盲，可静默销毁其他类型的句柄**
- 位置：`/Users/eric8810/Code/aimux/aimux-ffi/src/lib.rs:2306-2314`
- 证据：drop 先 `registry().lock()...remove(&session)` 再 `if let Some(ModelHandle::TranscriptionSession(s)) = removed { s.terminate(); }`。若宿主误传 model/abort 句柄，该句柄被从 registry **无条件移除**（if-let 不匹配 → 直接丢弃 Arc），模型句柄静默失效、无错误返回。对比同文件 `drop_abort_signal`（lib.rs:175-182）先用 `matches!(registry.get(&handle), Some(ModelHandle::Abort(_)))` 做类型检查再移除——新代码没有沿用该防御。
- 建议：与 `drop_abort_signal` 对齐——类型不匹配时不移除（0 安全语义保留）。

**F3. user-abort 链接任务每会话泄漏（FFI 与 Node 两处同型）**
- 位置：`/Users/eric8810/Code/aimux/aimux-ffi/src/transcription_session.rs:92-99`；`/Users/eric8810/Code/aimux/bindings/node/src/multimodal.rs`（`start_transcription_session` 内 `sources.push(b.core_signal())` 后的 `napi::tokio::spawn`）。
- 证据：`effective` 信号靠"每个源 spawn 一个转发任务"实现 OR 合成。`session_drop`/`close()` 只 fire 内部 `token`；若宿主的 user abort 信号从未触发，该转发任务在 `source.cancelled().await` 上**永久 pending**，且持有 user `AbortSignal`（CancellationToken）克隆，存活至 runtime 终止。FFI 长驻进程中每创建一个带 abort_handle 的会话泄漏一个任务；Node 的 bridge 同理（`AbortSignal` 无 OR 组合的注释已说明动机，但没说明代价）。
- 建议：转发任务内在 token 分支完成后即退出（例如 `tokio::select!` 两个源都监听、任一触发后双方任务都结束），或给 user-abort 链接任务挂一个会话结束通知。

**F4. 服务端非 JSON 文本帧被静默丢弃（无 Raw、无 warning、无错误）**
- 位置：`/Users/eric8810/Code/aimux/aimux-providers/src/openai/transcription.rs:595-597`
- 证据：`let Ok(value) = serde_json::from_str::<Value>(&text) else { continue; };` —— parse 失败的文本帧直接跳过，即使 `include_raw_chunks=true` 也不产出 `Raw` part，也不产生 `Warning`。协议漂移（服务端改发非 JSON 或下发 HTML 错误页式帧）时用户看到的现象是"流静默无输出直至 idle 超时"。
- 建议：至少 `continue` 前 yield 一个带截断原文的 `Warning`（StreamStart 已有 warnings 通道的先例是开始时注入，可改为流中 Raw part），或 parse 失败映射为 Error part。

**F5. `rate_limits` 事件未映射 provider_metadata —— 与 RFC §3.2 不一致且 §10 未记录**
- 位置：`/Users/eric8810/Code/aimux/aimux-providers/src/openai/transcription.rs:652-655`
- 证据：RFC §3.2 步骤 3b 写明 "rate_limits / error 事件 → provider_metadata / Error part"；实现中 error 有映射，rate_limits 落入 `_ => {}`（注释还把它列为"no part mapping"的代表）。RFC §10 的偏差清单 D1–D5 未收录此项。
- 建议：要么补映射（如挂到相邻 part 的 `provider_metadata`），要么在 §10 增记 D6 说明丢弃理由。

**F6. Go 类型化 opts 缺 `timeout` 字段 —— 8 条路径中唯一无法设置 WS 超时的绑定**
- 位置：`/Users/eric8810/Code/aimux/bindings/go/multimodal.go:745-750`
- 证据：`TranscriptionSessionOpts` 只有 InputAudioFormat/ProviderOptions/Headers/IncludeRawChunks。FFI opts_json（aimux-ffi.h:466-468 明示）、Node/Python 的 SessionOpts 均含 `timeout`；Swift/Java/Kotlin/Flutter 收原始 JSON 字符串可手写传入，唯 Go 用户被类型化结构挡住。
- 建议：加 `Timeout *TranscriptionTimeoutOpts \`json:"timeout,omitempty"\``（或直接内嵌 map）。

**F7. Python 会话缺外部 abort 参数 —— 8 条路径中唯一不支持**
- 位置：`/Users/eric8810/Code/aimux/bindings/python/src/multimodal.rs:569-574`
- 证据：`start_transcription_session(model, opts_json=None)`。Node 有 `bridge`、FFI/Go/Swift/Java/Kotlin/Flutter 均有 abort_handle 参数；Python 仅靠 `close()`（会终止整个会话而非协作取消）。无法把会话纳入既有的 abort 信号编排（如与其它调用共享取消）。
- 建议：加可选 `abort: Optional[AimuxAbortSignal]`（或 opts_json 约定），与其它绑定对齐；至少在 docstring 说明该限制。

**F8. 错误路径测试缺口（聚合）**
- 位置：`/Users/eric8810/Code/aimux/aimux-providers/tests/openai_transcription_stream_test.rs`、`/Users/eric8810/Code/aimux/aimux-ffi/src/lib.rs`（tests mod）
- 已覆盖：happy path + 客户端 close(1000)、session.update wire 形状回归、base64 编码/透传、abort 中途、非 realtime 拒绝、first-chunk 超时；FFI 全生命周期、可重试超时、abort、幂等 drop、坏输入。
- 零测试的路径：① **connect 失败**（服务端不监听 → `ws_error("websocket connect failed")`，含握手 401/403 路径）；② 服务端 **error 事件 → Error part** 映射（transcription.rs:640-651）；③ **peer close frame 携带 code/reason**（R1 S3 修复本体，无回归测试）；④ **chunk-idle 与 total 超时**（R1 S2 修复本体，无回归测试——ping 续命场景未验）；⑤ FFI **push 在 input_done 之后** 报错路径；⑥ channel 满时 push 的**阻塞背压**行为；⑦ join 5s 超时 detach 分支；⑧ **8 个绑定零行为测试**（commit 只验证了编译/工件）。③④ 是上一轮 review 修的 bug，无测试锚定最容易回退。
- 建议：优先补 ①③④（provider 层本地 WS server 即可）与 Kotlin/Go 各一个 nextPart 哨兵行为测试（直接捕获 F1/F6 类问题）。

### L（9 项）

**F9. Java `TranscriptionModel.startStream` 上方残留孤儿 javadoc**
- 位置：`/Users/eric8810/Code/aimux/bindings/java/src/main/java/ai/arcships/aimux/TranscriptionModel.java:94-103`
- 证据：两个连续 javadoc 块，第一块是 `transcribe(...)` 的文档复制粘贴残留（"Transcribe audio (base64-encoded) to text..."），不附着任何声明。建议删除第一块。

**F10. ws.rs 静默跳过 `Message::Frame(_)`**
- 位置：`/Users/eric8810/Code/aimux/aimux-provider-utils/src/ws.rs:245-248`
- 证据：`Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_))) => continue`。当前 tungstenite 0.24 的 `StreamExt::next` 不产出 `Frame` 变体（仅聚合后的消息），无害；但若未来版本在此变体交付数据帧则会被静默吞掉。建议把 Frame 从跳过列表中拆出并按数据帧处理或 debug_assert。

**F11. `total_ms` 锚点在 connect 完成后 + 无超时配置时 connect 无限挂起**
- 位置：`/Users/eric8810/Code/aimux/aimux-provider-utils/src/ws.rs:173-177`（total 锚点）、`:139-144`（connect 超时只来自 first_chunk_ms）
- 证据：`total_deadline = now + total_ms` 在 connect 成功后才计算——connect 耗时不计入 total_ms（RFC "total_ms 管整个会话" 的宽松解读，可接受但宜注明）。同时 `first_chunk_ms=None` 时 connect 无任何超时（仅有 abort），行为等同 HTTP 层无超时语义，属设计使然，建议在 `WebSocketRequest::timeout` 文档标注"无 first_chunk_ms = connect 不设界"。

**F12. Node `pushAudio` 错误类型与 `nextPart` 不一致**
- 位置：`/Users/eric8810/Code/aimux/bindings/node/src/multimodal.rs`（push_audio 返回 `napi::Result`，错误为 `Status::GenericFailure` 裸字符串；nextPart 返回 `AimuxResult` 走 MappedError 映射）。宿主难以用统一的方式分类 push 失败（ended vs poisoned）。建议 pushAudio 也走 AimuxResult 映射。

**F13. `next_part` 并发调用在 tokio Mutex 上无界串行，且未文档化**
- 位置：`/Users/eric8810/Code/aimux/aimux-ffi/src/transcription_session.rs:235`（`parts_rx.lock().await` 无超时）；头文件只写了"回调内重入禁止"，未写"两个线程并发调用 next_part 会串行、后到者可能等到前者的无限等待"。
- 证据：线程 A `next_part(s, -1)` 持锁等待时，线程 B `next_part(s, 100)` 先卡在锁上、其 100ms 超时语义失效。建议头文件/注释补一句"next_part 非并发安全（同一会话请单线程拉取）"，或锁获取也纳入超时。

**F14. C 样例绑定未覆盖新会话 API**
- 位置：`/Users/eric8810/Code/aimux/bindings/c/example.c`（0 处 `transcription_session` 引用；该 commit 的 29 文件不含 bindings/c）。C 样例本就是最小集（无 files/abort 样例），不算违背惯例；但 `next_part` 的"NULL + err.code 三态消歧 + TIMEOUT 时必须 free err.message"恰是 C 用户最容易写错的地方，建议补一段会话样例。

**F15. transcription.rs 代码卫生三则**
- 位置：`/Users/eric8810/Code/aimux/aimux-providers/src/openai/transcription.rs:495`（`std::mem::take(&mut header_list)` 多余——`header_list` 之后不再使用，直接移动即可，`mut` 也可去）；`:667`（`serde_json::to_string(&session_update).unwrap_or_default()`——对已知可序列化值静默吞错，建议 `unwrap_or_else` 带 tracing 或直接 expect）；`:542-567`（audio arm 发送失败 `yield Err(e); break` 后未调用 `ws.close()`，socket 直接 drop 走 TCP RST——事件侧的 error/completed 路径都有 close，风格不一致）。
- 另：二进制帧 `Some(Ok(WsMessage::Binary(_))) => {}` 静默忽略（:590-593）有注释说明，属有意行为，仅提示。

**F16. `package-lock.json` 混入与特性无关的依赖变动**
- 位置：`/Users/eric8810/Code/aimux/bindings/node/package-lock.json`——typescript `^5.5.0 → ^5.7.0`、多处 `libc` 字段删除。应为新版 npm 重新生成的副产品，与 RFC-0028 无关；建议特性 PR 里避免无关 lockfile churn（或单独提交）。

**F17. 信息项：不可达 part 类型与延迟失败语义**
- ① `TranscriptionStreamPart` 的 `TranscriptPartial` / `ResponseMetadata` 在本 provider 永不产生（上游 OpenAI realtime 也不发 partial，属对齐上游，仅记录）；② FFI 对不支持 `do_stream` 的模型在 `session_new` 时不失败、首个 `next_part` 才报错——已在 aimux-ffi.h 文档化，是 spawn 即返回句柄这一 wire 设计的必然妥协；③ next_part 出现终态错误后再调用返回"正常结束"（NULL+OK），与头文件"流中途出错"的三态描述并存，属可接受的边界行为。

## 统计

| 严重度 | 数量 | 编号 |
|---|---|---|
| H | 1 | F1（Kotlin 超时哨兵死代码） |
| M | 7 | F2–F8 |
| L | 9 | F9–F17 |
| 合计 | 17 | — |

按主题分布：错误路径完整性 3（F4/F5/F11）、FFI 安全 3（F2/F3/F13）、绑定一致性 5（F1/F6/F7/F12/F14）、测试缺口 1 聚合（F8，含 8 类零覆盖路径）、RFC 一致性 2（F5/F17）、语言规范/卫生 3（F9/F15/F16）。

正面确认（首审通过项）：ws.rs 每个 await 点（connect/send/recv）均有 abort + first-chunk/chunk-idle/total 竞争（RFC-0016 R1–R4 模式成立）；chunk-idle 窗口按 next() 调用计算一次（S2 修复在位）；peer close code/reason 冒泡（S3 在位，惜无测试）；FFI drop 顺序（先摘 registry → abort → 有界 join）与 push 的 clone-before-await 正确；futures-mpsc 断连检测用 `try_send + is_disconnected` 而非 flush（S1 在位）；Go RWMutex 并发 push/pull 在位；Python allow_threads + clone-before-block_on 在位；Node `#[napi]` 方法属性与再生工件一致；RFC §10 D1–D5 偏差记录与实现相符（唯 F5 漏记）。
