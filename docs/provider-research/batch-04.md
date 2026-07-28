# 第 4 批调研记录（14 个 provider）

> 调研日期：2026-07-28
> 核验原则：以厂商官方 API 文档/SDK 为准；inventory 元数据仅作线索。证据不足者标"无"并建议搁置，不臆造。

---

### alibaba_coding_plan — Alibaba Coding Plan

- **canonical ID**：alibaba_coding_plan
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://www.alibabacloud.com/help/en/model-studio/coding-plan
- **核验来源**：官方 API 文档（阿里云 Model Studio）
- **证据强度**：强（官方文档明确给出 OpenAI 兼容协议 base URL 与 API Key 格式）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://coding-intl.dashscope.aliyuncs.com/v1`（OpenAI 兼容协议）；另提供 Anthropic 兼容协议 `https://coding-intl.dashscope.aliyuncs.com/apps/anthropic`
- **鉴权**：方式=HTTP Bearer（OpenAI 兼容协议标准） / 环境变量=`ALIBABA_CODING_PLAN_API_KEY` / 是否必需=是；API Key 格式为 `sk-sp-xxxxx`（套餐专属，与百炼按量付费 `sk-xxxxx` 不互通）
- **endpoint 公式**：`POST {base_url}/chat/completions`（OpenAI 兼容）
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions（model/messages/stream/tools 等）
- **响应结构要点**：标准 OpenAI Chat Completions 响应
- **流式**：SSE（OpenAI 兼容标准）
- **错误结构**：与 OpenAI 共享结构一致（OpenAI 兼容协议）；可能含 Dashscope 特定错误码
- **特有行为**：套餐按月订阅、5 小时/周/月配额滚动；**官方明确"严禁 API 调用"**，仅限在编程工具（Claude Code、Cursor、Cline 等）中交互式使用，禁止自动化脚本/批量调用，违例可能封禁

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方文档明确 OpenAI 兼容协议 + base URL + Bearer 鉴权，请求/响应结构对齐 OpenAI Chat Completions
- **可复用模型 ID 样例**：qwen3.7-plus、qwen3.6-plus、kimi-k2.5、glm-5、MiniMax-M2.5、qwen3-coder-next、qwen3-coder-plus、glm-4.7
- **是否需扩展共享层**：否

#### 4. 风险与限制

- ToS 风险：官方禁止直接 API 调用/自动化批量调用，仅允许编程工具交互式使用；aimux 若用于编程工具场景属预期用途，但需向用户明示合规边界
- 套餐专属 API Key（`sk-sp-`）与百炼通用 Key 不互通，base URL 必须用含 `coding` 的专属域名

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：证据强、薄封装；但因 ToS 限制仅限编程工具交互式用途，非通用 API 接入，降为 P1

---

### alibaba_coding_plan_cn — Alibaba Coding Plan (China)

- **canonical ID**：alibaba_coding_plan_cn
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://help.aliyun.com/zh/model-studio/coding-plan
- **核验来源**：官方 API 文档（阿里云百炼 Model Studio 中国站）
- **证据强度**：强（官方文档明确给出 OpenAI 兼容协议 base URL 与 API Key 格式）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://coding.dashscope.aliyuncs.com/v1`（OpenAI 兼容协议）；另提供 Anthropic 兼容协议 `https://coding.dashscope.aliyuncs.com/apps/anthropic`
- **鉴权**：方式=HTTP Bearer / 环境变量=`ALIBABA_CODING_PLAN_API_KEY` / 是否必需=是；API Key 格式 `sk-sp-xxxxx`
- **endpoint 公式**：`POST {base_url}/chat/completions`（OpenAI 兼容）
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions
- **响应结构要点**：标准 OpenAI Chat Completions 响应
- **流式**：SSE（OpenAI 兼容标准）
- **错误结构**：与 OpenAI 共享结构一致；可能含 Dashscope 特定错误码
- **特有行为**：与 alibaba_coding_plan 完全同构，仅为中国区域名（`coding.dashscope.aliyuncs.com`）；同样禁止直接 API 调用

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方文档明确 OpenAI 兼容协议；与 alibaba_coding_plan 仅 base URL 域名差异，可复用同一薄封装（按 region/profile 切换 base URL）
- **可复用模型 ID 样例**：qwen3.7-plus、qwen3.6-plus、kimi-k2.5、glm-5、MiniMax-M2.5、qwen3-coder-next、glm-4.7
- **是否需扩展共享层**：否（建议与 alibaba_coding_plan 共用实现，仅 base URL 不同）

