# 第 6 批调研记录（14 个 provider）

> 调研日期：2026-07-28。按 canonical id 字母序排列。inventory 的 tier/protocol/openai_compatible 字段仅为自动推断线索，本批均以官方文档/多来源交叉核验为准；无法确认的字段写"未知"或留空，未臆造协议细节。

---

### ebcloud — EBCloud

- **canonical ID**：ebcloud
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://docs.ebtech.com/ai/model-api.html
- **核验来源**：官方 API 文档
- **证据强度**：中（官方文档明确声明兼容 OpenAI API 格式，但具体请求/响应 JSON 在控制台模型接入页展示，未直接抓取到）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://maas-api.ebcloud.com/v1
- **鉴权**：方式=Bearer API Key / 环境变量=EBCLOUD_API_KEY / 是否必需=是
- **endpoint 公式**：POST /v1/chat/completions（OpenAI 兼容，推断）
- **协议类型**：OpenAI 兼容
- **请求结构要点**：官方文档明确"兼容 OpenAI API 格式，通过一个 Key 即可实现多模型调用"；控制台模型 API 接入页提供 OpenAI SDK / Curl / Requests 示例（未直接抓取 JSON 请求体）
- **响应结构要点**：未知（按 OpenAI Chat Completions 响应结构推断）
- **流式**：未知（OpenAI 兼容通常支持 SSE stream 参数，文档未直接确认）
- **错误结构**：未知
- **特有行为**：一个 Key 多模型调用；覆盖 LLM/文生图/文生视频；按 token 后付费（算力点）

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方文档声明兼容 OpenAI API 格式并提供 OpenAI SDK 调用示例
- **可复用模型 ID 样例**：DeepSeek-V4-Flash、DeepSeek-V4-Pro、GLM-5.1、Kimi-K2.6
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 官方文档为高层声明，未直接展示请求/响应 JSON；实际请求体/流式/错误结构需在控制台示例或实测确认
- 多模态（文生图/文生视频）超出 chat 能力范围，本次仅覆盖 chat

#### 5. 优先级建议

- **优先级**：P1
- **理由**：官方明确 OpenAI 兼容，薄封装成本低；建议补充请求/响应证据后落实

---

### empiriolabs — EmpirioLabs AI

- **canonical ID**：empiriolabs
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://docs.empiriolabs.ai/、https://docs.empiriolabs.ai/compatibility、https://docs.empiriolabs.ai/authentication
- **核验来源**：官方 API 文档
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://api.empiriolabs.ai/v1
- **鉴权**：方式=Bearer Token（`Authorization: Bearer sk-empiriolabs-...`）/ 环境变量=EMPIRIOLABS_API_KEY / 是否必需=是
- **endpoint 公式**：POST /v1/chat/completions（OpenAI 兼容）；另提供 POST /v1/responses（OpenAI Responses）、POST /v1/messages（Anthropic）、POST /v1beta/models/{model}:generateContent（Gemini）、GET /v1/models
- **协议类型**：OpenAI 兼容（同时兼容 Anthropic Messages 与 Gemini 格式）
- **请求结构要点**：与 OpenAI Chat Completions 相同（messages/model/stream/temperature/max_tokens 等）；支持 response_format（json_object / json_schema）；未提供 system/developer 消息时自动前置默认 system prompt，提供则完全替换
- **响应结构要点**：与 OpenAI Chat Completions 一致
- **流式**：SSE（标准 OpenAI stream 参数；Gemini 端点 streamGenerateContent 无 [DONE] 哨兵）
- **错误结构**：厂商专属（401 鉴权失败 / 402 余额不足 / 429 限流，详见 errors 文档）
- **特有行为**：API key 前缀 `sk-empiriolabs-`；每账户最多 50 个 key；默认 50 RPM / 2,000,000 TPM；结构化输出按模型支持

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方文档完整展示 OpenAI 兼容 chat/completions 请求与响应，并提供 OpenAI SDK 配置
- **可复用模型 ID 样例**：deepseek-v4-pro、qwen3-7-max、glm-4-5-flash、seed-2-0-pro、mistral-medium-3-1
- **是否需扩展共享层**：否（若需利用 402 余额不足语义可选择性扩展错误映射）

#### 4. 风险与限制

- 多格式端点（OpenAI/Anthropic/Gemini）需明确只对接 OpenAI chat/completions
- 错误码 402 为厂商特有，需在共享层错误处理中考虑

