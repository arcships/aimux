# 第 7 批调研记录（14 个 provider）

本批 14 个 provider 按 canonical id 字母序排列。证据来源以各 provider 官方 API 文档为主，官方文档为 SPA 未能加载内容时辅以成熟第三方实现（Haystack / litellm / Mastra 等）并标注证据强度。inventory 元数据（tier/protocol/openai_compatible）仅作线索，不作裁决依据。

---

### inferx — InferX

- **canonical ID**：inferx
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://inferx.net/docs/quickstart 、 https://inferx.net/docs/api-reference 、 https://inferx.net/
- **核验来源**：官方 API 文档（官网示例 + Quickstart）
- **证据强度**：强（官方文档直接给出 OpenAI SDK 调用示例与 base_url）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://model.inferx.net/v1`（官方 Quickstart 与官网示例一致；inventory 记录的 `https://model.inferx.net/endpoints/v1` 与官方不符，疑为 Console 路径，**以官方文档为准**）
- **鉴权**：方式=Bearer API Key / 环境变量=`INFERX_API_KEY`（官方示例使用 `os.environ["INFERX_API_KEY"]`，与 inventory 一致）/ 是否必需=是
- **endpoint 公式**：`POST {base_url}/chat/completions`（OpenAI Chat Completions）
- **协议类型**：OpenAI 兼容（官方明确 "OpenAI-compatible APIs"，使用标准 openai SDK `client.chat.completions.create`）
- **请求结构要点**：标准 OpenAI Chat Completions 请求体（model / messages / stream 等）
- **响应结构要点**：标准 OpenAI 响应；流式为 SSE，`choices[].delta.content`
- **流式**：SSE（`stream=True`，`chunk.choices[0].delta.content`）
- **错误结构**：未知（未在 Quickstart 中给出，按 OpenAI 兼容推断但不臆造）
- **特有行为**：serverless 推理，亚秒冷启动；模型 ID 形如 `Qwen/Qwen3.6-35B-A3B`；Console 中每个 endpoint 有独立 API Base URL/Model/API Key，需从 Client Setup 复制

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方文档明确 OpenAI 兼容，请求/响应/流式均由 OpenAI Chat Completions 表达，仅需 base_url + Bearer key
- **可复用模型 ID 样例**：`Qwen/Qwen3.6-35B-A3B`、`qwen/qwen3.6-27b-fp8`、`google/gemma-4-31b-it-fp8`
- **是否需扩展共享层**：否

#### 4. 风险与限制

- inventory base_url（`/endpoints/v1`）与官方（`/v1`）不一致，接入时须以官方为准并实测
- endpoint 级别的 API Key/URL 可能随 Console 部署变化

#### 5. 优先级建议

- **优先级**：P1
- **理由**：官方 OpenAI 兼容明确，薄封装成本低；serverless 开放模型推理有实际价值

---

### io_net — IO.NET

