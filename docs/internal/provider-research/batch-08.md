# 第 8 批调研记录（14 个 provider）

> 调研日期：2026-07-28。证据来源以各 provider 官方文档/SDK 为主，inventory 元数据仅作线索。
> 实现路径判定依据 RFC-0006 §2.2 四条路径；证据裁决依据 §2.1。

---

### llmtr — LLMTR

- **canonical ID**：llmtr
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://llmtr.com/docs（及 https://llmtr.com/docs/gateway/chat-completions/ 、https://llmtr.com/docs/authentication/ ）
- **核验来源**：官方 API 文档
- **证据强度**：强（官方文档可直接确认 OpenAI 兼容请求结构）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://llmtr.com/v1`
- **鉴权**：方式=Bearer（key 前缀 `llmtr-`）/ 环境变量=`LLMTR_API_KEY`（inventory）/ 是否必需=是
- **endpoint 公式**：`POST https://llmtr.com/v1/chat/completions`
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions 体（`model`、`messages`）。模型 ID 用 `provider/model` 格式（如 `openai/gpt-4o`、`google/gemini-2.5-flash`）。
- **响应结构要点**：与 OpenAI Chat Completions 一致（文档明确“现有 OpenAI SDK 不改动即可使用，仅改 base_url 与 api_key”）。
- **流式**：SSE（OpenAI 兼容网关标准，未单独标注但兼容层默认支持）
- **错误结构**：未知（未在所读页面展开，按 OpenAI 兼容推断，需最终核验）
- **特有行为**：多模态（视/音频/文件输入，按模型支持）；信用计费含 8% 平台加价；prompt/响应内容不持久化。

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方文档明确 OpenAI 兼容，仅替换 base_url + Bearer key 即可用 OpenAI SDK；模型 ID 用 `provider/model`。
- **可复用模型 ID 样例**：`openai/gpt-4o`、`google/gemini-2.5-flash`、`qwen3-6-35b`
- **是否需扩展共享层**：否（`provider/model` 形式与多数聚合网关一致，可在薄封装内处理）

#### 4. 风险与限制

- 模型 ID 带 `provider/` 前缀，需确认 aimux 模型透传是否完整保留斜杠。
- 错误结构未在文档展开，需实测。

#### 5. 优先级建议

- **优先级**：P1
- **理由**：证据强、协议标准 OpenAI 兼容、有 6 个模型，薄封装成本低。

---

### lucidquery — LucidQuery

- **canonical ID**：lucidquery
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://lucidquery.com/api/docs （首页即给出 OpenAI 兼容示例）
- **核验来源**：官方 API 文档
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.lucidquery.com/v1`
- **鉴权**：方式=Bearer（key 前缀 `lq_live_`）/ 环境变量=`LUCIDQUERY_API_KEY`（inventory）/ 是否必需=是
- **endpoint 公式**：`POST /v1/chat/completions`、`GET /v1/models`
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions（`model`、`messages`、`stream`）。
- **响应结构要点**：与 OpenAI 一致；`chunk.choices[0].delta.content` 流式增量。
- **流式**：SSE（官方首页明确“Token-by-token SSE”，默认流式）
- **错误结构**：未知（文档未展开）
- **特有行为**：欧元计费、按量付费；部分模型已下线（lucidquery-nexus-coder、lucidnova-rf1-100b 标 Decommissioned）。

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方明确“Drop-in compatible with the OpenAI SDK”，示例直接用 `openai.OpenAI(base_url=...)` 调 `chat.completions.create`。
- **可复用模型 ID 样例**：`lucidquery-agi-01-frontier`、`lucidquery-agi-01-swift`
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 在售模型仅 2 个（swift/frontier），另 2 个已下线；模型供给薄。
- 错误结构未展开。

#### 5. 优先级建议

- **优先级**：P1
- **理由**：证据强、纯 OpenAI 兼容，但模型数量少，价值中等。

---

### lynkr — Lynkr

- **canonical ID**：lynkr
- **aliases**：无
- **provider_kind**：local_runtime
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://github.com/Fast-Editor/Lynkr （README）
- **核验来源**：官方仓库 README
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`http://localhost:8081/v1`（inventory 的 `http://127.0.0.1:8081/v1` 与之等价）
- **鉴权**：方式=可选（README 对 Cursor 配置写“API Key: any-value”）/ 环境变量=`LYNKR_API_KEY`（inventory，本地配置）/ 是否必需=否（本地代理）
- **endpoint 公式**：本地网关；Cursor 接 `http://localhost:8081/v1`（覆盖 Base URL）；Codex 配置 `wire_api = "responses"`。
- **协议类型**：OpenAI 兼容（同时支持 OpenAI Responses API）
- **请求结构要点**：对外暴露 OpenAI 兼容接口；内部做 token 压缩、语义缓存、按复杂度分层路由到后端 provider（Ollama/OpenRouter/Bedrock/Azure/OpenAI/DeepSeek 等）。
- **响应结构要点**：与 OpenAI 一致（透明代理 + 优化）。
- **流式**：SSE（兼容上游）
- **错误结构**：未知
- **特有行为**：本地运行（npm 全局包 `lynkr`）；模型 `lynkr-auto` 为路由别名，实际后端为用户配置的上游 provider；无自有模型。

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：对外为 OpenAI 兼容端点，可作普通 OpenAI base_url 接入。
- **可复用模型 ID 样例**：`lynkr-auto`（路由别名）
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 本质是本地代理/路由器，无自有模型、无远程公网入口（127.0.0.1），aimux 接入价值有限。
- 鉴权非必需，与远程商用 provider 模型不同。

