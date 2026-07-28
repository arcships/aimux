# 第 5 批调研记录（14 个 provider）

> 调研日期：2026-07-28。证据裁决遵循 RFC-0006 §2.1：官方 API 文档/SDK 优先，inventory 元数据仅作线索。逐 provider 实际核验官方文档后填写，无法确认的字段标"未知"或留空，未臆造任何协议细节。

---

### cloudferro_sherlock — CloudFerro Sherlock

- **canonical ID**：cloudferro_sherlock
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://docs.sherlock.cloudferro.com/docs/introduction/ ；https://docs.sherlock.cloudferro.com/chat-completion-endpoint/ ；https://docs.sherlock.cloudferro.com/models-endpoint/
- **核验来源**：官方 API 文档 + 官方博客（cloudferro.com/blog/how-to-use-cloudferro-ai-hub-sherlock/）
- **证据强度**：强（官方文档与博客明确声明 OpenAI 兼容，并给出 base URL）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api-sherlock.cloudferro.com/openai/v1`
- **鉴权**：方式=API Key（Bearer，标准 OpenAI 头）/ 环境变量=CLOUDFERRO_SHERLOCK_API_KEY / 是否必需=是
- **endpoint 公式**：`POST {base}/chat/completions`；模型列表 `GET {base}/models`
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions（messages、model、stream 等），官方明确"compatible with the OpenAI API"
- **响应结构要点**：标准 OpenAI Chat Completions 响应（choices[].message.content）
- **流式**：SSE（OpenAI 兼容 stream 协议）
- **错误结构**：未知（官方文档未明确，推测与 OpenAI 共享结构一致）
- **特有行为**：欧盟波兰数据中心，no-training 策略，按 token 计费；模型 ID 形如 `MiniMaxAI/MiniMax-M2.5`、`meta-llama/Llama-3.3-70B-Instruct`、`openai/gpt-oss-120b`、`speakleash/Bielik-11B-v2.6-Instruct`

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方文档与博客明确 OpenAI 兼容，base URL 以 `/openai/v1` 收尾，鉴权/请求/响应均按 OpenAI Chat Completions
- **可复用模型 ID 样例**：`MiniMaxAI/MiniMax-M2.5`、`meta-llama/Llama-3.3-70B-Instruct`、`openai/gpt-oss-120b`
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 官方错误结构未在文档中明确，需以实测兜底。
- 模型 ID 使用 `vendor/model` 形式，需确认是否需原样透传。

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：OpenAI 兼容薄封装，成本低；欧盟合规定位有差异化价值。

---

### cloudflare — Cloudflare

- **canonical ID**：cloudflare
- **aliases**：无
- **provider_kind**：cloud_platform
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://developers.cloudflare.com/workers-ai/configuration/open-ai-compatibility/ ；https://docs.litellm.ai/docs/providers/cloudflare_workers
- **核验来源**：官方 API 文档（Cloudflare Workers AI OpenAI 兼容端点）+ litellm 成熟实现
- **证据强度**：强（官方文档直接确认请求响应结构）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.cloudflare.com/client/v4/accounts/{CLOUDFLARE_ACCOUNT_ID}/ai/v1`（inventory 中的 `https://api.cloudflare.com` 不完整，需补 account 作用域路径）
- **鉴权**：方式=Bearer API Token / 环境变量=CLOUDFLARE_API_KEY + CLOUDFLARE_ACCOUNT_ID / 是否必需=是
- **endpoint 公式**：`POST {base}/chat/completions`、`POST {base}/embeddings`
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions / Embeddings，可直接用 openai SDK 换 base URL
- **响应结构要点**：标准 OpenAI 响应
- **流式**：SSE（OpenAI 兼容）
- **错误结构**：与 OpenAI 共享结构一致（OpenAI 兼容端点）
- **特有行为**：base URL 必须含 account_id（账户作用域）；inventory 模型 ID 带 `cloudflare/@cf/...` 前缀（litellm 惯例），实际模型名为 `@cf/...`

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方文档明确 OpenAI 兼容端点；与 `cloudflare_workers_ai` 为同一产品（Workers AI），本条目来自 litellm 源、模型带 `cloudflare/` 前缀
- **可复用模型 ID 样例**：`@cf/meta/llama-3.1-8b-instruct`、`@cf/deepseek-ai/deepseek-r1-distill-qwen-32b`
- **是否需扩展共享层**：是（base URL 需支持 account_id 模板插值，建议共享层支持路径变量）

