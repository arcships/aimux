# 第 3 批调研记录（14 个 provider）

> 调研日期：2026-07-28
> 依据：RFC-0006 §2.1（官方文档/SDK > reference/ 成熟实现 > 多来源一致 > 单一第三方）；§2.2（四条实现路径）。
> 原则：inventory 元数据仅作线索，协议事实以官方文档为准；证据不足者标“无”并搁置，不臆造。

---

### abacus — Abacus

- **canonical ID**：abacus
- **aliases**：（无）
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://abacus.ai/help/developer-platform/route-llm/ （RouteLLM API Reference）
- **核验来源**：官方 API 文档
- **证据强度**：强（官方文档可直接确认请求响应）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://routellm.abacus.ai/v1`（自服务组织）；企业版 `https://<workspace>.abacus.ai/v1`
- **鉴权**：方式=Bearer API key（`Authorization: Bearer <key>`）/ 环境变量=`ABACUS_API_KEY`（inventory；官方未显式命名环境变量）/ 是否必需=是
- **endpoint 公式**：`POST /v1/chat/completions`（OpenAI Chat Completions）；另提供 `POST /v1/responses`（OpenAI Responses）、`POST /v1/messages`（Anthropic Messages）；`GET /v1/models`
- **协议类型**：OpenAI 兼容（Chat Completions 路径）
- **请求结构要点**：标准 OpenAI Chat Completions（`model`、`messages`、`stream` 等）；可用 `route-llm` 路由标识或指定具体模型
- **响应结构要点**：OpenAI Chat Completions 响应；支持流式、tool calling、多模态（文本/图像/音频/PDF）、图像生成、TTS、音频理解
- **流式**：SSE（支持）
- **错误结构**：与 OpenAI 共享结构一致（官方未详述差异，按兼容假设）
- **特有行为**：`route-llm` 智能路由；需 ChatLLM 订阅；同一 base URL/key 下三套请求格式（OpenAI Chat / Responses / Anthropic）

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方明确 `/v1/chat/completions` 为 OpenAI Chat Completions 格式，鉴权/URL/流式均可由 OpenAI 共享层正确表达
- **可复用模型 ID 样例**：`route-llm`、`gpt-5.5`、`o4-mini`（以 `GET /v1/models` 实时为准）
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 为路由网关，需 ChatLLM 订阅；模型列表随上游变化
- 企业版 base URL 含 `<workspace>` 段，需配置化
- inventory 模型样例（`MiniMaxAI/MiniMax-M2.7` 等）与官方列表不一致，须以 `/v1/models` 为准

#### 5. 优先级建议

- **优先级**：P0
- **理由**：证据强 + 薄封装 + 有可用模型 ID；属网关类，需订阅

---

### abliteration_ai — abliteration.ai