- **canonical ID**：io_net
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://io.net/docs/guides/intelligence/io-intelligence-apis
- **核验来源**：官方 API 文档
- **证据强度**：强（官方文档明确声明 "we fully support the API contract presented by OpenAI, making it fully OpenAI API compatible"，并给出 cURL 示例）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.intelligence.io.solutions/api/v1`
- **鉴权**：方式=Bearer API Key / 环境变量=`IOINTELLIGENCE_API_KEY`（官方 cURL 使用 `Authorization: Bearer $IOINTELLIGENCE_API_KEY`）/ 是否必需=是
- **endpoint 公式**：`POST {base_url}/chat/completions`
- **协议类型**：OpenAI 兼容（官方明示）
- **请求结构要点**：标准 OpenAI Chat Completions（model / messages / temperature 等）
- **响应结构要点**：标准 OpenAI 响应
- **流式**：未知（官方示例未展示 stream，但既全兼容 OpenAI 契约，预期支持 SSE；未直接确认）
- **错误结构**：未知（未在文档中给出，不臆造）
- **特有行为**：部署于 io.net 硬件的开源模型市场 + AI Agents；API Key 有权限（All/Read/Write）与有效期（30/60/90/180 天）属性；按模型有不同的免费日额度

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方明确全兼容 OpenAI API 契约，base_url + Bearer key 即可
- **可复用模型 ID 样例**：`meta-llama/Llama-3.3-70B-Instruct`、`Qwen/Qwen3-235B-A22B-Thinking-2507`、`deepseek-ai/DeepSeek-R1-0528`
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 流式协议官方未直接展示，接入时需实测 `stream:true` 的 SSE 行为
- API Key 有效期机制可能影响长期可用性

#### 5. 优先级建议

- **优先级**：P1
- **理由**：官方明示 OpenAI 全兼容，薄封装成本低，模型丰富

---

### jiekou — Jiekou.AI

- **canonical ID**：jiekou
- **aliases**：接口AI
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://docs.jiekou.ai/docs/support/quickstart 、 https://docs.jiekou.ai/docs/model/llm
- **核验来源**：官方 API 文档
- **证据强度**：强（官方文档明确 "提供了与 OpenAI API 标准兼容的 API 服务"，给出 ChatCompletion / Completion 的 Python 与 cURL 示例）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.highwayapi.ai/openai`（官方文档示例统一使用该地址；**inventory 记录的 `https://api.jiekou.ai/openai` 与官方文档不一致，需进一步核实**——以官方文档为准）
- **鉴权**：方式=Bearer API Key（`Authorization: Bearer {API Key}`）/ 环境变量=未在文档明确命名（inventory 记 `JIEKOU_API_KEY`）/ 是否必需=是
- **endpoint 公式**：`POST {base_url}/v1/chat/completions`、`POST {base_url}/v1/completions`
- **协议类型**：OpenAI 兼容（官方明示与 OpenAI API 标准兼容）
- **请求结构要点**：标准 OpenAI ChatCompletions / Completions 请求体；支持 model/messages/stream/max_tokens/temperature/top_p/top_k/presence_penalty/frequency_penalty/repetition_penalty/stop 等
- **响应结构要点**：标准 OpenAI 响应
- **流式**：SSE（`stream=true` 流式输出，`chunk.choices[0].delta.content`）
- **错误结构**：未知（文档未给出错误结构）
- **特有行为**：聚合网关，153+ 模型（含 claude / ernie / deepseek / stheno 等）；提供 MCP 服务与 Agent Skills

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方明示 OpenAI 标准兼容，ChatCompletion + Completion 均由 OpenAI 共享层表达
- **可复用模型 ID 样例**：`deepseek/deepseek-r1`、`baidu/ernie-4.5-300b-a47b-paddle`、`Sao10K/L3-8B-Stheno-v3.2`
- **是否需扩展共享层**：否

#### 4. 风险与限制

- base_url 存在 `api.highwayapi.ai/openai`（官方文档）与 `api.jiekou.ai/openai`（inventory）两份记录，须实测确认哪一个为现行可用地址
- 文档未明确环境变量名

#### 5. 优先级建议

- **优先级**：P1
- **理由**：官方 OpenAI 兼容明确、模型多；但 base_url 歧义须先核实

---

### jimeng — Jimeng

- **canonical ID**：jimeng
- **aliases**：即梦
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：image / video 生成（**inventory 标注的 "chat" 能力有误**）

#### 1. 官方协议证据

- **文档 URL**：https://www.volcengine.com/docs/85621/1756900 （即梦文生图3.1）、https://www.volcengine.com/docs/85621/1785204 （即梦视频3.0 图生视频）
- **核验来源**：官方 API 文档（火山引擎文档中心，页面为 SPA，正文未能通过 WebFetch 加载；以下事实由官方文档标题/摘要 + 多来源一致确认）
- **证据强度**：中（官方文档存在但正文未加载，依赖官方文档索引 + 第三方 MCP/Skill 实现一致描述）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://visual.volcengineapi.com`（火山引擎视觉智能服务）
- **鉴权**：方式=火山引擎 IAM V4 签名（AccessKey / SecretKey 签名，**非 Bearer**）/ 环境变量=inventory 无记录（火山引擎通用为 `VOLC_ACCESSKEY` / `VOLC_SECRETKEY`，未在官方文档直接确认）/ 是否必需=是
- **endpoint 公式**：未知（官方文档为 SPA 未加载正文；火山引擎视觉服务通常为 Action + Version 查询参数风格，但**不臆造具体 path**）
- **协议类型**：原生（火山引擎视觉服务专用协议）+ 专用模态（图像/视频生成）
- **请求结构要点**：未知（正文未加载）；属火山引擎 CV 服务请求结构，与 OpenAI Chat 无关
- **响应结构要点**：未知（异步任务提交 + 结果查询模式，第三方描述一致，但具体字段未确认）
- **流式**：未知
- **错误结构**：未知
- **特有行为**：文生图 / 图生图 / 文生视频 / 图生视频；与即梦产品同源；异步任务模式

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：模态专用（图像/视频生成）+ 原生协议
- **依据**：非 chat 能力，火山引擎 V4 签名 + Action 风格原生协议，与 OpenAI 契约无任何结构相似
- **可复用模型 ID 样例**：无（按能力/版本调用，非 model ID 驱动）
- **是否需扩展共享层**：否（不走 OpenAI 共享层）

