# 第 11 批调研记录（14 个 provider）

本批覆盖 inventory 中 14 个 `implemented_in_aimux=false` 的 provider，按 canonical id 字母序排列。证据裁决遵循 RFC-0006 §2.1：以官方 API 文档/SDK 为首选证据，inventory 元数据（tier/protocol/openai_compatible）仅作线索。

---

### suno_api — SunoAPI

- **canonical ID**：suno_api
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：inventory 标 chat，但核验后实为音乐生成模态（非 chat）

#### 1. 官方协议证据

- **文档 URL**：https://docs.sunoapi.org/
- **核验来源**：官方 API 文档（docs.sunoapi.org，系第三方 Suno 代理产品 "Suno API" 的官方文档）
- **证据强度**：中（官方文档可确认其为音乐生成 API；但 inventory 实体来源为 new_api 网关，与该 docs.sunoapi.org 产品是否同一对象存歧义）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://api.sunoapi.org
- **鉴权**：方式=Bearer token（`Authorization: Bearer YOUR_API_KEY`）/ 环境变量=未知（inventory 无）/ 是否必需=是
- **endpoint 公式**：原生音乐生成端点（音乐生成、扩展、歌词、人声分离、翻唱、音乐视频等），非 OpenAI Chat Completions
- **协议类型**：专用模态（音乐/音频生成，原生任务式协议）
- **请求结构要点**：任务式请求（文本描述、模型版本 V4/V4.5/V5/V5.5 等），非 `messages` 数组
- **响应结构要点**：异步任务式（任务 ID + webhook 回调/轮询获取结果）
- **流式**：厂商专属（宣称 20 秒流式输出，非标准 SSE chat 流）
- **错误结构**：厂商专属
- **特有行为**：音乐/歌词/音频处理/音乐视频；webhook 回调；按 credits 计费

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：模态专用
- **依据**：实际为音乐生成模态 API，与 OpenAI Chat Completions 结构无关；inventory 的 chat 能力标注有误
- **可复用模型 ID 样例**：V5_5、V5、V4.5PLUS、V4.5、V4（音乐模型版本，非 chat 模型）
- **是否需扩展共享层**：否（不在 OpenAI chat 共享层范围）

#### 4. 风险与限制

- inventory 标 chat 能力与实际音乐模态不符
- 实体来源为 new_api 网关，可能与 docs.sunoapi.org 的 "Suno API" 产品非同一对象
- 音乐生成属异步任务式（回调/轮询），与同步 chat 模型差异大

#### 5. 优先级建议

- **优先级**：搁置
- **理由**：音乐生成模态不在 aimux 当前 chat 适配范围；inventory 实体身份存歧义，无 chat 协议价值

---

### tencent_coding_plan — Tencent Coding Plan (China)

- **canonical ID**：tencent_coding_plan
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://cloud.tencent.com/document/product/1772/128947
- **核验来源**：官方 API 文档（腾讯云）
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：OpenAI 兼容 `https://api.lkeap.cloud.tencent.com/coding/v3`；Anthropic 兼容 `https://api.lkeap.cloud.tencent.com/coding/anthropic`
- **鉴权**：方式=Bearer（套餐专属 API Key，格式 `sk-sp-xxxx`）/ 环境变量=`TENCENT_CODING_PLAN_API_KEY`（inventory）/ 是否必需=是
- **endpoint 公式**：`{base}/chat/completions`（OpenAI 兼容）
- **协议类型**：OpenAI 兼容（同时提供 Anthropic 兼容）
- **请求结构要点**：标准 OpenAI Chat Completions（model、messages、stream 等）
- **响应结构要点**：标准 OpenAI `chat.completion`
- **流式**：SSE（OpenAI 标准）
- **错误结构**：与 OpenAI 共享结构一致（推断）
- **特有行为**：套餐专属 Key（`sk-sp-xxxx`）与预/后付费 Key（`sk-xxxx`）不互通；官方声明"严禁 API 调用"用于自动化脚本/批量调用（仅限编程工具交互式使用）；模型库动态更新

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方明示"OpenAI 兼容协议"，base URL `/coding/v3`，标准 `chat/completions`
- **可复用模型 ID 样例**：glm-5、kimi-k2.5、hunyuan-t1、hunyuan-turbos、minimax-m2.5、tc-code-latest（auto）
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 官方禁止自动化/批量 API 调用，违规可能封禁 Key——与 aimux 作为程序化适配库的用途存在合规冲突
- 套餐模型动态调整，部分模型已下线

