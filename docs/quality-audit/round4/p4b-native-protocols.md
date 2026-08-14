# P4b: 原生协议 Provider 转换层深审（Round 4）

> **审查对象**: `aimux-providers/src` 下原生协议目录 `openai/`、`google/`、`anthropic/`、`bedrock/`、`vertex/`、`azure/`、`mistral/`、`cohere/`、`xai/`、`voyage/`、`huggingface/`，及 `openai/convert_common.rs`（本仓库唯一的 convert_common，位于 openai 模块内，非 crate 顶层）与关联的 `open_responses.rs`、`provider.rs`。
>
> **基线**: master @ `cf2cea5`（只读审计，未运行 cargo）
>
> **日期**: 2026-08-14
>
> **上游信号**: providers 生产代码 unwrap 20（google/utils.rs 7、provider.rs 3、xai 3）、expect 6、panic! 2、unreachable! 1、.ok() 19、unwrap_or_default 84。上轮遗留 M10 partial（top_level_media_type 等小工具散落约 5 处）。

---

## 1. 概述

本轮对 11 个原生协议目录做转换层深审。**总体结论：上轮（2026-08-06 P1）的两大结构问题已显著收敛**——

- M10（模型能力转换重复）：`get_gpt_version` / `get_o_series_version` / `get_model_capabilities` / `SystemMessageMode` 已收敛到 `openai/convert_common.rs`（144 行，文档清晰，Chat 与 Responses 两个 converter 共享），~140 行逐字重复已消除。
- P1-5（400+ 行超大 `build_request_body`）：openai/convert.rs（现 107 行 + `apply_*` helper 族）、anthropic/convert.rs（103 行 + M11 拆分）、openai/responses/convert.rs（104 行 + `apply_responses_*` 族）均已完成分解，`build_request_body_with_warnings` 系列 fail-fast 返回 `Result`（H2 修复确认）。
- xAI 的两个 converter 通过 `crate::xai::convert` 的 pub 导出正确共享 helper——但代价是 xai/convert.rs 与 openai/convert.rs 之间存在**另一组逐字拷贝**（见 M-12）。

**但仍有两类问题突出**：(1) 小工具函数（media type / tool result / text join / ToolChoice 映射）的散落数量比上轮记录的"约 5 处"更多——`top_level_media_type` 家族实际 **9 处**、`resolve_full_media_type` **4 处**（3 种语义）、`tool_result_to_content` **3 处**、`join_text_parts` **3 处**；(2) Google 多轮工具调用回传的 `functionResponse.name` 用 `tool_call_id` 冒充函数名，非流式路径下会生成空 name，属正确性缺陷。另发现 Anthropic 流终止行为违反自身 stream contract。

---

## 2. 方法

- 精读：`openai/convert.rs`（1500 行）、`openai/model.rs` 流路径（420 行 SSE 循环）、`google/convert.rs`（1099）、`google/model.rs`（751）、`google/utils.rs`（588）、`anthropic/convert.rs`（1750）、`anthropic/stream.rs`（524）。
- 抽查：bedrock/convert.rs、mistral/convert.rs、cohere/convert.rs、vertex/model.rs、azure/model.rs + responses.rs、voyage/embedding+reranking、huggingface/responses.rs、open_responses.rs、provider.rs。
- 全量 grep：unwrap/expect/panic!/unreachable!/.ok()/unwrap_or_default（按调用点上下文定性）；用脚本扫描 >100 行函数；对 8 类重复 helper 家族做逐一定位与逐字比对。
- 参照上轮 `docs/quality-audit/p1-convert-structure-review.md` 与 `p1-google-unwrap-review.md` 验证修复状态。

---

## 3. 发现列表

### H（正确性 / panic 风险）

**H-1 google：`functionResponse.name` 用 `tool_call_id` 冒充函数名，非流式路径产生空 name**
- 位置：`aimux-providers/src/google/convert.rs:228-237`（`convert_tool_parts`），配合 `google/model.rs:619-623`
- 证据：`let name = tool_call_id.clone(); function_response.insert("name", ...)`——`ContentPart::ToolResult` 本身有可选 `tool_name` 字段（`aimux-core/src/content.rs:124-128`）但被 `..` 忽略。非流式路径 Gemini 返回的 `functionCall` 无 id 时 `tool_call_id` 默认 `""`（model.rs:619-623），回传 `name: ""`（API 必填字段）→ 400；流式路径合成 `call-N` id 后 name 与函数名不匹配。Gemini API 要求 `functionResponse.name` 为函数名。
- 代码注释自认："sufficient for round-tripping with our own test fixtures; production callers … should use ContentPart::ToolCall first"。
- 影响：Gemini 多轮 function calling（核心路径）回传结果即失败。
- 建议：`tool_name.clone().unwrap_or_else(|| tool_call_id.clone())`，并在响应侧尽量回填 `ToolResult.tool_name`；非流式路径空 id 时用 name 做 id（与流式 `call-N` 策略对齐需二选一，二者必须一致）。