#### 5. 优先级建议

- **优先级**：P2
- **理由**：协议确认 OpenAI 兼容，但属本地运行时代理、无自有模型、无远程端点，接入优先级低。

---

### maritalk — Maritalk

- **canonical ID**：maritalk
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://github.com/maritaca-ai/maritalk-api （官方 README）；文档站 https://docs.maritaca.ai （SPA，未能直接抓取正文）
- **核验来源**：官方 SDK/仓库 README
- **证据强度**：中（官方 README 确认 OpenAI 兼容 + Responses API 用法；但 Chat Completions 端点契约未在 README 直接展示）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://chat.maritaca.ai/api`（注意无 `/v1` 后缀）
- **鉴权**：方式=Bearer（OpenAI SDK 标准，key 形如 `100088...` 纯数字）/ 环境变量=未知（inventory 为空；密钥在 plataforma.maritaca.ai 生成）/ 是否必需=是
- **endpoint 公式**：README 示例用 `client.responses.create`（对应 `POST /responses`）；Chat Completions（`/chat/completions`）兼容性待确认。
- **协议类型**：OpenAI 兼容（Responses API 已确认；Chat Completions 待确认）
- **请求结构要点**：Responses API 形式——`model`（如 `sabia-4`）、`input`（字符串或 messages 列表，role=user/system/assistant）、`max_output_tokens`、`temperature`、`stream`。
- **响应结构要点**：`response.output[0].content[0].text`；流式事件 `response.output_text.delta`。
- **流式**：SSE（`stream=True`，事件式）
- **错误结构**：未知
- **特有行为**：模型族 Sabiá（sabia-4、sabiazinho-4），面向葡萄牙语/巴西语境；按 token 计费（雷亚尔）。

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：待定（若 aimux 共享层支持 OpenAI Responses API → 薄封装；若仅 Chat Completions 则需先确认 `/chat/completions` 是否可用，否则需扩展）
- **依据**：官方 README 现行示例统一使用 Responses API，未直接展示 Chat Completions 契约，不能臆断。
- **可复用模型 ID 样例**：`sabia-4`、`sabiazinho-4`
- **是否需扩展共享层**：未知（取决于 aimux 是否实现 Responses API 及 maritalk 的 `/chat/completions` 支持情况）

#### 4. 风险与限制

- inventory 无任何 base_url/模型/env 线索（来源 litellm_constants），实际协议与 litellm 历史实现可能已有偏移（litellm 旧实现走 chat/completions，官方 README 现走 responses）。
- Chat Completions 端点未在官方 README 确认，存在协议歧义。

#### 5. 优先级建议

