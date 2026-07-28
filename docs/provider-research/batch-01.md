# 第 1 批调研记录（14 个 provider）

> 调研日期：2026-07-28。证据裁决遵循 RFC-0006 §2.1（官方文档/SDK > 成熟实现 > 多来源一致 > 单一第三方）。
> inventory 元数据（tier/protocol/openai_compatible/confidence）仅作线索，下方"协议事实"均以官方文档或多来源核验为准。
> 环境变量名标注"（推断）"表示官方文档未明示该变量名，仅为建议约定。

---

### apertis — Apertis

- **canonical ID**：apertis
- **aliases**：Stima API（旧名）
- **provider_kind**：model_vendor
- **inventory 分层**：tier=L2 / protocol=openai / openai_compatible=true / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://apertis.ai/ ；https://api.stima.tech （litellm 标注的官方文档站）；https://docs.litellm.ai/docs/providers/apertis
- **核验来源**：官方站点 + LiteLLM provider 文档 + LlamaIndex provider 参考
- **证据强度**：中（官方站点存在但首页未直接渲染 API 细节；base URL/鉴权/参数由 LiteLLM 与 LlamaIndex 一致佐证，二者均指向同一官方域名 api.stima.tech）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.stima.tech/v1`
- **鉴权**：方式=Bearer API Key / 环境变量=`STIMA_API_KEY`（沿用旧品牌名 Stima，litellm 记载） / 是否必需=是
- **endpoint 公式**：`POST {base_url}/chat/completions`
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions 字段（model、messages、stream、temperature、top_p、max_tokens、frequency_penalty、presence_penalty、stop、tools、tool_choice）
- **响应结构要点**：标准 OpenAI Chat Completions 响应（未直接渲染，按兼容性推断；litellm 以标准 OpenAI 解析）
- **流式**：SSE（litellm 示例支持 stream=True，兼容 OpenAI 流式）
- **错误结构**：未知（官方未渲染；按 OpenAI 兼容推断）
- **特有行为**：品牌由 Stima 更名为 Apertis，但 API 域名仍为 api.stima.tech；自称聚合 430+ 模型的统一网关

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：base URL + Bearer + /chat/completions + 标准 OpenAI 参数，LiteLLM 与 LlamaIndex 均按 OpenAI 兼容接入
- **可复用模型 ID 样例**：inventory 未给样例；按 litellm 路由前缀 `apertis/<model>`，实际模型 ID 需从 api.stima.tech/v1/models 拉取
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 官方文档站 apertis.ai / api.stima.tech 未能直接渲染出 API 细节，base URL 与鉴权依赖第三方（litellm）一致佐证，建议实现前再访问 api.stima.tech/v1/models 实测确认。
- 品牌更名导致域名（stima.tech）与品牌名（apertis）不一致，注意 env var 命名沿用 STIMA_API_KEY。

#### 5. 优先级建议

- **优先级**：P2（后续）
- **理由**：OpenAI 兼容薄封装，实现成本低；但官方文档可访问性一般、模型样例缺失、第三方佐证为主，价值与紧迫度中等。

---

### aws_polly — AWS Polly

- **canonical ID**：aws_polly
- **aliases**：Amazon Polly
- **provider_kind**：model_vendor
- **inventory 分层**：tier=L3 / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：audio_speech（文本转语音 TTS）

#### 1. 官方协议证据

- **文档 URL**：https://docs.aws.amazon.com/polly/latest/dg/API_SynthesizeSpeech.html
- **核验来源**：官方 AWS API 文档
- **证据强度**：强（官方 API 参考完整给出请求/响应结构）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://polly.<region>.amazonaws.com`（AWS 区域化端点）
- **鉴权**：方式=AWS Signature V4（SigV4，需 Access Key ID + Secret Access Key + Region，可选 Session Token） / 环境变量=标准 AWS 凭证链（AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY / AWS_REGION 等） / 是否必需=是
- **endpoint 公式**：`POST /v1/speech`（SynthesizeSpeech）；另有 `POST /v1/voices`（DescribeVoices）、`POST /v1/lexicons` 等管理端点
- **协议类型**：原生（AWS 专属协议，与 OpenAI 无关）
- **请求结构要点**：JSON body，字段 Engine（standard|neural|long-form|generative）、Text、OutputFormat（mp3|ogg_opus|ogg_vorbis|pcm|mulaw|alaw|json）、VoiceId、LanguageCode、SampleRate、TextType（text|ssml）、LexiconNames、SpeechMarkTypes
- **响应结构要点**：HTTP 200，body 为二进制 AudioStream（音频字节流），响应头 Content-Type 反映 OutputFormat，x-amzn-RequestCharacters 计费字符数；非 JSON
- **流式**：无（同步返回完整音频流；长文本可用 StartSpeechSynthesisTask 异步任务）
- **错误结构**：AWS 标准错误 JSON（EngineNotSupportedException / InvalidSampleRateException / TextLengthExceededException 等，含 HTTP 状态码与 error code）
- **特有行为**：单文本上限 6000 字符（SynthesizeSpeech）/ 20 万字符（异步任务）；engine 与 voice 须匹配；返回二进制音频而非文本/JSON

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：模态专用
- **依据**：仅 TTS 单一模态，AWS 原生 SigV4 鉴权 + 二进制音频响应 + 区域化端点，与 OpenAI Chat/Embeddings 协议无任何结构重叠
- **可复用模型 ID 样例**：aws_polly/generative、aws_polly/long-form、aws_polly/neural、aws_polly/standard（对应 Engine 取值）
- **是否需扩展共享层**：否（独立模态实现，不进 OpenAI 共享层）