#### 5. 优先级建议

- **优先级**：P2
- **理由**：协议上属标准 OpenAI 薄封装（证据强），但官方使用条款限制程序化 API 调用，接入需谨慎，宜后续低优先级处理

---

### tencent_token_plan — Tencent Token Plan

- **canonical ID**：tencent_token_plan
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://cloud.tencent.com/document/product/1823/130060
- **核验来源**：官方 API 文档（腾讯云 TokenHub）
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：OpenAI 兼容 `https://api.lkeap.cloud.tencent.com/plan/v3`；Anthropic 兼容 `https://api.lkeap.cloud.tencent.com/plan/anthropic`
- **鉴权**：方式=Bearer / 环境变量=`TENCENT_TOKEN_PLAN_API_KEY`（inventory）/ 是否必需=是
- **endpoint 公式**：完整 URL `https://api.lkeap.cloud.tencent.com/plan/v3/chat/completions`（OpenAI 兼容）；Anthropic `…/plan/anthropic/v1/messages`
- **协议类型**：OpenAI 兼容（同时提供 Anthropic 兼容）
- **请求结构要点**：标准 OpenAI Chat Completions
- **响应结构要点**：标准 OpenAI `chat.completion`
- **流式**：SSE（OpenAI 标准）
- **错误结构**：与 OpenAI 共享结构一致（推断）
- **特有行为**：通用 Token Plan 与 Hy Token Plan 共用同一 API Key 与同一 base URL；面向"龙虾/编程"场景的个人订阅套餐

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方明示"兼容 OpenAI 接口协议工具"，给出完整 `/plan/v3/chat/completions` URL
- **可复用模型 ID 样例**：hy3、hy3-preview（Hy 系列）；通用系列另有 deepseek-v4-flash/pro、glm-5/5.1、kimi-k2.5、minimax-m2.5/m2.7、tc-code-latest（auto）
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 个人订阅套餐面向交互式编程工具，程序化批量调用存在与 coding plan 类似的合规风险
- 通用/Hy 共用端点，需以 model 参数区分

#### 5. 优先级建议

- **优先级**：P2
- **理由**：协议属标准 OpenAI 薄封装（证据强），但订阅套餐使用场景受限，宜后续处理

---

### tencent_token_plan_enterprise_auto — 腾讯云 Token Plan / Token Plan 企业版轻享套餐

- **canonical ID**：tencent_token_plan_enterprise_auto
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://cloud.tencent.com/document/product/1823/130660
- **核验来源**：官方 API 文档（腾讯云 TokenHub，企业版快速入门，含 cURL 请求与响应示例）
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：广州 `https://tokenhub.tencentmaas.com/plan/v3`；新加坡（intl）`https://tokenhub-intl.tencentmaas.com/plan/v3`
- **鉴权**：方式=Bearer（`Authorization: Bearer $your_api_key`）/ 环境变量=未知（inventory 无，建议 `TENCENT_TOKEN_PLAN_ENTERPRISE_API_KEY`）/ 是否必需=是
- **endpoint 公式**：`{base}/chat/completions`
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions（`{model, messages, stream}`）
- **响应结构要点**：`{id, object:"chat.completion", model, created, choices:[{index, message:{role, content, reasoning_content}, finish_reason}], usage:{prompt_tokens, completion_tokens, total_tokens, prompt_tokens_details, completion_tokens_details:{reasoning_tokens}}}`
- **流式**：SSE（OpenAI 标准，`stream` 字段控制）
- **错误结构**：与 OpenAI 共享结构一致（推断）
- **特有行为**：响应含 `reasoning_content` 字段（DeepSeek 风格扩展）与 `reasoning_tokens` 用量；企业版支持按 Key 配置模型权限、独占额度、TPM 限制

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方 cURL 示例与响应体直接确认标准 OpenAI Chat Completions 契约
- **可复用模型 ID 样例**：auto（轻享套餐可用模型较少）
- **是否需扩展共享层**：否（`reasoning_content` 为常见 DeepSeek 风格扩展，若共享层未覆盖可作可选扩展）