#### 4. 风险与限制

- 官方文档为 SPA，正文未能加载，请求/响应契约未直接确认——**协议细节均为未知**
- inventory 能力标注 "chat" 有误，实际为图像/视频生成
- 鉴权为火山引擎签名，非 Bearer，实现复杂度高

#### 5. 优先级建议

- **优先级**：搁置（就 chat 适配而言不在范围；若 aimux 支持图像/视频模态则 P2）
- **理由**：非 chat 能力 + 原生签名协议 + 官方文档正文未确认，证据不足以排期实现

---

### jina — Jina

- **canonical ID**：jina
- **aliases**：Jina AI
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：embeddings / rerank（**inventory 标注的 "chat" 能力有误**）

#### 1. 官方协议证据

- **文档 URL**：https://jina.ai/embeddings/ 、 https://api.jina.ai/scalar
- **核验来源**：官方 API 文档（embeddings 页面给出 cURL 示例与端点）
- **证据强度**：强（官方文档直接给出 `/v1/embeddings` 端点与请求/响应示例）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.jina.ai`
- **鉴权**：方式=Bearer API Key（`Authorization: Bearer <key>`）/ 环境变量=inventory 无记录（通用为 `JINA_API_KEY`）/ 是否必需=是
- **endpoint 公式**：`POST /v1/embeddings`（OpenAI 兼容 embeddings）、`POST /v1/rerank`（Jina 原生 rerank）、`POST /v1/train`（classifier）
- **协议类型**：专用模态（embeddings + rerank），其中 embeddings 与 OpenAI Embeddings 兼容
- **请求结构要点**：embeddings 请求与 OpenAI `/v1/embeddings` 兼容（input / model 等），并扩展 `normalized`、`embedding_type`、`encoding_format`、`output_dtype` 等字段；rerank 为 Jina 原生结构
- **响应结构要点**：embeddings 返回标准 `data[].embedding`；rerank 返回 `results[]`（原生）
- **流式**：无（embeddings/rerank 非流式）
- **错误结构**：未知（未在文档中确认）
- **特有行为**：多模态 embeddings（文本/图像/音频/视频，v5-omni）；Matryoshka 维度；无 chat completions 端点

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：模态专用（embeddings 走 OpenAI Embeddings 兼容；rerank 走原生）
- **依据**：无 chat 能力；embeddings 与 OpenAI Embeddings 兼容可复用 embeddings 共享层，rerank 需原生
- **可复用模型 ID 样例**：`jina-embeddings-v5-text`、`jina-embeddings-v5-omni`、`jina-reranker-v3`
- **是否需扩展共享层**：否（embeddings）；是（若支持 rerank，需新增 rerank 原生实现）

#### 4. 风险与限制

- inventory 能力标注 "chat" 有误，实际为 embeddings/rerank
- rerank 为原生协议，无法走 OpenAI 共享层

#### 5. 优先级建议

- **优先级**：P2
- **理由**：embeddings 模态专用、证据强；但 aimux 以 chat 适配为主，且 rerank 需原生，排后续

---

### kenari — Kenari

- **canonical ID**：kenari
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://kenari.id/docs
- **核验来源**：官方 API 文档（含 OpenAPI / llms.txt）
- **证据强度**：强（官方文档明确 "Base URL https://kenari.id/v1 是 https://api.openai.com/v1 的直接替换"，并给出 Python/TS/cURL 示例）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://kenari.id/v1`
- **鉴权**：方式=Bearer API Key（key 以 `kn-` 开头，`Authorization: Bearer kn-...`）/ 环境变量=`KENARI_API_KEY`（inventory 一致）/ 是否必需=是
- **endpoint 公式**：`POST {base_url}/chat/completions`
- **协议类型**：OpenAI 兼容（官方明示为 `api.openai.com/v1` 的直接替换）
- **请求结构要点**：标准 OpenAI Chat Completions（model / messages 等）
- **响应结构要点**：标准 OpenAI 响应（`choices[0].message.content`）
- **流式**：SSE（OpenAI 兼容，预期支持；官方未单独展示流式但既为直接替换）
- **错误结构**：未知（文档未单独给出，按 OpenAI 兼容推断）
- **特有行为**：一个 key 适用于所有模型/provider；同时提供 OpenAI 与 Anthropic 两种 API；按 token 以印尼盾计费；提供 `/openapi.json`、`/llms.txt`、`/llms-full.txt`

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方明示为 OpenAI base_url 直接替换，请求/响应结构一致
- **可复用模型 ID 样例**：`gpt-4o-mini`（官方示例）、以及 inventory 的 `claude-opus-4-8`、`deepseek-v4-flash` 等
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 同时支持 Anthropic 协议，但 OpenAI 路径走薄封装即可
- 模型名含 `claude-*` / `deepseek-*` 等转发，须注意上游可用性