#### 4. 风险与限制

- 鉴权为 AWS SigV4，需引入 AWS 凭证签名实现，复杂度高于 Bearer Key 的薄封装。
- 响应为二进制音频流，aimux 需有 audio_speech 模态的返回处理能力。
- 区域选择、voice 枚举、SSML 支持等需额外建模。

#### 5. 优先级建议

- **优先级**：P2（后续）
- **理由**：明确的模态专用实现（L3），协议证据强；但属单一 TTS 模态且需 AWS 签名，应在 aimux 具备 audio_speech 模态管线后纳入，非近期优先。

---

### chutes — Chutes

- **canonical ID**：chutes
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=L2 / protocol=openai / openai_compatible=true / confidence=1.0
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://chutes.ai/llms.txt ；https://chutes.ai/docs/api-reference/overview
- **核验来源**：官方 llms.txt 索引 + 官方文档站
- **证据强度**：强（官方 llms.txt 明确给出 base URL、鉴权、OpenAI 兼容声明）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://llm.chutes.ai/v1`
- **鉴权**：方式=`Authorization: Bearer cpk_...`（API key 前缀 cpk_） / 环境变量=`CHUTES_API_KEY` / 是否必需=是（无 Bearer 走匿名限流，429）
- **endpoint 公式**：`POST {base_url}/chat/completions`；`GET {base_url}/models`（公开目录，无需 key）
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions；model 字段支持 `default`、`default:latency`/`default:throughput`、逗号分隔 failover 列表等特有路由写法
- **响应结构要点**：标准 OpenAI Chat Completions 响应
- **流式**：SSE（OpenAI 格式，官方声明支持 streaming/tool calling/JSON mode/structured outputs/vision）
- **错误结构**：未知（按 OpenAI 兼容推断；无 Bearer 触发 429）
- **特有行为**：双 host——`llm.chutes.ai/v1`（推理）与 `api.chutes.ai`（管理/计费/OAuth）；model 字段支持 failover/latency/throughput 路由；TEE（confidential_compute）模型可选；目录含 USD/TAO 双币种价格

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：base URL + Bearer + /chat/completions + 标准 OpenAI 请求响应与 SSE 流式，共享层可正确表达；failover/latency 路由写法属 model 字符串约定，不影响协议结构
- **可复用模型 ID 样例**：MiniMaxAI/MiniMax-M2.5-TEE、NousResearch/DeepHermes-3-Mistral-24B-Preview、Qwen/Qwen2.5-72B-Instruct
- **是否需扩展共享层**：否（如需暴露 failover/latency 路由可作为 model 字符串透传，不必改共享层）

#### 4. 风险与限制

- 推理 host（llm.chutes.ai）与管理 host（api.chutes.ai）分离，配置时勿混淆。
- 匿名请求走限流路径，鉴权必须使用 Bearer（X-API-Key 不被推理端支持）。

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：官方文档完备、OpenAI 兼容薄封装、模型规模 43+、多来源（litellm/mastra/tokenhub）一致，实现成本低、价值高。

---

### darkbloom — Darkbloom

