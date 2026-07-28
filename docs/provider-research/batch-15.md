# 第 15 批调研记录（13 个 provider）

> 调研日期：2026-07-28。本批覆盖 inventory 中 13 个 `implemented_in_aimux=false` 的 provider。
> 证据裁决遵循 RFC-0006 §2.1：官方 API 文档/SDK > reference 成熟实现 > 多来源一致 > 单一第三方。
> inventory 的 tier/protocol/openai_compatible 字段仅作线索，下文结论均以官方文档实际核验为准。

---

### nearai — NEAR AI Cloud

- **canonical ID**：nearai
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat、embedding、rerank

#### 1. 官方协议证据

- **文档 URL**：https://docs.near.ai/cloud/quickstart ；https://docs.near.ai/cloud/guides/openai-compatibility
- **核验来源**：官方 API 文档（Quickstart + OpenAI 兼容指南，含 curl 请求与响应样例）
- **证据强度**：强（官方文档可直接确认请求响应结构）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://cloud-api.near.ai/v1`（网关）；亦支持直连模型端点 `{slug}.completions.near.ai`
- **鉴权**：方式=Bearer Token / 环境变量=`NEARAI_API_KEY` / 是否必需=是
- **endpoint 公式**：`POST /v1/chat/completions`（标准 OpenAI 路径）
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions 请求体（`model`、`messages`、`stream` 等）
- **响应结构要点**：标准 `chat.completion` 对象（`id`/`object`/`created`/`model`/`choices[].message`/`usage`）。推理模型额外返回 `message.reasoning_content` 与 `usage.reasoning_tokens`
- **流式**：SSE（OpenAI 兼容，官方提供 OpenAI Compatibility Guide 覆盖 streaming/async/Files API）
- **错误结构**：与 OpenAI 共享结构一致（官方未声明差异，按兼容处理）
- **特有行为**：推理模型（如 `zai-org/GLM-5.1-FP8`）带 `reasoning_content`；所有推理在 TEE 内执行；响应头含 `X-Request-Id`

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方明确“OpenAI-compatible API”，请求/响应/鉴权/流式均能由 OpenAI 共享层正确表达；`reasoning_content` 为加性字段，不破坏共享层
- **可复用模型 ID 样例**：`Qwen/Qwen3-30B-A3B-Instruct-2507`、`zai-org/GLM-5.1-FP8`、`Qwen/Qwen3-VL-30B-A3B-Instruct`
- **是否需扩展共享层**：否（embedding/rerank 如需支持另作模态专用评估）

#### 4. 风险与限制

- Beta 阶段，API 可能变动；embedding/rerank 端点结构未在 quickstart 中明确，需另行核验后才能纳入薄封装

#### 5. 优先级建议

- **优先级**：P1
- **理由**：OpenAI 兼容、实现成本低、模型数 37、TEE 隐私为差异化卖点

---

### oci — OCI

- **canonical ID**：oci
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.5
- **能力**（本次调研覆盖）：chat、embedding

#### 1. 官方协议证据

- **文档 URL**：https://docs.oracle.com/en-us/iaas/Content/generative-ai/openai-compatible-api.htm ；https://docs.oracle.com/en-us/iaas/Content/generative-ai/api-keys.htm
- **核验来源**：官方 API 文档（OCI Generative AI OpenAI-Compatible Endpoints）
- **证据强度**：强（官方文档确认 base URL、鉴权、endpoint 路径）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://inference.generativeai.${region}.oci.oraclecloud.com/openai/v1`（region 为 OCI 区域标识）
- **鉴权**：方式=OCI Generative AI API Key（Bearer）或 OCI IAM 签名 / 环境变量=无统一标准（建议 `OCI_API_KEY` 或 IAM 配置）/ 是否必需=是
- **endpoint 公式**：`/chat/completions`（无状态 chat）、`/responses`（主推 Responses API）、`/conversations`、`/files`、`/vector_stores`、`/containers`
- **协议类型**：OpenAI 兼容（另存在独立的原生 OCI 推理 API，用于 chat/embedding/rerank，走不同端点）
- **请求结构要点**：OpenAI Chat Completions / Responses 请求体
- **响应结构要点**：OpenAI 兼容响应
- **流式**：SSE（OpenAI 兼容）
- **错误结构**：与 OpenAI 共享结构一致（官方未声明差异）
- **特有行为**：鉴权非 OpenAI 凭证，而是 OCI API Key 或 OCI IAM； Responses API 为主推接口、支持 MCP 工具与 conversations 上下文；模型多为 Cohere Command 系列（如 `oci/cohere.command-a-03-2025`）

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装（OpenAI 兼容路径 + OCI API Key 鉴权）
- **依据**：官方文档明确请求/响应 OpenAI 兼容；API Key 路径可作为 Bearer 复用共享层；IAM 签名路径为替代项，按需另行支持
- **可复用模型 ID 样例**：`oci/cohere.command-a-03-2025`、`oci/cohere.command-a-reasoning`、`oci/cohere.command-a-vision`
- **是否需扩展共享层**：是（需支持 region 化 base URL 与 OCI 鉴权方式选择；IAM 签名若支持则需 AWS-SigV4 风格签名扩展）