#### 4. 风险与限制

- 企业版按 Key 限定可用模型，未授权模型返回无权限错误
- 轻享与专业套餐可用模型集合不同

#### 5. 优先级建议

- **优先级**：P2
- **理由**：协议属标准 OpenAI 薄封装（证据强，含完整请求/响应示例）；企业版面向组织 API 调用，合规风险低于个人版，但属订阅套餐，宜后续处理

---

### tencent_token_plan_enterprise_pro — 腾讯云 Token Plan / Token Plan 企业版专业套餐

- **canonical ID**：tencent_token_plan_enterprise_pro
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://cloud.tencent.com/document/product/1823/130660
- **核验来源**：官方 API 文档（腾讯云 TokenHub，企业版快速入门，含 cURL 请求与响应示例）
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：广州 `https://tokenhub.tencentmaas.com/plan/v3`；新加坡（intl）`https://tokenhub-intl.tencentmaas.com/plan/v3`
- **鉴权**：方式=Bearer（`Authorization: Bearer $your_api_key`）/ 环境变量=未知（inventory 无，建议 `TENCENT_TOKEN_PLAN_ENTERPRISE_API_KEY`）/ 是否必需=是
- **endpoint 公式**：`{base}/chat/completions`
- **协议类型**：OpenAI 兼容
- **请求结构要点**：标准 OpenAI Chat Completions（`{model, messages, stream}`）
- **响应结构要点**：`{id, object:"chat.completion", model, created, choices:[{index, message:{role, content, reasoning_content}, finish_reason}], usage:{prompt_tokens, completion_tokens, total_tokens, prompt_tokens_details, completion_tokens_details:{reasoning_tokens}}}`
- **流式**：SSE（OpenAI 标准）
- **错误结构**：与 OpenAI 共享结构一致（推断）
- **特有行为**：响应含 `reasoning_content` 与 `reasoning_tokens`；专业套餐支持自定义积分规格、按 Key 模型权限/独占额度/TPM

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方 cURL 示例与响应体直接确认标准 OpenAI Chat Completions 契约
- **可复用模型 ID 样例**：auto、deepseek-v4-flash、deepseek-v4-flash-202605、deepseek-v4-pro、deepseek-v4-pro-202606
- **是否需扩展共享层**：否（`reasoning_content` 为可选扩展）

#### 4. 风险与限制

- 与轻享套餐共用端点与协议，仅可用模型集合不同
- 企业版按 Key 限定模型权限

#### 5. 优先级建议

- **优先级**：P2
- **理由**：协议属标准 OpenAI 薄封装（证据强）；企业版面向组织 API 调用，合规风险低，但属订阅套餐，宜后续处理

---

### tencent_token_plan_general_personal — 腾讯云 Token Plan / 通用 Token Plan（个人版）