- **canonical ID**：abliteration_ai
- **aliases**：（无）
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://docs.abliteration.ai/models
- **核验来源**：官方 API 文档
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.abliteration.ai/v1`
- **鉴权**：方式=Bearer / 环境变量=`ABLIT_KEY`（官方 curl 使用 `$ABLIT_KEY`）/ 是否必需=是
- **endpoint 公式**：`POST /v1/chat/completions`；`GET /v1/models`（官方 curl 示例确认）
- **协议类型**：OpenAI 兼容
- **请求结构要点**：`{model, messages:[...]}`；支持 `stream`、tools、JSON mode / JSON Schema、reasoning effort、hide reasoning
- **响应结构要点**：OpenAI Chat Completions；含 reasoning trace
- **流式**：SSE（支持）
- **错误结构**：与 OpenAI 共享结构一致（按兼容假设；`abliterated-model-large` 收到图像/视频返回 400）
- **特有行为**：两个无审查（uncensored）推理模型；`abliterated-model` 多模态（图/视频，限 Chat Completions），`abliterated-model-large` 仅文本 1M 上下文；web search、web fetch

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方确认 `/v1/chat/completions` 为 OpenAI 格式 + Bearer 鉴权 + `/v1/models`
- **可复用模型 ID 样例**：`abliterated-model`、`abliterated-model-large`
- **是否需扩展共享层**：否（`reasoning_content` 已由共享层支持，见 [alibaba.rs](../../aimux-providers/src/alibaba.rs) 注释）

#### 4. 风险与限制

- 模型为“无审查（uncensored）”abliterated 模型，合规/内容风险需评估
- `abliterated-model-large` 拒绝图像/视频输入（400）
- niche 小厂商，稳定性未知

#### 5. 优先级建议

- **优先级**：P0
- **理由**：证据强 + 薄封装 + 2 个可用模型 ID

---

### advanced_custom — Advanced Custom

- **canonical ID**：advanced_custom
- **aliases**：（无）
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：无
- **核验来源**：无（inventory 源 `new_api`）
- **证据强度**：无
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：未知（inventory 为空）
- **鉴权**：未知
- **endpoint 公式**：未知
- **协议类型**：未知
- **请求结构要点**：未知
- **响应结构要点**：未知
- **流式**：未知
- **错误结构**：未知
- **特有行为**：未知

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：待定
- **依据**：经核验，“Advanced Custom”并非独立 provider，而是 New API 网关项目中的“自定义上游通道”类型（new-api changelog：“discover available models from advanced custom ... upstreams”）。inventory 记录无 base_url/文档/模型（model_count=0），属网关通道类别而非真实厂商。
- **可复用模型 ID 样例**：无
- **是否需扩展共享层**：不适用

#### 4. 风险与限制

- 非 provider，无法核验协议；inventory 条目为网关通道类型误入

#### 5. 优先级建议

- **优先级**：搁置
- **理由**：证据无；为 new-api 网关的通道类型而非独立厂商，无实现价值

---

### ai_router — AI-ROUTER

- **canonical ID**：ai_router
- **aliases**：（无）
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://ai-router.dev/openai-compatible-api-gateway （产品/FAQ 页，非完整 API reference）
- **核验来源**：官方产品页 + 多来源一致（apideposu 目录、positron/hermes-agent/CopilotForXcode 等 GitHub issue 均引用 `https://api.ai-router.dev/v1` 为 live OpenAI-compatible 目标）
- **证据强度**：中（官方页声明 OpenAI 兼容但无完整请求/响应契约；多第三方一致确认 base URL/bearer/chat path）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.ai-router.dev/v1`
- **鉴权**：方式=Bearer token / 环境变量=`AI_ROUTER_API_KEY` / 是否必需=是
- **endpoint 公式**：`POST /v1/chat/completions`；`GET /v1/models`
- **协议类型**：OpenAI 兼容
- **请求结构要点**：按 OpenAI Chat Completions（多来源一致；官方未发布完整 reference）
- **响应结构要点**：按 OpenAI Chat Completions（未由官方直接确认）
- **流式**：未知（按 OpenAI 兼容假设 SSE，未官方确认）
- **错误结构**：未知
- **特有行为**：面向运营商的网关/计费控制面；channel routing、group billing、payment-linked billing

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：多来源一致确认 OpenAI 兼容入口；但缺官方完整契约
- **可复用模型 ID 样例**：待 `/v1/models` 确认（inventory 样例 `gpt-5.4`/`gpt-5.5`/`gpt-5.6-luna/sol/terra` 疑为占位/虚假名）
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 官方未提供完整 API reference，请求/响应/流式/错误契约未由官方直接确认
- 模型样例疑为虚假/占位名，须以 `/v1/models` 实测
- 为 B2B 网关产品，公开可用性/计费模式不明

#### 5. 优先级建议

- **优先级**：P1
- **理由**：证据中 + 路径明确（薄封装）；实现前需补齐官方完整契约或实测

---

### aiand — ai&

- **canonical ID**：aiand
- **aliases**：（无）
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://docs.aiand.com
- **核验来源**：官方 API 文档
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.aiand.com`
- **鉴权**：方式=Bearer（`sk-...`）/ 环境变量=`AIAND_API_KEY` / 是否必需=是
- **endpoint 公式**：`POST /v1/chat/completions`；Files API（`/v1/files`）用于多模态上传
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions（`model`、`messages`）；多模态通过 Files API 上传后以 `file_id` 引用，官方称“OpenAI-canonical wire shape”
- **响应结构要点**：OpenAI Chat Completions
- **流式**：未知（文档未明确，按兼容假设 SSE）
- **错误结构**：与 OpenAI 共享结构一致（按兼容假设）
- **特有行为**：自托管开源权重模型；按 token 信用计费，失败请求不计费；声称兼容 OpenAI SDK / LangChain / LlamaIndex

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方明确 `/v1/chat/completions` 为 OpenAI 格式 drop-in replacement，Bearer 鉴权
- **可复用模型 ID 样例**：`openai/gpt-oss-120b`（官方示例）；完整 catalog 见 https://docs.aiand.com/models/catalog
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 多模态需 Files API 两步流程（非纯 OpenAI 内联），若支持图像需额外实现 Files 上传
- niche 自托管厂商，稳定性未知
- inventory 模型样例（`deepseek-v4`/`gemma-4`/`kimi-k2.6` 等）疑为占位名，以 catalog 为准

