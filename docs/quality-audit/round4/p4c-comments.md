# P4c 注释质量专项（Round 4，全新维度）

- 基线：`/tmp/aimux-audit-master`（master @ cf2cea5，只读）
- 范围：aimux-core / aimux-providers（含原生协议目录）/ aimux-ffi / aimux-provider-utils 生产代码
- 方法：只读（Read/Grep/python 文本分析），未运行 cargo

## 一、方法

1. **what/why/noise 抽样**：正则提取四 crate 全部 `//` 行注释（排除 `///`、`//!`、URL 中的 `//`、纯分隔线），共 2256 条；`random.seed(42)` 均匀抽样 60 条，人工读上下文（±4 行）分级，并用括号计数法判定是否位于 `#[cfg(test)]` 内。
   - 分级标准：**why** = 解释约束/意图/协议要求/TS SDK 对齐/设计取舍（信息不可从代码恢复）；**what** = 复述代码动作或纯结构标记（章节分隔、步骤编号）；**noise** = 过时/误导/与代码矛盾。
2. **过期引用验证**：从抽样注释 + 既有信号中提取符号名/文件路径/RFC 编号，逐一 grep 验证目标存在性与语义一致性，共验证 45 个引用（≥30 要求）。
3. **//! 模块漂移**：按要求检查 10 个模块，对比 //! 声明与实际导出/行为。
4. **pub API 文档**：脚本统计 aimux-core 347 个 pub item 的 /// 覆盖；人工评估 10 个重点 API。
5. **error.rs 变体语义**：抽 8 个变体，grep 全仓构造点对照文档声明。
6. **重复注释 54 组定性**：按信号清单逐组归类。

## 二、发现列表

严重度：**H** = 误导性注释/与代码矛盾；**M** = 过期/冗余显著；**L** = 风格。

### H 档（1 项）

**H1. bedrock 注释声称 `ReasoningDelta` 无 `provider_metadata` 字段——与定义直接矛盾**
- `aimux-providers/src/bedrock/model.rs:385-390`：「The signature cannot be represented on `StreamPart::ReasoningDelta` (no provider_metadata field), so it is intentionally not emitted.」
- 实际定义 `aimux-core/src/stream_part.rs:138-144`：`ReasoningDelta` **有** `provider_metadata: Option<ProviderMetadata>` 字段（带 doc「Provider-specific metadata (e.g. xAI `itemId`)」，标 `#[allow(unused)]`）。
- 影响：给出的"不能表示"理由是假的（真实情况是字段无人填充）；读者会错误推断 signature 无法透传，或据此做下游设计。应改为「字段存在但当前无填充方，Bedrock 侧选择不 emit」或直接填充。

### M 档（8 项）

**M1. generate.rs 模块文档漂移：4 步描述遗漏录制/会话/追踪三大行为**
- `aimux-core/src/generate.rs:1-8` //! 只列「转换 prompt → 构建 CallOptions → 调 do_generate/do_stream → 包装结果」。
- 实际 `generate.rs:461-501` 还做：recorder 快照绑定 + record_input/transport_closed（RFC-0023）、session grouping（RFC-0024，代码注释自称步骤 "2b"）、trace 上下文。模块文档完全未提，新读者无法从 //! 得知这些副作用。

**M2. openai/mod.rs 模块文档严重低于模块实际范围**
- `aimux-providers/src/openai/mod.rs:1-5` 仅 4 行「OpenAI-compatible provider」。
- 实际子模块（mod.rs:6-15）：convert、convert_common、embedding、files、image、model、responses、speech、transcription、types —— 6 种 modality 均未在 //! 提及；responses API 与 chat completions 的双端点差异也未说明。

**M3. google/mod.rs 模块文档只覆盖 LanguageModel**
- `aimux-providers/src/google/mod.rs:1-16` 详细描述 Gemini 语言模型 API 形态（已核实 v1beta、x-goog-api-key、SSE 均准确）。
- 实际子模块（mod.rs:15-22）还含 embedding、files、image、video，//! 只字未提。（事实无错，范围缺失，程度轻于 M2）

