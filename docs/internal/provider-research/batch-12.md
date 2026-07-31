# 第 12 批调研记录（14 个 provider）

> 调研日期：2026-07-28
> 数据来源：`batches/batch-12.json`（线索）+ 各 provider 官方 API 文档
> 原则：inventory 元数据仅作线索；以官方文档为准；无法确认的字段写"未知"或留空，不臆造。

---

### umans_ai — Umans AI

- **canonical ID**：umans_ai
- **aliases**：[]
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://app.umans.ai/offers/code/docs （API Reference 节）；组织版 https://app.umans.ai/offers/code/docs/orgs
- **核验来源**：官方 API 文档（用户指南内嵌 API Reference + curl 示例）
- **证据强度**：强（官方文档直接给出 OpenAI / Anthropic 两套端点的 curl 与请求体）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.code.umans.ai`（OpenAI 路由用 `https://api.code.umans.ai/v1`，对应 `/v1/chat/completions`）
- **鉴权**：方式= Bearer（OpenAI 路由 `Authorization: Bearer sk-...`）/ 环境变量= `UMANS_AI_API_KEY`（inventory 标注，官方 Dashboard → API Keys 生成）/ 是否必需= 是
- **endpoint 公式**：`POST /v1/chat/completions`（OpenAI 兼容）；另提供 `POST /v1/messages`（Anthropic 兼容，`x-api-key` + `anthropic-version: 2023-06-01`）
- **协议类型**：OpenAI 兼容（同时提供 Anthropic Messages 兼容端点）
- **请求结构要点**：标准 OpenAI Chat Completions `{model, messages, stream, ...}`
- **响应结构要点**：标准 OpenAI Chat Completions 响应
- **流式**：SSE（`stream:true`，curl 用 `-N`）
- **错误结构**：与 OpenAI 共享结构一致（推断，官方未单独列出错误码表）
- **特有行为**：`umans-coder` 为路由别名（当前指向 Kimi K2.7-Code，可能随评估变更）；`umans-glm-5.2` 在 OpenAI 路由上仅文本，视觉走组合路径；模型为开源权重自托管（Kimi K2.7-Code / GLM 5.2 / Qwen3.6-35B-A3B）

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装（OpenAI 兼容）
- **依据**：官方文档明确实现 OpenAI Chat Completions API，base URL / Bearer 鉴权 / 请求响应 / SSE 流式均可由 OpenAI 共享层表达
- **可复用模型 ID 样例**：`umans-coder`、`umans-kimi-k2.7`、`umans-glm-5.2`、`umans-flash`
- **是否需扩展共享层**：否

#### 4. 风险与限制

- `umans-coder` 为动态路由别名，底层模型可能变更，需以官方 `/v1/models` 或文档为准。
- inventory model_sample 中 `umans-glm-5.1`、`umans-kimi-k2.6` 为旧版本号，当前文档为 `umans-glm-5.2`、`umans-kimi-k2.7`。
- GLM 5.2 视觉能力在 OpenAI 路由不可用（仅文本）。

#### 5. 优先级建议

- **优先级**：P0（立即）
- **理由**：证据强 + 薄封装 + 有可用模型 ID，OpenAI 共享层即可承载。

---

### umans_ai_coding_plan — Umans AI Coding Plan