**H-2 anthropic：流错误终止不发 `Finish` part，违反 stream contract**
- 位置：`aimux-providers/src/anthropic/stream.rs:481-495`（`StreamEvent::Error` → yield Error 后 `return`）、`:499-504`（SSE 传输 Err → 同样 `return`）
- 证据：`aimux-core/src/stream_part.rs:43` 定义 Finish 为 "Final chunk — carries usage, finish reason, and metadata"。openai/model.rs:836-854 与 google/model.rs:560-574 在错误后均先发 `StreamPart::Error` 再发 `Finish { unified: Error }`；anthropic 提前 return，Finish 缺失。
- 影响：Anthropic `overloaded_error` 等真实场景下，依赖 Finish 收尾（取 usage / 释放会话）的消费方（FFI/bindings）收到不完整的流；三大家族终止语义不一致。
- 建议：与 openai/google 对齐——Error 后置 `stream_errored = true` 继续走统一收尾，发 `Finish { unified: Error }`。

### M（显著可维护性 / 潜在风险）

**M-1 anthropic：`parse_stop_reason` 漏映射 `stop_sequence`（及 `refusal`）**
- 位置：`anthropic/convert.rs:1698-1709`
- 证据：仅映射 `end_turn/max_tokens/tool_use`；Anthropic 文档化的 `stop_sequence` 落入 `Other`（应→Stop），`refusal` 落入 `Other`（应→ContentFilter）。同仓库 `bedrock/convert.rs:617` 对同一家模型正确映射了 `"stop_sequence" => Stop`，证明属遗漏而非取舍。
- 影响：用户设置 stop_sequences 后，unified finish reason 误报 Other。
- 建议：补 `"stop_sequence" => Stop`、`"refusal" => ContentFilter`（可加 `pause_turn` 注释说明）。

**M-2 anthropic：SSE 事件 JSON 解析失败被静默吞掉**
- 位置：`anthropic/stream.rs:496` — `Ok(_) | Err(_) => {}`
- 证据：`Err(_)`（serde 解析失败）与未知事件同样无副作用地 continue；对比 `openai/model.rs:536-544`、`google/model.rs:273-296` 对解析失败发 `StreamPart::Error` 并终止。
- 建议：`Err(e)` 分支单独处理（Error part + 终止），未知 `Ok(_)` 保留忽略。

**M-3 google：mid-conversation system message 静默丢弃**
- 位置：`google/convert.rs:66-79` — `continue` + TODO
- 证据：TS SDK 对会话中段 system message 抛 `UnsupportedFunctionalityError`；此处 `continue` 静默吞掉（注释承认应改 `Result` 签名）。
- 建议：改 `Result` 或至少发 `Warning`（anthropic 的处理方式——inline system message + beta header——是更好的先例）。

**M-4 全家族：finish_reason 缺失时非流式→Other、流式→Stop 的系统性分裂**
- 位置：非流式 `openai/model.rs:390-395`、`google/model.rs:162-169`、`vertex/model.rs:193-200`、`anthropic/stream.rs:180-187`（全 →Other）；流式 `openai/model.rs:843`、`google/model.rs:567`、`anthropic/stream.rs:510`、`vertex/model.rs:423`（全 →Stop，伪造完成信号）
- 建议：统一语义（TS 语义为 unknown/stop 二选一后全家族一致），流式至少不应默认伪造 Stop。

**M-5 anthropic：工具参数 JSON 解析失败静默替换为 `{}`**
- 位置：`anthropic/stream.rs:443-445` — `serde_json::from_str(&accumulated_json).unwrap_or(json!({}))`
- 证据：openai/model.rs:750-751 同场景保底 `Value::String(args)` 保留原始文本；anthropic 直接丢数据且无 warning。
- 建议：与 openai 对齐（字符串保底）或至少打 warning。

