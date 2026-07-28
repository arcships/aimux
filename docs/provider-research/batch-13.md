# 第 13 批调研记录（14 个 provider）

> 调研日期：2026-07-28。本批共 14 个 provider，按 canonical id 字母序排列。
> 证据裁决遵循 RFC-0006 §2.1：官方 API 文档/SDK > 多来源一致 > 单一第三方。
> inventory 的 tier/protocol/openai_compatible/confidence 字段仅作线索，不作依据。

---

### vidu — Vidu

- **canonical ID**：vidu
- **aliases**：
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：inventory 标 chat，**实际为视频生成（text-to-video / image-to-video），chat 标注疑似错误**

#### 1. 官方协议证据

- **文档 URL**：https://platform.vidu.com/docs/text-to-video 、https://platform.vidu.com/docs/image-to-video 、https://platform.vidu.com/docs/vidu-s1
- **核验来源**：官方 API 文档（platform.vidu.com，JS 渲染页面，部分内容经 WebSearch 片段确认）
- **证据强度**：中（官方文档确认其为视频生成平台 + Token 鉴权 + 异步任务模型；但页面 JS 渲染，未能读取完整 endpoint 参数细节）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://api.vidu.cn
- **鉴权**：方式=`Authorization: Token {api_key}`（示例 `Token vda_xxx`）/ 环境变量=未知（inventory 未给出）/ 是否必需=是
- **endpoint 公式**：异步任务模式——POST 创建生成任务（需设置 `callback_url`），任务状态变化时 Vidu 回调；亦支持轮询查询。具体 path 未能从可读片段完整确认（疑似 `/ent/v2/...` 系列）
- **协议类型**：专用模态（视频生成，原生异步任务协议，非 OpenAI Chat 格式）
- **请求结构要点**：任务创建体含 prompt/图片输入、callback_url 等字段；非 messages 数组结构
- **响应结构要点**：任务对象含 task_id、状态字段；视频产物通过回调或查询返回 URL
- **流式**：未知（任务为异步回调/轮询；Vidu S1 实时交互为 session 模式）
- **错误结构**：未知
- **特有行为**：纯视频生成；S1 模型为实时交互式视频生成（session create API）；与 chat completions 无关

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：模态专用
- **依据**：Vidu 是视频生成平台，采用异步任务+回调协议，与 OpenAI Chat Completions 结构无关，inventory 的 chat 能力标注错误
- **可复用模型 ID 样例**：未知（inventory model_sample 为空）
- **是否需扩展共享层**：否（属于视频模态独立实现，不走 OpenAI 共享层）

#### 4. 风险与限制

- inventory 将能力标为 chat，与官方文档（视频生成）不符，需在 inventory 修正能力标注
- 官方文档为 JS 渲染 SPA，endpoint 完整公式未完整读取，实现前需二次核验 `/ent/v2` 系列路径与请求/回调字段
- 鉴权用 `Token` 前缀而非 `Bearer`，需注意 header 构造

#### 5. 优先级建议

- **优先级**：P2（后续）
- **理由**：视频生成模态，非 chat 适配范畴；若 aimux 规划视频模态支持可纳入，否则暂缓

---

### vivgrid — Vivgrid

- **canonical ID**：vivgrid
- **aliases**：
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat（聚合 GPT/Claude/Gemini/DeepSeek/GLM 等多家模型）

#### 1. 官方协议证据

- **文档 URL**：https://vivgrid.com/docs/api/agent/completions 、https://docs.vivgrid.com/models
- **核验来源**：官方 API 文档
- **证据强度**：强（官方文档直接给出 OpenAI-compatible Chat Completions endpoint、请求体与响应体示例）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://api.vivgrid.com/v1
- **鉴权**：方式=Bearer token（`Authorization: Bearer <token>`）/ 环境变量=VIVGRID_API_KEY（inventory）/ 是否必需=是
- **endpoint 公式**：`POST https://api.vivgrid.com/v1/chat/completions`
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions 体——`model`、`messages`、`stream`、`max_completion_tokens`；额外支持 `reasoning_effort`（medium 等）字段
- **响应结构要点**：标准 `chat.completion` 对象——`id`、`object`、`created`、`model`、`choices[].message`、`finish_reason`、`usage{prompt_tokens,completion_tokens,total_tokens}`
- **流式**：SSE（`stream: true`）
- **错误结构**：返回 401/404/500 等状态码（具体 body 结构未在可读片段给出）
- **特有行为**：模型可为 `managed`（由后端 Agent 设置管理）；文档称 API 调用可不指定 model-name；`reasoning_effort` 为 OpenAI o-series 风格扩展

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装（`reasoning_effort` 字段属 OpenAI o-series 已有概念，可由共享层透传，无需单独扩展）
- **依据**：官方文档明确标注 "OpenAI-compatible Chat Completions endpoint"，请求/响应结构与 OpenAI 一致
- **可复用模型 ID 样例**：glm-5.2、deepseek-v4-pro、gemini-3.1-pro-preview、claude-sonnet-5、gpt-5.5（聚合多家）
- **是否需扩展共享层**：否

