# RFC-0034: 实时转写收尾 —— WS 代理、ElevenLabs/Cartesia 实现

> **Status**: P1 已实现(#183);P2 已实现(RFC-0034/elevenlabs-realtime 分支,PR 见 #184);P3 暂缓(D6);P4 待做
> **Date**: 2026-09-13
> **Scope**: 完成 RFC-0028 明确遗留的三件事:WS 代理隧道(全局 `ProxyConfig` 对 WS 生效)、ElevenLabs `scribe_v2_realtime` 与 Cartesia `ink-2` 的 `do_stream` 独立实现
> **Related**: [RFC-0028](0028-transcription-streaming.md)(本 RFC 是其遗留项的收尾)、[#178](https://github.com/arcships/aimux/issues/178)(跟踪 issue,含研究记录)、[#157](https://github.com/arcships/aimux/pull/157)(Go 会话生命周期先例)
> **Closes**: #178

---

## 1. 背景

RFC-0028 落地了 WS 基础设施 + OpenAI realtime 转写 + FFI 会话 + 8 语言绑定,但留了三个尾巴(见 #178 研究记录):

1. **WS 完全绕过代理**。`ProxyConfig`(全局 `OnceLock`,FFI/Node/Python 均暴露 `init_proxy`)只驱动 reqwest;`ws.rs` 用裸 `connect_async` 直连。对必须经代理访问 provider API 的用户,实时转写不可用。
2. **只有 OpenAI 一家实现 `do_stream`**。Cartesia 最尴尬:代码已按 `ink-2*` 门控且 `do_generate` 明确返回"仅支持 WebSocket 端点"([cartesia.rs](../aimux-providers/src/cartesia.rs) L618/L710),但 `do_stream` 不存在——两条路径都失败。ElevenLabs `scribe_v2_realtime` 零代码。
3. 上游 tokio-tungstenite **明确不做代理支持**(tungstenite-rs#177),只能自己实现 CONNECT 隧道。

### 1.1 关键前置决策:不做统一抽象

**每家独立实现,不抽公共"WS 转写会话"层。** 依据(#178 研究记录中的对照):

| | OpenAI | ElevenLabs | Cartesia |
|---|---|---|---|
| 配置载体 | 连接后 `session.update` 消息 | URL query 参数 | 连接后 `config` 消息 |
| 音频编码 | base64 进 JSON | base64 进 JSON(每帧带 `sample_rate`) | **原始二进制帧** |
| 转录事件语义 | 增量(delta,追加) | **全量替换**(partial 为当前完整文本) | turn 级事件 |
| 结束方式 | 收 `completed`,客户端 close | **无结束事件**,末帧 `commit:true` | 发 `close` 命令 |
| 触发转录 | 自动 | `commit_strategy`(manual/vad) | manual 模式需 `finalize` 命令 |

真正公共的部分——连接、背压、abort/超时竞争、close——**已经在 `ws.rs` 的 `WsConnection` 里**,三家共用;表内每一行差异都发生在 provider 层,统一层只会把每行变成一个参数或钩子,抽象本身比任何单家实现更复杂。

**复查条款**:Cartesia 落地后(第三家)若重复代码确实可观,再基于两个真实样本评估抽取;本 RFC 不做。

## 2. Phase 1 — WS 代理隧道(`ws.rs` + `http.rs` 小改)

### 2.1 行为定义

`ws_connect` 在发起连接前查询全局代理配置(复用 `http.rs` 的 `GLOBAL_PROXY`,新增只读访问器):

1. **选代理**:`wss://` → `https_url`(`all_url` 兜底);`ws://` → `http_url`(`all_url` 兜底)。均无 → 直连(现状路径,零行为变化)。
2. **no_proxy 匹配**:自实现,语义对齐 reqwest `NoProxy::from_string`(逗号分隔;后缀匹配;`*` 全匹配;带端口的条目要求端口相等)。命中 → 直连。
3. **非 http scheme 一律明确报错**:代理 URL scheme 为 `socks5`/`socks5h`(CONNECT 隧道不通)或 `https`(需要 TLS-to-proxy,TLS 套 TLS,无需求来源)时,返回 `AiMuxError::UnsupportedFunctionality`,消息注明拒绝直连原因。**绝不静默直连**——静默直连等于绕过用户的网络边界。
4. **代理 URL 带 userinfo**(如 `http://user:pass@proxy:8080`)→ CONNECT 请求附 `Proxy-Authorization: Basic base64(user:pass)`。

### 2.2 隧道流程(全部 await 点在 `select!` 内与 abort + `first_chunk_ms` 竞争,沿用 RFC-0028 §3.1 强制模式)

```
1. TcpStream::connect(proxy_host:proxy_port)
2. 写  CONNECT target_host:target_port HTTP/1.1\r\n
        Host: target_host:target_port\r\n
        [Proxy-Authorization: ...]\r\n \r\n
3. 读至 \r\n\r\n(上限 8 KiB),校验状态行 2xx(非 2xx → ApiCall,报代理状态码与
   status line,按共享 `is_retryable_status` 规则分类——407/403 等认证/策略判定
   不可重试,502/503/504 等瞬态可重试;无应答(EOF/超长, status 0)视为瞬态可重试)
4. 将该 TcpStream 交给 tokio_tungstenite::client_async_tls_with_config(
        request, stream, None,
        Connector::Rustls(Arc<rustls::ClientConfig>)   ← wss 目标
        Connector::Plain)                              ← ws 目标
   ——与直连的 connect_async 不同点仅在 TLS 由我们自备:
   ClientConfig 显式 ring provider + webpki-roots(与 reqwest 侧 roots 对齐;
   显式 provider 避免多 CryptoProvider feature 合并时的隐式 default panic)。
   CONNECT 应答的残余字节:代理在客户端发出 WS 握手请求前没有任何合法的
   下行数据,故读到应答头结束即把 socket 交给握手是安全的(残余只可能来自
   不守规矩的代理,丢弃无害)。
```

- `WsConnection.stream` 类型不变(`WebSocketStream<MaybeTlsStream<TcpStream>>`,`client_async_tls_with_config` 返回同型),对上层零感知。
- `ws.rs` 头部文档的"**No proxy support**"段删除,替换为本节指针。
- 依赖变更:workspace 已有 tokio-tungstenite 0.24;`aimux-provider-utils` 需显式引入 `rustls` + `webpki-roots` + `base64`(Proxy-Authorization 编码;版本对齐 tokio-tungstenite 0.24 传递的 rustls 0.23 系)。

### 2.3 测试

- 本地假 CONNECT 代理(`TcpListener` 手写:校验 CONNECT 目标行 → 200 → 透传到真实本地 WS server):断言 CONNECT 目标、握手成功、事件往返。
- no_proxy 命中 → 断言未经过代理;`*` 通配;带端口条目。
- SOCKS scheme → 明确错误;代理回 407 → ApiCall 且不可重试、503 → 可重试;CONNECT 阶段 abort → `Aborted`;代理不通 → 超时归入 `first_chunk_ms` 语义。(P1 全部落地于 `ws_proxy_test.rs`;另钉住 CONNECT 请求行/Host 头/无凭据时无 Proxy-Authorization 的 wire 形状、IPv6 代理 host 去方括号、错误信息脱敏。wss 隧道的 rustls 分支已由握手级 smoke 闭环:经本地 CONNECT 代理连真实 `wss://api.elevenlabs.io`,真实服务器应答握手即证明隧道+证书链全链路执行(`#[ignore]` 手动跑,`live_wss_handshake_through_connect_proxy`);完整转写 round-trip 仍需 provider key,见 §3.5。)

### 2.4 范围外

- SOCKS 隧道、PAC/autoproxy、per-request 代理(与 HTTP 侧一致,代理是全局配置)。

## 3. Phase 2 — ElevenLabs `scribe_v2_realtime`

### 3.1 门控(对称于 OpenAI)

`elevenlabs.rs` 新增 `is_realtime_transcription_model_id`:`scribe_v2_realtime` 前缀 → `do_stream`;其余 → `do_generate`。**`do_generate` 对 realtime ID 返回 `UnsupportedFunctionality`**——现状是把 `scribe_v2_realtime` 当批处理模型发到 `/v1/speech-to-text`,错误来自服务端且信息误导。

### 3.2 协议序列

```
1. connect  wss://{base}/v1/speech-to-text/realtime
            ?model_id=scribe_v2_realtime
            &audio_format=pcm_{rate}          ← options.input_audio_format
            &commit_strategy=manual           ← 固定,不暴露(§3.4 D3)
            [&language_code=…                ← providerOptions.elevenlabs.languageCode
             &include_timestamps=true]        ← providerOptions.elevenlabs.includeTimestamps
            headers: xi-api-key(沿用 build_headers)
2. 等 session_started(配置回显)→ 发 StreamStart
3. loop select!(audio | ws.next() | abort | timeout):
   a. audio chunk → {"message_type":"input_audio_chunk",
                     "audio_base_64":…, "commit":false,
                     "sample_rate": rate}
      音频流结束(FFI input_done / 流 None)→ 不再发 chunk;
      直接进入收尾(§3.3)
   b. 事件映射:
      partial_transcript      → TranscriptPartial   ← 全量文本,替换语义
      committed_transcript    → TranscriptFinal     ← 可多次(多段)
      warning                 → 挂入 warnings / Raw(include_raw_chunks)
      error 类事件            → ApiCall;retryable 一行规则:
                                名称为 rate_limited / queue_overflow /
                                resource_exhausted → true,其余 false
```

**参数面(消融后)**:`providerOptions.elevenlabs` 仅 `languageCode` 与 `includeTimestamps` 两个命名参数;keyterms / secondary_languages / VAD 调优 / `previousText` 等均不做——无需求来源,待有人要再加(加 = 追加 query 参数,零结构改动)。

### 3.3 终止(与 OpenAI 的关键差异)

服务端**没有"完成"事件**。`commit_strategy` 固定为 manual(D3):

1. 音频流结束 → 末尾补发一条 `input_audio_chunk`(空音频或最后真实 chunk 带 `commit:true`);
2. 等待最后的 `committed_transcript` 到达 → 发 `Finish`(segments 由历次 committed 拼装)→ **客户端主动 close(1000)**;
3. 边界:commit 后若在 `chunk_ms` 窗口内无新事件,视为服务端静默,发 `Finish` 并 close(空 Finish 优于挂死——RFC-0028 的"终止保险丝"原则同样适用)。

流中途(音频未结束)收到的 `committed_transcript` 照常发 `TranscriptFinal`,不发 Finish——Finish 只属于收尾。

### 3.4 决策

- **`commit_strategy` 固定 manual,不作为选项暴露**:manual 模式下 `partial_transcript` 本来就持续流出,流式体验不损失;而 vad 模式需要另一条 Finish 语义分支。`vad` 模式待真实需求出现再加。

### 3.5 测试

本地 WS mock:`session_started` → `partial×2` → `committed` → (commit) → `committed` → close,逐字段断言 query 参数、base64、`sample_rate`、commit 时机、事件序列、close code 1000、abort 中途取消、retryable 分类规则(retryable 三名 / 其余 false,各抽一个错误事件断言)。**live smoke 一次**(几秒 PCM,真实 key)——RFC-0028 D4 的教训:OpenAI 当年没跑真 API,wire 形状只被 mock 验证过。

## 4. Phase 3 — Cartesia `ink-2`(**暂缓**,D6)

> 2026-09-13 复议后暂缓:官方文档在登录墙后、turns API 年轻易变、事件 schema 只能从 SDK 推断、无 key 可验证、无需求信号——三家里出错风险最高,而暂缓成本为零(现状对 ink-2 返回的 UnsupportedFunctionality 是诚实信息)。触发条件三选一即重启:有人提需求 / 文档公开出墙 / 拿到 key 决定跑 smoke。`cartesia.rs` 门控注释已标注 deferred。

门控已存在(`is_streaming_transcription_model_id`,L618),补 `do_stream`。结构与 §3 同型,差异点:

```
1. connect  wss://{base}/…                    ← 见 Open Question 1(路径锁定)
            headers: Authorization Bearer + Cartesia-Version(沿用 build_headers)
2. 发 config 文本帧 {type:"config", model, encoding, sample_rate,
                     [language]}             ← 参数面最小化,见下
3. loop select!:
   a. audio chunk → **Binary 帧**(不 base64、不 JSON)
      音频流结束 → 发 {"type":"close"}
   b. 事件映射(turn 事件,auto-finalize 模式):
      turn 中间态   → TranscriptPartial
      turn 完成     → TranscriptFinal(带词级时间戳时填充 segments)
      收尾完成      → Finish → 客户端 close
      error        → ApiCall
```

**只做 auto-finalize 模式**(turn detection 自动分段,实时转写的默认形态)。manual finalize 是 push-to-talk 场景,aimux 当前没有这类调用方,不做;turn 阈值(end_threshold / end_timeout_ms 等)只在调优 turn 行为时有意义,不透传,用服务端默认值。两者待需求出现再加。

`do_generate` 中对 `providerOptions.cartesia.streaming` 的 Unsupported warning:ink-2 走 `do_stream` 后该选项无意义,`do_stream` 侧消费/忽略并在文档注明,warning 保留在批处理路径不动。

### 4.1 测试

同 §3.5 模式:mock 断言 Binary 帧、config/finalize/close 命令序列、turn 事件映射;live smoke 一次。

## 5. 实施计划

| 阶段 | 内容 | 依赖 | PR |
|------|------|------|----|
| P1 | WS 代理隧道 + 测试 | 无 | ✅ #183(draft) |
| P2 | ElevenLabs realtime 门控 + `do_stream` + mock 测试 + live smoke | 无(建议在 P1 后,便于 smoke 走代理验证) | ✅ 实现+mock 测试落地(live smoke 待 key,见 §3.5 注) |
| P3 | Cartesia `do_stream` + mock 测试 + live smoke | 无(同上) | **暂缓**(2026-09-13 复议,触发条件见 §4;D6) |
| P4 | RFC-0028 文档更新:状态行加 follow-up 指针、§3.4"骨架同构,按需加"修正为"各家独立实现(本 RFC §1.1)"、§9.2/§9.4 关闭指向本 RFC、§9.5 挂 #167 | P1-P3 | 随 P3 或单独 docs PR |

P2/P3 不依赖 P1,但排序在其后:live smoke 顺手验证代理路径。

## 6. Non-goals

1. **不做统一 WS 转写会话抽象**(§1.1,P3 落地后复查)。
2. **不做 xAI STT**——无公开端点;RFC-0028 §1.1 的提法来自 AI SDK 生态转述,待 API 公开再立项。
3. **不做回调式 FFI**——拉取式是刻意设计(RFC-0028 §4.2);Node/Python 需要流式语法的在绑定层加 async-iterator 薄包装,零 wire 改动。
4. **不做 WS 会话录制**(RFC-0023 覆盖)——若 #167 的传输层回放落地,WS 会话录制搭同一机制,单独实现不划算(交叉引用 #167)。
5. **不做 WS 断线自动重连**——实时会话有状态(已发送的音频),静默重连会产生丢字或重复;断开即错误,由上层重建会话。
6. **不做 SOCKS/PAC 代理**(§2.4)。

## 7. 风险

| 风险 | 等级 | 对策 |
|---|---|---|
| Cartesia turns API 文档在登录墙后,事件 schema 以 SDK 源码推断 | 中 | Open Question 1:实现时以官方 Python SDK 类型定义逐字段对齐,mock 测试断言 schema;schema 不符时报回本 RFC |
| ElevenLabs 无显式结束事件,Finish 时机是推断的协议边界 | 中 | §3.3 定死:commit → 最后 committed → Finish + close;`chunk_ms` 静默兜底;live smoke 重点验证此边界 |
| no_proxy 自实现与 reqwest 语义有细节差 | 低 | reqwest 的匹配器非公开 API,无法程序化交叉验证——语义按其文档对齐并表格化单测;已知分歧(CIDR/IP 段条目仅按字面匹配,不展开网段)在 `ws.rs` docstring 与本表显式记录;非命中的安全方向是走代理 |
| rustls ClientConfig 与 reqwest 侧 roots 不一致 | 低 | 锁同一 webpki-roots 版本;隧道内 TLS 由 tokio-tungstenite 握手 |
| 两家 API 均为新/实验性,事件形状可能变 | 中 | 事件映射集中在各自文件一处(OpenAI 先例);版本变化只动映射 |

## 8. Open Questions

1. **Cartesia turns WS 的准确路径与事件 schema**。旧 ink-whisper 时代为 `wss://api.cartesia.ai/stt/ws`;现 SDK 指向 turns 端点(docs 路径 `api-reference/stt/turns/websocket`,登录墙)。P3 动手前用官方 SDK 源码锁定 URL 与响应类型,结论记回本节。
2. **WS connect 失败的重试**(P1 已验证并关闭):`stream_transcribe` 直通 `do_stream`,**没有任何 attempt 级重试**;且不重试是当前正确行为——音频输入流在首次尝试即被消费,重放需要可重播的音频源(HTTP 流可重试是因为请求体可克隆),非免费能力。连接失败的补救属于上层会话重建(Non-goal 5)。若未来要重试,需先设计可重播音频源,另立 RFC。

## 9. 决策记录

- **D1 不抽统一抽象**(§1.1):三家协议差异表为证;共享边界止于 `WsConnection`。
- **D2 非 http 代理 scheme 报错不直连**(§2.1.3):SOCKS 隧道不通、https 代理需 TLS 套 TLS(无需求来源);代理环境下静默直连 = 功能性错误(要么失败要么绕过网络边界),必须显式失败。
- **D3 ElevenLabs 固定 manual commit**(§3.4):manual 下 partial 事件持续流出,流式体验无损;vad 是第二条 Finish 语义分支,无需求不做。
- **D4 Cartesia 只做 auto-finalize**(§4):manual finalize 是 push-to-talk 场景,无调用方;turn 阈值不透传,用服务端默认。
- **D5 参数面最小化**(§3.2/§4):ElevenLabs 仅 languageCode/includeTimestamps,Cartesia 仅 language;每个额外参数都要映射+文档+测试,没有需求来源的一律不加,追加成本为零结构改动。
- **D6 Cartesia(P3)暂缓**(§4):与 P2 的决定性差异是可验证性——ElevenLabs 有公开 API 参考,mock 断言的是文档事实;Cartesia 文档在登录墙后、schema 靠 SDK 反推、API 新且易变,叠加 D4 教训(mock-only 验证的 wire 可能是错的),推断+无验证的组合风险不可接受。暂缓不损失任何东西:能力缺失的报错是诚实的,且无消费者。
