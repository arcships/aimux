# 第 10 批调研记录（14 个 provider）

> 本批按 canonical id 字母序排列。证据裁决遵循 RFC-0006 §2.1（官方文档/SDK > 成熟实现 > 多源一致 > 单一第三方）。inventory 的 tier/protocol/openai_compatible/confidence 仅为线索，不作证据依据。无法确认的字段写"未知"或留空，不臆造。

---

### poolside — Poolside

- **canonical ID**：poolside
- **aliases**：[]
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://docs.poolside.ai/api/overview
- **核验来源**：官方 API 文档
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://inference.poolside.ai/v1`（Poolside Platform）；自管部署为 `https://<api-domain>/openai/v1`
- **鉴权**：方式=Bearer token（`Authorization: Bearer <api-key>`）/ 环境变量=`POOLSIDE_API_KEY` / 是否必需=是
- **endpoint 公式**：`POST /chat/completions`；`GET /models`
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions 请求体 `{model, messages}`
- **响应结构要点**：标准 OpenAI Chat Completions 响应（`choices[].message.content`）
- **流式**：SSE（OpenAI 兼容 stream）
- **错误结构**：与 OpenAI 共享结构一致（官方明示 OpenAI-compatible）
- **特有行为**：模型 ID 形如 `poolside/laguna-s-2.1`；亦可通过 OpenRouter 访问（`https://openrouter.ai/api/v1`）；自管部署路径为 `/openai/v1`（与 Platform 的 `/v1` 不同）

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方明确"OpenAI-compatible API"，提供 OpenAI SDK 直连示例，base URL + Bearer + `/chat/completions` 完全符合 OpenAI 共享层
- **可复用模型 ID 样例**：`poolside/laguna-s-2.1`、`poolside/laguna-m.1`、`poolside/laguna-xs-2.1`
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 自管部署路径为 `/openai/v1`（与 Platform 的 `/v1` 不同），需按部署方式切换 path
- 主要面向软件工程/编码场景

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：官方 OpenAI 兼容，证据强，薄封装成本低

---

### ppinfra — PPInfra（PPIO 派欧云）

- **canonical ID**：ppinfra
- **aliases**：[]
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://ppio.com/docs/model/llm （官方 PPIO 文档；ppinfra.com 已合并升级至 ppio.com）
- **核验来源**：官方站点 + 第三方成熟实现（ragflow 集成）
- **证据强度**：中
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.ppio.com/openai`（现行）；历史 `https://api.ppinfra.com/v3/openai`
- **鉴权**：方式=Bearer API Key / 环境变量=未知（inventory 未提供；PPIO 控制台生成 API Key）/ 是否必需=是
- **endpoint 公式**：OpenAI 兼容 `/chat/completions`（及 `/completions`）
- **协议类型**：OpenAI 兼容
- **请求结构要点**：官方提供 OpenAI SDK 直连示例 `{model, messages/prompt}`
- **响应结构要点**：OpenAI 兼容响应
- **流式**：SSE（OpenAI 兼容 stream，官方示例 `stream=True`）
- **错误结构**：与 OpenAI 共享结构一致（推断）
- **特有行为**：模型 ID 形如 `deepseek/deepseek-v3-0324`、`baidu/ernie-4.5-*` 等 `vendor/model`；另提供 Anthropic 兼容端点 `https://api.ppinfra.com/anthropic`

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方明示 OpenAI SDK 直连，base URL 指向 `/openai`，请求响应 OpenAI 兼容
- **可复用模型 ID 样例**：`deepseek/deepseek-v3-0324`、`baidu/ernie-4.5-21b-a3b-thinking`
- **是否需扩展共享层**：否

#### 4. 风险与限制

- inventory 无 base_url/docs/env，识别为 PPIO（原 ppinfra.com）属推断，id 归属需二次确认
- inventory 模型样例含 `ai_infer_test_2`、`ai_infer_test_3` 等测试模型，可疑
- 现行品牌域 ppio.com 与 inventory id `ppinfra` 不一致

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：OpenAI 兼容薄封装；但 id 归属待确认，证据中

---

### qihang_ai — QiHang（启航 AI）