#### 4. 风险与限制

- 同 alibaba_coding_plan：ToS 限制（仅限编程工具交互式用途）、套餐专属 Key/域名不互通

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：证据强、薄封装，可与 alibaba_coding_plan 同期实现；ToS 限制同上

---

### ambient — Ambient

- **canonical ID**：ambient
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://docs.ambient.xyz/（官方文档，如 Headless x402 Subscriptions 页明确给出 OpenAI 兼容配置）
- **核验来源**：官方文档 + 官网首页
- **证据强度**：强（官方文档明确 "configure OpenAI-compatible clients with: Base URL: https://api.ambient.xyz/v1, Authorization: Bearer <api_key>"）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.ambient.xyz/v1`（OpenAI 兼容）；官网另称同时提供 Anthropic 兼容端点
- **鉴权**：方式=HTTP Bearer / 环境变量=`AMBIENT_API_KEY` / 是否必需=是（API Key 在 app.ambient.xyz/keys 获取）
- **endpoint 公式**：`POST {base_url}/chat/completions`（OpenAI 兼容）
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions（官网称 "Point your OpenAI or Anthropic SDK at Ambient, no rewrite required"）
- **响应结构要点**：标准 OpenAI Chat Completions 响应
- **流式**：SSE（OpenAI 兼容标准）
- **错误结构**：与 OpenAI 共享结构一致（OpenAI 兼容）
- **特有行为**：可验证推理（Proof of Logits，密码学证明模型身份）；按 token 计费

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方文档明确 OpenAI 兼容 base URL + Bearer 鉴权，可直连 OpenAI SDK
- **可复用模型 ID 样例**：ambient/large、deepseek/deepseek-v4-flash、moonshotai/kimi-k2.6、moonshotai/kimi-k2.7-code、stepfun/step-3.7-flash
- **是否需扩展共享层**：否

#### 4. 风险与限制

- Proof of Logits 验证为附加能力，OpenAI 兼容层不暴露；基础 chat 不受影响
- 相对较新服务，模型目录以开放模型为主

#### 5. 优先级建议

- **优先级**：P0（立即）
- **理由**：证据强、薄封装、有可用模型 ID、无 ToS 风险，且可验证推理具备差异化价值

---

### anthropic_text — Anthropic Text

- **canonical ID**：anthropic_text
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat（实际为 text completions）

#### 1. 官方协议证据

- **文档 URL**：https://platform.claude.com/docs/en/api/completions/create（Anthropic 官方 Text Completions API 参考）
- **核验来源**：官方 API 文档（Anthropic）+ litellm provider 映射
- **证据强度**：中（官方文档确认 Text Completions API 协议；但该 API 已声明 legacy，且 inventory 条目极稀疏，仅 litellm_constants 来源，无 base_url/环境变量）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.anthropic.com`（Anthropic 官方；inventory 未提供，依官方文档推断）
- **鉴权**：方式=`x-api-key` 头 + `anthropic-version` 头 / 环境变量=未知（inventory 未提供）/ 是否必需=是
- **endpoint 公式**：`POST /v1/complete`（legacy Text Completions）
- **协议类型**：原生（Anthropic 原生 Text Completions，区别于 Messages API）
- **请求结构要点**：prompt/completion 式（非 messages），含 `model`、`prompt`、`max_tokens_to_sample`、`stream` 等
- **响应结构要点**：Anthropic text completion 响应（`completion` 字段）
- **流式**：SSE（Anthropic 事件流）
- **错误结构**：厂商专属（Anthropic 错误结构）
- **特有行为**：legacy API；官方明确"未来模型与特性不再支持 Text Completions，建议使用 Messages API"；对 Claude 3 及以后模型基本不支持

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：原生
- **依据**：Anthropic 原生协议（x-api-key + anthropic-version + /v1/complete + prompt/completion 结构），与 OpenAI Chat Completions 结构性不同
- **可复用模型 ID 样例**：无（legacy，仅极旧模型支持）
- **是否需扩展共享层**：是（需独立的 Anthropic text completions 解析路径，与 Messages API 不同）