#### 4. 风险与限制

- base URL 含 region 变量，需配置化；IAM 签名鉴权实现成本高；Responses API 为主推而 Chat Completions 为兼容遗留，长期可能弱化

#### 5. 优先级建议

- **优先级**：P1
- **理由**：大型企业平台、模型数 44、OpenAI 兼容路径实现可行；OCI 鉴权为唯一主要差异点

---

### palm — Palm

- **canonical ID**：palm
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat、completion

#### 1. 官方协议证据

- **文档 URL**：https://ai.google.dev/gemini-api/docs/deprecations ；https://developers.googleblog.com/palm-api-makersuite-moving-into-public-preview/ （Google 官方 PaLM/MakerSuite 公告与 Gemini 迁移说明）
- **核验来源**：官方公告 + 官方 Gemini 迁移/弃用文档（PaLM API 已由 Gemini API 取代）
- **证据强度**：中（官方确认 PaLM API 已被 Gemini 取代并弃用；具体 v1beta2 端点细节为多来源一致推断）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://generativelanguage.googleapis.com/v1beta2`（PaLM 2 Generative Language API，已被取代）
- **鉴权**：方式=API Key（`x-goog-api-key` 头或 `?key=` 查询参数）/ 环境变量=`PALM_API_KEY` 或 `GOOGLE_API_KEY` / 是否必需=是
- **endpoint 公式**：`POST /v1beta2/models/{model}:generateText`（文本）、`:generateMessage`（chat）、`:countTokens`
- **协议类型**：原生（Google Generative Language 协议，非 OpenAI）
- **请求结构要点**：`prompt`/`messages` 字段，非 OpenAI `messages[]` 结构
- **响应结构要点**：`candidates[]` 结构，非 OpenAI `choices[]`
- **流式**：未知（PaLM v1beta2 原生 stream 支持有限）
- **错误结构**：厂商专属（Google RPC 风格 error）
- **特有行为**：text-bison / chat-bison 等 PaLM 2 模型；已被 Gemini（`:generateContent`）取代，处于弃用/下线状态

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：原生（Google PaLM 协议）
- **依据**：请求/响应结构与 OpenAI 不兼容，需独立实现
- **可复用模型 ID 样例**：`palm/chat-bison`、`palm/text-bison`、`palm/text-bison-001`
- **是否需扩展共享层**：否（独立原生实现）

#### 4. 风险与限制

- PaLM API 已被 Gemini 取代并弃用，模型逐步下线；新建无价值，维护即负担

#### 5. 优先级建议

- **优先级**：搁置
- **理由**：已弃用并被 Gemini 取代，无实现价值；如确有 PaLM 需求应引导迁移至 Gemini/Vertex

---

### privatemode_ai — Privatemode AI

- **canonical ID**：privatemode_ai
- **aliases**：无
- **provider_kind**：local_runtime
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat、embedding

#### 1. 官方协议证据

