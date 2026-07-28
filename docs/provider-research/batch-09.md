# 第 9 批调研记录（14 个 provider）

调研日期：2026-07-28。依据 RFC-0006 §2.1/§2.2，以官方 API 文档/SDK 为首要证据，inventory 元数据仅作线索。本批涉及多个"多协议网关"与若干"已实现/同源入口"，已在风险与限制中标注。

---

### model_oracle_ai — Model Oracle AI

- **canonical ID**：model_oracle_ai
- **aliases**：[]
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://modeloracle.com/setup
- **核验来源**：官方 API 文档（setup 页含 curl 示例与 OpenCode/Cursor 接入配置）
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://api.modeloracle.com/api/v1
- **鉴权**：方式=Bearer token（`Authorization: Bearer $MODEL_ORACLE_API_KEY`）/ 环境变量=MODEL_ORACLE_API_KEY / 是否必需=是
- **endpoint 公式**：`{base_url}/chat/completions`（POST）
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions 体（model、messages、可选 reasoning_effort）；OpenCode 配置使用 `@ai-sdk/openai-compatible` 包，Cursor 通过 OpenAI-compatible base URL override 接入。
- **响应结构要点**：OpenAI Chat Completions 响应；推理模型返回 reasoning 字段。
- **流式**：SSE（OpenAI 兼容 stream 协议；setup 页未单独展开，遵循 OpenAI 规范）
- **错误结构**：与 OpenAI 共享结构一致（未单独文档化，按 OpenAI 兼容推断）
- **特有行为**：顶层 `reasoning_effort` 字段（OpenAI o-series 已有字段，值集 none/low/medium/high/xhigh/max；Kimi K3 仅 low/high/max）；逻辑模型名（如 gpt-5.6-sol、claude-fable-5）由网关映射到上游 provider 模型并做 fallback；另提供 Anthropic-compatible Messages 端点供 Claude Code 使用（独立协议，非本次 chat 兼容路径）。

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方 setup 页 curl 与 SDK 配置确认鉴权/URL/请求均为标准 OpenAI Chat Completions；`@ai-sdk/openai-compatible` 直接可用；reasoning_effort 为 OpenAI 已知字段。
- **可复用模型 ID 样例**：auto、gpt-5.6-sol、claude-fable-5、kimi-k3、deepseek-v4-pro
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 逻辑模型名大小写敏感；不得传入上游 provider 命名空间的模型 ID。
- base URL 为 `/api/v1`（非 `/v1`），endpoint 拼接需注意。
- 另有 Anthropic Messages 端点（独立协议），如需 Claude 原生协议需单独适配。

#### 5. 优先级建议

- **优先级**：P1
- **理由**：官方文档强证据、协议干净、OpenAI 兼容薄封装即可，成本低价值明确。

---

### moka_ai — MokaAI

- **canonical ID**：moka_ai
- **aliases**：[]
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：无（inventory 未提供）
- **核验来源**：仅第三方搜索（未发现 LLM API 官方文档）
- **证据强度**：无
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：inventory 标 https://api.moka.ai（未证实为 LLM 端点）
- **鉴权**：未知
- **endpoint 公式**：未知
- **协议类型**：未知
- **请求结构要点**：未知
- **响应结构要点**：未知
- **流式**：未知
- **错误结构**：未知
- **特有行为**：搜索 "MokaAI moka.ai API documentation LLM" 仅命中 moka.ai / mokahr.com 的人力资源（HR/招聘）平台 API（REST + HTTP basic auth），未发现任何 LLM chat completions 端点。

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：待定
- **依据**：无法确认 moka.ai 提供 LLM chat API；现有证据指向其为 HR 平台，疑似 inventory 误录。
- **可复用模型 ID 样例**：无
- **是否需扩展共享层**：否

#### 4. 风险与限制

- provider 可能并非 LLM 厂商，base_url 与能力疑似误录。

#### 5. 优先级建议

- **优先级**：搁置
- **理由**：未找到任何 LLM API 官方文档或协议证据。

---

### moonshotai_cn — Moonshot AI (China)