#### 4. 风险与限制

- `model: "managed"` 特殊值与按 model-name 路由的语义不同，薄封装需保留透传模型字符串
- 聚合网关，模型可用性与计费随上游变动

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：官方文档明确 OpenAI 兼容，薄封装成本低、模型覆盖广

---

### volc_engine — VolcEngine

- **canonical ID**：volc_engine
- **aliases**：
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：chat（火山方舟 Ark 大模型推理）

#### 1. 官方协议证据

- **文档 URL**：https://www.volcengine.com/docs/82379/2160841 （火山方舟接入三方工具，官方）
- **核验来源**：官方 API 文档 + 多来源一致（GitHub hermes-agent、Dify marketplace 均一致）
- **证据强度**：强（官方文档明确"火山方舟兼容 OpenAI 和 Anthropic 接口协议"，多来源一致确认 endpoint 与鉴权）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://ark.cn-beijing.volces.com/api/v3 （inventory 给出根域 `https://ark.cn-beijing.volces.com`，实际 OpenAI 兼容路径为 `/api/v3`）
- **鉴权**：方式=Bearer API Key（Ark API Key）/ 环境变量=未知（inventory 未给出，社区常用 `ARK_API_KEY` 或 `VOLC_API_KEY`）/ 是否必需=是
- **endpoint 公式**：`POST https://ark.cn-beijing.volces.com/api/v3/chat/completions`
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions 体；`model` 字段填接入点 endpoint id（如 `ep-xxx`）而非模型名
- **响应结构要点**：与 OpenAI Chat Completions 响应结构一致
- **流式**：SSE（`stream: true`）
- **错误结构**：与 OpenAI 共享结构基本一致（具体 body 未完整读取）
- **特有行为**：同时兼容 Anthropic Messages 协议；模型通过 endpoint id 路由；地域为 cn-beijing

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方文档明确兼容 OpenAI 接口协议，请求/响应可由 OpenAI 共享层表达；endpoint id 作为 model 字符串透传即可
- **可复用模型 ID 样例**：inventory 为空（实际使用 endpoint id，如 `ep-20240xxxxxxxx`）
- **是否需扩展共享层**：否

#### 4. 风险与限制

- base URL 需补 `/api/v3` 路径，inventory 根域不完整
- 鉴权环境变量 inventory 缺失，需补 `ARK_API_KEY`
- 模型以 endpoint id 而非模型名调用，文档/样例需说明

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：主流国内大模型网关，官方明确 OpenAI 兼容，薄封装成本低

---

### vultr — Vultr

- **canonical ID**：vultr
- **aliases**：
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat（Vultr Inference，托管开源模型）

#### 1. 官方协议证据