- **canonical ID**：qihang_ai
- **aliases**：[]
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://www.qhaigc.net/docs
- **核验来源**：官方 API 文档
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.qhaigc.net/v1`
- **鉴权**：方式=Bearer API Key（`sk-...`）/ 环境变量=`QIHANG_API_KEY` / 是否必需=是
- **endpoint 公式**：OpenAI 兼容 `/chat/completions`
- **协议类型**：OpenAI 兼容
- **请求结构要点**：官方明示"完全兼容 OpenAI API 标准"，OpenAI SDK 直连 `{model, messages}`
- **响应结构要点**：OpenAI 兼容响应
- **流式**：SSE（官方称支持流式输出）
- **错误结构**：与 OpenAI 共享结构一致
- **特有行为**：聚合网关，提供 GPT/Claude/DeepSeek/Kimi/Gemini 等多厂商模型；另支持图像/语音/视频/音乐生成、嵌入、重排序

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方文档明示"完全兼容 OpenAI API 标准"，提供 OpenAI SDK 直连示例
- **可复用模型 ID 样例**：`gpt-4o`、`claude-opus-4-5-20251101`、`gemini-2.5-flash`、`deepseek-v4-pro`
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 聚合网关，模型 availability 与上游一致性问题
- 国内服务（qhaigc.net）

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：官方明示 OpenAI 兼容，证据强，薄封装成本低

---

### routing_run — routing.run

- **canonical ID**：routing_run
- **aliases**：[]
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://docs.routing.run （官方文档；本次 WebFetch 抓取失败，疑似屏蔽爬虫）
- **核验来源**：第三方成熟实现（mastra registry）+ inventory base_url
- **证据强度**：中
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.routing.run/v1`（另 `https://ai.routing.sh/v1`）
- **鉴权**：方式=Bearer API Key / 环境变量=`ROUTING_RUN_API_KEY` / 是否必需=是
- **endpoint 公式**：OpenAI 兼容 `/chat/completions`
- **协议类型**：OpenAI 兼容
- **请求结构要点**：mastra 明示"uses the OpenAI-compatible `/chat/completions` endpoint"，OpenAI 风格 `{model, messages}`
- **响应结构要点**：OpenAI 兼容响应
- **流式**：SSE（mastra 示例支持 stream）
- **错误结构**：与 OpenAI 共享结构一致（推断）
- **特有行为**：聚合路由网关，模型 ID 形如 `routing-run/claude-opus-4-8`；部分模型有 `-nitro` 变体

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：mastra registry 明确使用 OpenAI 兼容 `/chat/completions`，base_url 以 `/v1` 结尾
- **可复用模型 ID 样例**：`claude-opus-4-8`、`deepseek-v4-pro`、`glm-5.2`、`kimi-k2.6`、`gpt-5.6-luna`
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 官方文档未能直接抓取确认，协议细节依据第三方 mastra registry
- 聚合网关，模型 availability 随上游变化

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：多源一致指向 OpenAI 兼容，薄封装；但官方文档未直接确认，证据中

---

### sagemaker_chat — Sagemaker Chat

- **canonical ID**：sagemaker_chat
- **aliases**：[]
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://docs.litellm.ai/docs/providers/aws_sagemaker （litellm 成熟实现）；AWS SageMaker 官方文档
- **核验来源**：成熟实现（litellm）+ AWS 官方
- **证据强度**：中
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://runtime.sagemaker.<region>.amazonaws.com/endpoints/<endpoint-name>/invocations`（SageMaker InvokeEndpoint）
- **鉴权**：方式=AWS SigV4 签名（`AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` / `AWS_REGION_NAME`，可选 session token/role）/ 环境变量=`AWS_ACCESS_KEY_ID` 等 / 是否必需=是
- **endpoint 公式**：`POST /endpoints/<endpoint>/invocations`（SageMaker Messages API 路由，litellm 前缀 `sagemaker_chat/`）
- **协议类型**：原生
- **请求结构要点**：SageMaker Messages API 载荷（messages 形式但 SageMaker 专属，非 OpenAI Chat Completions 原生）；litellm 做 OpenAI→SageMaker 转换
- **响应结构要点**：SageMaker 端点专属响应
- **流式**：未知/有限（litellm 早期称 SageMaker 不支持流式，靠伪流式）
- **错误结构**：AWS/SageMaker 专属
- **特有行为**：需先在 SageMaker 部署端点，model 字段为端点名；按端点计费

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：原生
- **依据**：AWS SigV4 鉴权 + SageMaker InvokeEndpoint 传输 + SageMaker Messages API 载荷，与 OpenAI 共享层结构差异显著
- **可复用模型 ID 样例**：无（依赖用户自部署端点名）
- **是否需扩展共享层**：是（需 AWS SigV4 签名 + SageMaker 端点 URL 模式，属原生实现）