#### 5. 优先级建议

- **优先级**：P1
- **理由**：官方 OpenAI 兼容明确、有 OpenAPI/llms.txt，薄封装成本极低

---

### kimi — Kimi

- **canonical ID**：kimi
- **aliases**：Moonshot AI / Kimi 开放平台
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://platform.kimi.ai/docs/api/overview
- **核验来源**：官方 API 文档（Kimi 开放平台 / Moonshot）
- **证据强度**：强（官方文档明确 "Kimi Open Platform provides OpenAI-compatible HTTP APIs"，给出 base_url、鉴权、端点表）
- **核验日期**：2026-07-28

> 说明：inventory 该条目仅有 display_name=Kimi、source=rust_genai，无 base_url/env/docs。本调研据官方 Kimi 开放平台（platform.kimi.ai）确认其为 Moonshot 平台；该对应关系为推断，见风险。

#### 2. 协议事实

- **base URL**：`https://api.moonshot.ai/v1`（国际站 platform.kimi.ai；另有国内站 `https://api.moonshot.cn/v1`）
- **鉴权**：方式=Bearer API Key（`Authorization: Bearer $MOONSHOT_API_KEY`）/ 环境变量=`MOONSHOT_API_KEY`（inventory 无记录，以官方为准）/ 是否必需=是
- **endpoint 公式**：`POST /v1/chat/completions`、`GET /v1/models`、`POST /v1/tokenizers/estimate-token-count`、`GET /v1/users/me/balance`、`/v1/files` 系列
- **协议类型**：OpenAI 兼容（官方明示兼容 OpenAI Chat Completions 请求/响应格式）
- **请求结构要点**：标准 OpenAI Chat Completions；Kimi 特有扩展：`thinking` 参数需经 SDK `extra_body` 传入，`partial` 为 messages 中 assistant 消息的字段（非顶层参数）
- **响应结构要点**：标准 OpenAI 响应
- **流式**：SSE（OpenAI 兼容）
- **错误结构**：厂商专属要点：返回 JSON `error.type` 与 `error.message`；HTTP 400/401/429/500 等
- **特有行为**：`thinking` 推理参数、`partial` 模式、文件上传与 token 估算端点

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装（主体）+ 共享层扩展（若需支持 `thinking`）
- **依据**：主体兼容 OpenAI Chat Completions；`thinking` 经 `extra_body` 透传通常无需改共享层，但若一等公民支持推理开关可作共享层扩展
- **可复用模型 ID 样例**：moonshot kimi 系列模型（具体 ID 见官方模型列表）
- **是否需扩展共享层**：否（`thinking` 可透传 extra_body）；是（若要一等公民 reasoning_effort/thinking 抽象）

#### 4. 风险与限制

- inventory "kimi" 与 Moonshot 平台的对应关系为推断（rust_genai 源未直接确认）；若 inventory 实指其他 Kimi 产品则结论不成立
- 国际站 `.ai` 与国内站 `.cn` 两个域名并存，须按部署区域选择

#### 5. 优先级建议

- **优先级**：P1
- **理由**：官方 OpenAI 兼容明确、生态成熟；但 inventory 身份对应需复核

---

### kimi_for_coding — Kimi For Coding