- **文档 URL**：https://api.vultrinference.com （官方 OpenAPI 1.1.3 规范）
- **核验来源**：官方 OpenAPI 规范
- **证据强度**：强（官方 OpenAPI 直接给出 `/chat/completions` 完整请求/响应 schema）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://api.vultrinference.com/v1
- **鉴权**：方式=API Key（OpenAPI 标 `API Key` 鉴权，按惯例 `Authorization: Bearer`）/ 环境变量=VULTR_API_KEY / 是否必需=是
- **endpoint 公式**：`POST /chat/completions`
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI 体——`model`、`messages`、`stream`、`max_tokens`、`n`、`seed`、`temperature`、`top_p`、`frequency_penalty`、`presence_penalty`、`stop`、`logprobs`、`top_logprobs`、`tool_choice`、`tools`；模型名形如 `deepseek-ai/DeepSeek-V4-Pro`
- **响应结构要点**：标准 `chat.completion`——`id`、`created`、`model`、`choices[].message{role,content,reasoning,tool_calls}`、`logprobs`、`finish_reason`、`usage{completion_tokens,prompt_tokens,total_tokens}`
- **流式**：SSE（`stream: true`）
- **错误结构**：400 Bad Request / 401 Unauthorized / 422 Validation Error（标准 HTTP 状态码）
- **特有行为**：提供 `-normalize` 后缀（如 `model-normalize`）走 normalizer 代理，修正非标准 OpenAI 响应（`reasoning_content`→`reasoning`、tool call id 格式化、`content=None`→`""` 等）；另有 RAG Chat Completion endpoint

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方 OpenAPI 规范确认请求/响应结构与 OpenAI Chat Completions 一致
- **可复用模型 ID 样例**：deepseek-ai/DeepSeek-V4-Flash、Qwen/Qwen3.6-27B、MiniMaxAI/MiniMax-M2.7
- **是否需扩展共享层**：否（`reasoning` 字段为非标准但可作为透传/兼容处理）

#### 4. 风险与限制

- `-normalize` 后缀语义非 OpenAI 标准，若需修正 reasoning_content 等需额外说明
- 响应 `reasoning` 字段（非 OpenAI 标准 `reasoning_content`）需注意兼容映射

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：官方 OpenAPI 规范完整，OpenAI 兼容明确，薄封装成本低

---

### watsonx_text — Watsonx Text

- **canonical ID**：watsonx_text
- **aliases**：
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat（IBM watsonx.ai 文本生成，对应 litellm 的 watsonx_text）

#### 1. 官方协议证据

- **文档 URL**：https://www.ibm.com/docs/en/watsonx/saas?topic=code-text-generation
- **核验来源**：官方 API 文档（IBM watsonx.ai docs）
- **证据强度**：强（官方文档明确 endpoint、鉴权、请求体结构，并标注该 API 已废弃）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://<region>.ml.cloud.ibm.com`（按区域，如 `us-south.ml.cloud.ibm.com`；inventory base_urls 为空）
- **鉴权**：方式=Bearer token（IBM IAM access token，由 API key 换取，约每小时刷新）/ 环境变量=未知（inventory 为空；社区常用 `WATSONX_APIKEY`/`WATSONX_URL`/`WATSONX_PROJECT_ID`）/ 是否必需=是
- **endpoint 公式**：`POST https://<region>.ml.cloud.ibm.com/ml/v1/text/generation?version=2025-02-11`
- **协议类型**：原生（非 OpenAI 兼容）
- **请求结构要点**：`input`（纯文本字符串，非 messages 数组）、`parameters`（如 `max_new_tokens`）、`model_id`、`project_id`；结构与 OpenAI Chat Completions 完全不同
- **响应结构要点**：返回 `results[]` 含 `generated_text` 等，非 `choices[].message`
- **流式**：SSE（Infer text event stream，`text/event-stream`）
- **错误结构**：厂商专属（IBM 标准错误响应）
- **特有行为**：需 `project_id`；鉴权需先以 API key 调 IAM 换 bearer token；该 text generation API **已废弃**，将于 2027-03-14 移除，官方建议改用 Chat API

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：原生（若必须支持该 text generation 端点）；**建议改用 watsonx Chat API 或 watsonx 的 OpenAI 兼容端点，另立 provider**
- **依据**：请求体用 `input`+`parameters`+`project_id`，鉴权为 IAM token，与 OpenAI 共享层结构不兼容
- **可复用模型 ID 样例**：ibm/granite-3-8b-instruct 等（inventory 为空）
- **是否需扩展共享层**：否（原生路径，独立实现）

#### 4. 风险与限制

- 该 text generation API 已废弃（2027-03-14 移除），投入产出比低
- 鉴权需两步（API key → IAM token），复杂度高
- 需 `project_id` 与区域配置，与 aimux 单 base_url+key 模型有差异
- inventory 无 base_url、无 env、无模型样例，元数据缺失严重

#### 5. 优先级建议

- **优先级**：P2（后续）——倾向于搁置该废弃端点，优先调研 watsonx Chat API / OpenAI 兼容端点
- **理由**：原生协议且官方已废弃，实现成本高、价值递减；建议以 watsonx 新版 Chat/OpenAI 兼容端点替代

---

### xiaomi_token_plan_ams — Xiaomi Token Plan (Europe)