#### 4. 风险与限制

- inventory 无 base_url/docs/env/models，此条目实为 litellm 路由前缀，非独立 API 服务
- 需 AWS SDK 签名，实现成本高
- 流式支持有限

#### 5. 优先级建议

- **优先级**：P2（后续）
- **理由**：原生 AWS 集成成本高，且为路由前缀而非独立服务；证据中

---

### sagemaker_nova — Sagemaker Nova

- **canonical ID**：sagemaker_nova
- **aliases**：[]
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://docs.litellm.ai/docs/providers/aws_sagemaker （litellm）；AWS 官方博客"Amazon SageMaker Inference for Custom Amazon Nova Models"
- **核验来源**：成熟实现（litellm）+ AWS 官方
- **证据强度**：中
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：SageMaker 端点 InvokeEndpoint URL（`https://runtime.sagemaker.<region>.amazonaws.com/endpoints/<endpoint>/invocations`）
- **鉴权**：方式=AWS SigV4 签名 / 环境变量=`AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` / `AWS_REGION_NAME` / 是否必需=是
- **endpoint 公式**：`POST /endpoints/<endpoint>/invocations`（litellm 前缀 `sagemaker_nova/`）
- **协议类型**：原生（请求/响应体 OpenAI 兼容，但鉴权与传输为 AWS SageMaker 原生）
- **请求结构要点**：自定义/微调 Nova 模型使用 OpenAI 兼容 Chat Completions 载荷 `{model, messages, temperature, max_tokens}`，支持多模态图像（base64 data URI）
- **响应结构要点**：OpenAI 兼容响应（`choices[].message.content`）
- **流式**：SSE（OpenAI 兼容 stream，`stream_options.include_usage`）
- **错误结构**：AWS/SageMaker 专属
- **特有行为**：需先部署 Nova 端点；model 字段为端点名；支持 Amazon Nova Micro/Lite/Nova 2 Lite

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：原生
- **依据**：虽请求响应体 OpenAI 兼容，但鉴权为 AWS SigV4、端点为 SageMaker InvokeEndpoint，结构差异显著
- **可复用模型 ID 样例**：无（依赖用户自部署端点名）
- **是否需扩展共享层**：是（需 AWS SigV4 签名 + SageMaker 端点 URL；可复用 OpenAI 载荷编解码）

#### 4. 风险与限制

- inventory 无 base_url/docs/env/models，实为 litellm 路由前缀
- 需 AWS SDK 签名
- 与新版 SageMaker `/openai/v1` OpenAI 兼容路径（2026-05 发布，Bearer）可能重叠，需厘清目标

#### 5. 优先级建议

- **优先级**：P2（后续）
- **理由**：原生 AWS 集成成本高，路由前缀而非独立服务；证据中

---

### sap_ai_core — SAP AI Core