- **canonical ID**：moonshotai_cn
- **aliases**：[]
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://platform.moonshot.cn/docs/api/chat（OpenAI 兼容 chat 文档）；`/anthropic` 端点由多来源佐证
- **核验来源**：官方 API 文档（OpenAI 兼容 chat）+ 多第三方一致（Anthropic 兼容端点）
- **证据强度**：中（OpenAI 兼容端点证据强；inventory 所列 `/anthropic` 端点为多第三方佐证，未在所给官方文档直接展开）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：inventory 标 https://api.moonshot.cn/anthropic/v1（Anthropic 兼容端点）；官方 chat 文档端点为 https://api.moonshot.cn/v1
- **鉴权**：方式=Bearer token / 环境变量=MOONSHOT_API_KEY / 是否必需=是
- **endpoint 公式**：OpenAI 兼容 `{base}/v1/chat/completions`；Anthropic 兼容 `{base}/anthropic/v1/messages`
- **协议类型**：双协议——OpenAI 兼容（`/v1`）与 Anthropic 兼容（`/anthropic/v1`）。本条目 base_url 指向 Anthropic 兼容端点。
- **请求结构要点**：OpenAI 兼容为标准 Chat Completions 体（model、messages、tools、partial 等）；Anthropic 兼容为 Anthropic Messages 体（供 Claude Code 接入 Kimi）。
- **响应结构要点**：OpenAI 兼容返回 `chat.completion` 对象（choices/usage/reasoning_content）；Anthropic 兼容返回 Anthropic Messages 结构。
- **流式**：SSE（两协议均支持）
- **错误结构**：OpenAI 兼容端点为 `{"error":{"message","type","code"}}`（与 OpenAI 一致）
- **特有行为**：Partial Mode（`partial` 字段）、`reasoning_content`；`/anthropic` 端点用于让 Claude Code 指向 Kimi（多来源确认存在，如 forum.moonshot.ai、OpenClaw、apidog）。

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装（OpenAI 兼容端点已由现有 moonshotai provider 覆盖；如需 `/anthropic` 变体则基于 Anthropic 共享层薄封装）
- **依据**：aimux 已有 `moonshotai.rs`（OpenAI 兼容，https://api.moonshot.cn/v1）与 Anthropic 共享层（`src/anthropic`）。`/anthropic` 端点为 Anthropic Messages 协议，可用 Anthropic 共享层薄封装。
- **可复用模型 ID 样例**：kimi-k2-thinking、kimi-k2-turbo-preview、kimi-k3
- **是否需扩展共享层**：否（Anthropic 共享层已存在）

#### 4. 风险与限制

- 本条目 base_url 与官方 chat 文档端点不一致（`/anthropic/v1` vs `/v1`），需明确目标协议。
- OpenAI 兼容入口已实现，`/anthropic` 变体为面向 Claude Code 的窄场景，增量价值有限。

#### 5. 优先级建议

- **优先级**：P2
- **理由**：OpenAI 兼容已覆盖；`/anthropic` 变体证据中等且场景窄。

---

### neon — Neon

- **canonical ID**：neon
- **aliases**：[]
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.5
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://neon.com/docs/ai-gateway/overview、/chat-completions、/authentication
- **核验来源**：官方 API 文档
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://<branch-host>/v1`（branch 维度动态主机；env `NEON_AI_GATEWAY_BASE_URL` 为裸主机，需自行拼接 `/v1`）
- **鉴权**：方式=Bearer token（`nt_live_...`，scope `ai_gateway:invoke`）/ 环境变量=NEON_AI_GATEWAY_TOKEN（+NEON_AI_GATEWAY_BASE_URL）/ 是否必需=是
- **endpoint 公式**：`{NEON_AI_GATEWAY_BASE_URL}/v1/chat/completions`（POST）；另 `/openai/v1`（Responses）、`/v1/gemini`（Gemini）；`GET /v1/models`
- **协议类型**：OpenAI 兼容（chat completions 为推荐统一端点，适配 catalog 全部模型）
- **请求结构要点**：标准 OpenAI Chat Completions 体（model、messages、max_tokens、stream 等），切换 model 字段即可换 provider。
- **响应结构要点**：OpenAI Chat Completions 响应；少数模型 `message.content` 返回为 content block 数组而非字符串。
- **流式**：SSE（`stream:true`，所有端点支持）
- **错误结构**：`{"error":{"message":"..."}}`（OpenAI 风格）；429 含 `Retry-After` 与 `X-Ratelimit-*` 头。
- **特有行为**：base URL 随 Neon branch 变化；credential 绑定 branch lineage；beta，仅 `aws-us-east-2`。

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方文档明确 "fully compatible with the OpenAI Chat Completions API"，OpenAI SDK 仅改 baseURL 即可。
- **可复用模型 ID 样例**：gpt-5-mini、gemini-3-flash、qwen3-next-80b-a3b-instruct
- **是否需扩展共享层**：否