- **优先级**：P2
- **理由**：证据中等且存在 Responses/Chat Completions 歧义，需补充核验 docs.maritaca.ai 全文或实测后再定路径。

---

### meganova — Meganova

- **canonical ID**：meganova
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://docs.meganova.ai （Overview）、https://docs.meganova.ai/api-reference 、https://docs.meganova.ai/quickstart
- **核验来源**：官方 API 文档
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.meganova.ai/v1`（Inference API）；另有 Platform API `https://api.meganova.ai/api/v1`（仅账户/计费，非推理）
- **鉴权**：方式=Bearer（API key 在 console.meganova.ai 生成/轮换）/ 环境变量=`MEGANOVA_API_KEY`（inventory）/ 是否必需=是
- **endpoint 公式**：推理走 OpenAI 兼容端点（`/v1/chat/completions`、`/v1/embeddings`、图像、vision 等）
- **协议类型**：OpenAI 兼容（Inference API）
- **请求结构要点**：标准 OpenAI（文档明确“point your OpenAI SDK at https://api.meganova.ai/v1”）；支持 text/vision/image/embeddings。
- **响应结构要点**：与 OpenAI 一致。
- **流式**：SSE（OpenAI 兼容）
- **错误结构**：官方有“Errors & Response Conventions”页（Platform API 侧），推理侧未单独展开，按 OpenAI 兼容推断。
- **特有行为**：按 tier 分级（Tier1 仅免费 <100B 模型；Tier4 全模型 + 专属 GPU）；RPD 限额随 tier。

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方明确 Inference API 为 OpenAI Compatible，base_url 指向 `/v1`，可用 OpenAI SDK。
- **可复用模型 ID 样例**：`Qwen/Qwen3-235B-A22B-Instruct-2507`、`MiniMaxAI/MiniMax-M2.5`、`Qwen/Qwen2.5-VL-32B-Instruct`
- **是否需扩展共享层**：否（模型 ID 带 `provider/` 前缀，薄封装内处理）

#### 4. 风险与限制

- 模型 ID 带 `provider/` 前缀，需确认透传。
- tier 限制可能影响可用模型集合。

#### 5. 优先级建议

- **优先级**：P1
- **理由**：证据强、OpenAI 兼容、模型多（19+），薄封装即可。

---

### merge_gateway — Merge Gateway

- **canonical ID**：merge_gateway
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://docs.merge.dev/merge-gateway/get-started
- **核验来源**：官方 API 文档
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：OpenAI SDK 走 `https://api-gateway.merge.dev/v1/openai`；原生 SDK/Responses API 走 `https://api-gateway.merge.dev/v1`；Anthropic SDK 走 `.../v1/anthropic`；AI SDK 走 `.../v1/ai-sdk`
- **鉴权**：方式=Bearer（Merge Gateway API key，dashboard 获取）/ 环境变量=未知（inventory 为空）/ 是否必需=是
- **endpoint 公式**：OpenAI 兼容路径 `POST https://api-gateway.merge.dev/v1/openai/chat/completions`；原生 SDK 用 `responses.create`（Responses API）。
- **协议类型**：OpenAI 兼容（经 `/v1/openai` 路径）；原生 SDK 为 OpenAI Responses API
- **请求结构要点**：OpenAI SDK 路径下标准 `chat.completions.create`；该路径模型名**无需 provider 前缀**（如 `gpt-5.2`）。原生 Responses API 路径模型名为 `provider/model`（如 `openai/gpt-5.2`、`anthropic/claude-sonnet-4-20250514`）。
- **响应结构要点**：与 OpenAI 一致（chat.completions 返回 `choices[0].message.content`；responses 返回 `output[0].content[0].text`）。
- **流式**：SSE（OpenAI 兼容）
- **错误结构**：未知（未展开）
- **特有行为**：多 provider 路由网关（OpenAI/Anthropic/Google/Bedrock），支持路由策略、自动 failover、project_id 作用域、`model` 可省略走默认路由策略。

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方明确“Already using the OpenAI SDK? Point it at Merge Gateway”，`/v1/openai` 路径走标准 `chat.completions.create`，无需改业务代码。
- **可复用模型 ID 样例**：OpenAI 路径用裸名 `gpt-5.2`、`claude-sonnet-4-20250514`（inventory 的 `anthropic/...` 样例属原生 Responses API 形式）
- **是否需扩展共享层**：否（注意 base_url 需带 `/openai` 段；模型名在 OpenAI 路径不带前缀）