**M4. error.rs 三个变体的文档语义在生产代码零构造点**
- `NoSuchModel`（error.rs:125-135，doc 称「Registry-level "model id does not resolve"... Pre-HTTP」）：全仓 grep 仅 golden test 构造（`aimux-core/tests/error_value_golden_test.rs:107,153`），无任何 registry/catalogue 代码 raise 它。
- `InvalidPrompt`（error.rs:108-109）与 `Tool`（error.rs:102-103）：仅出现在 FFI 错误码映射（`aimux-ffi/src/lib.rs:348,350`）、http.rs 错误命名（`http.rs:1530,1532`）和测试——同样无人构造。
- 影响：错误分类学描述了不可能发生的 raise 语义；FFI/错误分类为此维护映射分支。要么补实现（registry 解析失败应 raise NoSuchModel），要么在文档标注「reserved, 当前无构造点」。

**M5. 17 份 thin-wrapper 模块文档复制同一段无代码依据的表述**
- `cybertron.rs:5-8` 起的「A placeholder API key is sent ... the shared `OpenAIProvider` requires a non-empty key string」在 17 个文件逐字重复（cybertron、local、onnx、openvino、localai、vllm、mistralrs、docker_model_runner、mlx、sglang、llamacpp、litellm_proxy、jlama、oobabooba、xinference、omlx、gaudi）。
- openai 模块中**不存在**空 key 校验（`OpenAIConfig::new` 接受任意 String，`from_env` 才经 `load_api_key` 报错）；"requires a non-empty key" 是对构造函数签名要求的不精确表述，×17 传播。若 `OpenAIProvider` 将来允许空 key，需同步改 17 处。

**M6. copy-paste 传播组对应真实代码重复，应下沉共享 helper**
- 「Poll for completion.」×8 文件（replicate.rs:443、prodia.rs:395、revai.rs:239、luma.rs:321、runwayml.rs:265、klingai.rs:309、google/video.rs:148、vertex/video.rs:181）——各自重写 poll 循环。
- 「Capture response headers.」×10 文件——全部是 `let response_headers = resp.headers;` 的复述。
- 「Parse provider options.」×4（cartesia.rs:697 等）、「Build content array.」×4（cohere/model.rs:157 等）、「Build provider metadata.」×3（google/files.rs:324）。
- 判定：这是代码重复的注释镜像；poll 循环 / header 捕获下沉到 provider-utils 共享 helper 后，注释自然收敛为 helper 文档。

**M7. 核心 API 文档有示例无语义：无 # Errors、无参数约束**
- `generate_text`（generate.rs:424-445）、`stream_text`（generate.rs:742-765）、`generate_object`（generate.rs:678-692）：均只有 `# Example`，未写会返回哪些错误（Aborted/Timeout/ApiCall 语义在 error.rs 里很精彩但入口不引用）、abort_signal/超时行为、空 prompt 或 options 约束。对比：`LanguageModel` trait 文档（language_model.rs:17-23）明确写了 StreamStart/Finish/Warning 契约——入口函数反而缺失同级信息。

**M8. AiMuxError 变体文档覆盖不均**
- 有 /// 的仅 3 个（TokenExpired、NoSuchModel、NoSuchProvider）+ ApiCallError 结构体（质量极高）；无 /// 的 9 个：JsonParse、InvalidResponseData、Tool、InvalidArgument、InvalidPrompt、UnsupportedFunctionality、Timeout、Aborted、Other。Timeout「不可重试的调用方预算」这类关键语义只写在 `is_retryable()` 的 doc 里，变体本身无文档。

### L 档（6 项）

**L1. 分隔线样式三种混用**：`// ────`（×192，最多）、`// ════`（replicate/prodia/fal/cartesia/elevenlabs）、`// ====`（json_repair.rs、anthropic/prepare_tools.rs）。建议 rustfmt 注释规范或统一一种。

**L2. 中英文注释混用**：core 的 replay.rs/recording.rs、provider-utils 的 http.rs 主体中文；aimux-providers 绝大多数英文；个别文件内部混排（recording.rs 中文章节标题 + 英文 doc）。对外开源项目建议定一种为主。

**L3. what 级注释占比 45%**（详见三档统计）——「Build content array.」「Check for mid-stream error.」类低信息注释大量存在。

