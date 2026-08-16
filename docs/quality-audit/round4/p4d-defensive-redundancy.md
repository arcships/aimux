# P4d:防御性代码冗余专项(FFI / core / provider 三层边界)

- 基线:master @ `cf2cea5`(`/tmp/aimux-audit-master`,只读)
- 对象:
  - FFI 边界:`aimux-ffi/src/lib.rs`(3767 行,生产 1-3100;`extern "C" fn` 108 个)+ `transcription_session.rs`(276 行)
  - 用户 API 层:`aimux-core/src/generate.rs`(994)、`options.rs`(196)、`message.rs`(106)、`content.rs`(241)、`tool.rs`(224)
  - Provider 层:`aimux-providers/src/{openai,anthropic,xai,google}/convert.rs` 及各 mod.rs 入参校验 + `aimux-provider-utils/src/{http,retry,response,url}.rs`
  - 补充证据:`bindings/{python,node}`(原生绑定绕过 FFI 直连 providers,是事实上的"第四层")
- 日期:2026-08-14(Round 4 子任务 4d)
- 方法:只读代码审查(Read/Grep),未运行任何 cargo 命令
- 上下文:workspace release profile `panic = "abort"`(上轮报告已确认)——release 下任何 panic 直接终止宿主进程;`invoke_stream_callback` 的 `catch_unwind`(H1 修复)只护回调,不护库内 panic

---

## 概述

本专项回答四个问题:**同一校验是否在多层重复执行**、**每类校验应归属哪一层**、**FFI 17 处生产 `expect` 是否可被外部触发**、**哪里防御多余/哪里缺失**。

总体结论:

1. **跨层重复是真实且系统性的,但集中在"JSON 线格式约定"一条链上**:FFI、Python 绑定、Node 绑定各自维护一份逐字等价的 prompt/options 解析器;"空串/`"null"` = 默认值"这一条不变量在 workspace 内有 **20 份拷贝**。除此之外,校验的层间分工大体清晰(FFI 管指针/句柄,serde 管结构,provider convert 管协议格式),未发现"同一语义校验三层都做"的实例。
2. **17 处生产 `expect` 无一可被绑定侧误用或外部数据直接触发**(逐条评估见 §4),但注册表 mutex 存在**中毒级联**结构风险:任何一次临界区内 panic(如 OOM)会让之后所有触碰句柄的 FFI 调用变成进程终止。另有 1 处 FFI 可达的 expect 藏在 `aimux-provider-utils/src/http.rs:177`(首次 HTTP 调用时构建 reqwest Client)。
3. **一处确凿的死防御 + 一处确凿的反向缺口同源**:`validate_base_url` 已写好 scheme/host 校验但**零生产调用者**;而活的路径上 FFI 只滤空串,非法 base_url 一路拖到 reqwest 才以晦涩的传输错误暴露。
4. **空 prompt 无任何校验**:`"[]"`/空消息数组可从 FFI 一路直通 provider HTTP(400 由远端返回)。

---

## 方法

- 用 Grep 全量列出 FFI 的 `expect/unwrap/panic`,对照 `#[cfg(test)]` 边界(lib.rs:3101)剔除非生产项,得生产 `expect` 恰 17 处(lib.rs 13 + transcription_session.rs 4);逐条追溯触发条件。
- 用不变量关键词(`"null"`、`trim_end_matches`、`iVBORw0KGgo`、`signature`、`abort_signal`、`is_empty`)横切 FFI / core / providers / bindings 四个目录,定位同一不变量的多处实现。
- 对每处重复,给出"三层各自的 file:line"与触发路径;对归属建议,以"谁能最早拿到类型化信息、谁拥有该语义"为判据。
- 冗余热点函数抽样:FFI 取 `stream_text_with_signal` / `parse_prompt` / 构造器族;core 取 `generate_text` / `generate_object` / `StreamTextResult::consume`;provider 取 anthropic `convert_system_message` / http `send`/`send_stream`。