#### 4. 风险与限制

- base URL 动态（branch-host），需从 env 读取而非固定常量。
- 少数模型 content 形状为数组，需兼容。
- beta 阶段，区域受限。

#### 5. 优先级建议

- **优先级**：P1
- **理由**：官方强证据、OpenAI 兼容统一端点，薄封装即可。

---

### netlify — Netlify

- **canonical ID**：netlify
- **aliases**：[]
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://docs.netlify.com/build/ai-gateway/overview
- **核验来源**：官方 API 文档
- **证据强度**：中（架构清晰，但无统一 OpenAI 端点；inventory 模型样例与官方模型表不一致）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：无统一端点；按 provider 分别注入 `OPENAI_BASE_URL`、`ANTHROPIC_BASE_URL`、`GOOGLE_GEMINI_BASE_URL`；另有 `NETLIFY_AI_GATEWAY_KEY` / `NETLIFY_AI_GATEWAY_BASE_URL`
- **鉴权**：方式=各 provider 原生鉴权（OpenAI/Anthropic/Gemini key 由网关注入）/ 环境变量=NETLIFY_AI_GATEWAY_KEY 等 / 是否必需=是
- **endpoint 公式**：OpenAI 模型走 `OPENAI_BASE_URL`（OpenAI Chat Completions）；Anthropic 模型走 `ANTHROPIC_BASE_URL/v1/messages`；Gemini 走 `GOOGLE_GEMINI_BASE_URL`
- **协议类型**：多协议原生代理（保留各 provider 原生协议，非统一 OpenAI 兼容网关）
- **请求结构要点**：各自遵循 OpenAI / Anthropic / Gemini 原生请求格式
- **响应结构要点**：各 provider 原生响应
- **流式**：各协议原生 SSE
- **错误结构**：各 provider 原生
- **特有行为**：网关按 token 计费转 Netlify credits；不存储 prompt/输出；inventory 模型样例带 `anthropic/` 前缀与官方模型表（`claude-fable-5` 等无前缀）不一致，疑为 mastra 误录。

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：原生（多协议代理；OpenAI 子集可薄封装，Anthropic 子集用 Anthropic 共享层）
- **依据**：无统一 OpenAI 端点，每个 provider 走原生协议；aimux 已有 OpenAI 与 Anthropic 共享层可分别薄封装，但单 provider 抽象不自然。
- **可复用模型 ID 样例**：claude-fable-5、gpt-4.1、gemini-2.5-pro（官方表，无 provider 前缀）
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 无统一端点，作为单一 provider 入口抽象 awkward。
- 模型样例前缀与官方不符，需以官方模型表为准。
- 仅 Credit-based 计划可用。

#### 5. 优先级建议

- **优先级**：P2
- **理由**：多协议代理、无统一端点，单 provider 价值有限；OpenAI/Anthropic 已有共享层可分别接入。

---

### neuralwatt — Neuralwatt

- **canonical ID**：neuralwatt
- **aliases**：[]
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://portal.neuralwatt.com/docs（quickstart 自述 OpenAI-compatible）
- **核验来源**：官方 API 文档
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://api.neuralwatt.com/v1
- **鉴权**：方式=Bearer token（`sk-xxxxx`）/ 环境变量=NEURALWATT_API_KEY / 是否必需=是
- **endpoint 公式**：`{base}/chat/completions`（POST）；`GET /models`；另有 usage/quota
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions 体（官方用 openai Python SDK + base_url 接入）
- **响应结构要点**：OpenAI Chat Completions 响应
- **流式**：SSE（guides/streaming）
- **错误结构**：与 OpenAI 共享结构一致（guides/error-handling）
- **特有行为**：主打节能推理；含 energy/usage 统计、flex tier、preview models。

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方 quickstart 明示 OpenAI-compatible，提供 openai SDK 与 cURL 示例。
- **可复用模型 ID 样例**：meta-llama/Llama-3.3-70B-Instruct、glm-5-fast、Qwen/Qwen3.5-397B-A17B-FP8
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 模型 ID 多带 provider 前缀（`meta-llama/`、`Qwen/`），需原样透传。

#### 5. 优先级建议