- **canonical ID**：tencent_token_plan_general_personal
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://cloud.tencent.com/document/product/1823/130060
- **核验来源**：官方 API 文档（腾讯云 TokenHub）
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：OpenAI 兼容 `https://api.lkeap.cloud.tencent.com/plan/v3`；Anthropic 兼容 `https://api.lkeap.cloud.tencent.com/plan/anthropic`
- **鉴权**：方式=Bearer / 环境变量=未知（inventory 无）/ 是否必需=是
- **endpoint 公式**：完整 URL `https://api.lkeap.cloud.tencent.com/plan/v3/chat/completions`
- **协议类型**：OpenAI 兼容（同时提供 Anthropic 兼容）
- **请求结构要点**：标准 OpenAI Chat Completions
- **响应结构要点**：标准 OpenAI `chat.completion`
- **流式**：SSE（OpenAI 标准）
- **错误结构**：与 OpenAI 共享结构一致（推断）
- **特有行为**：通用 Token Plan 与 Hy Token Plan 共用同一 API Key 与 base URL；四档套餐（Lite/Standard/Pro/Max）按月 Token 额度计费

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方明示"兼容 OpenAI 接口协议工具"并给出完整 `/plan/v3/chat/completions` URL
- **可复用模型 ID 样例**：deepseek-v4-flash-202605、deepseek-v4-pro-202606、glm-5、glm-5.1、kimi-k2.5、minimax-m2.5/m2.7、tc-code-latest（auto）
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 个人订阅套餐面向交互式编程工具，程序化批量调用存在合规风险
- 与 tencent_token_plan / tencent_token_plan_hy_personal 共用同一端点，区别仅在套餐与可用模型

#### 5. 优先级建议

- **优先级**：P2
- **理由**：协议属标准 OpenAI 薄封装（证据强），但订阅套餐使用场景受限，宜后续处理

---

### tencent_token_plan_hy_personal — 腾讯云 Token Plan / Hy Token Plan（个人版）

- **canonical ID**：tencent_token_plan_hy_personal
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://cloud.tencent.com/document/product/1823/130060
- **核验来源**：官方 API 文档（腾讯云 TokenHub）
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：OpenAI 兼容 `https://api.lkeap.cloud.tencent.com/plan/v3`；Anthropic 兼容 `https://api.lkeap.cloud.tencent.com/plan/anthropic`
- **鉴权**：方式=Bearer / 环境变量=未知（inventory 无）/ 是否必需=是
- **endpoint 公式**：完整 URL `https://api.lkeap.cloud.tencent.com/plan/v3/chat/completions`
- **协议类型**：OpenAI 兼容（同时提供 Anthropic 兼容）
- **请求结构要点**：标准 OpenAI Chat Completions
- **响应结构要点**：标准 OpenAI `chat.completion`
- **流式**：SSE（OpenAI 标准）
- **错误结构**：与 OpenAI 共享结构一致（推断）
- **特有行为**：Hy Token Plan 仅提供 Hy3 模型；与通用 Token Plan 共用同一 API Key 与 base URL

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方明示"兼容 OpenAI 接口协议工具"并给出完整 `/plan/v3/chat/completions` URL
- **可复用模型 ID 样例**：hy3、hy3-preview
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 个人订阅套餐，程序化批量调用存在合规风险
- 与通用 Token Plan 共用端点，仅模型集合不同

#### 5. 优先级建议

- **优先级**：P2
- **理由**：协议属标准 OpenAI 薄封装（证据强），但订阅套餐使用场景受限，宜后续处理

---

### tencent_tokenhub — Tencent TokenHub

- **canonical ID**：tencent_tokenhub
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://cloud.tencent.com/document/product/1823/130050 （产品简介）；https://cloud.tencent.com/document/product/1823/130058 （快速入门，含 cURL/多语言 SDK 示例）
- **核验来源**：官方 API 文档（腾讯云）
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://tokenhub.tencentmaas.com/v1`
- **鉴权**：方式=Bearer（`Authorization: Bearer YOUR_API_KEY`）/ 环境变量=`TENCENT_TOKENHUB_API_KEY`（inventory）/ 是否必需=是
- **endpoint 公式**：`{base}/chat/completions`
- **协议类型**：OpenAI 兼容（官方明示"大模型服务平台 TokenHub 兼容 OpenAI API 协议"）
- **请求结构要点**：标准 OpenAI Chat Completions（`{model, messages, stream}`）
- **响应结构要点**：标准 OpenAI `chat.completion`（官方示例用 OpenAI SDK 直接消费）
- **流式**：SSE（OpenAI 标准，`stream:true`）
- **错误结构**：与 OpenAI 共享结构一致（推断）
- **特有行为**：一站式聚合平台（混元/DeepSeek/MiniMax/Kimi/GLM/通义千问等），统一 API；支持文本/图片/视频/3D 多能力；按 Token 计费（非订阅套餐）

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方快速入门给出 `https://tokenhub.tencentmaas.com/v1/chat/completions` 与 OpenAI SDK/cURL 示例，直接确认 OpenAI Chat Completions 契约
- **可复用模型 ID 样例**：hy3、hy3-preview（inventory）；平台另支持 deepseek-v3 等多厂商模型
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 平台聚合多厂商模型，单模型能力/参数支持以模型列表为准
- 与 Token Plan/Coding Plan 等订阅套餐的 base URL 与 Key 不互通

