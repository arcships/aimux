# 第 14 批调研记录（14 个 provider）

> 调研日期：2026-07-28。按 RFC-0006 §2.1 证据裁决顺序核验，inventory 元数据仅作线索。
> 协议事实以官方文档为主；无法确认的字段写「未知」或留空，不臆造。

---

### alibaba_token_plan — Alibaba Token Plan

- **canonical ID**：alibaba_token_plan
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat、image_generation

#### 1. 官方协议证据

- **文档 URL**：
  - https://www.alibabacloud.com/help/en/model-studio/token-plan-overview （套餐概览）
  - https://www.alibabacloud.com/help/en/model-studio/compatibility-of-openai-with-dashscope （OpenAI 兼容模式说明）
- **核验来源**：官方 API 文档
- **证据强度**：强（官方文档确认 base URL 为 `/compatible-mode/v1`，即官方 OpenAI 兼容模式；第三方博客亦印证 Token Plan 使用专属 API key + `/compatible-mode/v1`）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://token-plan.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1`
- **鉴权**：方式=Bearer API Key / 环境变量=`ALIBABA_TOKEN_PLAN_API_KEY` / 是否必需=是
- **endpoint 公式**：`{base_url}/chat/completions`（OpenAI 兼容）；图像生成走 OpenAI 兼容 images 接口
- **协议类型**：OpenAI 兼容
- **请求结构要点**：OpenAI Chat Completions 标准结构（messages/model/stream 等）
- **响应结构要点**：OpenAI Chat Completions 标准结构
- **流式**：SSE（OpenAI 兼容）
- **错误结构**：与 OpenAI 共享结构一致（DashScope compatible-mode 返回 OpenAI 风格错误体）
- **特有行为**：仅新加坡（ap-southeast-1）区域；按 Credits 订阅计费；模型为精确字符串白名单（qwen3.7-max、deepseek-v4-pro、glm-5.2、MiniMax-M2.5 等）；仅限交互式编码/agent 工具使用

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：base URL 路径 `/compatible-mode/v1` 为阿里云官方 OpenAI 兼容模式，请求/响应/鉴权/流式均与 OpenAI Chat Completions 一致
- **可复用模型 ID 样例**：qwen3.7-max、qwen3.6-plus、deepseek-v4-pro、deepseek-v3.2、glm-5.2、MiniMax-M2.5、kimi-k2.5
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 套餐使用策略限制：仅限交互式编码/agent 工具，禁止自动化脚本后端调用，违规可能停用
- 仅新加坡区域；模型严格按白名单字符串匹配

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：标准 OpenAI 兼容，证据充分，薄封装即可；但属订阅套餐有使用策略限制

---

### alibaba_token_plan_cn — Alibaba Token Plan (China)

- **canonical ID**：alibaba_token_plan_cn
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat、image_generation

#### 1. 官方协议证据

- **文档 URL**：
  - https://www.alibabacloud.com/help/zh/model-studio/token-plan-overview （中文套餐概览）
  - https://www.alibabacloud.com/help/en/model-studio/compatibility-of-openai-with-dashscope （OpenAI 兼容模式说明）