- **canonical ID**：xiaomi_token_plan_ams
- **aliases**：
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat（Xiaomi MiMo 模型，token 套餐欧洲区域网关）

#### 1. 官方协议证据

- **文档 URL**：https://mimo.mi.com/docs/en-US/api/chat/openai-api （MiMo 官方开发者文档，与 inventory 给出的 platform.xiaomimimo.com 同平台）
- **核验来源**：官方 API 文档（协议与鉴权）+ 第三方 inventory（区域 base_url）
- **证据强度**：中（官方文档确认平台为 OpenAI Chat Completions 兼容、鉴权方式与请求/响应结构；但官方可读文档仅确认通用 endpoint `https://api.xiaomimimo.com/v1/chat/completions`，本条目的区域网关 `token-plan-ams.xiaomimimo.com/v1` 来自 mastra/tokenhub inventory，未在官方文档直接确认）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://token-plan-ams.xiaomimimo.com/v1 （inventory；官方文档通用 base 为 https://api.xiaomimimo.com/v1）
- **鉴权**：方式=支持两种——`api-key: $MIMO_API_KEY` 或 `Authorization: Bearer $MIMO_API_KEY` / 环境变量=官方 MIMO_API_KEY（inventory 标 XIAOMI_API_KEY）/ 是否必需=是
- **endpoint 公式**：`POST /chat/completions`（区域网关沿用同一 path）
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI 体——`model`、`messages`、`max_completion_tokens`、`temperature`、`top_p`、`stream`、`stop`、`frequency_penalty`、`presence_penalty`；扩展 `thinking:{type}` 字段
- **响应结构要点**：标准 `chat.completion`——`id`、`choices[].message{role,content,tool_calls}`、`finish_reason`、`created`、`model`、`object`、`usage{completion_tokens,prompt_tokens,total_tokens,completion_tokens_details}`
- **流式**：SSE（`stream: true`）
- **错误结构**：与 OpenAI 共享结构基本一致
- **特有行为**：`thinking` 字段控制深度思考；平台另有 TTS（mimo-v2-tts）等模态；本条目为 token 套餐欧洲区域网关

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方文档明确 OpenAI Chat Completions 兼容；区域网关仅 base_url 不同，协议一致
- **可复用模型 ID 样例**：mimo-v2.5-pro、mimo-v2.5、mimo-v2-pro、mimo-v2-omni
- **是否需扩展共享层**：否（`thinking` 字段可透传）

#### 4. 风险与限制

- 区域网关 base_url 仅来自第三方 inventory，未在官方文档直接确认，实现前建议实测
- 环境变量名不一致：官方 MIMO_API_KEY vs inventory XIAOMI_API_KEY
- 与 xiaomi_token_plan_cn / xiaomi_token_plan_sgp 为同平台区域副本，建议合并为单一 provider + 区域配置

#### 5. 优先级建议

- **优先级**：P2（后续）
- **理由**：协议已确认 OpenAI 兼容，但与 cn/sg 两个区域条目重复，建议合并实现

---

### xiaomi_token_plan_cn — Xiaomi Token Plan (China)

- **canonical ID**：xiaomi_token_plan_cn
- **aliases**：
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat（Xiaomi MiMo 模型，token 套餐中国区域网关）

#### 1. 官方协议证据

- **文档 URL**：https://mimo.mi.com/docs/en-US/api/chat/openai-api （同平台官方文档）
- **核验来源**：官方 API 文档（协议与鉴权）+ 第三方 inventory（区域 base_url）
- **证据强度**：中（同 xiaomi_token_plan_ams：官方确认平台 OpenAI 兼容；区域网关 `token-plan-cn.xiaomimimo.com/v1` 来自 inventory，未在官方文档直接确认）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://token-plan-cn.xiaomimimo.com/v1 （inventory；官方通用 base 为 https://api.xiaomimimo.com/v1）
- **鉴权**：方式=`api-key: $MIMO_API_KEY` 或 `Authorization: Bearer $MIMO_API_KEY` / 环境变量=官方 MIMO_API_KEY（inventory 标 XIAOMI_API_KEY）/ 是否必需=是
- **endpoint 公式**：`POST /chat/completions`
- **协议类型**：OpenAI 兼容
- **请求结构要点**：同 ams 条目——标准 OpenAI 体 + `thinking:{type}` 扩展
- **响应结构要点**：标准 `chat.completion` 结构
- **流式**：SSE
- **错误结构**：与 OpenAI 共享结构基本一致
- **特有行为**：中国区域网关；其余同 ams

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装（与 ams/sgp 合并为单一 provider + 区域 base_url 配置）
- **依据**：官方文档确认 OpenAI 兼容，仅区域 base_url 不同
- **可复用模型 ID 样例**：mimo-v2.5-pro、mimo-v2.5、mimo-v2-pro、mimo-v2-omni
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 区域网关 base_url 仅来自第三方 inventory，建议实测确认
- 与 ams/sgp 重复，建议合并