#### 5. 优先级建议

- **优先级**：P1
- **理由**：主流国产模型聚合平台，OpenAI 兼容协议证据强、按量计费无订阅合规限制，近期接入价值高

---

### the_grid_ai — The Grid AI

- **canonical ID**：the_grid_ai
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.65
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://thegrid.ai/docs （Overview，给出 Consumption API base URL 与鉴权）
- **核验来源**：官方 API 文档
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：OpenAI Chat Completions `https://api.thegrid.ai/v1`；Anthropic Messages（beta）`https://messages-beta.api.thegrid.ai/v1`
- **鉴权**：方式=Bearer（OpenAI，`Authorization: Bearer <key>`）；Anthropic 端用 `x-api-key: <key>` / 环境变量=`THEGRIDAI_API_KEY`（inventory）/ 是否必需=是
- **endpoint 公式**：`{base}/chat/completions`（OpenAI 兼容）
- **协议类型**：OpenAI 兼容（同时提供 Anthropic Messages 兼容）
- **请求结构要点**：标准 OpenAI Chat Completions；`model` 字段传 instrument 字符串（如 `text-prime`、`code-prime`、`claude-opus-latest`）而非具体模型名
- **响应结构要点**：标准 OpenAI `chat.completion`（官方称兼容 OpenAI Chat Completions 格式）
- **流式**：SSE（OpenAI 标准，推断）
- **错误结构**：与 OpenAI 共享结构一致（推断）
- **特有行为**："现货市场"按 task type + quality tier（Standard/Prime/Max）或 lab latest 市场路由到竞争供应商；instrument 串作为 model 名

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方明示"compatible with OpenAI Chat Completions"，给出 base URL `https://api.thegrid.ai/v1` 与 Bearer 鉴权
- **可复用模型 ID 样例**：agent-max、agent-prime、agent-standard、code-max、code-prime、text-prime、gpt-sol-latest、claude-opus-latest
- **是否需扩展共享层**：否

#### 4. 风险与限制

- model 字段语义为 instrument（任务/质量档）而非传统模型 ID，需文档化
- Anthropic 端为 beta

#### 5. 优先级建议

- **优先级**：P1
- **理由**：OpenAI 兼容协议证据强、接入简单（仅改 base URL），近期接入价值高

---

### thinkingmachines — Thinking Machines

- **canonical ID**：thinkingmachines
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：
  - Anthropic 兼容：https://tinker-docs.thinkingmachines.ai/tinker/compatible-apis/anthropic
  - OpenAI 兼容：https://tinker-docs.thinkingmachines.ai/tinker/compatible-apis/openai/
- **核验来源**：官方 API 文档（Tinker Documentation）
- **证据强度**：强（两个兼容端点均有官方文档与代码示例）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：
  - OpenAI 兼容 `https://tinker.thinkingmachines.dev/services/tinker-prod/oai/api/v1`（支持 `/completions` 与 `/chat/completions`）
  - Anthropic 兼容 `https://tinker.thinkingmachines.dev/services/tinker-prod/anthropic/api`（客户端追加 `/v1/messages`）
  - 注：inventory 的 base_url 指向 Anthropic 端点，非 OpenAI 端点