#### 4. 风险与限制

- base_url 必须精确到 `/v1/openai`，与一般 `/v1` 不同，薄封装需支持自定义 base path。
- OpenAI 路径模型名格式与 inventory 样例（带 `provider/`）不一致，需以官方 OpenAI 路径规范为准。
- 环境变量名未在文档给出。

#### 5. 优先级建议

- **优先级**：P1
- **理由**：证据强、模型多（83+）、多厂商聚合，薄封装价值高；需注意 base path 细节。

---

### midjourney — Midjourney

- **canonical ID**：midjourney
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：chat（inventory 标注，存疑）

#### 1. 官方协议证据

- **文档 URL**：无（inventory 为空）。官方产品文档 https://docs.midjourney.com 仅为产品使用说明，不含公开 chat/REST API 契约。
- **核验来源**：仅第三方/推断
- **证据强度**：无
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：inventory 给 `https://oa.api2d.net`（经核验为 API2D——一个 OpenAI 中转/代理服务，非 Midjourney 官方）
- **鉴权**：未知
- **endpoint 公式**：未知
- **协议类型**：未知（real Midjourney 为图像生成服务，无官方 OpenAI 兼容 chat API；社区普遍确认其无官方公开 API）
- **请求结构要点**：未知
- **响应结构要点**：未知
- **流式**：未知
- **错误结构**：未知
- **特有行为**：条目来源 `new_api`（New API 网关渠道导出），base_url 指向 api2d 中转，疑似被误标为“Midjourney”；与真实 Midjourney 图像生成服务无对应官方协议。

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：待定（证据不足，不臆造）
- **依据**：无法确认任何匹配该条目的官方协议契约。
- **可复用模型 ID 样例**：无
- **是否需扩展共享层**：未知

#### 4. 风险与限制

- 条目身份存疑：base_url 属 api2d 中转，display_name 为 Midjourney，二者不对应。
- 真实 Midjourney 无官方 OpenAI chat API，且 inventory 标 capability=chat 与图像生成定位不符。

#### 5. 优先级建议

- **优先级**：搁置
- **理由**：无官方文档可核验，条目疑似 New API 误标；不臆造协议。

---

### midjourney_plus — MidjourneyPlus

- **canonical ID**：midjourney_plus
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：chat（inventory 标注，存疑）

#### 1. 官方协议证据

- **文档 URL**：无（inventory 为空）
- **核验来源**：仅第三方/推断
- **证据强度**：无
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：inventory 给 `https://api.openai-sb.com`（经核验为 openai-sb——OpenAI 中转代理服务，非 Midjourney 官方）
- **鉴权**：未知
- **endpoint 公式**：未知
- **协议类型**：未知
- **请求结构要点**：未知
- **响应结构要点**：未知
- **流式**：未知
- **错误结构**：未知
- **特有行为**：与 `midjourney` 同源（`new_api`），base_url 指向 openai-sb 中转，疑似误标条目。

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：待定（证据不足，不臆造）
- **依据**：无法确认匹配该条目的官方协议契约。
- **可复用模型 ID 样例**：无
- **是否需扩展共享层**：未知

#### 4. 风险与限制

- 条目身份存疑，与 `midjourney` 同类误标风险。
- 无官方文档、无模型样例。

#### 5. 优先级建议

- **优先级**：搁置
- **理由**：无官方文档可核验，疑似 New API 误标；不臆造协议。

---

### mimo — Mimo