#### 5. 优先级建议

- **优先级**：P2（后续）
- **理由**：与 ams/sgp 同平台区域副本，合并实现即可

---

### xiaomi_token_plan_sgp — Xiaomi Token Plan (Singapore)

- **canonical ID**：xiaomi_token_plan_sgp
- **aliases**：
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat（Xiaomi MiMo 模型，token 套餐新加坡区域网关）

#### 1. 官方协议证据

- **文档 URL**：https://mimo.mi.com/docs/en-US/api/chat/openai-api （同平台官方文档）
- **核验来源**：官方 API 文档（协议与鉴权）+ 第三方 inventory（区域 base_url）
- **证据强度**：中（同上：官方确认平台 OpenAI 兼容；区域网关 `token-plan-sgp.xiaomimimo.com/v1` 来自 inventory，未在官方文档直接确认）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://token-plan-sgp.xiaomimimo.com/v1 （inventory；官方通用 base 为 https://api.xiaomimimo.com/v1）
- **鉴权**：方式=`api-key: $MIMO_API_KEY` 或 `Authorization: Bearer $MIMO_API_KEY` / 环境变量=官方 MIMO_API_KEY（inventory 标 XIAOMI_API_KEY）/ 是否必需=是
- **endpoint 公式**：`POST /chat/completions`
- **协议类型**：OpenAI 兼容
- **请求结构要点**：同 ams 条目——标准 OpenAI 体 + `thinking:{type}` 扩展
- **响应结构要点**：标准 `chat.completion` 结构
- **流式**：SSE
- **错误结构**：与 OpenAI 共享结构基本一致
- **特有行为**：新加坡区域网关；其余同 ams

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装（与 ams/cn 合并为单一 provider + 区域 base_url 配置）
- **依据**：官方文档确认 OpenAI 兼容，仅区域 base_url 不同
- **可复用模型 ID 样例**：mimo-v2.5-pro、mimo-v2.5、mimo-v2-pro、mimo-v2-omni
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 区域网关 base_url 仅来自第三方 inventory，建议实测确认
- 与 ams/cn 重复，建议合并

#### 5. 优先级建议

- **优先级**：P2（后续）
- **理由**：与 ams/cn 同平台区域副本，合并实现即可

---

### xpersona — Xpersona

- **canonical ID**：xpersona
- **aliases**：
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://www.xpersona.co/docs 、https://www.xpersona.co/api/v1/openapi/ai-public （官方 OpenAPI）
- **核验来源**：官方 API 文档 + 官方 OpenAPI
- **证据强度**：强（官方文档直接给出 `/v1/chat/completions`、Bearer 鉴权与 curl 示例）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://www.xpersona.co （endpoint 在 `/v1` 下）
- **鉴权**：方式=`Authorization: Bearer $XPERSONA_API_KEY` / 环境变量=XPERSONA_API_KEY / 是否必需=是
- **endpoint 公式**：`POST /v1/chat/completions`；另有 `GET /v1/models`、`GET /v1/pricing`、`GET /v1/usage`
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI 体——`model`、`messages`；扩展 `reasoning:{effort}` 字段
- **响应结构要点**：与 OpenAI Chat Completions 一致（具体 schema 在 OpenAPI 中，未完整读取）
- **流式**：未知（文档未在可读片段明确，按 OpenAI 兼容惯例应支持 SSE）
- **错误结构**：与 OpenAI 共享结构基本一致（未完整读取）
- **特有行为**：原生支持 OpenCode 路由；提供月度套餐（Builder/Pro/Studio）与 PAYG；有 pricing/usage 仪表盘 endpoint

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方文档与 curl 示例确认 OpenAI 兼容 `/v1/chat/completions` + Bearer 鉴权
- **可复用模型 ID 样例**：claude-fable-5、xpersona-frieren-coder、xpersona-gpt-5.5
- **是否需扩展共享层**：否（`reasoning.effort` 可透传）