#### 5. 优先级建议

- **优先级**：P1
- **理由**：证据强、协议标准 OpenAI 兼容，薄封装即可；140+ 模型聚合价值高

---

### firepass — Fireworks (Firepass)

- **canonical ID**：firepass
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://docs.fireworks.ai/firepass
- **核验来源**：官方 API 文档
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://api.fireworks.ai/inference/v1（OpenAI 兼容）；Anthropic 兼容 base URL：https://api.fireworks.ai/inference
- **鉴权**：方式=Bearer API Key（Fire Pass 专用 key，前缀 `fpk_...`）/ 环境变量=Fireworks 通用 FIREWORKS_API_KEY（inventory 未给出）/ 是否必需=是
- **endpoint 公式**：POST /v1/chat/completions（OpenAI 兼容）；Anthropic 兼容 POST /v1/messages
- **协议类型**：OpenAI 兼容（同时 Anthropic 兼容）
- **请求结构要点**：标准 OpenAI Chat Completions；model 需用专用 router ID（如 `accounts/fireworks/routers/kimi-k3-fast`）
- **响应结构要点**：与 OpenAI 一致
- **流式**：SSE（标准 OpenAI stream，按 OpenAI 兼容推断，文档未单独说明）
- **错误结构**：与 OpenAI 共享结构一致（推断）
- **特有行为**：Fire Pass 是 Fireworks 的订阅通行证产品，仅对 included 开源模型零 per-token 计费；需专用 Fire Pass key（`fpk_...`）；仅供非生产编码用途；router ID 固定

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方文档明确 OpenAI 兼容 base URL 与 router model ID，本质为 Fireworks API 的计费变体
- **可复用模型 ID 样例**：accounts/fireworks/routers/kimi-k3-fast、accounts/fireworks/routers/kimi-k2p6-turbo
- **是否需扩展共享层**：否

#### 4. 风险与限制

- Fire Pass 是实验性产品，特性/可用性/定价可能变化
- 仅限 included 开源模型且仅供非生产编码用途；专用 `fpk_` key 与普通 Fireworks key 不同
- 与已有/可能的 fireworks provider 高度重叠，建议复用同一 Fireworks 适配器并区分 key/计费

#### 5. 优先级建议

- **优先级**：P2
- **理由**：协议明确为 OpenAI 兼容薄封装，但本质是 Fireworks 的计费通行证，价值与 Fireworks 适配器重叠；建议复用 Fireworks 实现

---

### freemodel — FreeModel

- **canonical ID**：freemodel
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://freemodel.dev/（官方站点，SPA，meta 描述与示例可读；完整 API reference 未抓取到）
- **核验来源**：官方站点 + 第三方聚合（Mastra）
- **证据强度**：中
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://cc.freemodel.dev/v1（inventory 给出；官方站点示例使用 `client.chat.completions.create` 并提示"swap your base URL to FreeModel"）
- **鉴权**：方式=Bearer API Key / 环境变量=FREEMODEL_API_KEY / 是否必需=是
- **endpoint 公式**：POST /v1/chat/completions（OpenAI 兼容）；另支持 /v1/responses（OpenAI Responses）与 /v1/messages（Anthropic）
- **协议类型**：OpenAI 兼容（同时兼容 Anthropic）
- **请求结构要点**：官方描述"routes each request to the best open model behind a single endpoint — compatible with both the OpenAI (v1/responses) and Anthropic (v1/messages) formats"；标准 OpenAI 请求结构
- **响应结构要点**：未知（按 OpenAI 推断）
- **流式**：未知
- **错误结构**：未知
- **特有行为**：多模型智能路由（单 endpoint 路由到最优开源模型）；第三方 issue 提及可能按用户层级提供多个 API 连接点

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方站点与 Mastra 均确认 OpenAI 兼容 /chat/completions
- **可复用模型 ID 样例**：claude-fable-5、claude-haiku-4-5-20251001、claude-opus-4-6、claude-opus-4-7、claude-opus-4-8
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 官方文档为 SPA，未能抓取完整请求/响应/错误结构，需补充官方 API reference
- 模型样例含 claude-* 命名，疑似路由到 Anthropic 系模型，需确认实际可用模型
- 多连接点/用户层级差异需确认

#### 5. 优先级建议