- **核验来源**：官方 API 文档
- **证据强度**：强（与 alibaba_token_plan 同协议；中国区 cn-beijing 主机同样使用 `/compatible-mode/v1` 官方 OpenAI 兼容路径）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1`
- **鉴权**：方式=Bearer API Key / 环境变量=`ALIBABA_TOKEN_PLAN_API_KEY` / 是否必需=是
- **endpoint 公式**：`{base_url}/chat/completions`（OpenAI 兼容）
- **协议类型**：OpenAI 兼容
- **请求结构要点**：OpenAI Chat Completions 标准结构
- **响应结构要点**：OpenAI Chat Completions 标准结构
- **流式**：SSE（OpenAI 兼容）
- **错误结构**：与 OpenAI 共享结构一致
- **特有行为**：中国区（cn-beijing）部署；其余计费/白名单策略与国际版一致

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：与 alibaba_token_plan 同为 `/compatible-mode/v1` OpenAI 兼容模式，仅主机区域不同
- **可复用模型 ID 样例**：qwen3.7-max、deepseek-v4-pro、glm-5.2、MiniMax-M2.5
- **是否需扩展共享层**：否（可与 alibaba_token_plan 共用同一封装，仅 base URL 不同）

#### 4. 风险与限制

- 国际版官方说明仅新加坡可用；中国区为 cn-beijing 对应版本，需确认账号/开通区域差异
- 套餐使用策略限制同国际版

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：OpenAI 兼容薄封装，可与 alibaba_token_plan 复用实现

---

### azure_cognitive_services — Azure Cognitive Services

- **canonical ID**：azure_cognitive_services
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat、embedding

#### 1. 官方协议证据

- **文档 URL**：https://learn.microsoft.com/en-us/azure/ai-services/openai/concepts/models （实际为「Foundry Models sold by Azure」模型目录页）
- **核验来源**：官方文档（但该条目本身混杂多协议，不足以确认单一契约）
- **证据强度**：弱（官方文档确认各 Azure 服务 API 存在，但该 inventory 条目无 base_url，且模型样例含 Claude + GPT 混合，分属不同 Azure 服务/协议）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：未知（条目为空；Azure OpenAI 用 `https://{resource}.openai.azure.com`，Azure AI Foundry Model Inference 用 `https://{project}.services.ai.azure.com/models`）
- **鉴权**：未知（Azure OpenAI 用 `api-key` 头；Foundry 可用 AAD/DefaultAzureCredential 或 key；Claude on Foundry 走 Anthropic 协议）
- **endpoint 公式**：未知（条目未指定）
- **协议类型**：待定（Azure OpenAI=OpenAI 兼容部署式 URL；Foundry Model Inference API `/models/chat/completions`=OpenAI 兼容；Claude on Foundry=Anthropic Messages 协议）
- **请求结构要点**：未知
- **响应结构要点**：未知
- **流式**：未知
- **错误结构**：未知
- **特有行为**：canonical id「azure_cognitive_services」过于宽泛，混杂 Azure OpenAI、Foundry 直营模型与 Claude 合作伙伴模型

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：待定
- **依据**：条目无 base_url 且模型跨多协议，无法判定单一实现路径
- **可复用模型 ID 样例**：claude-haiku-4-5、claude-opus-4-5、claude-sonnet-4-5（Claude，Anthropic 协议）；另含 GPT 类（OpenAI 兼容）
- **是否需扩展共享层**：未知

#### 4. 风险与限制

- 条目语义不清：Azure OpenAI（OpenAI 兼容、部署式 URL + api-key 头）与 Azure AI Foundry（Model Inference API + AAD/key）与 Claude-on-Foundry（Anthropic 协议）混在一起
- 需先拆分为 azure_openai / azure_foundry 等独立 provider 再核验

#### 5. 优先级建议

- **优先级**：搁置
- **理由**：证据不足以确认单一请求响应契约，且条目本身需拆分界定；待明确为 Azure OpenAI 或 Foundry 后再定路径

---

### azure_text — Azure Text

- **canonical ID**：azure_text
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.5
- **能力**（本次调研覆盖）：chat、completion

#### 1. 官方协议证据

- **文档 URL**：无（inventory 未提供）；依据 litellm 路由 + Azure OpenAI 既有协议
- **核验来源**：仅第三方（litellm models/constants）+ Azure OpenAI 既有公开协议
- **证据强度**：中（litellm 明确 `azure_text` 路由对应 Azure OpenAI legacy `/completions`；Azure OpenAI completions 协议成熟稳定，但本轮未直接抓取官方 Azure 文档）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://{resource}.openai.azure.com/openai/deployments/{deployment-id}/completions?api-version={api-version}`（Azure OpenAI 原生部署式 URL）
- **鉴权**：方式=`api-key` 请求头（非 Bearer）/ 环境变量=`AZURE_API_KEY`、`AZURE_API_BASE`、`AZURE_API_VERSION` / 是否必需=是
- **endpoint 公式**：`/openai/deployments/{deployment-id}/completions?api-version=...`
- **协议类型**：原生（Azure 部署式 URL + `api-key` 头 + legacy text completions）
- **请求结构要点**：OpenAI legacy Text Completions 结构（`prompt` 字段，非 `messages`）
- **响应结构要点**：OpenAI legacy completions 结构（`choices[].text`）
- **流式**：SSE
- **错误结构**：Azure OpenAI 专属错误体（含 `error.code`）
- **特有行为**：部署 id 嵌入 URL；`api-version` 查询参数必填

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：原生（或模态专用：legacy text completions）
- **依据**：URL 结构、鉴权头、端点（legacy `/completions`）均与 OpenAI Chat Completions 薄封装模型不同
- **可复用模型 ID 样例**：azure/gpt-35-turbo-instruct、azure/gpt-35-turbo-instruct-0914、azure/gpt-3.5-turbo-instruct-0914
- **是否需扩展共享层**：是（Azure 部署式 URL 模板、`api-key` 头、`api-version` 参数）