- **canonical ID**：umans_ai_coding_plan
- **aliases**：[]
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://app.umans.ai/offers/code/docs （与 umans_ai 同一份用户指南；"Coding Plan" 为 models.dev 上的计费方案命名）
- **核验来源**：官方 API 文档
- **证据强度**：强（与 umans_ai 同入口同协议）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.code.umans.ai/v1`（与 umans_ai 完全一致）
- **鉴权**：方式= Bearer / 环境变量= `UMANS_AI_CODING_PLAN_API_KEY`（inventory 标注）/ 是否必需= 是
- **endpoint 公式**：`POST /v1/chat/completions`（OpenAI 兼容）；`POST /v1/messages`（Anthropic 兼容）
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions
- **响应结构要点**：标准 OpenAI Chat Completions
- **流式**：SSE
- **错误结构**：与 OpenAI 共享结构一致（推断）
- **特有行为**：与 umans_ai 同一 API 端点与模型集；差异仅在计费方案（个人 Pro/Max 订阅 vs 组织 service-account key）与 inventory 环境变量名

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装（复用 umans_ai 实现）
- **依据**：与 umans_ai 同 base URL、同协议、同模型，仅需额外环境变量别名
- **可复用模型 ID 样例**：`umans-coder`、`umans-kimi-k2.7`、`umans-glm-5.2`、`umans-flash`
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 作为独立 provider 与 umans_ai 完全重复（同入口、同协议、同模型），仅 env 名/计费方案不同。
- 不建议作为独立 provider 单独实现，应合并入 umans_ai 薄封装，将 `UMANS_AI_CODING_PLAN_API_KEY` 作为 env 别名。

#### 5. 优先级建议

- **优先级**：搁置
- **理由**：与 umans_ai 同入口/同协议/同模型，已有 umans_ai 薄封装覆盖，作为独立 provider 冗余；仅需 env 别名即可复用。

---

### unorouter — UnoRouter

- **canonical ID**：unorouter
- **aliases**：[]
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://unorouter.com/en （首页 curl 示例）；https://unorouter.com/en/blog/unorouter-vs-openrouter （官方博客明述 OpenAI 兼容）；https://unorouter.com/en/docs/integrations/sillytavern （集成页给 base URL）。注：站点为 JS 渲染 SPA，WebFetch 无法读取完整页面，证据来自官方域名片段。
- **核验来源**：官方文档（站点首页/博客/集成页片段）
- **证据强度**：中（官方来源直接确认 OpenAI 兼容 + base URL + endpoint；完整 auth 头/错误结构未在可读片段中显式捕获，auth 按 OpenAI 兼容标准推断为 Bearer）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.unorouter.com/v1`
- **鉴权**：方式= Bearer（推断，OpenAI 兼容标准）/ 环境变量= `UNOROUTER_API_KEY`（inventory 标注）/ 是否必需= 是
- **endpoint 公式**：`POST /v1/chat/completions`（OpenAI 兼容）；`GET /v1/models`（模型列表）；官方另提供 Anthropic 原生 `/v1/messages` 与 Gemini 原生 `/v1beta` 端点
- **协议类型**：OpenAI 兼容（聚合网关，同时暴露 Anthropic / Gemini 原生端点）
- **请求结构要点**：标准 OpenAI Chat Completions
- **响应结构要点**：标准 OpenAI Chat Completions
- **流式**：SSE（OpenAI 标准，推断）
- **错误结构**：与 OpenAI 共享结构一致（推断）
- **特有行为**：聚合 190+ 模型（含免费 tier，模型名跨厂商，如 `claude-*`、`deepseek-v4-flash:free`）；支持 BYO OpenAI 兼容端点

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装（OpenAI 兼容）
- **依据**：官方博客明述"The API is OpenAI compatible"，base URL `https://api.unorouter.com/v1`，模型名来自 `GET /v1/models`，请求响应可由 OpenAI 共享层表达
- **可复用模型 ID 样例**：`claude-haiku-4-5-20251001`、`claude-opus-4-8`、`deepseek-v4-flash`（聚合，需以 `GET /v1/models` 为准）
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 模型集随上游免费池变动，需运行时拉取 `/v1/models`。
- 跨厂商代理模型名，与原厂能力可能存在差异。
- 官方站点为 SPA，完整文档未能读取，auth 头/错误结构为推断，落地前建议人工复核。

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：证据中 + 薄封装路径明确；落地前需人工补确认 auth 头与错误结构。

---

### venice — Venice AI