#### 4. 风险与限制

- 与 `cloudflare_workers_ai` 实质重复，建议合并实现，避免两条配置。
- inventory base_url 不完整，须以 account 作用域路径为准。

#### 5. 优先级建议

- **优先级**：P2（后续）
- **理由**：与 `cloudflare_workers_ai` 重复，统一在后者实现即可；本条目作为别名/前缀映射处理。

---

### cloudflare_workers_ai — Cloudflare Workers AI

- **canonical ID**：cloudflare_workers_ai
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://developers.cloudflare.com/workers-ai/configuration/open-ai-compatibility/ ；https://developers.cloudflare.com/workers-ai/get-started/rest-api/
- **核验来源**：官方 API 文档
- **证据强度**：强（官方文档给出 cURL 与 SDK 示例，明确请求响应结构）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.cloudflare.com/client/v4/accounts/{CLOUDFLARE_ACCOUNT_ID}/ai/v1`
- **鉴权**：方式=Bearer API Token / 环境变量=CLOUDFLARE_API_KEY + CLOUDFLARE_ACCOUNT_ID / 是否必需=是
- **endpoint 公式**：`POST {base}/chat/completions`、`POST {base}/embeddings`（另有原生 `POST .../ai/run/{model}` 端点，非 OpenAI 结构）
- **协议类型**：OpenAI 兼容（同时存在原生 `/ai/run/{model}` 端点，但 OpenAI 兼容端点可直接复用 OpenAI SDK）
- **请求结构要点**：标准 OpenAI Chat Completions（model 用 `@cf/...` 形式，如 `@cf/meta/llama-3.1-8b-instruct`）
- **响应结构要点**：标准 OpenAI 响应（choices[].message.content）
- **流式**：SSE（OpenAI 兼容）
- **错误结构**：与 OpenAI 共享结构一致（OpenAI 兼容端点）
- **特有行为**：base URL 含 account_id；原生 `/ai/run` 返回 `{result:{response:...}, success, errors, messages}` 结构，与本兼容端点不同

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方文档明确 OpenAI 兼容端点可直接用 openai SDK，仅换 base URL 与 model
- **可复用模型 ID 样例**：`@cf/meta/llama-3.1-8b-instruct`、`@cf/deepseek-ai/deepseek-r1-distill-qwen-32b`、`@cf/openai/gpt-oss-120b`
- **是否需扩展共享层**：是（base URL 需 account_id 模板插值）

#### 4. 风险与限制

- base URL 为账户作用域，须支持 `CLOUDFLARE_ACCOUNT_ID` 路径变量。
- 同时提供原生 `/ai/run` 端点，需确保实现走 OpenAI 兼容端点而非原生端点。

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：OpenAI 兼容薄封装，官方文档充分；建议作为 `cloudflare` / `cloudflare_workers_ai` 的统一实现。

---

### cohere_chat — Cohere Chat

- **canonical ID**：cohere_chat
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.5
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://docs.cohere.com/reference/chat ；https://docs.cohere.com/v2/docs/chat-api
- **核验来源**：官方 API 文档（Cohere API v2 reference）
- **证据强度**：强（官方 reference 给出完整请求/响应/错误结构）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.cohere.com`（v2 端点 `POST /v2/chat`）
- **鉴权**：方式=Bearer token / 环境变量=COHERE_API_KEY（inventory 未给，按官方惯例）/ 是否必需=是
- **endpoint 公式**：`POST https://api.cohere.com/v2/chat`
- **协议类型**：原生（非 OpenAI 兼容）
- **请求结构要点**：`messages` 数组（role: user/assistant/tool/system），含 `stream`、`model`、`tools`、`documents`、`citation_options`、`response_format`、`safety_mode`、`max_tokens`、`stop_sequences`、`temperature`、`frequency_penalty`、`presence_penalty`、`k`、`p`、`seed`、`thinking`、`tool_choice` 等特有/差异字段
- **响应结构要点**：返回 `{id, finish_reason, message, usage, logprobs}`，**无 OpenAI 的 `choices` 数组**；`finish_reason` 取值不同（complete / max_tokens / stop_sequence / tool_call / error / timeout）
- **流式**：SSE（Cohere 自有事件结构，与 OpenAI SSE 不一致）
- **错误结构**：厂商专属（HTTP 状态码 + Cohere 错误体，含 498 Invalid Token / 499 Client Closed Request 等特有码）
- **特有行为**：`documents`/`citation_options` 引用增强、`safety_mode`、`k`/`p` 采样参数、`thinking` 推理配置

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：原生
- **依据**：请求/响应/流式状态机/错误码与 OpenAI Chat Completions 存在结构性差异（无 choices、finish_reason 枚举不同、SSE 事件结构不同、特有 documents/citation/safety 字段）
- **可复用模型 ID 样例**：`command-a-03-2025`、`command-r`、`command-r-plus`、`command-r-08-2024`
- **是否需扩展共享层**：否（需独立原生实现）