#### 4. 风险与限制

- 已声明 legacy/弃用，新模型不支持；实际可用模型极少
- 与现有/未来的 anthropic（Messages）provider 高度重叠，价值低
- inventory 条目无 base_url/环境变量/模型，信息不足以独立确认接入意图

#### 5. 优先级建议

- **优先级**：搁置
- **理由**：legacy 弃用端点、无可用新模型、与 anthropic Messages provider 重叠、价值低；若未来确需可走原生路径

---

### anyapi — AnyAPI

- **canonical ID**：anyapi
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://docs.anyapi.ai
- **核验来源**：官方文档（含 curl 示例）
- **证据强度**：强（官方文档 curl 示例明确 `POST https://api.anyapi.ai/v1/chat/completions` + `Authorization: Bearer $ANYAPI_KEY` + 标准 OpenAI 请求体）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.anyapi.ai/v1`
- **鉴权**：方式=HTTP Bearer / 环境变量=文档用 `ANYAPI_KEY`，inventory 用 `ANYAPI_API_KEY`（建议以官方 `ANYAPI_KEY` 为准并兼容别名）/ 是否必需=是
- **endpoint 公式**：`POST /v1/chat/completions`
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions（`model`、`messages` 等；模型形如 `openai/gpt-4-turbo`、`anthropic/claude-*`）
- **响应结构要点**：标准 OpenAI Chat Completions 响应（`choices[0].delta.content` 等）
- **流式**：SSE（OpenAI 兼容标准，推断）；官方文档另明确提供 WebSocket 流式 `wss://api.anyapi.ai/v1/stream`
- **错误结构**：与 OpenAI 共享结构一致（OpenAI 兼容）
- **特有行为**：统一聚合 400+ 模型，智能路由/故障转移；按 anyToken 计费

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方 curl 示例确认 OpenAI Chat Completions 请求/响应契约 + Bearer 鉴权
- **可复用模型 ID 样例**：anthropic/claude-haiku-4-5、anthropic/claude-opus-4-6、anthropic/claude-opus-4-7、anthropic/claude-sonnet-4-5、anthropic/claude-sonnet-4-6
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 文档未显式给出 SSE `stream:true` 示例（仅 WebSocket 流式示例）；SSE 为 OpenAI 兼容推断，建议实现时验证
- 环境变量名 inventory（ANYAPI_API_KEY）与官方（ANYAPI_KEY）不一致
- 第三方聚合网关，稳定性/计费透明度依赖平台

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：证据强、薄封装、模型丰富；为聚合网关，价值中等，列 P1

---

### atomic_chat — Atomic Chat