- **优先级**：P1
- **理由**：官方强证据、OpenAI 兼容薄封装即可。

---

### new_api — New API

- **canonical ID**：new_api
- **aliases**：[]
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：无固定 hosted 文档（inventory 未提供）；项目仓库 https://github.com/QuantumNous/new-api ，官网 https://www.newapi.ai/en
- **核验来源**：官方项目 README（开源网关软件）
- **证据强度**：中（项目文档确认 OpenAI 兼容，但为自托管软件，无固定 hosted base URL）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：无固定值（自托管，每个部署各自 URL，常见 `http://<host>/v1`）
- **鉴权**：方式=Bearer token（部署自定）/ 环境变量=用户自定 / 是否必需=是
- **endpoint 公式**：`{base}/v1/chat/completions`（OpenAI 兼容）；同时支持 Claude-compatible、Gemini-compatible
- **协议类型**：OpenAI 兼容（另提供 Claude/Gemini 兼容格式）
- **请求结构要点**：标准 OpenAI Chat Completions 体（One API 衍生，向后兼容）
- **响应结构要点**：OpenAI 兼容响应
- **流式**：SSE
- **错误结构**：与 OpenAI 共享结构一致（One API 兼容）
- **特有行为**：自托管 LLM 网关与 AI 资产管理系统；官方声明不直接售卖 API 访问。

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装（但自托管，无固定 base URL）
- **依据**：项目自述 "unified OpenAI-compatible API"；属 One API 衍生。
- **可复用模型 ID 样例**：无（随部署而异）
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 无固定 base URL/环境变量，无法作为开箱即用的 hosted provider 配置。
- 用户应通过通用 OpenAI 兼容入口（自定义 base URL）接入。

#### 5. 优先级建议

- **优先级**：搁置
- **理由**：自托管软件无固定 hosted 端点，单列 provider 增量价值低；建议走通用 OpenAI 兼容入口。

---

### nova — Nova

- **canonical ID**：nova
- **aliases**：[]
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：inventory 标 https://nova.amazon.com/dev/documentation（抓取为空，疑似不存在）；真实官方文档 https://docs.aws.amazon.com/nova/latest/userguide/getting-started-api.html
- **核验来源**：官方 AWS 文档（Amazon Nova = AWS Bedrock）
- **证据强度**：无（对 inventory 所列 `api.nova.amazon.com/v1` + Bearer 端点无法证实）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：inventory 标 https://api.nova.amazon.com/v1（未证实存在）；真实 Amazon Nova 经 AWS Bedrock runtime（如 `bedrock-runtime.<region>.amazonaws.com`）访问
- **鉴权**：真实路径为 AWS sigv4（IAM access key / 临时凭证），非 Bearer NOVA_API_KEY
- **endpoint 公式**：真实为 Bedrock `/model/{modelId}/invoke`（或 `invoke-with-response-stream`）；inventory 所列 `/v1/chat/completions` 未证实
- **协议类型**：原生（AWS Bedrock，sigv4；非 OpenAI 兼容）
- **请求结构要点**：Bedrock 原生请求体（按模型族而异）
- **响应结构要点**：Bedrock 原生响应
- **流式**：Bedrock `invoke-with-response-stream`（EventStream）
- **错误结构**：AWS 错误（sigv4，XML/JSON）
- **特有行为**：真实 Amazon Nova 2（nova-2-lite-v1 等）经 Bedrock；aimux `bedrock` provider 已支持 Amazon Nova（`bedrock/mod.rs`、`bedrock/image.rs` `amazon.nova-canvas-v1:0`、`bedrock/embedding.rs` nova embed）。

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：搁置（真实 Nova 已由 bedrock provider 覆盖）
- **依据**：inventory 的 `api.nova.amazon.com/v1` + Bearer 配置与官方 AWS 文档矛盾且无法证实；真实访问走 Bedrock sigv4，aimux 已有 bedrock provider 支持 Nova。
- **可复用模型 ID 样例**：真实为 `amazon.nova-2-lite-v1:0` 等（Bedrock ID）
- **是否需扩展共享层**：否

#### 4. 风险与限制

- inventory 元数据疑似臆造（独立 nova API + Bearer）。
- 模型样例 `nova-2-lite-v1` 为 OpenRouter 风格 ID，非 Bedrock 原生 ID。

#### 5. 优先级建议