#### 4. 风险与限制

- v1 与 v2 协议差异大，需明确以 v2 为准；v1 接口（message 字符串而非 messages 数组）已不推荐。
- 流式事件结构需独立解析。

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：知名厂商、用户量大，但为原生协议，实现成本高于薄封装，需独立开发与测试。

---

### cortecs — Cortecs

- **canonical ID**：cortecs
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://docs.cortecs.ai/quickstart ；https://docs.cortecs.ai/usage/advanced-usage
- **核验来源**：官方 API 文档
- **证据强度**：强（官方 quickstart 给出 OpenAI SDK 示例与 base URL）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.cortecs.ai/v1/`
- **鉴权**：方式=Bearer API Key / 环境变量=CORTECS_API_KEY / 是否必需=是
- **endpoint 公式**：`POST {base}/chat/completions`（OpenAI 兼容）
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions；路由器专属参数通过 `extra_body` 传入：`preference`（speed / cost / balanced）、`allowed_providers`
- **响应结构要点**：标准 OpenAI 响应
- **流式**：SSE（OpenAI 兼容）
- **错误结构**：与 OpenAI 共享结构一致（OpenAI 兼容端点）
- **特有行为**：欧洲 LLM 路由器，按 speed/cost/balanced 路由；同时提供 Anthropic 兼容端点；模型样例为 `claude-*` 系列

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装（可选择性扩展共享层以原生支持 `preference`/`allowed_providers`）
- **依据**：核心协议为标准 OpenAI Chat Completions；`preference` 等为可选路由参数，经 `extra_body` 透传即可，基础薄封装即可工作
- **可复用模型 ID 样例**：`claude-4-5-sonnet`、`claude-opus4-5`、`claude-haiku-4-5`
- **是否需扩展共享层**：否（基础场景）；若要一等公民支持路由偏好可扩展共享层（有限差异）

#### 4. 风险与限制

- 模型样例均为 `claude-*`，需确认是否为 Claude 系列经由 OpenAI 兼容封装（注意 vision/tool 等能力的兼容性差异）。
- 路由偏好字段为厂商扩展，需文档化。

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：OpenAI 兼容薄封装，官方文档充分；欧洲合规路由有差异化价值。

---

### crof — CrofAI

- **canonical ID**：crof
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://crof.ai/docs ；https://github.com/nahcrof-code/crofAI （官方仓库 README）
- **核验来源**：官方 API 文档 + 官方 GitHub 仓库
- **证据强度**：强（官方 README 给出完整 OpenAI SDK 示例、端点与支持参数）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://crof.ai/v1`（亦提供 `https://crof.ai/v2`，等价）
- **鉴权**：方式=Bearer API Key / 环境变量=CROF_API_KEY / 是否必需=是
- **endpoint 公式**：`POST {base}/chat/completions`、`GET {base}/models`
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions；支持 `max_tokens`、`temperature`、`top_p`、`stop`、`seed`、`tools`、`stream`；支持 vision（image_url）、reasoning（`delta.reasoning_content`）
- **响应结构要点**：标准 OpenAI 响应（choices[].message / delta）
- **流式**：SSE（OpenAI 兼容，含 reasoning_content 增量）
- **错误结构**：与 OpenAI 共享结构一致（OpenAI 兼容）
- **特有行为**：`/v1/models` 返回标准模型列表（含 context_length、pricing、quantization、speed 等扩展元数据）；另提供 Anthropic 端点 `https://anthropic.nahcrof.com/v1/messages`

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方明确"supports the OpenAI SDK"，端点结构与参数均为 OpenAI Chat Completions
- **可复用模型 ID 样例**：`deepseek-v3.2`、`deepseek-v4-pro`、`gemma-4-31b-it`、`kimi-k2.5`
- **是否需扩展共享层**：否