---

## 1. 同一校验多层重复实例清单

### D1(H)JSON 线格式解析器三份逐字等价实现

同一不变量——"`{\"prompt\": <value>}` 包装解包 + 空串/`\"null\"` 选项 = 默认值"——在三个绑定入口各写一份:

| 层 | 位置 | 内容 |
|---|---|---|
| FFI | `aimux-ffi/src/lib.rs:263-281` | `parse_prompt`(Value→guard→`get("prompt").expect("checked by guard")`→`from_value(inner.clone())`)+ `parse_opts` |
| Python 绑定 | `bindings/python/src/lib.rs:1052-1077` | 同名 `parse_prompt`(含同一条 `expect("checked by guard")`,L1057)+ `parse_opts` |
| Node 绑定 | `bindings/node/src/lib.rs:1524-1544` | 同名 `parse_prompt`(同 expect,L1529)+ `parse_opts` |

- 证据:三处代码逐字等价(连 expect 消息都相同);Python/Node 绕过 FFI 直连 `aimux-providers`,被迫重写线格式。
- 风险:线格式演进(如 prompt 包装规则变化)需同步三处;三处已经出现漂移基础(python/node 的错误消息文案不同)。
- 建议:提取 `aimux-wire`(或在 aimux-core 加 `wire` 模块)承载 `parse_prompt`/`parse_opts`/空-NULL 约定,FFI 与原生绑定共用;`inner.clone()` 深拷贝同时消除(用 `#[serde(untagged)]` 包装枚举一次反序列化)。

### D2(H)"空串/`\"null\"` = 默认值"约定 20 份拷贝

同一不变量:NULL/空/空白/字面 `"null"` 的 JSON 参数 → 默认值。分布:

| 层 | 份数 | 位置 |
|---|---|---|
| FFI | 4 | `lib.rs:277`(parse_opts)、`lib.rs:568`(parse_provider_options)、`lib.rs:592`(normalize_config_json)、`lib.rs:1902-1905`(aimux_embed 内联第三次重写) |
| Python 绑定 | 9 | `bindings/python/src/lib.rs:632,699,1070`;`multimodal.rs:49,153,204,267,315,582` |
| Node 绑定 | 7 | `bindings/node/src/lib.rs:1542`;`multimodal.rs:46,190,253,347,404,741` |

- 证据:`grep '"null"'` 全 workspace 命中如上(Swift/Kotlin 走 FFI 不需自查——正是正确架构的反证)。
- 特别注意:FFI 内部自己就有 4 份——`normalize_config_json`(L583-595)就是为收敛该约定而生的函数,但 `parse_opts`、`parse_provider_options`、`aimux_embed` 没有改用它。
- 建议:FFI 内 4 份先收敛到 `normalize_config_json`;跨绑定收敛到 D1 的 wire 模块。

### D3(M)FFI prompt/opts 解析样板 6 份拷贝(FFI 层内)

同一段"get_model → cstr_to_string(prompt) → parse_prompt → fail_json → cstr_to_string(opts) → parse_opts → fail_json"的 20 行块逐字重复于:

- `lib.rs:1221-1240`(aimux_generate_text)
- `lib.rs:1260-1279`(aimux_generate_object)
- `lib.rs:1298-1317`(aimux_consume_stream_text)
- `lib.rs:1444-1463`(aimux_generate_text_as_openai)
- `lib.rs:1546-1571`(stream_text_as_openai_with_signal)
- `lib.rs:1666-1691`(stream_text_with_signal)

- 建议:提取 `fn parse_model_call_args(handle, prompt_json, opts_json, err) -> Option<(Arc<dyn LanguageModel>, ModelPrompt, GenerateTextOptions)>`;六处调用点各缩为一行。任何一处解析行为的修补(如错误码调整)目前要改六处。