- **canonical ID**：sap_ai_core
- **aliases**：[]
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://help.sap.com/docs/sap-ai-core （官方；本次首页抓取为空，结合 SAP Cloud SDK for AI 与官方社区交叉确认）
- **核验来源**：官方文档 + SAP Cloud SDK for AI + 官方社区
- **证据强度**：中
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.ai.<region>.<cloud>.ml.hana.ondemand.com`（SAP AI Core AI_API_URL）
- **鉴权**：方式=Bearer token（OAuth2 client credentials 换取 JWT access token，`Authorization: Bearer <token>`）/ 环境变量=`AICORE_CLIENT_ID` / `AICORE_CLIENT_SECRET` 等（inventory 未提供）/ 是否必需=是
- **endpoint 公式**：`POST /v2/inference/deployments/<deployment_id>/chat/completions`（亦有 `chat-completion` 变体）；需先 `GET /v2/lm/scenarios/foundation-models/models` 与部署获取 `deployment_id`
- **协议类型**：原生
- **请求结构要点**：`/chat/completions` 端点请求体类 OpenAI（messages），但 URL 含 `deployment_id`；不同模型/部署 URL 不同
- **响应结构要点**：类 OpenAI chat completions 响应
- **流式**：SSE（部分模型支持流式）
- **错误结构**：SAP AI Core 专属（含 status/code/message）
- **特有行为**：多步骤——需先创建部署/获取 `deployment_id` 再调用推理；模型 ID 形如 `anthropic--claude-3.5-sonnet`

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：原生
- **依据**：URL 含 `deployment_id`（每模型/部署不同）、OAuth2 client credentials 鉴权、多步骤部署查询，与 OpenAI 共享层结构差异显著
- **可复用模型 ID 样例**：`anthropic--claude-3.5-sonnet`、`anthropic--claude-3.7-sonnet`
- **是否需扩展共享层**：是（需 SAP OAuth2 鉴权 + deployment_id 路径解析 + 部署查询步骤）

#### 4. 风险与限制

- inventory 无 base_url/env，需 SAP AI Core 实例配置（client_id/secret/auth_url/resource_group）
- 多步骤、部署 ID 绑定，模型切换需重新解析 `deployment_id`
- 官方首页抓取为空，部分细节来自 SAP 社区/SDK 文档

#### 5. 优先级建议

- **优先级**：P2（后续）
- **理由**：原生多步骤协议，实现成本高；企业场景为主；证据中

---

### snowflake_cortex — Snowflake Cortex

- **canonical ID**：snowflake_cortex
- **aliases**：[]
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://docs.snowflake.com/en/user-guide/snowflake-cortex/cortex-rest-api
- **核验来源**：官方 API 文档
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://<account-identifier>.snowflakecomputing.com/api/v2/cortex/v1`
- **鉴权**：方式=Bearer token（`Authorization: Bearer <PAT/JWT/OAuth>`，推荐 Programmatic Access Token PAT）/ 环境变量=`SNOWFLAKE_CORTEX_PAT` / 是否必需=是；另需角色授权 `SNOWFLAKE.CORTEX_USER` 或 `CORTEX_REST_API_USER`
- **endpoint 公式**：`POST /chat/completions`（OpenAI 兼容）；`POST /messages`（Anthropic 兼容，仅 Claude）
- **协议类型**：OpenAI 兼容（同时提供 Anthropic Messages 兼容）
- **请求结构要点**：Chat Completions 完全遵循 OpenAI 规范，OpenAI SDK 直连 `{model, messages}`
- **响应结构要点**：OpenAI Chat Completions 响应
- **流式**：SSE（OpenAI 兼容 stream）
- **错误结构**：与 OpenAI 共享结构一致（Chat Completions 端点）
- **特有行为**：双协议（OpenAI Chat Completions + Anthropic Messages）；推理在 Snowflake 边界内；模型含 `claude-opus-5/4-7`、`claude-sonnet-4-5/4-6` 等

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方明示 Chat Completions API 遵循 OpenAI 规范，提供 OpenAI SDK 直连示例，base URL + Bearer(PAT) + `/chat/completions` 完全符合共享层
- **可复用模型 ID 样例**：`claude-sonnet-4-5`、`claude-opus-4-7`、`claude-opus-5`
- **是否需扩展共享层**：否

#### 4. 风险与限制

- base URL 含 `<account-identifier>` 占位，需用户填入账户标识
- 需 Snowflake PAT 与角色授权配置

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：官方 OpenAI 兼容，证据强，薄封装成本低

---

### sora — Sora

- **canonical ID**：sora
- **aliases**：[]
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：chat（inventory 标注有误，实为 video 生成）

#### 1. 官方协议证据