#### 4. 风险与限制

- `reasoning_content` 为 DeepSeek 风格扩展字段，需确认共享层是否已支持透传。
- `/v2` 与 `/v1` 等价但并存，建议统一用 `/v1`。

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：OpenAI 兼容薄封装，官方仓库示例充分，实现成本低。

---

### crossmodel — CrossModel

- **canonical ID**：crossmodel
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://www.crossmodel.ai/docs ；https://www.crossmodel.ai/docs/api-reference/chat
- **核验来源**：官方 API 文档
- **证据强度**：强（官方 reference 给出端点、鉴权、请求参数表与 cURL 示例）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.crossmodel.ai/v1`
- **鉴权**：方式=Bearer（OpenAI 兼容端点用 `Authorization: Bearer`，key 以 `cm-` 开头；Anthropic 兼容端点可用 `x-api-key`）/ 环境变量=CROSSMODEL_API_KEY / 是否必需=是
- **endpoint 公式**：`POST {base}/chat/completions`、`POST {base}/responses`、`POST {base}/messages`（Anthropic 兼容）、`GET {base}/models`
- **协议类型**：OpenAI 兼容（同时提供 OpenAI Responses 风格与 Anthropic Messages 风格）
- **请求结构要点**：标准 OpenAI Chat Completions；模型 ID 用 `vendor/model` 形式（如 `deepseek/deepseek-v4-pro`、`anthropic/claude-sonnet-4.6`）；扩展参数 `reasoning_effort`、`safety_identifier`
- **响应结构要点**：标准 OpenAI 响应；成功响应含 `x-request-id` 头
- **流式**：SSE（OpenAI 兼容，`stream:true`）
- **错误结构**：与 OpenAI 共享结构一致（含 `429 rate_limit_error`）；建议对 429/502/503 指数退避
- **特有行为**：多协议网关；按 token 计费，余额耗尽拒绝请求；RPM/TPM 按 key 限流

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方明确"fully compatible with OpenAI Chat Completions"，仅需换 base URL 与 model ID
- **可复用模型 ID 样例**：`anthropic/claude-haiku-4-5`、`anthropic/claude-opus-5`、`deepseek/deepseek-v4-pro`
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 模型 ID 为 `vendor/model` 形式，需原样透传。
- `reasoning_effort` 取值为厂商枚举（none/minimal/low/medium/high/xhigh），非法值报错。

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：OpenAI 兼容薄封装，官方文档充分，多模型聚合有实用性。

---

### crusoe — Crusoe

- **canonical ID**：crusoe
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://docs.crusoecloud.com/serverless-inference/overview ；https://docs.crusoecloud.com/quickstart/getting-started-with-serverless-inference
- **核验来源**：官方 API 文档
- **证据强度**：强（官方文档明确"OpenAI-API compatible endpoint at api.inference.crusoecloud.com"）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.inference.crusoecloud.com/v1`（inventory `base_urls` 为空，需补全；官方文档仅给主机 `api.inference.crusoecloud.com`，按 OpenAI 兼容惯例补 `/v1`）
- **鉴权**：方式=Bearer API Key / 环境变量=未知（inventory 未给，按 Crusoe 惯例 `CRUSOE_API_KEY`，需实测确认）/ 是否必需=是
- **endpoint 公式**：`POST {base}/chat/completions`（OpenAI 兼容）
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions；模型 ID 形如 `deepseek-ai/DeepSeek-V3-0324`、`meta-llama/Llama-3.3-70B-Instruct`（inventory 带 `crusoe/` 前缀）
- **响应结构要点**：标准 OpenAI 响应
- **流式**：SSE（OpenAI 兼容，推断）
- **错误结构**：与 OpenAI 共享结构一致（OpenAI 兼容，推断）
- **特有行为**：Crusoe Intelligence Foundry Serverless Inference，MemoryAlloy 缓存路由；模型样例含 `crusoe/` 前缀（litellm 惯例）

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方文档明确 OpenAI-API 兼容端点
- **可复用模型 ID 样例**：`deepseek-ai/DeepSeek-V3-0324`、`meta-llama/Llama-3.3-70B-Instruct`、`qwen/Qwen3-235B-A22B`
- **是否需扩展共享层**：否