- **canonical ID**：darkbloom
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=L2 / protocol=openai / openai_compatible=true / confidence=0.85
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://darkbloom.dev/（官方首页含 OpenAI 兼容示例代码）
- **核验来源**：官方站点
- **证据强度**：强（官方首页直接给出 base URL、鉴权方式、OpenAI SDK 示例与 SSE 声明）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.darkbloom.dev/v1`
- **鉴权**：方式=Bearer API Key / 环境变量=未知（官方未命名，建议 `DARKBLOOM_API_KEY`，推断） / 是否必需=是
- **endpoint 公式**：`POST {base_url}/chat/completions`
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions（官方示例使用 openai SDK，model/messages/stream）
- **响应结构要点**：标准 OpenAI Chat Completions（流式 delta.content）
- **流式**：SSE（官方明确"SSE in the OpenAI format"）
- **错误结构**：未知（按 OpenAI 兼容推断）
- **特有行为**：去中心化私有推理（Apple Silicon），端到端加密 + 硬件远程证明；Public Alpha 阶段；模型少（gemma-4-26b、gpt-oss-20b）

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方示例即 openai SDK 改 base_url，请求/响应/流式均为 OpenAI 形态
- **可复用模型 ID 样例**：gemma-4-26b、gpt-oss-20b（对应 inventory darkbloom/gemma-4-26b、darkbloom/gpt-oss-20b 去前缀）
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 仍处 Public Alpha，稳定性与 SLA 待观察；模型仅 2 个。
- 官方未给出 env var 名与完整 API 参考（仅首页示例），错误结构未公开。

#### 5. 优先级建议

- **优先级**：P2（后续）
- **理由**：协议明确为 OpenAI 兼容薄封装，但模型极少、Alpha 阶段、生态较新，近期价值有限。

---

### empower — Empower

- **canonical ID**：empower
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=L2 / protocol=openai / openai_compatible=true / confidence=0.65
- **能力**（本次调研覆盖）：chat（未能在官方层面确认）

#### 1. 官方协议证据

- **文档 URL**：https://empower.dev/（官方站点，本次抓取返回空内容/不可访问）；https://docs.litellm.ai/docs/providers/empower（第三方）
- **核验来源**：仅第三方（LiteLLM）+ HuggingFace 模型页 empower-dev/empower-functions-medium
- **证据强度**：弱（官方站点 empower.dev 无法获取有效 API 文档；唯一第三方 litellm 记载自相矛盾——模型表所需变量标注为 TOGETHERAI_API_KEY，暗示模型实际由 Together 承载）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：未知（官方未确认；litellm 未给出独立 base URL）
- **鉴权**：方式=未知 / 环境变量=litellm 记载 EMPOWER_API_KEY，但模型表又标 TOGETHERAI_API_KEY（矛盾） / 是否必需=未知
- **endpoint 公式**：未知
- **协议类型**：未知（inventory 标 openai_compatible=true 仅为自动推断，无官方证据）
- **请求结构要点**：未知
- **响应结构要点**：未知
- **流式**：未知
- **错误结构**：未知
- **特有行为**：Empower Functions 系列模型权重在 HuggingFace（empower-dev）公开，但似乎并非通过 empower.dev 自有 API 独立提供推理

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：待定
- **依据**：无法确认 empower.dev 是否存在可用的自有 OpenAI 兼容 API；litellm 集成指向 Together，疑似模型由 Together 托管或自有 API 已弃用
- **可复用模型 ID 样例**：无（inventory model_count=0）
- **是否需扩展共享层**：未知

#### 4. 风险与限制

- 官方站点不可访问/无有效文档，协议契约无法确认。
- 第三方证据自相矛盾，存在"已弃用"或"非独立 provider"风险。

#### 5. 优先级建议

- **优先级**：搁置
- **理由**：无官方公开 API 文档可核验，第三方证据矛盾且疑似依赖 Together；按 RFC-0006 §2.1 证据不足确认请求响应契约时不得臆造，搁置待官方信息。

---

### libertai — Libertai

- **canonical ID**：libertai
- **aliases**：LibertAI
- **provider_kind**：model_vendor
- **inventory 分层**：tier=L2 / protocol=openai / openai_compatible=true / confidence=0.85
- **能力**（本次调研覆盖）：chat、embedding（官方另提及 image generation/speech）

#### 1. 官方协议证据

- **文档 URL**：https://libertai.io/（官方首页含迁移示例）；https://docs.libertai.io（官方文档站）
- **核验来源**：官方站点
- **证据强度**：强（官方首页直接给出 base_url 迁移 diff 与 OpenAI 兼容声明）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.libertai.io/v1`
- **鉴权**：方式=Bearer API Key（console.libertai.io/api-keys 生成） / 环境变量=未知（官方未命名，建议 `LIBERTAI_API_KEY`，推断） / 是否必需=是
- **endpoint 公式**：`POST {base_url}/chat/completions`（OpenAI 兼容）；另有 embeddings 等
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions（官方："keep your SDK, prompts and stack. Point one base URL at us"）
- **响应结构要点**：标准 OpenAI Chat Completions（官方文档覆盖 streaming/tool use/vision）
- **流式**：SSE（官方 FAQ 明确支持 streaming）
- **错误结构**：未知（按 OpenAI 兼容推断）
- **特有行为**：去中心化（Aleph Cloud）、TEE 远程证明、无 KYC、无日志；chat+embedding+image+speech 多模态同端点；订阅/credits 双计费

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方迁移示例即"改 base_url 一行"，请求/响应/流式均为 OpenAI 形态
- **可复用模型 ID 样例**：libertai/bge-m3、libertai/deepseek-v4-flash、libertai/gemma-4-31b-it（去前缀：bge-m3、deepseek-v4-flash、gemma-4-31b-it；官方旗舰 GLM-5.2）
- **是否需扩展共享层**：否