#### 4. 风险与限制

- gpt-3.5-turbo-instruct 系列为 legacy 模型，正逐步退役
- 本轮未直接核验官方 Azure 文档，协议细节依据 litellm + Azure 既有约定

#### 5. 优先级建议

- **优先级**：P2（后续）
- **理由**：legacy instruct 模型逐步退役，价值有限；且为 Azure 原生协议需独立实现

---

### bedrock_mantle — Bedrock Mantle

- **canonical ID**：bedrock_mantle
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.5
- **能力**（本次调研覆盖）：chat、responses

#### 1. 官方协议证据

- **文档 URL**：
  - https://docs.aws.amazon.com/bedrock/latest/userguide/bedrock-mantle.html （AWS 官方，经 litellm 引用）
  - https://docs.litellm.ai/docs/providers/bedrock_mantle
- **核验来源**：官方 AWS 文档（引用）+ litellm + pydantic-ai
- **证据强度**：强（多源一致：litellm 与 pydantic-ai 均明确称「Amazon Bedrock Mantle OpenAI-compatible API」；AWS 官方页面存在且被引用）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://bedrock-mantle.{region}.api.aws/v1`（region 解析顺序：`BEDROCK_MANTLE_REGION` → `AWS_REGION` → 默认 `us-east-1`）
- **鉴权**：方式=Bearer API Key（`BEDROCK_MANTLE_API_KEY`）或 AWS SigV4 凭证（`AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY`）+ region / 环境变量=`BEDROCK_MANTLE_API_KEY`、`BEDROCK_MANTLE_REGION` / 是否必需=是
- **endpoint 公式**：`/chat/completions`（gpt-oss 等）、`/responses`（GPT-5.x）、`/messages`（Claude Mythos，Anthropic 协议）
- **协议类型**：OpenAI 兼容（chat/responses），同时支持 Anthropic `/messages`
- **请求结构要点**：OpenAI Chat Completions / Responses 标准结构；Claude 走 Anthropic Messages 结构
- **响应结构要点**：对应 OpenAI / Anthropic 标准结构
- **流式**：SSE
- **错误结构**：与 OpenAI 共享结构一致（chat/responses）
- **特有行为**：区域化 base URL；同一服务同时暴露 OpenAI `/chat/completions`、`/responses` 与 Anthropic `/messages`

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装（chat/completions）+ 共享层扩展（区域 base URL 解析、`/responses`、`/messages`）
- **依据**：chat/completions 走标准 OpenAI 兼容；但 base URL 含 region 变量、且支持 Responses 与 Anthropic Messages 多端点
- **可复用模型 ID 样例**：bedrock_mantle/openai.gpt-oss-120b、bedrock_mantle/openai.gpt-5.5、bedrock_mantle/anthropic.claude-mythos-preview
- **是否需扩展共享层**：是（region 化 base URL；多端点路由 `/responses`、`/messages`）

#### 4. 风险与限制

- region 必填且影响 base URL
- 多端点（chat/responses/messages）需分别处理；Claude 走 Anthropic 协议

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：OpenAI 兼容核心端点证据充分；多端点/region 需共享层扩展

---

### burncloud — burncloud

- **canonical ID**：burncloud
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：audio_speech、chat、image_generation、video_generation

#### 1. 官方协议证据

- **文档 URL**：https://www.burncloud.com/aiapi.html （官方站点模型列表页，但未公开 API 协议/base URL 文档）
- **核验来源**：仅官方站点（无 API 协议文档）+ 第三方推测
- **证据强度**：弱（官方站点确认其为 AI API 转售/聚合服务，但未公开 base URL、鉴权、请求响应契约；base URL 仅有第三方推测）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：未知（第三方推测 `https://api.burncloud.com/v1`，明确标注「presumed, 待确认」；playground 在 `https://ai.burncloud.com/playground`）
- **鉴权**：未知（推测 Bearer API Key）
- **endpoint 公式**：未知（推测 OpenAI 兼容 `/v1/chat/completions`，因转售 OpenAI/Claude/Gemini 模型）
- **协议类型**：未知（推测 OpenAI 兼容，未官方确认）
- **请求结构要点**：未知
- **响应结构要点**：未知
- **流式**：未知
- **错误结构**：未知
- **特有行为**：同时提供 GPU 租赁与 AI API 转售；模型列表含 GPT/Claude/Gemini/Grok/DeepSeek 等

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：待定
- **依据**：官方未公开 API 协议契约，无法确认请求响应结构
- **可复用模型 ID 样例**：anthropic/claude-3.5-haiku、anthropic/claude-opus-4、deepseek/deepseek-chat
- **是否需扩展共享层**：未知