- **canonical ID**：atomic_chat
- **aliases**：无
- **provider_kind**：local_runtime
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://atomic.chat（官网）+ https://github.com/AtomicBot-ai/Atomic-Chat（官方 GitHub README）
- **核验来源**：官方 GitHub README + 官方博客指南
- **证据强度**：强（官方 GitHub 明确 "Atomic Chat runs an OpenAI-compatible server at http://localhost:1337/v1 — a drop-in replacement for the OpenAI SDK"）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`http://127.0.0.1:1337/v1`（本地服务器，默认绑定 loopback）
- **鉴权**：方式=无（本地服务器，"No servers, no API keys"）/ 环境变量=inventory 给 `ATOMIC_CHAT_API_KEY`（本地场景实际非必需）/ 是否必需=否
- **endpoint 公式**：`POST {base_url}/chat/completions`（OpenAI 兼容）
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions（OpenAI SDK drop-in）
- **响应结构要点**：标准 OpenAI Chat Completions 响应
- **流式**：SSE（OpenAI 兼容标准）
- **错误结构**：与 OpenAI 共享结构一致（OpenAI 兼容）
- **特有行为**：需先在 Atomic Chat 桌面端加载模型，再由本地服务器对外暴露；支持 GGUF/MLX/ONNX；内置 Google TurboQuant 加速

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方明确 OpenAI 兼容 drop-in 服务器 + base URL，请求/响应结构对齐 OpenAI
- **可复用模型 ID 样例**：Meta-Llama-3_1-8B-Instruct-GGUF、Qwen3_5-9B-MLX-4bit、Qwen3_5-9B-Q4_K_M、gemma-4-E4B-it-IQ4_XS、gemma-4-E4B-it-MLX-4bit
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 本地运行时：用户须先安装并运行 Atomic Chat 桌面端并加载模型，否则端点不可用
- 仅监听 loopback，远程访问需手动改绑定
- 场景较窄（本地推理桌面应用的附属服务器）

#### 5. 优先级建议

- **优先级**：P2（后续）
- **理由**：证据强、薄封装，但属本地运行时、需用户自起桌面端、场景较窄，价值低于云端 provider

---

### auriko — Auriko

- **canonical ID**：auriko
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://docs.auriko.ai（含 OpenAI 兼容页 https://docs.auriko.ai/openai-compatibility、OpenAPI spec、llms.txt）
- **核验来源**：官方文档 + OpenAPI 规范
- **证据强度**：强（官方文档明确 "OpenAI-compatible: both Chat Completions and the Response API work through Auriko"，含 OpenAI SDK 直连示例）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.auriko.ai/v1`
- **鉴权**：方式=HTTP Bearer / 环境变量=`AURIKO_API_KEY` / 是否必需=是
- **endpoint 公式**：`POST /v1/chat/completions`（OpenAI 兼容）；另支持 Response API `/v1/responses`（preview）
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions（model/messages/tools/response_format 等）
- **响应结构要点**：标准 OpenAI Chat Completions 响应
- **流式**：SSE（OpenAI 兼容标准，官方确认 streaming 可用）
- **错误结构**：与 OpenAI 共享结构一致；厂商专属错误码（如 `budget_exhausted` 映射为 429 `rate_limit_error`）
- **特有行为**：LLM 路由层（成本/延迟/质量优化、自动故障转移、prompt 缓存、预算管理、BYOK）；响应头含 `request_id`、限流头、credit 用量；legacy `functions`/`function_call` 自动转 `tools`/`tool_choice`

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方文档明确 OpenAI 兼容 + base URL + Bearer 鉴权 + OpenAI SDK 直连示例；核心请求/响应契约对齐 OpenAI
- **可复用模型 ID 样例**：claude-opus-4-6、claude-opus-4-7、claude-sonnet-4-6、deepseek-v4-flash、deepseek-v4-pro
- **是否需扩展共享层**：否（路由/预算通过模型名与配置驱动，不在请求体破坏 OpenAI 契约）

#### 4. 风险与限制

- 零加价转售第三方模型，依赖上游 provider 可用性
- Response API 仍为 preview
- 路由元数据/credit 用量在响应头，OpenAI 共享层默认不解析（如需可后补）

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：证据强、薄封装、文档完善（含 OpenAPI）；为路由聚合层，价值中等

---

### aws — AWS

- **canonical ID**：aws
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://docs.aws.amazon.com/bedrock/latest/userguide/endpoints.html（AWS Bedrock 官方文档）
- **核验来源**：官方文档（AWS Bedrock）
- **证据强度**：中（官方文档充分描述 Bedrock 协议；但 inventory 条目极稀疏——无 base_url/环境变量/模型/文档 URL，仅 source=new_api，"aws" 具体所指服务面存在歧义，默认推断为 AWS Bedrock）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://bedrock-runtime.<region>.amazonaws.com`（原生 Invoke API）；新近 Bedrock Mantle 端点提供 OpenAI 兼容面（`bedrock-mantle.<region>.amazonaws.com`）
- **鉴权**：方式=AWS SigV4 签名（Access Key + Secret Key + Session Token，非 Bearer key）/ 环境变量=未知（inventory 未提供，标准 AWS 凭证链）/ 是否必需=是
- **endpoint 公式**：原生 `POST /model/{modelId}/invoke` 与 `/invoke-with-response-stream`；Mantle 提供 OpenAI 兼容 `/chat/completions`、`/responses` 与 Anthropic `/messages`
- **协议类型**：原生（原生 Invoke API：SigV4 + 区域端点 + 模型专属请求体 + EventStream 流式）；Mantle 面为 OpenAI 兼容但仍需 SigV4 鉴权
- **请求结构要点**：原生面按模型族不同（Anthropic Messages、Llama、Titan 等各自结构）；Mantle 面为标准 OpenAI/Anthropic 结构
- **响应结构要点**：原生面按模型族不同；Mantle 面为标准 OpenAI/Anthropic 响应
- **流式**：原生 EventStream（`invoke-with-response-stream`）；Mantle 面 SSE
- **错误结构**：厂商专属（AWS 错误结构）
- **特有行为**：区域化端点、SigV4 签名、模型 ID 含版本后缀、跨区域推理（cross-region inference profile）

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：原生
- **依据**：鉴权（SigV4）与 OpenAI Bearer key 结构性不同，区域端点 + 模型专属 payload + EventStream 流式均为原生协议；即便 Mantle OpenAI 兼容面仍需 SigV4，不能由纯薄封装表达
- **可复用模型 ID 样例**：无（inventory 未提供；原生面需 `anthropic.claude-*`、`meta.llama-*` 等 Bedrock 模型 ID）
- **是否需扩展共享层**：是（需 SigV4 签名能力、区域端点路由、模型族 payload 适配、EventStream 解析——属 core 契约/能力扩展）