### D4(M)base64 媒体类型嗅探 3 份(openai / xai 逐字重复,anthropic 同义异构)

同一不变量:"top-level 媒体类型(`image`/`image/*`)→ 从数据嗅探完整类型":

- `aimux-providers/src/openai/convert.rs:295-317`(`resolve_full_media_type`,base64 前缀 `iVBORw0KGgo`/`/9j/`/`R0lGOD`/`UklGR`)
- `aimux-providers/src/xai/convert.rs:226-247`(**逐字相同**的实现与魔数)
- `aimux-providers/src/anthropic/convert.rs:678-712`(`detect_media_type`,原始字节魔数表,多覆盖 bmp/tiff/pdf)

- 附带:`resolve_provider_reference`(openai convert.rs:320-336 vs xai convert.rs:249-265)也是逐字重复。
- 注:xai 本就是 OpenAI 兼容协议,`convert.rs` 大量复用 `crate::openai::convert`(如 anthropic convert.rs:1687 反向调用 openai 的 `deep_merge_json`),这两组函数是上轮 M10(partitial)残留。
- 建议:下沉到 `aimux-provider-utils`(或 openai convert_common.rs)导出一份;xai 直接 re-use;anthropic 的字节版可保留但魔数表与语义注释应指向同一处。

### D5(M)URL trailing-slash 规范化 3 种风格 5+ 处

同一不变量:"拼接 endpoint 前去掉 base_url 尾部斜杠":

- `aimux-provider-utils/src/url.rs:4-16`(`without_trailing_slash`,被 openai `with_base_url` mod.rs:191 及 serper/replicate/searxng/linkup 使用)
- `aimux-providers/src/anthropic/mod.rs:216-225`(`normalize_base_url`,自己又叠了去 `/v1`)
- `aimux-providers/src/google/mod.rs:161`、`mistral/mod.rs:112`、`cohere/mod.rs:118`(endpoint 处内联 `trim_end_matches('/')`)

- 风险:同一 provider 的 `with_base_url` 入口(google/mistral/cohere 若有)与 endpoint 拼接处规范化时机不一致时,`base_url` 记录值与实际请求 URL 漂移。
- 建议:统一在 config 构造入口(`with_base_url`)调用 `without_trailing_slash`,endpoint 处不再内联;anthropic 的 `/v1` 剥离保留为 provider 特例。

### D6(M)abort 监控 6 个执行点,FFI 层与 http 层重复

同一不变量:"abort 信号触发 → 调用以 `AiMuxError::Aborted` 终止":

| 层 | 位置 | 阶段 |
|---|---|---|
| provider-utils/http | `http.rs:1421-1431`(send_request) | 建连 |
| provider-utils/http | `http.rs:1390`(sleep_or_abort) | 重试退避 |
| provider-utils/http | `http.rs:343-359`(send) | 2xx 后读 body |
| provider-utils/http | `http.rs:713-735 + 864`(TimeoutBodyStream) | 流式 body 每_chunk |
| FFI | `lib.rs:1593 + 1692`(opts.abort_signal 传入 core) | 信号下发 |
| FFI | `lib.rs:1597-1607/1696-1704` 与 `1612-1623/1710-1721`(本地 `tokio::select!`) | 建连 + 每 part |

- 证据:`stream_text_with_signal` 把 `abort_signal.clone()` 塞进 opts 下发(L1692),**同时**又在本地对 `stream_text(...)` 与每个 `stream.next()` 做 `select!`——而 `TimeoutBodyStream` 已保证"只要带 abort_signal 就包裹 body 阶段"(http.rs:713-723 注释明说)。
- 定性:http 层 4 点是同一职责(传输)的分阶段落实,合理;FFI 层的 2 组 select 与其重复,唯一收益是对不透传 abort 的非 HTTP 模型兜底。
- 建议:要么删除 FFI 本地 select(信任 trait 契约),要么保留但注释标明"兜底:非 HTTP 模型可能忽略 abort_signal",避免后来者以为是必要层。