#### 4. 风险与限制

- 官方无公开 API 文档；base URL/鉴权/协议均为第三方推测
- 转售性质，上游协议可能随模型不同而变化

#### 5. 优先级建议

- **优先级**：搁置
- **理由**：证据不足，无法确认请求响应契约；待官方公开 API 文档后再核验

---

### cherryin — cherryin

- **canonical ID**：cherryin
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat、completion、embedding、image_generation、rerank

#### 1. 官方协议证据

- **文档 URL**：https://docs.cherryin.ai/en/docs/newapi/getting-started （官方 Quick Start）
- **核验来源**：官方文档
- **证据强度**：强（官方文档明确「支持所有 OpenAI 格式兼容的 Chat 客户端」，并给出 base URL `https://open.cherryin.net`）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://open.cherryin.net`（OpenAI 兼容，完整路径 `https://open.cherryin.net/v1/chat/completions`）
- **鉴权**：方式=Bearer API Key（token）/ 环境变量=未知（建议 `CHERRYIN_API_KEY`）/ 是否必需=是
- **endpoint 公式**：`/v1/chat/completions`（OpenAI 兼容）
- **协议类型**：OpenAI 兼容（聚合网关）
- **请求结构要点**：OpenAI Chat Completions 标准结构
- **响应结构要点**：OpenAI Chat Completions 标准结构
- **流式**：SSE（OpenAI 兼容）
- **错误结构**：与 OpenAI 共享结构一致（聚合网关透传/标准化）
- **特有行为**：聚合多上游（OpenAI/Claude/Gemini/DeepSeek 等）；token 分组（default / gemini 等折扣组）影响计费

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方明确 OpenAI 格式兼容，base URL 与鉴权清晰
- **可复用模型 ID 样例**：agent/deepseek-v3.2-exp、agent/glm-4.6、agent/kimi-k2-0905、BAAI/bge-reranker-v2-m3
- **是否需扩展共享层**：否（chat/embedding 标准 OpenAI 兼容；rerank 若需支持或需扩展）

#### 4. 风险与限制

- 聚合网关，上游模型可用性/配额受第三方影响
- token 分组机制（折扣组）需注意计费差异

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：标准 OpenAI 兼容，证据充分，薄封装即可

---

### cloudflare_ai_gateway — Cloudflare AI Gateway

- **canonical ID**：cloudflare_ai_gateway
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat、embedding、rerank

#### 1. 官方协议证据

- **文档 URL**：https://developers.cloudflare.com/ai-gateway （官方）
- **核验来源**：官方文档
- **证据强度**：强（官方文档明确其为 AI 推理代理/网关，非模型厂商）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：不适用/未知（网关 URL 形如 `https://gateway.ai.cloudflare.com/v1/{account_id}/{gateway_id}/{provider}`，按上游 provider 变化）
- **鉴权**：取决于上游 provider（透传上游 API Key）
- **endpoint 公式**：按 provider 路由（OpenAI/Anthropic/Gemini/Replicate 等），非单一 OpenAI 端点；另有 Universal endpoint 可作 OpenAI 兼容入口
- **协议类型**：原生/网关（非模型厂商；按上游协议透传）
- **请求结构要点**：透传上游请求结构
- **响应结构要点**：透传上游响应结构
- **流式**：透传上游（SSE 等）
- **错误结构**：透传上游
- **特有行为**：提供缓存、限流、重试、fallback、分析日志；本质是控制面代理而非推理提供方

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：待定（网关/代理，非四路径之一）
- **依据**：非模型 provider，无自有协议；需网关式实现或作为已有 provider 的代理前置
- **可复用模型 ID 样例**：anthropic/claude-3-5-haiku、anthropic/claude-3-opus（均为上游模型经网关路由）
- **是否需扩展共享层**：未知

#### 4. 风险与限制

