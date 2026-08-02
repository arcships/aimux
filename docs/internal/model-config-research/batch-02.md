# Batch 02 — Model Request Config 调研

> 状态: ✅ 已完成调研 · 厂商数: 42
> 模板见 [_template.md](_template.md) · 方法论见 [README.md](README.md)
> 证据硬性要求: 每条目必须有例子或证明(A/B/C 级),禁止编造来源。

## 厂商清单

| # | id | display | base_url | env_var | profile |
|---|---|---|---|---|---|
| 1 | chatgpt | ChatGPT (璁㈤槄) | https://chatgpt.com/backend-api/codex | CHATGPT_API_KEY | OpenAICompatProfile::full() |
| 2 | cherryin | cherryin | https://open.cherryin.net | CHERRYIN_API_KEY | OpenAICompatProfile::full() |
| 3 | chutes | Chutes | https://llm.chutes.ai/v1 | CHUTES_API_KEY | OpenAICompatProfile::full() |
| 4 | clarifai | Clarifai | https://api.clarifai.com/v2/ext/openai/v1 | CLARIFAI_API_KEY | OpenAICompatProfile::full() |
| 5 | claudinio | Claudinio | https://api.claudin.io | CLAUDINIO_API_KEY | OpenAICompatProfile::full() |
| 6 | cline_pass | Cline | https://api.cline.bot/v1 | CLINE_API_KEY | OpenAICompatProfile::full() |
| 7 | closeai | CloseAI | https://api.closeai-proxy.xyz/v1 | CLOSEAI_API_KEY | OpenAICompatProfile::full() |
| 8 | cloudferro_sherlock | CloudFerro Sherlock | https://api-sherlock.cloudferro.com/openai/v1 | CLOUDFERRO_SHERLOCK_API_KEY | OpenAICompatProfile::full() |
| 9 | cloudflare | Cloudflare | https://api.cloudflare.com/client/v4/accounts/{CLOUDFLARE_ACCOUNT_ID}/ai/v1 | CLOUDFLARE_API_KEY | OpenAICompatProfile::full() |
| 10 | cloudflare_workers_ai | Cloudflare Workers AI | https://api.cloudflare.com/client/v4/accounts/{CLOUDFLARE_ACCOUNT_ID}/ai/v1 | CLOUDFLARE_API_KEY | OpenAICompatProfile::full() |
| 11 | codestral | Codestral (Mistral) | https://api.mistral.ai/v1 | CODESTRAL_API_KEY | OpenAICompatProfile::full() |
| 12 | cometapi | CometAPI | https://api.cometapi.com/v1 | COMETAPI_API_KEY | OpenAICompatProfile::full() |
| 13 | commandcode | CommandCode | https://api.commandcode.com/v1 | COMMANDCODE_API_KEY | OpenAICompatProfile::full() |
| 14 | compactifai | CompactifAI | https://api.compactif.ai/v1 | COMPACTIFAI_API_KEY | OpenAICompatProfile::full() |
| 15 | copilot | GitHub Copilot | https://api.githubcopilot.com | COPILOT_API_KEY | OpenAICompatProfile::full() |
| 16 | cortecs | Cortecs | https://api.cortecs.ai/v1/ | CORTECS_API_KEY | OpenAICompatProfile::full() |
| 17 | coze | Coze (鎵ｅ瓙) | https://api.coze.cn/v1 | COZE_API_KEY | OpenAICompatProfile::full() |
| 18 | crof | CrofAI | https://crof.ai/v1 | CROF_API_KEY | OpenAICompatProfile::full() |
| 19 | crossmodel | CrossModel | https://api.crossmodel.ai/v1 | CROSSMODEL_API_KEY | OpenAICompatProfile::full() |
| 20 | crusoe | Crusoe | https://api.inference.crusoecloud.com/v1 | CRUSOE_API_KEY | OpenAICompatProfile::full() |
| 21 | daoxe | DaoXE | https://daoxe.com/v1 | DAOXE_API_KEY | OpenAICompatProfile::full() |
| 22 | darkbloom | Darkbloom | https://api.darkbloom.dev/v1 | DARKBLOOM_API_KEY | OpenAICompatProfile::full() |
| 23 | databricks | Databricks | https://databricks.com/serving-endpoints | DATABRICKS_API_KEY | OpenAICompatProfile::full() |
| 24 | datarobot | DataRobot | https://app.datarobot.com/api/v2 | DATAROBOT_API_TOKEN | OpenAICompatProfile::full() |
| 25 | deepbricks | DeepBricks | https://api.deepbricks.ai/v1 | DEEPBRICKS_API_KEY | OpenAICompatProfile::full() |
| 26 | deepinfra | DeepInfra | https://api.deepinfra.com/v1/openai | DEEPINFRA_API_KEY | OpenAICompatProfile::full() |
| 27 | deepseek | DeepSeek | https://api.deepseek.com/v1 | DEEPSEEK_API_KEY | OpenAICompatProfile::deepseek() |
| 28 | digitalocean | DigitalOcean | https://inference.do-ai.run | DIGITALOCEAN_ACCESS_TOKEN | OpenAICompatProfile::full() |
| 29 | dinference | DInference | https://api.dinference.com/v1 | DINFERENCE_API_KEY | OpenAICompatProfile::full() |
| 30 | doubao | Doubao | https://ark.cn-beijing.volces.com/api/v3 | ARK_API_KEY | OpenAICompatProfile::full() |
| 31 | doubleword | Doubleword | https://api.doubleword.ai/v1 | DOUBLEWORD_API_KEY | OpenAICompatProfile::full() |
| 32 | drun | D.Run (China) | https://chat.d.run/v1 | DRUN_API_KEY | OpenAICompatProfile::full() |
| 33 | ebcloud | EBCloud | https://maas-api.ebcloud.com/v1 | EBCLOUD_API_KEY | OpenAICompatProfile::full() |
| 34 | embercloud | Embercloud | https://api.embercloud.com/v1 | EMBERCLOUD_API_KEY | OpenAICompatProfile::full() |
| 35 | empiriolabs | EmpirioLabs AI | https://api.empiriolabs.ai/v1 | EMPIRIOLABS_API_KEY | OpenAICompatProfile::full() |
| 36 | evroc | evroc | https://models.think.evroc.com/v1 | EVROC_API_KEY | OpenAICompatProfile::full() |
| 37 | fastcrw | FastCRW | https://fastcrw.com/api/v1 | FASTCRW_API_KEY | OpenAICompatProfile::full() |
| 38 | fastgpt | FastGPT | https://api.fastgpt.in/v1 | FASTGPT_API_KEY | OpenAICompatProfile::full() |
| 39 | fastrouter | FastRouter | https://api.fastrouter.ai/v1 | FASTROUTER_API_KEY | OpenAICompatProfile::full() |
| 40 | featherless_ai | Featherless AI | https://api.featherless.ai/v1 | FEATHERLESS_API_KEY | OpenAICompatProfile::full() |
| 41 | firepass | Fireworks (Firepass) | https://api.fireworks.ai/inference/v1锛圤penAI | FIREWORKS_API_KEY | OpenAICompatProfile::full() |
| 42 | fireworks | Fireworks | https://api.fireworks.ai/inference/v1 | FIREWORKS_API_KEY | OpenAICompatProfile::full() |

## 调研条目

> 说明: 本批次 42 家中,多数为 OpenAI 兼容 thin wrapper/代理,官方无独立请求格式文档。
> 有独立官方文档/参考实现的大厂商逐项给出差异与证据;查不到任何资料的厂商按
> "registry 声明为 OpenAI 兼容(full()),无差异"处理并标 ⚠️ 证据不足,不臆造差异。

---

### chatgpt — ChatGPT (订阅)