- **canonical ID**：kimi_for_coding
- **aliases**：Kimi Code
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://www.kimi.com/code/docs/en/（Kimi Code Overview）
- **核验来源**：官方 API 文档
- **证据强度**：强（官方文档明确 "Kimi Code API is compatible with both OpenAI and Anthropic protocols"，给出 base_url 与端点示例表）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.kimi.com/coding/v1`（OpenAI 兼容）
- **鉴权**：方式=API Key（Bearer，OpenAI 兼容路径）/ 环境变量=`KIMI_API_KEY`（inventory 一致）/ 是否必需=是（在第三方工具中手动配置）
- **endpoint 公式**：OpenAI 兼容：`POST {base_url}/chat/completions`；Anthropic 兼容：`POST https://api.kimi.com/coding/v1/messages`
- **协议类型**：OpenAI 兼容（同时提供 Anthropic 兼容）
- **请求结构要点**：标准 OpenAI Chat Completions
- **响应结构要点**：标准 OpenAI 响应
- **流式**：SSE（OpenAI 兼容）
- **错误结构**：未知（文档未给出；社区反映第三方工具调用偶有 `error.type` 类错误，但不臆造）
- **特有行为**：会员订阅制（非按量付费），按 5 小时窗口限频（约 300–1200 请求、最高 30 并发）；两档速度（Standard / HighSpeed）；4 个 model ID：`k3`、`k3-256k`、`kimi-for-coding`、`kimi-for-coding-highspeed`；要求保持客户端 User-Agent 标识，篡改可能停用权益

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方明示 OpenAI 兼容，base_url + Bearer key + 标准 chat/completions
- **可复用模型 ID 样例**：`k3`、`k3-256k`、`kimi-for-coding`、`kimi-for-coding-highspeed`
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 会员订阅 + 限频，非按量，商业模型特殊
- 须保留客户端 User-Agent 标识，否则可能被停用
- 与 `kimi`（Moonshot 开放平台）是不同产品/不同 base_url，勿混淆

#### 5. 优先级建议

- **优先级**：P1
- **理由**：官方 OpenAI 兼容明确，编程场景需求高；薄封装即可

---

### kling — Kling

- **canonical ID**：kling
- **aliases**：Kling AI / 可灵
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：image / video 生成（**inventory 标注的 "chat" 能力有误**）

#### 1. 官方协议证据