#### 4. 风险与限制

- inventory 条目信息不足（无 base_url/env/模型/文档），"aws" 所指服务面需确认（推断为 Bedrock）
- 原生 Bedrock 实现工作量大（SigV4 + 区域 + 模型族适配 + EventStream）
- 即便采用 Mantle OpenAI 兼容面，SigV4 鉴权仍是共享层无法直接覆盖的差异

#### 5. 优先级建议

- **优先级**：P2（后续）
- **理由**：协议证据中（官方文档充分），但为高成本原生实现（SigV4/区域/模型族/EventStream），且 inventory 条目需补充澄清；价值高但应排在薄封装批次之后

---

### baidu_v2 — BaiduV2

- **canonical ID**：baidu_v2
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://intl.cloud.baidu.com/doc/qianfan/index.html（千帆官方）+ https://cloud.baidu.com/doc/qianfan-api/s/3m7of64lb（千帆 API 文档）
- **核验来源**：官方文档 + 多来源一致（OpenClaw provider 文档、社区实践）
- **证据强度**：强（官方文档明确提供 OpenAI 兼容 SDK；OpenClaw 官方文档明确 "OpenAI-compatible (openai-completions), Base URL: https://qianfan.baidubce.com/v2"）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://qianfan.baidubce.com/v2`（OpenAI 兼容；**注意：inventory 给的 `https://qianfan.baidubce.com` 缺 `/v2` 路径，需修正**）
- **鉴权**：方式=HTTP Bearer（API Key）/ 环境变量=inventory 未提供，官方/OpenClaw 用 `QIANFAN_API_KEY` / 是否必需=是；Key 格式 `bce-v3/ALTAK-...`
- **endpoint 公式**：`POST /v2/chat/completions`（OpenAI 兼容）
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions（model/messages/stream/tools 等）；官方称通过 OpenAI-compatible transport path，provider 专属参数或不可转发
- **响应结构要点**：标准 OpenAI Chat Completions 响应
- **流式**：SSE（OpenAI 兼容标准）
- **错误结构**：与 OpenAI 共享结构一致（OpenAI 兼容面）
- **特有行为**：千帆 v2 为 OpenAI 兼容面（区别于 v1 原生 AK/SK 签名面）；模型含 ERNIE、DeepSeek 等

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方 + OpenClaw 多来源确认 v2 为 OpenAI 兼容 + base URL `/v2` + Bearer API Key
- **可复用模型 ID 样例**：deepseek-v4-pro、ernie-5.1、ernie-5.0、deepseek-v3.2（去掉 `qianfan/` 前缀）
- **是否需扩展共享层**：否