- **canonical ID**：venice
- **aliases**：[]
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://docs.venice.ai/api-reference/api-spec （含 OpenAPI YAML: https://api.venice.ai/doc/api/swagger.yaml）；https://docs.venice.ai/getting-started/quick-start
- **核验来源**：官方 API 文档 + OpenAPI spec
- **证据强度**：强（官方文档直接给出 base URL、Bearer 鉴权、curl/SDK 示例、流式、venice_parameters 扩展表）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.venice.ai/api/v1`（inventory `base_urls` 为空，已由官方文档补全）
- **鉴权**：方式= Bearer（`Authorization: Bearer VENICE_API_KEY`）/ 环境变量= `VENICE_API_KEY`（inventory `api_key_env_vars` 为空，已由官方文档补全）/ 是否必需= 是
- **endpoint 公式**：`POST /api/v1/chat/completions`（OpenAI 兼容）；另有 `/models`、image、audio、video、embeddings 端点
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions + 可选 `venice_parameters` 扩展对象
- **响应结构要点**：标准 OpenAI Chat Completions
- **流式**：SSE（`stream:true`）
- **错误结构**：与 OpenAI 共享结构一致（官方有错误码页）
- **特有行为**：`venice_parameters` 扩展（`character_slug`、`enable_web_search`、`enable_web_scraping`、`include_venice_system_prompt`、`strip_thinking_response`、`disable_thinking` 等）；模型名可带特性后缀（如 `zai-org-glm-5:enable_web_search=auto`）；支持 prompt caching（`cache_control`）

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装（OpenAI 兼容）；`venice_parameters` 作为可选扩展经 `extra_body` 透传，非必需
- **依据**：官方明述"Venice's API implements the OpenAI API specification"，OpenAI SDK 直接可用，base URL / Bearer / 请求响应 / SSE 流式均可由共享层表达
- **可复用模型 ID 样例**：`zai-org-glm-5`、`kimi-k2-6`、`claude-opus-4-8`、`venice-uncensored-1-2`
- **是否需扩展共享层**：否（`venice_parameters` 走透传即可，无需改共享层）

#### 4. 风险与限制

- 含大量跨厂商代理模型名（claude-*、kimi-*、glm-* 等），与原厂能力/定价可能不一致。
- `venice_parameters` 为厂商扩展，若要一等公民支持需共享层透传机制。
- inventory `base_urls` 与 `api_key_env_vars` 均为空，已由官方文档修正补全。

#### 5. 优先级建议

- **优先级**：P0（立即）
- **理由**：证据强 + 薄封装 + 有可用模型 ID，OpenAI 共享层即可承载。

---

### vertex_ai_ai21_models — Vertex AI Ai21 Models

- **canonical ID**：vertex_ai_ai21_models
- **aliases**：[]
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：无独立文档 URL（inventory 来自 litellm_prices）。核验依据：Google Cloud 官方文档 https://docs.cloud.google.com/gemini-enterprise-agent-platform/models/partner-models/use-partner-models （partner MaaS 总则）+ AI21 Jamba model card（Model Garden `publishers/ai21`）
- **核验来源**：官方 API 文档（Google Cloud Vertex AI / Gemini Enterprise Agent Platform partner-models 文档）
- **证据强度**：强（官方文档确认 partner MaaS 模型复用 Vertex AI 入口、Google OAuth2 鉴权、经 `publishers/{publisher}/models/{model}` 路径调用；非 OpenAI 兼容）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：复用 Vertex AI 入口 `https://{location}-aiplatform.googleapis.com` 或 `https://aiplatform.googleapis.com`（global），路径 `/v1/projects/{project}/locations/{location}/publishers/ai21/models/{model}`
- **鉴权**：方式= Google Cloud OAuth2 Bearer token（ADC / service account）/ 环境变量= 复用 `GOOGLE_VERTEX_ACCESS_TOKEN` + `GOOGLE_VERTEX_PROJECT` + `GOOGLE_VERTEX_LOCATION`（与现有 vertex provider 一致）/ 是否必需= 是（partner MaaS 不支持 Express API key）
- **endpoint 公式**：`publishers/ai21/models/{model}:rawPredict` / `:streamRawPredict`（透传 AI21 原生请求体）；模型样例 `jamba-1.5`、`jamba-1.5-large`、`jamba-1.5-mini`
- **协议类型**：原生（非 OpenAI 兼容；Vertex 的 OpenAI 兼容端点 `/endpoints/openapi/chat/completions` 不覆盖 partner MaaS 模型）
- **请求结构要点**：AI21 原生请求体经 `rawPredict` 透传（确切字段需按 AI21 model card 核验）
- **响应结构要点**：AI21 原生响应（经 rawPredict 包装）
- **流式**：SSE（`streamRawPredict`）
- **错误结构**：Vertex AI / 厂商原生错误结构
- **特有行为**：模型名带 `-maas` / `@001` 后缀；复用 vertex_ai 入口与计费

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：共享层扩展 / 原生（扩展现有 `vertex` provider 以支持 partner publisher 路由 + `rawPredict`/`streamRawPredict`）
- **依据**：复用 Vertex AI 入口与 Google OAuth2；非 OpenAI 兼容，不能薄封装；现有 vertex provider 仅支持 `publishers/google` + `generateContent`
- **可复用模型 ID 样例**：`vertex_ai/jamba-1.5`、`vertex_ai/jamba-1.5-large`、`vertex_ai/jamba-1.5-mini`
- **是否需扩展共享层**：是（vertex provider 需新增 partner publisher 路由 + rawPredict 透传）

#### 4. 风险与限制

- AI21 原生请求体格式需按 model card 核验，本批未逐一确认字段。
- 需先扩展 vertex provider 的 publisher 路由与 rawPredict 支持。