- **优先级**：搁置
- **理由**：所列端点无法证实且与官方矛盾；真实 Amazon Nova 已通过 bedrock provider 可达。

---

### ofox — OfoxAI

- **canonical ID**：ofox
- **aliases**：[]
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://ofox.ai/docs（+ /docs/api、/docs/develop）
- **核验来源**：官方 API 文档
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://api.ofox.ai/v1
- **鉴权**：方式=Bearer token（`sk-of-...`）/ 环境变量=OFOX_API_KEY / 是否必需=是
- **endpoint 公式**：OpenAI 兼容 `{base}/chat/completions`；另 Anthropic Native、Gemini Native 端点
- **协议类型**：多协议网关（OpenAI Compatible / Anthropic Native / Gemini Native 三协议）
- **请求结构要点**：OpenAI 兼容端点为标准 Chat Completions 体（curl 示例 `model: openai/gpt-5.4-mini`）
- **响应结构要点**：OpenAI Chat Completions 响应
- **流式**：SSE（OpenAI 兼容）
- **错误结构**：与 OpenAI 共享结构一致（OpenAI 协议路径）
- **特有行为**："Three Protocols, One Gateway"；模型 ID 带 provider 前缀（`openai/`、`anthropic/`、`bailian/`、`deepseek/`）；`anthropic/*` 模型在 OpenAI `/chat/completions` 端点的路由行为文档未明确（可能需走 Anthropic Native 端点）。

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装（OpenAI 兼容端点）
- **依据**：官方文档确认 OpenAI 兼容 `/v1/chat/completions` + Bearer + 标准 OpenAI 体。
- **可复用模型 ID 样例**：openai/gpt-5.4-mini、deepseek/deepseek-v4-pro、glm-5.2
- **是否需扩展共享层**：否

#### 4. 风险与限制

- `anthropic/*` 模型经 OpenAI `/chat/completions` 是否可路由未确认；如不可则需 Anthropic Native 端点（aimux Anthropic 共享层）。
- 模型 ID 带 provider 前缀需原样透传。

#### 5. 优先级建议

- **优先级**：P1
- **理由**：官方强证据、OpenAI 兼容薄封装即可；多协议扩展可后续。

---

### ollama_chat — Ollama Chat

- **canonical ID**：ollama_chat
- **aliases**：[]
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://docs.ollama.com/api/openai-compatibility
- **核验来源**：官方 API 文档
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：http://localhost:11434/v1（默认本地）
- **鉴权**：方式=无鉴权（api_key 必填但被忽略，占位 `ollama`）/ 环境变量=OLLAMA_BASE_URL（存 base URL 非 key）/ 是否必需=否
- **endpoint 公式**：`{base}/chat/completions`（POST）；另 `/v1/completions`、`/v1/responses`、`/v1/embeddings`、`/v1/models`
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions 体（messages、model、stream、tools、reasoning_effort 等）
- **响应结构要点**：OpenAI Chat Completions 响应
- **流式**：SSE（`stream:true`）
- **错误结构**：与 OpenAI 共享结构一致
- **特有行为**：本地推理，无鉴权；另有原生 `/api/chat`（NDJSON，非本路径）。

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方文档明示 OpenAI 兼容 `/v1/chat/completions`；aimux 已有 `ollama.rs` 实现该薄封装（base `http://127.0.0.1:11434/v1`，占位 key，env `OLLAMA_BASE_URL`）。
- **可复用模型 ID 样例**：gpt-oss:20b、qwen3:8b、llama3.2
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 本条目与已实现 `ollama` provider 同源（OpenAI 兼容 chat），为重复入口。

#### 5. 优先级建议

- **优先级**：搁置
- **理由**：aimux 已实现 `ollama.rs`（OpenAI 兼容薄封装），本条目为重复/同源入口。

---

### oobabooga — Oobabooga