### D7(L)reasoning signature 元数据键清单两处

- `aimux-core/src/generate.rs:728-740`(`extract_reasoning_signature` 读 `["anthropic","bedrock","amazonBedrock"]`)
- `aimux-providers/src/bedrock/model.rs:509-540`(写 `amazonBedrock` + `bedrock` 双键)

- 定性:一处写一处读,键名清单重复。新增 provider 签名(如新键)需同步两处,否则静默丢失 multi-turn 签名回显。
- 建议:键清单常量化进 `aimux-core`(如 `pub const SIGNATURE_METADATA_KEYS`),bedrock 写侧引用。

### D8(L)"空 base_url = 默认"双层执行

- FFI `lib.rs:538-540`(`parse_base_url`:`cstr_to_string(base_url).filter(|url| !url.is_empty())`)
- Provider `anthropic/mod.rs:217-219`(`normalize_base_url`:空串 → `ANTHROPIC_API_URL`)

- 经 FFI 路径时空串已在 FFI 被滤掉,provider 分支不可达;但原生 Rust 调用者直接构造 `AnthropicConfig` 可达。归属应留给 provider(见规则表),FFI 侧过滤是冗余但无害的第一道。同类:openai `with_base_url` 依赖默认值链。**不算缺陷,记为归属模糊实例。**

### D9(L)FFI 构造器样板:29 处 `parse_two_args` + 19 处 `if let Some(url) = parse_base_url(...) { config = config.with_base_url(url) }`

- 证据:`grep -c parse_two_args lib.rs` = 29;`grep -c 'with_base_url(url)' lib.rs` = 19。
- 建议:宏(如 `ffi_constructor!(openai, OpenAIProvider, OpenAIConfig)`)或泛型 helper;`aimux_trace_new` 与 `aimux_trace_new_audited`(lib.rs:2691-2703 vs 2710-2722,整函数只差一个 bool)同源。

### D10(L)core 层 generate/stream 脚手架重复

- `aimux-core/src/generate.rs:461-505`(generate_text 的 recording 2a + session 2b)与 `781-825`(stream_text)为同一段 40+ 行逐字重复(注释都相同)。
- 建议:提取 `fn prepare_call_context(model, &mut call_options)`;两入口共享。

### D11(L)http.rs 响应头脱敏闭包两份

- `http.rs:400-410`(send)与 `461-470`(send_stream)重复 `is_sensitive_key → "[REDACTED]"` 映射闭包。
- 建议:提取 `fn redacted_header_pairs(headers: &HashMap<String,String>) -> Vec<(String,String)>`。

### 明确否证:未发现"同一语义校验三层串联重复"的实例

对四类候选逐一排查后排除:

- **必填字段校验(query/documents/prompt 等非空)**:FFI 只做 `parse_json_arg` 反序列化(L544-553),非空约束由 serde 的字段类型/`ToolChoice` 手写反序列化(core tool.rs:183-224)兜住,provider 层不再重复——三层各司其职。
- **枚举合法性**:`ToolChoice`(core tool.rs:202-222)、`Role`(serde lowercase)、`ResponseFormat` 均只在一层校验;provider convert 对未知值走 warning/透传而非再校验。
- **API key 存在性**:仅 `load_api_key`(provider-utils/api_key.rs:10-29)一处;FFI 构造器不查(provider 惰性失败)。
- **句柄合法性**:仅 FFI registry 一层;core/provider 拿到的是 `Arc<dyn Trait>`,类型系统已消除句柄概念(正向设计)。

---

## 2. 校验归属规则表(可写入贡献指南)