#### 4. 风险与限制

- inventory base_url 缺 `/v2`，须以官方 `/v2` 为准
- 须用 v2 OpenAI 兼容面 + `bce-v3/` 格式 API Key，勿与 v1 原生 AK/SK 面混淆
- provider 专属参数在 OpenAI 兼容面可能不被转发

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：证据强、薄封装、有可用模型；仅需修正 base_url 为 `/v2` 并用 Bearer API Key

---

### bailing — Bailing

- **canonical ID**：bailing
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://developer.ant-ling.com/zh-CN/docs/api-reference/openai/（蚂蚁百灵官方开发者文档）
- **核验来源**：官方 API 文档（含完整请求/响应示例）
- **证据强度**：强（官方文档明确 "接口格式与 OpenAI chat/completions 完全兼容，可直接使用 OpenAI SDK 接入"，含 cURL/Python/Node 示例与 SSE 响应样例）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.ant-ling.com/v1`（官方开发者文档；**注意：inventory 给的 `https://api.tbox.cn/api/llm/v1/chat/completions` 为旧 TBox 平台端点，与官方百灵直连端点不一致，建议以官方 `api.ant-ling.com` 为准并核实**）
- **鉴权**：方式=HTTP Bearer / 环境变量=`BAILING_API_TOKEN` / 是否必需=是（令牌在 chat.ant-ling.com/open 创建）
- **endpoint 公式**：`POST /v1/chat/completions`（OpenAI 兼容）
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions（model/messages/stream/tools/temperature/top_p）+ 厂商特有可选字段 `enable_search`、`search_options`、`reasoning`（仅 Ring）、`thinking`（仅 Ling-3.0-flash）
- **响应结构要点**：标准 OpenAI Chat Completions 响应；流式 `object: "chat.completion.chunk"`、`data:{...}`
- **流式**：SSE（OpenAI 兼容标准，官方示例确认）
- **错误结构**：与 OpenAI 共享结构一致（OpenAI 兼容）
- **特有行为**：非流式调用超时 90 秒，长任务建议开 stream；`reasoning.effort`（high/xhigh）、`thinking.type`（enabled/disabled）为模型特有控制

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装（核心 OpenAI 契约完整兼容）；若需暴露 `enable_search`/`reasoning`/`thinking` 特有字段，则升级为共享层扩展
- **依据**：官方文档明确 OpenAI chat/completions 完全兼容 + Bearer 鉴权 + 完整请求/响应示例；特有字段均为非必填，不影响核心契约
- **可复用模型 ID 样例**：Ling-3.0-flash、Ling-2.6-1T、Ling-2.6-flash、Ring-2.6-1T（inventory 的 Ling-1T/Ring-1T 已过时）
- **是否需扩展共享层**：否（核心）；是（若暴露联网搜索/推理控制特有字段）

#### 4. 风险与限制

- inventory base_url（api.tbox.cn）与官方百灵端点（api.ant-ling.com）不一致，须以官方为准并确认是否为同一服务的两个入口
- inventory 模型样例（Ling-1T/Ring-1T）已过时，须用官方当前模型 ID
- 特有字段（enable_search/reasoning/thinking）需共享层扩展方能完整支持

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：证据强、核心 OpenAI 兼容、薄封装可行；须先核实 base_url 入口与模型 ID

---