**M-6 anthropic：FileBase64 无效 base64 静默变成空文件**
- 位置：`anthropic/convert.rs:386-388` — `decode(data).unwrap_or_default()`
- 证据：decode 失败 → 空 bytes 继续走 media-type 嗅探/上传；对比 openai 的 `image_file_to_bytes`（image.rs:326-339）对无效 base64 返回 `InvalidArgument`。
- 建议：decode 失败返回 `AiMuxError::InvalidArgument`。

**M-7 anthropic：usage 反序列化失败静默归零**
- 位置：`anthropic/usage.rs:80` — `serde_json::from_value(...).unwrap_or_default()`
- 证据：上游 usage 结构变化后所有响应 usage 全 0 且无任何信号（usage 是计费关键信号）。
- 建议：解析失败至少打 warning 或透传 raw。

**M-8 图片字节读取失败静默上传空文件**
- 位置：`openai/image.rs:400,405`、`bedrock/image.rs`（`image_file_to_bytes().unwrap_or_default()`）
- 证据：`image_file_to_bytes` 返回 `Result`（URL 文件/无效 base64 均 Err），调用点却 `unwrap_or_default()` → multipart 里塞 0 字节图片，错误在服务端才暴露。
- 建议：`?` 传播（该函数签名本就支持）。

**M-9 bedrock：`ToolChoice::None => unreachable!()`**
- 位置：`bedrock/convert.rs:501`
- 证据：`prepare_tools` 在 462-464 行对 `ToolChoice::None` 早退 return，当前确实不可达；但 `ToolChoice` 是用户可控 enum，用 `unreachable!()` 兜底意味着任何早退被重构掉即成用户可触发 panic（provider 转换层）。
- 建议：改为 `json!({})` / `Value::Null` 或 debug_assert + 安全值。

**M-10 azure：`expect` 依赖"构造时已校验"，但公开构造器绕过校验**
- 位置：`azure/model.rs:381-384`、`azure/responses.rs:82-88`
- 证据：`AzureProvider::new`（model.rs:207-216）校验 resource_name/base_url 并返回 `Result`；但 `AzureModel::new`（model.rs:359-362）与 `AzureResponsesModel` 构造器为 pub 且不校验，直接以 `AzureConfig::new()`（两者皆 None）构造后首次请求时 `endpoint()` panic。
- 建议：`AzureModel::new` 返回 `Result` 或在构造时校验。

**M-11 provider.rs：全局 RwLock 的毒化 unwrap**
- 位置：`provider.rs:234,243,365`（`overlays().read()/.write().unwrap()`）
- 证据：任一线程持锁 panic 后，锁中毒 → 之后所有 `register_provider` / `is_external_provider` / provider 解析连锁 panic。库代码中锁毒化传播是可避免的健壮性风险。
- 建议：`unwrap_or_else(|e| e.into_inner())`（overlay map 本身无不变量需要毒化保护）。

**M-12 小工具函数散落（M10 未竟部分，规模大于上轮记录）**
- 证据（grep 全量定位，均为逐字或一行逻辑重复）：
  - `top_level_media_type` / `get_top_level_media_type` **9 处**：fn 定义 `openai/convert.rs:288`、`anthropic/convert.rs:281`、`xai/convert.rs:223`、`huggingface/responses.rs:811`、`open_responses.rs:1318`；内联 `split('/').next().unwrap_or("")` `bedrock/convert.rs:249`、`cohere/convert.rs:160`、`mistral/convert.rs:228`、`xai/responses/convert.rs:358`
  - `resolve_full_media_type` **4 处 3 种语义**：`openai/convert.rs:295` 与 `xai/convert.rs:226` 逐字（base64 前缀嗅探、返回 String、未知 image 默认 png）；`anthropic/convert.rs:717`（魔数嗅探、返回 Result，语义更严格）；`huggingface/responses.rs:834`（base64 + 通配签名表）
  - `resolve_provider_reference` 3 处 2 种错误类型：`openai/convert.rs:320`、`xai/convert.rs:249` 逐字（`Result<String,String>`）；`anthropic/convert.rs:788` 变体（`AiMuxError`）
  - `tool_result_to_content` 3 处逐字：`openai/convert.rs:738`、`mistral/convert.rs:203`、`cohere/convert.rs:287`；`google/convert.rs:220-223` 内联
  - `join_text_parts` 3 处：`cohere/convert.rs:276`、`mistral/convert.rs:192`、`openai/responses/convert.rs:262`
  - `ToolChoice → JSON` 映射 **12 处**（openai×2、xai、xai-responses、mistral、hf、openai-responses 同构 json；google×2、bedrock、cohere、open_responses 各自变体）
  - finish_reason 字符串解析 8 处、usage 转换 8 处（结构同构：total/no_cache/cache_read/reasoning/text 计算）