- **canonical ID**：mimo
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://mimo.mi.com/docs/en-US/api/chat/openai-api （小米 MiMo 官方文档）
- **核验来源**：官方 API 文档
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.xiaomimimo.com/v1`
- **鉴权**：方式=Bearer 或 `api-key` 头二选一（文档明确两种均支持：`Authorization: Bearer $MIMO_API_KEY` 或 `api-key: $MIMO_API_KEY`）/ 环境变量=`MIMO_API_KEY`（文档示例）/ 是否必需=是
- **endpoint 公式**：`POST https://api.xiaomimimo.com/v1/chat/completions`
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions 体（`model`、`messages`、`max_completion_tokens`、`temperature`、`top_p`、`stream`、`stop`、`frequency_penalty`、`presence_penalty`），并含扩展字段 `thinking`（`{"type":"disabled"|"adaptive"}`）。
- **响应结构要点**：标准 `chat.completion` 对象（`id`、`choices[].message`、`object:"chat.completion"`、`usage` 含 `completion_tokens_details.reasoning_tokens`）。
- **流式**：SSE（`stream:true`，chunk 对象）
- **错误结构**：未知（文档未展开）
- **特有行为**：小米 MiMo（模型 `mimo-v2.5-pro` 等）；支持 function call、web search、image/audio/video input、structured output、deep thinking。

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方文档标题即“OpenAI Chat Completions API Compatibility”，请求/响应结构与 OpenAI 一致；Bearer 鉴权可用。
- **可复用模型 ID 样例**：`mimo-v2.5-pro`
- **是否需扩展共享层**：否（`thinking` 为可选扩展字段，不影响标准兼容；如需深度推理可后续支持）

#### 4. 风险与限制

- inventory 完全无线索（base_url/docs/env/模型均空，来源 rust_genai），实际协议已通过官方文档补全。
- 同时支持 `api-key` 头与 Bearer，aimux 用 Bearer 即可。
- `thinking` 扩展字段若要支持需共享层小扩展，但非薄封装前提。

#### 5. 优先级建议

- **优先级**：P1
- **理由**：证据强、标准 OpenAI 兼容、官方维护，薄封装成本低。

---

### minimax_cn — MiniMax (minimaxi.com)

- **canonical ID**：minimax_cn
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://platform.minimaxi.com/docs/guides/quickstart 、https://platform.minimaxi.com/docs/api-reference/text-anthropic-api 、https://platform.minimaxi.com/docs/api-reference/text-openai-api
- **核验来源**：官方 API 文档
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：OpenAI 兼容 `https://api.minimaxi.com/v1`；Anthropic 兼容 `https://api.minimaxi.com/anthropic`（inventory 的 `https://api.minimaxi.com/anthropic/v1` 为 Anthropic 端点）；另有 AI SDK 兼容。
- **鉴权**：方式=Bearer（API Key）/ 环境变量=`MINIMAX_API_KEY`（inventory）/ 是否必需=是
- **endpoint 公式**：OpenAI 兼容 `POST https://api.minimaxi.com/v1/chat/completions`；Anthropic 兼容 `POST /anthropic/v1/messages`。
- **协议类型**：OpenAI 兼容（`/v1`）与 Anthropic 兼容（`/anthropic`）并存
- **请求结构要点**：OpenAI 路径标准 Chat Completions；Anthropic 路径支持 `model`、`messages`、`max_tokens`、`system`、`stream`、`temperature`、`top_p`、`tools`、`tool_choice`、`thinking`、`metadata`、`service_tier` 等（`top_k`/`stop_sequences`/`mcp_servers` 等被忽略）。
- **响应结构要点**：随所选兼容协议（OpenAI 或 Anthropic）一致。
- **流式**：SSE（两种协议均支持 `stream`）
- **错误结构**：未知
- **特有行为**：模型族 MiniMax-M3/M2.7/M2.5/M2.1/M2（含 highspeed 变体）；M3 支持图/视频输入与 thinking；`service_tier` 可选 standard/priority；Anthropic 路径提供 `/anthropic/v1/messages/count_tokens`。

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装（采用 OpenAI 兼容端点 `https://api.minimaxi.com/v1`）
- **依据**：官方明确提供 OpenAI SDK 兼容（`OPENAI_BASE_URL=https://api.minimaxi.com/v1`）。注意 inventory 记录的 base_url 为 Anthropic 端点，OpenAI 薄封装应改用 `/v1`。
- **可复用模型 ID 样例**：`MiniMax-M2`、`MiniMax-M2.1`、`MiniMax-M2.5`、`MiniMax-M2.7`、`MiniMax-M3`
- **是否需扩展共享层**：否（标准 OpenAI 兼容；若需 thinking/service_tier 可后续小扩展）