**L4. 行注释复述紧邻 doc**：`bedrock/mod.rs:123`「// Check for bearer token first.」与上方 `///`（119-121 行「Checks for `AWS_BEARER_TOKEN_BEDROCK` first...」）内容重复，删行注释即可。

**L5. error.rs //! 仅一行**（`//! Error types for aimux-core.`），相对 338 行高信息模块（含分类学论文级 doc）头重脚轻。

**L6. RFC-0027 编号复用导致注释引用歧义**：rfc/ 下同时存在 `0027-list-models-coverage.md` 与 `0027-model-catalogue-and-list-api.md`；代码注释裸引「RFC-0027」（model_catalogue.rs:1,10,25、provider.rs:36）无法区分指向。另 ws.rs:137「(RFC §3.1)」省略编号，需靠模块上下文还原为 RFC-0028。

## 三、分项统计

### 3.1 what/why/noise 抽样（60 条，其中生产 49 / 测试 11）

| 分级 | 生产代码 (n=49) | 含测试 (n=60) | 典型样本 |
|---|---|---|---|
| why | 27（55%） | 34（57%） | http.rs:1386「Backoff must also be abortable...」；anthropic/convert.rs:208「Anthropic does not allow trailing whitespace...」；bedrock/model.rs:245（增量解码取舍）；google/utils.rs:206（空段防 panic） |
| what | 22（45%） | 26（43%） | google/video.rs:148「Poll for completion.」；cohere/reranking.rs:159「Capture response headers.」；openai/model.rs:328「Build content array.」；各类分隔线/步骤号 |
| noise | 0 | 0 | （随机样本中未出现；H1 的矛盾注释系定向验证发现，不在随机样本内） |

结论：**why 略过半**，质量重心在协议约束/TS 对齐说明；what 类主要是 provider 骨架代码的复述注释与分隔线。注释密度：2256 条 // ≈ 每 33 行 1 条（约 75k 行）。

### 3.2 过期/失效引用（验证 45 个，目标 ≥30）

