# RFC-0028: Transcription 流式支持(实时 STT)

> **Status**: 已实现(2026-08-14,三个 Phase 一次性落地;实现细节与本文差异见 §10)
> **Date**: 2026-08-14
> **Scope**: 为 `TranscriptionModel::do_stream` 落地完整实现 —— WebSocket 传输基础设施 + OpenAI realtime provider 实现(Rust 核心),以及后续 FFI 会话式 API + 8 语言绑定
> **Related**: [RFC-0008](0008-multimodal-bindings.md) §2.3 Mode C(当初 defer 的决策)、[#43](https://github.com/arcships/aimux/issues/43)、[RFC-0016](0016-align-with-aisdk.md) H1(abort 基础设施)
> **Closes**: #43(Phase 1-3 全部落地时)

---

## 1. 背景与调研结论(2026-08-14)

### 1.1 AI SDK 侧现状

- transcription 流式在 AI SDK 是**实验性功能**(类型带 `Experimental_` 前缀,文档明示契约可能在 patch 版本变化)
- 契约是**真双向流**:`audio: ReadableStream<Uint8Array | string>`(音频 chunk 流入)+ `TranscriptionStreamPart` 流(转录流出)
- **底层全部是 WebSocket,没有例外**:OpenAI `gpt-realtime-whisper*`、ElevenLabs `scribe_v2_realtime`、xAI Grok STT、Cartesia Ink-2。普通预录模型(whisper-1 等)不支持流式
- OpenAI 实现要点(`packages/openai/src/transcription/openai-transcription-model.ts`):
  - 连 `wss://.../realtime?intent=transcription`
  - 发 `session.update`(session.type=transcription + input_audio_format + turn_detection=null)
  - 音频 chunk 逐个 `input_audio_buffer.append`(base64),带 WS 缓冲背压
  - 入站事件映射:`conversation.item.input_audio_transcription.delta` → `transcript-delta`;`...completed` → `transcript-final` + `finish`
  - 浏览器 WS 不能设 header,API key 走 `openai-insecure-api-key.<token>` subprotocol(Rust 侧无此限制,见 §3.2)
- 流出的 8 种 part 与 aimux **已有的** `TranscriptionStreamPart` 一一对应(stream-start / transcript-delta / transcript-partial / transcript-final / response-metadata / finish / raw / error)

### 1.2 aimux 侧现状

| 层 | 状态 | 位置 |
|---|---|---|
| core 类型 | ✅ **全部已定义且与 AI SDK V4 对齐**:`do_stream` 签名、`AudioChunk` 输入流(`Pin<Box<dyn Stream>>`)、`TranscriptionStreamPart` 8 变体、TS 类型已导出 | [transcription_model.rs](../aimux-core/src/transcription_model.rs) |
| FFI 输出方向 | ✅ `on_part`/`on_done` 回调模式成熟 | `aimux_stream_text` |
| 响应流基础设施 | ✅ `send_stream_timed`(abort/timeout/retry)+ SSE/NDJSON 解析器 | [http.rs](../aimux-provider-utils/src/http.rs)、aimux-stream |
| provider 实现 | ❌ 9 个 STT provider(openai/deepgram/cartesia/vertex/assemblyai/gladia/elevenlabs/revai/fal)全部只有 `do_generate`(单次 POST + 整段 JSON),**零个实现 `do_stream`** | 全部 STT provider |
| WebSocket | ❌ **完全没有**(全仓零引用);而所有实时 STT API 都是 WS 协议 | — |
| 请求体流式上传 | ❌ `HttpBody` 只有 `Json`/`Bytes`/`Empty` | [http.rs](../aimux-provider-utils/src/http.rs) |
| FFI 音频输入 | ❌ 目前整段音频一个 base64 C 字符串,无"分块推入"机制 | `aimux_transcription_generate` |

**核心判断**:没有 WebSocket 就没有 transcription 流式 —— 不是传输选型问题,是所有主流实时 STT 的唯一协议。本 RFC 的真实成本 = **引入 tokio-tungstenite 依赖 + 全新 FFI 会话式 wire 模式 + 8 语言绑定**,是 RFC-0008 §2.3 当初 defer(Mode C)的根本原因。

### 1.3 定位边界

- **做**:单次会话内的实时音频转录(麦克风 → 文本流)。上层应用把麦克风采集的 chunk 推进来,收到增量转录
- **不做**:多轮对话式 realtime(Voice 模式的 function calling / 会话状态机);speaker diarization 之类 provider 特性的深度封装(透传 provider_options 即可)
- 与 AI SDK 相同的实验性定位:契约跟随上游,允许 patch 版本调整

---

## 2. 总体架构

```
宿主语言 (Rust 直连 / Node / Python / C-ABI×5)
   │  音频 chunk 流入                     ▲ 转录 part 流出
   ▼                                      │
┌─────────────────────────────────────────────────┐
│ Phase 1: core                                    │
│   TranscriptionModel::do_stream(TranscriptionStreamOptions)│
│     audio: Stream<AudioChunk> ──► WS pump ──► wss://…/realtime │
│     WS events ──► part 映射 ──► Stream<TranscriptionStreamPart>│
│   (OpenAI gpt-realtime-whisper 首个实现)          │
├─────────────────────────────────────────────────┤
│ Phase 2: FFI 会话(新增 wire 模式)                │
│   session handle + push_audio + next_part        │
├─────────────────────────────────────────────────┤
│ Phase 3: 8 语言绑定                               │
│   Node/Python native async;C-ABI 5 语言 session 类│
└─────────────────────────────────────────────────┘
```

分三个 Phase,各自独立成 PR、独立闭环。**Phase 1 完成后 Rust 用户即可用**;Phase 2/3 视真实需求决定是否推进(若无跨语言需求可无限期缓,不影响 Phase 1 价值)。

---

## 3. Phase 1 — Rust 核心

### 3.1 WebSocket 基础设施(`aimux-provider-utils/src/ws.rs` 新增)

```rust
pub struct WebSocketRequest {
    pub url: String,                        // wss:// 或 ws://
    pub headers: Vec<(String, String)>,     // Authorization 等直接设 header(Rust 无浏览器限制)
    pub subprotocols: Vec<String>,          // 可选(某些 provider 需要)
    pub abort_signal: Option<AbortSignal>,
    /// 超时沿用 `TimeoutConfiguration` 既有字段:`total_ms` 管整个会话,
    /// `first_chunk_ms` 映射为"连接 + session.update 应答"(首事件),无独立 connect 字段。
    pub timeout: Option<TimeoutConfiguration>,
}

pub struct WsConnection {
    /* 封装 tokio_tungstenite::WebSocketStream */
}

impl WsConnection {
    pub async fn send_text(&mut self, s: &str) -> Result<(), AiMuxError>;   // send 天然带背压(等待写完成)
    pub async fn send_binary(&mut self, b: &[u8]) -> Result<(), AiMuxError>;
    pub async fn next(&mut self) -> Option<Result<WsMessage, AiMuxError>>;  // None = 关闭
}

pub async fn ws_connect(req: WebSocketRequest) -> Result<WsConnection, AiMuxError>;
```

**依赖决策**:`tokio-tungstenite` + rustls(workspace 已有 rustls 0.23 生态:reqwest 走 `rustls-tls`、无 native-tls,tokio-tungstenite 的 rustls feature 对齐)。

- **feature-gate 跨 crate 管道**(cargo feature 不向下传播,必须逐层声明):
  - `aimux-provider-utils/Cargo.toml`:`tokio-tungstenite = { version = "…", optional = true }` + `[features] ws = ["dep:tokio-tungstenite"]`;`ws.rs` 与 `ws_connect` 用 `#[cfg(feature = "ws")]` 门控
  - `aimux-providers/Cargo.toml`:`[features] realtime = ["aimux-provider-utils/ws"]`(默认 features 含 `realtime`);realtime transcription 实现同样 `#[cfg(feature = "realtime")]`
  - 这样 `--no-default-features` 时 tokio-tungstenite 整个离开依赖图,release profile(opt-level="z"+strip+lto)的最小二进制退路成立
- **背压**:tungstenite 的 `send().await` 驱动 flush、在 socket 写缓冲满时 pending —— 即 socket 级天然背压(等价 AI SDK 的 `waitForWebSocketBufferDrain`;后者只是浏览器同步 send 的补丁,Rust 侧不需要)。注意这是 socket 级而非"服务端已读"级
- **proxy(诚实声明)**:tokio-tungstenite **没有 proxy 参数**(Connector 只选 TLS)。全局 `ProxyConfig` 无法免费复用,reqwest client 也不能给 WS 用。Phase 1 MVP **WS 不走 proxy**(直连);proxy 隧道(手动 TCP 拨 proxy → HTTP CONNECT → `client_async_tls_with_config` + 自备 rustls ClientConfig + 自实现 no_proxy 匹配)留作后续增强,记入 Open Questions
- **abort 模式(强制)**:**每一个 await 点**(connect / 每次 send / 首事件 / 每次事件接收)都必须在 `select!` 内与 abort token + timeout 竞争 —— 这是 RFC-0016 R1–R4 为 HTTP 修过的同类 bug(单纯"循环里 select"盖不住 send 路径),实现与 review 都按此验收
- 错误映射:WS close frame / 意外断开 → `AiMuxError::ApiCall`(message 带 close code)

### 3.2 OpenAI realtime 实现(`aimux-providers/src/openai/transcription.rs` 的 `do_stream`)

**模型门控**:`gpt-realtime-whisper*` 前缀(含 dated snapshot)走 `do_stream`;其余模型调用 `do_stream` 返回 `UnsupportedFunctionality`(与现状的 `do_generate` 门控对称 —— 非实时模型走 `do_generate`,实时模型走 `do_stream`)。

**协议序列**(对齐 AI SDK 的实际 wire 形状 —— model 不在 URL 里,在 `session.update` 的嵌套结构中传递):

```
1. connect  wss://{base}/realtime?intent=transcription
            headers: Authorization: Bearer <key>(Rust 侧可直接设 header。AI SDK 用
            openai-insecure-api-key.<token> subprotocol 是浏览器 WS 不能设 header 的
            变通;服务端拒绝双通道,header-only 成立)
2. send     {"type":"session.update","session":{
              "type":"transcription",
              "audio":{"input":{
                "format": {"type": <fmt>, "rate": <rate>},   // ← options.input_audio_format
                "transcription": {"model": <self.model_id>}, // ← model 在这里,不是 URL
                "turn_detection": null                       // ← 嵌套在 audio.input 下
              }}}}
3. loop select! 四路(每个 await 点都在 select 内,见 §3.1 abort 模式):
   a. options.audio.next() ──► {"type":"input_audio_buffer.append",
                                 "audio": base64(chunk)}     // AudioChunk::Binary→base64, Base64→原样
      音频流结束 ──► {"type":"input_audio_buffer.commit"}
   b. ws.next() ──► 事件映射:
      conversation.item.input_audio_transcription.delta
          → TranscriptDelta { id: item_id, delta }
      conversation.item.input_audio_transcription.completed
          → TranscriptFinal { text, start/end_second } + Finish { text, segments }
          然后 → 客户端主动 close(1000),流结束     // ← 终止条件,见下
      rate_limits / error 事件 → provider_metadata / Error part
   c. abort token ──► 关闭 WS,return Aborted
   d. timeout ──► return Timeout
4. 终止:收到 completed 事件即发 Finish 并**客户端主动关闭 WS(close code 1000)**
   —— realtime 会话是长连接设计,服务端不会在单次转录后关闭;
   等服务端 close 会把 happy path 挂死(AI SDK 同样在 finish 后 cleanup(1000))
```

**单 `select!` 循环**(而非 AI SDK 的双任务 + 手动背压):tungstenite 的异步 send 已含背压,音频 pump 和事件 pump 合并在一个循环里更简单、无竞态(双向同时阻塞的对称死锁在"音频字节量 ≫ 转录字节量 + 内核缓冲"的现实下不可达)。

**首事件发 `StreamStart { warnings: vec![] }`,收到 completed 后发 `Finish`** —— 与 core 已定义的 `TranscriptionStreamPart` 完全一致,零类型改动。

### 3.3 Phase 1 测试

- 单元:本地起真 WS listener(tokio-tungstenite server 侧)做集成测试 —— mock OpenAI 会话:收到 session.update → 回 delta 事件 → 回 completed;断言 part 序列、base64 编码、abort 中途取消、客户端主动 close
- **live-API smoke(一次,手工)**:本地 mock 验证不了"session.update 的嵌套结构服务端是否接受"这类 wire 真值 —— Phase 1 收尾时用真实 key 对 `gpt-realtime-whisper` 跑一次最小会话(几秒 PCM),确认协议序列无误;结果记回本 RFC
- 门控:非 realtime 模型调 `do_stream` → `UnsupportedFunctionality`
- feature-gate:`--no-default-features` 编译通过(realtime 路径被编译掉)

### 3.4 Phase 1 范围外

- 其他 provider(ElevenLabs/Cartesia 等)的 WS 实现 —— 骨架同构,后续按需加
- FFI / 绑定 —— Phase 2/3

---

## 4. Phase 2 — FFI 会话式 API(新增 wire 模式)

### 4.1 为什么不能用现有回调模式

`aimux_stream_text` 是**阻塞到流结束**的推模式(on_part/on_done 回调)。双向流要求"推音频的同时收转录",宿主线程不能被占死 —— 需要会话句柄 + 非阻塞操作。

### 4.2 设计:会话句柄 + 推入/拉出

```c
/* 创建转录会话。opts_json: { input_audio_format: {format_type, rate},
   provider_options, headers }。abort_handle 可为 0(无取消);
   非零时复用 aimux_abort_signal_* 体系(与 aimux_stream_text_with_abort 对称)。
   返回非零会话句柄;内部立即 spawn tokio task 驱动 do_stream。*/
uint64_t aimux_transcription_session_new(uint64_t model_handle,
                                         uint64_t abort_handle,
                                         const char *opts_json, AimuxError *err);

/* 推入音频 chunk(二进制)。**阻塞式**(有界 channel 满时等待 —— 把 WS 背压
   传导回宿主的麦克风采集循环,防无界内存);经 ffi_block_on 驱动,故不可从
   aimux 回调内重入(与全仓一致)。返回 1 成功 / 0 失败(err 区分会话已结束 /
   已 abort / channel 已关)。*/
int32_t aimux_transcription_push_audio(uint64_t session,
                                       const uint8_t *data, size_t len,
                                       AimuxError *err);

/* 信号"音频输入结束"(等价音频流 None)。返回 1/0。*/
int32_t aimux_transcription_input_done(uint64_t session, AimuxError *err);

/* 拉取下一个转录 part(JSON 序列化 TranscriptionStreamPart),带超时。
   timeout_ms: >0 等待上限;0 立即返回(纯 poll);<0 无限等待。
   返回 JSON 字符串(aimux_free_string 释放)。NULL 时 err.code 区分三种情况:
     AIMUX_E_TIMEOUT  → 超时(会话仍活,可继续调用)
     AIMUX_E_OK       → 流正常结束(收到 Finish 后 channel 关闭)
     其他错误码       → 流中途出错(abort / API error,细节在 err)
   注意:err 传 NULL 时无法区分上述三种,调用方必须传非 NULL err。
   (成功路径不写 err,与全仓约定一致。) */
char *aimux_transcription_next_part(uint64_t session, int64_t timeout_ms,
                                    AimuxError *err);

/* 结束会话并释放(幂等,0 安全)。 */
void aimux_transcription_session_drop(uint64_t session);
```

**内部结构**:

```
session_new ──► spawn tokio task ──► TranscriptionModel::do_stream(audio_rx, …)
                                        │ parts 流出
push_audio  ──► mpsc::Sender<AudioChunk>│   (有界,容量 64,见 Open Questions)
input_done  ──► drop Sender(流 None)   ▼
next_part   ◄── mpsc::Receiver<Result<Part>> + recv_timeout
session_drop ─► 先从 registry 摘除句柄 → 再 abort token → join task → 释放
```

- **push 是阻塞式**:背压经"WS send 背压 → do_stream 内部 audio 流 → 有界 channel"一路传回宿主(等价朗读:麦克风循环天然节流)。与 §7 风险表一致
- **拉取式而非回调式**:C ABI 下拉取对宿主要求最低(无线程安全/重入约束),且超时语义显式。回调式留作后续可选增强
- **drop 顺序防死锁**:(1) 先从 registry 摘除(不持全局 mutex 做 join —— 否则 join 期间阻塞所有其他 FFI 调用);(2) abort token 必须 select 在**两处**——WS 循环(§3.2)和"parts → channel 转发循环"(否则宿主停调 next_part、channel 满、任务卡在 send,join 永远不返回);(3) join(带兜底超时,如 5s,防极端卡死)
- session 句柄复用现有 `ModelHandle` registry(新增 `TranscriptionSession` 变体)
- **录制(RFC-0023)**:chat 流式有层 A 录制(`stream_text` 内 record_input/out);transcription 今天完全不录制是因为**没有 core 级转录入口**(generate.rs 无 `transcribe()`),不是"流式不录"的先例 —— 会话录制需要 session 内生成 call_id + 音频/part 的 recording_context 接线,Phase 2 暂不做,记入 Open Questions

### 4.3 Phase 2 测试

FFI 层:mock provider 实现 `do_stream` → 验证 push/next_part/input_done/drop 全生命周期、超时语义(`AIMUX_E_TIMEOUT` vs `AIMUX_OK`)、push 在 drop 后报错、abort 传播。

---

## 5. Phase 3 — 8 语言绑定

| 语言 | 形态 |
|---|---|
| **Node**(native) | ~~streamTranscribe + ReadableStream/AsyncGenerator~~ → **实际落地为会话对象**(见 §10 D1):`startTranscriptionSession(model, optsJson?, bridge?)` 返回 `TranscriptionSession` 类(pushAudio/inputDone/nextPart/close) |
| **Python**(native) | 同 D1:`start_transcription_session(model, opts_json?)` + `TranscriptionSession` pyclass(push_audio/input_done/next_part/close) |
| **Go/Swift/Java/Kotlin/Flutter**(C-ABI) | 各包一个 `TranscriptionSession` 类:start → pushAudio(bytes) → nextPart(timeoutMs) → inputDone → close。模式同各自的 Model wrapper(锁 + handle);Go 用 RWMutex 允许并发 push/pull(双向会话不死锁) |

Node/Python 的 native 路径不走 Phase 2 的 FFI 会话(直连 core,与 generateText 同架构);C-ABI 5 语言消费 Phase 2。

---

## 6. Non-Goals

1. **不做多轮 realtime 会话**(function calling / 会话状态机)—— 那是 Voice Agent 编排层
2. **不做 WS 层的通用化**(给 chat 模型开 WS 通道)—— 只为 transcription 服务,接口最小化
3. **不透传 WS 原始帧** —— `include_raw_chunks` 已在 options 里,按 AI SDK 语义映射到 `Raw` part
4. **不做浏览器端 polyfill 语义**(subprotocol 鉴权)—— Rust 直连 header 即可;Node binding 也是 native WS

## 7. 风险

| 风险 | 等级 | 对策 |
|---|---|---|
| tokio-tungstenite 增大二进制 | 中 | feature-gate `realtime` 默认开、可关;实测增量后记录到 RFC |
| OpenAI realtime 契约变化(AI SDK 自己都标实验性) | 中 | 契约集中在 transcription.rs 的事件映射表一处;版本变化只动映射 |
| FFI 会话 wire 是全新模式,5 语言 wrapper 工作量大 | 高(Phase 2/3) | 分 PR;Phase 1 独立价值;Phase 2/3 无真实需求可缓 |
| 背压语义(音频推太块) | 低 | FFI push **阻塞式**:有界 channel(容量 64)满则等待,把 WS 背压传回宿主采集循环(与 §4.2 一致);core 侧 send().await 天然背压 |
| 测试依赖真 WS 交互 | 低 | 本地 tokio-tungstenite server mock,不打真实 API |

## 8. 实施计划

| 阶段 | 内容 | 依赖 | 状态 |
|------|------|------|------|
| **Phase 1** | ws.rs 基础设施 + OpenAI `gpt-realtime-whisper` do_stream + 测试 | 无 | ✅ 已实现(2026-08-14) |
| **Phase 2** | FFI 会话 API(5 个新符号)+ 测试 | Phase 1 | ✅ 已实现(2026-08-14) |
| **Phase 3** | 8 语言绑定 | Phase 2(C-ABI)/ Phase 1(native) | ✅ 已实现(2026-08-14) |

**实施记录**:三个 Phase 在一个 PR 内落地(#43)。R1-R3 三轮独立 review 的发现(超时死代码、ws 定时器语义、绑定内存/死锁)全部修复;R3 通过后合并。

## 9. Open Questions

1. **channel 容量**:FFI push_audio 的有界 channel 容量(草案 64 chunk)是否需要可配?MVP 固定,文档标注。
2. **WS proxy 隧道**:tokio-tungstenite 无 proxy 支持(§3.1);企业 mTLS/代理场景需要手动 CONNECT 隧道 + 自备 rustls ClientConfig + no_proxy 匹配。Phase 1 直连,后续按需求实现。
3. **next_part 的零拷贝变体**:是否需要 `aimux_transcription_next_part_into(session, buf, len)`(调用方提供缓冲避免 JSON 字符串分配)?MVP 用 JSON 字符串(与全仓 wire 一致),性能需求出现再加。
4. **ElevenLabs/Cartesia 等其他 realtime provider**:骨架同构但协议各异,是否在 Phase 1 一起做?建议 Phase 1 只做 OpenAI(验证骨架),其余按需。
5. **录制支持**(RFC-0023):转录会话暂不录制(chat 流式有层 A 录制;transcription 无 core 入口是现状起点);若要,session 内部生成 call_id + recording_context 接线 —— 留待需求。

---

## 10. 实现记录(2026-08-14,与设计稿的差异)

1. **D1 — Node/Python 也用会话对象而非流式糖**(§5 变更):原设计给 Node/Python 做流式语法糖(ReadableStream 进 / AsyncGenerator 出)。落地时改为与 C-ABI 同构的 push/pull 会话——(a) 三条路径行为一致、文档单份;(b) napi 的 ReadableStream→Rust Stream 桥接与 PyO3 的 async-iterator 桥接各自都是不小的基础设施,而会话对象在两边都是薄层;(c) 流式糖可以后续在会话之上加纯 TS/Python 包装(零 wire 改动)。
2. **D2 — `TranscriptionStreamOptions` 增加 `timeout` 字段**(R1 B1):设计稿的超时只在 ws 层;实现把 `Option<TimeoutConfiguration>` 提到 options(FFI opts_json 的 `"timeout"` 对象 / Node/Python opts 同),`first_chunk_ms` 为 connect+首事件**合并预算**(锚定在 connect 前,connect 后只花余额)。
3. **D3 — peer close 携带 code/reason**(R1 S3):设计稿只说"close frame → ApiCall 带 code";实现为 `"websocket closed by peer (code NNN: reason)"`。
4. **D4 — live-API smoke 未执行**:§3.3 的"用真实 key 跑一次最小会话"需要 API key,未在 CI/本地执行。wire 形状已由本地 WS 集成测试逐字段断言(含 session.update 嵌套结构与 turn_detection 嵌套位置);首次真实调用时如遇 shape 偏差请回报此 RFC。
5. **D5 — 其余 review 修正**:ws close() 5s 有界;chunk-idle 窗口按 next() 调用计算(ping 不续命);FFI 驱动的 futures-mpsc `flush()` 不可用于断连检测(改 `try_send + is_disconnected`);五个 C-ABI 绑定的 timeout 路径先消费错误串(防泄漏);Go 会话 RWMutex(并发 push/pull);Python push/next 释放 GIL(allow_threads)。