#### 4. 风险与限制

- inventory `base_urls` 为空且无 env var，需以官方文档补全 base URL 与鉴权变量后实测。
- 模型 ID 是否需剥离 `crusoe/` 前缀需确认。

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：OpenAI 兼容薄封装，官方文档明确；需补全 base URL/鉴权字段后即可接入。

---

### custom_provider — custom provider

- **canonical ID**：custom_provider
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：无（inventory 无 documentation_urls / base_urls / api_key_env_vars）
- **核验来源**：仅第三方（tokenhub 源），无可识别的官方提供方
- **证据强度**：无（无可识别 provider，无 base URL，无文档）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：未知
- **鉴权**：方式=未知 / 环境变量=未知 / 是否必需=未知
- **endpoint 公式**：未知
- **协议类型**：未知
- **请求结构要点**：未知
- **响应结构要点**：未知
- **流式**：未知
- **错误结构**：未知
- **特有行为**：模型样例（MiniMax-M2.5、claude-haiku-4-5 等）疑似聚合网关，但无任何可定位的端点或文档

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：待定
- **依据**：无任何官方协议证据，无法判定实现路径
- **可复用模型 ID 样例**：无
- **是否需扩展共享层**：未知

#### 4. 风险与限制

- 条目疑似"自定义 OpenAI 兼容端点"占位（litellm/注册表中常见的 generic custom 概念），并非一个可实现的具名 provider。
- 在获得可识别的 base URL 与官方文档前无法实现。

#### 5. 优先级建议

- **优先级**：搁置（证据不足或无价值）
- **理由**：无可识别 provider、无 base URL、无文档，证据强度为"无"；待 inventory 补全身份信息后再评估。

---

### daoxe — DaoXE

- **canonical ID**：daoxe
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://daoxe.com/ ；https://docs.privategpt.dev/providers/daoxe ；https://models.dev/providers/daoxe
- **核验来源**：官方站点 + 多来源一致（PrivateGPT 集成文档、models.dev 目录）
- **证据强度**：强（官方站点声明 OpenAI 兼容端点，多来源一致确认请求响应结构）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://daoxe.com/v1`
- **鉴权**：方式=Bearer API Key / 环境变量=DAOXE_API_KEY / 是否必需=是
- **endpoint 公式**：`POST {base}/chat/completions`、`GET {base}/models`、`POST {base}/embeddings`
- **协议类型**：OpenAI 兼容（多协议网关，另支持 Anthropic Messages 等协议）
- **请求结构要点**：标准 OpenAI Chat Completions；支持 tool/function calling、structured output、streaming、vision（均 model-dependent）；模型 ID 取决于账户套餐
- **响应结构要点**：标准 OpenAI 响应
- **流式**：SSE（OpenAI 兼容）
- **错误结构**：与 OpenAI 共享结构一致（OpenAI 兼容）
- **特有行为**：多模型聚合（GPT、Claude、Grok、GLM 等）；不暴露 `/tokenize`；中国大陆不可用

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方与多来源一致确认 OpenAI 兼容端点，可直接用 OpenAI SDK 换 base URL
- **可复用模型 ID 样例**：`claude-sonnet-4-20250514`、`gpt-4o`、`grok-3`（取决于账户套餐）
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 模型 ID 依赖账户套餐，需动态获取 `/v1/models`。
- 中国大陆不可用，影响部分用户。

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：OpenAI 兼容薄封装，多来源证据一致，实现成本低。