- **文档 URL**：https://docs.privatemode.ai/api/overview ；https://docs.privatemode.ai/api/proxy-configuration/ ；https://docs.privatemode.ai/api/chat-completions/
- **核验来源**：官方 API 文档（含 OpenAI/Anthropic 兼容端点清单与 proxy 配置）
- **证据强度**：强（官方文档明确兼容 OpenAI 与 Anthropic 协议、base URL、鉴权）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：客户端侧 `http://localhost:8080/v1`（Privatemode proxy 默认监听端口）；后端实际为 `api.privatemode.ai:443`，不可直连
- **鉴权**：方式=Bearer API Key（可由 proxy `--apiKey` 注入或客户端透传 Authorization 头）/ 环境变量=`PRIVATEMODE_API_KEY`（官方示例亦用 `PRIVATE_MODE_API_KEY`）/ 是否必需=是
- **endpoint 公式**：OpenAI 兼容：`/chat/completions`、`/completions`、`/embeddings`、`/models`、`/audio/speech-to-text`（Anthropic 兼容：`/messages`）
- **协议类型**：OpenAI 兼容（同时提供 Anthropic 兼容）
- **请求结构要点**：标准 OpenAI Chat Completions / Embeddings 请求体
- **响应结构要点**：标准 OpenAI 响应
- **流式**：SSE（OpenAI 兼容）
- **错误结构**：与 OpenAI 共享结构一致（透传式代理）
- **特有行为**：必须先运行 Privatemode proxy 容器（Docker），proxy 负责端到端加密与远程证明（attestation）；支持 per-request `cache_salt` prompt 缓存控制

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：OpenAI 兼容端点请求/响应/流式均由共享层表达；base URL 指向本地 proxy 为配置项
- **可复用模型 ID 样例**：`gpt-oss-120b`、`gemma-3-27b`、`kimi-k2.6`、`qwen3-coder-30b-a3b`、`qwen3-embedding-4b`
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 运行时强依赖本地 proxy 容器（local_runtime），非纯远程 API；加密/证明链路增加排障复杂度；模型数仅 7

#### 5. 优先级建议

- **优先级**：P2
- **理由**：协议实现简单（OpenAI 兼容薄封装），但需本地 proxy 运维、模型少、受众偏隐私场景

---

### regolo_ai — Regolo AI

- **canonical ID**：regolo_ai
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat、embedding、image_generation、rerank

#### 1. 官方协议证据