#### 5. 优先级建议

- **优先级**：P2（后续）
- **理由**：证据强但属 vertex provider 扩展（非独立 provider），原生协议需共享层工作，优先级低于薄封装候选。

---

### vertex_ai_anthropic_models — Vertex AI Anthropic Models

- **canonical ID**：vertex_ai_anthropic_models
- **aliases**：[]
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://docs.cloud.google.com/gemini-enterprise-agent-platform/models/partner-models/claude/use-claude （Request predictions with Claude models）；https://docs.cloud.google.com/gemini-enterprise-agent-platform/models/partner-models/claude
- **核验来源**：官方 API 文档（Google Cloud + Anthropic Vertex SDK 示例）
- **证据强度**：强（官方文档直接确认 Claude on Vertex 经 Vertex 端点、Google OAuth2、Anthropic Messages API、SSE 流式；`client.messages.stream(model="claude-3-5-sonnet-v2@20241022", ...)`）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：复用 Vertex AI 入口 `https://{location}-aiplatform.googleapis.com`（如 us-east5）或 `https://aiplatform.googleapis.com`（global），路径 `/v1/projects/{project}/locations/{location}/publishers/anthropic/models/{model}`
- **鉴权**：方式= Google Cloud OAuth2 Bearer token（ADC / `gcloud auth application-default login`）/ 环境变量= 复用 `GOOGLE_VERTEX_ACCESS_TOKEN` + `GOOGLE_VERTEX_PROJECT` + `GOOGLE_VERTEX_LOCATION` / 是否必需= 是
- **endpoint 公式**：`publishers/anthropic/models/{model}:rawPredict` / `:streamRawPredict`，请求体为 Anthropic Messages API（`messages`、`max_tokens`、`system`、`anthropic_version` 等）
- **协议类型**：原生（Anthropic Messages API 经 Vertex rawPredict 透传；非 OpenAI 兼容）
- **请求结构要点**：Anthropic Messages 格式（role/content blocks、max_tokens、system、tools 等）
- **响应结构要点**：Anthropic Messages 响应（content blocks、stop_reason、usage）
- **流式**：SSE（`streamRawPredict`，Anthropic 事件流）
- **错误结构**：Vertex AI / Anthropic 原生错误结构
- **特有行为**：模型名如 `claude-3-5-sonnet-v2@20241022`、`claude-opus-4-8`；复用 vertex_ai 入口与计费；支持 vision、computer use、prompt caching、token counting（count-tokens 端点）

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：共享层扩展 / 原生（新增 `anthropic_vertex` 适配器，类比现有 `anthropic_aws`：复用 `anthropic::convert` 消息转换 + Vertex Google OAuth2 + rawPredict/streamRawPredict 端点）
- **依据**：官方确认 Anthropic Messages 协议经 Vertex 端点；aimux 已有 `anthropic` provider（`convert.rs`）与 `anthropic_aws` 范式可复用
- **可复用模型 ID 样例**：`vertex_ai/claude-3-5-haiku@20241022`、`vertex_ai/claude-3-7-sonnet@20250219`、`vertex_ai/claude-opus-4-8`
- **是否需扩展共享层**：是（需 vertex provider 支持 partner/anthropic publisher + rawPredict + 复用 Anthropic 转换）

#### 4. 风险与限制

- Claude on Vertex 受 Anthropic 转售政策限制（部分 reseller billing 账号无法启用）。
- 需区域端点（如 us-east5）支持 Claude；多区域/global 端点选择需配置。
- 复用 Anthropic 消息转换需回归测试 Vertex 特有包装层。

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：证据强、路径明确（复用 anthropic::convert + vertex auth，类比 anthropic_aws），Claude on Vertex 为高需求模型；属 vertex provider 扩展但价值与可行性高于其他 partner family。

---

### vertex_ai_deepseek_models — Vertex AI Deepseek Models