---

### dify — Dify

- **canonical ID**：dify
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://docs.dify.ai/en/api-reference/guides/get-started ；https://docs.dify.ai/en/api-reference/guides/chat
- **核验来源**：官方 API 文档
- **证据强度**：强（官方文档给出完整端点族与鉴权方式）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.dify.ai/v1`（Dify Cloud；自托管用实例自身 base URL）
- **鉴权**：方式=Bearer token（app 作用域 API Key）/ 环境变量=DIFY_API_KEY（按官方惯例）/ 是否必需=是
- **endpoint 公式**：`POST {base}/chat-messages`、`POST {base}/completion-messages`、`POST {base}/workflows/run`、`GET {base}/info` 等
- **协议类型**：原生（应用编排平台 API，非 OpenAI 兼容）
- **请求结构要点**：以"应用"为单位调用，请求含 `query`/`inputs`/`user`/`conversation_id`/`response_mode` 等；非 OpenAI 的 messages 数组结构
- **响应结构要点**：返回 `answer`/`conversation_id`/`message_id` 等，结构围绕会话与工作流编排，无 OpenAI `choices`
- **流式**：SSE（Dify 自有事件结构，`response_mode: streaming`）
- **错误结构**：厂商专属
- **特有行为**：每个 app 一个 API Key；需 `user` 标识终端用户；区分 chatbot/agent/chatflow/workflow/completion 等应用类型

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：原生
- **依据**：协议围绕"应用/会话/工作流"组织，请求响应结构与 OpenAI Chat Completions 存在结构性差异，无法由共享层表达
- **可复用模型 ID 样例**：无（Dify 不以模型 ID 暴露，而是以 app 为单位）
- **是否需扩展共享层**：否（需独立原生实现）

#### 4. 风险与限制

- Dify 本质是应用编排平台而非模型推理 API，与 aimux 作为 LLM provider 适配库的定位不符。
- 调用以 app 为粒度，无法直接映射为"模型 × chat completions"。

#### 5. 优先级建议

- **优先级**：搁置（证据不足或无价值）
- **理由**：协议证据强但定位为应用编排平台，非模型 provider；与 provider 适配器目标不匹配，价值低。

---

### dinference — DInference

- **canonical ID**：dinference
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://dinference.com/docs ；https://models.dev/providers/dinference
- **核验来源**：官方 API 文档 + models.dev 目录
- **证据强度**：强（官方文档声明 OpenAI 兼容，models.dev 列出 base URL 与模型清单）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.dinference.com/v1`
- **鉴权**：方式=Bearer API Key / 环境变量=DINFERENCE_API_KEY / 是否必需=是
- **endpoint 公式**：`POST {base}/chat/completions`、`GET {base}/models`（OpenAI 兼容）
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions；官方声明"compatible with OpenAI API standards"
- **响应结构要点**：标准 OpenAI 响应
- **流式**：SSE（OpenAI 兼容，推断）
- **错误结构**：与 OpenAI 共享结构一致（OpenAI 兼容，推断）
- **特有行为**：主推 GLM 系列（glm-4.7/5/5.1/5.2）、gpt-oss-120b、minimax-m2.5

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方文档明确 OpenAI API 兼容标准
- **可复用模型 ID 样例**：`glm-5`、`glm-5.1`、`glm-5.2`、`gpt-oss-120b`、`minimax-m2.5`
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 官方文档页面较简略，流式与错误结构为推断，建议实测兜底。
- models.dev 标注包为 `@ai-sdk/openai-compatible`，与判定一致。

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：OpenAI 兼容薄封装，官方声明 + 目录证据一致。

---

### doubao_video — DoubaoVideo

- **canonical ID**：doubao_video
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：chat（注：inventory 标 chat，但 id/display_name 指向视频生成，存在矛盾）

#### 1. 官方协议证据