- **鉴权**：方式=OpenAI 端用 Bearer（OpenAI SDK 默认）；Anthropic 端用 `x-api-key` 或 `Authorization: Bearer` / 环境变量=`TINKER_API_KEY`（inventory + 文档）/ 是否必需=是
- **endpoint 公式**：OpenAI `{base}/chat/completions`、`{base}/completions`
- **协议类型**：OpenAI 兼容（同时提供 Anthropic Messages 兼容）
- **请求结构要点**：标准 OpenAI Chat Completions；model 字段传 Tinker sampler 权重路径（`tinker://...:train:0/sampler_weights/000080`）或模型名（如 `thinkingmachines/Inkling`）
- **响应结构要点**：标准 OpenAI 结构；`/chat/completions` 支持非标准 `separate_reasoning`（默认 true）将推理置于 `reasoning_content` 字段
- **流式**：SSE（OpenAI 标准；推理与正文分 separate_reasoning/content 事件）
- **错误结构**：与 OpenAI 共享结构一致（推断）
- **特有行为**：非标准 `separate_reasoning`、`reasoning_effort`（OpenAI 串或 [0.0,0.99] 浮点）；model 为动态 sampler checkpoint 路径；当前为 beta，面向测试/内部低流量

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方文档给出 OpenAI 兼容 base URL 与 `/chat/completions` 代码示例，直接确认 OpenAI Completions 契约
- **可复用模型 ID 样例**：`thinkingmachines/Inkling`、`thinkingmachines/Inkling:peft:262144`、`tinker://<uuid>:train:0/sampler_weights/000080`
- **是否需扩展共享层**：否（`separate_reasoning`/`reasoning_content` 为可选扩展，核心 OpenAI 契约无需改动）

#### 4. 风险与限制

- inventory base_url 错误指向 Anthropic 端点，OpenAI 薄封装需改用 `…/oai/api/v1`
- model 标识为动态 sampler checkpoint 路径，非稳定模型 ID
- 当前为 beta，面向测试/内部用，延迟与吞吐可能波动
- `reasoning_effort`/`separate_reasoning` 为非标准字段

#### 5. 优先级建议

- **优先级**：P2
- **理由**：OpenAI 兼容协议证据强可薄封装，但为 beta 且 model 标识为动态训练 checkpoint，生产价值有限，宜后续处理

---

### tinfoil — Tinfoil

- **canonical ID**：tinfoil
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://docs.tinfoil.sh （Introduction）；https://docs.tinfoil.sh/quickstart （含多语言 SDK 与 cURL 示例）
- **核验来源**：官方 API 文档 + 官方 SDK
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://inference.tinfoil.sh/v1`
- **鉴权**：方式=Bearer（`Authorization: Bearer $TINFOIL_API_KEY`）/ 环境变量=`TINFOIL_API_KEY`（inventory + 文档）/ 是否必需=是
- **endpoint 公式**：`{base}/chat/completions`
- **协议类型**：OpenAI 兼容（官方称 "OpenAI compatible inference API"，"Drop-in Compatibility"，SDK 兼容 OpenAI API）
- **请求结构要点**：标准 OpenAI Chat Completions（`{model, messages}`）；支持 tool calling、structured outputs、image（base64/url）、document
- **响应结构要点**：标准 OpenAI `chat.completion`（官方 SDK 直接复用 OpenAI 客户端类型）
- **流式**：SSE（OpenAI 标准，推断）
- **错误结构**：与 OpenAI 共享结构一致（推断）
- **特有行为**：基于安全 enclave 的隐私推理；SDK 自动做远程证明（attestation）校验；裸 HTTP 调用为标准 OpenAI 兼容，证明校验由 SDK 侧完成

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方 quickstart 给出 `https://inference.tinfoil.sh/v1/chat/completions` 与 Bearer 鉴权，OpenAI SDK 直接可用
- **可复用模型 ID 样例**：gpt-oss-120b、gpt-oss-safeguard-120b、gemma4-31b、glm-5-2、kimi-k2-6
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 隐私 enclave 证明校验为厂商特有，但属 SDK 侧可选行为，不影响 OpenAI 兼容线路协议
- 模型为开源模型集合，能力以模型为准