- **文档 URL**：https://docs.regolo.ai/getting-started/first-api-call/ ；https://docs.api.regolo.ai （Swagger/OpenAPI）；https://docs.regolo.ai/models/families/completions/
- **核验来源**：官方 API 文档（含 curl 请求与响应样例）+ 官方 API Reference（Swagger）
- **证据强度**：强（官方文档可直接确认请求响应结构）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.regolo.ai/v1`
- **鉴权**：方式=Bearer Token / 环境变量=`REGOLO_API_KEY` / 是否必需=是
- **endpoint 公式**：`/chat/completions`、`/embeddings`、`/rerank`、`/images/generations`、`/audio/transcriptions`（均 `/v1` 下）
- **协议类型**：OpenAI 兼容（chat/embeddings）；rerank/image_generation/stt 为专用模态端点
- **请求结构要点**：标准 OpenAI Chat Completions 请求体（`model`、`messages`、`stream` 等）
- **响应结构要点**：标准 `chat.completion` 对象（`id`/`object`/`created`/`model`/`choices`/`usage`）
- **流式**：SSE（OpenAI 兼容，官方 streaming 文档）
- **错误结构**：与 OpenAI 共享结构一致（官方未声明差异）
- **特有行为**：支持 vision、thinking（reasoning）、response parameters；提供完整 Swagger API Reference

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装（chat/embeddings）+ 模态专用（rerank/image_generation/stt）
- **依据**：chat/embeddings 为标准 OpenAI 兼容；rerank 与图像生成需按专用模态端点单独适配
- **可复用模型 ID 样例**：`llama-3.3-70b-instruct`、`llama-3.1-8b-instruct`、`gpt-oss-120b`、`minimax-m2.5`
- **是否需扩展共享层**：否（chat/embeddings）；rerank/image-gen 需模态专用模块

#### 4. 风险与限制

- rerank/image_generation 端点请求结构需对照 Swagger 二次核验后才能确定模态专用细节

#### 5. 优先级建议

- **优先级**：P1
- **理由**：OpenAI 兼容、提供官方 Swagger、多模态、实现成本低

---

### sagemaker — Sagemaker

- **canonical ID**：sagemaker
- **aliases**：无
- **provider_kind**：cloud_platform
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.5
- **能力**（本次调研覆盖）：chat、completion

#### 1. 官方协议证据

- **文档 URL**：https://docs.aws.amazon.com/sagemaker/latest/dg/jumpstart-foundation-models.html ；https://aws.amazon.com/blogs/machine-learning/announcing-openai-compatible-api-support-for-amazon-sagemaker-ai-endpoints/ ；https://docs.litellm.ai/docs/providers/aws_sagemaker
- **核验来源**：官方 AWS 文档 + 官方 AWS 博客 + LiteLLM reference
- **证据强度**：中（官方文档确认 SageMaker runtime 调用模型与新增 OpenAI 兼容支持；具体每模型 payload schema 因部署容器而异）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://runtime.sagemaker.${region}.amazonaws.com`（SageMaker runtime，`InvokeEndpoint`）
- **鉴权**：方式=AWS SigV4 签名 / 环境变量=AWS 标准凭证链（`AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY`/`AWS_SESSION_TOKEN`/role）/ 是否必需=是
- **endpoint 公式**：`POST /endpoints/{endpoint_name}/invocations`（runtime）；新增 OpenAI 兼容模式走 endpoint 上的 `/v1/chat/completions` 路径（2026-05 起）
- **协议类型**：原生（SageMaker runtime，payload schema 随部署容器变化；JumpStart Llama-2 系列为 `{"inputs":..., "parameters":...}` HuggingFace TGI 风格，非 OpenAI）
- **请求结构要点**：原生路径请求体因模型族而异（如 `meta-textgeneration-llama-2-*` 用 `inputs`+`parameters`）；OpenAI 兼容路径为标准 Chat Completions
- **响应结构要点**：原生路径响应因模型族而异；OpenAI 兼容路径为标准 OpenAI 响应
- **流式**：原生路径为模型自定义（InvokeEndpointWithResponseStream）；OpenAI 兼容路径为 SSE
- **错误结构**：厂商专属（AWS 错误 + 模型容器错误叠加）
- **特有行为**：需先在 SageMaker 部署 endpoint；模型 ID 形如 `sagemaker/meta-textgeneration-llama-2-13b`；payload schema 与 inference spec 绑定

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：原生（SageMaker runtime，逐模型 schema）；若目标 endpoint 已启用 OpenAI 兼容模式可退化为薄封装
- **依据**：原生 `invoke_endpoint` 的请求/响应/鉴权/流式与 OpenAI 结构性不同，且按模型族分叉；OpenAI 兼容为新近可选能力
- **可复用模型 ID 样例**：`sagemaker/meta-textgeneration-llama-2-13b`、`sagemaker/meta-textgeneration-llama-2-70b`、`sagemaker/meta-textgeneration-llama-2-7b`
- **是否需扩展共享层**：是（需 AWS SigV4 签名、逐模型 payload 适配器）

#### 4. 风险与限制

- 实现复杂度高（AWS 鉴权 + 逐模型 schema + endpoint 部署依赖）；inventory 模型样例为遗留 Llama-2 JumpStart 格式；OpenAI 兼容支持范围与区域受限

#### 5. 优先级建议

- **优先级**：P2
- **理由**：大型平台但原生实现成本高、模型 schema 分叉严重；建议后续按 OpenAI 兼容 endpoint 优先，原生逐模型 schema 视需求再补

---

### snowflake — Snowflake

- **canonical ID**：snowflake
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat、embedding

#### 1. 官方协议证据

- **文档 URL**：https://docs.snowflake.com/en/user-guide/snowflake-cortex/cortex-rest-api
- **核验来源**：官方 API 文档（Cortex REST API，含 OpenAI/Anthropic 兼容说明与代码样例）
- **证据强度**：强（官方文档确认 endpoint、base URL、鉴权、OpenAI SDK 直接可用）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://<account-identifier>.snowflakecomputing.com/api/v2/cortex/v1`（account-identifier 为账户级动态值）
- **鉴权**：方式=Bearer Token（Snowflake Programmatic Access Token / JWT / OAuth）/ 环境变量=无统一标准（建议 `SNOWFLAKE_PAT`）/ 是否必需=是
- **endpoint 公式**：`/chat/completions`（OpenAI 兼容，全模型）、`/messages`（Anthropic 兼容，仅 Claude）
- **协议类型**：OpenAI 兼容（Chat Completions API）
- **请求结构要点**：标准 OpenAI Chat Completions 请求体，可用 OpenAI SDK 直连
- **响应结构要点**：标准 OpenAI 响应
- **流式**：SSE（OpenAI 兼容）
- **错误结构**：与 OpenAI 共享结构一致（官方未声明差异）
- **特有行为**：授权依赖 Snowflake 角色（`SNOWFLAKE.CORTEX_USER` 或 `SNOWFLAKE.CORTEX_REST_API_USER`）；推理在 Snowflake 边界内执行；模型含 claude/llama/mistral/deepseek/snowflake 等

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：Chat Completions 端点请求/响应/流式完全 OpenAI 兼容，官方示例用 OpenAI SDK 直连
- **可复用模型 ID 样例**：`claude-sonnet-4-5`、`claude-opus-5`、`mistral-large`（官方模型表）；inventory 样例 `snowflake/claude-3-5-sonnet` 等需去掉 `snowflake/` 前缀
- **是否需扩展共享层**：是（需支持账户级动态 base URL 与 PAT/JWT 鉴权配置）