#### 4. 风险与限制

- 模型数量少（3 个），含自研与聚合模型
- 套餐模式下"达到额度暂停请求"行为需注意限流处理

#### 5. 优先级建议

- **优先级**：P2（后续）
- **理由**：OpenAI 兼容明确，但规模较小、模型少，可后续纳入

---

### xunfei — Xunfei

- **canonical ID**：xunfei
- **aliases**：
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat（科大讯飞星火认知大模型）

#### 1. 官方协议证据

- **文档 URL**：https://www.xfyun.cn/doc/spark/HTTP%E8%B0%83%E7%94%A8%E6%96%87%E6%A1%A3.html （HTTP 接口官方文档）、https://www.xfyun.cn/doc/spark/Web.html （WebSocket 接口）
- **核验来源**：官方 API 文档
- **证据强度**：强（官方文档明确 HTTP 接口兼容 OpenAI SDK、请求地址、鉴权与请求/响应参数）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://spark-api-open.xf-yun.com/v1 （inventory base_urls 为空，需补此地址）
- **鉴权**：方式=`Authorization: Bearer {APIPassword}`（控制台获取 APIPassword）/ 环境变量=未知（inventory 为空；社区常用 `IFLYTEK_API_PASSWORD` 或 `SPARK_API_PASSWORD`）/ 是否必需=是
- **endpoint 公式**：`POST https://spark-api-open.xf-yun.com/v1/chat/completions`（官方注明兼容 OpenAI SDK，`base_url=https://spark-api-open.xf-yun.com/v1/`）
- **协议类型**：OpenAI 兼容（HTTP 接口）；另有 WebSocket 原生接口（`wss://spark-api.xf-yun.com/...`，需 APIKey/APISecret/AppID 签名鉴权，属原生协议）
- **请求结构要点**：标准 OpenAI 体——`model`、`messages`、`stream`、`temperature`、`max_tokens`、`top_p`、`presence_penalty`、`frequency_penalty`、`tools`、`response_format`、`user`；扩展 `top_k`、`tools[].type=web_search` 等
- **响应结构要点**：与 OpenAI Chat Completions 一致（流式用 SSE 推送）
- **流式**：SSE（`stream: true`，服务端 SSE 推送）
- **错误结构**：厂商专属（HTTP 状态码 + 错误信息，未完整读取 body 结构）
- **特有行为**：模型版本映射——`4.0Ultra`/`generalv3.5`(Max)/`max-32k`/`generalv3`(Pro)/`pro-128k`/`lite`；支持 web_search 工具与内置插件；WebSocket 接口为另一套原生协议

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装（采用 HTTP OpenAI 兼容接口）
- **依据**：官方 HTTP 文档明确兼容 OpenAI SDK，请求/响应结构与 OpenAI 一致；WebSocket 原生接口不必纳入 chat 适配
- **可复用模型 ID 样例**：4.0Ultra、generalv3.5、generalv3、pro-128k、lite、max-32k
- **是否需扩展共享层**：否（`top_k`、`web_search` 工具可透传）

#### 4. 风险与限制

- inventory 无 base_url、无 env、无模型样例，元数据缺失严重，需补全
- 鉴权用 APIPassword（非 APIKey/APISecret），与 WebSocket 接口鉴权不同，易混淆
- 模型版本字符串（generalv3.5 等）与版本名映射需文档说明

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：主流国内大模型，HTTP 接口官方明确 OpenAI 兼容，薄封装成本低；需补全 inventory 元数据

---

### zai_coding_plan — Z.AI Coding Plan

- **canonical ID**：zai_coding_plan
- **aliases**：
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat（Z.AI / 智谱 GLM Coding Plan 订阅套餐，面向编码工具）

#### 1. 官方协议证据