- **文档 URL**：https://kling.ai/document-api/api/get-started/authentication 、https://kling.ai/document-api/guides/get-started/quick-start
- **核验来源**：官方 API 文档（页面为 SPA，正文未能通过 WebFetch 加载；以下事实由官方文档索引 + 第三方封装库/教程一致确认）
- **证据强度**：中（官方文档存在但正文未加载；JWT 鉴权 + 视频/图像生成由多个第三方实现一致描述）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.klingai.com`（inventory 一致）
- **鉴权**：方式=JWT（由 Access Key + Secret Key 签名生成 JWT，**非 Bearer API Key**）/ 环境变量=inventory 无记录（通用为 AK/SK 对）/ 是否必需=是
- **endpoint 公式**：未知（官方文档为 SPA 未加载正文；第三方描述为视频/图像生成端点，**不臆造具体 path**）
- **协议类型**：原生 + 专用模态（视频/图像生成）
- **请求结构要点**：未知（正文未加载）；属视频/图像生成任务结构，与 OpenAI Chat 无关
- **响应结构要点**：未知（异步任务模式，第三方描述一致，具体字段未确认）
- **流式**：未知
- **错误结构**：未知
- **特有行为**：文生视频 / 图生视频 / 文生图 / 图像扩展 / 数字人；异步任务提交 + 查询

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：模态专用（视频/图像生成）+ 原生协议
- **依据**：非 chat 能力；JWT 鉴权 + 异步任务原生协议，与 OpenAI 契约无结构相似
- **可复用模型 ID 样例**：无（按能力/版本调用）
- **是否需扩展共享层**：否（不走 OpenAI 共享层）

#### 4. 风险与限制

- 官方文档为 SPA，正文未加载，请求/响应契约未直接确认——**协议细节多为未知**
- inventory 能力标注 "chat" 有误，实际为视频/图像生成
- JWT 鉴权（AK/SK）实现复杂度高于 Bearer

#### 5. 优先级建议

- **优先级**：搁置（就 chat 适配而言不在范围；若 aimux 支持视频/图像模态则 P2）
- **理由**：非 chat 能力 + 原生 JWT 协议 + 官方文档正文未确认，证据不足以排期

---

### kuae_cloud_coding_plan — KUAE Cloud Coding Plan

- **canonical ID**：kuae_cloud_coding_plan
- **aliases**：KUAE Cloud
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://docs.mthreads.com/kuaecloud/kuaecloud-doc-online/coding_plan/ （摩尔线程官方文档，页面为 SPA，正文未能通过 WebFetch 加载）
- **核验来源**：官方文档（正文未加载）+ Mastra 第三方注册表（https://mastra.ai/models/providers/kuae-cloud-coding-plan）
- **证据强度**：中（官方文档 URL 存在但正文未加载；Mastra 第三方与 inventory 一致指向 OpenAI 兼容 `/chat/completions` 与同一 base_url/env，多来源一致但缺官方正文直接确认）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://coding-plan-endpoint.kuaecloud.net/v1`（Mastra 与 inventory 一致）
- **鉴权**：方式=Bearer API Key（Mastra 示例 `apiKey: process.env.KUAE_API_KEY`，OpenAI 兼容推断 Bearer）/ 环境变量=`KUAE_API_KEY`（inventory 与 Mastra 一致）/ 是否必需=是
- **endpoint 公式**：`POST {base_url}/chat/completions`（Mastra 明示使用 OpenAI 兼容 `/chat/completions`）
- **协议类型**：OpenAI 兼容（Mastra 第三方描述；**官方正文未直接确认**）
- **请求结构要点**：标准 OpenAI Chat Completions（推断，官方未加载）
- **响应结构要点**：标准 OpenAI 响应（推断）
- **流式**：未知（Mastra 示例展示 `agent.stream`，预期 SSE，但未由官方确认）
- **错误结构**：未知
- **特有行为**：摩尔线程（mthreads）KUAE Cloud 编程套餐；单模型 `GLM-4.7`；205K 上下文

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装（待官方文档确认）
- **依据**：Mastra + inventory 一致指向 OpenAI 兼容 chat/completions；但官方文档正文未加载，须实测确认请求/响应契约
- **可复用模型 ID 样例**：`GLM-4.7`（Mastra 标记为 `kuae-cloud-coding-plan/GLM-4.7`）
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 官方文档为 SPA 未加载正文，OpenAI 兼容性仅由第三方 + inventory 推断，**须实测**
- 仅单模型，且与 z.AI / 智谱 GLM Coding Plan 名称相近，易混淆（不同产品）

#### 5. 优先级建议

- **优先级**：P2
- **理由**：疑似 OpenAI 兼容薄封装，但官方协议未直接确认、仅单模型，排后续并先实测

---

### lemonade — Lemonade

- **canonical ID**：lemonade
- **aliases**：Lemonade Server
- **provider_kind**：model_vendor（本地推理服务器）
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.5
- **能力**（本次调研覆盖）：chat（亦提供 embeddings / audio / images / realtime）

#### 1. 官方协议证据

- **文档 URL**：https://lemonade-server.ai/docs/api/openai/
- **核验来源**：官方 API 文档（Lemonade Server 文档）
- **证据强度**：强（官方文档给出完整 OpenAI 兼容端点表与请求/响应示例）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`http://localhost:13305/v1`（本地服务器默认端口 13305；**官方文档示例使用 `/v1`，AMD Playbook 另记 `/api/v1`，以官方文档 `/v1` 为准**）
- **鉴权**：方式=无（本地服务器，默认不鉴权）/ 环境变量=无 / 是否必需=否
- **endpoint 公式**：`POST /v1/chat/completions`、`POST /v1/completions`、`POST /v1/embeddings`、`POST /v1/responses`、`POST /v1/audio/transcriptions`、`POST /v1/audio/speech`、`/realtime`（WS）、`/v1/images/*`、`GET /v1/models`
- **协议类型**：OpenAI 兼容（官方明示实现 OpenAI API）
- **请求结构要点**：标准 OpenAI Chat Completions（model / messages / stream / temperature / top_p / top_k / tools / max_tokens 等）；额外支持 `repeat_penalty`
- **响应结构要点**：标准 OpenAI 响应（`object: chat.completion`、`choices[].message`）；流式为 `chat.completion.chunk` + `choices[].delta`
- **流式**：SSE（`stream:true`，data-only server-sent events）
- **错误结构**：未知（文档未单独给出）
- **特有行为**：本地 AMD GPU/NPU 推理；按需加载模型；Omni collection 模型有 server-side tools 扩展（非 OpenAI 标准）；模型 ID 形如 `Qwen3-0.6B-GGUF`

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方明示 OpenAI 兼容，chat/completions 完全由 OpenAI 共享层表达；本地无鉴权
- **可复用模型 ID 样例**：`Qwen3-0.6B-GGUF`、`lemonade/Qwen3-Coder-30B-A3B-Instruct-GGUF`、`lemonade/gpt-oss-120b-mxfp-GGUF`
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 本地服务器，base_url 为 localhost，非云端 provider；须由用户本地启动
- base_url 路径 `/v1` vs `/api/v1` 文档间不一致，须以官方为准并实测
- Omni collection server-side tools 为非标准扩展，普通模型不受影响