#### 5. 优先级建议

- **优先级**：P0
- **理由**：证据强 + 薄封装 + 有可用模型 ID（纯 chat）

---

### aiproxy — AIProxy

- **canonical ID**：aiproxy
- **aliases**：（无）
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：无（inventory 无；域名 `aiproxy.io` 现为第三方状态/导流页）
- **核验来源**：仅第三方（独立状态页 aiproxy.io 报告服务下线）
- **证据强度**：无（协议契约无法核验）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.aiproxy.io`（inventory；服务已下线）
- **鉴权**：未知
- **endpoint 公式**：未知（历史为 `aiproxy.io/v1` OpenAI 中转，但无官方文档可证）
- **协议类型**：未知
- **请求结构要点**：未知
- **响应结构要点**：未知
- **流式**：未知
- **错误结构**：未知
- **特有行为**：未知

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：待定
- **依据**：服务已下线，无法核验契约
- **可复用模型 ID 样例**：无
- **是否需扩展共享层**：不适用

#### 4. 风险与限制

- `aiproxy.io` 中转服务于 2025 年末下线、域名过期，无可用 dashboard/support；预付余额视为损失
- 注意区分：GitHub `labring/aiproxy` 为同名自托管网关项目，与已下线的 `api.aiproxy.io` 中转无关

#### 5. 优先级建议

- **优先级**：搁置
- **理由**：证据无；服务已下线，无实现价值

---

### aiproxy_library — AIProxyLibrary

- **canonical ID**：aiproxy_library
- **aliases**：（无）
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：无
- **核验来源**：无
- **证据强度**：无
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.aiproxy.io`（与 aiproxy 相同；服务已下线）
- **鉴权**：未知
- **endpoint 公式**：未知
- **协议类型**：未知
- **请求结构要点**：未知
- **响应结构要点**：未知
- **流式**：未知
- **错误结构**：未知
- **特有行为**：与 aiproxy 同一域名，疑为重复/别名条目

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：待定
- **依据**：与 aiproxy 同一域名（`api.aiproxy.io`），疑为重复/别名条目；服务已下线
- **可复用模型 ID 样例**：无
- **是否需扩展共享层**：不适用

#### 4. 风险与限制

- 与 aiproxy 重复，且服务下线

#### 5. 优先级建议

- **优先级**：搁置
- **理由**：证据无；服务下线且与 aiproxy 重复

---

### aki_io — AKI.IO

- **canonical ID**：aki_io
- **aliases**：（无）
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://aki.io/docs/compatibility/openai-api-compatibility/
- **核验来源**：官方 API 文档
- **证据强度**：强（协议契约明确；但 base URL 路径官方文档自相矛盾，见风险）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://aki.io/openai/v1`（cURL 与机器可读 JSON 配置一致）；官方文档同时出现 `https://aki.io/v1` 表述，需实测确认
- **鉴权**：方式=Bearer / 环境变量=`AKI_IO_API_KEY`（key 以 `aki-` 开头）/ 是否必需=是
- **endpoint 公式**：`POST /v1/chat/completions`、`GET /v1/models`、`POST /v1/images/generations`、`POST /v1/images/edits`
- **协议类型**：OpenAI 兼容（提供独立 OpenAI 兼容接口；另存在原生 `/api/call/{model}` 模型中心 API，鉴权在 JSON body）
- **请求结构要点**：标准 OpenAI Chat Completions（`model`、`messages`、`temperature`、`max_tokens`、`stream`、`stop`）
- **响应结构要点**：OpenAI Chat Completions；`/v1/models` 返回 `id`/`object`/`created`/`owned_by`
- **流式**：SSE（`stream=true`，server-sent events with token deltas）
- **错误结构**：与 OpenAI 共享结构一致（按兼容假设）
- **特有行为**：GDPR 合规、欧盟托管；原生 API 为双向实时流式（JSON 消息，二进制 Base64 内嵌）；`/v1/models` 的 `max_model_len` 为 0 时需回退上下文限值

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方提供独立 OpenAI 兼容接口，`/v1/chat/completions` + Bearer，drop-in replacement
- **可复用模型 ID 样例**：`llama3-chat-70b`、`gpt-oss-120b`、`gemma4-26b`、`kimi-k2.7-code-1100b`、`minimax-m2.5-230b`、`mistral4-119b`、`qwen3.6-35b`
- **是否需扩展共享层**：否