- **文档 URL**：https://docs.z.ai/devpack/overview 、https://docs.z.ai/devpack/quick-start 、https://docs.z.ai/devpack/tool/others
- **核验来源**：官方 API 文档
- **证据强度**：强（官方文档明确支持 OpenAI 与 Anthropic 双协议，给出 OpenAI Chat Completions 的 base_url）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://api.z.ai/api/coding/paas/v4 （OpenAI Chat Completions 协议）；Anthropic 协议为 `https://api.z.ai/api/anthropic`
- **鉴权**：方式=API Key（Z.AI API Key，按 OpenAI 兼容惯例 `Authorization: Bearer`）/ 环境变量=ZHIPU_API_KEY（inventory）/ 是否必需=是
- **endpoint 公式**：`POST https://api.z.ai/api/coding/paas/v4/chat/completions`（OpenAI 兼容 base_url 即 `.../paas/v4`）
- **协议类型**：OpenAI 兼容（同时兼容 Anthropic Messages）
- **请求结构要点**：标准 OpenAI Chat Completions 体（model、messages 等）；在编码工具中以 "OpenAI Compatible" provider 配置
- **响应结构要点**：与 OpenAI Chat Completions 一致
- **流式**：SSE（按 OpenAI 兼容惯例，未在可读片段逐字确认）
- **错误结构**：与 OpenAI 共享结构基本一致
- **特有行为**：订阅套餐有 5 小时/周配额限制；Coding Plan Key 与团队/个人套餐绑定，不与其他 Z.AI key 互通；提供 Vision/Web Search/Web Reader/Zread MCP；此为 z.ai 国际端点，区别于国内 open.bigmodel.cn

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方文档明确 OpenAI Chat Completions 协议与 base_url，请求/响应可由共享层表达
- **可复用模型 ID 样例**：glm-4.5-air、glm-4.7、glm-5-turbo、glm-5.1、glm-5.2
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 与 zhipu_v4（国内 open.bigmodel.cn）为同公司不同端点/套餐，需区分 provider
- 配额限制（5h/周）可能触发限流，需处理 429
- base_url 已含 `/api/coding/paas/v4`，薄封装需避免重复追加 `/v1`

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：官方明确 OpenAI 兼容，编码场景需求高，薄封装成本低

---

### zeldoc — Zeldoc

- **canonical ID**：zeldoc
- **aliases**：
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat（私有 LLM + LLM 路由）

#### 1. 官方协议证据

- **文档 URL**：https://docs.zeldoc.ai 、https://docs.zeldoc.ai/use-zeldoc-anywhere
- **核验来源**：官方 API 文档
- **证据强度**：强（官方文档明确 base_url、Bearer 鉴权、OpenAI 兼容 `/chat/completions` 与 curl 示例）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://api.zeldoc.ai/v1
- **鉴权**：方式=`Authorization: Bearer $ZELDOC_API_KEY` / 环境变量=ZELDOC_API_KEY / 是否必需=是
- **endpoint 公式**：`POST https://api.zeldoc.ai/v1/chat/completions`
- **协议类型**：OpenAI 兼容（同时支持 Anthropic 兼容 `/messages`）
- **请求结构要点**：标准 OpenAI 体——`model`、`messages`（官方 curl 示例）
- **响应结构要点**：与 OpenAI Chat Completions 一致（未完整读取 schema）
- **流式**：未知（按 OpenAI 兼容惯例应支持 SSE，文档可读片段未明确）
- **错误结构**：与 OpenAI 共享结构基本一致
- **特有行为**：单 key 提供 LLM 路由；支持 OpenAI 与 Anthropic 两种 API 格式

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方文档明确 OpenAI 兼容 `/chat/completions` + Bearer 鉴权 + base_url
- **可复用模型 ID 样例**：z-code
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 模型数量少（1 个 z-code），规模小
- 流式协议未在可读片段明确，实现前建议实测

#### 5. 优先级建议

- **优先级**：P2（后续）
- **理由**：OpenAI 兼容明确，但规模小、模型少，可后续纳入

---

### zenifra — Zenifra

- **canonical ID**：zenifra
- **aliases**：
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：chat（inventory 标 chat）

#### 1. 官方协议证据