| # | 校验类别 | 唯一归属层 | 现状 | 备注 |
|---|---|---|---|---|
| R1 | 指针非空 / C 字符串 UTF-8 / 生存期 | **FFI**(`cstr_to_string` 等) | ✅ 已如此 | 唯一允许 `unsafe` 解引用的层 |
| R2 | 句柄存在性 / 句柄类型匹配 | **FFI**(registry) | ✅ 已如此 | `get_model`/`get_handle` + `fail_invalid_handle` |
| R3 | JSON 语法 + "空/NULL=默认" + prompt 包装线格式 | **wire 层**(新;FFI 与原生绑定共用) | ❌ 20 份拷贝(D1/D2) | 建议新 `aimux-wire` crate 或 core::wire 模块 |
| R4 | 结构/枚举合法性(ToolChoice、ContentPart 形状) | **core 的 serde 反序列化边界** | ✅ 已如此 | tool.rs 手写 Deserialize 即范例 |
| R5 | 语义约束(prompt 非空、tool 名非空、schema 形状) | **core 用户 API 入口**(generate_text/stream_text) | ❌ 空 prompt 直通(见 G1) | 唯一能同时看到 Rust 调用者与反序列化结果的层 |
| R6 | provider_options 键语义 | **provider 层**(convert) | ✅ 已如此 | 未知键透传/忽略,不预校验 |
| R7 | 协议格式(media type 合法性、role 映射、body 字段) | **provider convert** | ✅ 已如此 | openai convert `convert_file_part_to_openai` 即范例 |
| R8 | base_url 格式(scheme/host/trailing slash) | **provider-utils**(config 构造入口) | ❌ 校验器死了,规范化散 5 处(见 D5/X1) | `validate_base_url` 接线到各 `with_base_url` |
| R9 | HTTP 传输(超时/abort/重试/退避) | **provider-utils http** | 🟡 FFI 重复 select(D6) | FFI 只下发信号,不自行监控 |
| R10 | panic 边界防护 | **FFI**(回调 catch_unwind + 库内禁 panic) | 🟡 仅回调有(H1) | 见 §4 |

---

## 3. FFI 17 处生产 `expect` 逐条评估

> 前提:release `panic=abort`,任何 panic 即杀宿主。评估维度:(a) 绑定侧误用能否触发;(b) 外部数据(API 响应/用户 JSON)能否触发;(c) 环境能否触发。

### 3.1 mutex 中毒类(13 处)——不可直接触发,但有级联结构风险

| # | 位置 | 守护对象 |
|---|---|---|
| E1 | lib.rs:109 | registry(intern_model) |
| E2 | lib.rs:118 | registry(intern_handle) |
| E3 | lib.rs:148 | registry(get_handle) |
| E4 | lib.rs:165 | registry(drop_handle) |
| E5 | lib.rs:170 | TRACE_STORES(drop_handle) |
| E6 | lib.rs:178 | registry(drop_abort_signal) |
| E7 | lib.rs:2313 | registry(aimux_transcription_session_drop) |
| E8 | lib.rs:2681 | TRACE_STORES(get_trace_store) |
| E9 | lib.rs:2700 | TRACE_STORES(aimux_trace_new) |
| E10 | lib.rs:2719 | TRACE_STORES(aimux_trace_new_audited) |
| E11 | transcription_session.rs:200 | audio_tx(push_audio) |
| E12 | transcription_session.rs:220 | audio_tx(input_done) |
| E13 | transcription_session.rs:257 | task(terminate) |