- **canonical ID**：vertex_ai_deepseek_models
- **aliases**：[]
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：无独立文档 URL（inventory 来自 litellm_prices）。核验依据：Google Cloud partner-models 总则 + Model Garden `publishers/deepseek-ai` model card（模型名 `-maas` 后缀）
- **核验来源**：官方 API 文档（Google Cloud Vertex AI partner MaaS 机制）
- **证据强度**：强（复用 Vertex 入口、Google OAuth2、rawPredict 机制已确认；DeepSeek 原生请求体格式未逐一核验）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：复用 Vertex AI 入口，路径 `/v1/projects/{project}/locations/{location}/publishers/deepseek-ai/models/{model}`
- **鉴权**：方式= Google Cloud OAuth2 Bearer / 环境变量= 复用 vertex env / 是否必需= 是
- **endpoint 公式**：`publishers/deepseek-ai/models/{model}:rawPredict` / `:streamRawPredict`；模型样例 `deepseek-r1-0528-maas`、`deepseek-v3.1-maas`、`deepseek-v3.2-maas`
- **协议类型**：原生（非 OpenAI 兼容）
- **请求结构要点**：DeepSeek 原生请求体经 rawPredict 透传（确切字段需按 model card 核验）
- **响应结构要点**：DeepSeek 原生响应
- **流式**：SSE（`streamRawPredict`）
- **错误结构**：Vertex AI / 厂商原生错误结构
- **特有行为**：模型名带 `-maas` 后缀；复用 vertex_ai 入口

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：共享层扩展 / 原生（扩展 vertex provider 支持 partner publisher + rawPredict）
- **依据**：复用 Vertex 入口与 Google OAuth2；非 OpenAI 兼容
- **可复用模型 ID 样例**：`vertex_ai/deepseek-ai/deepseek-r1-0528-maas`、`vertex_ai/deepseek-ai/deepseek-v3.1-maas`
- **是否需扩展共享层**：是

#### 4. 风险与限制

- DeepSeek 原生请求体格式需按 model card 核验。
- 依赖 vertex provider 的 partner/rawPredict 扩展先行。

#### 5. 优先级建议

- **优先级**：P2（后续）
- **理由**：证据强但属 vertex provider 扩展，原生协议，优先级低于薄封装候选与 Claude on Vertex。

---

### vertex_ai_llama_models — Vertex AI Llama Models

- **canonical ID**：vertex_ai_llama_models
- **aliases**：[]
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：无独立文档 URL（inventory 来自 litellm_prices）。核验依据：Google Cloud partner-models 总则 + Model Garden `publishers/meta` model card
- **核验来源**：官方 API 文档（Google Cloud Vertex AI partner MaaS 机制）
- **证据强度**：强（复用 Vertex 入口、Google OAuth2、rawPredict 机制已确认；Llama 请求体格式未逐一核验）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：复用 Vertex AI 入口，路径 `/v1/projects/{project}/locations/{location}/publishers/meta/models/{model}`
- **鉴权**：方式= Google Cloud OAuth2 Bearer / 环境变量= 复用 vertex env / 是否必需= 是
- **endpoint 公式**：`publishers/meta/models/{model}:rawPredict` / `:streamRawPredict`（Llama MaaS）；模型样例 `llama-3.1-405b-instruct-maas`、`llama-3.2-90b-vision-instruct-maas`、`llama-4-maverick-17b-128e-instruct-maas`
- **协议类型**：原生（非 OpenAI 兼容；OpenAI 兼容端点不覆盖 partner MaaS）
- **请求结构要点**：Llama 原生请求体经 rawPredict 透传（部分 Llama 模型亦支持 `:generateContent` Gemini 格式，需按 model card 核验）
- **响应结构要点**：Llama 原生响应
- **流式**：SSE（`streamRawPredict`）
- **错误结构**：Vertex AI / 厂商原生错误结构
- **特有行为**：含 vision-instruct 变体；模型名带 `-maas` 后缀

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：共享层扩展 / 原生（扩展 vertex provider 支持 partner publisher + rawPredict）
- **依据**：复用 Vertex 入口与 Google OAuth2；非 OpenAI 兼容
- **可复用模型 ID 样例**：`vertex_ai/meta/llama-3.1-405b-instruct-maas`、`vertex_ai/meta/llama-3.2-90b-vision-instruct-maas`
- **是否需扩展共享层**：是

#### 4. 风险与限制

- Llama 请求体格式（rawPredict 原生 vs generateContent Gemini）需按 model card 核验。
- 依赖 vertex provider 的 partner/rawPredict 扩展先行。

#### 5. 优先级建议

- **优先级**：P2（后续）
- **理由**：证据强但属 vertex provider 扩展，原生协议，优先级低于薄封装候选与 Claude on Vertex。

---

### vertex_ai_minimax_models — Vertex AI Minimax Models