#### 5. 优先级建议

- **优先级**：P2
- **理由**：OpenAI 兼容协议证据强可薄封装，但属隐私 enclave 细分场景，主流需求有限，宜后续处理

---

### tokenflux — Tokenflux

- **canonical ID**：tokenflux
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://docs.tokenflux.ai/quickstart （含 JS/Python/cURL 示例）；https://tokenflux.ai/docs/introduction
- **核验来源**：官方 API 文档
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://tokenflux.ai/v1`
- **鉴权**：方式=同时接受 `Authorization: Bearer <key>`（OpenAI SDK 默认）与 `X-Api-Key: <key>`（cURL 示例）/ 环境变量=未知（inventory 无，建议 `TOKENFLUX_API_KEY`）/ 是否必需=是
- **endpoint 公式**：`{base}/chat/completions`；另有 `/v1/embeddings`、`/v1/images/generations`、`/v1/models`
- **协议类型**：OpenAI 兼容（官方称 "fully compatible with OpenAI's client libraries"，"switch by simply changing the base URL and API key"）
- **请求结构要点**：标准 OpenAI Chat Completions（`{model, messages, stream}`）
- **响应结构要点**：标准 OpenAI `chat.completion`（`choices[0].message.content`、流式 `choices[0].delta.content`）
- **流式**：SSE（OpenAI 标准，`stream:true`）
- **错误结构**：与 OpenAI 共享结构一致（推断）
- **特有行为**：200+ 模型聚合（OpenAI/Anthropic/Google/Mistral/DeepSeek 等）；cURL 示例用 `X-Api-Key` 头（与 OpenAI SDK 的 Bearer 并存）

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方 quickstart 给出 `https://tokenflux.ai/v1/chat/completions` 与 OpenAI SDK/cURL 示例，直接确认 OpenAI Chat Completions 契约
- **可复用模型 ID 样例**：gpt-4o、claude-3.5-sonnet、gemini-pro、mistral-large、deepseek-chat、anthropic/claude-3.5-sonnet（inventory）
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 鉴权头同时支持 Bearer 与 X-Api-Key，OpenAI 共享层用 Bearer 即可
- inventory 无 base_url/env 线索，本次由搜索定位官方文档确认

#### 5. 优先级建议

- **优先级**：P1
- **理由**：主流模型聚合网关，OpenAI 兼容协议证据强、接入简单，近期接入价值高

---

### triton — Triton

- **canonical ID**：triton
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：chat（inventory 标注）

#### 1. 官方协议证据

- **文档 URL**：https://docs.nvidia.com/deeplearning/triton-inference-server/user-guide/docs/client_guide/openai_readme.html （OpenAI 兼容前端）；https://docs.litellm.ai/docs/providers/triton-inference-server （litellm 集成）
- **核验来源**：官方文档（NVIDIA Triton Inference Server）
- **证据强度**：中（官方文档确认存在 OpenAI 兼容前端与原生 /generate 协议；但 inventory 实体无 base_url/模型/鉴权，属自托管服务器，非厂商 provider）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：用户自定义（自托管 Triton 服务器，如 `http://localhost:8000`）
- **鉴权**：方式=默认无鉴权，可通过 `--allow-http-header` 配置 token / 环境变量=无 / 是否必需=否（取决于部署）
- **endpoint 公式**：OpenAI 兼容前端 `{base}/v1/chat/completions`、`{base}/v1/models`、`{base}/v1/embeddings`；原生协议 `{base}/generate`、`{base}/embeddings`
- **协议类型**：原生（Triton 原生 /generate、/embeddings）+ OpenAI 兼容前端可选
- **请求结构要点**：OpenAI 前端为标准 Chat Completions；原生 /generate 为 `{text_input, parameters}` 等 Triton 专属结构
- **响应结构要点**：OpenAI 前端为标准 chat.completion；原生 /generate 为 Triton 专属响应
- **流式**：SSE（OpenAI 前端）/ 原生流式（推断）
- **错误结构**：厂商专属（Triton 原生）/ OpenAI 兼容（前端）
- **特有行为**：自托管推理服务器，模型由用户自行部署；litellm 以 `triton/` 前缀路由至原生 /generate

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：待定（若用 OpenAI 前端可薄封装；若用原生 /generate 则需原生实现）
- **依据**：协议取决于选用前端还是原生端点；且为自托管，无固定 base_url/鉴权/模型
- **可复用模型 ID 样例**：无（由部署的模型决定）
- **是否需扩展共享层**：否（若走 OpenAI 前端）/ 是（若走原生 /generate 需原生适配）