- **优先级**：P2
- **理由**：协议疑为 OpenAI 兼容薄封装，但证据仅中等；待补充官方 API reference 后可升 P1

---

### frogbot — FrogBot

- **canonical ID**：frogbot
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://docs.frogbot.ai/api-reference
- **核验来源**：官方 API 文档
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://app.frogbot.ai/api/v1
- **鉴权**：方式=Bearer API Key / 环境变量=FROGBOT_API_KEY / 是否必需=是
- **endpoint 公式**：POST /v1/chat/completions（OpenAI 兼容）；POST /v1/messages（Anthropic 兼容）；GET /v1/models；另 /v1/embeddings、/v1/rerank、/v1/audio/transcriptions、/v1/audio/speech、/v1/images/generations、GET /v1/quota
- **协议类型**：OpenAI 兼容（同时 Anthropic 兼容，多模态）
- **请求结构要点**：标准 OpenAI Chat Completions，支持 streaming、tool calling、MCP
- **响应结构要点**：与 OpenAI 一致
- **流式**：SSE（标准 OpenAI stream）
- **错误结构**：未知（推断与 OpenAI 一致）
- **特有行为**：提供 /v1/quota 查询 5 小时滚动计费窗口用量；同时提供 embedding/rerank/audio/image 等多模态能力

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装（chat 能力）；其他模态按需模态专用
- **依据**：官方 API 参考明确 OpenAI 兼容 /chat/completions
- **可复用模型 ID 样例**：claude-haiku-4-5、claude-opus-4-6、claude-opus-4-7、claude-sonnet-4-6、deepseek-v4-pro
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 注意：docs.frogbot.ai 首页描述的是 FrogBot 开源 agent 框架，与 app.frogbot.ai/api/v1 的推理 API 是同一品牌不同产品，勿混淆
- 多模态端点（embedding/rerank/audio/image）超出 chat 范围

#### 5. 优先级建议

- **优先级**：P1
- **理由**：官方 API 参考清晰，OpenAI 兼容薄封装成本低

---

### gitlab — GitLab Duo

- **canonical ID**：gitlab
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://docs.gitlab.com/user/duo_agent_platform、https://docs.gitlab.com/api/chat/
- **核验来源**：官方 API 文档
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：无公开直接模型 API base URL（依赖 GitLab 实例 / AI Gateway）
- **鉴权**：方式=GitLab 会话/OAuth（非 API Key Bearer）；Self-Hosted 走 AI Gateway / 是否必需=是
- **endpoint 公式**：无公开 OpenAI 兼容 chat/completions 端点；Duo Chat completions API（/api/chat）官方明确"internal use only, must be a GitLab team member"
- **协议类型**：原生/不适用（非公开模型 API）
- **请求结构要点**：无公开对外请求契约
- **响应结构要点**：未知
- **流式**：未知
- **错误结构**：未知
- **特有行为**：GitLab Duo 是嵌入 GitLab 产品（IDE 扩展、Agent Platform、flows）的 AI 平台，按 GitLab Credits 计费；不提供独立公开 LLM 推理 API；模型样例 duo-chat-* 为内部路由模型

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：待定（无公开协议可对接）
- **依据**：官方文档确认 Duo Chat completions API 仅供内部使用，无对外 OpenAI 兼容端点
- **可复用模型 ID 样例**：duo-chat-fable-5、duo-chat-gpt-5-1、duo-chat-gpt-5-2（内部模型，不可直接对外调用）
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 不存在公开可直接调用的模型 API；强行对接需绕过 GitLab 产品边界，不符合典型 provider 适配范围
- Self-Hosted 需部署 AI Gateway，属私有部署场景

#### 5. 优先级建议

- **优先级**：搁置
- **理由**：官方明确无对外公开模型 API，无法作为标准 provider 适配

---

### gmi — GMI