- **canonical ID**：vertex_ai_minimax_models
- **aliases**：[]
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：无独立文档 URL（inventory 来自 litellm_prices）。核验依据：Google Cloud partner-models 总则 + Model Garden `publishers/minimaxai` model card
- **核验来源**：官方 API 文档（Google Cloud Vertex AI partner MaaS 机制）
- **证据强度**：强（复用 Vertex 入口、Google OAuth2、rawPredict 机制已确认；MiniMax 原生请求体格式未逐一核验）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：复用 Vertex AI 入口，路径 `/v1/projects/{project}/locations/{location}/publishers/minimaxai/models/{model}`
- **鉴权**：方式= Google Cloud OAuth2 Bearer / 环境变量= 复用 vertex env / 是否必需= 是
- **endpoint 公式**：`publishers/minimaxai/models/{model}:rawPredict` / `:streamRawPredict`；模型样例 `minimax-m2-maas`
- **协议类型**：原生（非 OpenAI 兼容）
- **请求结构要点**：MiniMax 原生请求体经 rawPredict 透传（确切字段需按 model card 核验）
- **响应结构要点**：MiniMax 原生响应
- **流式**：SSE（`streamRawPredict`）
- **错误结构**：Vertex AI / 厂商原生错误结构
- **特有行为**：模型名带 `-maas` 后缀；复用 vertex_ai 入口

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：共享层扩展 / 原生（扩展 vertex provider 支持 partner publisher + rawPredict）
- **依据**：复用 Vertex 入口与 Google OAuth2；非 OpenAI 兼容
- **可复用模型 ID 样例**：`vertex_ai/minimaxai/minimax-m2-maas`
- **是否需扩展共享层**：是

#### 4. 风险与限制

- MiniMax 原生请求体格式需按 model card 核验。
- 依赖 vertex provider 的 partner/rawPredict 扩展先行。

#### 5. 优先级建议

- **优先级**：P2（后续）
- **理由**：证据强但属 vertex provider 扩展，原生协议，优先级低于薄封装候选与 Claude on Vertex。

---

### vertex_ai_mistral_models — Vertex AI Mistral Models

- **canonical ID**：vertex_ai_mistral_models
- **aliases**：[]
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：无独立文档 URL（inventory 来自 litellm_prices）。核验依据：Google Cloud partner-models 总则（官方明列 Mistral 为 partner MaaS）+ Model Garden `publishers/mistral` model card
- **核验来源**：官方 API 文档（Google Cloud Vertex AI partner MaaS 机制）
- **证据强度**：强（复用 Vertex 入口、Google OAuth2、rawPredict 机制已确认；Mistral 原生请求体格式未逐一核验）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：复用 Vertex AI 入口，路径 `/v1/projects/{project}/locations/{location}/publishers/mistral/models/{model}`
- **鉴权**：方式= Google Cloud OAuth2 Bearer / 环境变量= 复用 vertex env / 是否必需= 是
- **endpoint 公式**：`publishers/mistral/models/{model}:rawPredict` / `:streamRawPredict`；模型样例 `codestral-2`、`codestral-2501`、`codestral@2405`
- **协议类型**：原生（非 OpenAI 兼容）
- **请求结构要点**：Mistral 原生请求体经 rawPredict 透传（确切字段需按 model card 核验）
- **响应结构要点**：Mistral 原生响应
- **流式**：SSE（`streamRawPredict`）
- **错误结构**：Vertex AI / 厂商原生错误结构
- **特有行为**：模型名带版本/`@latest` 后缀；复用 vertex_ai 入口

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：共享层扩展 / 原生（扩展 vertex provider 支持 partner publisher + rawPredict）
- **依据**：复用 Vertex 入口与 Google OAuth2；非 OpenAI 兼容
- **可复用模型 ID 样例**：`vertex_ai/codestral-2`、`vertex_ai/codestral@2405`
- **是否需扩展共享层**：是

#### 4. 风险与限制

- Mistral 原生请求体格式需按 model card 核验。
- 依赖 vertex provider 的 partner/rawPredict 扩展先行。

#### 5. 优先级建议

- **优先级**：P2（后续）
- **理由**：证据强但属 vertex provider 扩展，原生协议，优先级低于薄封装候选与 Claude on Vertex。

---

### vertex_ai_moonshot_models — Vertex AI Moonshot Models