#### 4. 风险与限制

- inventory 实体无 base_url/模型/鉴权线索，无法定位具体服务
- 自托管服务器，不属于厂商 provider 范畴
- 协议路径取决于部署配置（前端 vs 原生）

#### 5. 优先级建议

- **优先级**：搁置
- **理由**：自托管推理服务器，无固定 base_url/鉴权/模型清单，不属于厂商 provider；inventory 实体线索不足，无法确定协议路径

---

### trustedrouter — TrustedRouter

- **canonical ID**：trustedrouter
- **aliases**：无
- **provider_kind**：model_vendor
- **inventory 分层**：tier=unknown / protocol=unknown / openai_compatible=null / confidence=0.45
- **能力**（本次调研覆盖）：chat

#### 1. 官方协议证据

- **文档 URL**：https://trustedrouter.com/docs （Quickstart + API reference）
- **核验来源**：官方 API 文档
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：`https://api.trustedrouter.com/v1`
- **鉴权**：方式=Bearer（`Bearer sk-tr-…`）/ 环境变量=`TRUSTEDROUTER_API_KEY`（inventory + 文档）/ 是否必需=是
- **endpoint 公式**：`{base}/chat/completions`；另有 `/v1/responses`、`/v1/embeddings`、`/v1/models`
- **协议类型**：OpenAI 兼容（同时提供 Anthropic 兼容；官方称 "point any OpenAI-compatible SDK at TrustedRouter"）
- **请求结构要点**：标准 OpenAI Chat Completions；model 字段传路由别名（如 `trustedrouter/auto`、`trustedrouter/socrates`）；可选 OpenRouter 兼容的 `provider` 对象（`min_privacy`、`data_collection`）
- **响应结构要点**：标准 OpenAI `chat.completion`（官方示例用 OpenAI SDK 直接消费）
- **流式**：SSE（OpenAI 标准，推断）
- **错误结构**：与 OpenAI 共享结构一致（推断）
- **特有行为**：证明网关（attested gateway）；`provider.min_privacy`（zdr/confidential）硬性隐私下限；模型别名路由（auto/eu/socrates/iris/prometheus/zeus/openpatcher 等）；支持 Responses API web search

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：薄封装
- **依据**：官方明示 OpenAI 兼容，给出 base URL `https://api.trustedrouter.com/v1` 与 `/v1/chat/completions` 端点、Bearer 鉴权
- **可复用模型 ID 样例**：trustedrouter/auto、trustedrouter/cheap、trustedrouter/e2e、trustedrouter/fast、trustedrouter/synth、trustedrouter/socrates
- **是否需扩展共享层**：否（`provider.min_privacy` 等为可选 OpenRouter 兼容扩展，不影响核心薄封装）

#### 4. 风险与限制

- model 字段为路由别名而非具体模型，需文档化
- `provider.*` 隐私字段为厂商扩展，但可选

#### 5. 优先级建议

- **优先级**：P1
- **理由**：OpenAI 兼容协议证据强、接入简单（仅改 base URL + key），近期接入价值高