#### 4. 风险与限制

- inventory base_url 指向 Anthropic 端点，与 OpenAI 薄封装所需 `/v1` 不同，实现时须以 OpenAI 端点为准。
- Anthropic 路径部分参数被忽略，若误用 Anthropic 端点会有行为差异。

#### 5. 优先级建议

- **优先级**：P1
- **理由**：证据强、官方明确 OpenAI 兼容、模型矩阵完整，薄封装即可。

---

### minimax_cn_coding_plan — MiniMax Token Plan (minimaxi.com)

- **canonical ID**：minimax_cn_coding_plan
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://platform.minimaxi.com/docs/token-plan/intro
- **核验来源**：官方 API 文档
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：与 minimax_cn 同一 API（OpenAI 兼容 `https://api.minimaxi.com/v1`；Anthropic 兼容 `https://api.minimaxi.com/anthropic/v1`，即 inventory 所记）
- **鉴权**：方式=Bearer（**订阅 Key**，与按量计费 API Key 不互换）/ 环境变量=`MINIMAX_API_KEY`（inventory）/ 是否必需=是
- **endpoint 公式**：同 minimax_cn（`/v1/chat/completions` 或 `/anthropic/v1/messages`）
- **协议类型**：OpenAI 兼容 / Anthropic 兼容（与 minimax_cn 同）
- **请求结构要点**：同 minimax_cn。
- **响应结构要点**：同 minimax_cn。
- **流式**：SSE
- **错误结构**：未知
- **特有行为**：Token Plan 订阅制（Plus ¥49/月、Max ¥119/月、Ultra ¥469/月），5 小时滚动 + 周窗口额度；订阅 Key 在资源可用前即可存在；面向 Agent/编程工具（Claude Code/Cursor 等）。

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装（与 minimax_cn 共用实现，仅 Key 类型/计费不同）
- **依据**：协议端点与 minimax_cn 完全相同，差异仅在订阅 Key 与额度规则。
- **可复用模型 ID 样例**：`MiniMax-M3`、`MiniMax-M2.7`、`MiniMax-M2.5`
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 与 minimax_cn 实质同协议，建议在 aimux 中复用同一 provider 实现，仅以不同 profile/Key 区分，避免重复实现。
- trust_score=0，inventory 置信度存疑。

#### 5. 优先级建议

- **优先级**：P2
- **理由**：协议已确认但与 minimax_cn 重复，宜合并实现；单独接入优先级低。

---

### minimax_coding_plan — MiniMax Token Plan (minimax.io)

- **canonical ID**：minimax_coding_plan
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://platform.minimax.io/docs/token-plan/intro
- **核验来源**：官方 API 文档
- **证据强度**：中（Token Plan 概要已确认；OpenAI 兼容端点 `/v1` 系由 minimaxi.com 对应关系推断，未在所读 minimax.io 页面直接展开）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：inventory 记 Anthropic 端点 `https://api.minimax.io/anthropic/v1`；OpenAI 兼容端点预计为 `https://api.minimax.io/v1`（与 minimaxi.com 对应，需最终核验）
- **鉴权**：方式=Bearer（订阅 Key）/ 环境变量=`MINIMAX_API_KEY`（inventory）/ 是否必需=是
- **endpoint 公式**：同 MiniMax 国际站（`/v1/chat/completions` 或 `/anthropic/v1/messages`）
- **协议类型**：OpenAI 兼容 / Anthropic 兼容（与 minimax_cn 同族）
- **请求结构要点**：同 minimax_cn（推断一致）。
- **响应结构要点**：同 minimax_cn。
- **流式**：SSE
- **错误结构**：未知
- **特有行为**：国际站（minimax.io）Token Plan，价格 $20/$50/$120 每月；订阅 Key 与按量计费 Key 不互换；面向 Agent/编程工具。

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装（与 minimax_cn 共用实现，域名/Key/计费不同）
- **依据**：协议族与 minimax_cn 一致，差异在域名（minimax.io 国际站）与订阅计费。
- **可复用模型 ID 样例**：`MiniMax-M3`、`MiniMax-M2.7`、`MiniMax-M2.5`
- **是否需扩展共享层**：否