- 本质是代理/网关，非独立模型来源；aimux 作为 provider 适配库，需评估是否以「网关」形态接入
- base URL 依赖 account_id/gateway_id/provider 三段路径

#### 5. 优先级建议

- **优先级**：搁置
- **理由**：非模型厂商，无自有协议契约；属代理/网关范畴，需单独设计接入方式而非薄封装

---

### digitalocean — DigitalOcean

- **canonical ID**：digitalocean
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat、embedding、rerank

#### 1. 官方协议证据

- **文档 URL**：
  - https://docs.digitalocean.com/products/inference/getting-started/quickstart/
  - https://docs.digitalocean.com/products/inference/details/models/
- **核验来源**：官方文档
- **证据强度**：强（官方明确「改 base URL 为 `https://inference.do-ai.run` 并用 DigitalOcean API key 鉴权即可用 OpenAI SDK」，并作为 Codex/Claude Code 的 drop-in OpenAI 代理）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://inference.do-ai.run/v1`
- **鉴权**：方式=Bearer API Key（DigitalOcean API key / inference key）/ 环境变量=`DIGITALOCEAN_ACCESS_TOKEN` / 是否必需=是
- **endpoint 公式**：`/chat/completions`、`/embeddings`、`/rerank`（OpenAI 兼容 + rerank 扩展）
- **协议类型**：OpenAI 兼容
- **请求结构要点**：OpenAI Chat Completions / Embeddings 标准结构
- **响应结构要点**：OpenAI 标准结构
- **流式**：SSE（OpenAI 兼容）
- **错误结构**：与 OpenAI 共享结构一致
- **特有行为**：单一控制面含 Model Catalog、serverless/dedicated 部署；可作 coding agent 的 drop-in 代理；支持 embeddings/rerank

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装（chat/embeddings）；rerank 或需共享层扩展
- **依据**：官方确认 OpenAI 兼容，base URL 与 Bearer 鉴权清晰
- **可复用模型 ID 样例**：alibaba-qwen3-32b、all-mini-lm-l6-v2、anthropic-claude-3.5-sonnet
- **是否需扩展共享层**：否（chat/embeddings）；rerank 视实现而定

#### 4. 风险与限制

- 模型 ID 命名用连字符（如 `anthropic-claude-3.5-sonnet`）而非标准 vendor 前缀，需注意映射
- 含第三方商业模型，可用性受上游影响

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：标准 OpenAI 兼容，证据充分，薄封装即可

---

### doubao — Doubao

- **canonical ID**：doubao
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat、embedding、image_generation、video_generation

#### 1. 官方协议证据

- **文档 URL**：
  - https://www.volcengine.com/docs/82379/1399008 （火山方舟快速入门）
  - https://www.volcengine.com/docs/82379/2160841 （三方工具接入，含 baseUrl/apiKey 配置）
- **核验来源**：官方文档
- **证据强度**：强（官方文档给出 base URL `https://ark.cn-beijing.volces.com/api/v3` 与 Bearer apiKey，并展示 `/responses` 等调用；多源一致）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://ark.cn-beijing.volces.com/api/v3`
- **鉴权**：方式=Bearer API Key / 环境变量=`ARK_API_KEY`（或 `VOLCENGINE_API_KEY`）/ 是否必需=是
- **endpoint 公式**：`/chat/completions`、`/responses`、`/embeddings`（OpenAI 兼容）
- **协议类型**：OpenAI 兼容
- **请求结构要点**：OpenAI Chat Completions 标准结构（model 用接入点 id 或模型名）
- **响应结构要点**：OpenAI 标准结构
- **流式**：SSE（OpenAI 兼容）
- **错误结构**：与 OpenAI 共享结构一致（Ark 兼容模式）
- **特有行为**：火山方舟 Ark 平台；支持 `/responses`；多模态/视频/图像生成另有专属端点

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：Ark 提供 OpenAI 兼容 chat/completions/embeddings/responses，base URL 与 Bearer 鉴权清晰
- **可复用模型 ID 样例**：doubao-1-5-pro-32k-250115、doubao-1-5-lite-32k-250115、deepseek-v3-2-251201
- **是否需扩展共享层**：否（chat/embeddings）；`/responses` 与视频/图像生成端点视需求扩展

#### 4. 风险与限制

- 模型 id 可能需用「接入点 id（endpoint id）」而非模型名，配置易混淆
- 中国区服务（cn-beijing），跨境访问需考虑

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：标准 OpenAI 兼容，证据充分，薄封装即可