- **文档 URL**：https://www.volcengine.com/docs/82379/1520757 ；https://www.volcengine.com/docs/82379/1521309 ；https://www.volcengine.com/docs/82379/2298881
- **核验来源**：官方 API 文档（火山引擎火山方舟 Seedance / Doubao Seedance 视频生成）
- **证据强度**：强（官方文档明确视频生成任务为"创建任务 + 查询任务"异步流程）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://ark.cn-beijing.volces.com`（火山方舟主机；视频任务端点位于该主机下的内容生成路径，非 OpenAI 兼容的 `/api/v3/chat/completions`）
- **鉴权**：方式=Bearer API Key（火山方舟 API Key）/ 环境变量=未知（inventory 未给，按方舟惯例 `ARK_API_KEY`/`VOLC_API_KEY`，需确认）/ 是否必需=是
- **endpoint 公式**：创建视频生成任务 `POST .../contents/generations/tasks`（具体路径以官方为准）；查询任务 `GET .../contents/generations/tasks/{task_id}`
- **协议类型**：原生（异步任务式内容生成，非 OpenAI 兼容）
- **请求结构要点**：基于图片/文本输入生成视频；请求为任务参数对象，非 OpenAI messages 数组
- **响应结构要点**：返回任务 `id` 与状态，需轮询查询任务获取结果（视频 URL），无 OpenAI `choices`
- **流式**：无（异步任务，轮询查询；非流式推理）
- **错误结构**：厂商专属（火山方舟错误码体系）
- **特有行为**：两步异步（创建任务 → 查询任务取结果）；Doubao Seedance / Seedance 2.0 系列视频模型

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：模态专用
- **依据**：视频生成为单一模态的异步任务式原生协议，与 OpenAI Chat Completions 结构性不同；方舟主机虽另提供 OpenAI 兼容的文本 chat 端点，但本条目 id 明确指向视频生成
- **可复用模型 ID 样例**：Doubao Seedance / Seedance 2.0 系列（inventory `model_sample` 为空）
- **是否需扩展共享层**：否（需独立的视频生成模态实现）

#### 4. 风险与限制

- inventory 元数据内部矛盾：`capabilities` 标 chat，但 `id`/`display_name` 为 video，`model_count=0`；需澄清本条目究竟指视频生成还是方舟文本 chat。
- 视频生成为非 chat 模态，超出当前 chat 适配范围。
- 具体任务端点路径需以官方文档实测确认。

#### 5. 优先级建议

- **优先级**：搁置（证据不足或无价值）
- **理由**：协议证据强但属视频生成模态（非 chat），且 inventory 分类自相矛盾；超出 chat provider 适配范围，待定位澄清与视频模态规划后再评估。

---

### drun — D.Run (China)

- **canonical ID**：drun
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://docs.d.run/models/api-call.html ；https://www.d.run
- **核验来源**：官方 API 文档（d.run / DaoCloud Runs Intelligence）
- **证据强度**：强（官方文档给出 OpenAI SDK 调用示例与 endpoint）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://chat.d.run/v1`（MaaS by Token）；独立模型服务为 `<region>.d.run`
- **鉴权**：方式=Bearer API Key / 环境变量=DRUN_API_KEY / 是否必需=是
- **endpoint 公式**：`POST {base}/chat/completions`（OpenAI 兼容）
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions；官方示例直接用 openai SDK；模型名形如 `public/deepseek-r1`、`public/deepseek-v3`、`public/minimax-m25`
- **响应结构要点**：标准 OpenAI 响应（choices[].message.content）
- **流式**：SSE（OpenAI 兼容，推断）
- **错误结构**：与 OpenAI 共享结构一致（OpenAI 兼容，推断）
- **特有行为**：DaoCloud 旗下智算中心平台；MaaS by Token（共享、按 token 计费）与独立模型服务（独享实例）两种托管方式；模型名带 `public/` 前缀

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方文档明确用 openai SDK 调用，endpoint 与请求响应均为 OpenAI Chat Completions
- **可复用模型 ID 样例**：`public/deepseek-r1`、`public/deepseek-v3`、`public/minimax-m25`
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 官方示例使用旧版 openai SDK 写法（`openai.ChatCompletion.create`），但端点本身为 OpenAI 兼容；流式/错误结构为推断，建议实测。
- 模型名带 `public/` 前缀，需原样透传。

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：OpenAI 兼容薄封装，官方文档明确；中国区智算平台有本土化价值。