#### 4. 风险与限制

- base URL 为账户级动态值；鉴权为 Snowflake PAT/JWT（非静态 API Key），需角色授权配置；inventory model_sample 带 `snowflake/` 前缀需映射

#### 5. 优先级建议

- **优先级**：P2
- **理由**：协议兼容实现简单，但账户级 base URL + PAT 鉴权增加配置复杂度，企业场景为主

---

### text_completion_codestral — Text Completion Codestral

- **canonical ID**：text_completion_codestral
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.5
- **能力**（本次调研覆盖）：completion（FIM 文本补全）

#### 1. 官方协议证据

- **文档 URL**：https://docs.mistral.ai/api/endpoint/fim （Mistral FIM Endpoints 官方 API Reference）
- **核验来源**：官方 API 文档（含请求体字段、curl、响应样例）
- **证据强度**：强（官方文档可直接确认 FIM 端点请求响应）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.mistral.ai/v1`（通用）；Codestral 专用 `https://codestral.mistral.ai/v1`
- **鉴权**：方式=Bearer Token / 环境变量=`MISTRAL_API_KEY` / 是否必需=是
- **endpoint 公式**：`POST /v1/fim/completions`
- **协议类型**：专用模态（FIM 文本补全，非 chat completions）
- **请求结构要点**：`model`、`prompt`（前缀）、`suffix`（后缀，可选）、`max_tokens`、`temperature`、`top_p`、`stop`、`stream`、`random_seed`、`prompt_cache_key`；无 `messages[]`
- **响应结构要点**：`object: "chat.completion"`，`choices[].message.content`、`finish_reason`、`usage`（prompt/completion/total tokens）
- **流式**：SSE（`stream=true`，`data: [DONE]` 终止）
- **错误结构**：厂商专属（Mistral 错误格式）
- **特有行为**：专为代码 fill-in-the-middle 设计；默认模型 `codestral-2404`/`codestral-latest`

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：模态专用（FIM 文本补全）
- **依据**：使用 `prompt`+`suffix` 而非 `messages`，属文本补全专用协议，无法由 chat completions 共享层表达
- **可复用模型 ID 样例**：`codestral-2405`、`codestral-latest`
- **是否需扩展共享层**：否（独立 FIM 文本补全模块；OpenAI `/v1/completions` 风格若有共享可复用部分）

#### 4. 风险与限制

- 与 Mistral chat completions（`inception`/`mistral` 等 provider）区分，本 id 仅覆盖 FIM 补全；codestral 专用域名与通用域名并存需确认

#### 5. 优先级建议

- **优先级**：P2
- **理由**：FIM 为代码补全专用能力、模型仅 2、受众窄；协议清晰可按需实现

---

### text_completion_inception — Text Completion Inception

- **canonical ID**：text_completion_inception
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.5
- **能力**（本次调研覆盖）：completion（FIM 文本补全）

#### 1. 官方协议证据