---

### evroc — evroc

- **canonical ID**：evroc
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat、embedding

#### 1. 官方协议证据

- **文档 URL**：https://docs.evroc.com/products/think/think.html （官方 Inference API；inventory 所给 overview.html 已 404，实际为 think.html）
- **核验来源**：官方文档 + models.dev + 第三方一致
- **证据强度**：强（官方文档明确「所有端点需 Bearer token 鉴权，Server: https://models.think.evroc.com/v1」；models.dev 标注 @ai-sdk/openai-compatible）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://models.think.evroc.com/v1`
- **鉴权**：方式=Bearer API Key / 环境变量=`EVROC_API_KEY` / 是否必需=是
- **endpoint 公式**：`/chat/completions`、`/embeddings`（OpenAI 兼容）
- **协议类型**：OpenAI 兼容
- **请求结构要点**：OpenAI Chat Completions / Embeddings 标准结构
- **响应结构要点**：OpenAI 标准结构
- **流式**：SSE（OpenAI 兼容，依兼容约定）
- **错误结构**：与 OpenAI 共享结构一致
- **特有行为**：含 KBLab/kb-whisper ASR、Qwen 系列、Reranker 模型

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方确认 OpenAI 兼容端点 + Bearer 鉴权 + base URL
- **可复用模型 ID 样例**：Qwen/Qwen3-30B-A3B-Instruct-2507-FP8、Qwen/Qwen3-Embedding-8B、Qwen/Qwen3-VL-30B-A3B-Instruct
- **是否需扩展共享层**：否（chat/embeddings）；rerank/ASR 视需求

#### 4. 风险与限制

- inventory 提供的 overview.html 已失效，实际文档为 think.html
- 平台较新，模型以 Qwen/开源为主

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：标准 OpenAI 兼容，证据充分，薄封装即可

---

### google_vertex — Vertex

- **canonical ID**：google_vertex
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat、embedding

#### 1. 官方协议证据

- **文档 URL**：
  - https://cloud.google.com/vertex-ai/generative-ai/docs/models
  - https://docs.cloud.google.com/gemini-enterprise-agent-platform/models/start/openai （OpenAI 兼容端点说明）
- **核验来源**：官方文档
- **证据强度**：中（官方确认存在 OpenAI 兼容端点 `/endpoints/openapi/chat/completions` 与原生 Gemini API；但 inventory 条目 base_url 为空，且模型样例含 Claude，分属不同协议）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：
  - OpenAI 兼容：`https://{location}-aiplatform.googleapis.com/v1/projects/{project}/locations/{location}/endpoints/openapi/chat/completions`
  - 原生 Gemini：`https://{location}-aiplatform.googleapis.com/v1beta1/...`
- **鉴权**：方式=GCP OAuth access token（Bearer，非简单 API key，需 gcloud/Service Account 授权）/ 环境变量=未知（通常 `GOOGLE_APPLICATION_CREDENTIALS`）/ 是否必需=是
- **endpoint 公式**：OpenAI 兼容 `/endpoints/openapi/chat/completions`；原生 Gemini `generateContent`；Claude 走 Anthropic-on-Vertex
- **协议类型**：原生（多协议：OpenAI 兼容 + Gemini 原生 + Anthropic-on-Vertex）
- **请求结构要点**：OpenAI 兼容端点为 OpenAI 结构；Gemini 原生为 Google `generateContent` 结构
- **响应结构要点**：对应 OpenAI / Google 结构
- **流式**：SSE
- **错误结构**：Google Cloud 错误体（含 `error.code`/`status`）
- **特有行为**：鉴权用 GCP OAuth；模型 id 含 `@version` 后缀；Claude 模型走 Anthropic 协议

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：原生
- **依据**：多协议并存且鉴权为 GCP OAuth（非 Bearer API key），与 OpenAI 薄封装模型结构性差异
- **可复用模型 ID 样例**：claude-3-5-haiku@20241022、claude-opus-4-5@20251101、gemini 系列模型
- **是否需扩展共享层**：是（GCP OAuth 鉴权、项目/区域化 URL、多端点协议）

#### 4. 风险与限制

- 鉴权复杂（GCP OAuth/Service Account），非简单 API key
- 条目混含 Gemini（原生/OpenAI 兼容）与 Claude（Anthropic-on-Vertex），需分别实现
- base_url 为空，需用户配置 project/location