- **canonical ID**：gmi
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：无（inventory 未提供；交叉引用 GMI Cloud 官方文档 https://docs.gmicloud.ai/inference-engine/api-reference/llm-api-reference 与 litellm）
- **核验来源**：仅第三方（litellm）+ 同一 provider 的 gmicloud 官方文档交叉印证
- **证据强度**：中
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://api.gmi-serving.com/v1（与 gmicloud 同一 provider，未对 "gmi" 条目单独确认）
- **鉴权**：方式=Bearer API Key / 环境变量=litellm 用 GMI_API_KEY / 是否必需=是
- **endpoint 公式**：POST /v1/chat/completions（OpenAI 兼容，依 gmicloud 官方文档）
- **协议类型**：OpenAI 兼容
- **请求结构要点**：与 gmicloud 一致（OpenAI Chat Completions；支持 tools/max_tokens/temperature/top_p/stream/response_format 等）
- **响应结构要点**：与 OpenAI 一致（id/object/choices/usage）
- **流式**：SSE（stream 参数）
- **错误结构**：与 OpenAI 共享结构一致（推断）
- **特有行为**：inventory 模型样例带 `gmi/` 前缀（gmi/anthropic/claude-opus-4），该前缀命名约定未由官方文档确认

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装（与 gmicloud 复用）
- **依据**：经交叉印证 gmi 即 GMI Cloud，OpenAI 兼容
- **可复用模型 ID 样例**：gmi/anthropic/claude-opus-4、gmi/Qwen/Qwen3-VL-235B-A22B-Instruct-FP8、gmi/MiniMaxAI/MiniMax-M2.1（前缀待确认）
- **是否需扩展共享层**：否

#### 4. 风险与限制

- "gmi" 与 "gmicloud" 疑似同一 provider 重复条目，建议去重合并
- `gmi/` 模型 ID 前缀未由官方确认，实际可能为不带前缀的 anthropic/claude-opus-4 形式

#### 5. 优先级建议

- **优先级**：P2
- **理由**：疑似 gmicloud 重复条目，先去重；证据中等

---

### gmicloud — GMI Cloud

- **canonical ID**：gmicloud
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://docs.gmicloud.ai/inference-engine/api-reference/llm-api-reference
- **核验来源**：官方 API 文档
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://api.gmi-serving.com/v1
- **鉴权**：方式=Bearer（`Authorization: Bearer GMI_API_KEY`）/ 环境变量=GMI_API_KEY（inventory 标 GMICLOUD_API_KEY，官方文档用 GMI_API_KEY）/ 是否必需=是；多组织可加 `X-Organization-ID`
- **endpoint 公式**：GET /v1/models；POST /v1/chat/completions
- **协议类型**：OpenAI 兼容
- **请求结构要点**：model/messages 必需；可选 tools/max_tokens(默认2000)/temperature(默认1)/top_p/top_k/stop(最多4)/response_format/stream/ignore_eos/context_length_exceeded_behavior(truncate|error)；支持文本/图像/音频
- **响应结构要点**：id/object(chat.completion)/created/model/choices/usage(prompt/completion/total_tokens)，与 OpenAI 一致
- **流式**：SSE（stream 参数；流式最终 chunk 含 usage）
- **错误结构**：与 OpenAI 共享结构一致（推断）
- **特有行为**：context_length_exceeded_behavior 默认 truncate（与其他 provider 默认 error 不同）；response_format json_object；建议新项目用 Responses 格式

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方 API reference 完整展示 OpenAI 兼容请求/响应
- **可复用模型 ID 样例**：deepseek-ai/DeepSeek-R1、anthropic/claude-opus-4.6、Qwen/Qwen3.7-Max
- **是否需扩展共享层**：否（context_length_exceeded_behavior 为可选字段，可不传）

#### 4. 风险与限制

- 环境变量名官方为 GMI_API_KEY，与 inventory 的 GMICLOUD_API_KEY 不一致，需以官方为准
- 默认 truncate 上下文行为与多数 provider 不同，需注意

#### 5. 优先级建议

- **优先级**：P1
- **理由**：证据强、OpenAI 兼容薄封装；注意与 gmi 条目去重

---

### google_vertex_anthropic — Vertex (Anthropic)