- **文档 URL**：https://docs.inceptionlabs.ai/get-started/models （官方，本次抓取被 Cloudflare 拦截，未取得正文）；https://docs.litellm.ai/docs/providers/inception （LiteLLM reference，引用官方 Inception Platform Documentation）；https://www.inceptionlabs.ai/blog/introducing-inception-api
- **核验来源**：官方文档（间接，被反爬拦截）+ reference 成熟实现（LiteLLM）+ 官方博客
- **证据强度**：中（base URL、端点、模型路由经 LiteLLM reference 与官方博客多来源一致；官方正文未能直接抓取）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.inceptionlabs.ai/v1`
- **鉴权**：方式=Bearer Token / 环境变量=`INCEPTION_API_KEY` / 是否必需=是
- **endpoint 公式**：FIM 补全 `POST /v1/fim/completions`（本 id 对应路径）；另提供 OpenAI 兼容 `POST /v1/chat/completions`（对应 `inception/` 路由，非本 id）
- **协议类型**：专用模态（FIM 文本补全）
- **请求结构要点**：`model`、`prompt`（前缀）、`suffix`（后缀，可选）、`max_tokens`（经 LiteLLM `text_completion` 透传）
- **响应结构要点**：`choices[].text`（文本补全风格）
- **流式**：SSE（OpenAI 兼容 stream，推断）
- **错误结构**：与 OpenAI 共享结构一致（推断）
- **特有行为**：Mercury 系列 diffusion LLM；`mercury-edit-2` 专为代码 FIM autocomplete

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：模态专用（FIM 文本补全）
- **依据**：FIM 用 `prompt`+`suffix` 文本补全协议，与 chat completions 不同；chat 路径归 `inception` provider
- **可复用模型 ID 样例**：`mercury-edit-2`
- **是否需扩展共享层**：否（独立 FIM 文本补全模块）

#### 4. 风险与限制

- 官方文档正文未能直接抓取（Cloudflare 拦截），端点细节依赖 LiteLLM reference 推断；模型仅 1；建议实现前复核官方 `/v1/fim/completions` 请求体

#### 5. 优先级建议

- **优先级**：P2
- **理由**：FIM 专用、模型单一、官方正文未取得；协议可参照 Mistral FIM 实现，证据补强后推进

---

### vertex_ai_language_models — Vertex AI Language Models

- **canonical ID**：vertex_ai_language_models
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat、embedding、image_generation、realtime

#### 1. 官方协议证据

- **文档 URL**：https://docs.cloud.google.com/gemini-enterprise-agent-platform/reference/models/inference （官方 Gemini generateContent 参考）；https://docs.cloud.google.com/gemini-enterprise-agent-platform/models/start
- **核验来源**：官方 API 文档（Gemini Enterprise Agent Platform，前 Vertex AI，含请求体/响应体结构）
- **证据强度**：强（官方文档确认 generateContent 请求响应结构）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://{region}-aiplatform.googleapis.com/v1`（区域化 aiplatform 端点）
- **鉴权**：方式=Google OAuth2 Bearer Token（服务账号 / ADC）/ 环境变量=`GOOGLE_APPLICATION_CREDENTIALS`（标准 GCP 凭证链）/ 是否必需=是
- **endpoint 公式**：`POST /v1/projects/{project}/locations/{location}/publishers/google/models/{model}:generateContent`、`:streamGenerateContent`、`:embedContent`；另有 OpenAI 兼容端点 `/v1beta1/projects/.../endpoints/openapi:generateContent`（兼容 Chat Completions，次选）
- **协议类型**：原生（Google Gemini 协议）
- **请求结构要点**：`contents[].role`（user/model）+ `parts[]`（text/inlineData/fileData）、`systemInstruction`、`tools`、`generationConfig`（maxOutputTokens/temperature/topP/topK/thinkingConfig 等）；非 OpenAI `messages[]`
- **响应结构要点**：`candidates[].content.parts[]`、`finishReason`、`usageMetadata`；非 OpenAI `choices[]`
- **流式**：SSE（`streamGenerateContent`，Google 流式格式）
- **错误结构**：厂商专属（Google RPC `error` 结构）
- **特有行为**：多模态（text/audio/video/image）、thinking/reasoning、function calling、realtime；模型含 gemini-2.0-flash、deep-research 等

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：原生（Google Gemini 协议）；如仅需 chat 可退用 OpenAI 兼容端点走薄封装
- **依据**：原生 generateContent 请求/响应/流式/鉴权与 OpenAI 结构性不同；realtime/deep-research 等能力需原生
- **可复用模型 ID 样例**：`gemini-2.0-flash`、`gemini-2.0-flash-001`、`gemini-2.0-flash-lite`、`deep-research-pro-preview-12-2025`
- **是否需扩展共享层**：是（Google OAuth/ADC 鉴权 + Gemini 消息结构适配）