- **canonical ID**：vertex_ai_moonshot_models
- **aliases**：[]
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：无独立文档 URL（inventory 来自 litellm_prices）。核验依据：Google Cloud partner-models 总则 + Model Garden `publishers/moonshotai` model card
- **核验来源**：官方 API 文档（Google Cloud Vertex AI partner MaaS 机制）
- **证据强度**：强（复用 Vertex 入口、Google OAuth2、rawPredict 机制已确认；Moonshot 原生请求体格式未逐一核验）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：复用 Vertex AI 入口，路径 `/v1/projects/{project}/locations/{location}/publishers/moonshotai/models/{model}`
- **鉴权**：方式= Google Cloud OAuth2 Bearer / 环境变量= 复用 vertex env / 是否必需= 是
- **endpoint 公式**：`publishers/moonshotai/models/{model}:rawPredict` / `:streamRawPredict`；模型样例 `kimi-k2-thinking-maas`
- **协议类型**：原生（非 OpenAI 兼容）
- **请求结构要点**：Moonshot 原生请求体经 rawPredict 透传（确切字段需按 model card 核验）
- **响应结构要点**：Moonshot 原生响应
- **流式**：SSE（`streamRawPredict`）
- **错误结构**：Vertex AI / 厂商原生错误结构
- **特有行为**：模型名带 `-maas` 后缀；复用 vertex_ai 入口

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：共享层扩展 / 原生（扩展 vertex provider 支持 partner publisher + rawPredict）
- **依据**：复用 Vertex 入口与 Google OAuth2；非 OpenAI 兼容
- **可复用模型 ID 样例**：`vertex_ai/moonshotai/kimi-k2-thinking-maas`
- **是否需扩展共享层**：是

#### 4. 风险与限制

- Moonshot 原生请求体格式需按 model card 核验。
- 依赖 vertex provider 的 partner/rawPredict 扩展先行。

#### 5. 优先级建议

- **优先级**：P2（后续）
- **理由**：证据强但属 vertex provider 扩展，原生协议，优先级低于薄封装候选与 Claude on Vertex。

---

### vertex_ai_openai_models — Vertex AI Openai Models

- **canonical ID**：vertex_ai_openai_models
- **aliases**：[]
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：无独立文档 URL（inventory 来自 litellm_prices）。核验依据：Google Cloud partner-models 总则 + Model Garden `publishers/openai` / `publishers/google` model card
- **核验来源**：官方 API 文档（Google Cloud Vertex AI partner MaaS 机制）
- **证据强度**：强（复用 Vertex 入口、Google OAuth2、rawPredict 机制已确认；gpt-oss / gemma 原生请求体格式未逐一核验）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：复用 Vertex AI 入口，路径 `/v1/projects/{project}/locations/{location}/publishers/{publisher}/models/{model}`（publisher 为 `openai` 或 `google`）
- **鉴权**：方式= Google Cloud OAuth2 Bearer / 环境变量= 复用 vertex env / 是否必需= 是
- **endpoint 公式**：`publishers/openai/models/{model}:rawPredict` / `:streamRawPredict`（gpt-oss MaaS）；模型样例 `openai/gpt-oss-120b-maas`、`openai/gpt-oss-20b-maas`、`google/gemma-4-26b-a4b-it-maas`
- **协议类型**：原生（非 OpenAI 兼容；注意：尽管 publisher 为 "openai"，模型经 Vertex rawPredict 调用，OpenAI 兼容端点不覆盖此类 partner MaaS）
- **请求结构要点**：厂商原生请求体经 rawPredict 透传（确切字段需按 model card 核验）
- **响应结构要点**：厂商原生响应
- **流式**：SSE（`streamRawPredict`）
- **错误结构**：Vertex AI / 厂商原生错误结构
- **特有行为**：模型名带 `-maas` 后缀；混含 openai(gpt-oss) 与 google(gemma) 两个 publisher；复用 vertex_ai 入口

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：共享层扩展 / 原生（扩展 vertex provider 支持 partner publisher + rawPredict）
- **依据**：复用 Vertex 入口与 Google OAuth2；非 OpenAI 兼容
- **可复用模型 ID 样例**：`vertex_ai/openai/gpt-oss-120b-maas`、`vertex_ai/google/gemma-4-26b-a4b-it-maas`
- **是否需扩展共享层**：是

#### 4. 风险与限制

- publisher 名 "openai" 易误判为 OpenAI 兼容，实为 Vertex rawPredict 透传，需注意区分。
- 请求体格式需按 model card 核验；依赖 vertex provider 扩展先行。

#### 5. 优先级建议

- **优先级**：P2（后续）
- **理由**：证据强但属 vertex provider 扩展，原生协议，优先级低于薄封装候选与 Claude on Vertex。

---

### vertex_ai_qwen_models — Vertex AI Qwen Models