- **canonical ID**：google_vertex_anthropic
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://cloud.google.com/vertex-ai/generative-ai/docs/partner-models/claude
- **核验来源**：官方 API 文档 + 多来源一致（langwatch、Google blog）
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://{LOCATION}-aiplatform.googleapis.com/v1（Vertex AI / Agent Platform 端点）
- **鉴权**：方式=Google Cloud OAuth2 Access Token（Bearer，来自 gcloud / service account）/ 环境变量=Google Application Default Credentials（GOOGLE_APPLICATION_CREDENTIALS 等）/ 是否必需=是
- **endpoint 公式**：POST /v1/projects/{PROJECT}/locations/{LOCATION}/publishers/anthropic/models/{MODEL}:rawPredict（非流式）；:streamRawPredict（流式）；支持多区域端点
- **协议类型**：原生（Anthropic Messages 协议经 Google Cloud rawPredict 封装，非 OpenAI 兼容）
- **请求结构要点**：body 为 Anthropic Messages 格式（messages/max_tokens 等）并带 anthropic_version 字段，外层由 Google rawPredict 包装
- **响应结构要点**：Anthropic Messages 响应（经 rawPredict 包装）；流式为 SSE
- **流式**：SSE（streamRawPredict）
- **错误结构**：Google Cloud 错误结构（HTTP 状态 + error 对象），与 OpenAI/Anthropic 直连不同
- **特有行为**：模型 ID 形如 `claude-opus-4-6@default` / `claude-haiku-4-5@20251001`（@version 后缀）；按 pay-as-you-go 或 provisioned throughput 计费；支持请求响应日志

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：原生（若 aimux 已有 Anthropic 共享层，可作 Anthropic 共享层扩展 + Google Cloud 鉴权/端点适配）
- **依据**：鉴权（Google IAM）、端点公式（rawPredict）、消息封装均与直连 Anthropic / OpenAI 存在结构性差异
- **可复用模型 ID 样例**：claude-opus-4-6@default、claude-haiku-4-5@20251001、claude-opus-4-5@20251101、claude-3-5-haiku@20241022
- **是否需扩展共享层**：是（需 Google Cloud 鉴权 + rawPredict 端点封装；消息体可复用 Anthropic 结构）

#### 4. 风险与限制

- 需 Google Cloud 项目/权限与 OAuth token 获取流程，鉴权复杂度高于 API Key 型 provider
- 端点含 project/location 变量，需参数化
- rawPredict/streamRawPredict 的精确请求封装建议再核对官方 rawPredict 示例

#### 5. 优先级建议

- **优先级**：P1
- **理由**：主流企业级 Claude 入口，价值高；但需原生实现 Google Cloud 鉴权与端点封装，工作量大于薄封装

---

### hetzner — Hetzner

- **canonical ID**：hetzner
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://experiments.hetzner.com/docs/inference（官方，JS 渲染未直接抓取）；官方社区教程 https://community.hetzner.com/tutorials/opencode-with-hetzner-inference-api-systemd-sandbox/
- **核验来源**：官方社区教程 + 多来源一致（sliplane、reddit）
- **证据强度**：中
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://inference.hetzner.com/api/v1
- **鉴权**：方式=Bearer API Token / 环境变量=HETZNER_VLLM_API_KEY（官方教程；inventory 标 HETZNER_API_KEY）/ 是否必需=是
- **endpoint 公式**：POST /v1/chat/completions（OpenAI 兼容）；GET /v1/models
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions（@ai-sdk/openai-compatible）；接受文本与图像
- **响应结构要点**：与 OpenAI 一致
- **流式**：SSE（标准 OpenAI stream，推断）
- **错误结构**：与 OpenAI 共享结构一致（推断）
- **特有行为**：实验性产品，免费、明确非生产就绪、无 SLA/计费；约 262K 上下文；欧洲 provider

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方教程与多来源确认 OpenAI 兼容 base URL 与 /chat/completions
- **可复用模型 ID 样例**：Qwen/Qwen3.6-35B-A3B-FP8
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 实验性、免费、无 SLA，不适合生产；特性/可用性可能随时变化
- 官方主文档页 JS 渲染未能直接抓取，环境变量名以官方教程 HETZNER_VLLM_API_KEY 为准
- 当前仅 1 个模型

#### 5. 优先级建议

- **优先级**：P2
- **理由**：OpenAI 兼容薄封装成本低，但实验性/单模型/非生产，价值有限

---

### hpc_ai — HPC-AI