- **canonical ID**：oobabooga
- **aliases**：[]
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://github.com/oobabooga/textgen/wiki/12‐‐OpenAI-API
- **核验来源**：官方项目 Wiki
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：http://127.0.0.1:5000/v1（默认，`--api` 启动；`--api-port` 改端口）
- **鉴权**：方式=可选 API key（`--api-key`），默认无鉴权 / 环境变量=自定（base URL）/ 是否必需=否
- **endpoint 公式**：`{base}/chat/completions`（POST）；另 `/v1/completions`、`/v1/images/generations`、`/v1/internal/*`
- **协议类型**：OpenAI 兼容（drop-in replacement，含 Chat/Completions/Messages）
- **请求结构要点**：标准 OpenAI Chat Completions 体（messages、temperature、top_p、top_k、stream、tools、vision content blocks）
- **响应结构要点**：OpenAI Chat Completions 响应（`finish_reason: tool_calls` 等）
- **流式**：SSE（`stream:true`）
- **错误结构**：与 OpenAI 共享结构一致
- **特有行为**：100% 本地离线；额外 `top_k`、`instruction_template`、`character`、`--api-key` 等本地参数。

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方 Wiki 明示 OpenAI/Anthropic-compatible drop-in；aimux 已有 `oobabooba.rs`（OpenAI 兼容薄封装，base `http://127.0.0.1:5000/v1`，env `OOBABOOBA_BASE_URL`）。
- **可复用模型 ID 样例**：随本地加载模型而异
- **是否需扩展共享层**：否

#### 4. 风险与限制

- canonical id 拼写不一致（inventory `oobabooga` vs 实现 `oobabooba`），需统一。
- 本地自托管，无固定远程端点。

#### 5. 优先级建议

- **优先级**：搁置
- **理由**：aimux 已实现 `oobabooba.rs`（OpenAI 兼容薄封装），仅需核对 canonical id 拼写。

---

### opencode — OpenCode Zen

- **canonical ID**：opencode
- **aliases**：[]
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://opencode.ai/docs/zen
- **核验来源**：官方 API 文档（端点表）
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://opencode.ai/zen/v1（官方端点表）；aimux 现有 `opencode_zen.rs` 用 `https://api.opencode.zen/v1`（与官方不符，需核对）
- **鉴权**：方式=Bearer token / 环境变量=OPENCODE_API_KEY（aimux 现用 `OPENCODE_ZEN_API_KEY`）/ 是否必需=是
- **endpoint 公式**：多端点——`/zen/v1/chat/completions`（OpenAI 兼容，`@ai-sdk/openai-compatible`）、`/zen/v1/responses`（OpenAI Responses，`@ai-sdk/openai`）、`/zen/v1/messages`（Anthropic，`@ai-sdk/anthropic`）、`/zen/v1/models/{id}`（Gemini，`@ai-sdk/google`）
- **协议类型**：多协议网关（OpenAI 兼容 / OpenAI Responses / Anthropic / Gemini）
- **请求结构要点**：`/chat/completions` 为标准 OpenAI Chat Completions 体；`/messages` 为 Anthropic Messages；`/responses` 为 OpenAI Responses。
- **响应结构要点**：各协议原生响应
- **流式**：各协议原生 SSE
- **错误结构**：各协议原生
- **特有行为**：模型按协议分组——GPT 系走 `/responses`，Claude/Qwen 走 `/messages`，Grok/DeepSeek/MiniMax/GLM/Kimi 等走 `/chat/completions`，Gemini 走 `/models/{id}`。

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装（OpenAI 兼容 `/chat/completions` 子集；aimux 已有 `opencode_zen.rs`）
- **依据**：官方端点表确认 `/zen/v1/chat/completions` 为 OpenAI 兼容；现有 `opencode_zen.rs` 即 OpenAI 兼容薄封装。
- **可复用模型 ID 样例**：grok-4.5、deepseek-v4-pro、glm-5.2、kimi-k3、minimax-m2.5（`/chat/completions` 路径）
- **是否需扩展共享层**：否（OpenAI/Responses/Anthropic 共享层均已存在）

#### 4. 风险与限制

- 现有实现 base URL（`api.opencode.zen/v1`）与官方（`opencode.ai/zen/v1`）不一致，需修正。
- env 变量名（`OPENCODE_ZEN_API_KEY` vs inventory `OPENCODE_API_KEY`）需统一。
- 现有仅覆盖 OpenAI 兼容 `/chat/completions`；Claude/GPT/Gemini 模型需对应协议端点。

#### 5. 优先级建议

- **优先级**：P2
- **理由**：已实现 OpenAI 兼容薄封装，仅需核对/修正 base URL 与 env；多协议扩展可后续。

---

### pa_lm — PaLM