- **文档 URL**：https://developers.openai.com/api/reference/resources/videos/methods/create/ ；https://developers.openai.com/api/docs/guides/video-generation
- **核验来源**：官方 API 文档（OpenAI）
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.openai.com`
- **鉴权**：方式=Bearer API Key（`Authorization: Bearer $OPENAI_API_KEY`）/ 环境变量=`OPENAI_API_KEY` / 是否必需=是
- **endpoint 公式**：`POST /v1/videos`（创建视频生成任务）；`GET /v1/videos/{id}`（轮询状态）
- **协议类型**：专用模态（video）
- **请求结构要点**：multipart/form-data，`{model: sora-2|sora-2-pro, prompt, seconds: 4|8|12, size, input_reference}`
- **响应结构要点**：Video 对象 `{id, object:"video", model, status: queued|in_progress|completed|failed, progress, created_at, size, seconds}`
- **流式**：无（异步任务，通过轮询或 webhook `video.completed`/`video.failed`）
- **错误结构**：OpenAI 错误结构 `{error: {code, message}}`
- **特有行为**：异步视频生成；任务状态机；输出为视频文件（`expires_at` 过期）

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：模态专用
- **依据**：视频生成异步任务协议，与 Chat Completions 结构完全不同，属单一模态专用实现
- **可复用模型 ID 样例**：`sora-2`、`sora-2-pro`
- **是否需扩展共享层**：否（独立模态实现）

#### 4. 风险与限制

- inventory 将能力标为 chat 有误，实为 video
- 异步任务需轮询/webhook 状态机
- 实为 OpenAI 官方视频 API，非独立 provider

#### 5. 优先级建议

- **优先级**：P2（后续）
- **理由**：模态专用（视频），与 chat 共享层无关；非独立 provider；按视频模态统一规划

---

### stackit — STACKIT

- **canonical ID**：stackit
- **aliases**：[]
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://docs.stackit.cloud/products/data-and-ai/ai-model-serving/basics/available-shared-models
- **核验来源**：官方文档
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.openai-compat.model-serving.eu01.onstackit.cloud/v1`
- **鉴权**：方式=Bearer API Key（OpenAI 兼容）/ 环境变量=`STACKIT_API_KEY` / 是否必需=是
- **endpoint 公式**：`POST /chat/completions`；`POST /completions`；`GET /models`
- **协议类型**：OpenAI 兼容
- **请求结构要点**：官方明示"Specification: OpenAI-compatible"，OpenAI SDK 直连 `{model, messages}`
- **响应结构要点**：OpenAI 兼容响应
- **流式**：SSE（OpenAI 兼容，推断）
- **错误结构**：与 OpenAI 共享结构一致
- **特有行为**：模型 ID 形如 `Qwen/Qwen3-VL-235B-A22B-Instruct-FP8`、`cortecs/Llama-3.3-70B-Instruct-FP8-Dynamic`；支持 tool calling、reasoning；按模型设 TPM/RPM 限额

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方明示 OpenAI-compatible，提供 `/chat/completions` `/completions` `/models` 端点
- **可复用模型 ID 样例**：`Qwen/Qwen3-VL-235B-A22B-Instruct-FP8`、`Qwen/Qwen3.6-27B`、`cortecs/Llama-3.3-70B-Instruct-FP8-Dynamic`、`google/gemma-3-27b-it`
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 区域固定 eu01（欧洲）
- 部分模型有图像数量、TPM/RPM 限额

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：官方 OpenAI 兼容，证据强，薄封装成本低

---

### stepfun_ai_step_plan — StepFun Step Plan (Global)

- **canonical ID**：stepfun_ai_step_plan
- **aliases**：[]
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://platform.stepfun.ai/docs/en/step-plan/integrations/reasoning-api
- **核验来源**：官方 API 文档
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.stepfun.ai/step_plan/v1`
- **鉴权**：方式=Bearer API Key（`Authorization: Bearer $STEP_API_KEY`）/ 环境变量=`STEPFUN_API_KEY`（inventory）/ `STEP_API_KEY`（官方文档）/ 是否必需=是
- **endpoint 公式**：`POST /step_plan/v1/chat/completions`（OpenAI 协议）；`POST /step_plan/v1/messages`（Anthropic 协议）
- **协议类型**：OpenAI 兼容（同时提供 Anthropic Messages 兼容）
- **请求结构要点**：OpenAI Chat Completions 载荷 `{model, messages, reasoning_effort: low|medium|high}`；Anthropic 路径用 `output_config.effort`
- **响应结构要点**：OpenAI 兼容响应
- **流式**：SSE（OpenAI 兼容 stream）
- **错误结构**：与 OpenAI 共享结构一致
- **特有行为**：固定域名 `api.stepfun.ai` + `/step_plan/v1` 前缀；推理强度字段 `reasoning_effort`；需订阅 Step Plan

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方明示 Chat Completion（OpenAI 协议），提供 OpenAI SDK 直连示例
- **可复用模型 ID 样例**：`step-3.7-flash`、`step-3.5-flash-2603`、`step-3.5-flash`
- **是否需扩展共享层**：否（`reasoning_effort` 为 OpenAI 标准扩展字段，共享层应已支持）

#### 4. 风险与限制

- 路径前缀 `/step_plan/v1` 而非 `/v1`
- env var 名称官方为 `STEP_API_KEY`，inventory 的 `STEPFUN_API_KEY` 待统一
- 需订阅 Step Plan

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：官方 OpenAI 兼容，证据强，薄封装成本低

---

### stepfun_step_plan — StepFun Step Plan (China)

- **canonical ID**：stepfun_step_plan
- **aliases**：[]
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://platform.stepfun.com/docs/zh/step-plan/integrations/reasoning-api
- **核验来源**：官方 API 文档
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.stepfun.com/step_plan/v1`
- **鉴权**：方式=Bearer API Key（`Authorization: Bearer $STEP_API_KEY`）/ 环境变量=`STEPFUN_API_KEY`（inventory）/ `STEP_API_KEY`（官方文档）/ 是否必需=是
- **endpoint 公式**：`POST /step_plan/v1/chat/completions`（OpenAI 协议）；`POST /step_plan/v1/messages`（Anthropic 协议）
- **协议类型**：OpenAI 兼容（同时提供 Anthropic Messages 兼容）
- **请求结构要点**：OpenAI Chat Completions 载荷 `{model, messages, reasoning_effort: low|medium|high}`
- **响应结构要点**：OpenAI 兼容响应
- **流式**：SSE（OpenAI 兼容 stream）
- **错误结构**：与 OpenAI 共享结构一致
- **特有行为**：国内域名 `api.stepfun.com` + `/step_plan/v1` 前缀；额外模型 `step-router-v1`（自动在 `deepseek-v4-pro` 与 `step-3.7-flash` 间路由，字段约束有差异）

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方明示 Chat Completion（OpenAI 协议），提供 OpenAI SDK 直连示例
- **可复用模型 ID 样例**：`step-3.7-flash`、`step-3.5-flash-2603`、`step-3.5-flash`、`step-router-v1`
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 路径前缀 `/step_plan/v1` 而非 `/v1`
- `step-router-v1` 有字段约束（`max_tokens` 上限、不支持的内容类型等）
- 国内服务，与全球版 `stepfun_ai_step_plan` 协议一致仅域名不同

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：官方 OpenAI 兼容，证据强；与全球版可共享实现