### berget — Berget.AI

- **canonical ID**：berget
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://api.berget.ai/（官方 API 参考）
- **核验来源**：官方 API 参考
- **证据强度**：强（官方 API 参考首页明确 "OpenAI-compatible inference API for chat completions, embeddings, audio transcription, and model discovery. This endpoint is compatible with OpenAI's models ..."）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.berget.ai/v1`
- **鉴权**：方式=HTTP Bearer（OpenAI 兼容标准）/ 环境变量=`BERGET_API_KEY` / 是否必需=是
- **endpoint 公式**：`POST /v1/chat/completions`（OpenAI 兼容）；另支持 embeddings、audio transcription、models 列表
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions
- **响应结构要点**：标准 OpenAI Chat Completions 响应
- **流式**：SSE（OpenAI 兼容标准）
- **错误结构**：与 OpenAI 共享结构一致（OpenAI 兼容）
- **特有行为**：OpenAI 兼容推理平台，模型形如 `google/`、`meta-llama/`、`mistralai/`、`moonshotai/` 命名空间

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方 API 参考明确 OpenAI 兼容 + base URL，请求/响应结构对齐 OpenAI
- **可复用模型 ID 样例**：google/gemma-4-31B-it、meta-llama/Llama-3.3-70B-Instruct、mistralai/Mistral-Medium-3.5-128B、mistralai/Mistral-Small-3.2-24B-Instruct-2506、moonshotai/Kimi-K2.6
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 官方 API 参考首页内容较简，详细端点字段未在本核验中逐项展开（建议实现时对照官方 OpenAPI/参考补全）
- 第三方推理平台，依赖其上游模型可用性

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：证据强、薄封装、有可用模型；与同类 OpenAI 兼容聚合平台并列 P1

---

### blueclaw — Blue Claw

- **canonical ID**：blueclaw
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://blueclaw.network（官网，含 Quick Start）
- **核验来源**：官方官网（含代码示例）
- **证据强度**：强（官网明确 "Blue Claw is an OpenAI-compatible endpoint"，并给出 `client = OpenAI(base_url="https://openai.blueclaw.network/v1", api_key="your-api-key")` 代码示例）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://openai.blueclaw.network/v1`
- **鉴权**：方式=HTTP Bearer / 环境变量=`BLUECLAW_API_KEY` / 是否必需=是（Beta 期需 console 申请）
- **endpoint 公式**：`POST /v1/chat/completions`（OpenAI 兼容）
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions（"change base_url, keep the stack"）
- **响应结构要点**：标准 OpenAI Chat Completions 响应
- **流式**：SSE（官方明确 "Streaming in the standard SSE format"）
- **错误结构**：与 OpenAI 共享结构一致（OpenAI 兼容）
- **特有行为**：面向 agent 循环工作负载的开放模型推理；支持 tool calling；独立 GPU 网络；Beta 期免费（fair-use）

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官网明确 OpenAI 兼容 + base URL + Bearer 鉴权 + SSE 流式 + tool calling
- **可复用模型 ID 样例**：Qwen/Qwen3.6-35B-A3B-FP8、Qwen3.6-27B
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 仍处 Beta，免费 fair-use；正式计费/SLA 未定，稳定性与长期可用性待观察
- 模型数量少（inventory 仅 2），定位为低成本开放模型 agent 工作负载

#### 5. 优先级建议

- **优先级**：P2（后续）
- **理由**：证据强、薄封装，但处于 Beta、模型少、场景窄（agent 循环工作负载），待正式发布后再提升优先级

---

### chat_gpt_subscription_codex — ChatGPT Subscription (Codex)