- **触发分析**:中毒需要"某线程在持锁时 panic"。审查全部临界区:registry 的 insert/get/remove/clone(Arc 计数)与 TRACE_STORES 的同类操作均为非 panic 操作(除 OOM);audio_tx 临界区是 `as_ref/clone/take`,同样无 panic 路径。**绑定侧误用(乱句柄、空指针、坏 JSON)与外部数据均在拿锁前已被 `fail_*` 拦截,无法触达这些 expect。**
- **结构性风险(M)**:一旦因 OOM 或未来代码演化在临界区内 panic 一次,`std::sync::Mutex` 永久中毒 → 之后**每一个**触碰句柄的 FFI 调用(expect E1-E13)都变成进程终止——单点故障放大为永久性 DoS。且 dev 构建下 panic=unwind,回调 panic 虽被 H1 的 `catch_unwind` 捕获,但若未来有人在持锁路径调用用户代码,中毒即被"合法"制造。
- **建议**:三张表改用 `parking_lot::Mutex`(不中毒, poisoned 语义消失)或统一包一个 `fn lock_registry() -> MutexGuard` 做 `lock().unwrap_or_else(|p| p.into_inner())`(对纯数据 HashMap,中毒后继续是安全的)。

### 3.2 运行时构建类(1 处)——环境可触发

- **E14** `lib.rs:191`:`Runtime::new().expect("aimux-ffi: failed to build tokio runtime")`。
  - 绑定/外部数据不可触发;但线程数受限的宿主(嵌入式、sandbox、ulimit)首个 FFI 调用即可能失败 → 进程终止,且无 err 通道可报。
  - 建议:降级为 lazy OnceLock + 返回 `AiMuxError::Other` 的初始化函数,或至少在文档标注宿主线程要求。定级 **M(环境相关)**。

### 3.3 逻辑守卫类(2 处)——真不可达

- **E15** `lib.rs:267`:`obj.get("prompt").expect("checked by guard")`——由 L266 的 `obj.len()==1 && contains_key("prompt")` 保证,不可达。可改成 `obj.get("prompt").unwrap_or(&Value::Null)` 消除 expect,或用模式匹配 `if let Some(inner)`。**注意同一 expect 被 python/node 复制(D1)**——修一处应连三处。
- **E16** `transcription_session.rs:160`:`err.take().expect("err only None after successful send")`——循环不变量:进入时 `err` 必为 `Some`(初始 `Some(e)`;重发路径 `send_err.into_inner().err()` 必还原 `Some(e)`,因为消息类型恒为 `Err`)。不可达;但它在**独立 tokio task** 内,即便触发也只是任务终止(channel 关闭,next_part 报正常结束),不直接跨 FFI。建议改 `while let Some(e) = err.take()` 结构消除。

### 3.4 消毒后不可达类(1 处)

- **E17** `lib.rs:303`:`CString::new(sanitized).expect("impossible: NUL-free string...")`——L300 已把 `\0` 全部替换为 U+FFFD,确不可达。注释已说明(M1 修复)。可保留;如追求零 expect 可用 `unwrap_or_default` 语义化失败。**L**。

### 3.5 关联发现:FFI 可达的第 18 处 expect 在 provider-utils

- `aimux-provider-utils/src/http.rs:177`:`b.build().expect("shared reqwest Client build failed")`——首次 HTTP 调用(含 FFI 的 `aimux_generate_text` 全链路)时构建共享 Client。TLS 后端初始化失败/系统 socket 受限的宿主 → 进程终止。与 E14 同属"环境可触发的库内 expect",建议同样走错误通道。**M**。

### 3.6 对照 H1(catch_unwind)模式的新路径评估

- RFC-0028 transcription 路径(RFC-0028 是 cf2cea5 最新合入)采用 **pull 模型无 C 回调**,故无需 catch_unwind——设计上回避了 H1 类问题,✅。
- `terminate()`(transcription_session.rs:251-275)经 `ffi_block_on` join:从回调内重入会命中 M7 守卫 → `joined.is_err()` → warn + detach,✅ 有防护。
- **缺口确认**:catch_unwind 只包裹宿主回调;库自身(provider stream、serde、convert)在 release panic=abort 下任何 panic 仍直接杀宿主。当前生产代码里请求路径的 unwrap/expect 经排查仅上述 E14/E15/E17 与 http.rs:177(以及 openai/convert.rs:216/248 两个 `#[doc(hidden)] #[deprecated]` panic 包装——已确认**无生产调用者**,仅 tests/ 引用,风险受控)。结论:**新增路径未遗漏 H1 式防护**。