- **registry 现状**: profile=`full()` · base_url=`https://chatgpt.com/backend-api/codex` · env=`CHATGPT_API_KEY`（[openai_compat_registry.rs:394-402](../../aimux-providers/src/openai_compat_registry.rs#L394)）
- **变体**: 无（registry 仅此一条;对应模型为 gpt-5-codex 系列,走 Codex backend）

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 使用 **Responses API** 请求体(`input`/`max_output_tokens`/`reasoning`),**不是** Chat Completions 的 `messages`/`max_tokens` | `POST https://chatgpt.com/backend-api/codex/responses` body: `{"model":"gpt-5-codex","input":"...","max_output_tokens":10000,"reasoning":{"effort":"high"}}` | C | [simonwillison.net — Reverse engineering Codex CLI](https://simonwillison.net/2025/Nov/9/gpt-5-codex-mini/) |
| 能力支持 | Responses API 能力集(含 tool_choice/tools/parallel_tool_calls/stream SSE) | 同 OpenAI Responses API | C | [learn.chatgpt.com — Authentication](https://learn.chatgpt.com/docs/auth) |
| 思考机制 | gpt-5-codex 为推理模型,`reasoning.effort` 控制思考档位 | `"reasoning":{"effort":"high"}` | C | 同上,Responses 格式 |
| 流式/usage | SSE 流式事件,usage 在流内返回 | — | C | [openclaw issue #81756 — codex/responses 需 WebSocket/SSE 传输](https://github.com/openclaw/openclaw/issues/81756) |
| 消息格式 | Responses `input` 消息项(role: user/assistant)而非 chat messages | 同 Responses API | C | 同上 |
| 特殊字段 | 无 | - | - | - |
| headers/认证 | `Authorization: Bearer {ChatGPT access token|API key}` + **`OAI-Product-Sku: codex`** header + account-id 类身份 header;token 来自 ChatGPT 订阅 OAuth 或 API key | `chatgpt_client.rs`: `.header(OAI_PRODUCT_SKU_HEADER, CODEX_PRODUCT_SKU)`（`OAI-Product-Sku: codex`） | B | [openai/codex codex-rs/chatgpt/src/chatgpt_client.rs](https://github.com/openai/codex/blob/main/codex-rs/chatgpt/src/chatgpt_client.rs) |
| URL/端点 | 端点路径为 `{base_url}/responses` 而非 `{base_url}/chat/completions`;base_url=`https://chatgpt.com/backend-api/codex` | `POST .../backend-api/codex/responses` | C | [simonwillison.net](https://simonwillison.net/2025/Nov/9/gpt-5-codex-mini/) |
| 模型 ID | gpt-5-codex / gpt-5.1-codex 等 Codex 系列模型名 | `"model":"gpt-5-codex"` | C | [learn.chatgpt.com](https://learn.chatgpt.com/docs/auth) |

#### 2. aimux 现状对比

- **对比结论**: ❌ 未覆盖（端点路径与请求格式双重不匹配）
- **aimux 代码位置**: `openai_compat_registry.rs:394-402`（声明为 `full()` chat provider）;`model.rs:endpoint()` 拼接 `{base_url}/chat/completions`;responses 专用模型在 `openai/responses/mod.rs`（拼接 `{base_url}/responses`）
- **差距说明**: ① registry 将 chatgpt 声明为 chat-completions 薄封装,aimux 会请求 `https://chatgpt.com/backend-api/codex/chat/completions`,而真实端点是 `.../codex/responses`(Responses API 请求体);② `OAI-Product-Sku: codex` 与 ChatGPT 订阅 token 认证未在配置层暴露。
- **建议动作**: chatgpt 应映射到 `OpenAIResponsesModel`(或新增 provider 类型),并支持特殊 headers(`OAI-Product-Sku`)与订阅 token 来源;或至少给出 profile/端点标记,禁止默认 chat-completions 路径。

#### 3. 证据与验证

- **证据等级**: B + C
- **验证状态**: 🔲 未验证(无 aimux 侧 cassette)
- **存疑标记**: 无

---

### cherryin — cherryin

- **registry 现状**: profile=`full()` · base_url=`https://open.cherryin.net` · env=`CHERRYIN_API_KEY`（[openai_compat_registry.rs:403-411](../../aimux-providers/src/openai_compat_registry.rs#L403)）
- **变体**: 无

#### 1. request 差异发现

| 类别 | 差异 | 例子 | 证据等级 | 来源 |
|------|------|------|---------|------|
| 参数命名 | 无差异（按 OpenAI 兼容声明） | - | - | - |
| 能力支持 | 无差异（按 OpenAI 兼容声明） | - | - | - |
| 思考机制 | 未查到公开文档 ⚠️ | - | - | - |
| 流式/usage | 未查到公开文档 ⚠️ | - | - | - |
| 消息格式 | 无差异（按 OpenAI 兼容声明） | - | - | - |
| 特殊字段 | 未查到公开文档 ⚠️ | - | - | - |
| headers/认证 | 未查到公开文档 ⚠️（声明为 Bearer key） | - | - | - |
| URL/端点 | 无差异（按 registry 声明,`{base_url}/chat/completions`） | - | - | - |
| 模型 ID | 未查到公开文档 ⚠️ | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖（按 full() 声明）;⚠️ 证据不足
- **aimux 代码位置**: `openai_compat_registry.rs:403-411`
- **差距说明**: 未找到 cherryin(open.cherryin.net)的独立 API 文档,无法确认除声明外的特殊配置。
- **建议动作**: 暂无动作;待有实测或文档后补充。

#### 3. 证据与验证

- **证据等级**: 无（未找到来源）
- **验证状态**: 🔲 未验证
- **存疑标记**: ⚠️ 证据不足（无法确认除 OpenAI 兼容声明外的任何细节）

---

### chutes — Chutes

- **registry 现状**: profile=`full()` · base_url=`https://llm.chutes.ai/v1` · env=`CHUTES_API_KEY`（[openai_compat_registry.rs:412-420](../../aimux-providers/src/openai_compat_registry.rs#L412)）
- **变体**: 无

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异(OpenAI chat completions) | `{"model":"...","messages":[...],"stream":true}` | C | [chutes.ai/llms.txt](https://chutes.ai/llms.txt) |
| 能力支持 | 无差异;`supported_features`/`supported_sampling_parameters` 由 `/v1/models` 目录逐模型声明 | - | C | 同上 |
| 思考机制 | 无统一机制;按模型(如 deepseek 系)经 `chat_template_kwargs: {thinking: true}` 传递 ⚠️(见 chutes 社区帖) | `{"chat_template_kwargs":{"thinking":true}}` | C | [reddit r/chutesAI — DeepSeek V3.2 Reasoning API Reference](https://www.reddit.com/r/chutesAI/comments/1piyffd/deepseek_v32_reasoning_api_reference/) |
| 流式/usage | 无差异(SSE + usage) | - | C | [chutes.ai/llms.txt](https://chutes.ai/llms.txt) |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | `model` 字段支持路由语法(见模型 ID) | - | - | - |
| headers/认证 | `Authorization: Bearer cpk_...`;**明确不支持 X-API-Key** | `Authorization: Bearer cpk_xxx` | C | [chutes.ai/llms.txt](https://chutes.ai/llms.txt) |
| URL/端点 | 无差异(`https://llm.chutes.ai/v1` + `/chat/completions`);管理 API 在 `api.chutes.ai` | - | C | 同上 |
| 模型 ID | **路由约定**:`model` 可为 `default`、`default:latency`/`default:throughput`、`modelA,modelB`(逗号内联 failover)、`modelA,modelB:latency` | `"model":"qwen/qwen3-32b,meta-llama/llama-3.3-70b"` | C | [chutes.ai/llms.txt](https://chutes.ai/llms.txt) |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖(请求体/认证/URL 均兼容);模型 ID 路由语法 🔶 需用户侧支持
- **aimux 代码位置**: `openai_compat_registry.rs:412-420`
- **差距说明**: aimux 把 `model` 原样透传,逗号 failover 列表与 `:latency` 后缀可工作,但库内无专门解析。
- **建议动作**: 补测试即可;模型路由语法留给用户传入 model_id。

#### 3. 证据与验证

- **证据等级**: C
- **验证状态**: 🔲 未验证(仅文档引用)
- **存疑标记**: 无

---

### clarifai — Clarifai

- **registry 现状**: profile=`full()` · base_url=`https://api.clarifai.com/v2/ext/openai/v1` · env=`CLARIFAI_API_KEY`（[openai_compat_registry.rs:421-429](../../aimux-providers/src/openai_compat_registry.rs#L421)）
- **变体**: 无

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异;`max_completion_tokens` 与 `max_tokens` 均可用 | `client.chat.completions.create(model=..., messages=[...], max_completion_tokens=100, temperature=0.7)` | C | [docs.clarifai.com — OpenAI](https://docs.clarifai.com/compute/inference/open-ai/) |
| 能力支持 | 支持 tool calling、streaming、多模态(image_url base64)、developer 角色 | messages 支持 `system/user/assistant/developer/tool` 角色 | C | 同上 |
| 思考机制 | 无统一机制(按托管模型,如 DeepSeek-R1 的 reasoning_content 透传) ⚠️ | - | - | - |
| 流式/usage | 无差异(SSE 流式) | `stream=True` | C | 同上 |
| 消息格式 | 无差异;多模态用标准 `image_url` data URI | `{"type":"image_url","image_url":{"url":"data:image/jpeg;base64,..."}}` | C | 同上 |
| 特殊字段 | 无 | - | - | - |
| headers/认证 | OpenAI SDK 以 `api_key=PAT` 直连(即 `Authorization: Bearer <PAT>`);Clarifai 原生 API 用 `Authorization: Key <PAT>`,OpenAI 兼容端点按官方示例接受 SDK 的 Bearer 形式 | `client = OpenAI(base_url="https://api.clarifai.com/v2/ext/openai/v1", api_key=os.environ["CLARIFAI_PAT"])` | C | 同上 |
| URL/端点 | 无差异(registry base_url 与官方一致) | - | C | 同上 |
| 模型 ID | **模型参数为完整 Clarifai URL**,如 `https://clarifai.com/openai/chat-completion/models/gpt-oss-120b` | `model="https://clarifai.com/openai/chat-completion/models/gpt-oss-120b"` | C | 同上 |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖(URL/认证/能力均兼容);模型 ID 约定 🔶(完整 URL 由用户传入即可)
- **aimux 代码位置**: `openai_compat_registry.rs:421-429`;认证逻辑 `model.rs`(Bearer)
- **差距说明**: 无实质差距;仅需注意用户须传完整模型 URL。
- **建议动作**: 补测试即可。

#### 3. 证据与验证

- **证据等级**: C
- **验证状态**: 🔲 未验证(仅文档引用)
- **存疑标记**: 无

---

### claudinio — Claudinio

- **registry 现状**: profile=`full()` · base_url=`https://api.claudin.io` · env=`CLAUDINIO_API_KEY`（[openai_compat_registry.rs:430-438](../../aimux-providers/src/openai_compat_registry.rs#L430)）
- **变体**: 无

#### 1. request 差异发现

| 类别 | 差异 | 例子 | 证据等级 | 来源 |
|------|------|------|---------|------|
| 参数命名 | 未查到公开文档 ⚠️ | - | - | - |
| 能力支持 | 未查到公开文档 ⚠️ | - | - | - |
| 思考机制 | 未查到公开文档 ⚠️ | - | - | - |
| 流式/usage | 未查到公开文档 ⚠️ | - | - | - |
| 消息格式 | 未查到公开文档 ⚠️ | - | - | - |
| 特殊字段 | 未查到公开文档 ⚠️ | - | - | - |
| headers/认证 | 未查到公开文档 ⚠️ | - | - | - |
| URL/端点 | 无差异(按 registry 声明,`{base_url}/chat/completions`) | - | - | - |
| 模型 ID | 未查到公开文档 ⚠️ | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖(按 full() 声明);⚠️ 证据不足
- **aimux 代码位置**: `openai_compat_registry.rs:430-438`
- **差距说明**: 无法确认任何独立配置。
- **建议动作**: 暂无动作。

#### 3. 证据与验证

- **证据等级**: 无
- **验证状态**: 🔲 未验证
- **存疑标记**: ⚠️ 证据不足

---

### cline_pass — Cline

- **registry 现状**: profile=`full()` · base_url=`https://api.cline.bot/v1` · env=`CLINE_API_KEY`（[openai_compat_registry.rs:439-447](../../aimux-providers/src/openai_compat_registry.rs#L439)）
- **变体**: 无

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异(OpenAI chat completions) | `{"model":"anthropic/claude-sonnet-4-6","messages":[...]}` | C | [docs.cline.bot — API Overview](https://docs.cline.bot/api/overview) |
| 能力支持 | 支持流式与 tool calling | - | C | 同上 |
| 思考机制 | 文档提及模型有 reasoning 支持选项;未发现独立请求字段 ⚠️ | - | C | [docs.cline.bot — API Overview](https://docs.cline.bot/api/overview) |
| 流式/usage | 无差异(SSE) | - | C | 同上 |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无 | - | - | - |
| headers/认证 | `Authorization: Bearer {CLINE_API_KEY}`(官方示例直接 Bearer) | `curl -X POST https://api.cline.bot/api/v1/chat/completions -H "Authorization: Bearer YOUR_API_KEY"` | C | [docs.cline.bot — API Overview](https://docs.cline.bot/api/overview) |
| URL/端点 | ⚠️ 官方文档示例路径为 `https://api.cline.bot/api/v1/chat/completions`(**含 /api 段**),registry 为 `https://api.cline.bot/v1`;`/v1` 是否同样可用未证实 | `POST https://api.cline.bot/api/v1/chat/completions` | C | 同上 |
| 模型 ID | **`provider/model` 前缀约定**,如 `anthropic/claude-sonnet-4-6`、`openai/...` | `"model":"anthropic/claude-sonnet-4-6"` | C | 同上 |

#### 2. aimux 现状对比

- **对比结论**: 🔶 部分覆盖(认证/格式兼容;base_url 路径与模型前缀需确认)
- **aimux 代码位置**: `openai_compat_registry.rs:439-447`(base_url 无 `/api` 段)
- **差距说明**: ① registry base_url 与官方文档(`/api/v1`)不一致,若官方仅接受 `/api/v1` 前缀则请求会 404 ⚠️;② 模型 ID 为 `provider/model` 形式,aimux 原样透传可工作。
- **建议动作**: 实测 `/v1` 与 `/api/v1` 路径;若 `/v1` 不可用则修正 registry base_url。

#### 3. 证据与验证

- **证据等级**: C
- **验证状态**: 🔲 未验证(仅文档引用)
- **存疑标记**: base_url 路径 ⚠️ 存疑

---

### closeai — CloseAI

- **registry 现状**: profile=`full()` · base_url=`https://api.closeai-proxy.xyz/v1` · env=`CLOSEAI_API_KEY`（[openai_compat_registry.rs:448-456](../../aimux-providers/src/openai_compat_registry.rs#L448)）
- **变体**: 无

#### 1. request 差异发现

| 类别 | 差异 | 例子 | 证据等级 | 来源 |
|------|------|------|---------|------|
| 参数命名 | 未查到公开文档 ⚠️ | - | - | - |
| 能力支持 | 未查到公开文档 ⚠️ | - | - | - |
| 思考机制 | 未查到公开文档 ⚠️ | - | - | - |
| 流式/usage | 未查到公开文档 ⚠️ | - | - | - |
| 消息格式 | 未查到公开文档 ⚠️ | - | - | - |
| 特殊字段 | 未查到公开文档 ⚠️ | - | - | - |
| headers/认证 | 未查到公开文档 ⚠️ | - | - | - |
| URL/端点 | 无差异(按 registry 声明) | - | - | - |
| 模型 ID | 未查到公开文档 ⚠️ | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖(按 full() 声明);⚠️ 证据不足
- **aimux 代码位置**: `openai_compat_registry.rs:448-456`
- **差距说明**: 代理型服务,无独立文档。
- **建议动作**: 暂无动作。

#### 3. 证据与验证

- **证据等级**: 无
- **验证状态**: 🔲 未验证
- **存疑标记**: ⚠️ 证据不足

---

### cloudferro_sherlock — CloudFerro Sherlock

- **registry 现状**: profile=`full()` · base_url=`https://api-sherlock.cloudferro.com/openai/v1` · env=`CLOUDFERRO_SHERLOCK_API_KEY`（[openai_compat_registry.rs:457-465](../../aimux-providers/src/openai_compat_registry.rs#L457)）
- **变体**: 无

#### 1. request 差异发现

| 类别 | 差异 | 例子 | 证据等级 | 来源 |
|------|------|------|---------|------|
| 参数命名 | 未查到公开文档 ⚠️ | - | - | - |
| 能力支持 | 未查到公开文档 ⚠️ | - | - | - |
| 思考机制 | 未查到公开文档 ⚠️ | - | - | - |
| 流式/usage | 未查到公开文档 ⚠️ | - | - | - |
| 消息格式 | 未查到公开文档 ⚠️ | - | - | - |
| 特殊字段 | 未查到公开文档 ⚠️ | - | - | - |
| headers/认证 | 未查到公开文档 ⚠️ | - | - | - |
| URL/端点 | 无差异(按 registry 声明) | - | - | - |
| 模型 ID | 未查到公开文档 ⚠️ | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖(按 full() 声明);⚠️ 证据不足
- **aimux 代码位置**: `openai_compat_registry.rs:457-465`
- **差距说明**: CloudFerro(波兰云)的 Sherlock AI 平台,未找到请求格式文档。
- **建议动作**: 暂无动作。

#### 3. 证据与验证

- **证据等级**: 无
- **验证状态**: 🔲 未验证
- **存疑标记**: ⚠️ 证据不足

---

### cloudflare — Cloudflare (Workers AI OpenAI 兼容端点)

- **registry 现状**: profile=`full()` · base_url=`https://api.cloudflare.com/client/v4/accounts/{CLOUDFLARE_ACCOUNT_ID}/ai/v1` · env=`CLOUDFLARE_API_KEY`（[openai_compat_registry.rs:466-474](../../aimux-providers/src/openai_compat_registry.rs#L466)）
- **变体**: `cloudflare_workers_ai`(同 base_url/env,见下一条)

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异(OpenAI chat completions) | `{"model":"@cf/meta/llama-3.1-8b-instruct","messages":[{"role":"user","content":"..."}]}` | C | [developers.cloudflare.com — OpenAI compatible endpoints](https://developers.cloudflare.com/workers-ai/configuration/open-ai-compatibility/) |
| 能力支持 | 大部分文本模型仅支持 messages/temperature 等基础参数;tools/response_format 按模型而异(无统一声明) ⚠️ | - | C | 同上 |
| 思考机制 | 无(开放权重模型为主) | - | - | - |
| 流式/usage | 无差异(SSE) | - | C | 同上 |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无 | - | - | - |
| headers/认证 | `Authorization: Bearer {API_TOKEN}`(Cloudflare API token,非 Key) | `--header "Authorization: Bearer {api_token}"` | C | 同上 |
| URL/端点 | 无差异;base_url 含 **account_id 占位符**,需替换为真实 `{CLOUDFLARE_ACCOUNT_ID}`;还提供 `/responses`(如 gpt-oss-120b) | `POST https://api.cloudflare.com/client/v4/accounts/{account_id}/ai/v1/chat/completions` | C | 同上 |
| 模型 ID | **前缀约定**:`@cf/...`、`@hf/...`、`@openai/...` 等厂商前缀 | `"model":"@cf/meta/llama-3.1-8b-instruct"` | C | 同上 |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖(请求体/认证/URL 拼接均兼容)
- **aimux 代码位置**: `openai_compat_registry.rs:466-474`
- **差距说明**: base_url 中的 `{CLOUDFLARE_ACCOUNT_ID}` 占位符需用户在配置时替换;模型前缀 `@cf/` 由用户传入。
- **建议动作**: 补测试即可;可考虑文档提示 account_id 替换。

#### 3. 证据与验证

- **证据等级**: C
- **验证状态**: 🔲 未验证(仅文档引用)
- **存疑标记**: 无

---

### cloudflare_workers_ai — Cloudflare Workers AI

- **registry 现状**: profile=`full()` · base_url=`https://api.cloudflare.com/client/v4/accounts/{CLOUDFLARE_ACCOUNT_ID}/ai/v1` · env=`CLOUDFLARE_API_KEY`（[openai_compat_registry.rs:475-483](../../aimux-providers/src/openai_compat_registry.rs#L475)）
- **变体**: 与 `cloudflare` 为同一端点的重复声明(display 不同、base_url/env 完全相同)

#### 1. request 差异发现

| 类别 | 差异 | 例子 | 证据等级 | 来源 |
|------|------|------|---------|------|
| 参数命名 | 与 cloudflare 条目相同(无差异) | - | C | [developers.cloudflare.com — OpenAI compatible endpoints](https://developers.cloudflare.com/workers-ai/configuration/open-ai-compatibility/) |
| 能力支持 | 与 cloudflare 条目相同 | - | C | 同上 |
| 思考机制 | 无 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无 | - | - | - |
| headers/认证 | Bearer API token,同 cloudflare | - | C | 同上 |
| URL/端点 | 与 `cloudflare` 完全相同(account_id 占位符) | - | C | 同上 |
| 模型 ID | `@cf/` 前缀,同 cloudflare | - | C | 同上 |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖(与 cloudflare 相同);建议确认是否保留重复声明
- **aimux 代码位置**: `openai_compat_registry.rs:475-483`
- **差距说明**: 与 `cloudflare` 条目完全重复(base_url/env_var 一致),属注册表冗余。
- **建议动作**: 可考虑合并声明(保留 display 别名),非 request 层问题。

#### 3. 证据与验证

- **证据等级**: C
- **验证状态**: 🔲 未验证
- **存疑标记**: 无

---

### codestral — Codestral (Mistral)

- **registry 现状**: profile=`full()` · base_url=`https://api.mistral.ai/v1` · env=`CODESTRAL_API_KEY`（[openai_compat_registry.rs:484-492](../../aimux-providers/src/openai_compat_registry.rs#L484)）
- **变体**: 无（Mistral 平台模型;Codestral 官方独立端点见下）

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | **`max_tokens`**(官方无 `max_completion_tokens`);**`random_seed`** 而非 OpenAI `seed`;**`safe_prompt`** 布尔参数 | `{"model":"codestral-latest","messages":[...],"max_tokens":1024,"random_seed":42}` | C | [docs.mistral.ai — Chat Endpoints](https://docs.mistral.ai/api/endpoint/chat) |
| 能力支持 | 官方参数表**未列出 `top_k`** ⚠️;支持 tools(含 WebSearchTool/CodeInterpreterTool 等类型)、response_format(json_object/json_schema)、prediction、metadata、prompt_cache_key、guardrails | tools 可为 `WebSearchTool` 等非 function 类型 | C | 同上 |
| 思考机制 | `reasoning_effort`: `none/minimal/low/medium/high/xhigh` | `"reasoning_effort":"high"` | C | 同上 |
| 流式/usage | SSE `data: [DONE]` 结束;usage 在响应顶层;`stream_options` 未在官方参数表 ⚠️ | - | C | 同上 |
| 消息格式 | 无差异(roles: system/user/assistant/tool) | - | C | 同上 |
| 特殊字段 | `random_seed`、`safe_prompt`、`guardrails`、`prompt_cache_key`、`prompt_mode`("reasoning") | `"safe_prompt":false` | C | 同上 |
| headers/认证 | `Authorization: Bearer {api_key}` | - | C | 同上 |
| URL/端点 | ⚠️ registry 用 `https://api.mistral.ai/v1`;Codestral 官方专属端点为 `https://api.codestral.ai/v1`(API key 也分 Codestral key) | `https://api.codestral.ai/v1/chat/completions` | C | [mistral.ai/news/codestral](https://mistral.ai/news/codestral/) |
| 模型 ID | `codestral-latest`、`codestral-2505` 等;或 Mistral 平台模型名 | `"model":"codestral-latest"` | C | 同上 |

#### 2. aimux 现状对比

- **对比结论**: ⚠️ 不一致/🔶 部分覆盖
- **aimux 代码位置**: `convert.rs:1111-1137`(top_k 发送、max_tokens/max_completion_tokens)、`convert.rs:1211-1213`(`seed` 字段)、`convert.rs:1103-1108`(stream_options)
- **差距说明**: ① aimux full() 会发送 `top_k`(Mistral 参数表无此字段 ⚠️);② aimux 发送 `seed`,Mistral 需要 `random_seed`;③ 推理模型分支会发 `max_completion_tokens`(Mistral 为 `max_tokens`);④ `stream_options.include_usage` 官方未文档化 ⚠️;⑤ registry base_url 指向 Mistral 平台而非 Codestral 专属端点。
- **建议动作**: codestral 需独立 profile 或 bodyOverrides(seed→random_seed);确认 base_url 与 `top_k`/`stream_options` 兼容性。

#### 3. 证据与验证

- **证据等级**: C
- **验证状态**: 🔲 未验证(仅文档引用)
- **存疑标记**: top_k / stream_options / base_url ⚠️ 需实测

---

### cometapi — CometAPI

- **registry 现状**: profile=`full()` · base_url=`https://api.cometapi.com/v1` · env=`COMETAPI_API_KEY`（[openai_compat_registry.rs:493-501](../../aimux-providers/src/openai_compat_registry.rs#L493)）
- **变体**: 无

#### 1. request 差异发现

| 类别 | 差异 | 例子 | 证据等级 | 来源 |
|------|------|------|---------|------|
| 参数命名 | 无差异(OpenAI chat completions + Responses API) | - | C | [apidoc.cometapi.com](https://apidoc.cometapi.com/) |
| 能力支持 | 无差异(聚合 GPT/Claude/Gemini 等 500+ 模型) | - | C | [cometapi.com](https://www.cometapi.com/) |
| 思考机制 | 未查到独立文档 ⚠️ | - | - | - |
| 流式/usage | 未查到独立文档 ⚠️ | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 未查到独立文档 ⚠️ | - | - | - |
| headers/认证 | 无差异(Bearer key,按 OpenAI 兼容声明) | - | C | [cometapi.com — How to Use AI API](https://www.cometapi.com/how-to-use-ai-api-via-cometapi/) |
| URL/端点 | 无差异(`{base_url}/chat/completions`) | - | C | [apidoc.cometapi.com](https://apidoc.cometapi.com/) |
| 模型 ID | 无差异(各家模型短名透传) ⚠️ | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖(按 full() 声明);⚠️ 部分细节未验证
- **aimux 代码位置**: `openai_compat_registry.rs:493-501`
- **差距说明**: 聚合代理,官方文档未披露请求体特化;默认兼容声明合理。
- **建议动作**: 暂无动作。

#### 3. 证据与验证

- **证据等级**: C
- **验证状态**: 🔲 未验证
- **存疑标记**: ⚠️ 思考机制/特殊字段证据不足

---

### commandcode — CommandCode

- **registry 现状**: profile=`full()` · base_url=`https://api.commandcode.com/v1` · env=`COMMANDCODE_API_KEY`（[openai_compat_registry.rs:502-510](../../aimux-providers/src/openai_compat_registry.rs#L502)）
- **变体**: 无

#### 1. request 差异发现

| 类别 | 差异 | 例子 | 证据等级 | 来源 |
|------|------|------|---------|------|
| 参数命名 | 未查到公开文档 ⚠️ | - | - | - |
| 能力支持 | 未查到公开文档 ⚠️ | - | - | - |
| 思考机制 | 未查到公开文档 ⚠️ | - | - | - |
| 流式/usage | 未查到公开文档 ⚠️ | - | - | - |
| 消息格式 | 未查到公开文档 ⚠️ | - | - | - |
| 特殊字段 | 未查到公开文档 ⚠️ | - | - | - |
| headers/认证 | 未查到公开文档 ⚠️ | - | - | - |
| URL/端点 | 无差异(按 registry 声明) | - | - | - |
| 模型 ID | 未查到公开文档 ⚠️ | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖(按 full() 声明);⚠️ 证据不足
- **aimux 代码位置**: `openai_compat_registry.rs:502-510`
- **差距说明**: 无法确认任何独立配置。
- **建议动作**: 暂无动作。

#### 3. 证据与验证

- **证据等级**: 无
- **验证状态**: 🔲 未验证
- **存疑标记**: ⚠️ 证据不足

---

### compactifai — CompactifAI

- **registry 现状**: profile=`full()` · base_url=`https://api.compactif.ai/v1` · env=`COMPACTIFAI_API_KEY`（[openai_compat_registry.rs:511-519](../../aimux-providers/src/openai_compat_registry.rs#L511)）
- **变体**: 无

#### 1. request 差异发现

| 类别 | 差异 | 例子 | 证据等级 | 来源 |
|------|------|------|---------|------|
| 参数命名 | 未查到公开文档 ⚠️ | - | - | - |
| 能力支持 | 未查到公开文档 ⚠️ | - | - | - |
| 思考机制 | 未查到公开文档 ⚠️ | - | - | - |
| 流式/usage | 未查到公开文档 ⚠️ | - | - | - |
| 消息格式 | 未查到公开文档 ⚠️ | - | - | - |
| 特殊字段 | 未查到公开文档 ⚠️ | - | - | - |
| headers/认证 | 未查到公开文档 ⚠️ | - | - | - |
| URL/端点 | 无差异(按 registry 声明) | - | - | - |
| 模型 ID | 未查到公开文档 ⚠️ | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖(按 full() 声明);⚠️ 证据不足
- **aimux 代码位置**: `openai_compat_registry.rs:511-519`
- **差距说明**: 无法确认任何独立配置。
- **建议动作**: 暂无动作。

#### 3. 证据与验证

- **证据等级**: 无
- **验证状态**: 🔲 未验证
- **存疑标记**: ⚠️ 证据不足

---

### copilot — GitHub Copilot

- **registry 现状**: profile=`full()` · base_url=`https://api.githubcopilot.com` · env=`COPILOT_API_KEY`（[openai_compat_registry.rs:520-528](../../aimux-providers/src/openai_compat_registry.rs#L520)）
- **变体**: 无（codex 系模型走同一端点 `/responses`）

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异(OpenAI chat completions;codex 模型用 Responses API) | `POST https://api.githubcopilot.com/chat/completions` | C | [docs.litellm.ai — GitHub Copilot](https://docs.litellm.ai/docs/providers/github_copilot) |
| 能力支持 | 支持 chat completions、embeddings、responses(gpt-5.1-codex 仅 responses) | - | C | 同上 |
| 思考机制 | gpt-5.1-codex 等推理模型经 Responses API(`reasoning` 配置);chat 模型无特殊字段 | `model="github_copilot/gpt-5.1-codex"` + responses 调用 | C | 同上 |
| 流式/usage | 无差异(OpenAI SSE 流式) | `stream=True` | C | 同上 |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无 | - | - | - |
| headers/认证 | **必须模拟 VSCode 客户端头**:`editor-version`(如 `vscode/1.85.1`)、`editor-plugin-version`、`Copilot-Integration-Id`(如 `vscode-chat`)、`user-agent`;认证为 `Authorization: Bearer {GitHub Copilot token}`(OAuth device flow 获取,非普通 API key) | `extra_headers = {"editor-version":"vscode/1.85.1","editor-plugin-version":"copilot/1.155.0","Copilot-Integration-Id":"vscode-chat","user-agent":"GithubCopilot/1.155.0"}` | C | [docs.litellm.ai — GitHub Copilot](https://docs.litellm.ai/docs/providers/github_copilot) / [litellm issue #6564](https://github.com/BerriAI/litellm/issues/6564) |
| URL/端点 | 无差异(`{base_url}/chat/completions` 即 `https://api.githubcopilot.com/chat/completions`);codex 模型为 `{base_url}/responses` | - | C | 同上 |
| 模型 ID | `gpt-4`、`gpt-4o`、`gpt-5.1-codex` 等 OpenAI 模型名透传 | `"model":"gpt-4"` | C | 同上 |

#### 2. aimux 现状对比

- **对比结论**: 🔶 部分覆盖（URL/请求体兼容;认证与必需 headers 未覆盖）
- **aimux 代码位置**: `openai_compat_registry.rs:520-528`;认证固定 `Authorization: Bearer {api_key}`(`model.rs`);`config.headers` 可注入额外头
- **差距说明**: ① 无 `editor-version`/`Copilot-Integration-Id` 等必需头,官方 API 会拒绝(401/403);② token 为 GitHub Copilot OAuth token(device flow),与 env `COPILOT_API_KEY` 语义不同;③ codex 模型需走 responses 模型。
- **建议动作**: 为 copilot 提供默认 headers(editor-version 等)的 profile 扩展或配置模板;文档说明 token 获取方式。

#### 3. 证据与验证

- **证据等级**: C
- **验证状态**: 🔲 未验证(仅文档引用)
- **存疑标记**: 无

---

### cortecs — Cortecs

- **registry 现状**: profile=`full()` · base_url=`https://api.cortecs.ai/v1/` · env=`CORTECS_API_KEY`（[openai_compat_registry.rs:529-537](../../aimux-providers/src/openai_compat_registry.rs#L529)）
- **变体**: 无

#### 1. request 差异发现

| 类别 | 差异 | 例子 | 证据等级 | 来源 |
|------|------|------|---------|------|
| 参数命名 | 未查到公开文档 ⚠️ | - | - | - |
| 能力支持 | 未查到公开文档 ⚠️ | - | - | - |
| 思考机制 | 未查到公开文档 ⚠️ | - | - | - |
| 流式/usage | 未查到公开文档 ⚠️ | - | - | - |
| 消息格式 | 未查到公开文档 ⚠️ | - | - | - |
| 特殊字段 | 未查到公开文档 ⚠️ | - | - | - |
| headers/认证 | 未查到公开文档 ⚠️ | - | - | - |
| URL/端点 | 无差异(按 registry 声明;base_url 带尾斜杠,aimux `without_trailing_slash` 会处理) | - | - | - |
| 模型 ID | 未查到公开文档 ⚠️ | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖(按 full() 声明);⚠️ 证据不足
- **aimux 代码位置**: `openai_compat_registry.rs:529-537`
- **差距说明**: 无法确认任何独立配置;尾斜杠由 `without_trailing_slash` 归一化,无问题。
- **建议动作**: 暂无动作。

#### 3. 证据与验证

- **证据等级**: 无
- **验证状态**: 🔲 未验证
- **存疑标记**: ⚠️ 证据不足

---

### coze — Coze (扣子)

- **registry 现状**: profile=`full()` · base_url=`https://api.coze.cn/v1` · env=`COZE_API_KEY`（[openai_compat_registry.rs:538-546](../../aimux-providers/src/openai_compat_registry.rs#L538)）
- **变体**: 无（国际站 api.coze.com 同理）

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | **原生 API 为 Coze 自有格式**(`/v3/chat`: `bot_id`、`user_id`、`messages[{role,content,content_type}]`、`stream`、`auto_save_history`),**不是** OpenAI `messages/model` | `POST https://api.coze.cn/v3/chat` body: `{"bot_id":"xxx","user_id":"u","stream":false,"messages":[{"role":"user","content":"hi","content_type":"text"}]}` | B | [reference/simple-one-api/docs/coze.cn申请API使用流程.md](../../reference/simple-one-api/docs/coze.cn申请API使用流程.md) |
| 能力支持 | 原生为 bot 对话 + workflow 调用,非通用 LLM 参数;无 OpenAI 式 tools/response_format 语义 | - | C | [coze2openai README](https://github.com/fatwang2/coze2openai) |
| 思考机制 | 无统一机制(依赖 bot 配置) | - | - | - |
| 流式/usage | 原生 SSE 事件格式(`message`/`message_delta` 等),非 OpenAI chunk 格式 | - | C | [coze2openai README](https://github.com/fatwang2/coze2openai) |
| 消息格式 | `content_type` 字段(text/image/audio/file 等)替代 OpenAI content 数组 | `{"role":"user","content":"...","content_type":"text"}` | C | 同上 |
| 特殊字段 | `bot_id`、`user_id`、`auto_save_history` 等 | `"bot_id":"739..."` | C | 同上 |
| headers/认证 | `Authorization: Bearer {PAT}` | - | C | [coze-studio wiki — API 参考](https://github.com/coze-dev/coze-studio/wiki/6.-API-%E5%8F%82%E8%80%83) |
| URL/端点 | 原生端点为 `https://api.coze.cn/v3/chat`(及 /v1/chat);未发现官方 OpenAI 兼容 chat/completions 端点 ⚠️ | `POST https://api.coze.cn/v3/chat` | C | [coze2openai README](https://github.com/fatwang2/coze2openai) / [zhihu 文章](https://zhuanlan.zhihu.com/p/707567256) |
| 模型 ID | 以 bot_id 标识(URL 中 bot 参数后数字),无模型名概念 | `"model":"bot-73428668341****"`(coze2openai 用 bot 名当 model) | C | [coze2openai README](https://github.com/fatwang2/coze2openai) |

#### 2. aimux 现状对比

- **对比结论**: ❌ 未覆盖(registry 声明与 Coze 原生 API 不符)
- **aimux 代码位置**: `openai_compat_registry.rs:538-546`
- **差距说明**: registry 把 coze 声明为 OpenAI 兼容 full(),aimux 将请求 `https://api.coze.cn/v1/chat/completions`(OpenAI 请求体);而 Coze 原生协议是 `/v1/chat`、`/v3/chat`(bot_id 等),两者不兼容;社区存在 coze2openai/simple-one-api 中转佐证原生不兼容。⚠️ 未排除 2026 年 Coze 新增官方 OpenAI 兼容端点(未检索到)。
- **建议动作**: 需要 Coze 专用适配(bot_id 映射)或确认官方 OpenAI 兼容端点后再声明;在此之前标注为不兼容。

#### 3. 证据与验证

- **证据等级**: B + C
- **验证状态**: 🔲 未验证
- **存疑标记**: ⚠️ 是否新增官方 OpenAI 兼容端点未证实

---

### crof — CrofAI

- **registry 现状**: profile=`full()` · base_url=`https://crof.ai/v1` · env=`CROF_API_KEY`（[openai_compat_registry.rs:547-555](../../aimux-providers/src/openai_compat_registry.rs#L547)）
- **变体**: 无

#### 1. request 差异发现

| 类别 | 差异 | 例子 | 证据等级 | 来源 |
|------|------|------|---------|------|
| 参数命名 | 未查到公开文档 ⚠️ | - | - | - |
| 能力支持 | 未查到公开文档 ⚠️ | - | - | - |
| 思考机制 | 未查到公开文档 ⚠️ | - | - | - |
| 流式/usage | 未查到公开文档 ⚠️ | - | - | - |
| 消息格式 | 未查到公开文档 ⚠️ | - | - | - |
| 特殊字段 | 未查到公开文档 ⚠️ | - | - | - |
| headers/认证 | 未查到公开文档 ⚠️ | - | - | - |
| URL/端点 | 无差异(按 registry 声明) | - | - | - |
| 模型 ID | 未查到公开文档 ⚠️ | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖(按 full() 声明);⚠️ 证据不足
- **aimux 代码位置**: `openai_compat_registry.rs:547-555`
- **差距说明**: 无法确认任何独立配置。
- **建议动作**: 暂无动作。

#### 3. 证据与验证

- **证据等级**: 无
- **验证状态**: 🔲 未验证
- **存疑标记**: ⚠️ 证据不足

---

### crossmodel — CrossModel

- **registry 现状**: profile=`full()` · base_url=`https://api.crossmodel.ai/v1` · env=`CROSSMODEL_API_KEY`（[openai_compat_registry.rs:556-564](../../aimux-providers/src/openai_compat_registry.rs#L556)）
- **变体**: 无

#### 1. request 差异发现

| 类别 | 差异 | 例子 | 证据等级 | 来源 |
|------|------|------|---------|------|
| 参数命名 | 未查到公开文档 ⚠️ | - | - | - |
| 能力支持 | 未查到公开文档 ⚠️ | - | - | - |
| 思考机制 | 未查到公开文档 ⚠️ | - | - | - |
| 流式/usage | 未查到公开文档 ⚠️ | - | - | - |
| 消息格式 | 未查到公开文档 ⚠️ | - | - | - |
| 特殊字段 | 未查到公开文档 ⚠️ | - | - | - |
| headers/认证 | 未查到公开文档 ⚠️ | - | - | - |
| URL/端点 | 无差异(按 registry 声明) | - | - | - |
| 模型 ID | 未查到公开文档 ⚠️ | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖(按 full() 声明);⚠️ 证据不足
- **aimux 代码位置**: `openai_compat_registry.rs:556-564`
- **差距说明**: 无法确认任何独立配置。
- **建议动作**: 暂无动作。

#### 3. 证据与验证

- **证据等级**: 无
- **验证状态**: 🔲 未验证
- **存疑标记**: ⚠️ 证据不足

---

### crusoe — Crusoe

- **registry 现状**: profile=`full()` · base_url=`https://api.inference.crusoecloud.com/v1` · env=`CRUSOE_API_KEY`（[openai_compat_registry.rs:565-573](../../aimux-providers/src/openai_compat_registry.rs#L565)）
- **变体**: 无

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异(OpenAI chat completions) | `client.chat.completions.create(model="meta-llama/Llama-3.3-70B-Instruct", messages=[...])` | C | [docs.crusoecloud.com — Serverless Inference](https://docs.crusoecloud.com/quickstart/getting-started-with-serverless-inference/) |
| 能力支持 | 无差异(OpenAI 兼容 chat 端点) | - | C | 同上 |
| 思考机制 | 无统一机制(按托管模型) | - | - | - |
| 流式/usage | 无差异(SSE) | - | C | 同上 |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无 | - | - | - |
| headers/认证 | `Authorization: Bearer {API_KEY}` | `api_key=CRUSOE_API_KEY` | C | 同上 |
| URL/端点 | 无差异(`{base_url}/chat/completions`;还支持自托管部署别名) | - | C | [docs.crusoecloud.com — Self-serve deployments](https://docs.crusoecloud.com/self-serve-deployments/quickstart) |
| 模型 ID | 模型名含 `org/model` 斜杠,如 `meta-llama/Llama-3.3-70B-Instruct` | `"model":"meta-llama/Llama-3.3-70B-Instruct"` | C | [docs.crusoecloud.com](https://docs.crusoecloud.com/quickstart/getting-started-with-serverless-inference/) |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖(无差异)
- **aimux 代码位置**: `openai_compat_registry.rs:565-573`
- **差距说明**: 无实质差距;`org/model` 形式模型名透传即可。
- **建议动作**: 补测试即可。

#### 3. 证据与验证

- **证据等级**: C
- **验证状态**: 🔲 未验证(仅文档引用)
- **存疑标记**: 无

---

### daoxe — DaoXE

- **registry 现状**: profile=`full()` · base_url=`https://daoxe.com/v1` · env=`DAOXE_API_KEY`（[openai_compat_registry.rs:574-582](../../aimux-providers/src/openai_compat_registry.rs#L574)）
- **变体**: 无

#### 1. request 差异发现

| 类别 | 差异 | 例子 | 证据等级 | 来源 |
|------|------|------|---------|------|
| 参数命名 | 未查到公开文档 ⚠️ | - | - | - |
| 能力支持 | 未查到公开文档 ⚠️ | - | - | - |
| 思考机制 | 未查到公开文档 ⚠️ | - | - | - |
| 流式/usage | 未查到公开文档 ⚠️ | - | - | - |
| 消息格式 | 未查到公开文档 ⚠️ | - | - | - |
| 特殊字段 | 未查到公开文档 ⚠️ | - | - | - |
| headers/认证 | 未查到公开文档 ⚠️ | - | - | - |
| URL/端点 | 无差异(按 registry 声明) | - | - | - |
| 模型 ID | 未查到公开文档 ⚠️ | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖(按 full() 声明);⚠️ 证据不足
- **aimux 代码位置**: `openai_compat_registry.rs:574-582`
- **差距说明**: 无法确认任何独立配置。
- **建议动作**: 暂无动作。

#### 3. 证据与验证

- **证据等级**: 无
- **验证状态**: 🔲 未验证
- **存疑标记**: ⚠️ 证据不足

---

### darkbloom — Darkbloom

- **registry 现状**: profile=`full()` · base_url=`https://api.darkbloom.dev/v1` · env=`DARKBLOOM_API_KEY`（[openai_compat_registry.rs:583-591](../../aimux-providers/src/openai_compat_registry.rs#L583)）
- **变体**: 无

#### 1. request 差异发现

| 类别 | 差异 | 例子 | 证据等级 | 来源 |
|------|------|------|---------|------|
| 参数命名 | 未查到公开文档 ⚠️ | - | - | - |
| 能力支持 | 未查到公开文档 ⚠️ | - | - | - |
| 思考机制 | 未查到公开文档 ⚠️ | - | - | - |
| 流式/usage | 未查到公开文档 ⚠️ | - | - | - |
| 消息格式 | 未查到公开文档 ⚠️ | - | - | - |
| 特殊字段 | 未查到公开文档 ⚠️ | - | - | - |
| headers/认证 | 未查到公开文档 ⚠️ | - | - | - |
| URL/端点 | 无差异(按 registry 声明) | - | - | - |
| 模型 ID | 未查到公开文档 ⚠️ | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖(按 full() 声明);⚠️ 证据不足
- **aimux 代码位置**: `openai_compat_registry.rs:583-591`
- **差距说明**: 无法确认任何独立配置。
- **建议动作**: 暂无动作。

#### 3. 证据与验证

- **证据等级**: 无
- **验证状态**: 🔲 未验证
- **存疑标记**: ⚠️ 证据不足

---

### databricks — Databricks

- **registry 现状**: profile=`full()` · base_url=`https://databricks.com/serving-endpoints` · env=`DATABRICKS_API_KEY`（[openai_compat_registry.rs:592-600](../../aimux-providers/src/openai_compat_registry.rs#L592)）
- **变体**: 无（Foundation Model 与自建 serving endpoint 共用一套 OpenAI 兼容格式）

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | Chat 用 `max_tokens`(非 max_completion_tokens);Responses API 用 `input`/`max_output_tokens` | `{"model":"databricks-meta-llama-3-3-70b-instruct","messages":[...],"max_tokens":512,"temperature":0.5}` | C | [docs.databricks.com — Foundation model REST API reference](https://docs.databricks.com/aws/en/machine-learning/foundation-model-apis/api-reference) |
| 能力支持 | 支持 `top_k`、tools(仅 function,max 32 个)、response_format(text/json_schema/json_object)、n、stream | 表格: `top_k` "Defines the number of k most likely tokens to use for top-k-filtering" | C | 同上 |
| 思考机制 | 无独立 thinking 字段;reasoning 模型走 Responses API `reasoning: {effort: low/medium/high}` | `"reasoning":{"effort":"medium"}` | C | 同上 |
| 流式/usage | SSE + `stream_options.include_usage`;usage 含 `reasoning_tokens` | `{"stream":true,"stream_options":{"include_usage":true}}` | C | 同上 |
| 消息格式 | 无差异(chat);Responses 用 input 块(input_text/input_image/input_file) | - | C | 同上 |
| 特殊字段 | Responses API 支持 `metadata`(16 对)、`prompt_cache_key`(替代 user)、`prompt_cache_retention`("24h")、`service_tier`(priority/default)、`safety_identifier`、`top_logprobs`、`truncation`;**不支持** `store`/`background`/`conversation`(400) | `{"prompt_cache_key":"k1","prompt_cache_retention":"24h"}` | C | 同上 |
| headers/认证 | `Authorization: Bearer {PAT}`(Databricks personal access token) | - | C | 同上 |
| URL/端点 | **真实端点为 `{workspace-host}/serving-endpoints/{name}/invocations`**(Foundation Model 为 `/serving-endpoints/databricks-{model}/invocations`);registry 的 `https://databricks.com/serving-endpoints` 不是真实 host(占位模板) | `POST https://{workspace-host}/serving-endpoints/databricks-meta-llama-3-3-70b-instruct/invocations` | C | [docs.databricks.com — Query a chat model](https://docs.databricks.com/aws/en/machine-learning/model-serving/query-chat-models) |
| 模型 ID | chat 请求 `model` = endpoint 名或 `databricks-{model}` 预置名 | `"model":"databricks-meta-llama-3-3-70b-instruct"` | C | 同上 |

#### 2. aimux 现状对比

- **对比结论**: ❌ URL/端点未覆盖;特殊字段 🔶 部分覆盖
- **aimux 代码位置**: `openai_compat_registry.rs:592-600`;`model.rs:endpoint()`(拼 `/chat/completions`);`convert.rs:1297-1317`(whitelist 已有 prompt_cache_key/prompt_cache_retention/prompt_cache_options/safety_identifier/metadata/service_tier)
- **差距说明**: ① registry base_url 无法拼接出 `/serving-endpoints/{name}/invocations` 形态,请求路径错误;② `max_tokens` 命名与 aimux 非推理分支一致(✅),推理分支会发 `max_completion_tokens`(Databricks chat 无此字段 ⚠️);③ Responses API 特殊字段大多已在 whitelist(bodyOverrides 可兜底)。
- **建议动作**: registry 需支持 workspace-host + endpoint 名模板;建议 base_url 修正为 `https://{WORKSPACE_HOST}/serving-endpoints` 并文档说明 model=endpoint 名。

#### 3. 证据与验证

- **证据等级**: C
- **验证状态**: 🔲 未验证(仅文档引用)
- **存疑标记**: 无

---

### datarobot — DataRobot

- **registry 现状**: profile=`full()` · base_url=`https://app.datarobot.com/api/v2` · env=`DATAROBOT_API_TOKEN`（[openai_compat_registry.rs:601-609](../../aimux-providers/src/openai_compat_registry.rs#L601)）
- **变体**: 无

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | OpenAI chat completions 参数(`max_tokens` 等) | `client.chat.completions.create(model="datarobot-deployed-llm", messages=[...], max_tokens=512)` | C | [docs.datarobot.com — Bolt-on Governance API](https://docs.datarobot.com/en/docs/agentic-ai/genai-code/genai-chat-completion-api.html) |
| 能力支持 | 支持 streaming;citations 需关联 vector database | - | C | 同上 |
| 思考机制 | 无(依赖底层 LLM) | - | - | - |
| 流式/usage | SSE 流式 | `stream=True` | C | 同上 |
| 消息格式 | 无差异(含 tool 角色) | - | - | - |
| 特殊字段 | **自定义字段**:`llm_id`(透传给 LLM)、`datarobot_association_id`(替换自动关联 ID)、`datarobot_metrics`(自定义指标);moderation 时返回 `datarobot_moderations` | `extra_body = {"llm_id":"azure-gpt-6","datarobot_association_id":"my_id","datarobot_metrics":{"field1":24}}` | C | 同上 |
| headers/认证 | `Authorization: Bearer {DATAROBOT_API_TOKEN}` | `api_key=DATAROBOT_API_TOKEN` | C | 同上 |
| URL/端点 | **真实 base 为 `https://app.datarobot.com/api/v2/deployments/{DEPLOYMENT_ID}`**,registry 只有 `/api/v2`(缺 `/deployments/{id}`) | `OpenAI(base_url=f"https://app.datarobot.com/api/v2/deployments/{DEPLOYMENT_ID}")` | C | 同上 |
| 模型 ID | `model` 固定用占位名 `datarobot-deployed-llm`(部署已绑定) | `"model":"datarobot-deployed-llm"` | C | 同上 |

#### 2. aimux 现状对比

- **对比结论**: ❌ URL/端点未覆盖;特殊字段 ❌(bodyOverrides 可兜底)
- **aimux 代码位置**: `openai_compat_registry.rs:601-609`;`convert.rs` whitelist(无 datarobot_* 字段)
- **差距说明**: ① base_url 缺少 `/deployments/{deployment_id}` 段,aimux 拼接 `/chat/completions` 后端点不成立;② `datarobot_association_id`/`datarobot_metrics`/`llm_id` 不在 whitelist(可通过 `bodyOverrides` 透传);③ `model` 应为固定占位名。
- **建议动作**: 修正 registry base_url 模板(支持 `{DEPLOYMENT_ID}`),文档说明 model 占位名;特殊字段走 bodyOverrides 兜底。

#### 3. 证据与验证

- **证据等级**: C
- **验证状态**: 🔲 未验证(仅文档引用)
- **存疑标记**: 无

---

### deepbricks — DeepBricks

- **registry 现状**: profile=`full()` · base_url=`https://api.deepbricks.ai/v1` · env=`DEEPBRICKS_API_KEY`（[openai_compat_registry.rs:610-618](../../aimux-providers/src/openai_compat_registry.rs#L610)）
- **变体**: 无

#### 1. request 差异发现

| 类别 | 差异 | 例子 | 证据等级 | 来源 |
|------|------|------|---------|------|
| 参数命名 | 无差异(OpenAI 兼容代理) | - | C | [docs.portkey.ai — Deepbricks](https://docs.portkey.ai/docs/integrations/llms/deepbricks) |
| 能力支持 | 无差异(LLM 推理代理) | - | C | [deepbricks.ai](https://deepbricks.ai/) |
| 思考机制 | 未查到独立文档 ⚠️(若托管 deepseek-reasoner 可能有 thinking 透传) | - | - | - |
| 流式/usage | 未查到独立文档 ⚠️ | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 未查到独立文档 ⚠️ | - | - | - |
| headers/认证 | 无差异(Bearer key,按 OpenAI 兼容声明) ⚠️ | - | C | 同上 |
| URL/端点 | 无差异(`{base_url}/chat/completions`) | - | C | 同上 |
| 模型 ID | 无差异(模型名透传) ⚠️ | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖(按 full() 声明);⚠️ 细节未验证
- **aimux 代码位置**: `openai_compat_registry.rs:610-618`
- **差距说明**: 代理型服务;默认兼容声明合理。
- **建议动作**: 暂无动作。

#### 3. 证据与验证

- **证据等级**: C
- **验证状态**: 🔲 未验证
- **存疑标记**: ⚠️ 思考机制/流式细节证据不足

---

### deepinfra — DeepInfra

- **registry 现状**: profile=`full()` · base_url=`https://api.deepinfra.com/v1/openai` · env=`DEEPINFRA_API_KEY`（[openai_compat_registry.rs:619-627](../../aimux-providers/src/openai_compat_registry.rs#L619)）
- **变体**: 无

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异(OpenAI chat completions;`max_tokens` 为主) | `{"model":"meta-llama/Llama-2-70b-chat-hf","messages":[...],"max_tokens":512}` | C | [docs.deepinfra.com — Openai Chat Completions](https://docs.deepinfra.com/api-reference/chat-completions/openai-chat-completions) |
| 能力支持 | 支持 `top_k`、`min_p`、`repetition_penalty`、`logprobs`、`stop_token_ids`、n(1-4)、seed、response_format(json/json_schema/regex) | `{"top_k":40,"min_p":0.05,"repetition_penalty":1.1}` | C | 同上 |
| 思考机制 | **`reasoning: {"enabled": bool}`**(非 DeepSeek 式 thinking);`reasoning_effort` 支持 `none/minimal/low/medium/high/xhigh/max` | `{"reasoning":{"enabled":true},"reasoning_effort":"high"}` | C | 同上 |
| 流式/usage | `stream_options: {include_usage, continuous_usage_stats}` 支持 | `{"stream":true,"stream_options":{"include_usage":true,"continuous_usage_stats":false}}` | C | 同上 |
| 消息格式 | 无差异;tool 消息支持 `cache_control` | `{"role":"tool","tool_call_id":"...","content":"...","cache_control":{}}` | C | 同上 |
| 特殊字段 | `fail_fast`(429 快速失败)、`chat_template_kwargs`、`continue_final_message`、`prompt_cache_key`、`prompt_cache_options({"ttl":"1h"})`、`service_tier`(default/priority/flex) | `{"fail_fast":false,"prompt_cache_key":"k1","prompt_cache_options":{"ttl":"1h"}}` | C | 同上 |
| headers/认证 | `Authorization: Bearer {token}`;**另支持 `xi-api-key` / `x-api-key` header**,及 `x-deepinfra-source`(溯源) | `--header 'x-deepinfra-source: your-source'` | C | 同上 |
| URL/端点 | OpenAI 兼容端点为 `https://api.deepinfra.com/v1/openai`(registry 已含 `/v1/openai`) ✅ | `POST https://api.deepinfra.com/v1/openai/chat/completions`(docs 亦显示 `/v1/chat/completions`) | C | [docs.deepinfra.com — API Reference](https://docs.deepinfra.com/api-reference/introduction) |
| 模型 ID | `org/model` 完整名,如 `meta-llama/Llama-2-70b-chat-hf` | `"model":"meta-llama/Llama-2-70b-chat-hf"` | C | 同上 |

#### 2. aimux 现状对比

- **对比结论**: 🔶 部分覆盖
- **aimux 代码位置**: `convert.rs:1327-1329`(reasoning_effort 白名单发送)、`convert.rs:1098-1108`(top_k/stream_options)、DeepSeek override 仅作用于 deepseek provider
- **差距说明**: ① DeepInfra 思考字段是 `reasoning:{enabled}` 而非 DeepSeek `thinking:{type}`,aimux 无此 override;② `reasoning_effort` 白名单可发且取值兼容(✅);③ `min_p`/`repetition_penalty`/`fail_fast`/`continue_final_message`/`chat_template_kwargs` 不在 whitelist(bodyOverrides 兜底);④ `x-deepinfra-source` header 未发送(经 `config.headers` 可补)。
- **建议动作**: 若需内置,给 deepinfra 增加 reasoning 字段 override(或并入 DeepSeek override 的泛化 `reasoning` 形态);其余 bodyOverrides 兜底。

#### 3. 证据与验证

- **证据等级**: C
- **验证状态**: 🔲 未验证(仅文档引用)
- **存疑标记**: 无

---

### deepseek — DeepSeek

- **registry 现状**: profile=`deepseek()` · base_url=`https://api.deepseek.com/v1` · env=`DEEPSEEK_API_KEY`（[openai_compat_registry.rs:628-636](../../aimux-providers/src/openai_compat_registry.rs#L628)）
- **变体**: 无

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 官方仅 `max_tokens`(无 `max_completion_tokens` ⚠️);**用户标识字段为 `user_id`(非 OpenAI `user`)**;`frequency_penalty`/`presence_penalty` 已废弃(传了也不生效);thinking 模式下 temperature/top_p 无效 | `{"model":"deepseek-v4-pro","messages":[...],"max_tokens":4096,"user_id":"u-123"}` | C | [api-docs.deepseek.com — Create Chat Completion](https://api-docs.deepseek.com/api/create-chat-completion/) |
| 能力支持 | 支持 tools、response_format、logprobs(需设置)、stream、seed;`frequency_penalty`/`presence_penalty` 支持 | - | C | 同上 |
| 思考机制 | **`thinking: {"type":"enabled"/"disabled"}`**(OpenAI 格式,默认 enabled);`reasoning_effort`: `low/high/max`(medium/xhigh 服务端映射为 high;v4-pro 当前仅 high/max) | `{"model":"deepseek-v4-pro","thinking":{"type":"enabled"},"reasoning_effort":"high"}` | C | [api-docs.deepseek.com — Thinking Mode](https://api-docs.deepseek.com/guides/thinking_mode/) / [Chat Completions API](https://api-docs.deepseek.com/api/create-chat-completion/) |
| 流式/usage | SSE;`reasoning_content` 在 delta;`stream_options.include_usage` **官方支持**(在 `data: [DONE]` 前追加 usage chunk,其余 chunk 的 usage 为 null);usage 含 `prompt_cache_hit_tokens`/`reasoning_tokens` | `{"stream":true,"stream_options":{"include_usage":true}}`;response: `"usage":{"completion_tokens":789,"completion_tokens_details":{"reasoning_tokens":415},"prompt_cache_hit_tokens":0}` | A | [aimux-providers/tests/cassettes/deepseek/test_deepseek_model_thinking_part.json](../../aimux-providers/tests/cassettes/deepseek/test_deepseek_model_thinking_part.json) + [Chat Completions API](https://api-docs.deepseek.com/api/create-chat-completion/) |
| 消息格式 | **assistant 消息需回传 `reasoning_content`**(有 tools 时必须回传,否则 400);`reasoning_content` 与 `content` 同级 | `{"role":"assistant","content":"...","reasoning_content":"...","tool_calls":[...]}` | C | [api-docs.deepseek.com — Thinking Mode](https://api-docs.deepseek.com/guides/thinking_mode/) |
| 特殊字段 | 无(thinking/reasoning_effort 即特殊机制) | - | - | - |
| headers/认证 | `Authorization: Bearer {api_key}` | - | C | [api-docs.deepseek.com](https://api-docs.deepseek.com/) |
| URL/端点 | base_url `https://api.deepseek.com`(官方推荐)与 `https://api.deepseek.com/v1`(registry)均可;path `/chat/completions` | `POST https://api.deepseek.com/chat/completions` | A | [aimux-providers/tests/cassettes/deepseek/test_deepseek_model_thinking_part.json](../../aimux-providers/tests/cassettes/deepseek/test_deepseek_model_thinking_part.json)(host: api.deepseek.com, path: /chat/completions) |
| 模型 ID | `deepseek-chat`(V3/V3.2 系)、`deepseek-reasoner`(R1)、`deepseek-v4-pro`/`deepseek-v4-flash` 等 | `"model":"deepseek-reasoner"` | A | 同上 |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖(thinking/reasoning_effort/reasoning_content 回传);⚠️ 两处待验证
- **aimux 代码位置**: `openai/mod.rs:83-91`(deepseek() profile)、`convert.rs:1427-1432 + 1485-1552`(apply_deepseek_override: thinking{type} + reasoning_effort 重映射)、`convert.rs:788-790`(assistant reasoning_content 回传)、`model.rs`(usage 顶层解析,含 reasoning_tokens)
- **差距说明**: ① aimux override 的 thinking 形态(`{"type":"enabled"/"disabled"}`)与官方一致 ✅;reasoning_effort 取值 low/high/max 兼容(aimux 发 `"medium"` 时服务端映射为 high ✅;xhigh→max 与官方映射一致 ✅);② 推理模型分支发送 `max_completion_tokens`,DeepSeek 官方请求体无此字段 ⚠️ 需实测是否被接受/忽略;③ `stream_options.include_usage` 官方已支持,aimux 默认发送 ✅;④ `user` 字段:aimux 发 `user`,DeepSeek 为 `user_id` ⚠️ 命名差异;⑤ thinking 模式不支持 temperature/top_p/penalties,aimux 已对推理模型剥离 temperature/top_p(penalties 始终剥离)✅;⑥ `frequency_penalty`/`presence_penalty` 已废弃(传了不生效,aimux 发送无副作用)。
- **建议动作**: 补 A 级测试覆盖 thinking 开关与 reasoning_effort 映射(已有 cassette 覆盖响应解析);实测 `max_completion_tokens` 与 `user`/`user_id` 兼容性。

#### 3. 证据与验证

- **证据等级**: A + C
- **验证状态**: ✅ 已验证(有 A 级 cassette: [test_deepseek_model_thinking_part.json](../../aimux-providers/tests/cassettes/deepseek/test_deepseek_model_thinking_part.json)、[deepseek_chat_test.rs](../../aimux-providers/tests/deepseek_chat_test.rs)、[deepseek_reasoning_test.rs](../../aimux-providers/tests/deepseek_reasoning_test.rs))
- **存疑标记**: `max_completion_tokens` 兼容性、`user` vs `user_id` ⚠️ 待实测

---

### digitalocean — DigitalOcean

- **registry 现状**: profile=`full()` · base_url=`https://inference.do-ai.run` · env=`DIGITALOCEAN_ACCESS_TOKEN`（[openai_compat_registry.rs:637-645](../../aimux-providers/src/openai_compat_registry.rs#L637)）
- **变体**: 无（Agent Inference 为另一形态,不在本条目）

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异;`max_tokens` 与 `max_completion_tokens` 均列出 | `{"model":"llama3-8b-instruct","messages":[...],"max_completion_tokens":512}` | C | [docs.digitalocean.com — Serverless Inference API](https://docs.digitalocean.com/reference/api/reference/serverless-inference/) |
| 能力支持 | 支持 `top_k`? 参数表未列 top_k ⚠️;支持 logprobs、top_logprobs、seed、n、tools(仅 function)、logit_bias、metadata | 参数表含 `logprobs`/`top_logprobs`/`metadata` | C | 同上 |
| 思考机制 | `reasoning_effort`(枚举)支持;无独立 thinking 字段 | `"reasoning_effort":"high"` | C | 同上 |
| 流式/usage | SSE + `stream_options.include_usage`(标准) | `{"stream":true,"stream_options":{"include_usage":true}}` | C | 同上 |
| 消息格式 | assistant 消息支持 **`reasoning_content` 回传** | `{"role":"assistant","content":"...","reasoning_content":"..."}` | C | 同上 |
| 特殊字段 | `metadata`(16 对 key-value)、n | - | C | 同上 |
| headers/认证 | `Authorization: Bearer {DO token}`(dop_v1_* 等) | `curl -H "Authorization: Bearer $DIGITALOCEAN_TOKEN"` | C | 同上 |
| URL/端点 | **官方端点为 `https://inference.do-ai.run/v1/chat/completions`**(含 `/v1`);registry base_url 无 `/v1` → aimux 拼出 `.../chat/completions` ❌ | `POST https://inference.do-ai.run/v1/chat/completions` | C | 同上 |
| 模型 ID | 模型短名如 `llama3-8b-instruct`;路由模式 `router:{name}` | `curl 'https://inference.do-ai.run/v1/chat/completions'` model 加 `router:` 前缀 | C | [DO 概念文档 — Serverless vs Dedicated](https://www.digitalocean.com/community/conceptual-articles/serverless-vs-dedicated-vs-batch-inference) |

#### 2. aimux 现状对比

- **对比结论**: ❌ URL/端点未覆盖(缺 `/v1` 段);其余 ✅
- **aimux 代码位置**: `openai_compat_registry.rs:637-645`;`model.rs:endpoint()`(拼 `/chat/completions`)
- **差距说明**: base_url 应为 `https://inference.do-ai.run/v1`,否则请求路径错误。
- **建议动作**: 修正 registry base_url 为 `https://inference.do-ai.run/v1`;补测试。

#### 3. 证据与验证

- **证据等级**: C
- **验证状态**: 🔲 未验证(仅文档引用)
- **存疑标记**: top_k 是否支持 ⚠️(参数表未列)

---

### dinference — DInference

- **registry 现状**: profile=`full()` · base_url=`https://api.dinference.com/v1` · env=`DINFERENCE_API_KEY`（[openai_compat_registry.rs:646-654](../../aimux-providers/src/openai_compat_registry.rs#L646)）
- **变体**: 无

#### 1. request 差异发现

| 类别 | 差异 | 例子 | 证据等级 | 来源 |
|------|------|------|---------|------|
| 参数命名 | 未查到公开文档 ⚠️ | - | - | - |
| 能力支持 | 未查到公开文档 ⚠️ | - | - | - |
| 思考机制 | 未查到公开文档 ⚠️ | - | - | - |
| 流式/usage | 未查到公开文档 ⚠️ | - | - | - |
| 消息格式 | 未查到公开文档 ⚠️ | - | - | - |
| 特殊字段 | 未查到公开文档 ⚠️ | - | - | - |
| headers/认证 | 未查到公开文档 ⚠️ | - | - | - |
| URL/端点 | 无差异(按 registry 声明) | - | - | - |
| 模型 ID | 未查到公开文档 ⚠️ | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖(按 full() 声明);⚠️ 证据不足
- **aimux 代码位置**: `openai_compat_registry.rs:646-654`
- **差距说明**: 无法确认任何独立配置。
- **建议动作**: 暂无动作。

#### 3. 证据与验证

- **证据等级**: 无
- **验证状态**: 🔲 未验证
- **存疑标记**: ⚠️ 证据不足

---

### doubao — Doubao (Volcengine Ark)

- **registry 现状**: profile=`full()` · base_url=`https://ark.cn-beijing.volces.com/api/v3` · env=`ARK_API_KEY`（[openai_compat_registry.rs:655-663](../../aimux-providers/src/openai_compat_registry.rs#L655)）
- **变体**: `byteplus`(国际站 ark.bytepluses.com,在 batch-01);本条目覆盖方舟全部模型(doubao-seed/doubao-pro/DeepSeek-R1 托管等)

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异(OpenAI chat completions;`max_tokens` 等);也提供 Responses API | `{"model":"doubao-seed-1-6-251015","messages":[...],"max_tokens":2048}` | C | [火山引擎方舟调研(花叔)](https://huasheng.ai/insights/volcengine-ark-api-guide/) / [方舟 Chat API 文档](https://www.volcengine.com/docs/82379/1494384) |
| 能力支持 | 支持 tools、JSON 模式、图片/视频/音频多模态输入、上下文缓存(Context Cache/Store) | - | C | 同上 |
| 思考机制 | **`thinking: {"type":"enabled","budget_tokens":N}`**(深度思考;budget_tokens 控制思考预算);Seed-1.6 系支持思考/非思考/自适应三模式 | `extra_body={"thinking":{"type":"enabled","budget_tokens":32000}}` | C | [huasheng.ai 方舟调研](https://huasheng.ai/insights/volcengine-ark-api-guide/) / [方舟深度思考文档](https://www.volcengine.com/docs/82379/1449737) |
| 流式/usage | SSE;delta 含 `reasoning_content`(深度思考模型);usage 顶层 | - | C | [方舟深度思考文档](https://www.volcengine.com/docs/82379/1449737) |
| 消息格式 | 深度思考模型 assistant 消息带 `reasoning_content`,多轮需回传(同 DeepSeek 约定) ⚠️(未逐字核实) | - | C | 同上 |
| 特殊字段 | `thinking.budget_tokens`、上下文缓存相关 | - | - | - |
| headers/认证 | `Authorization: Bearer {ARK_API_KEY}` | - | B | [reference/simple-one-api/docs/火山方舟大模型接入指南.md](../../reference/simple-one-api/docs/火山方舟大模型接入指南.md) |
| URL/端点 | 无差异(`https://ark.cn-beijing.volces.com/api/v3` + `/chat/completions`) | `server_url":"https://ark.cn-beijing.volces.com/api/v3"` | B | [reference/simple-one-api/docs/火山方舟大模型接入指南.md](../../reference/simple-one-api/docs/火山方舟大模型接入指南.md) |
| 模型 ID | 模型名(doubao-seed-1-6-251015)或**推理接入点 ID `ep-xxxx`** | `"model":"ep-20240612090709-hzjz5"` | B | [reference/simple-one-api/docs/火山方舟大模型接入指南.md](../../reference/simple-one-api/docs/火山方舟大模型接入指南.md) |

#### 2. aimux 现状对比

- **对比结论**: 🔶 部分覆盖(thinking 机制未覆盖,其余兼容)
- **aimux 代码位置**: `openai_compat_registry.rs:655-663`(full());DeepSeek override 只对 deepseek profile 生效
- **差距说明**: ① doubao/方舟的 `thinking:{type,budget_tokens}` 未映射(profile=full() 无 override,用户只能 bodyOverrides);② 若复用 DeepSeek override 会缺 `budget_tokens`;③ `reasoning_content` 回传机制 aimux 已具备(convert.rs:788-790)✅;④ 模型 ID 支持 ep-xxx 透传 ✅。
- **建议动作**: 为方舟/豆包增加 thinking override(可泛化 RequestBodyOverride 支持 budget_tokens);或文档引导 bodyOverrides。

#### 3. 证据与验证

- **证据等级**: B + C
- **验证状态**: 🔲 未验证(官方文档为 JS 渲染,引用二手聚合 + reference 文档)
- **存疑标记**: reasoning_content 多轮回传规则 ⚠️ 未逐字核实

---

### doubleword — Doubleword

- **registry 现状**: profile=`full()` · base_url=`https://api.doubleword.ai/v1` · env=`DOUBLEWORD_API_KEY`（[openai_compat_registry.rs:664-672](../../aimux-providers/src/openai_compat_registry.rs#L664)）
- **变体**: 无

#### 1. request 差异发现

| 类别 | 差异 | 例子 | 证据等级 | 来源 |
|------|------|------|---------|------|
| 参数命名 | 未查到公开文档 ⚠️ | - | - | - |
| 能力支持 | 未查到公开文档 ⚠️ | - | - | - |
| 思考机制 | 未查到公开文档 ⚠️ | - | - | - |
| 流式/usage | 未查到公开文档 ⚠️ | - | - | - |
| 消息格式 | 未查到公开文档 ⚠️ | - | - | - |
| 特殊字段 | 未查到公开文档 ⚠️ | - | - | - |
| headers/认证 | 未查到公开文档 ⚠️ | - | - | - |
| URL/端点 | 无差异(按 registry 声明) | - | - | - |
| 模型 ID | 未查到公开文档 ⚠️ | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖(按 full() 声明);⚠️ 证据不足
- **aimux 代码位置**: `openai_compat_registry.rs:664-672`
- **差距说明**: 无法确认任何独立配置。
- **建议动作**: 暂无动作。

#### 3. 证据与验证

- **证据等级**: 无
- **验证状态**: 🔲 未验证
- **存疑标记**: ⚠️ 证据不足

---

### drun — D.Run (China)

- **registry 现状**: profile=`full()` · base_url=`https://chat.d.run/v1` · env=`DRUN_API_KEY`（[openai_compat_registry.rs:673-681](../../aimux-providers/src/openai_compat_registry.rs#L673)）
- **变体**: 无

#### 1. request 差异发现

| 类别 | 差异 | 例子 | 证据等级 | 来源 |
|------|------|------|---------|------|
| 参数命名 | 未查到公开文档 ⚠️ | - | - | - |
| 能力支持 | 未查到公开文档 ⚠️ | - | - | - |
| 思考机制 | 未查到公开文档 ⚠️ | - | - | - |
| 流式/usage | 未查到公开文档 ⚠️ | - | - | - |
| 消息格式 | 未查到公开文档 ⚠️ | - | - | - |
| 特殊字段 | 未查到公开文档 ⚠️ | - | - | - |
| headers/认证 | 未查到公开文档 ⚠️ | - | - | - |
| URL/端点 | 无差异(按 registry 声明) | - | - | - |
| 模型 ID | 未查到公开文档 ⚠️ | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖(按 full() 声明);⚠️ 证据不足
- **aimux 代码位置**: `openai_compat_registry.rs:673-681`
- **差距说明**: 无法确认任何独立配置。
- **建议动作**: 暂无动作。

#### 3. 证据与验证

- **证据等级**: 无
- **验证状态**: 🔲 未验证
- **存疑标记**: ⚠️ 证据不足

---

### ebcloud — EBCloud

- **registry 现状**: profile=`full()` · base_url=`https://maas-api.ebcloud.com/v1` · env=`EBCLOUD_API_KEY`（[openai_compat_registry.rs:682-690](../../aimux-providers/src/openai_compat_registry.rs#L682)）
- **变体**: 无

#### 1. request 差异发现

| 类别 | 差异 | 例子 | 证据等级 | 来源 |
|------|------|------|---------|------|
| 参数命名 | 未查到公开文档 ⚠️ | - | - | - |
| 能力支持 | 未查到公开文档 ⚠️ | - | - | - |
| 思考机制 | 未查到公开文档 ⚠️ | - | - | - |
| 流式/usage | 未查到公开文档 ⚠️ | - | - | - |
| 消息格式 | 未查到公开文档 ⚠️ | - | - | - |
| 特殊字段 | 未查到公开文档 ⚠️ | - | - | - |
| headers/认证 | 未查到公开文档 ⚠️ | - | - | - |
| URL/端点 | 无差异(按 registry 声明,maas = Model-as-a-Service) | - | - | - |
| 模型 ID | 未查到公开文档 ⚠️ | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖(按 full() 声明);⚠️ 证据不足
- **aimux 代码位置**: `openai_compat_registry.rs:682-690`
- **差距说明**: 无法确认任何独立配置。
- **建议动作**: 暂无动作。

#### 3. 证据与验证

- **证据等级**: 无
- **验证状态**: 🔲 未验证
- **存疑标记**: ⚠️ 证据不足

---

### embercloud — Embercloud

- **registry 现状**: profile=`full()` · base_url=`https://api.embercloud.com/v1` · env=`EMBERCLOUD_API_KEY`（[openai_compat_registry.rs:691-699](../../aimux-providers/src/openai_compat_registry.rs#L691)）
- **变体**: 无

#### 1. request 差异发现

| 类别 | 差异 | 例子 | 证据等级 | 来源 |
|------|------|------|---------|------|
| 参数命名 | 未查到公开文档 ⚠️ | - | - | - |
| 能力支持 | 未查到公开文档 ⚠️ | - | - | - |
| 思考机制 | 未查到公开文档 ⚠️ | - | - | - |
| 流式/usage | 未查到公开文档 ⚠️ | - | - | - |
| 消息格式 | 未查到公开文档 ⚠️ | - | - | - |
| 特殊字段 | 未查到公开文档 ⚠️ | - | - | - |
| headers/认证 | 未查到公开文档 ⚠️ | - | - | - |
| URL/端点 | 无差异(按 registry 声明) | - | - | - |
| 模型 ID | 未查到公开文档 ⚠️ | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖(按 full() 声明);⚠️ 证据不足
- **aimux 代码位置**: `openai_compat_registry.rs:691-699`
- **差距说明**: 无法确认任何独立配置。
- **建议动作**: 暂无动作。

#### 3. 证据与验证

- **证据等级**: 无
- **验证状态**: 🔲 未验证
- **存疑标记**: ⚠️ 证据不足

---

### empiriolabs — EmpirioLabs AI

- **registry 现状**: profile=`full()` · base_url=`https://api.empiriolabs.ai/v1` · env=`EMPIRIOLABS_API_KEY`（[openai_compat_registry.rs:700-708](../../aimux-providers/src/openai_compat_registry.rs#L700)）
- **变体**: 无

#### 1. request 差异发现

| 类别 | 差异 | 例子 | 证据等级 | 来源 |
|------|------|------|---------|------|
| 参数命名 | 未查到公开文档 ⚠️ | - | - | - |
| 能力支持 | 未查到公开文档 ⚠️ | - | - | - |
| 思考机制 | 未查到公开文档 ⚠️ | - | - | - |
| 流式/usage | 未查到公开文档 ⚠️ | - | - | - |
| 消息格式 | 未查到公开文档 ⚠️ | - | - | - |
| 特殊字段 | 未查到公开文档 ⚠️ | - | - | - |
| headers/认证 | 未查到公开文档 ⚠️ | - | - | - |
| URL/端点 | 无差异(按 registry 声明) | - | - | - |
| 模型 ID | 未查到公开文档 ⚠️ | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖(按 full() 声明);⚠️ 证据不足
- **aimux 代码位置**: `openai_compat_registry.rs:700-708`
- **差距说明**: 无法确认任何独立配置。
- **建议动作**: 暂无动作。

#### 3. 证据与验证

- **证据等级**: 无
- **验证状态**: 🔲 未验证
- **存疑标记**: ⚠️ 证据不足

---

### evroc — evroc

- **registry 现状**: profile=`full()` · base_url=`https://models.think.evroc.com/v1` · env=`EVROC_API_KEY`（[openai_compat_registry.rs:709-717](../../aimux-providers/src/openai_compat_registry.rs#L709)）
- **变体**: 无

#### 1. request 差异发现

| 类别 | 差异 | 例子 | 证据等级 | 来源 |
|------|------|------|---------|------|
| 参数命名 | 无差异(OpenAI 兼容 chat completions) | - | C | [docs.evroc.com — Inference API](https://docs.evroc.com/products/think/think.html) |
| 能力支持 | 支持 chat completions、embeddings、audio transcription、模型列表 | - | C | 同上 |
| 思考机制 | 未查到独立文档 ⚠️(托管开源模型为主) | - | - | - |
| 流式/usage | 未查到独立文档 ⚠️ | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 未查到独立文档 ⚠️ | - | - | - |
| headers/认证 | 无差异(Bearer key,按 OpenAI 兼容声明) ⚠️ | - | C | 同上 |
| URL/端点 | 无差异(`{base_url}/chat/completions`) | - | C | 同上 |
| 模型 ID | 无差异(托管模型名) ⚠️ | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖(按 full() 声明);⚠️ 细节未验证
- **aimux 代码位置**: `openai_compat_registry.rs:709-717`
- **差距说明**: evroc Think 平台官方声明 OpenAI 兼容;无特化请求配置发现。
- **建议动作**: 暂无动作。

#### 3. 证据与验证

- **证据等级**: C
- **验证状态**: 🔲 未验证
- **存疑标记**: ⚠️ 流式/认证细节证据不足

---

### fastcrw — FastCRW

- **registry 现状**: profile=`full()` · base_url=`https://fastcrw.com/api/v1` · env=`FASTCRW_API_KEY`（[openai_compat_registry.rs:718-726](../../aimux-providers/src/openai_compat_registry.rs#L718)）
- **变体**: 无

#### 1. request 差异发现

| 类别 | 差异 | 例子 | 证据等级 | 来源 |
|------|------|------|---------|------|
| 参数命名 | 未查到公开文档 ⚠️ | - | - | - |
| 能力支持 | 未查到公开文档 ⚠️ | - | - | - |
| 思考机制 | 未查到公开文档 ⚠️ | - | - | - |
| 流式/usage | 未查到公开文档 ⚠️ | - | - | - |
| 消息格式 | 未查到公开文档 ⚠️ | - | - | - |
| 特殊字段 | 未查到公开文档 ⚠️ | - | - | - |
| headers/认证 | 未查到公开文档 ⚠️ | - | - | - |
| URL/端点 | 无差异(按 registry 声明) | - | - | - |
| 模型 ID | 未查到公开文档 ⚠️ | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖(按 full() 声明);⚠️ 证据不足
- **aimux 代码位置**: `openai_compat_registry.rs:718-726`
- **差距说明**: 无法确认任何独立配置。
- **建议动作**: 暂无动作。

#### 3. 证据与验证

- **证据等级**: 无
- **验证状态**: 🔲 未验证
- **存疑标记**: ⚠️ 证据不足

---

### fastgpt — FastGPT

- **registry 现状**: profile=`full()` · base_url=`https://api.fastgpt.in/v1` · env=`FASTGPT_API_KEY`（[openai_compat_registry.rs:727-735](../../aimux-providers/src/openai_compat_registry.rs#L727)）
- **变体**: 无（FastGPT 开源项目本体提供 OpenAI 兼容 API;api.fastgpt.in 为托管服务）

#### 1. request 差异发现

| 类别 | 差异 | 例子 | 证据等级 | 来源 |
|------|------|------|---------|------|
| 参数命名 | 无差异(FastGPT 开源版提供 OpenAI 兼容 API) | `baseURL http://localhost:50010/api/v1/chat/completions`(自托管) | C | [FastChat/FastGPT 社区帖](https://www.reddit.com/r/OpenWebUI/comments/1jp4jfe/how_to_connect_to_fastgpt_api/) |
| 能力支持 | 无差异(OpenAI 兼容) | - | C | 同上 |
| 思考机制 | 未查到独立文档 ⚠️ | - | - | - |
| 流式/usage | 未查到独立文档 ⚠️ | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 未查到独立文档 ⚠️ | - | - | - |
| headers/认证 | 未查到独立文档 ⚠️ | - | - | - |
| URL/端点 | ⚠️ 自托管路径含 `/api` 段;registry 为 `api.fastgpt.in/v1`,托管端点形态未证实 | - | - | - |
| 模型 ID | 未查到独立文档 ⚠️ | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖(按 full() 声明);⚠️ 证据不足
- **aimux 代码位置**: `openai_compat_registry.rs:727-735`
- **差距说明**: 托管端点细节无法确认。
- **建议动作**: 暂无动作。

#### 3. 证据与验证

- **证据等级**: C(仅社区帖)
- **验证状态**: 🔲 未验证
- **存疑标记**: ⚠️ 证据不足

---

### fastrouter — FastRouter

- **registry 现状**: profile=`full()` · base_url=`https://api.fastrouter.ai/v1` · env=`FASTROUTER_API_KEY`（[openai_compat_registry.rs:736-744](../../aimux-providers/src/openai_compat_registry.rs#L736)）
- **变体**: 无

#### 1. request 差异发现

| 类别 | 差异 | 例子 | 证据等级 | 来源 |
|------|------|------|---------|------|
| 参数命名 | 无差异(OpenAI 兼容) | - | C | [docs.fastrouter.ai](https://docs.fastrouter.ai/function-calling) |
| 能力支持 | 支持 function calling 等 OpenAI 兼容格式 | - | C | 同上 |
| 思考机制 | 未查到独立文档 ⚠️ | - | - | - |
| 流式/usage | 未查到独立文档 ⚠️ | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 未查到独立文档 ⚠️ | - | - | - |
| headers/认证 | 无差异(Bearer key,按 OpenAI 兼容声明) ⚠️ | - | - | - |
| URL/端点 | 无差异(`{base_url}/chat/completions`) | - | C | [docs.fastrouter.ai](https://docs.fastrouter.ai/function-calling) |
| 模型 ID | 无差异(多家模型路由透传) ⚠️ | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖(按 full() 声明);⚠️ 细节未验证
- **aimux 代码位置**: `openai_compat_registry.rs:736-744`
- **差距说明**: 聚合网关,官方文档确认 OpenAI 兼容与 function calling。
- **建议动作**: 暂无动作。

#### 3. 证据与验证

- **证据等级**: C
- **验证状态**: 🔲 未验证
- **存疑标记**: ⚠️ 流式/认证细节证据不足

---

### featherless_ai — Featherless AI

- **registry 现状**: profile=`full()` · base_url=`https://api.featherless.ai/v1` · env=`FEATHERLESS_API_KEY`（[openai_compat_registry.rs:745-753](../../aimux-providers/src/openai_compat_registry.rs#L745)）
- **变体**: 无

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异(OpenAI chat completions) | `{"model":"Qwen/Qwen2.5-7B-Instruct","messages":[{"role":"user","content":"Hello!"}]}` | C | [featherless.ai/docs — Quickstart](https://featherless.ai/docs/quickstart-guide) |
| 能力支持 | 无差异(OpenAI 兼容;/completions 与 /chat/completions 均提供) | - | C | 同上 |
| 思考机制 | 未查到独立文档 ⚠️(按托管模型,如 deepseek-r1 的 reasoning_content 透传) | - | - | - |
| 流式/usage | 未查到独立文档 ⚠️(按 OpenAI 兼容声明为 SSE) | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 未查到独立文档 ⚠️ | - | - | - |
| headers/认证 | `Authorization: Bearer FEATHERLESS_API_KEY` | `"Authorization": "Bearer FEATHERLESS_API_KEY"` | C | 同上 |
| URL/端点 | 无差异(`{base_url}/chat/completions`) | `POST https://api.featherless.ai/v1/chat/completions` | C | 同上 |
| 模型 ID | `org/model` 完整名,如 `Qwen/Qwen2.5-7B-Instruct` | `"model":"Qwen/Qwen2.5-7B-Instruct"` | C | 同上 |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖(无差异)
- **aimux 代码位置**: `openai_compat_registry.rs:745-753`
- **差距说明**: 无实质差距;`org/model` 模型名透传即可。
- **建议动作**: 补测试即可。

#### 3. 证据与验证

- **证据等级**: C
- **验证状态**: 🔲 未验证(仅文档引用)
- **存疑标记**: 无

---

### firepass — Fireworks (Firepass)

- **registry 现状**: profile=`full()` · base_url=`https://api.fireworks.ai/inference/v1锛圤penAI` · env=`FIREWORKS_API_KEY`（[openai_compat_registry.rs:754-762](../../aimux-providers/src/openai_compat_registry.rs#L754)）
- **变体**: 与 `fireworks`(下一条)为同一厂商同一端点;本条目为重复声明(display 名 Firepass,base_url 尾串含乱码 `锛圤penAI`)

#### 1. request 差异发现

| 类别 | 差异 | 例子 | 证据等级 | 来源 |
|------|------|------|---------|------|
| 参数命名 | 与 fireworks 条目相同 | - | C | [docs.fireworks.ai — Post Chat Completions](https://docs.fireworks.ai/api-reference/post-chatcompletions) |
| 能力支持 | 与 fireworks 条目相同 | - | C | 同上 |
| 思考机制 | 与 fireworks 条目相同(`thinking` 字段) | - | C | 同上 |
| 流式/usage | 与 fireworks 条目相同 | - | C | 同上 |
| 消息格式 | 与 fireworks 条目相同 | - | C | 同上 |
| 特殊字段 | 与 fireworks 条目相同 | - | C | 同上 |
| headers/认证 | 与 fireworks 条目相同(Bearer) | - | C | 同上 |
| URL/端点 | ⚠️ **base_url 尾串 `（OpenAI`(全角括号+文本)误入 URL**(原注释 "（OpenAI 兼容）" 被拼接进字符串),aimux 将拼接出无效 URL | `https://api.fireworks.ai/inference/v1（OpenAI/chat/completions` ❌ | A | [openai_compat_registry.rs:754-762](../../aimux-providers/src/openai_compat_registry.rs#L754) |
| 模型 ID | 与 fireworks 条目相同 | - | C | 同上 |

#### 2. aimux 现状对比

- **对比结论**: ⚠️ 不一致(base_url 乱码,请求必然失败)
- **aimux 代码位置**: `openai_compat_registry.rs:758`(base_url)
- **差距说明**: base_url 字符串尾带 `（OpenAI`(全角括号+文本,注释误入),aimux 将拼接出无效 URL;且该条目与 fireworks 完全重复。
- **建议动作**: 修正/删除 firepass 条目(建议保留 fireworks 单条,display 别名可合并)。

#### 3. 证据与验证

- **证据等级**: A(registry 源码可证 base_url 乱码)
- **验证状态**: 🔲 未验证(未实际请求)
- **存疑标记**: 无(乱码为事实)

---

### fireworks — Fireworks

- **registry 现状**: profile=`full()` · base_url=`https://api.fireworks.ai/inference/v1` · env=`FIREWORKS_API_KEY`（[openai_compat_registry.rs:763-771](../../aimux-providers/src/openai_compat_registry.rs#L763)）
- **变体**: `firepass`(上一条,同一端点重复声明)

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异;`max_tokens` 与 `max_completion_tokens` 均支持;`top_k` 支持 | `{"model":"accounts/fireworks/models/llama-v3p3-70b-instruct","messages":[...],"max_completion_tokens":512,"top_k":40}` | C | [docs.fireworks.ai — Post Chat Completions](https://docs.fireworks.ai/api-reference/post-chatcompletions) |
| 能力支持 | 支持 `top_k`、`min_p`、`typical_p`、`repetition_penalty`、`mirostat_target/lr`、logprobs/top_logprobs、seed、parallel_tool_calls、echo、ignore_eos、context_length_exceeded_behavior、logit_bias、n、functions、safe_tokenization | `{"min_p":0.1,"repetition_penalty":1.1,"mirostat_target":5}` | C | 同上 |
| 思考机制 | **`thinking: {"type":"enabled","budget_tokens":N,"keep":"...","budget_end_str":"..."}`**(DeepSeek 系 thinking 增强版);`reasoning_effort` 未出现在官方参数表 ⚠️ | `{"thinking":{"type":"enabled","budget_tokens":1024}}` | C | 同上 |
| 流式/usage | `stream_options: {include_usage, include_internal_content, buffer_tokens, buffer_ms}`;usage 顶层返回(含 prompt_tokens_details.cached_tokens) | `{"stream":true,"stream_options":{"include_usage":true,"buffer_tokens":1}}` | C | 同上 |
| 消息格式 | assistant 消息支持 `reasoning_content` 回传(DeepSeek 系模型) | `{"role":"assistant","content":"...","reasoning_content":"...","tool_calls":[...]}` | C | 同上 |
| 特殊字段 | `prompt_cache_key`、`prompt_cache_isolation_key`、`prediction`、`metadata`、`service_tier`(default)、`speculation`、`raw_output`、`perf_metrics_in_response`、`return_token_ids`、`prompt_token_ids`、`prompt_truncate_len`;会话亲和 header(`x-session-affinity`/`x-multi-turn-session-id`,RL 场景) | `{"prompt_cache_key":"k1","prediction":{"type":"content","content":"..."},"metadata":{}}` | C | 同上 |
| headers/认证 | `Authorization: Bearer {API_KEY}` | - | C | 同上 |
| URL/端点 | 无差异(`{base_url}/chat/completions`) | `POST https://api.fireworks.ai/inference/v1/chat/completions` | C | 同上 |
| 模型 ID | 短名 `accounts/fireworks/models/{model}` 或别名 | `"model":"accounts/fireworks/models/llama-v3p3-70b-instruct"` | C | 同上 |

#### 2. aimux 现状对比

- **对比结论**: 🔶 部分覆盖(thinking 机制未覆盖;reasoning_effort ⚠️)
- **aimux 代码位置**: `openai_compat_registry.rs:763-771`(full());`convert.rs:1327-1329`(reasoning_effort 白名单发送)
- **差距说明**: ① Fireworks 思考机制为 `thinking`(可带 budget_tokens),aimux 的 DeepSeek override 只输出 `{"type":...}` 且仅对 deepseek profile 生效;fireworks 当前 full() 不发 thinking;② aimux 对设置 reasoning 的调用会向 Fireworks 发 `reasoning_effort`,官方参数表未列出 ⚠️;③ `min_p`/`typical_p`/`repetition_penalty`/`mirostat_*`/`prompt_cache_isolation_key` 等不在 whitelist(bodyOverrides 兜底);④ stream_options.include_usage 兼容 ✅。
- **建议动作**: 若内置,把 thinking override 泛化到 fireworks(或新增 profile);`reasoning_effort` 与 Fireworks 的兼容性需实测;A 级测试已存在([fireworks_test.rs](../../aimux-providers/tests/fireworks_test.rs)、cassettes/fireworks/*)。

#### 3. 证据与验证

- **证据等级**: C(有 A 级测试: [fireworks_test.rs](../../aimux-providers/tests/fireworks_test.rs))
- **验证状态**: 🔲 未验证(现有测试覆盖 thin wrapper 基本路径,未覆盖 thinking)
- **存疑标记**: reasoning_effort 兼容性 ⚠️

---

## 附:本批次存疑归档

以下条目因查不到任何公开资料,按"registry OpenAI 兼容声明(full())"给出 ✅ 结论并标记 ⚠️ 证据不足,不参与差异项判定(不占用内置/对比结论):

cherryin、claudinio、closeai、cloudferro_sherlock、commandcode、compactifai、cortecs、crof、crossmodel、daoxe、darkbloom、dinference、doubleword、drun、ebcloud、embercloud、empiriolabs、fastcrw、fastgpt(部分)、fastrouter(部分)

需实测验证的存疑点(带 ⚠️):

- cline_pass: base_url 是否接受 `/v1`(官方文档为 `/api/v1`)
- codestral: `top_k`/`stream_options` 支持、base_url 应指向 api.codestral.ai 还是 api.mistral.ai
- deepseek: 推理模型分支发送 `max_completion_tokens`、`user` vs `user_id` 命名差异(stream_options.include_usage 已确认官方支持 ✅)
- digitalocean: `top_k` 是否支持(官方参数表未列)
- coze: 2026 年是否新增官方 OpenAI 兼容端点
- doubao: 深度思考 `reasoning_content` 多轮回传规则未逐字核实(官方文档 JS 渲染)
- fireworks: `reasoning_effort` 兼容性