- **canonical ID**：hpc_ai
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://www.hpc-ai.com/doc/docs/Model-APIs/API-Reference/OpenAI-Compatible-API/OpenAI-SDK/、https://www.hpc-ai.com/doc/docs/quickstart
- **核验来源**：官方 API 文档
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://api.hpc-ai.com/inference/v1
- **鉴权**：方式=Bearer（`Authorization: Bearer <api_key>`）/ 环境变量=官方示例用 INFERENCE_API_KEY（inventory 标 HPC_AI_API_KEY）/ 是否必需=是
- **endpoint 公式**：POST /v1/chat/completions（OpenAI 兼容）；GET /v1/models
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions（OpenAI SDK chat.completions.create，model/messages）
- **响应结构要点**：与 OpenAI 一致
- **流式**：SSE（标准 OpenAI stream，未在该页直接展示，按 OpenAI 兼容推断）
- **错误结构**：与 OpenAI 共享结构一致（推断）
- **特有行为**：HPC-AI 同时提供 GPU 云主机（quickstart 主体）与 Model APIs（MaaS，按量付费）；模型 ID 形如 minimax/minimax-m2.5、anthropic/claude-opus-4.7

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方 OpenAI-Compatible API 文档展示 OpenAI SDK + base URL + Bearer 鉴权
- **可复用模型 ID 样例**：anthropic/claude-opus-4.7、deepseek/deepseek-v4-flash、deepseek/deepseek-v4-pro、minimax/minimax-m2.5、moonshotai/kimi-k2.5
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 官方 OpenAI-SDK 页未直接展示完整请求字段/流式/错误结构，需参考其 API reference 补充
- 环境变量名官方示例为通用 INFERENCE_API_KEY，与 inventory HPC_AI_API_KEY 不一致

#### 5. 优先级建议

- **优先级**：P1
- **理由**：证据强、OpenAI 兼容薄封装，模型丰富

---

### hyper — Charm Hyper

- **canonical ID**：hyper
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://hyper.charm.land（官方站点，营销页；/docs 返回 404）；官方客户端 charmbracelet/crush
- **核验来源**：官方站点 + 官方客户端（Crush）
- **证据强度**：中
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://hyper.charm.land/v1（inventory 给出）
- **鉴权**：方式=API Key（master/sub key）/ 环境变量=HYPER_API_KEY / 是否必需=是
- **endpoint 公式**：推断 POST /v1/chat/completions（OpenAI 兼容）；官方未提供公开 API reference
- **协议类型**：OpenAI 兼容（推断；官方客户端 Crush 同时支持 OpenAI 兼容与 Anthropic 兼容 provider 配置，Hyper 为其官方 provider）
- **请求结构要点**：未知（无公开 API reference）
- **响应结构要点**：未知
- **流式**：未知
- **错误结构**：未知
- **特有行为**：面向编码优化的推理服务，零数据留存；Hypercredits 计费（1 credit=5¢）；master/sub key 细粒度管理；免费层每月 100 credits

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装（待证据补充确认）
- **依据**：base URL /v1 约定 + 官方客户端 Crush 走 OpenAI/Anthropic 兼容配置；但缺少直接 API reference
- **可复用模型 ID 样例**：deepseek-v4-flash、deepseek-v4-pro、gemma-4-26b-a4b-it、glm-5、glm-5.1
- **是否需扩展共享层**：否（待确认）

#### 4. 风险与限制

- 未找到公开 API reference，请求/响应/流式/错误结构均未确认，存在臆测风险
- 需以官方文档或 Crush 的 Hyper provider 配置源码二次确认 endpoint 与鉴权细节

#### 5. 优先级建议

- **优先级**：搁置
- **理由**：缺少可直接确认请求响应契约的官方 API 文档，证据不足以落实薄封装；待补充官方 API reference

---

### iflowcn — iFlow

- **canonical ID**：iflowcn
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://platform.iflow.cn/en/docs（注：该官方文档实为 iFlow Search API；chat 推理 API 的官方文档未直接定位到）；交叉引用 models.dev、Mastra、Medium
- **核验来源**：仅第三方聚合（models.dev/Mastra）+ 第三方文章，多来源一致
- **证据强度**：中
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://apis.iflow.cn/v1（chat 推理 API；与 platform.iflow.cn 的 Search API 是不同产品）
- **鉴权**：方式=Bearer API Key / 环境变量=IFLOW_API_KEY / 是否必需=是
- **endpoint 公式**：POST /v1/chat/completions（OpenAI 兼容，@ai-sdk/openai-compatible）；GET /v1/models
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions；支持 tool call、reasoning、temperature；模型多为免费
- **响应结构要点**：与 OpenAI 一致
- **流式**：SSE（标准 OpenAI stream，推断）
- **错误结构**：与 OpenAI 共享结构一致（推断）
- **特有行为**：iFlow 同时提供 Search API（platform.iflow.cn，web/image search、webFetch）与 LLM chat API（apis.iflow.cn/v1，免费开源模型路由）；模型含 deepseek-r1/v3/v3.2、glm-4.6、kimi-k2、qwen3 系列等

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：models.dev/Mastra/Medium 多来源一致确认 OpenAI 兼容 /chat/completions
- **可复用模型 ID 样例**：deepseek-r1、deepseek-v3、deepseek-v3.2、glm-4.6、kimi-k2、qwen3-235b、qwen3-max
- **是否需扩展共享层**：否