---

## 4. 多余防御 / 冗余实现

### X1(M)死代码:`validate_base_url` 零生产调用者

- `aimux-provider-utils/src/url.rs:27-47`:完整的 scheme(http/https)+ host + trailing-slash 校验,`lib.rs:38` re-export——**grep 全 workspace 无任何生产调用**(仅测试)。
- 讽刺点:外部注册 provider 路径(`provider.rs` ExternalProviderEntry L179)文档声称 "must be a valid `http(s)://` URL",但运行时同样没有调用它做校验(由 reqwest 在请求期才失败)。
- 建议:接线到各 `with_base_url`(归属 R8),或删除。与 G2 合并处理。

### X2(L)`aimux_drop_handle` / `aimux_abort_signal_drop` 的 `handle != 0` 前置检查

- `lib.rs:1384-1388`、`1761-1765`:句柄从 1 起(L104 文档),0 永不存在于 registry;`remove(&0)`/`get(&0)` 本就是 no-op。检查冗余(但文档化了"0 = no-op"语义,保留成本为零)。

### X3(L)FFI 流回调序列化的双重保险死分支

- `lib.rs:1727-1733`(及 1629-1635 的 openai 变体):
  ```rust
  let json = serde_json::to_string(&part).unwrap_or_else(|_| "{}".to_string()); // StreamPart 序列化实际不会失败
  if let Ok(cstr) = CString::new(json) { ... }                                   // serde_json 输出不含内嵌 NUL,永真
  ```
  两个 fallback 均不可达,且一旦"可达"则**静默丢弃该 part**(宿主少收一个 chunk,无日志)。建议:要么删掉 fallback 并用 `into_cstring_raw`(已做 NUL 消毒),要么失败时 `tracing::warn`。

### X4(L)`parse_prompt` 的 `inner.clone()` 深拷贝(×3,随 D1)

- `lib.rs:271`、python L1061、node L~1533:先 `from_str` 成 `Value` 再整体 clone 再 `from_value`——大 prompt 双倍内存 + 双遍解析。`#[serde(untagged)] enum PromptWire { Wrapped { prompt: ModelPrompt }, Bare(ModelPrompt) }` 可一次完成。

### X5(L)anthropic `convert_system_message` 全量克隆后只取第一个

- `anthropic/convert.rs:707-720`:`filter_map(... text.clone() ...).next()`——对 system 消息的**每个** text part 做克隆配对,只消费第一个。改为先 `iter().find(...)` 再克隆。

### X6(L)core 结果装配的克隆链(热点抽样)

- `generate.rs:557-571`(generate_text):每个 ToolCall 内容 `clone()` 两遍(一遍进 top-level `tool_calls`,一遍进 `response_content_parts`);`632-641`:`finish_reason/usage/warnings` 再从 `result` clone——共 3 份。
- `generate.rs:702-712`(generate_object):7 个字段逐个 clone,而 `844-849`(stream_text)已示范**解构避免部分 move**的正确姿势——同一文件两种风格并存。
- `generate.rs:280+287`(consume):`reasoning_text_buf.clone()` 连续两次(可 `mem::take`)。
- 建议统一采用 stream_text 的解构模式;ToolCall 双写可用引用收集后一次 clone。

### X7(信息)`aimux_embed` 的空构造再覆写

- `lib.rs:1901-1915`:`EmbeddingCallOptions::new("")` 先造一个含空串的 Vec,随后 `opts.values = values` 覆写。无后果但语义误导(默认值看着像"空 provider")。

---

## 5. 反向缺口(该有防御而没有;仅列证据确凿项)

### G1(M)空 prompt 全链路无校验