#### 4. 风险与限制

- embedding/多模态同端点，需确认 embeddings 端点是否也走标准 OpenAI /v1/embeddings。
- 官方未命名 env var，需约定。

#### 5. 优先级建议

- **优先级**：P2（后续）
- **理由**：OpenAI 兼容薄封装、协议明确；但属小众去中心化平台、模型规模 12，近期优先级中等。

---

### meta — Meta

- **canonical ID**：meta
- **aliases**：Meta Model API
- **provider_kind**：model_vendor
- **inventory 分层**：tier=L2 / protocol=openai / openai_compatible=true / confidence=1.0
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://dev.meta.ai/docs/getting-started/overview/ ；https://developer.meta.com/ai/resources/blog/build-with-muse-spark/
- **核验来源**：官方 dev.meta.ai 文档 + 官方 developer.meta.com 博客（多来源一致）
- **证据强度**：强（官方文档代码片段直接给出 base_url 与 env var；官方博客明确"point a coding agent at api.meta.ai/v1"，多家媒体/平台佐证）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.meta.ai/v1`
- **鉴权**：方式=Bearer API Key / 环境变量=`MODEL_API_KEY`（官方文档代码所示；inventory 记 `META_MODEL_API_KEY`，建议 aimux 用后者避免与通用名冲突） / 是否必需=是
- **endpoint 公式**：`POST {base_url}/chat/completions`（OpenAI SDK 直连）
- **协议类型**：OpenAI 兼容（官方："drop-in compatible with the OpenAI SDK, the Anthropic SDK, and agent CLIs"）
- **请求结构要点**：标准 OpenAI Chat Completions（用 openai SDK 直接 base_url 指向 api.meta.ai/v1）
- **响应结构要点**：标准 OpenAI Chat Completions
- **流式**：SSE（OpenAI 兼容，支持多模态/工具/agent）
- **错误结构**：未知（按 OpenAI 兼容推断）
- **特有行为**：官方同时提供 OpenAI 与 Anthropic 兼容入口；旗舰模型 Muse Spark 1.1（muse-spark-1.1），Public Preview（美国开发者）；多模态推理/agent/工具/computer use

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方明确 OpenAI SDK drop-in，base URL + Bearer + /chat/completions + 标准结构
- **可复用模型 ID 样例**：muse-spark-1.1、meta/muse-spark-1.1
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 仍处 Public Preview，区域/配额限制（美国开发者）。
- 同时存在 Anthropic 兼容入口，若 aimux 也接 Anthropic 协议需注意区分。

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：Meta 官方 API、OpenAI 兼容薄封装、战略价值高、文档明确，应优先纳入。

---

### pinstripes — Pinstripes

- **canonical ID**：pinstripes
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=L2 / protocol=openai / openai_compatible=true / confidence=0.85
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://pinstripes.io/docs（官方 API Reference）
- **核验来源**：官方文档站
- **证据强度**：强（官方 API Reference 直接给出 base URL、鉴权 token 格式与 OpenAI 兼容声明）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.pinstripes.io/v1`
- **鉴权**：方式=`Authorization: Bearer sk-ps-...`（API key 前缀 sk-ps-） / 环境变量=未知（官方未命名，建议 `PINSTRIPES_API_KEY`，推断） / 是否必需=是
- **endpoint 公式**：`POST {base_url}/chat/completions`；`GET {base_url}/models`、`GET {base_url}/usage`
- **协议类型**：OpenAI 兼容（官方："pinstripes is OpenAI-compatible... no code changes needed if you are already using the OpenAI SDK"）
- **请求结构要点**：标准 OpenAI Chat Completions
- **响应结构要点**：标准 OpenAI Chat Completions
- **流式**：SSE（OpenAI 兼容，按 SDK 行为；官方未单独说明但兼容 SDK 流式）
- **错误结构**：未知（按 OpenAI 兼容推断）
- **特有行为**：账户/计费端点（/auth/*、/billing/*）用单独 Session token（Bearer JWT），与推理 API key 分离；Warp（量化共享）/Slices（专用容量）两种产品形态；量化透明（标注 weight quant/KV-cache precision）

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：base URL + Bearer + /chat/completions + 标准 OpenAI 结构，官方明确 OpenAI SDK 零改动
- **可复用模型 ID 样例**：ps/deepseek-v4-flash、ps/glm-4.5-air、ps/qwen3-coder-30b-a3b（对应 inventory pinstripes/ps/... 去外层前缀）
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 推理 API key（sk-ps-）与账户管理 Session token 分离，配置时勿混用。
- 官方未命名 env var，需约定。

#### 5. 优先级建议

- **优先级**：P2（后续）
- **理由**：OpenAI 兼容薄封装、协议明确；但模型规模 6、生态较新，近期优先级中等。

---

### poe — Poe

- **canonical ID**：poe
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=L2 / protocol=openai / openai_compatible=true / confidence=1.0
- **能力**（本次调研覆盖）：chat、image_generation（官方另支持 video/audio 生成 bot）

#### 1. 官方协议证据

- **文档 URL**：https://creator.poe.com/docs/external-applications/openai-compatible-api
- **核验来源**：官方 Poe Creator 文档
- **证据强度**：强（官方文档完整给出 base URL、鉴权、端点、字段支持矩阵与差异说明）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.poe.com/v1`
- **鉴权**：方式=`Authorization: Bearer $POE_API_KEY`（poe.com/api/keys 获取） / 环境变量=`POE_API_KEY` / 是否必需=是
- **endpoint 公式**：`POST {base_url}/chat/completions`（Chat Completions）；`POST {base_url}/responses`（OpenAI Responses API）
- **协议类型**：OpenAI 兼容（Chat Completions + Responses 双格式）
- **请求结构要点**：标准 OpenAI Chat Completions；model 用 Poe bot 名；支持 stream/stream_options/top_p/tools/tool_choice/parallel_tool_calls/stop/temperature(0-2)/max_tokens/max_completion_tokens/logprobs
- **响应结构要点**：标准 OpenAI Chat Completions 响应
- **流式**：SSE（OpenAI 格式，stream 完全支持）
- **错误结构**：厂商专属要点——多数不支持字段被静默忽略而非报错；私有 bot 不可用
- **特有行为**：n 必须为 1；Chat Completions 不支持 response_format json_schema 结构化输出（Responses API 才支持 text.format json_schema）；strict 工具参数被忽略；音频输入被忽略；媒体 bot 建议 stream=False；自定义 bot 参数经 extra_body 传递（如 aspect）；仅支持 public bot

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：base URL + Bearer + /chat/completions + 标准 OpenAI 请求响应与 SSE，共享层可正确表达；差异多为"字段被忽略/限制"而非结构改写
- **可复用模型 ID 样例**：anthropic/claude-haiku-3.5、anthropic/claude-opus-4.1（Poe bot 名，非标准 provider/model 格式，需按 Poe bot 名透传）
- **是否需扩展共享层**：否（若要利用 Responses API 的高级能力需另接，但基础 Chat Completions 薄封装即可）

#### 4. 风险与限制

- model 字段为 Poe bot 名，与一般 provider/model 命名不同，模型映射需注意。
- 部分字段静默忽略，调试时行为可能与标准 OpenAI 不一致。
- 结构化输出/strict 工具在 Chat Completions 下不可用。

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：官方文档完备、OpenAI 兼容薄封装、聚合 137+ 模型（含多家前沿模型）、单 key 多模型价值高。

---

### publicai — Publicai

- **canonical ID**：publicai
- **aliases**：Public AI Inference Utility
- **provider_kind**：model_vendor
- **inventory 分层**：tier=L2 / protocol=openai / openai_compatible=true / confidence=0.85
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://platform.publicai.co/（官方平台，本次未渲染出文档细节）；https://docs.litellm.ai/docs/providers/publicai ；https://huggingface.co/docs/inference-providers/en/providers/publicai
- **核验来源**：官方平台域名 + LiteLLM provider 文档 + HuggingFace Inference Providers 文档
- **证据强度**：中（官方平台 platform.publicai.co 存在但未直接渲染 API 细节；base URL/鉴权/端点由 LiteLLM 明确记载，HuggingFace 文档佐证 PublicAI 为真实公益推理服务且模型经其提供）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://platform.publicai.co/v1`（litellm 默认 `https://platform.publicai.co/`，可经 PUBLICAI_API_BASE 覆盖为 `/v1`）
- **鉴权**：方式=Bearer API Key / 环境变量=`PUBLICAI_API_KEY`（litellm 记载） / 是否必需=是
- **endpoint 公式**：`POST {base_url}/chat/completions`
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions；model 经 litellm 路由前缀 `publicai/<hf-model-id>`（如 publicai/swiss-ai/apertus-8b-instruct）
- **响应结构要点**：标准 OpenAI Chat Completions（litellm 以标准 OpenAI 解析）
- **流式**：SSE（litellm 支持 stream=True）
- **错误结构**：未知（按 OpenAI 兼容推断）
- **特有行为**：公益、开源、非营利（支持 Swiss AI / AI Singapore / AI Sweden / Barcelona Supercomputing Center 等公共模型）；亦可经 HuggingFace router（router.huggingface.co/v1）以 `<model>:publicai` 后缀访问（HF_TOKEN 鉴权）

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：base URL + Bearer + /chat/completions + 标准 OpenAI 结构（litellm 明确记载）
- **可复用模型 ID 样例**：publicai/BSC-LT/ALIA-40b-instruct_Q8_0、publicai/aisingapore/Gemma-SEA-LION-v4-27B-IT、publicai/allenai/Olmo-3-32B-Think（去前缀为 HF 模型 ID）
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 官方平台文档未能直接渲染，base URL/鉴权依赖 litellm（第三方）+ HF 文档佐证，建议实现前实测 platform.publicai.co/v1/models 确认。
- 亦可通过 HF router 访问，若 aimux 已实现 HuggingFace provider，存在覆盖重叠。

#### 5. 优先级建议

- **优先级**：P2（后续）
- **理由**：OpenAI 兼容薄封装、协议较明确；但官方文档可访问性一般、模型规模 9、公益属性商业优先级中等。

---

### ragflow — Ragflow

- **canonical ID**：ragflow
- **aliases**：RAGFlow
- **provider_kind**：model_vendor
- **inventory 分层**：tier=L2 / protocol=openai / openai_compatible=true / confidence=0.65
- **能力**（本次调研覆盖）：chat（经 RAG 会话/Agent）

#### 1. 官方协议证据

- **文档 URL**：https://ragflow.io/docs/http_api_reference ；https://docs.litellm.ai/docs/providers/ragflow
- **核验来源**：官方 ragflow.io 文档 + LiteLLM provider 文档
- **证据强度**：中（官方文档确认存在 OpenAI 兼容 API；litellm 详细给出路径结构与模型名格式）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：自托管实例根，如 `http://localhost:9380`（无统一托管域名；非 hosted 模型厂商）
- **鉴权**：方式=Bearer API Key / 环境变量=`RAGFLOW_API_KEY`（litellm 记载） / 是否必需=是
- **endpoint 公式**：`POST {base}/api/v1/chats_openai/{chat_id}/chat/completions`（会话）；`POST {base}/api/v1/agents_openai/{agent_id}/chat/completions`（Agent）
- **协议类型**：原生（OpenAI 兼容的请求/响应体，但路径结构嵌入 chat_id/agent_id，需先在 RAGFlow 创建会话/Agent 并取其 ID，属多步骤结构差异）
- **请求结构要点**：body 为标准 OpenAI Chat Completions；但 URL path 必须含预创建的 chat_id 或 agent_id；模型名格式 `ragflow/chat/{chat_id}/{model_name}` 或 `ragflow/agent/{agent_id}/{model_name}`
- **响应结构要点**：标准 OpenAI Chat Completions 响应（litellm 以标准解析）
- **流式**：SSE（litellm 支持 stream=True）
- **错误结构**：未知（litellm 仅记录模型格式/连接错误处理）
- **特有行为**：RAGFlow 本质是自托管 RAG 框架（infiniflow/ragflow），OpenAI 兼容端点绑定其 RAG 会话/Agent；api_base 可带或不带 /v1，litellm 自动归一到 /api/v1/...

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：原生
- **依据**：虽请求/响应体兼容 OpenAI，但 endpoint 公式含动态 chat_id/agent_id 且需先创建会话——鉴权/path/多步骤存在结构性差异，无法由 OpenAI 共享层薄封装直接表达
- **可复用模型 ID 样例**：无（inventory model_count=0；模型名嵌入 chat_id/agent_id，非固定可复用 ID）
- **是否需扩展共享层**：否（应作原生 provider 实现，path 模板与 model 名解析需定制）

#### 4. 风险与限制

- 自托管软件，无统一托管 base URL，aimux 需用户自填实例地址。
- 需先调用 RAGFlow 管理 API 创建会话/Agent 取 ID，多步骤流程与 aimux 单次 chat 调用模型不完全契合。
- 作为独立 chat provider 价值有限（核心是 RAG 而非模型供应）。

#### 5. 优先级建议

- **优先级**：搁置
- **理由**：自托管 RAG 框架（非托管模型厂商）、model_count=0、OpenAI 兼容路径需 chat_id/agent_id（结构性原生），作为独立 chat provider 价值低；协议已核验为原生，如确有需求再按原生路径实现。

---

### synthetic — Synthetic

- **canonical ID**：synthetic
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=L2 / protocol=openai / openai_compatible=true / confidence=1.0
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://dev.synthetic.new/ ；https://dev.synthetic.new/docs/api/getting-started ；https://docs.litellm.ai/docs/providers/synthetic
- **核验来源**：官方 dev.synthetic.new（搜索摘要确认 OpenAI 兼容声明）+ LiteLLM provider 文档
- **证据强度**：强（官方明确"OpenAI-compatible API"；litellm 给出 base URL/鉴权/参数矩阵）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.synthetic.new/openai/v1`（注意路径含 /openai 段；另有 Anthropic 兼容入口）
- **鉴权**：方式=Bearer API Key / 环境变量=`SYNTHETIC_API_KEY` / 是否必需=是
- **endpoint 公式**：`POST {base_url}/chat/completions`
- **协议类型**：OpenAI 兼容（官方同时提供 Anthropic 兼容 API）
- **请求结构要点**：标准 OpenAI Chat Completions（messages、model、stream、temperature、top_p、max_tokens、frequency_penalty、presence_penalty、stop）
- **响应结构要点**：标准 OpenAI Chat Completions
- **流式**：SSE（litellm 支持 stream=True）
- **错误结构**：未知（按 OpenAI 兼容推断）
- **特有行为**：隐私优先（US/EU 安全数据中心、14 天自动删除、不训练）；OpenAI 与 Anthropic 双兼容入口；模型多为 hf: 前缀的开源模型

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：base URL + Bearer + /chat/completions + 标准 OpenAI 参数与流式
- **可复用模型 ID 样例**：hf:MiniMaxAI/MiniMax-M2、hf:Qwen/Qwen2.5-Coder-32B-Instruct（inventory 样例）
- **是否需扩展共享层**：否（base URL 含 /openai 段为固定前缀，直接作为 base_url 配置即可）

#### 4. 风险与限制

- base URL 路径含 /openai/v1（非纯 /v1），配置 base_url 时需完整带入。
- 官方 dev.synthetic.new 为 Mintlify 站点，部分页面渲染受限，细节以 litellm 佐证。

#### 5. 优先级建议

- **优先级**：P2（后续）
- **理由**：OpenAI 兼容薄封装、协议明确、模型 38；但属隐私导向小众平台，近期优先级中等。

---

### tensormesh — Tensormesh

- **canonical ID**：tensormesh
- **aliases**：Tensormesh
- **provider_kind**：model_vendor
- **inventory 分层**：tier=L2 / protocol=openai / openai_compatible=true / confidence=0.85
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://docs.tensormesh.ai/quickstart ；https://docs.tensormesh.ai/introduction-tensormesh
- **核验来源**：官方文档站
- **证据强度**：强（官方 quickstart 完整给出 base URL、鉴权、端点与 cURL/Python/SDK 示例）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://serverless.tensormesh.ai`（inventory base_urls 为空，以官方为准；endpoint 含 /v1）
- **鉴权**：方式=`Authorization: Bearer YOUR_API_KEY` / 环境变量=未知（官方未命名，建议 `TENSORMESH_API_KEY`，推断） / 是否必需=是
- **endpoint 公式**：`POST https://serverless.tensormesh.ai/v1/chat/completions`
- **协议类型**：OpenAI 兼容（官方："The serverless API is OpenAI-compatible"）
- **请求结构要点**：标准 OpenAI Chat Completions（model、messages、max_tokens、temperature、top_p、top_k、presence_penalty、frequency_penalty）；含 top_k 额外参数
- **响应结构要点**：标准 OpenAI Chat Completions（官方 SDK 用 choices[0].message.content）
- **流式**：SSE（OpenAI 兼容，按 SDK 行为）
- **错误结构**：未知（按 OpenAI 兼容推断）
- **特有行为**：核心为 KV 缓存复用（缓存 token $0）；serverless + reserved 两种部署；支持 Claude Code / Codex CLI 接入；另提供自有 Python SDK

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：base URL + Bearer + /v1/chat/completions + 标准 OpenAI 结构，官方明示 OpenAI 兼容
- **可复用模型 ID 样例**：MiniMaxAI/MiniMax-M2.5、Qwen/Qwen3-Coder-480B-A35B-Instruct-FP8、deepseek-ai/DeepSeek-V4-Flash（对应 inventory tensormesh/... 去前缀）
- **是否需扩展共享层**：否（top_k 为常见额外采样参数，可透传或忽略，不影响协议）

#### 4. 风险与限制

- inventory base_urls 为空，须以官方 `https://serverless.tensormesh.ai` 为准。
- 官方未命名 env var，需约定。

#### 5. 优先级建议

- **优先级**：P2（后续）
- **理由**：OpenAI 兼容薄封装、官方文档完备；但模型规模 10、生态较新，近期优先级中等。

---

### wandb — Weights & Biases

- **canonical ID**：wandb
- **aliases**：Weights & Biases、W&B Inference
- **provider_kind**：model_vendor
- **inventory 分层**：tier=L2 / protocol=openai / openai_compatible=true / confidence=1.0
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://docs.wandb.ai/inference/api-reference/chat-completions ；https://docs.wandb.ai/inference
- **核验来源**：官方 W&B 文档
- **证据强度**：强（官方 API 参考完整给出 base URL、鉴权、请求/响应示例）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.inference.wandb.ai/v1`
- **鉴权**：方式=`Authorization: Bearer [W&B API Key]`（wandb.ai/settings 生成） / 环境变量=`WANDB_API_KEY` / 是否必需=是
- **endpoint 公式**：`POST {base_url}/chat/completions`
- **协议类型**：OpenAI 兼容（官方："This endpoint follows the OpenAI format"）
- **请求结构要点**：标准 OpenAI Chat Completions；可选 team/project 用于用量追踪（Python 经 `project=` 参数，cURL 经 `OpenAI-Project: [TEAM]/[PROJECT]` 头）
- **响应结构要点**：标准 OpenAI Chat Completions（官方给出 id/object/choices/usage 完整示例）
- **流式**：SSE（OpenAI 兼容，按 SDK 行为）
- **错误结构**：未知（按 OpenAI 兼容推断）
- **特有行为**：可选 `OpenAI-Project` 头携带 team/project 做用量归集（非协议必需）；底层为 CoreWeave 算力；模型为开源权重大模型

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：base URL + Bearer + /chat/completions + 标准 OpenAI 请求响应；team/project 头为可选用量追踪，不影响协议正确性
- **可复用模型 ID 样例**：MiniMaxAI/MiniMax-M2.5、Qwen/Qwen3-235B-A22B-Instruct-2507、JetBrains/Mellum2-12B-A2.5B-Instruct
- **是否需扩展共享层**：否（如需暴露 team/project 用量追踪，可作为可选 header 透传，不必改共享层）

#### 4. 风险与限制

- team/project 头为可选；若需用量按团队归集需额外传递。
- 复用通用 WANDB_API_KEY，注意该 key 亦用于 W&B 其他产品，权限范围较广。

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：官方文档完备、OpenAI 兼容薄封装、模型 48、trust_score 98（本批最高）、多来源一致，实现成本低、价值高。