#### 4. 风险与限制

- base URL 官方文档自相矛盾（`/v1` vs `/openai/v1`），薄封装配置前必须实测确认
- 原生 `/api/call/{model}` API 为非 OpenAI 协议（鉴权在 body），不要与兼容接口混淆
- `/v1/models` 的 `max_model_len` 可能为 0，需回退限值

#### 5. 优先级建议

- **优先级**：P1
- **理由**：证据强 + 薄封装，但 base URL 路径有官方矛盾需先实测确认，故非“立即”

---

### ali — Ali

- **canonical ID**：ali
- **aliases**：（无）
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：无（inventory 无；base_url 指向 DashScope 原生入口）
- **核验来源**：官方 API 文档（阿里云 Model Studio / DashScope，base_url 与之一致）
- **证据强度**：中（DashScope 原生协议官方可证；但 “ali” 条目本身无文档/模型/环境变量，属裸条目）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://dashscope.aliyuncs.com`（DashScope 原生）
- **鉴权**：方式=Bearer / 环境变量=`DASHSCOPE_API_KEY`（官方 DashScope 文档确认；inventory 未列）/ 是否必需=是
- **endpoint 公式**：`POST /api/v1/services/aigc/text-generation/generation`（原生 DashScope 文本生成）
- **协议类型**：原生（DashScope 原生协议，非 OpenAI 兼容）
- **请求结构要点**：原生格式 `input.messages` + `parameters`（非 OpenAI `messages` 顶层结构）
- **响应结构要点**：原生 DashScope `output` 结构
- **流式**：未知（DashScope 支持 SSE 增量输出，需按模型确认）
- **错误结构**：厂商专属（DashScope `code`/`message` 结构）
- **特有行为**：与 OpenAI 兼容模式（`/compatible-mode/v1`）为同一服务的不同入口

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：原生
- **依据**：裸 `dashscope.aliyuncs.com` URL 对应原生 DashScope 协议，请求/响应结构与 OpenAI 不同
- **可复用模型 ID 样例**：未知（inventory model_count=0）
- **是否需扩展共享层**：否（应作原生实现）

#### 4. 风险与限制

- OpenAI 兼容入口已由已实现的 `alibaba` provider（canonical id=alibaba，base `dashscope-intl.../compatible-mode/v1`，env `ALIBABA_API_KEY`）覆盖同一批 DashScope 模型的 chat；原生入口无额外价值
- “ali” 条目为裸条目（无文档/模型/env），协议虽可外部佐证但条目本身证据不足

#### 5. 优先级建议

- **优先级**：搁置
- **理由**：chat 已被 `alibaba` provider（OpenAI 兼容模式）覆盖；原生入口无额外价值且条目本身证据不足

---

### alibaba_cn — Alibaba (China)

- **canonical ID**：alibaba_cn
- **aliases**：（无）
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://www.alibabacloud.com/help/en/model-studio/models
- **核验来源**：官方 API 文档（Alibaba Cloud Model Studio）
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://dashscope.aliyuncs.com/compatible-mode/v1`（中国区 OpenAI 兼容模式）；新 MaaS 入口 `https://{WorkspaceId}.cn-beijing.maas.aliyuncs.com/compatible-mode/v1`
- **鉴权**：方式=Bearer / 环境变量=`DASHSCOPE_API_KEY` / 是否必需=是
- **endpoint 公式**：`POST /chat/completions`（OpenAI 兼容）；另有 Anthropic 兼容 `/apps/anthropic`、原生 `/api/v1`
- **协议类型**：OpenAI 兼容（compatible-mode）
- **请求结构要点**：OpenAI Chat Completions
- **响应结构要点**：OpenAI Chat Completions；reasoning 模型返回 `reasoning_content`（共享层已支持）
- **流式**：SSE（OpenAI 兼容）
- **错误结构**：与 OpenAI 共享结构一致（兼容模式）
- **特有行为**：多区域（北京/香港/新加坡/东京/法兰克福/弗吉尼亚）；同一模型三种入口（OpenAI 兼容 / Anthropic 兼容 / DashScope 原生）

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方确认 `compatible-mode/v1` 为 OpenAI 兼容
- **可复用模型 ID 样例**：`qwen3.7-max`、`qwen3.7-plus` 等
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 已被已实现的 `alibaba` provider 覆盖（[alibaba.rs](../../aimux-providers/src/alibaba.rs) 注释明确支持将 base URL 覆盖为中国端点 `https://dashscope.aliyuncs.com/compatible-mode/v1`）；仅区域/base_url/env 不同