- **canonical ID**：pa_lm
- **aliases**：[]
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：无（inventory 未提供）；Google Generative Language API（PaLM）历史文档
- **核验来源**：官方 Google 文档（已声明弃用）+ 多来源一致
- **证据强度**：强（协议历史清晰，但 API 已弃用/下线）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://generativelanguage.googleapis.com/v1beta2（历史 PaLM 端点）
- **鉴权**：方式=API key（`x-goog-api-key` 或 `?key=`）/ 环境变量=GOOGLE_API_KEY / 是否必需=是
- **endpoint 公式**：`/models/{model}:{generateText|generateMessage|chatMessages}`（历史）
- **协议类型**：原生（Google Generative Language，非 OpenAI 兼容）
- **请求结构要点**：Google 原生（prompt/instances、candidate 等）
- **响应结构要点**：Google 原生（candidates、safetyMetadata）
- **流式**：支持（streamGenerateContent 风格）
- **错误结构**：Google 错误（code/message/status）
- **特有行为**：PaLM API 已弃用并被 Gemini 取代，2024 年 8 月前后下线；官方建议迁移至 Gemini API。

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：原生（但已弃用）
- **依据**：Google 原生协议，与 OpenAI 不兼容；aimux 应通过 Gemini provider 覆盖同厂能力。
- **可复用模型 ID 样例**：text-bison、chat-bison（均已下线）
- **是否需扩展共享层**：否

#### 4. 风险与限制

- API 已弃用/下线，无可用价值。
- 同厂能力由 Gemini 覆盖。

#### 5. 优先级建议

- **优先级**：搁置
- **理由**：PaLM API 已弃用下线，实现无价值；应使用 Gemini。

---

### perplexity_agent — Perplexity Agent

- **canonical ID**：perplexity_agent
- **aliases**：[]
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://docs.perplexity.ai/docs/agent-api/quickstart、/docs/agent-api/openai-compatibility、/docs/agent-api/models
- **核验来源**：官方 API 文档
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://api.perplexity.ai/v1
- **鉴权**：方式=Bearer token / 环境变量=PERPLEXITY_API_KEY / 是否必需=是
- **endpoint 公式**：规范端点 `POST /v1/agent`；OpenAI 兼容别名 `POST /v1/responses`（OpenAI SDK `client.responses.create()` 自动路由）；`GET /v1/models`（OpenAI 兼容，无需鉴权）
- **协议类型**：OpenAI Responses 兼容（非 Chat Completions）
- **请求结构要点**：OpenAI Responses 体（model、input、preset、max_output_tokens 等）；模型 ID 带 provider 前缀（`openai/gpt-5.6-sol`、`anthropic/claude-opus-5` 等）；`anthropic/*` 模型必须传 `max_output_tokens`。
- **响应结构要点**：OpenAI Responses 响应（`object:"response"`、`output[]`、`usage.input_tokens/output_tokens`、含 `cost`）；含 Perplexity 特有 web 搜索结果（queries/results/citations）等附加字段。
- **流式**：SSE（Responses 流式）
- **错误结构**：OpenAI 风格；`anthropic/*` 缺 `max_output_tokens` 返回 400。
- **特有行为**：多 provider 统一入口（Perplexity/OpenAI/Anthropic/Google/xAI/Z.AI/Moonshot/NVIDIA），preset 预设，web 搜索 grounding。

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装（基于 OpenAI Responses 共享层，aimux 已有 `src/openai/responses`、`src/open_responses`）；若 `anthropic/*` 必填 `max_output_tokens` 或需消费 web 搜索字段，则需共享层扩展
- **依据**：官方文档明示 "fully compatible with OpenAI's Responses API interface"，OpenAI SDK 仅改 baseURL；aimux 已具备 Responses 共享层。
- **可复用模型 ID 样例**：openai/gpt-5.6-sol、anthropic/claude-opus-5、google/gemini-3.6-flash、perplexity/sonar
- **是否需扩展共享层**：否（Responses 共享层已存在）；如需消费 web 搜索/引用字段则可能需扩展

#### 4. 风险与限制

- 与现有 `perplexity.rs`（Sonar Chat Completions）不同：Agent API 走 Responses 协议，需作为独立 provider 或扩展。
- `anthropic/*` 模型 `max_output_tokens` 必填需在共享层/映射处理。
- 响应含 Perplexity 附加字段（web 搜索结果），OpenAI Responses 共享层应能忽略附加字段，但需回归测试。

#### 5. 优先级建议

- **优先级**：P1
- **理由**：官方强证据、Responses 共享层已有、多 provider 统一入口价值高。
