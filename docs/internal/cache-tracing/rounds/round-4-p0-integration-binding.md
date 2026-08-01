# Round 4:P0 集成改动方案 + 8 语言绑定影响评估(2026-08-01)

> 来源:设计 agent「设计戊」(P0 diff 方案)+ 本文件作者对 bindings/ 各语言解析行为的逐语言核实。
> 结论先行:**8 个语言绑定全部无需改动**——前提是 FFI 透传用现有 `Raw{raw_value}` StreamPart 变体,而非新增专用变体。

---

## 一、P0 改动清单(设计戊,已对照代码核验)

### P0-1 stream_text 透传 request_body/response_headers

**现状**
- [generate.rs:106-109](aimux-core/src/generate.rs#L106) `StreamTextResult` 仅 `{ stream }`,未派生 serde/ts_rs(纯用户面结构体)→ 加字段零序列化影响
- [generate.rs:255-257](aimux-core/src/generate.rs#L255) 唯一构造点,`result.request_body`/`response_headers` 被丢弃
- 解构点仅 3 处,全字段访问(加字段不破坏):FFI [lib.rs:471](aimux-ffi/src/lib.rs#L471)、`StreamTextResult::text()`([generate.rs:124](aimux-core/src/generate.rs#L124))、测试 2 处。无 pattern 解构,零编译破坏面

**diff 草案**
```rust
pub struct StreamTextResult {
    pub stream: Pin<Box<dyn Stream<Item = Result<StreamPart, AiMuxError>> + Send>>,
    pub request_body: Option<serde_json::Value>,              // 新增,与 GenerateResult 对称
    pub response_headers: Option<HashMap<String, String>>,    // 新增
}
// stream_text() 构造点:
Ok(StreamTextResult { stream: result.stream,
    request_body: result.request_body, response_headers: result.response_headers })
```

**FFI 透传(关键决策)**:FFI [lib.rs:432-494](aimux-ffi/src/lib.rs#L432) 逐 part 序列化。**不改 `StreamPart` 枚举、不加新变体、不加 ABI 回调**,而是在 on_part 流中**插入一个合成 part**:`{"Raw":{"raw_value":{"aimux_meta":{"request_body":…,"response_headers":…}}}}`,发射位置在 StreamStart 之后(不破坏"首 part=StreamStart"契约)。
- 各绑定已有 Raw 变体(Python `_SPRaw`、TS `Raw`、Swift `case "Raw"`、Dart `StreamPartRaw`、Go 任意单 key 宽容、Java/Kotlin/C 纯透传)→ 零绑定改动
- 体积注意:200KB request_body 序列化翻倍,FFI 侧无裁减选项,留 RFC 讨论(可加 `meta_cap_bytes`)

### P0-2 Anthropic 生产路径补 cache 字段(含两处 round-3 行号修正)

- 非流式 [stream.rs:180-192](aimux-providers/src/anthropic/stream.rs#L180):cache 全丢,且 **total 口径错误**(Anthropic `input_tokens` 不含 cache,应 = input+cache_read+cache_creation)
- 流式 cache 字段**在 message_start 268-276**(Anthropic 只在 message_start 上报 input/cache),**不在** 454-469;454-469 已正确填 output,不动
- 同模式副本 **vertex/anthropic_model.rs:224-234** 顺带修
- `convert_anthropic_usage`([usage.rs:75](aimux-providers/src/anthropic/usage.rs#L75))已有完整映射,全仓库零调用——复用转正
- 需给 `AnthropicUsage` 加 `Serialize` derive([types.rs:162](aimux-providers/src/anthropic/types.rs#L162),字段 snake_case 与 wire 一致)
- **⚠ 行为变更(RFC 必须明示)**:`Usage.input_tokens.total` 语义从"末断点后 token"改为"三字段和",对消费方是语义修正(对齐 SDK 惯例)

### P0-3 Usage.raw 填充(13 处构造点)

| 构造点 | 改动 |
|---|---|
| open_responses.rs:1258 extract_usage_from_value、huggingface/responses.rs:1143 | 入口已是 `&Value` → `raw: Some(usage.clone())` 零结构改动(覆盖 azure/hf/deepseek 等) |
| google/convert.rs:949、bedrock/convert.rs:638(已 derive Serialize) | 入口一行 `raw: Some(to_value(usage).ok()…)` |
| openai/model.rs:76、openai/responses/convert.rs:1004、cohere/model.rs:71、mistral/model.rs:80、xai/convert.rs:77、xai/responses/convert.rs:40(仅 Deserialize) | 每类型加 `Serialize` derive + 入口一行 |
| anthropic/stream.rs:180、269、vertex/anthropic_model.rs:224 | 随 P0-2 的 `conv.raw` 完成 |

`raw` 字段已带 `#[serde(default, skip_serializing_if = "Option::is_none")]` → FFI JSON additive 新键,ts_rs 类型不变,零绑定破坏。

### P0-4 时间戳

aimux-core 零 `std::time` 先例;provider 层有(google/files.rs:246、runwayml.rs:272)→ 无依赖顾虑。
**建议 wrapper 自己记,结果类型不加字段**:wrapper 覆盖完整调用边界(含重试);`GenerateResult` 被 FFI 整包序列化,加字段污染 13+ 构造点;`Instant` 不可 serde,反正转 `i64 ms`。TTFT 用 `Arc<AtomicU64>` 观察首 TextDelta,禁止把计时器移入 generator。

---

## 二、8 语言绑定影响评估(逐语言核实)

| 绑定 | StreamPart 解析方式 | 未知 tag 行为 | 合成 Raw part 影响 |
|---|---|---|---|
| **C**(bindings/c/example.c) | 透传 JSON 字符串回调 | — | ✅ 无需改 |
| **Kotlin**([Model.kt:199](bindings/kotlin/src/main/kotlin/aimux/Model.kt#L199)) | 透传 JSON 字符串 | — | ✅ 无需改 |
| **Java**([Model.java:165](bindings/java/src/main/java/io/aimux/Model.java#L165)) | 透传 JSON 字符串给 onPart Consumer | — | ✅ 无需改 |
| **Go**([types.go:242](bindings/go/types.go#L242) ParseStreamPart) | 任意单 key map → {Tag, Payload};0 或 >1 key 才报错 | **宽容,未知 tag 不报错** | ✅ 无需改 |
| **Node/TS**([index.ts:136](bindings/node/src/index.ts#L136)) | `JSON.parse(x) as StreamPart` 类型断言 | 运行时宽容;TS 类型已有 `Raw`(types/StreamPart.ts) | ✅ 无需改 |
| **Python**([wrapper.py:518](bindings/python/python/aimux/wrapper.py#L518)) | pydantic discriminated union(`_external_tag_before` 转换外 tag)+ **`_SPRaw` 已存在** | 未知 tag → ValidationError,但 **Raw 已支持** | ✅ 无需改(仅当新增专用变体时才需加模型) |
| **Swift**([Types.swift:1088](bindings/swift/Sources/Aimux/Types.swift#L1088)) | switch on tag,`case "Raw"` 已有;default 抛错 | Raw 已支持 | ✅ 无需改 |
| **Flutter/Dart**([types.dart:802](bindings/flutter/lib/types.dart#L802)) | `StreamPartRaw.fromJson` 已有;另有 [StreamPartUnknown] fallback | 未知 tag 兜底到 Unknown | ✅ 无需改 |

**必要条件(否则个别绑定要改)**
1. **用现有 `Raw{raw_value}` 变体**承载 meta,不新增专用变体(否则 Python 需加 pydantic 模型 + TS 类型 + Swift case + Dart 分派,4 个绑定要改)
2. 发射位置在 StreamStart 之后(测试断言"首 part=StreamStart"存在,如 test_cassette.py:33)
3. meta 内层结构对外是 opaque(消费方不解包即无契约)

---

## 三、残留决策(供 RFC)

- F1:OpenAI TTL_idle 统一 60min(规则表 §7 10min → 设计 §3 60min,"宁可高估"方向)
- F2:无 tokenizer 时字节代理 W 封顶 B(中),接入 tiktoken 后升 W
- F3:U 公式统一为块上界 `(j+1)·B`(或 gran=None 时 τ ≥ 1 block);tail_loss 为减项,与"保守上界"目标矛盾,建议取 0
- S4:FFI meta 发射形状/顺序如上定案;cap 字节数可配
- S5:Anthropic input_total 语义变更为 RFC 明示项