#### 5. 优先级建议

- **优先级**：P2
- **理由**：OpenAI 兼容明确、薄封装可行；但属本地 AMD 硬件场景，受众较窄，排后续

---

### lilac — Lilac

- **canonical ID**：lilac
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat（含图像输入 / 推理 / 工具调用 / 结构化输出）

#### 1. 官方协议证据

- **文档 URL**：https://docs.getlilac.com/inference/models 、https://docs.getlilac.com/inference/chat-completions
- **核验来源**：官方 API 文档
- **证据强度**：强（官方文档给出 `https://api.getlilac.com/v1/chat/completions` 的 cURL/Python 示例与 OpenAI SDK 用法）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.getlilac.com/v1`
- **鉴权**：方式=Bearer API Key（`Authorization: Bearer your-lilac-api-key`）/ 环境变量=`LILAC_API_KEY`（inventory 一致）/ 是否必需=是
- **endpoint 公式**：`POST {base_url}/chat/completions`
- **协议类型**：OpenAI 兼容（使用 OpenAI SDK，标准 chat/completions 请求/响应）
- **请求结构要点**：标准 OpenAI Chat Completions；多模态图像经 `content` 数组 `image_url` 传入（base64 data URI 或 URL）；推理经 `reasoning` 字段 + `chat_template_kwargs` 控制；结构化输出经 `response_format`（json_object / json_schema）
- **响应结构要点**：标准 OpenAI 响应；思维链返回在 `reasoning` 字段
- **流式**：SSE（OpenAI 兼容，预期支持；官方示例未单独展示流式）
- **错误结构**：未知（文档未单独给出）
- **特有行为**：按 token 计费含 cache read 折扣；推理开关因模型而异（Kimi K2.6 用 `chat_template_kwargs.thinking`，GLM 5.2 用 `enable_thinking` / `reasoning_effort`）

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装（主体）+ 共享层扩展（若一等公民支持 `reasoning` 字段透传）
- **依据**：主体兼容 OpenAI Chat Completions；`reasoning` 字段为多厂商共有的推理输出，可考虑共享层抽象
- **可复用模型 ID 样例**：`moonshotai/kimi-k2.6`、`zai-org/glm-5.2`、`google/gemma-4-31b-it`、`minimaxai/minimax-m3`
- **是否需扩展共享层**：否（基础 chat）；是（若统一 `reasoning` 字段处理）

#### 4. 风险与限制

- 不同模型推理开关参数名不同（thinking / enable_thinking / reasoning_effort），需按模型适配
- 模型名带 `k2.6` / `5.2` / `4` 等版本号，须以官方模型表为准

#### 5. 优先级建议

- **优先级**：P1
- **理由**：官方 OpenAI 兼容明确、模型较新，薄封装成本低

---

### llama — Llama

- **canonical ID**：llama
- **aliases**：Meta Llama API
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://llama.developer.meta.com/docs/features/compatibility/ 、https://llama.developer.meta.com/docs/models
- **核验来源**：官方 API 文档（页面为 SPA 未直接加载）+ 成熟实现 Haystack（`MetaLlamaChatGenerator`，继承 `OpenAIChatGenerator`）+ litellm-rs
- **证据强度**：强（官方兼容性文档 URL + Haystack 成熟实现直接给出 base_url/env/参数，且与 inventory base_url 一致；多来源一致）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.llama.com/compat/v1/`（OpenAI 兼容端点；inventory 记 `https://api.llama.com/compat/v1` 一致）
- **鉴权**：方式=Bearer API Key（OpenAI 兼容，`Authorization: Bearer $LLAMA_API_KEY`）/ 环境变量=`LLAMA_API_KEY`（inventory 与 Haystack 一致）/ 是否必需=是
- **endpoint 公式**：`POST {base_url}/chat/completions`
- **协议类型**：OpenAI 兼容（官方兼容性端点；Haystack 实现直接继承 OpenAI 客户端）
- **请求结构要点**：标准 OpenAI Chat Completions；支持 max_tokens / temperature / top_p / stream / safe_prompt / random_seed / response_format(json_schema) / tools
- **响应结构要点**：标准 OpenAI 响应
- **流式**：SSE（`stream:true`，data-only server-sent events，以 `data: [DONE]` 结束）
- **错误结构**：未知（未单独确认；按 OpenAI 兼容推断）
- **特有行为**：`safe_prompt` 安全提示开关；Llama 4 / Llama 3.3 系列；inventory 的模型样例（`cerebras-llama-*` / `groq-llama-*`）并非 Meta Llama API 原生模型，原生模型为 `Llama-4-Maverick-17B-128E-Instruct-FP8`、`Llama-4-Scout-17B-16E-Instruct-FP8`、`Llama-3.3-70B-Instruct`、`Llama-3.3-8B-Instruct`

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方提供 OpenAI 兼容端点 `/compat/v1`，请求/响应/流式均由 OpenAI 共享层表达
- **可复用模型 ID 样例**：`Llama-4-Scout-17B-16E-Instruct-FP8`、`Llama-4-Maverick-17B-128E-Instruct-FP8`、`Llama-3.3-70B-Instruct`、`Llama-3.3-8B-Instruct`
- **是否需扩展共享层**：否（`safe_prompt` 可经 extra_body 透传）