---

### sub2_api — Sub2API

- **canonical ID**：sub2_api
- **aliases**：[]
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：无（inventory 未提供；WebSearch 未找到官方文档）
- **核验来源**：无
- **证据强度**：无
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
- **特有行为**："sub2" 在第三方语境中似为账号/凭据格式标签（json 不带账密），非可识别的官方 API provider；inventory 无 base_url/docs/env/models

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：待定
- **依据**：无任何协议证据，无法判定
- **可复用模型 ID 样例**：无
- **是否需扩展共享层**：未知

#### 4. 风险与限制

- 无法确认是否为真实独立 provider
- 可能为凭据/账号格式或代理转售生态标签

#### 5. 优先级建议

- **优先级**：搁置
- **理由**：无官方文档、无 base_url、无 env、无模型，证据不足

---

### subconscious — Subconscious

- **canonical ID**：subconscious
- **aliases**：[]
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://docs.subconscious.dev
- **核验来源**：官方 API 文档
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.subconscious.dev/v1`（OpenAI 兼容）；Anthropic 路径 `https://api.subconscious.dev`
- **鉴权**：方式=Bearer API Key（OpenAI 路径 `Authorization: Bearer`）；Anthropic 路径 `x-api-key` / 环境变量=`SUBCONSCIOUS_API_KEY` / 是否必需=是
- **endpoint 公式**：`POST /v1/chat/completions`（OpenAI 协议）；`POST /v1/messages`（Anthropic 协议，需 `anthropic-version` 头）
- **协议类型**：OpenAI 兼容（同时提供 Anthropic Messages 兼容）
- **请求结构要点**：OpenAI Chat Completions 载荷 `{model, messages}`，OpenAI SDK 直连
- **响应结构要点**：OpenAI 兼容响应（`choices[].message.content`）
- **流式**：SSE（OpenAI 兼容 stream，推断）
- **错误结构**：与 OpenAI 共享结构一致
- **特有行为**：双协议（OpenAI + Anthropic）；模型 ID 形如 `subconscious/tim-qwen3.6-27b`、`subconscious/glm-5.2`；自研 TIM 模型 + TIMRUN 推理运行时

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方明示"compatible with both the OpenAI Completions and Anthropic Messages APIs"，提供 OpenAI SDK 直连示例
- **可复用模型 ID 样例**：`subconscious/tim-qwen3.6-27b`、`subconscious/glm-5.2`
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 模型数量少（2）
- Anthropic 路径鉴权头与 OpenAI 路径不同（`x-api-key` vs `Bearer`）

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：官方 OpenAI 兼容，证据强，薄封装成本低