- 建议：`top_level_media_type`、`tool_result_to_content`、`join_text_parts`、`resolve_provider_reference`、`deep_merge_json`（现居 openai/convert.rs:1469，被 anthropic 跨模块借用）上移到 `aimux-provider-utils` 新建 `convert.rs`；`resolve_full_media_type` 三种语义保留 anthropic 版（最严格）为共享实现。预估可消除 ~150 行重复。

**M-13 Chat-Completions / Gemini / Responses 三族 SSE 事件循环的多份手写拷贝**
- 证据（>100 行函数扫描，行数为函数体长度）：
  - Chat-Completions 族 4 份：`openai/model.rs:443`（420）、`xai/model.rs:278`（420）、`mistral/model.rs:325`（299）、`cohere/model.rs:254`（306）——同样的 first_event 预检、text_id/reasoning_id 状态、tool_calls 按 index 累积、stream_errored 收尾
  - Gemini 族 2 份：`google/model.rs:206`（377）与 `vertex/model.rs:234`（205）近逐字，**已漂移**——vertex 缺 code-execution / server tool / grounding sources / usage raw 快照；`extract_content_from_candidate` 也两份（google/model.rs:597、vertex/model.rs:444）
  - Responses 族 2 份：`openai/responses/responses_convert.rs:324`（610）与 `xai/responses/mod.rs:381`（610）
- 建议：xAI/mistral 已有 `OpenAICompatProfile` 机制（openai/mod.rs），SSE 循环本体应 profile 化复用（xai 的 JSON-error 预检可做成 profile 钩子）；google/vertex 抽出共享的 Gemini SSE 状态机（vertex 只注入差异）。

**M-14 mistral：未支持的内容变体序列化为字面 `null` 塞进 content 数组**
- 位置：`mistral/convert.rs:271-274` — `FileBase64 | FileUrl | FileReference | Reasoning => Value::Null`
- 证据：其他 provider 对未支持变体一律跳过；mistral 会产出 `content: [null, ...]` 的非法请求体（注释自认无测试覆盖）。
- 建议：改为跳过 + warning。

**M-15 google：`get_model_path` 死代码 + endpoint 构造内联重复**
- 位置：`google/utils.rs:15-21`（全仓库无调用点）；`google/model.rs:66-73` 与 `75-85` 两处内联重复同一 `models/` 前缀逻辑
- 建议：endpoint 函数改用 `get_model_path`。

### L（风格 / 惯用法）

**L-1 bedrock/convert.rs:1 — 文件头带 UTF-8 BOM**（`﻿//!`）。建议去 BOM。

**L-2 缩进错乱（rustfmt 未覆盖区域）**：`openai/model.rs:631-638,647-648,671-672,727-728`（yield 字段缩进塌陷）、`google/model.rs:368,394,411,520`、`google/convert.rs:631`。仓库有 rustfmt.toml，建议全量 `cargo fmt` 一次。

**L-3 中英文注释混用**：`openai/convert.rs:985-988,1014,1440-1441,1456-1457`、`openai/mod.rs:126-127`（apply_max_tokens 文档半中文）。若项目规范为英文注释应统一。

**L-4 过时注释**：`anthropic/convert.rs:786-787` — `resolve_anthropic_reference` 文档写 "Panics when no `anthropic` key is present"，实际返回 `Result`（已是 Err 分支）。

**L-5 非惯用写法杂项**：
- `google/convert.rs:668-671` — `if let Some(b) = schema.as_bool() { let _ = b; ... }` 应为 `schema.is_bool()`
- `anthropic/stream.rs:330-340,382-392` — 双重 `blocks.get_mut(&index)`（match 后再 get_mut 设 started），可用单次可变绑定
- `openai/convert.rs:53` — `let tool_warnings: Vec<ToolWarning> = Vec::new();` 冗余类型标注
- `bedrock/convert.rs:53-54` — `last()` 判断 + `last_mut().unwrap()` 应合并为 `if let Some(last) = blocks.last_mut()`
- `openai/convert.rs:879` — `std::string::String` 全限定（文件顶部已 `use String` 语义）；`:882` 函数内重复 `use std::collections::HashMap`
- `xai/convert.rs:481`、`xai/responses/convert.rs:592` — `is_some_and(...)` 后紧跟 `.unwrap()`，惯用 `if let Some(r) = options.reasoning.filter(...)`；`xai/responses/convert.rs:142` `non_empty.unwrap()` 依赖跨 match 的非局部不变量（安全但脆弱）