#### 4. 风险与限制

- OpenAI 兼容 base_url `/v1` 系推断，需最终核验 minimax.io 的 OpenAI SDK 接入页。
- 与 minimax_cn / minimax_cn_coding_plan 实质同协议，宜合并实现。
- trust_score=0。

#### 5. 优先级建议

- **优先级**：P2
- **理由**：协议基本确认但与同族重复，宜合并；OpenAI 端点需补一次核验。

---

### mixlayer — Mixlayer

- **canonical ID**：mixlayer
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://docs.mixlayer.com （Introduction）、https://docs.mixlayer.com/chat-completions
- **核验来源**：官方 API 文档
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://models.mixlayer.ai/v1`
- **鉴权**：方式=Bearer / 环境变量=`MIXLAYER_API_KEY`（inventory，文档示例同）/ 是否必需=是
- **endpoint 公式**：`POST https://models.mixlayer.ai/v1/chat/completions`
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions（`model`、`messages` 等）；文档明确“OpenAI-compatible REST API，swap base URL and API key”。
- **响应结构要点**：与 OpenAI 一致。
- **流式**：SSE（OpenAI 兼容）
- **错误结构**：未知
- **特有行为**：开源模型推理平台（qwen3.5 系列等）；支持 tool calling、reasoning（chain-of-thought）；模型 ID 带 `qwen/` 前缀。

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方明确 OpenAI 兼容，curl 示例为标准 `/v1/chat/completions` + Bearer。
- **可复用模型 ID 样例**：`qwen/qwen3.5-122b-a10b`、`qwen/qwen3.5-27b`、`qwen/qwen3.5-9b`
- **是否需扩展共享层**：否（模型 ID 带 `provider/` 前缀，薄封装内处理）

#### 4. 风险与限制

- 模型 ID 带 `qwen/` 前缀，需确认透传。
- 错误结构未展开。

#### 5. 优先级建议

- **优先级**：P1
- **理由**：证据强、纯 OpenAI 兼容、模型清晰，薄封装成本低。

---

### moark — Moark

- **canonical ID**：moark
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://moark.com/docs/openapi/v1 （模力方舟开放接口；正文为 SPA，经官方页面检索确认）、https://moark.com/docs （文档中心）
- **核验来源**：官方 API 文档（页面正文经检索获得）
- **证据强度**：强（官方文档明确“本接口兼容 OpenAI 的接口规范，可直接用 OpenAI SDK 调用”并给出 base_url）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.moark.com/v1`（官方文档示例；inventory 记 `https://moark.com/v1`，应以 `api.moark.com/v1` 为准）
- **鉴权**：方式=Bearer（OpenAI SDK 标准）/ 环境变量=`MOARK_API_KEY`（inventory）/ 是否必需=是
- **endpoint 公式**：`POST https://api.moark.com/v1/chat/completions`（OpenAI 兼容）
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions（官方示例 `from openai import OpenAI; client = OpenAI(base_url="https://api.moark.com/v1", ...)`）。
- **响应结构要点**：与 OpenAI 一致。
- **流式**：SSE（OpenAI 兼容，未单独标注）
- **错误结构**：未知
- **特有行为**：模力方舟（深圳奥思研工），面向开发者/产业场景的 AI 应用共创平台；提供百+模型在线体验、API 工作流、模型微调、算力市场；模型含 GLM-4.7、MiniMax-M2.1 等。

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方文档明确 OpenAI 接口规范兼容，可直接用 OpenAI SDK。
- **可复用模型 ID 样例**：`GLM-4.7`、`MiniMax-M2.1`
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 文档站为 SPA，部分页面直抓为空，正文经检索确认；建议实现前再读完整 OpenAPI 页核对错误结构。
- inventory base_url（`moark.com/v1`）与官方实际（`api.moark.com/v1`）不同，须以官方为准。

#### 5. 优先级建议

- **优先级**：P1
- **理由**：证据强、OpenAI 兼容、官方维护，薄封装成本低。