- **canonical ID**：vertex_ai_qwen_models
- **aliases**：[]
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：无独立文档 URL（inventory 来自 litellm_prices）。核验依据：Google Cloud partner-models 总则 + Model Garden `publishers/qwen` model card
- **核验来源**：官方 API 文档（Google Cloud Vertex AI partner MaaS 机制）
- **证据强度**：强（复用 Vertex 入口、Google OAuth2、rawPredict 机制已确认；Qwen 原生请求体格式未逐一核验）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：复用 Vertex AI 入口，路径 `/v1/projects/{project}/locations/{location}/publishers/qwen/models/{model}`
- **鉴权**：方式= Google Cloud OAuth2 Bearer / 环境变量= 复用 vertex env / 是否必需= 是
- **endpoint 公式**：`publishers/qwen/models/{model}:rawPredict` / `:streamRawPredict`；模型样例 `qwen3-235b-a22b-instruct-2507-maas`、`qwen3-coder-480b-a35b-instruct-maas`、`qwen3-next-80b-a3b-thinking-maas`
- **协议类型**：原生（非 OpenAI 兼容）
- **请求结构要点**：Qwen 原生请求体经 rawPredict 透传（确切字段需按 model card 核验）
- **响应结构要点**：Qwen 原生响应
- **流式**：SSE（`streamRawPredict`）
- **错误结构**：Vertex AI / 厂商原生错误结构
- **特有行为**：含 instruct / thinking / coder 变体；模型名带 `-maas` 后缀

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：共享层扩展 / 原生（扩展 vertex provider 支持 partner publisher + rawPredict）
- **依据**：复用 Vertex 入口与 Google OAuth2；非 OpenAI 兼容
- **可复用模型 ID 样例**：`vertex_ai/qwen/qwen3-235b-a22b-instruct-2507-maas`、`vertex_ai/qwen/qwen3-coder-480b-a35b-instruct-maas`
- **是否需扩展共享层**：是

#### 4. 风险与限制

- Qwen 原生请求体格式需按 model card 核验。
- 依赖 vertex provider 的 partner/rawPredict 扩展先行。

#### 5. 优先级建议

- **优先级**：P2（后续）
- **理由**：证据强但属 vertex provider 扩展，原生协议，优先级低于薄封装候选与 Claude on Vertex。

---

### vertex_ai_zai_models — Vertex AI ZAI Models

- **canonical ID**：vertex_ai_zai_models
- **aliases**：[]
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：无独立文档 URL（inventory 来自 litellm_prices）。核验依据：Google Cloud partner-models 总则 + Model Garden `publishers/zai-org` model card
- **核验来源**：官方 API 文档（Google Cloud Vertex AI partner MaaS 机制）
- **证据强度**：强（复用 Vertex 入口、Google OAuth2、rawPredict 机制已确认；Z.AI GLM 原生请求体格式未逐一核验）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：复用 Vertex AI 入口，路径 `/v1/projects/{project}/locations/{location}/publishers/zai-org/models/{model}`
- **鉴权**：方式= Google Cloud OAuth2 Bearer / 环境变量= 复用 vertex env / 是否必需= 是
- **endpoint 公式**：`publishers/zai-org/models/{model}:rawPredict` / `:streamRawPredict`；模型样例 `glm-4.7-maas`、`glm-5-maas`
- **协议类型**：原生（非 OpenAI 兼容）
- **请求结构要点**：Z.AI GLM 原生请求体经 rawPredict 透传（确切字段需按 model card 核验）
- **响应结构要点**：Z.AI GLM 原生响应
- **流式**：SSE（`streamRawPredict`）
- **错误结构**：Vertex AI / 厂商原生错误结构
- **特有行为**：模型名带 `-maas` 后缀；复用 vertex_ai 入口

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：共享层扩展 / 原生（扩展 vertex provider 支持 partner publisher + rawPredict）
- **依据**：复用 Vertex 入口与 Google OAuth2；非 OpenAI 兼容
- **可复用模型 ID 样例**：`vertex_ai/zai-org/glm-4.7-maas`、`vertex_ai/zai-org/glm-5-maas`
- **是否需扩展共享层**：是

#### 4. 风险与限制

- Z.AI GLM 原生请求体格式需按 model card 核验。
- 依赖 vertex provider 的 partner/rawPredict 扩展先行。

#### 5. 优先级建议

- **优先级**：P2（后续）
- **理由**：证据强但属 vertex provider 扩展，原生协议，优先级低于薄封装候选与 Claude on Vertex。