**L-6 google：`prepare_all_tools` Gemini-3 组合路径 Auto→VALIDATED vs `prepare_tools` Auto→AUTO（strict 时 VALIDATED）**
- 位置：`google/convert.rs:294-327` vs `493-516`。两处 functionCallingConfig 4 臂 match 重复且语义有差；若为 TS 对齐的有意差异应加注释，否则是漏掉 has_strict 判断。

**L-7 未知 image 类型静默改标为 png**：`openai/convert.rs:314`、`bedrock/convert.rs:396-404`（`_ => "png"`）。建议至少 warning（将 webp/avif 之外的真实类型误标 png 会被服务端拒）。

**L-8 anthropic/mod.rs:195 — `api_key: self.api_key.unwrap_or_default()`**：缺 key 得到空字符串 → 下游 401 难排查；建议构造期校验。

**L-9 deprecated panic 包装仍存**：`openai/convert.rs:209-249`（2 个）、`anthropic/convert.rs:245-278`（2 个，含 `panic!("{}", e)`）。已 `#[doc(hidden)]` + `#[deprecated]` 并注明测试专用，可接受；建议按 0.2.1 note 计划迁移 `tests/` 后删除。

**L-10 huggingface/responses.rs 单文件 1219 行**混合 config + model + convert + stream + tools；建议按其他家拆子模块。

---

## 4. 统计

| 维度 | 数值 |
|---|---|
| 审查目录 / 文件 | 11 目录 + open_responses.rs / provider.rs；精读 7 文件 ≈ 6,900 行 |
| 发现总数 | **31**（H 2 / M 15 / L 10 + L-5 合并杂项） |
| H | 2（google functionResponse.name；anthropic 流终止缺 Finish） |
| M | 15 |
| L | 14 条目（含杂项合并） |
| >100 行函数（本范围） | 58 个；>300 行 11 个（几乎全为 SSE 事件循环） |
| unwrap（生产，本范围 14 处） | 全部当前安全：google/utils.rs 7（serde 字符串序列化不失败 ×4；循环/`ensure_root` 不变量保护 ×3，依赖非局部调用顺序，建议 debug_assert 加固）、xai 3（前置条件守卫）、openai/image 1 + bedrock/image 1（调用点守卫）、bedrock/convert 1（last() 守卫）、provider.rs 3（锁中毒风险，M-11） |
| expect（本范围 7 处） | openai 2 + anthropic 2（deprecated 测试包装，L-9）、azure 2（可绕过校验，M-10）、bedrock/sigv4 1（HMAC 任意 key 长度，安全） |
| panic! / unreachable! | panic! 1 处在范围内（anthropic deprecated 包装）；unreachable! 1（bedrock，M-9，当前不可达） |
| unwrap_or_default（本范围 57 处） | ~48 处合理可选默认（usage/embedding 数组/header/可选 id）；关键信号吞噬 5 处（M-6/M-7/M-8 + anthropic mod api_key L-8 + google/utils finalize 良性） |
| .ok()（本范围 12 处） | 多为可选数值解析/serde 不失败序列化，良性；huggingface:936 base64 前缀解码失败→None 属合理降级 |
| 重复家族 | top_level_media_type 9、resolve_full_media_type 4（3 语义）、resolve_provider_reference 3、tool_result_to_content 3+1 内联、join_text_parts 3、ToolChoice 映射 12、finish_reason 解析 8、usage 转换 8；SSE 循环拷贝：Chat 族 4、Gemini 族 2（已漂移）、Responses 族 2 |
| 上轮遗留状态 | M10 模型能力收敛 ✔（convert_common.rs）；top_level_media_type 家族未收敛 ✘（5→9 处，因新增内联实现）；400 行级 build 函数拆分 ✔；openai 静默吞错（body:Null）✔ 已 fail-fast |

### 修复优先级建议
1. **P0**：H-1（google 工具回传）、H-2（anthropic Finish contract）——都是核心调用路径的正确性。
2. **P1**：M-1/M-2/M-5/M-6/M-8（吞错五连，改动小收益直接）、M-9/M-10（panic 面）。
3. **P2**：M-12（provider-utils convert 层，~150 行去重）、M-13（SSE 状态机复用，最大宗但工程量大）。
4. **P3**：M-3/M-4/M-7/M-11/M-14/M-15 与全部 L。