#### 4. 风险与限制

- inventory 模型样例混入 cerebras/groq 前缀模型，非本 API 原生，接入时须用官方模型 ID
- 官方文档为 SPA 未直接加载正文，协议细节经 Haystack 成熟实现 + litellm-rs 交叉确认

#### 5. 优先级建议

- **优先级**：P1
- **理由**：官方 OpenAI 兼容端点明确、生态成熟，薄封装成本低

---

### llmgateway — LLM Gateway

- **canonical ID**：llmgateway
- **aliases**：LLM Gateway
- **provider_kind**：model_vendor（开源 API 网关，提供托管服务）
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://llmgateway.io/docs 、https://llmgateway.io/quick-start
- **核验来源**：官方 API 文档
- **证据强度**：强（官方 Quickstart 明确 "Point your HTTP requests to https://api.llmgateway.io/v1/…, supply your LLM_GATEWAY_API_KEY"，给出 cURL/多语言示例与 OpenAI SDK 用法）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.llmgateway.io/v1`
- **鉴权**：方式=Bearer API Key（`Authorization: Bearer $LLM_GATEWAY_API_KEY`）/ 环境变量=`LLM_GATEWAY_API_KEY`（官方文档；**inventory 记 `LLMGATEWAY_API_KEY`（无下划线），与官方不一致，以官方为准**）/ 是否必需=是
- **endpoint 公式**：`POST {base_url}/chat/completions`
- **协议类型**：OpenAI 兼容（官方明示统一 OpenAI 兼容接口，可直接用 OpenAI SDK `baseURL`）
- **请求结构要点**：标准 OpenAI Chat Completions（model / messages 等）
- **响应结构要点**：标准 OpenAI 响应（`choices[0].message.content`）
- **流式**：SSE（`stream:true`，"Gateway will proxy the event stream unchanged"）
- **错误结构**：未知（文档未单独给出）
- **特有行为**：开源网关，路由/缓存/成本追踪；同时提供 Anthropic 兼容端点；支持图像/视频生成、Web 搜索、推理模型；可自托管

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方明示 OpenAI 兼容，base_url + Bearer key + 标准 chat/completions
- **可复用模型 ID 样例**：`gpt-4o`（官方示例）、`claude-3-7-sonnet`、`auto`
- **是否需扩展共享层**：否

#### 4. 风险与限制

- env var 命名 inventory（`LLMGATEWAY_API_KEY`）与官方（`LLM_GATEWAY_API_KEY`）不一致，须以官方为准
- 224 模型聚合，上游可用性随第三方波动

#### 5. 优先级建议

- **优先级**：P1
- **理由**：官方 OpenAI 兼容明确、薄封装成本低；env var 命名须校正