#### 4. 风险与限制

- inventory 提供的官方文档 URL 指向 Search API 而非 chat API，存在产品混淆风险
- chat API 官方文档未直接抓取，请求/响应/流式/错误结构来自第三方聚合，需以官方 chat API 文档二次确认

#### 5. 优先级建议

- **优先级**：P2
- **理由**：多来源确认 OpenAI 兼容薄封装，但官方 chat API 文档缺位，证据中等；模型免费有一定价值

---

### inceptron — Inceptron

- **canonical ID**：inceptron
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://docs.inceptron.io
- **核验来源**：官方 API 文档
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://api.inceptron.io/v1
- **鉴权**：方式=Bearer API Key / 环境变量=INCEPTRON_API_KEY / 是否必需=是
- **endpoint 公式**：POST /v1/chat/completions（OpenAI 兼容）；推断 GET /v1/models
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions（OpenAI SDK chat.completions.create，model/messages）；官方文档含 Stream 示例标签
- **响应结构要点**：与 OpenAI 一致
- **流式**：SSE（标准 OpenAI stream，官方示例含 Stream 选项）
- **错误结构**：与 OpenAI 共享结构一致（推断）
- **特有行为**：serverless AI 应用部署平台；模型 ID 形如 MiniMaxAI/MiniMax-M2.5、moonshotai/Kimi-K2.6、nvidia/llama-3.3-70b-instruct-fp8

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方文档以 OpenAI SDK 示例展示 base URL + Bearer 鉴权 + chat.completions.create
- **可复用模型 ID 样例**：MiniMaxAI/MiniMax-M2.5、moonshotai/Kimi-K2.6、moonshotai/Kimi-K2.6-Fast、moonshotai/Kimi-K2.7-Code、nvidia/llama-3.3-70b-instruct-fp8
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 官方 Get Started 页未展示完整请求字段/错误结构，需参考其完整 API reference 补充

#### 5. 优先级建议

- **优先级**：P1
- **理由**：证据强、OpenAI 兼容薄封装，支持流式

---

## 批次摘要

| id | 证据强度 | 实现路径 | 优先级 | 一句话备注 |
| --- | --- | --- | --- | --- |
| ebcloud | 中 | 薄封装 | P1 | 官方声明兼容 OpenAI API 格式，请求/响应细节待补 |
| empiriolabs | 强 | 薄封装 | P1 | OpenAI 兼容 chat/completions，140+ 模型聚合 |
| firepass | 强 | 薄封装 | P2 | Fireworks 计费通行证，OpenAI 兼容，建议复用 Fireworks 实现 |
| freemodel | 中 | 薄封装 | P2 | 官方+Mastra 确认 OpenAI 兼容，文档 SPA 未抓全 |
| frogbot | 强 | 薄封装 | P1 | 官方 API 参考 OpenAI 兼容，含多模态端点 |
| gitlab | 强 | 待定 | 搁置 | 官方明确 Duo Chat API 仅供内部，无公开模型 API |
| gmi | 中 | 薄封装 | P2 | 疑似 gmicloud 重复条目，OpenAI 兼容，建议去重 |
| gmicloud | 强 | 薄封装 | P1 | 官方 API reference 完整，OpenAI 兼容 |
| google_vertex_anthropic | 强 | 原生 | P1 | Anthropic 经 Vertex rawPredict，Google IAM 鉴权，非 OpenAI 兼容 |
| hetzner | 中 | 薄封装 | P2 | 官方教程确认 OpenAI 兼容，实验性/单模型/非生产 |
| hpc_ai | 强 | 薄封装 | P1 | 官方 OpenAI 兼容文档，模型丰富 |
| hyper | 中 | 待定 | 搁置 | 无公开 API reference，请求响应契约未确认 |
| iflowcn | 中 | 薄封装 | P2 | 多来源确认 OpenAI 兼容 chat API，官方文档指向 Search API |
| inceptron | 强 | 薄封装 | P1 | 官方 OpenAI SDK 示例，支持流式 |