#### 5. 优先级建议

- **优先级**：P2（后续）
- **理由**：多协议 + GCP OAuth 鉴权需原生实现，复杂度高；OpenAI 兼容端点可作子集先行

---

### llamagate — Llamagate

- **canonical ID**：llamagate
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat、embedding

#### 1. 官方协议证据

- **文档 URL**：
  - https://docs.litellm.ai/docs/providers/llamagate （litellm，引用官方 https://llamagate.dev/docs）
  - https://llamagate.dev/docs （官方，本轮未直接抓取 SPA）
- **核验来源**：litellm（引用官方文档）+ 官方 llamagate.dev
- **证据强度**：中（litellm 详细列出 base URL `https://api.llamagate.dev/v1`、端点、OpenAI 兼容参数并引用官方文档；但本轮未直接抓取官方 SPA 确认；另第三方有 `.io` 域名说法，存疑）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.llamagate.dev/v1`（litellm 给出；第三方有 `https://api.llamagate.io/v1` 说法，待官方确认）
- **鉴权**：方式=Bearer API Key / 环境变量=`LLAMAGATE_API_KEY` / 是否必需=是
- **endpoint 公式**：`/chat/completions`、`/embeddings`（OpenAI 兼容）
- **协议类型**：OpenAI 兼容
- **请求结构要点**：OpenAI Chat Completions / Embeddings 标准结构（支持 messages/model/stream/tools/response_format 等标准参数）
- **响应结构要点**：OpenAI 标准结构
- **流式**：SSE（OpenAI 兼容）
- **错误结构**：与 OpenAI 共享结构一致
- **特有行为**：仅开源模型；credit 计费；支持 vision/embedding/reasoning/code 模型

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：litellm 明确 OpenAI 兼容 drop-in，端点/参数/鉴权清晰
- **可复用模型 ID 样例**：llamagate/codellama-7b、llamagate/deepseek-coder-6.7b、llamagate/deepseek-r1-8b、llamagate/dolphin3-8b
- **是否需扩展共享层**：否

#### 4. 风险与限制

- base URL 域名 `.dev` vs `.io` 存在第三方不一致说法，需以官方 llamagate.dev 确认
- 仅开源模型，能力范围有限

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：标准 OpenAI 兼容，证据较充分；base URL 域名待官方最终确认

---

### zhipuai_coding_plan — Zhipu AI Coding Plan

- **canonical ID**：zhipuai_coding_plan
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://docs.bigmodel.cn/cn/coding-plan/quick-start （官方接入指南，含协议/base URL 表）
- **核验来源**：官方 API 文档
- **证据强度**：强（官方文档明确列出「OpenAI Chat Completion 协议 Base URL = `https://open.bigmodel.cn/api/coding/paas/v4`」及 Anthropic 协议备选）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://open.bigmodel.cn/api/coding/paas/v4`（OpenAI 协议）；另支持 Anthropic 协议 `https://open.bigmodel.cn/api/anthropic`
- **鉴权**：方式=Bearer API Key / 环境变量=`ZHIPU_API_KEY` / 是否必需=是
- **endpoint 公式**：OpenAI 协议 `{base_url}/chat/completions`；Anthropic 协议 `{anthropic_base}/messages`
- **协议类型**：OpenAI 兼容（+ Anthropic 协议备选）
- **请求结构要点**：OpenAI Chat Completions 标准结构
- **响应结构要点**：OpenAI Chat Completions 标准结构
- **流式**：SSE（OpenAI 兼容）
- **错误结构**：与 OpenAI 共享结构一致（智谱 PAAS v4 兼容模式）
- **特有行为**：套餐仅限官方指定编码工具使用；同时提供 OpenAI 与 Anthropic 两套接入协议；5 小时/周额度刷新

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装（OpenAI 协议）
- **依据**：官方明确 OpenAI Chat Completion 协议 base URL，请求/响应/鉴权/流式均兼容
- **可复用模型 ID 样例**：glm-4.5-air、glm-4.6v、glm-4.7、glm-5-turbo、glm-5.1
- **是否需扩展共享层**：否（OpenAI 协议）；Anthropic 协议备选视需求

#### 4. 风险与限制

- 套餐仅限指定编码工具环境调用，超出范围不享套餐额度
- 历史模型 GLM-5.1/GLM-5 自动切换至 GLM-5.2

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：标准 OpenAI 兼容，证据充分，薄封装即可