- **canonical ID**：chat_gpt_subscription_codex
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：无公开 API 文档（OpenAI Learn 仅说明 Codex CLI 支持 "Sign in with ChatGPT for subscription access"，未公开 chatgpt.com 后端 API 契约）
- **核验来源**：仅官方说明 + 社区/issue（无官方 API 契约文档）
- **证据强度**：弱（无官方 API 契约文档；chatgpt.com 后端为非公开接口，鉴权走 ChatGPT 账号 OAuth 而非 API Key）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://chatgpt.com`（消费端站点，**非公开 API 端点**）
- **鉴权**：方式=ChatGPT 账号 OAuth 登录（非 Bearer API Key）/ 环境变量=无 / 是否必需=是（账号登录）
- **endpoint 公式**：未知（chatgpt.com 后端为内部/未公开接口）
- **协议类型**：未知（非公开 API；后端据称类 Responses API，但无官方契约）
- **请求结构要点**：未知
- **响应结构要点**：未知
- **流式**：未知
- **错误结构**：未知
- **特有行为**：Codex CLI 的 "Sign in with ChatGPT" 模式，用 ChatGPT 订阅额度而非 API 按量计费；自动化/程序化调用存在 ToS 与封号风险

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：待定（非公开 API，无官方契约可依）
- **依据**：无官方 API 文档确认请求/响应契约，鉴权为账号 OAuth 而非标准 API Key，无法归入任一路径
- **可复用模型 ID 样例**：无
- **是否需扩展共享层**：未知

#### 4. 风险与限制

- 无公开 API 契约，依赖未公开的 chatgpt.com 后端，逆向实现脆弱且违反 OpenAI ToS 风险高
- 鉴权为账号 OAuth，非标准 API Key，难以纳入常规 provider 适配
- 程序化/自动化调用明确存在封号风险

#### 5. 优先级建议

- **优先级**：搁置
- **理由**：无官方 API 契约（证据弱）、鉴权为账号 OAuth（非 API Key）、ToS/封号风险高；不应作为正式 provider 实现

---

### claudinio — Claudinio

- **canonical ID**：claudinio
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://claudin.io/docs/api-reference/（官方 API 参考）
- **核验来源**：官方 API 文档（含完整端点/请求/响应/错误说明）
- **证据强度**：强（官方文档明确 "Claudin.io is an OpenAI-compatible API"，列出全部端点、鉴权、流式、错误结构）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.claudin.io`（OpenAI 风格路由位于 `/v1`，故 OpenAI 客户端 base URL 为 `https://api.claudin.io/v1`）
- **鉴权**：方式=HTTP Bearer（`Authorization: Bearer YOUR_API_KEY`）或 `x-api-key` 头 / 环境变量=`CLAUDINIO_API_KEY` / 是否必需=是
- **endpoint 公式**：`POST /v1/chat/completions`（主）；另提供 `/v1/completions`、`/v1/messages`（Anthropic 格式）、`/v1/responses`（Codex）、`/v1/embeddings`、`GET /v1/models`
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions（messages/temperature/top_p/max_tokens/stream/stop/tools/tool_choice/response_format 等）
- **响应结构要点**：标准 OpenAI Chat Completions 响应
- **流式**：SSE（官方明确 OpenAI 流式格式 `data: {...}` + `data: [DONE]`）
- **错误结构**：与 OpenAI 共享结构一致（官方明确 `{"error":{"message","type","code"}}`；402=无订阅、429=预算封顶含 `Retry-After`）
- **特有行为**：订阅制（flat-rate，按小时防失控额度）；`max_tokens` 低于 4000 自动抬升到 4000；多模态输入由代理转写为文本；错误信息脱敏不泄露上游 provider

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方文档明确 OpenAI 兼容 + base URL + Bearer 鉴权 + 完整端点/流式/错误说明
- **可复用模型 ID 样例**：claudinio（主模型，256K 上下文）；inventory 另列 claudius
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 订阅制 + 按小时防失控额度，超限返回 429（非标准额度模型，但对客户端表现为 OpenAI 429）
- `max_tokens` 自动抬升至 4000 的行为需注意（可能影响短输出预期）
- 错误脱敏隐藏上游 provider，排障需依赖官方 dashboard

#### 5. 优先级建议

- **优先级**：P1（近期）
- **理由**：证据强、薄封装、文档完善；为订阅制 OpenAI 兼容代理，接入成本低