- FFI:`aimux_generate_text` 接受 `"[]"`(`ModelPrompt::Messages(vec![])` 反序列化成功)与 `""`(Text 空串);
- core:`generate.rs:894-902` `split_prompt` 原样放行;`grep prompt.is_empty|messages.is_empty` 在 core 与三大 provider convert 均**零命中**;
- provider:openai convert 产出 `messages: []`、anthropic convert 产出 `messages: []`——由远端 API 返回 400,错误信息晦涩(且已经消耗一次 HTTP + 重试判断)。
- Vercel AI SDK 在入口抛 prompt 校验错误。建议:归属 R5,`generate_text`/`stream_text` 入口对"消息数为 0 或全部内容为空"返回 `AiMuxError::InvalidPrompt`。

### G2(M)base_url 活路径无格式校验(与 X1 同源)

- FFI `parse_base_url`(lib.rs:538-540)只滤空串;`"not-a-url"`、`"ftp://x"`、`"http://"` 均被 `with_base_url` 接受,拖到 reqwest 建连失败才报 `ApiCall`(消息不含原始入参)。
- 校验器 `validate_base_url` 已存在未接线(X1)。建议在 provider-utils 的 config 入口统一校验(R8)。

### G3(L)CAimuxError 复用未约定释放义务

- `fill_error`(lib.rs:362-373)每次失败都重新分配 `message`/`error_value` 并覆写字段;宿主把同一 `err` 结构体连续传给两次失败调用而不 free 时,前一块即泄漏。头文件注释只说"caller 释放",未说"复用前必须先释放"。建议在 aimux-error.h 与 lib.rs doc 注释中显式写明该契约(非代码缺陷,契约缺口)。

### 明确排查过、**不**列为缺口的项

- 句柄 use-after-free:`get_model/get_handle` 克隆 `Arc` 后立即放锁(L145-151),调用期句柄被并发 drop 亦安全——已是正确防御。
- 双重 `aimux_drop_handle`:第二次 `remove` 返回 None,no-op,安全。
- 回调重入:M7 线程局部守卫(lib.rs:195-236)覆盖所有 `ffi_block_on` 入口;sync 型 FFI 函数(drop_handle 等)在回调内调用无锁死风险(流式 async 块不持 registry 锁)。
- `aimux_router_new`/`aimux_moa_new` 的裸指针 slice:属 C 契约范畴(unsafe 契约已注释),非可加防御。

---

## 6. 统计

| 维度 | 数量 |
|---|---|
| 审计文件 | FFI 2(4043 行)+ core 5(1761 行)+ providers convert 4 + provider-utils 4 + bindings 4(佐证) |
| 跨层/跨文件重复实例 | **11 组确认**(D1-D11;H×2、M×5、L×4)+ 4 类候选被否证 |
| "空/NULL=默认"约定拷贝 | 20(FFI 4 + Python 9 + Node 7) |
| FFI 生产 expect | 17(mutex 13 / runtime 1 / 逻辑守卫 2 / 消毒后 1)+ FFI 可达关联 1(http.rs:177) |
| 可被绑定/外部数据触发的 expect | **0** |
| 环境可触发的 expect | 2(E14 lib.rs:191、http.rs:177) |
| 死防御 | 1 确凿(validate_base_url)+ 2 组不可达 fallback(X3)|
| 反向缺口 | 2 确凿(G1 空 prompt、G2 base_url)+ 1 契约级(G3) |
| 热点冗余 clone/拷贝 | 6 处(X3-X7) |

### 修复优先级建议

1. **H**:D1+D2+D4——wire 解析与媒体嗅探收敛(一次 PR 各消 3-20 份拷贝);
2. **M**:E-级联(锁中毒策略)、E14/http.rs:177(环境 expect 走错误通道)、G1(空 prompt)、G2+X1(validate_base_url 接线)、D3(FFI 六段样板提取);
3. **L**:D5-D11、X2-X7、E15/E16 消 expect、G3 契约注释。