#### 5. 优先级建议

- **优先级**：搁置
- **理由**：已有别名覆盖（`alibaba` provider 同协议，中国区仅为 base_url/env 差异）

---

### stability — Stability

- **canonical ID**：stability
- **aliases**：（无）
- **provider_kind**：model_vendor
- **inventory 分层**：tier=L3 / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：image_generation、image_edit

#### 1. 官方协议证据

- **文档 URL**：https://platform.stability.ai/docs/getting-started/stable-image ；https://platform.stability.ai/docs/getting-started
- **核验来源**：官方 API 文档 + 官方知识库（kb.stability.ai 集成指南确认 endpoint）
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.stability.ai`
- **鉴权**：方式=`Authorization: Bearer <api_key>`（官方：所有 API 通过 Authorization header 传递 API key）/ 环境变量=未知（inventory 未列；行业/SDK 惯例 `STABILITY_API_KEY`）/ 是否必需=是
- **endpoint 公式**：`POST /v2beta/stable-image/generate/{ultra|core|sd3}`（文本到图像）；image-to-image/edit 等对应 `/v2beta/stable-image/...` 子路径
- **协议类型**：专用模态（image）+ 原生 REST v2beta（非 OpenAI 图像格式）
- **请求结构要点**：multipart/form-data（`prompt`、`negative_prompt`、`aspect_ratio`、`seed`、`output_format` 等）；非 OpenAI `/v1/images/generations` JSON
- **响应结构要点**：返回图像（二进制/base64）；REST v2beta 支持异步轮询
- **流式**：无（图像生成；异步任务走轮询）
- **错误结构**：厂商专属
- **特有行为**：Stable Image Ultra/Core、SD3.5 等；v2beta 提供异步 API 轮询

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：模态专用
- **依据**：仅图像能力，REST v2beta multipart 协议与 OpenAI 图像格式结构性不同，需原生实现 image 模型 trait
- **可复用模型 ID 样例**：`stable-image-ultra`、`stable-image-core`、`sd3`（sd3.5）等
- **是否需扩展共享层**：否（作模态专用实现）

#### 4. 风险与限制

- 为原生 REST v2beta（multipart + 异步轮询），非薄封装，工作量中等
- env 变量未由 inventory 提供，需确认
- 模型迭代较快（已迁移至 v2beta）

#### 5. 优先级建议

- **优先级**：P1
- **理由**：证据强 + 模态专用 + 有模型 ID；但为原生 REST 实现（非薄封装），image 模态已支持，工作量中等

---

### tavily — Tavily

- **canonical ID**：tavily
- **aliases**：（无）
- **provider_kind**：model_vendor
- **inventory 分层**：tier=L3 / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：search

#### 1. 官方协议证据

- **文档 URL**：https://docs.tavily.com/documentation/api-reference/endpoint/search
- **核验来源**：官方 API 文档
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.tavily.com`
- **鉴权**：方式=Bearer / 环境变量=`TAVILY_API_KEY`（key 以 `tvly-` 前缀）/ 是否必需=是
- **endpoint 公式**：`POST /search`（Search）；另有 `/research`、`/extract`、`/crawl`、`/map`
- **协议类型**：专用模态（search）+ 原生
- **请求结构要点**：JSON body `{query, search_depth, max_results, topic, include_answer, include_domains, exclude_domains, country, ...}`
- **响应结构要点**：`{query, answer, results:[{title, url, content, score, ...}], response_time, usage:{credits}, request_id}`
- **流式**：无
- **错误结构**：厂商专属 `{detail:{error:"..."}}`；状态码 400/401/429/432/433/500
- **特有行为**：`search_depth` basic/advanced；`topic` general/news；Research 为多步研究任务

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：模态专用
- **依据**：纯 search 能力，原生 JSON 协议
- **可复用模型 ID 样例**：`tavily/search`、`tavily/search-advanced`（语义性标识）
- **是否需扩展共享层**：不适用