#### 4. 风险与限制

- 原生实现成本高（GCP 鉴权 + contents/parts 结构）；区域化 base URL；inventory 命名为“language models”但实际为 Gemini 全能力（含 realtime/image-gen）

#### 5. 优先级建议

- **优先级**：P1
- **理由**：Google 主力平台、模型数 42、能力全；原生实现成本高但价值大，OpenAI 兼容端点可作为低成本切入

---

### vertex_ai_text_models — Vertex AI Text Models

- **canonical ID**：vertex_ai_text_models
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：completion

#### 1. 官方协议证据

- **文档 URL**：https://docs.cloud.google.com/vertex-ai/docs/core-release-notes （官方发布说明确认 text-unicorn 为 PaLM 2 for Text GA 模型）；https://cloud.google.com/blog/products/application-development/vertex-ai-palm-and-gemini-apis-using-workflows
- **核验来源**：官方发布说明 + 官方博客（确认 text-unicorn 为 PaLM 2 for Text，Vertex AI SDK 已弃用、PaLM 由 Gemini 取代）
- **证据强度**：中（官方确认 text-unicorn 身份与 PaLM 弃用趋势；具体 `:predict` 端点细节为多来源一致）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://{region}-aiplatform.googleapis.com/v1`
- **鉴权**：方式=Google OAuth2 Bearer Token（服务账号 / ADC）/ 环境变量=`GOOGLE_APPLICATION_CREDENTIALS` / 是否必需=是
- **endpoint 公式**：`POST /v1/projects/{project}/locations/{location}/publishers/google/models/{model}:predict`（PaLM 2 for Text）
- **协议类型**：原生（Google Vertex PaLM Text 协议）
- **请求结构要点**：`instances[].prompt`、`parameters`（temperature/maxOutputTokens 等）；非 OpenAI 结构
- **响应结构要点**：`predictions[].content`；非 OpenAI `choices[]`
- **流式**：未知（PaLM Text `:predict` 原生流式支持有限）
- **错误结构**：厂商专属（Google RPC `error` 结构）
- **特有行为**：text-unicorn 为 PaLM 2 for Text 最大尺寸模型（GA）；PaLM 2 Text 系列已被 Gemini 取代、Vertex AI SDK 已弃用

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：原生（Google Vertex PaLM Text 协议）
- **依据**：`instances/predictions` 结构与 OpenAI 不兼容
- **可复用模型 ID 样例**：`text-unicorn`、`text-unicorn@001`
- **是否需扩展共享层**：否（独立原生实现，与 Gemini generateContent 不同）

#### 4. 风险与限制

- PaLM 2 Text 已被 Gemini 取代、Vertex AI SDK 弃用；模型仅 2、处于弃用通道

#### 5. 优先级建议

- **优先级**：搁置
- **理由**：PaLM 2 Text 弃用、模型仅 2、实现价值低；如需 Vertex 文本能力应走 Gemini generateContent

---

### watsonx — Watsonx

- **canonical ID**：watsonx
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.5
- **能力**（本次调研覆盖）：audio_transcription、chat

#### 1. 官方协议证据