- **文档 URL**：https://docs.zenifra.com （官方站点；但内容为 PaaS 部署/数据库/运维平台文档，**未发现 AI 推理 API 文档**）
- **核验来源**：仅第三方（mastra 注册表 https://mastra.ai/models/providers/zenifra 声称 OpenAI 兼容 `/chat/completions`，base `https://ai.zenifra.com/v1`，env `ZENIFRA_AI_KEY`）
- **证据强度**：弱（官方 docs.zenifra.com 无 AI 推理 API 文档；协议信息仅来自单一第三方 mastra，无法确认官方请求/响应契约）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://ai.zenifra.com/v1 （inventory / mastra；官方文档未确认）
- **鉴权**：方式=未知（mastra 示例用 `apiKey: process.env.ZENIFRA_AI_KEY`，按 OpenAI 兼容惯例推测 Bearer，未官方确认）/ 环境变量=ZENIFRA_AI_KEY / 是否必需=是
- **endpoint 公式**：`POST /chat/completions`（mastra 声称，未官方确认）
- **协议类型**：OpenAI 兼容（仅 mastra 声称，未官方确认）
- **请求结构要点**：未知（无官方契约）
- **响应结构要点**：未知（无官方契约）
- **流式**：未知
- **错误结构**：未知
- **特有行为**：mastra 列出模型 `alibaba/qwen3.6-35b-a3b`（262K 上下文）；官方 docs.zenifra.com 为部署平台，疑似 AI 推理为独立未公开服务

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：待定（若 mastra 声称属实则为薄封装，但官方契约未确认，不臆造）
- **依据**：仅第三方声称 OpenAI 兼容，官方文档缺失，证据不足
- **可复用模型 ID 样例**：alibaba/qwen3.6-35b-a3b（mastra）
- **是否需扩展共享层**：未知

#### 4. 风险与限制

- 官方 docs.zenifra.com 无 AI 推理 API 文档，无法确认协议
- 协议信息仅来自 mastra 单一第三方，按 RFC §2.1 不足以确认请求/响应契约
- base_url `ai.zenifra.com` 与主站 `docs.zenifra.com` 分离，服务存在性待验证

#### 5. 优先级建议

- **优先级**：搁置（证据不足）
- **理由**：官方文档未覆盖 AI 推理 API，仅第三方声称 OpenAI 兼容，按 RFC 不臆造协议细节，待官方文档出现后再调研

---

### zhipu_v4 — ZhipuV4

- **canonical ID**：zhipu_v4
- **aliases**：
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：chat（智谱 AI GLM 开放平台）

#### 1. 官方协议证据

- **文档 URL**：https://docs.bigmodel.cn/cn/guide/develop/http/introduction 、https://docs.bigmodel.cn/cn/api/introduction
- **核验来源**：官方 API 文档
- **证据强度**：强（官方文档明确 base_url、Bearer 鉴权、`/chat/completions` endpoint 与请求/响应示例）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://open.bigmodel.cn/api/paas/v4 （inventory 给出根域 `https://open.bigmodel.cn`）
- **鉴权**：方式=两种——`Authorization: Bearer YOUR_API_KEY`（API Key 鉴权）或 JWT Token 鉴权（用 API Key 的 id.secret 生成 HS256 JWT）/ 环境变量=未知（inventory 为空；社区常用 `ZHIPU_API_KEY`）/ 是否必需=是
- **endpoint 公式**：`POST https://open.bigmodel.cn/api/paas/v4/chat/completions`
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI 体——`model`、`messages`、`temperature`、`max_tokens`、`stream`；多轮对话用 messages 数组
- **响应结构要点**：与 OpenAI Chat Completions 一致——`choices[0].message.content`、`usage` 等
- **流式**：SSE（`stream: true`）
- **错误结构**：标准 HTTP 状态码（401 未授权 / 429 限流 / 500 服务器错误），body 厂商专属
- **特有行为**：API Key 形如 `{id}.{secret}`；JWT 鉴权为可选高安全方案；此为国内端点，区别于 z.ai 国际端点；另有 Coding 端点 `https://open.bigmodel.cn/api/coding/paas/v4`

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方文档明确 `/api/paas/v4/chat/completions` + Bearer 鉴权 + 标准 OpenAI 请求/响应结构
- **可复用模型 ID 样例**：glm-5.2、glm-5.1、glm-5-turbo、glm-4.7、glm-4.5-air（inventory 为空，按官方文档）
- **是否需扩展共享层**：否（JWT 鉴权为可选，标准 Bearer API Key 即可工作）

#### 4. 风险与限制

- inventory 无 base_url 路径、无 env、无模型样例，需补全（base 需补 `/api/paas/v4`，env 补 `ZHIPU_API_KEY`）
- 与 zai_coding_plan（z.ai 国际）为同公司不同端点，需区分
- API Key 含 `.` 分隔的 id.secret，JWT 生成逻辑若支持需额外实现（薄封装可仅用 Bearer API Key）

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：主流国内大模型，官方明确 OpenAI 兼容，薄封装成本低；需补全 inventory 元数据