#### 4. 风险与限制

- aimux-core 当前无 search/web_search 模型 trait（仅有 language/embedding/rerank/speech/transcription/image/video/files）；`web_search` 仅作为 provider-defined tool 存在（见 [tool.rs](../../aimux-core/src/tool.rs)）
- 接入需先定义 core search 契约

#### 5. 优先级建议

- **优先级**：搁置
- **理由**：证据强但 search 非已支持模态，需 core 契约变更后方可实现

---

### tinyfish — Tinyfish

- **canonical ID**：tinyfish
- **aliases**：（无）
- **provider_kind**：model_vendor
- **inventory 分层**：tier=L3 / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：search

#### 1. 官方协议证据

- **文档 URL**：https://docs.tinyfish.ai/search-api
- **核验来源**：官方 API 文档
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.search.tinyfish.ai`
- **鉴权**：方式=`X-API-Key` header / 环境变量=`TINYFISH_API_KEY` / 是否必需=是
- **endpoint 公式**：`GET https://api.search.tinyfish.ai?query=...`（根路径 + 查询参数）
- **协议类型**：专用模态（search）+ 原生
- **请求结构要点**：query 参数 `query`、`count`、`location`、`language`、`recency_minutes`、`after_date`、`before_date`、`domain_type`、`purpose` 等
- **响应结构要点**：`{query, results:[{position, site_name, title, snippet, url}], total_results, page}`
- **流式**：无
- **错误结构**：厂商专属（未详述）
- **特有行为**：search 免费；另提供 Fetch/Agent/Browser API；`domain_type` web/news/research_paper

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：模态专用
- **依据**：纯 search 能力，原生 REST（`X-API-Key` 鉴权）
- **可复用模型 ID 样例**：`tinyfish/search`（语义性标识）
- **是否需扩展共享层**：不适用

#### 4. 风险与限制

- aimux-core 无 search 模型 trait，需先定义 core 契约
- 鉴权为 `X-API-Key`（非 Bearer）

#### 5. 优先级建议

- **优先级**：搁置
- **理由**：证据强但 search 非已支持模态，需 core 契约变更

---

### you_com — YOU COM

- **canonical ID**：you_com
- **aliases**：（无）
- **provider_kind**：model_vendor
- **inventory 分层**：tier=L3 / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：search

#### 1. 官方协议证据

- **文档 URL**：https://you.com/docs/api-reference/search/v1-search
- **核验来源**：官方 API 文档
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://ydc-index.io`
- **鉴权**：方式=`X-API-Key` header / 环境变量=未知（inventory 未列；官方以 `X-API-Key` header 鉴权，SDK 惯例 `YDC_API_KEY`）/ 是否必需=是
- **endpoint 公式**：`GET /v1/search`（或 `POST /v1/search` 用于复杂参数）
- **协议类型**：专用模态（search）+ 原生
- **请求结构要点**：query 参数 `query`、`count`、`freshness`、`offset`、`country`、`language`、`safesearch`、`livecrawl`、`include_domains`、`exclude_domains`、`boost_domains` 等
- **响应结构要点**：`{results, metadata}`（web + news 分区）
- **流式**：无
- **错误结构**：401/403/422/500
- **特有行为**：`livecrawl`（按页计费）；web/news 分区；另有 Contents/Research API

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：模态专用
- **依据**：纯 search 能力，原生 REST（`X-API-Key` 鉴权，base URL `ydc-index.io`）
- **可复用模型 ID 样例**：`you_com/search`（语义性标识）
- **是否需扩展共享层**：不适用

#### 4. 风险与限制

- aimux-core 无 search 模型 trait，需先定义 core 契约
- 鉴权为 `X-API-Key`（非 Bearer）；base URL 为 `ydc-index.io` 而非 `you.com`

#### 5. 优先级建议

- **优先级**：搁置
- **理由**：证据强但 search 非已支持模态，需 core 契约变更