- **RFC 编号 18 个**（0003/0007/0009/0012/0014-0028）：文件全部存在，无失效；RFC-0027 一号两文件（L6）。
- **符号/路径 25 个**：`init_session_store`(session.rs:322)、`init_session_infer`(:335)、`entry_or_init`(recording.rs:689)、`parse_path`/`emit_navigation_to`(google/utils.rs:384/271)、`first_chunk_ms`、`cached_tokens`/`prompt_tokens_details`(openai/types.rs:74/76)、`with_provider`(40 文件)、`gaudi` 模块、`provider_name.rs`、`replay_with_model`(replay.rs:944)、`rebuild_provider`(providers/replay.rs:36)、`init_proxy` bool 语义(http.rs:125)、`send()` 仅 2xx 返回 Ok、`HttpBody::Bytes` 携带 Content-Type(http.rs:1474)、`config_snapshot_from_config`(openai/mod.rs:95)、`parse_retry_after`/`get_retry_delay_ms`(retry.rs:130/168)、`ffi_block_on`(ffi lib.rs:217)、node/python 绕过 FFI（bindings/*/Cargo.toml 证实）、`panic="abort"`（workspace profile 证实）、30s 默认超时(http.rs:79)、408/409/429/5xx 可重试(http.rs:1343)、`parse_polly_error` 仅测试调用、THIN_WRAPPERS 21 项 ↔ 实际 wrapper 模块同步。
- **失效/矛盾清单**：① H1（ReasoningDelta 字段矛盾）；② M5（"requires a non-empty key" ×17 文件无对应校验）。除此之外**无一条死引用**——注释与代码同步性整体非常好。

### 3.3 //! 模块漂移（10 个模块）

| 模块 | 判定 |
|---|---|
| core/recording.rs | 准确且极详尽（脱敏清单逐条核实 ：400-414） |
| core/replay.rs | 准确（MVP 仅 OpenAI 格式、Unsupported 语义与 :892 一致） |
| core/session.rs | 准确（全部符号存在） |
| core/error.rs | 无错但仅 1 行（L5） |
| core/generate.rs | **漂移（M1）**：漏 RFC-0023/0024 行为 |
| providers/openai/mod.rs | **漂移（M2）**：6 个 modality 子模块未提 |
| providers/google/mod.rs | 轻度漂移（M3）：embedding/files/image/video 未提 |
| ffi/lib.rs | 准确且优秀（并发/所有权/重入护栏均有代码对应） |
| provider-utils/http.rs | 准确（三职责均有实现对应） |
| provider-utils/retry.rs | 准确（两个入口 + 两个纯函数 helper 均存在） |

### 3.4 pub API 文档（aimux-core）

- 覆盖率：347 个 pub item，307 有 ///（**88.5%**）；40 个无文档，集中于 openai_output.rs(11)、trace/store.rs(7)、recording.rs(6)、model_id.rs(3)、replay.rs(3)。
- 重点 API 抽查（10 个）：generate_text / stream_text / generate_object（示例有、语义无，M7）；AiMuxError + ApiCallError + status_code()/is_retryable()/retry_after_hint()（优秀，声明与实现逐条吻合）；LanguageModel trait（契约清晰）；MockReplayModel、init_recording、SessionStore、replay_with_model（合格）。**准确率高、深度不均**：错误类型文档是全仓最佳，入口函数文档是最短板。

### 3.5 error.rs 变体文档 vs raise 语义（8 个）

| 变体 | 文档声明 | 验证结果 |
|---|---|---|
| TokenExpired | 「唯一生产者是 codex 401 映射」「status_code() 恒 Some(401)」 | ✓（唯一构造点 codex.rs:270；error.rs:211 实现一致） |
| ApiCall | is_retryable 408/409/429/5xx + 传输层 true | ✓（http.rs:1343 一致） |
| Aborted | — | ✓（http.rs abort 路径 :351,:877,:894,:932） |
| Timeout | 「调用方预算，不重试」 | ✓（luma:372、runwayml:275、bfl:392、google/files:277 等轮询超时；is_retryable 不含 Timeout） |
| UnsupportedFunctionality | — | ✓（trait 默认实现 provider.rs:24,49、replay.rs:894） |
| JsonParse | From 映射 + 「有传输错误应保留」 | ✓（generate.rs:698、replay.rs 多处，无传输上下文场景） |
| NoSuchModel | 「Registry 级、Pre-HTTP」 | ✗ **无生产构造点**（M4） |
| InvalidPrompt / Tool | （无文档） | ✗ 无生产构造点（M4） |

### 3.6 重复注释 54 组定性

- **(a) 纯分隔线/章节头 ~20 组**（x192 ──、x22 ══、"── Model ──" x9、"── Config ──" x9 等）：风格选择，无语义问题；仅需统一字符（L1）。
- **(b) thin-wrapper 模板文档 5 组**（cybertron x16、vertex_ai_ai21 x10、huggingface x5、voyage x3、llamafile x3）：copy-paste 模板 + 每文件特化；抽查 ai21 vs qwen 除首尾行外逐字相同、特化正确、无事实漂移。属**应收敛的传播**（共享文档页 / `#[doc]` 宏），且携带 M5 的不精确表述。
- **(c) 代码重复镜像 ~8 组**（Poll for completion x8、Capture response headers x10、Parse provider options x4、Build content array x4、Build provider metadata x3 等）：**应改共享 helper**（M6），注释随 helper 文档化。
- **(d) 合理的协议差异/不变量说明 ~6 组**（"Anthropic does not support 'none' tool choice" x3、"RFC-0015 P0-3 keep raw provider usage" x9、"provider-executed → does NOT set has_tool_calls" x4、"send() returns Ok only for 2xx" x3）：why 注释在多使用点的必要重复，判定**合理保留**；个别（send 2xx）已在 http.rs 有权威文档，provider 侧可改为引用。

## 四、总评

注释体系健康度高于典型同规模项目：**零死引用（45 项验证仅 1 项矛盾 + 1 项不精确）**、why 导向明确、RFC/审计锚点（M2b/C4-6/R-1.7/P0-3 等均可在 docs/ 追溯）、error.rs 文档达分类学水准。主要债务集中在：入口函数与 openai/google 模块的文档深度、3 个"僵尸"错误变体、thin-wrapper ×17 的模板维护成本，以及 45% 的 what 级复述注释（多为可下沉 helper 的重复代码的镜像）。