- **文档 URL**：https://www.ibm.com/docs/en/watsonx/saas?topic=code-chat （官方 watsonx.ai chat API 文档，含 curl 请求与响应样例）；https://dataplatform.cloud.ibm.com/docs/content/wsj/analyze-data/ml-authentication.html
- **核验来源**：官方 API 文档（IBM watsonx.ai）
- **证据强度**：强（官方文档确认 chat 端点、请求响应结构、鉴权）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://<region>.<cloud-provider-domain>`（如 `https://us-south.ml.cloud.ibm.com`）
- **鉴权**：方式=IBM Cloud IAM Bearer Token（由 API Key 经 `https://iam.cloud.ibm.com/identity/token` 换取，约 1 小时刷新）/ 环境变量=`WATSONX_APIKEY`/`WATSONX_URL`/`WATSONX_PROJECT_ID` / 是否必需=是
- **endpoint 公式**：chat `POST /ml/v1/text/chat?version=2024-10-08`；文本生成 `POST /ml/v1/text/generation?version=...`；模型列表 `GET /ml/v1/foundation_model_specs?version=...`
- **协议类型**：原生（IBM watsonx 协议）
- **请求结构要点**：`model_id`、`project_id`、`messages[]`（role 小写敏感，content 可为文本或 `[{type:text,text}]`）、`max_tokens`/`time_limit`/`reasoning_effort`/`chat_template_kwargs`；含 `version` 查询参数
- **响应结构要点**：`id`、`model_id`、`choices[].message`、`finish_reason`、`usage`、`created`/`created_at`；部分响应带 `object: "chat.completion"`，但请求侧 `model_id`+`project_id`+`version` 与 OpenAI 结构性不同
- **流式**：SSE（`stream=true`，推断，官方支持流式 chat）
- **错误结构**：厂商专属（IBM 错误结构）
- **特有行为**：强制 `project_id`；IAM token 周期刷新；支持 reasoning（`include_reasoning`/`reasoning_effort`）；模型含 ibm/granite、meta-llama、openai/gpt-oss 等

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：原生（IBM watsonx 协议）
- **依据**：请求需 `model_id`+`project_id`+`version`、鉴权为 IAM token 双步换取，与 OpenAI 结构性不同
- **可复用模型 ID 样例**：`watsonx/ibm/granite-13b-chat-v2`、`meta-llama/llama-3-8b-instruct`、`openai/gpt-oss-120b`
- **是否需扩展共享层**：是（IBM IAM token 刷新 + project_id + version 参数 + 消息结构适配）

#### 4. 风险与限制

- IAM token 周期刷新增加鉴权复杂度；强制 project_id 配置；audio_transcription 能力端点未在 chat 文档中覆盖，需另行核验

#### 5. 优先级建议

- **优先级**：P2
- **理由**：原生协议实现成本中等偏高、需 IAM 刷新与 project 配置；企业场景为主，模型数 29

---

### zenmux — ZenMux

- **canonical ID**：zenmux
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat、embedding、image_generation

#### 1. 官方协议证据

- **文档 URL**：https://docs.zenmux.ai/guide/quickstart.html ；https://docs.zenmux.ai/api/openai/create-chat-completion.html
- **核验来源**：官方 API 文档（Quickstart，含多协议 base URL 表与 curl 请求样例）
- **证据强度**：强（官方文档确认 base URL、鉴权、OpenAI 兼容）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：OpenAI 协议 `https://zenmux.ai/api/v1`；Anthropic 协议 `https://zenmux.ai/api/anthropic`；Google Gemini 协议 `https://zenmux.ai/api/vertex-ai`
- **鉴权**：方式=Bearer Token / 环境变量=`ZENMUX_API_KEY` / 是否必需=是
- **endpoint 公式**：OpenAI Chat Completions `POST /api/v1/chat/completions`、OpenAI Responses `/api/v1/responses`、Anthropic Messages `/api/anthropic/v1/messages`、Google Gemini `/api/vertex-ai/...`
- **协议类型**：OpenAI 兼容（同时兼容 Anthropic 与 Google Gemini）
- **请求结构要点**：标准 OpenAI Chat Completions 请求体，模型用 `provider/model-name` 格式（如 `google/gemini-3.1-pro-preview`）
- **响应结构要点**：标准 OpenAI 响应
- **流式**：SSE（OpenAI 兼容）
- **错误结构**：与 OpenAI 共享结构一致（官方未声明差异）
- **特有行为**：协议无关（cross-protocol calling）——任一协议可调任一模型；支持 provider/model 路由、fallback、structured output、tool calls、prompt cache、reasoning

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：OpenAI Chat Completions 端点请求/响应/鉴权/流式完全兼容，官方明确“fully compatible with the OpenAI Chat Completions API”
- **可复用模型 ID 样例**：`anthropic/claude-opus-4`、`google/gemini-3.1-pro-preview`、`anthropic/claude-3.7-sonnet`
- **是否需扩展共享层**：否（chat/embedding 薄封装即可；image_generation 若支持需模态专用评估）

#### 4. 风险与限制

- 聚合网关，模型可用性依赖上游；image_generation 端点结构需对照 API Reference 二次核验

#### 5. 优先级建议

- **优先级**：P1
- **理由**：OpenAI 兼容、模型数 164、实现成本极低、聚合多厂商模型价值高
