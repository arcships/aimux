# Batch 05 — Model Request Config 调研

> 状态: ✅ 已完成 · 厂商数: 42 · 2026-08-01
> 模板见 [_template.md](_template.md) · 方法论见 [README.md](README.md)
> 证据硬性要求: 每条目必须有例子或证明(A/B/C 级),禁止编造来源。

## 厂商清单

| # | id | display | base_url | env_var | profile |
|---|---|---|---|---|---|
| 1 | opencode | OpenCode Zen | opencode_zen.rs | OPENCODE_API_KEY | OpenAICompatProfile::full() |
| 2 | opencode_go | OpenCode Go | https://api.opencode.dev/v1 | OPENCODE_GO_API_KEY | OpenAICompatProfile::full() |
| 3 | opencode_zen | OpenCode Zen | https://api.opencode.zen/v1 | OPENCODE_ZEN_API_KEY | OpenAICompatProfile::full() |
| 4 | orcarouter | OrcaRouter | https://api.orcarouter.com/v1 | ORCAROUTER_API_KEY | OpenAICompatProfile::full() |
| 5 | ovhcloud | OVHcloud AI | https://oai.endpoints.kepler.ai.cloud.ovh.net/v1 | OVHCLOUD_API_KEY | OpenAICompatProfile::full() |
| 6 | parasail | Parasail | https://api.parasail.io/v1 | PARASAIL_API_KEY | OpenAICompatProfile::full() |
| 7 | perfxcloud | PerfXCloud | https://api.perfxcloud.com/v1 | PERFXCLOUD_API_KEY | OpenAICompatProfile::full() |
| 8 | perplexity | Perplexity | https://api.perplexity.ai | PERPLEXITY_API_KEY | OpenAICompatProfile::full() |
| 9 | perplexity_agent | Perplexity Agent | https://api.perplexity.ai/v1 | PERPLEXITY_API_KEY | OpenAICompatProfile::full() |
| 10 | petals | Petals | https://api.petals.dev/v1 | PETALS_API_KEY | OpenAICompatProfile::full() |
| 11 | pinstripes | Pinstripes | https://api.pinstripes.io/v1 | PINSTRIPES_API_KEY | OpenAICompatProfile::full() |
| 12 | pioneer | Pioneer | https://api.pioneer.ai/v1 | PIONEER_API_KEY | OpenAICompatProfile::full() |
| 13 | poe | Poe | https://api.poe.com/v1 | POE_API_KEY | OpenAICompatProfile::full() |
| 14 | poolside | Poolside | https://inference.poolside.ai/v1 | POOLSIDE_API_KEY | OpenAICompatProfile::full() |
| 15 | portkey | Portkey Gateway | https://api.portkey.ai/v1 | PORTKEY_API_KEY | OpenAICompatProfile::full() |
| 16 | ppinfra | PPInfra（PPIO 派欧云） | https://api.ppio.com/openai | PPIO_API_KEY | OpenAICompatProfile::full() |
| 17 | predibase | Predibase | https://serving.app.predibase.com/v1 | PREDIBASE_API_KEY | OpenAICompatProfile::full() |
| 18 | privatemode_ai | Privatemode AI | http://localhost:8080/v1 | PRIVATEMODE_API_KEY | OpenAICompatProfile::full() |
| 19 | publicai | Publicai | https://platform.publicai.co/v1 | PUBLICAI_API_KEY | OpenAICompatProfile::full() |
| 20 | qihang_ai | QiHang（启航 AI） | https://api.qhaigc.net/v1 | QIHANG_API_KEY | OpenAICompatProfile::full() |
| 21 | qihoo360 | 360 AI | https://api.360.cn/v1 | AI360_API_KEY | OpenAICompatProfile::full() |
| 22 | qiniu_ai | Qiniu AI | https://api.qiniu.com/v1 | QINIU_API_KEY | OpenAICompatProfile::full() |
| 23 | regolo_ai | Regolo AI | https://api.regolo.ai/v1 | REGOLO_API_KEY | OpenAICompatProfile::full() |
| 24 | reka_ai | Reka AI | https://api.reka.ai/v1 | REKA_API_KEY | OpenAICompatProfile::full() |
| 25 | requesty | Requesty | https://api.requesty.ai/v1 | REQUESTY_API_KEY | OpenAICompatProfile::full() |
| 26 | reve | Reve | https://api.reve.ai/v1 | REVE_API_KEY | OpenAICompatProfile::full() |
| 27 | routing_run | routing.run | https://api.routing.run/v1 | ROUTING_RUN_API_KEY | OpenAICompatProfile::full() |
| 28 | sakana | Sakana AI | https://api.sakana.ai/v1 | SAKANA_API_KEY | OpenAICompatProfile::full() |
| 29 | sambanova | SambaNova | https://api.sambanova.ai/v1 | SAMBANOVA_API_KEY | OpenAICompatProfile::full() |
| 30 | sarvam | Sarvam AI | https://api.sarvam.ai/v1 | SARVAM_API_KEY | OpenAICompatProfile::full() |
| 31 | scaleway | Scaleway AI | https://api.scaleway.ai/v1 | SCALEWAY_API_KEY | OpenAICompatProfile::full() |
| 32 | scx_ai | SCX AI | https://api.scx.ai/v1 | SCX_AI_API_KEY | OpenAICompatProfile::full() |
| 33 | siliconflow | SiliconFlow | https://api.siliconflow.cn/v1 | SILICONFLOW_API_KEY | OpenAICompatProfile::full() |
| 34 | snowflake | Snowflake | https://<account-identifier>.snowflakecomputing.com/api/v2/cortex/v1 | SNOWFLAKE_PAT | OpenAICompatProfile::full() |
| 35 | snowflake_cortex | Snowflake Cortex | https://<account-identifier>.snowflakecomputing.com/api/v2/cortex/v1 | SNOWFLAKE_CORTEX_PAT | OpenAICompatProfile::full() |
| 36 | stackit | STACKIT | https://api.openai-compat.model-serving.eu01.onstackit.cloud/v1 | STACKIT_API_KEY | OpenAICompatProfile::full() |
| 37 | stepfun | StepFun (阶跃星辰) | https://api.stepfun.com/v1 | STEPFUN_API_KEY | OpenAICompatProfile::full() |
| 38 | stepfun_ai_step_plan | StepFun Step Plan (Global) | https://api.stepfun.ai/step_plan/v1 | STEPFUN_API_KEY | OpenAICompatProfile::full() |
| 39 | stepfun_step_plan | StepFun Step Plan (China) | https://api.stepfun.com/step_plan/v1 | STEPFUN_API_KEY | OpenAICompatProfile::full() |
| 40 | subconscious | Subconscious | https://api.subconscious.dev/v1 | SUBCONSCIOUS_API_KEY | OpenAICompatProfile::full() |
| 41 | submodel | SubModel | https://api.submodel.com/v1 | SUBMODEL_API_KEY | OpenAICompatProfile::full() |
| 42 | synthetic | Synthetic | https://api.synthetic.new/openai/v1 | SYNTHETIC_API_KEY | OpenAICompatProfile::full() |

## 调研条目

### opencode — OpenCode Zen

- **registry 现状**：profile=`full()` · base_url=`"opencode_zen.rs"`（⚠️ 字面量是个文件名，不是 URL）· env=`OPENCODE_API_KEY`
- **变体**：opencode_zen（api.opencode.zen/v1）、opencode_go（api.opencode.dev/v1）

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（OpenAI 兼容网关，走标准 chat completions） | - | B | `reference/opencode/packages/web/src/content/docs/providers.mdx:87` |
| 能力支持 | 无差异 | - | B | 同上 |
| 思考机制 | 无差异（透传上游模型能力） | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer API key，`/connect` 存到 `auth.json`） | - | B | `reference/opencode/packages/web/src/content/docs/providers.mdx:20-21` |
| URL/端点 | ⚠️ registry 的 base_url 是 `"opencode_zen.rs"`（应为 https://api.opencode.zen/v1 之类的合法 URL），请求必然失败 | `base_url = "opencode_zen.rs"` → POST `opencode_zen.rs/chat/completions` 非法 | A(registry) | `openai_compat_registry.rs:1533` |
| 模型 ID | 官方文档格式 `provider/model-id`，如 `opencode/gpt-5.1-codex` | `"model": "opencode/gpt-5.1-codex"` | B | `reference/opencode/packages/web/src/content/docs/ar/agents.mdx:369` |

#### 2. aimux 现状对比

- **对比结论**：⚠️ 不一致（registry 声明错误）
- **aimux 代码位置**：`openai_compat_registry.rs:1533`
- **差距说明**：`opencode` 条目 base_url 被写成 `"opencode_zen.rs"`，不是合法 URL，任何调用都会失败；与 `opencode_zen` 条目（1547-1554，合法 URL）重复了 display 名 "OpenCode Zen"。
- **建议动作**：把 `openai_compat_registry.rs:1533` 的 base_url 修正为合法 URL（如 `https://api.opencode.zen/v1`），或删除该条目（与 opencode_zen 重复）。

#### 3. 证据与验证

- **证据等级**：B
- **验证状态**：🔲 未验证（registry 字面量可直接核对，属确定性事实）
- **存疑标记**：无（registry bug 为确定性事实）

### opencode_go — OpenCode Go

- **registry 现状**：profile=`full()` · base_url=`https://api.opencode.dev/v1` · env=`OPENCODE_GO_API_KEY`
- **变体**：-（与 opencode/opencode_zen 同族，独立订阅）

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（API key，OpenAI 兼容） | - | B | `reference/opencode/packages/web/src/content/docs/ar/go.mdx:42` |
| URL/端点 | 无差异（base `https://api.opencode.dev/v1` 与官方一致） | - | B | 同上 |
| 模型 ID | 无差异（标准模型 ID） | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:1537-1545`
- **差距说明**：无 request 级特殊配置。
- **建议动作**：无需动作。

#### 3. 证据与验证

- **证据等级**：B
- **验证状态**：🔲 未验证（仅文档引用）
- **存疑标记**：无

### opencode_zen — OpenCode Zen

- **registry 现状**：profile=`full()` · base_url=`https://api.opencode.zen/v1` · env=`OPENCODE_ZEN_API_KEY`
- **变体**：opencode、opencode_go

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer API key） | - | B | `reference/opencode/packages/web/src/content/docs/zen.mdx:44-45` |
| URL/端点 | 无差异（base `https://api.opencode.zen/v1`） | - | B | 同上 |
| 模型 ID | 无差异（`opencode/...` 前缀是 OpenCode 客户端侧写法，API 侧为裸模型名） | - | B | `reference/opencode/packages/web/src/content/docs/providers.mdx:96-119` |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:1546-1554`
- **差距说明**：无 request 级特殊配置。
- **建议动作**：无需动作；建议顺手处理 opencode 条目的坏 base_url。

#### 3. 证据与验证

- **证据等级**：B
- **验证状态**：🔲 未验证（仅文档引用）
- **存疑标记**：无

### orcarouter — OrcaRouter

- **registry 现状**：profile=`full()` · base_url=`https://api.orcarouter.com/v1` · env=`ORCAROUTER_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（OpenAI 兼容网关） | - | C | https://www.orcarouter.ai/ |
| 能力支持 | 无差异（透传上游模型能力，支持 tool calling） | - | C | https://www.promptfoo.dev/docs/providers/orcarouter/ |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer API key） | - | C | https://github.com/danny-avila/LibreChat/discussions/13316 |
| URL/端点 | ⚠️ 官方主域是 `orcarouter.ai`（www.orcarouter.ai、docs.orcarouter.ai）；registry 用 `api.orcarouter.com`，未见公开文档使用该域名 | `base_url=https://api.orcarouter.com/v1`（registry）vs 官方 `https://api.orcarouter.ai/v1` | C | https://www.orcarouter.ai/ |
| 模型 ID | 无差异（透传） | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖（端点域名存疑）
- **aimux 代码位置**：`openai_compat_registry.rs:1556-1563`
- **差距说明**：无法确认 `api.orcarouter.com` 是否仍有效（官方文档未见该域名）；网关类厂商本身无 request 体差异。
- **建议动作**：核实 `api.orcarouter.com` 与 `api.orcarouter.ai` 的对应关系；若旧域名失效则更新 registry。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证（仅文档引用）
- **存疑标记**：⚠️ 域名存疑

### ovhcloud — OVHcloud AI

- **registry 现状**：profile=`full()` · base_url=`https://oai.endpoints.kepler.ai.cloud.ovh.net/v1` · env=`OVHCLOUD_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（"compatible with the OpenAI API… just changing the base URL and the API key"） | - | B | `reference/langchain4j/docs/docs/integrations/embedding-models/ovh-ai.md:12` |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（`Authorization: Bearer` + AI Endpoints API key） | - | C | https://help.ovhcloud.com/csm/en-ca-public-cloud-ai-endpoints-responses-api?id=kb_article_view&sysparm_article=KB0075055 |
| URL/端点 | 无差异（base `https://oai.endpoints.kepler.ai.cloud.ovh.net/v1` 与官方一致；另有 `/doc/{model}/openapi.json` 生成式文档） | - | C | https://www.ovhcloud.com/en/public-cloud/ai-endpoints/catalog/ |
| 模型 ID | 无差异（模型目录 ID，如 `gpt-oss-120b`） | `"model": "gpt-oss-120b"` | C | https://www.ovhcloud.com/en/public-cloud/ai-endpoints/catalog/ |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:1565-1573`
- **差距说明**：无 request 级特殊配置。
- **建议动作**：无需动作。

#### 3. 证据与验证

- **证据等级**：B/C
- **验证状态**：🔲 未验证（仅文档引用）
- **存疑标记**：无

### parasail — Parasail

- **registry 现状**：profile=`full()` · base_url=`https://api.parasail.io/v1` · env=`PARASAIL_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（"fully OpenAI-compatible… use the OpenAI SDK against https://api.parasail.io/v1"） | - | C | https://docs.parasail.io/ |
| 能力支持 | 无差异（含 OpenAI Batch API 兼容） | - | C | https://docs.parasail.io/parasail-docs/api-reference/chat-completions |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer `PARASAIL_API_KEY`） | - | C | https://docs.parasail.io/ |
| URL/端点 | 无差异（base 与 registry 一致） | - | - | - |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:1574-1582`
- **差距说明**：无 request 级特殊配置。
- **建议动作**：无需动作。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证（仅文档引用）
- **存疑标记**：无

### perfxcloud — PerfXCloud

- **registry 现状**：profile=`full()` · base_url=`https://api.perfxcloud.com/v1` · env=`PERFXCLOUD_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（兼容 OpenAI ChatGPT 接口，"可直接使用 OpenAI SDK"） | - | C | https://zhuanlan.zhihu.com/p/706587985 |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | ⚠️ 未找到官方文档；推理类模型（DeepSeek-R1 系）大概率返回 `reasoning_content`，无证据 | - | ⚠️ | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer） | - | C | https://zhuanlan.zhihu.com/p/706587985 |
| URL/端点 | 无差异 | - | - | - |
| 模型 ID | 无差异（模型 ID 按平台模型广场） | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖（无证据显示差异）
- **aimux 代码位置**：`openai_compat_registry.rs:1583-1591`
- **差距说明**：公开资料仅社区文章，无官方 API 文档可核对。
- **建议动作**：无需动作；后续有官方文档再补。

#### 3. 证据与验证

- **证据等级**：C（弱，社区来源）
- **验证状态**：🔲 未验证
- **存疑标记**：⚠️ 证据不足（无官方文档，思考机制未确认）

### perplexity — Perplexity

- **registry 现状**：profile=`full()` · base_url=`https://api.perplexity.ai`（无 /v1）· env=`PERPLEXITY_API_KEY`
- **变体**：perplexity_agent

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | `max_tokens`（无 max_completion_tokens） | `{"model":"sonar-pro","messages":[...],"max_tokens":1024}` | C | https://docs.perplexity.ai/api-reference/sonar-post |
| 能力支持 | `response_format` 支持 `json_schema`（structured output）；官方参考未列出 `tools`/`top_k`（⚠️ 工具调用与 top_k 官方文档无证据，langchain 集成声称 topK 1-2048） | `{"response_format":{"type":"json_schema","json_schema":{...}}}` | C | https://docs.perplexity.ai/api-reference/sonar-post；https://docs.perplexity.ai/docs/sonar/quickstart |
| 思考机制 | `reasoning_effort`（minimal/low/medium/high）+ `stream_mode`（full 抑制 reasoning 事件 / concise 单独发 reasoning 事件）；推理 token 不可强制关闭（社区确认）；usage 含 `reasoning_tokens` | `{"model":"sonar-reasoning-pro","stream_mode":"concise","reasoning_effort":"high"}` | C | https://docs.perplexity.ai/api-reference/sonar-post；https://community.perplexity.ai/t/not-having-internal-cot-think-in-output-of-reasoning-models/2421 |
| 流式/usage | 官方支持 `stream_options:{"include_usage":true}`，usage 在最终 chunk 到达；usage 含 cost/search_context_size/citation_tokens/num_search_queries 等扩展字段 | `{"stream":true,"stream_options":{"include_usage":true}}` → 末尾 chunk 带 `"usage":{"reasoning_tokens":123,...}` | C | https://docs.perplexity.ai/docs/gateway/quickstart |
| 消息格式 | 无差异（标准 chat messages） | - | - | - |
| 特殊字段 | 搜索类字段：`web_search_options`、`search_mode`(web/academic/sec)、`disable_search`、`enable_search_classifier`、`search_domain_filter`、`search_recency_filter`、`return_images`、`return_related_questions`、`language_preference` 等 | `{"web_search_options":{"search_context_size":"high"},"search_mode":"academic","search_recency_filter":"week","return_related_questions":true}` | C | https://docs.perplexity.ai/api-reference/sonar-post |
| headers/认证 | 无差异（`Authorization: Bearer <token>`） | - | C | https://docs.perplexity.ai/api-reference/sonar-post |
| URL/端点 | ⚠️ 新规范端点 `POST /v1/sonar`（官方文档当前主页），legacy 为 `POST /chat/completions`（base 无 /v1）；registry base=`https://api.perplexity.ai`（无 /v1）+ `/chat/completions` 命中 legacy 路径，仍可用但与新规范不一致 | `POST https://api.perplexity.ai/v1/sonar`（新）vs `POST https://api.perplexity.ai/chat/completions`（legacy） | C | https://docs.perplexity.ai/api-reference/sonar-post |
| 模型 ID | `sonar`、`sonar-pro`、`sonar-deep-research`、`sonar-reasoning-pro`（官方枚举） | `"model":"sonar-reasoning-pro"` | C | https://docs.perplexity.ai/api-reference/sonar-post |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:1592-1600`；`convert.rs:1103-1109`（stream_options 已发）、`convert.rs:1326-1329`（reasoning_effort 透传）、`convert.rs:1216-1282`（response_format）
- **差距说明**：① `reasoning_effort` 语义：Perplexity 是 minimal/low/medium/high，aimux 透传不校验，需调用方对齐；② `stream_mode`/`disable_search`/`web_search_options` 等搜索字段无 profile 支持，只能 bodyOverrides 兜底；③ 端点：registry base 无 `/v1`，与新规范 `/v1/sonar` 路径不一致；④ 推理模型下 aimux 发 `max_completion_tokens`（convert.rs:1122-1130），Perplexity 只认 `max_tokens`。
- **建议动作**：bodyOverrides 兜底搜索字段；考虑把 base 更新为 `https://api.perplexity.ai/v1`（与 perplexity_agent 一致）并核对 legacy `/chat/completions` 兼容性；推理模型 `max_tokens` 命名可纳入 profile 新字段（如 `max_tokens_key`）。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证（官方文档引用；sonar-post 参考为 2026 版）
- **存疑标记**：⚠️ tools/top_k 支持情况无官方证据

### perplexity_agent — Perplexity Agent

- **registry 现状**：profile=`full()` · base_url=`https://api.perplexity.ai/v1` · env=`PERPLEXITY_API_KEY`
- **变体**：perplexity

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（Responses API 风格 `input`/`instructions`/`max_output_tokens`） | `{"model":"openai/gpt-5.1","input":"...","instructions":"..."}` | C | https://docs.perplexity.ai/docs/agent-api/openai-compatibility |
| 能力支持 | Agent API 完全兼容 OpenAI Responses API（`/v1/responses` 别名）；Sonar Chat Completions 已迁移到 Agent API | - | C | https://docs.perplexity.ai/api-reference/sonar-post（迁移说明） |
| 思考机制 | 无差异（同 Perplexity 主条目：`reasoning_effort`/`stream_mode`） | - | C | https://docs.perplexity.ai/api-reference/sonar-post |
| 流式/usage | 无差异（SSE 事件流，Responses 格式） | - | C | https://docs.perplexity.ai/docs/agent-api/openai-compatibility |
| 消息格式 | 无差异（Responses API `input` 数组） | - | - | - |
| 特殊字段 | 同 Perplexity 搜索类字段 | - | C | https://docs.perplexity.ai/api-reference/sonar-post |
| headers/认证 | 无差异（Bearer，同 PERPLEXITY_API_KEY） | - | C | https://docs.perplexity.ai/docs/agent-api/openai-compatibility |
| URL/端点 | canonical 端点为 `POST /v1/agent`；`/v1/responses` 为 OpenAI SDK 兼容别名；`/v1/sonar` 兼容 Chat Completions | `POST https://api.perplexity.ai/v1/agent` | C | https://docs.perplexity.ai/docs/agent-api/openai-compatibility |
| 模型 ID | 支持 `provider/model` 形式（`openai/gpt-5.6-sol`、`openai/gpt-5-mini` 等），即第三方模型前缀路由 | `"model":"openai/gpt-5.6-sol"` | C | https://docs.perplexity.ai/docs/agent-api/openai-compatibility |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:1601-1609`；`openai/responses.rs`（Responses API 模块存在）
- **差距说明**：aimux 的 OpenAI 兼容薄封装走 `/chat/completions` 路径；Agent API 的完整能力（`/v1/agent`、Responses 别名、`openai/...` 模型前缀）未在薄封装 profile 层表达。若走 chat completions 路径则基本可用（Sonar Chat Completions 仍兼容），但官方已建议迁移。
- **建议动作**：无需立即动作；若要完整支持需在封装层支持 `/v1/responses`（或 `/v1/agent`）路径与 `input` 消息格式。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证（仅文档引用）
- **存疑标记**：无

### petals — Petals

- **registry 现状**：profile=`full()` · base_url=`https://api.petals.dev/v1` · env=`PETALS_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | ⚠️ 无证据 | - | ⚠️ | - |
| 能力支持 | ⚠️ 无证据 | - | ⚠️ | - |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | ⚠️ 无证据（Petals 项目本体是去中心化本地推理，非托管 HTTP API） | - | ⚠️ | https://petals.dev/ |
| URL/端点 | ⚠️ 未找到 `api.petals.dev` 的官方托管服务文档 | - | ⚠️ | https://github.com/bigscience-workshop/petals |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：⚠️ 不一致（端点真实性无法确认）
- **aimux 代码位置**：`openai_compat_registry.rs:1610-1618`
- **差距说明**：Petals 官方项目（bigscience-workshop/petals）提供的是本地/去中心化推理，无 `api.petals.dev` 托管 OpenAI 兼容服务的公开文档；该 registry 条目指向的托管服务是否存在存疑。
- **建议动作**：核实 `api.petals.dev` 是否仍提供服务；无服务则删除或挂靠本地 Petals OpenAI 兼容 server。

#### 3. 证据与验证

- **证据等级**：⚠️（仅官网项目页，无 API 文档）
- **验证状态**：🔲 未验证
- **存疑标记**：⚠️ 证据不足，端点真实性存疑

### pinstripes — Pinstripes

- **registry 现状**：profile=`full()` · base_url=`https://api.pinstripes.io/v1` · env=`PINSTRIPES_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（标准 OpenAI SDK 直连） | - | C | https://pinstripes.io/blog/langchain/ |
| 能力支持 | 无差异（LangChain 全功能兼容，含 agent/tool calling） | - | C | https://pinstripes.io/blog/langchain/ |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异（自动前缀缓存，`usage` 计费含缓存 token） | - | C | https://pinstripes.io/blog/langchain/ |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（`api_key="sk-ps-..."`，Bearer） | - | C | https://pinstripes.io/blog/langchain/ |
| URL/端点 | ⚠️ 官方示例 base 为 `https://api.pinstripes.ai/v1`；registry 用 `api.pinstripes.io/v1`，两者关系未确认 | `base_url="https://api.pinstripes.ai/v1"`（官方示例）vs `https://api.pinstripes.io/v1`（registry） | C | https://pinstripes.io/blog/langchain/ |
| 模型 ID | 模型 ID 带 `ps/` 前缀约定 | `"model":"ps/qwen3.6-a3b"`、`"model":"ps/deepseek-v4-flash"` | C | https://pinstripes.io/blog/langchain/ |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:1619-1627`
- **差距说明**：① 域名 `api.pinstripes.io` vs 官方 `api.pinstripes.ai` 存疑；② `ps/` 模型前缀为透传约定，aimux 无校验逻辑（透传即用，不影响请求）。
- **建议动作**：核实域名；`ps/` 前缀无需动作（透传）。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证（仅官方博客示例）
- **存疑标记**：⚠️ 域名存疑

### pioneer — Pioneer

- **registry 现状**：profile=`full()` · base_url=`https://api.pioneer.ai/v1` · env=`PIONEER_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（"Drop-in OpenAI replacement… all SDK methods including streaming work unchanged"） | - | C | https://docs.pioneer.ai/concepts/inference |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer，Pioneer key） | - | C | https://docs.pioneer.ai/api-reference/inference/openai-compatible |
| URL/端点 | 无差异（base 与 registry 一致；另有原生 `POST /inference` 与 Anthropic 兼容端点，属可选） | - | C | https://agent.pioneer.ai/credits |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:1628-1636`
- **差距说明**：无 request 级特殊配置。
- **建议动作**：无需动作。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证（仅文档引用）
- **存疑标记**：无

### poe — Poe

- **registry 现状**：profile=`full()` · base_url=`https://api.poe.com/v1` · env=`POE_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（OpenAI Chat Completions 参数） | - | C | https://creator.poe.com/docs/external-applications/openai-compatible-api |
| 能力支持 | 同时支持 Chat Completions（`/v1/chat/completions`）与 Responses（`/v1/responses`） | - | C | https://creator.poe.com/docs/external-applications/openai-compatible-api |
| 思考机制 | 无差异（按上游模型能力） | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（`Authorization: Bearer YOUR_API_KEY`） | - | C | https://creator.poe.com/api-reference/overview |
| URL/端点 | 无差异（base `https://api.poe.com/v1` 与 registry 一致） | - | C | https://poe.com/api |
| 模型 ID | ⚠️ 模型名是 Poe 侧枚举（驼峰风格，如 `Claude-Sonnet-4.6`、`GPT-5.1-Codex`），不是上游裸模型名 | `{"model":"Claude-Sonnet-4.6","messages":[...]}` | C | https://creator.poe.com/api-reference/overview |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:1637-1645`
- **差距说明**：模型 ID 需用 Poe 平台枚举名（驼峰），aimux 透传模型名无映射问题（调用方填对即可）；无 request 体差异。
- **建议动作**：无需动作；文档标注 Poe 模型 ID 风格即可。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证（仅文档引用）
- **存疑标记**：⚠️ 模型 ID 枚举随 Poe 平台更新，未逐一核对

### poolside — Poolside

- **registry 现状**：profile=`full()` · base_url=`https://inference.poolside.ai/v1` · env=`POOLSIDE_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（OpenAI-compatible） | - | C | https://huggingface.co/poolside/Laguna-M.1 |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（`api_key` Bearer） | - | C | https://huggingface.co/poolside/Laguna-M.1 |
| URL/端点 | 无差异（base `https://inference.poolside.ai/v1` 与 registry 一致） | `OpenAI(base_url="https://inference.poolside.ai/v1", api_key="...")` | C | https://huggingface.co/poolside/Laguna-M.1 |
| 模型 ID | 无差异（如 `poolside/laguna-...`） | - | C | https://huggingface.co/poolside/Laguna-M.1 |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:1646-1654`
- **差距说明**：无 request 级特殊配置（证据较薄，仅 HF 模型卡示例）。
- **建议动作**：无需动作。

#### 3. 证据与验证

- **证据等级**：C（弱）
- **验证状态**：🔲 未验证
- **存疑标记**：无（若后续有官方 API 文档可补强）

### portkey — Portkey Gateway

- **registry 现状**：profile=`full()` · base_url=`https://api.portkey.ai/v1` · env=`PORTKEY_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（透传上游 OpenAI 参数） | - | C | https://docs.portkey.ai/docs/api-reference/inference-api/gateway-for-other-apis |
| 能力支持 | 无差异（250+ 模型路由） | - | - | - |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异（透传） | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异（配置/路由经 header 而非 body） | - | - | - |
| headers/认证 | **gateway 认证走 `x-portkey-api-key` header（值=$PORTKEY_API_KEY），并需 `x-portkey-provider` 指定上游 provider（如 `openai`、`@provider-slug`、`passthrough`），自定义端点加 `x-portkey-custom-host`，路由策略加 `x-portkey-config`；`Authorization` 传的是上游 provider 的 key，不是 Portkey key** | `curl https://api.portkey.ai/v1/chat/completions -H "x-portkey-api-key: $PORTKEY_API_KEY" -H "x-portkey-provider: openai" -H "Authorization: Bearer $OPENAI_API_KEY"` | C | https://docs.portkey.ai/docs/api-reference/inference-api/gateway-for-other-apis；https://docs.portkey.ai/docs/product/ai-gateway/universal-api |
| URL/端点 | base `https://api.portkey.ai/v1` 与 registry 一致；内置路由 `chat/completions` 走网关逻辑，`/v1/proxy/chat/completions` 前缀强制裸透传 | `POST https://api.portkey.ai/v1/proxy/chat/completions` | C | https://docs.portkey.ai/docs/api-reference/inference-api/gateway-for-other-apis |
| 模型 ID | 无差异（透传上游模型 ID） | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:1655-1663`；`openai/mod.rs:96-119`（`OpenAIConfig.headers` 自定义 header）
- **差距说明**：aimux 默认把 API key 放 `Authorization: Bearer`；Portkey 需要 `x-portkey-api-key`（网关认证）+ `x-portkey-provider`/`x-portkey-custom-host`（路由）。可通过 `OpenAIConfig.headers` 手工补 header，但 profile 层无内置支持，且 `PORTKEY_API_KEY` 不会自动落到 `x-portkey-api-key`。
- **建议动作**：文档标注需在 headers 配置中追加 `x-portkey-api-key`/`x-portkey-provider`；如用户量大可加 profile 字段（如 `auth_header_name`）。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证（官方文档引用）
- **存疑标记**：无

### ppinfra — PPInfra（PPIO 派欧云）

- **registry 现状**：profile=`full()` · base_url=`https://api.ppio.com/openai` · env=`PPIO_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（OpenAI 兼容，SDK base_url 配"域名 + /openai"） | `OpenAI(base_url="https://api.ppio.com/openai")` | C | https://ppio.com/docs/llms-full.txt |
| 能力支持 | 无差异（LLM/图像/视频/音频多模态） | - | C | https://ppio.com/docs/third-party/fastgpt-use |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer API 密钥） | - | C | https://ppio.com/docs/third-party/fastgpt-use |
| URL/端点 | 无差异：完整端点固定为 `https://api.ppio.com/openai/v1/chat/completions`，与 registry base（`/openai`）+ `/v1/chat/completions` 拼接一致 | `POST https://api.ppio.com/openai/v1/chat/completions` | C | https://ppio.com/docs/third-party/fastgpt-use |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:1664-1672`
- **差距说明**：base_url 无尾随 `/v1` 的设计与官方 SDK 用法（"域名 + /openai"）一致。
- **建议动作**：无需动作。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证（仅文档引用）
- **存疑标记**：无

### predibase — Predibase

- **registry 现状**：profile=`full()` · base_url=`https://serving.app.predibase.com/v1` · env=`PREDIBASE_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（OpenAI Chat Completions v1 兼容） | - | C | https://apis.io/apis/predibase/predibase-adapters-api/ |
| 能力支持 | LoRA adapter：`adapter_id`（Predibase ID 或 HF ID）+ `adapter_source`（pbase/hub/s3）+ `adapter_version` | `{"model":"mistral-7b","adapter_id":"my-adapter-id","adapter_version":3}` | C | https://apis.io/apis/predibase/predibase-adapters-api/；https://developers.llamaindex.ai/python/framework-api-reference/llms/predibase/ |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | `adapter_id`/`adapter_source`/`adapter_version` 为 LoRA 微调适配字段 | 见上 | C | https://developers.llamaindex.ai/python/framework-api-reference/llms/predibase/ |
| headers/认证 | 无差异（`Authorization: Bearer <PREDIBASE_API_TOKEN>`） | - | C | https://apis.io/apis/predibase/predibase-adapters-api/ |
| URL/端点 | ⚠️ 官方 OpenAPI 的 inference base 为 `https://serving.app.predibase.com/{tenant}/deployments/v2/llms/{model}`，OpenAI 兼容路由在其 `/v1` 后缀下（tenant=租户 ID，model=部署名）；registry 的 `https://serving.app.predibase.com/v1` 缺 tenant/deployment 路径段，能否直接用存疑 | `https://serving.app.predibase.com/{tenant}/deployments/v2/llms/{model}/v1/chat/completions` | C | https://apis.io/apis/predibase/predibase-adapters-api/ |
| 模型 ID | `model` = 部署名/serverless base model ID（如 `mistral-7b`） | - | C | https://developers.llamaindex.ai/python/framework-api-reference/llms/predibase/ |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:1673-1681`
- **差距说明**：① LoRA 字段（adapter_id 等）无 profile 支持 → bodyOverrides 兜底；② 端点路径（tenant/deployment）与 registry base 不一致，存疑。
- **建议动作**：核实 base_url（可能需要 `{tenant}` 路径变量支持，这是 registry 宏当前不支持的形态）；adapter 字段走 bodyOverrides。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证（OpenAPI 镜像 + LlamaIndex 集成文档）
- **存疑标记**：⚠️ base_url 路径形态存疑

### privatemode_ai — Privatemode AI

- **registry 现状**：profile=`full()` · base_url=`http://localhost:8080/v1` · env=`PRIVATEMODE_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | ⚠️ 本地代理（localhost:8080），认证取决于本地代理配置，无公开规范 | - | ⚠️ | - |
| URL/端点 | 无差异（本地代理，非托管服务） | - | ⚠️ | - |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖（无证据显示差异）
- **aimux 代码位置**：`openai_compat_registry.rs:1682-1690`
- **差距说明**：本地代理类端点，无公开文档；行为取决于用户本地部署。
- **建议动作**：无需动作。

#### 3. 证据与验证

- **证据等级**：⚠️
- **验证状态**：🔲 未验证
- **存疑标记**：⚠️ 证据不足（本地端点无公开规范）

### publicai — Publicai

- **registry 现状**：profile=`full()` · base_url=`https://platform.publicai.co/v1` · env=`PUBLICAI_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | **PublicAI 不接受 `max_completion_tokens`，需映射为 `max_tokens`**（litellm 将其作为首个 JSON 配置 provider 的 `param_mappings` 示例） | `"param_mappings": {"max_completion_tokens": "max_tokens"}`（litellm 配置原文） | B | `reference/litellm/litellm/llms/openai_like/README.md:68-87` |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | **要求 content 列表转字符串**（litellm `special_handling.convert_content_list_to_string: true`） | `"special_handling": {"convert_content_list_to_string": true}`（litellm 配置原文） | B | `reference/litellm/litellm/llms/openai_like/README.md:68-87` |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer，`PUBLICAI_API_KEY`） | - | B | 同上 |
| URL/端点 | 无差异（base 与 registry 一致：`https://api.publicai.co/v1`） | - | B | 同上 |
| 模型 ID | litellm 模型写法带 provider 前缀：`publicai/swiss-ai/apertus-8b-instruct`（API 侧模型 ID 为后段） | `model="publicai/swiss-ai/apertus-8b-instruct"` | B | 同上 |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:1691-1699`；`convert.rs:1118-1138`（max_tokens/max_completion_tokens 分支）
- **差距说明**：① 推理模型分支 aimux 发 `max_completion_tokens`（convert.rs:1122-1130），PublicAI 只认 `max_tokens` → 推理模型调用会失败；② content 列表转字符串：aimux 标准消息格式默认就是字符串，多模态 content 数组需人工处理。
- **建议动作**：若 PublicAI 上跑推理模型，纳入 `max_tokens_key` 类 profile 字段候选；其余 bodyOverrides 兜底。

#### 3. 证据与验证

- **证据等级**：B
- **验证状态**：🔲 未验证（litellm reference 文档）
- **存疑标记**：无

### qihang_ai — QiHang（启航 AI）

- **registry 现状**：profile=`full()` · base_url=`https://api.qhaigc.net/v1` · env=`QIHANG_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（兼容 OpenAI SDK） | - | C | https://www.qhaigc.net/docs/api-reference/introduction |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（`Authorization: Bearer sk-...`） | - | C | https://www.qhaigc.net/docs/models |
| URL/端点 | 无差异（base `https://api.qhaigc.net/v1` 与 registry 一致；官方教程提示"是否带 /v1 取决于平台"） | `POST https://api.qhaigc.net/v1/chat/completions` | C | https://www.qhaigc.net/docs/tutorials |
| 模型 ID | 无差异（统一 API，模型 ID 见官方模型列表） | - | C | https://www.qhaigc.net/docs/models |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:1700-1708`
- **差距说明**：无 request 级特殊配置。
- **建议动作**：无需动作。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证（仅文档引用）
- **存疑标记**：无

### qihoo360 — 360 AI

- **registry 现状**：profile=`full()` · base_url=`https://api.360.cn/v1` · env=`AI360_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（宣称 OpenAI 兼容，base_url=https://api.360.cn/v1 可一行迁移） | - | C(弱) | https://tools321.com/ai/ai-model-free-list/ |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | ⚠️ 未找到官方文档（模型家族含 360gpt2-pro/360gpt-turbo/360gpt-flash；推理/思考字段无证据） | - | ⚠️ | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | ⚠️ 无官方文档（推测 Bearer） | - | ⚠️ | - |
| URL/端点 | 无差异（base 与社区描述一致：https://api.360.cn/v1） | - | C(弱) | https://tools321.com/ai/ai-model-free-list/ |
| 模型 ID | 无差异（360gpt2-pro/360gpt-turbo/360gpt-flash/360zhinao-embedding） | `"model":"360gpt2-pro"` | C(弱) | https://tools321.com/ai/ai-model-free-list/ |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖（无证据显示差异）
- **aimux 代码位置**：`openai_compat_registry.rs:1709-1717`
- **差距说明**：未能访问 360 官方 API 文档（未索引到）；现有证据仅第三方汇总页。
- **建议动作**：无需动作；后续拿到官方文档再核对 thinking/认证细节。

#### 3. 证据与验证

- **证据等级**：C（弱，第三方汇总）
- **验证状态**：🔲 未验证
- **存疑标记**：⚠️ 证据不足（官方文档未获取）

### qiniu_ai — Qiniu AI

- **registry 现状**：profile=`full()` · base_url=`https://api.qiniu.com/v1` · env=`QINIU_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（"接口兼容 OpenAI 风格，参数完全兼容 OpenAI 的 sdk"） | - | C | https://developer.qiniu.com/aitokenapi/kb/13462/aitoken-use-faq?category=kb |
| 能力支持 | 兼容 OpenAI（/v1/chat/completions）与 Anthropic（/v1/messages）两套协议 | - | C | https://developer.qiniu.com/aitokenapi/kb/13462/aitoken-use-faq?category=kb |
| 思考机制 | DeepSeek-R1 默认带思考过程（回答开头有 think 标签）；grok-4-fast 无参数可关闭推理（只能提示词引导）⚠️ 此为七牛聚合平台侧行为 | - | C | https://developer.qiniu.com/aitokenapi/kb/13462/aitoken-use-faq?category=kb |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（`Authorization` header，FAQ 明示常见错误是 "authorization header missing"） | - | C | https://developer.qiniu.com/aitokenapi/kb/13462/aitoken-use-faq?category=kb |
| URL/端点 | ⚠️ 官方 FAQ 示例 base 为 `https://api.qnaigc.com/v1`（"域名 + /v1"，易错点示例），registry 用 `api.qiniu.com/v1`；两域名关系未确认（qnaigc 疑为七牛 AI Token API 分发域名） | `base_url=https://api.qnaigc.com/v1`（FAQ 示例）vs `https://api.qiniu.com/v1`（registry） | C | https://developer.qiniu.com/aitokenapi/kb/13462/aitoken-use-faq?category=kb |
| 模型 ID | `model` 必须与 AI 大模型广场的 "API model 参数" 严格一致，否则 503 quota_exceeded_error | `"model":"deepseek-r1:671b"`（FAQ 报错示例） | C | https://developer.qiniu.com/aitokenapi/kb/13462/aitoken-use-faq?category=kb |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:1718-1726`
- **差距说明**：① base 域名（api.qiniu.com vs api.qnaigc.com）存疑；② 模型 ID 严格匹配要求靠调用方保证；③ 推理思考字段（think 标签/reasoning_content）为平台行为，aimux 已解析 `reasoning_content`（model.rs:558-563）。
- **建议动作**：核实域名后修正 registry；模型 ID 校验不做（透传）。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证（官方 FAQ 引用）
- **存疑标记**：⚠️ 域名存疑

### regolo_ai — Regolo AI

- **registry 现状**：profile=`full()` · base_url=`https://api.regolo.ai/v1` · env=`REGOLO_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（"built on the OpenAI API standard"） | - | C | https://regolo.ai/ |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Authorization） | - | C | https://regolo.ai/ |
| URL/端点 | 无差异（base `https://api.regolo.ai/v1`，官方强调无尾随斜杠无多余路径） | - | C | https://regolo.ai/using-regolo-models-with-opencode/ |
| 模型 ID | 无差异 | - | C | https://models.dev/providers/regolo-ai |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:1727-1735`
- **差距说明**：无 request 级特殊配置。
- **建议动作**：无需动作。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证（仅文档引用）
- **存疑标记**：无

### reka_ai — Reka AI

- **registry 现状**：profile=`full()` · base_url=`https://api.reka.ai/v1` · env=`REKA_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | `max_tokens`（官方示例；未见 max_completion_tokens） | `client.chat.completions.create(model="reka-flash", messages=[...], max_tokens=2048, stream=True)` | C | https://docs.reka.ai/chat/overview |
| 能力支持 | 无差异（fully OpenAI-compatible，OpenAI Python SDK 直连） | - | C | https://docs.reka.ai/chat/overview |
| 思考机制 | 无差异（Reka 模型不以思维链输出） | - | - | - |
| 流式/usage | 无差异（`stream=True` 标准 SSE） | - | C | https://docs.reka.ai/chat/overview |
| 消息格式 | 无差异（多模态：图片/短视频/音频内容类型；assistant-completion 技巧=最后一条 assistant 消息续写） | - | C | https://docs.reka.ai/chat/overview |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 认证同时接受 `X-Api-Key` header 或 `Authorization: Bearer`（API reference 两种都列） | `X-Api-Key: YOUR_API_KEY` 或 `Authorization: Bearer <token>` | C | https://docs.reka.ai/vision/api-reference/inference/create-chat-completion |
| URL/端点 | 无差异（base `https://api.reka.ai/v1` 与 registry 一致；另有 vision 专用 `https://vision-agent.api.reka.ai`） | - | C | https://docs.reka.ai/chat/overview |
| 模型 ID | 无差异（`reka-flash`、`reka-core` 等，见 models 页） | - | C | https://docs.reka.ai/chat/models |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:1736-1744`
- **差距说明**：aimux 用 `Authorization: Bearer`（Reka 明确支持）；推理模型分支的 `max_completion_tokens` 理论上 Reka 不认，但 Reka 非思维链模型，`is_reasoning_model` 分支不触发时走 `max_tokens`，无实际问题。
- **建议动作**：无需动作。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证（仅文档引用）
- **存疑标记**：无

### requesty — Requesty

- **registry 现状**：profile=`full()` · base_url=`https://api.requesty.ai/v1` · env=`REQUESTY_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（OpenAI SDK 直连） | - | C | https://docs.requesty.ai/quickstart |
| 能力支持 | 无差异（300+ 模型统一端点） | - | - | - |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异（流式需 `stream_options:{"include_usage":true}` 收 usage；响应 `usage` 默认带 `cost` 字段；额外响应 header `x-requesty-provider/cache/latency-ms/request-id`） | `"usage":{"prompt_tokens":13,"completion_tokens":17,"total_tokens":30,"cost":0.0000935}` | C | https://docs.requesty.ai/quickstart |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | body 可带 `requesty` 元数据对象（tags/user_id/trace_id/extra）；可选 header `HTTP-Referer`/`X-Title`（分析打标） | `{"model":"openai/gpt-4o","messages":[...],"requesty":{"tags":["quickstart"],"user_id":"user_1234","trace_id":"session_abc123"}}` | C | https://docs.requesty.ai/quickstart |
| headers/认证 | 无差异（Bearer，`REQUESTY_API_KEY`）；`HTTP-Referer`/`X-Title` 可选 | `-H "HTTP-Referer: https://yourapp.com" -H "X-Title: My App"` | C | https://docs.requesty.ai/quickstart |
| URL/端点 | ⚠️ 官方文档 base 为 `https://router.requesty.ai/v1`（llms.txt 与 quickstart 一致）；registry 用 `api.requesty.ai/v1`，关系未确认 | `base_url="https://router.requesty.ai/v1"`（官方）vs `https://api.requesty.ai/v1`（registry） | C | https://docs.requesty.ai/quickstart；https://www.requesty.ai/llms.txt |
| 模型 ID | 模型 ID 可带 provider 前缀（`openai/gpt-4o`）或 policy 名（`policy/sonnet-with-fallback`） | `"model":"openai/gpt-4o"`、`"model":"policy/sonnet-with-fallback"` | C | https://docs.requesty.ai/quickstart |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:1745-1753`
- **差距说明**：① base 域名（api.requesty.ai vs router.requesty.ai）存疑；② `requesty` body 元数据对象无 profile 支持 → bodyOverrides 兜底；③ `HTTP-Referer`/`X-Title` 需自定义 headers。
- **建议动作**：核实域名；其余 bodyOverrides/headers 兜底即可。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证（官方文档引用）
- **存疑标记**：⚠️ 域名存疑

### reve — Reve

- **registry 现状**：profile=`full()` · base_url=`https://api.reve.ai/v1` · env=`REVE_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | ⚠️ 无证据 | - | ⚠️ | - |
| 能力支持 | ⚠️ 无证据 | - | ⚠️ | - |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | ⚠️ 无证据 | - | ⚠️ | - |
| URL/端点 | ⚠️ 未找到 `api.reve.ai` 官方 API 文档 | - | ⚠️ | - |
| 模型 ID | ⚠️ 无证据 | - | ⚠️ | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖（无证据显示差异）
- **aimux 代码位置**：`openai_compat_registry.rs:1754-1762`
- **差距说明**：完全查不到公开信息（WebSearch/WebFetch 无结果）。
- **建议动作**：无需动作；标记为需人工确认厂商。

#### 3. 证据与验证

- **证据等级**：⚠️
- **验证状态**：🔲 未验证
- **存疑标记**：⚠️ 证据不足（查不到任何信息）

### routing_run — routing.run

- **registry 现状**：profile=`full()` · base_url=`https://api.routing.run/v1` · env=`ROUTING_RUN_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（OpenAI-compatible `/v1/chat/completions`） | - | C | https://github.com/monotykamary/pi-routing-run-provider |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer，`ROUTING_RUN_API_KEY`） | - | C | https://mastra.ai/models/providers/routing-run |
| URL/端点 | 双端点：`api.routing.sh`（主，更快）与 `api.routing.run`（备） | `base_url=https://api.routing.sh/v1`（主） | C | https://github.com/monotykamary/pi-routing-run-provider |
| 模型 ID | 无差异（透传） | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:1763-1771`
- **差距说明**：无 request 级特殊配置；主域 api.routing.sh 可作备选 base。
- **建议动作**：无需动作（可备注主端点）。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证（社区 provider 代码 + Mastra 文档）
- **存疑标记**：无

### sakana — Sakana AI

- **registry 现状**：profile=`full()` · base_url=`https://api.sakana.ai/v1` · env=`SAKANA_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（OpenAI Chat Completions） | - | C | https://console.sakana.ai/get-started |
| 能力支持 | 同时支持 Chat Completions 与 Responses API | - | C | https://console.sakana.ai/get-started |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer） | - | C | https://www.analyticsvidhya.com/blog/2026/06/sakana-fugu-multi-agent-system-as-a-model/ |
| URL/端点 | 无差异（base `https://api.sakana.ai/v1` 与 registry 一致） | - | C | https://console.sakana.ai/get-started |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:1772-1780`
- **差距说明**：无 request 级特殊配置。
- **建议动作**：无需动作。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证（仅文档引用）
- **存疑标记**：无

### sambanova — SambaNova

- **registry 现状**：profile=`full()` · base_url=`https://api.sambanova.ai/v1` · env=`SAMBANOVA_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（`max_tokens` 标准；官方示例用 `model`+`messages`） | - | C | https://docs.sambanova.ai/docs/en/features/openai-compatibility |
| 能力支持 | ① `presence_penalty`/`frequency_penalty` **不支持，会被忽略**；② `n` 支持 1–8，且 **n>1 与 tools/function calling 组合返回 400**；③ `seed`/`logit_bias` 支持（文本模型）；④ `top_k` 支持（OpenAI 客户端无此参数，SambaNova 有）；⑤ 响应无 `system_fingerprint` | `{"model":"Meta-Llama-3.3-70B-Instruct","n":2,"tools":[...]}` → 400；`{"top_k":5}` → 接受 | C | https://docs.sambanova.ai/docs/en/features/openai-compatibility |
| 思考机制 | 推理模型（如 DeepSeek-R1 系）流式输出 `reasoning_content`（社区实证；官方 OpenAI 兼容页未列专用开关字段） | `{"choices":[{"delta":{"reasoning_content":"...","content":""}}]}` | C | https://docs.sambanova.ai/docs/en/features/openai-compatibility；https://github.com/karthink/gptel/issues/669 |
| 流式/usage | `stream_options.include_usage` 用于取流式用量（LlamaIndex 集成文档明示）；SSE 标准 | `SambaNovaCloud(..., stream_options={"include_usage": True})` | C | https://developers.llamaindex.ai/python/framework-api-reference/llms/sambanovasystems/ |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异（`top_k` 属能力字段） | - | - | - |
| headers/认证 | 无差异（`api_key` + `base_url`，Bearer） | - | C | https://docs.sambanova.ai/docs/en/features/openai-compatibility |
| URL/端点 | 无差异（base 与 registry 一致；另有 Responses API `POST /v1/responses`） | - | C | https://docs.sambanova.ai/docs/en/features/openai-compatibility |
| 模型 ID | 无差异（SambaNova 目录模型名，如 `Meta-Llama-3.3-70B-Instruct`、`gpt-oss-120b`） | `"model":"Meta-Llama-3.3-70B-Instruct"` | C | https://docs.sambanova.ai/docs/en/features/openai-compatibility |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:1781-1789`；`convert.rs:1202-1207`（frequency/presence_penalty 总是发送）
- **差距说明**：① aimux 有值时会发送 `frequency_penalty`/`presence_penalty`（convert.rs:1202-1207），SambaNova 忽略而非报错 → 行为安全但语义丢失；② `n>1` + tools 的 400 组合无预防；③ 推理模型分支 `max_completion_tokens`（SambaNova 规范未见该参数，OpenAI 兼容层一般也接受）；④ `reasoning_content` 解析 ✅ 已覆盖（model.rs:558-563）。
- **建议动作**：可加 profile 标志（如 `ignore_penalty_params: true` 时静默丢弃）；`n>1`+tools 组合校验可选。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证（官方 OpenAI 兼容页 + LlamaIndex 集成文档）
- **存疑标记**：无

### sarvam — Sarvam AI

- **registry 现状**：profile=`full()` · base_url=`https://api.sarvam.ai/v1` · env=`SARVAM_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | `max_tokens`（默认 2048；官方参数表无 max_completion_tokens） | `{"model":"sarvam-105b","messages":[...],"max_tokens":2048}` | C | https://docs.sarvam.ai/api-reference/chat/chat-completions |
| 能力支持 | `tools`/`tool_choice`/`response_format`（json_schema 与 json_object）均支持；`seed`（beta）、`n` 1-128 | - | C | https://docs.sarvam.ai/api-reference/chat/chat-completions |
| 思考机制 | `reasoning_effort`（默认 `medium`；**显式置 null 可关闭思考**） | `{"reasoning_effort":null}` 关闭；`{"reasoning_effort":"high"}` 加深 | C | https://docs.sarvam.ai/api-reference/chat/chat-completions |
| 流式/usage | 无差异（标准 SSE） | - | C | 同上 |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | `wiki_grounding`（bool，默认 false，开启后回答基于 wiki 检索）；可选 `api-subscription-key` header（sk_xxx 订阅 key） | `{"wiki_grounding":true}` | C | https://docs.sarvam.ai/api-reference/chat/chat-completions |
| headers/认证 | 标准 `Authorization: Bearer`；另可选 `api-subscription-key` header 走订阅计费 | `api-subscription-key: sk_xxx` | C | https://docs.sarvam.ai/api-reference/chat/chat-completions；https://docs.pipecat.ai/api-reference/server/services/llm/sarvam |
| URL/端点 | 无差异（base `https://api.sarvam.ai/v1` 与 registry 一致） | `POST https://api.sarvam.ai/v1/chat/completions` | C | https://docs.sarvam.ai/api-reference/chat/chat-completions |
| 模型 ID | 无差异（如 `sarvam-105b`，128K 上下文） | - | C | 同上 |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:1790-1798`；`convert.rs:1326-1329`（reasoning_effort 透传）
- **差距说明**：① `reasoning_effort` 的 null 关闭语义需调用方传 null（aimux 透传不校验）；② `wiki_grounding` 无 profile 支持 → bodyOverrides；③ `api-subscription-key` header 需自定义 headers；④ 推理模型分支 `max_completion_tokens` vs Sarvam `max_tokens`。
- **建议动作**：bodyOverrides 兜底 `wiki_grounding`；文档标注 subscription header 用法。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证（官方 API 参考引用）
- **存疑标记**：无

### scaleway — Scaleway AI

- **registry 现状**：profile=`full()` · base_url=`https://api.scaleway.ai/v1` · env=`SCALEWAY_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（OpenAI SDK 直连） | - | C | https://www.scaleway.com/en/docs/generative-apis/quickstart/ |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（`Authorization: Bearer ${SCW_SECRET_KEY}`，即 Scaleway IAM key） | `curl https://api.scaleway.ai/v1/chat/completions -H "Authorization: Bearer ${SCW_SECRET_KEY}"` | C | https://www.scaleway.com/en/docs/generative-apis/api-cli/using-generative-apis/ |
| URL/端点 | 默认项目可省略：`https://api.scaleway.ai/v1`；非默认项目需插入 Project ID：`https://api.scaleway.ai/{project_id}/v1` | `https://api.scaleway.ai/78e655b5-feb0-417c-bb3f-8c448bd0e8da/v1` | C | https://www.scaleway.com/en/docs/generative-apis/api-cli/using-generative-apis/ |
| 模型 ID | 无差异（模型目录 ID） | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:1799-1807`
- **差距说明**：默认项目 base 与 registry 一致；非默认项目需在 base_url 中插入 project_id（用户自定义 base_url 可解决，registry 宏不支持模板化 base）。
- **建议动作**：无需动作（文档标注 project_id 变体即可）。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证（官方文档引用）
- **存疑标记**：无

### scx_ai — SCX AI

- **registry 现状**：profile=`full()` · base_url=`https://api.scx.ai/v1` · env=`SCX_AI_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（OpenAI 与 Anthropic 兼容） | - | C | https://scx.ai/ |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer；官方示例环境变量名为 `SCX_API_KEY`，registry 用 `SCX_AI_API_KEY`，两者关系未确认） | `OpenAI(base_url="https://api.scx.ai/v1", api_key=os.environ.get("SCX_API_KEY"))` | C | https://scx.ai/ |
| URL/端点 | 无差异（base 与 registry 一致） | - | C | https://scx.ai/ |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:1808-1816`
- **差距说明**：env 变量名与官方示例（SCX_API_KEY vs SCX_AI_API_KEY）不同，属注册层面命名，不影响 request 构造。
- **建议动作**：无需动作（可备注 env 别名）。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证（仅文档引用）
- **存疑标记**：无

### siliconflow — SiliconFlow

- **registry 现状**：profile=`full()` · base_url=`https://api.siliconflow.cn/v1` · env=`SILICONFLOW_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | `max_tokens`（无 max_completion_tokens） | `{"model":"Pro/deepseek-ai/DeepSeek-R1","messages":[...],"max_tokens":4096}` | C | https://docs.siliconflow.cn/cn/userguide/capabilities/reasoning |
| 能力支持 | `response_format` json mode 支持；推理模型建议 temperature 0.5-0.7、top_p 0.95（官方 R1 建议） | - | C | https://docs.siliconflow.cn/cn/userguide/capabilities/reasoning |
| 思考机制 | **`thinking_budget` 请求参数**（控制思维链 token 上限，Qwen3 系原生强制截断，其他模型可能继续）；返回 `reasoning_content`（流式 delta 与 message 同级） | `{"thinking_budget":1024,"max_tokens":4096}` → 响应 `choices[0].message.reasoning_content` / 流式 `delta.reasoning_content` | C | https://docs.siliconflow.cn/cn/userguide/capabilities/reasoning |
| 流式/usage | 无差异（标准 SSE；`stream_options` 可用） | - | C | 同上 |
| 消息格式 | 无差异（`reasoning_content` 属思考机制类；多模态 image_url 支持） | - | C | 同上 |
| 特殊字段 | `thinking_budget` 为平台特有能力字段 | `{"thinking_budget":1024}` | C | 同上 |
| headers/认证 | 无差异（Bearer，`SILICONFLOW_API_KEY`） | - | C | 同上 |
| URL/端点 | 无差异（base `https://api.siliconflow.cn/v1/` 与 registry 一致） | - | C | 同上 |
| 模型 ID | 模型 ID 可带 `Pro/` 性能前缀（`Pro/deepseek-ai/DeepSeek-R1` vs `deepseek-ai/DeepSeek-R1`） | `"model":"Pro/deepseek-ai/DeepSeek-R1"` | C | 同上 |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:1817-1825`；`model.rs:558-563`（reasoning_content 流式解析 ✅）；`types.rs:37-40`（message.reasoning_content ✅）
- **差距说明**：① `thinking_budget` 无 profile 支持 → bodyOverrides 兜底；② 推理模型分支 `max_completion_tokens`（convert.rs:1122-1130）vs SiliconFlow `max_tokens`；③ `Pro/` 前缀透传即可，无需映射。
- **建议动作**：`thinking_budget` 文档化走 bodyOverrides；`max_tokens` 命名纳入 `max_tokens_key` 候选。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证（官方文档引用，含请求/响应示例）
- **存疑标记**：无

### snowflake — Snowflake（Cortex）

- **registry 现状**：profile=`full()` · base_url=`https://<account-identifier>.snowflakecomputing.com/api/v2/cortex/v1` · env=`SNOWFLAKE_PAT`
- **变体**：snowflake_cortex

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（Chat Completions 遵循 OpenAI 规范，OpenAI Python SDK 直连） | `OpenAI(api_key="<SNOWFLAKE_PAT>", base_url="https://<account-identifier>.snowflakecomputing.com/api/v2/cortex/v1")` | C | https://docs.snowflake.com/fr/user-guide/snowflake-cortex/cortex-rest-api |
| 能力支持 | 无差异（所有模型：OpenAI、Claude、Llama、Mistral、DeepSeek、Snowflake） | - | C | 同上 |
| 思考机制 | 无差异（取决于具体模型） | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异（另提供 Anthropic Messages API `/api/v2/cortex/v1/messages`，Claude 专用） | - | C | 同上 |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 标准 `Authorization: Bearer <PAT>`；**可选 `X-Snowflake-Authorization-Token-Type` header 声明 token 类型**（OAUTH / PROGRAMMATIC_ACCESS_TOKEN 等） | `X-Snowflake-Authorization-Token-Type: PROGRAMMATIC_ACCESS_TOKEN` | C | https://docs.snowflake.com/fr/user-guide/snowflake-cortex/cortex-rest-api |
| URL/端点 | 无差异（`https://<account-identifier>.snowflakecomputing.com/api/v2/cortex/v1/chat/completions`，registry 模板化 base 一致；account-identifier 需用户替换） | `POST https://<account>.snowflakecomputing.com/api/v2/cortex/v1/chat/completions` | C | 同上 |
| 模型 ID | 无差异（目录模型名，如 `claude-sonnet-4-5`） | `"model":"claude-sonnet-4-5"` | C | 同上 |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:1826-1834`
- **差距说明**：PAT 走 Bearer ✅；`X-Snowflake-Authorization-Token-Type` 可选 header 可经 `OpenAIConfig.headers` 配置；account-identifier 需用户替换（registry 模板 base 已表达占位符）。
- **建议动作**：无需动作（可选 header 文档化）。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证（官方文档引用）
- **存疑标记**：无

### snowflake_cortex — Snowflake Cortex

- **registry 现状**：profile=`full()` · base_url=`https://<account-identifier>.snowflakecomputing.com/api/v2/cortex/v1` · env=`SNOWFLAKE_CORTEX_PAT`
- **变体**：snowflake

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | C | https://docs.snowflake.com/fr/user-guide/snowflake-cortex/cortex-rest-api |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 与 snowflake 主条目相同（Bearer PAT + 可选 `X-Snowflake-Authorization-Token-Type`），仅 env 变量不同（SNOWFLAKE_CORTEX_PAT） | 同上 | C | 同上 |
| URL/端点 | 与 snowflake 主条目相同 | - | C | 同上 |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖（同 snowflake）
- **aimux 代码位置**：`openai_compat_registry.rs:1835-1843`
- **差距说明**：与 snowflake 条目为同一端点、不同 env（SNOWFLAKE_CORTEX_PAT），无 request 级差异。
- **建议动作**：无需动作。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证（同 snowflake）
- **存疑标记**：无

### stackit — STACKIT

- **registry 现状**：profile=`full()` · base_url=`https://api.openai-compat.model-serving.eu01.onstackit.cloud/v1` · env=`STACKIT_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（OpenAI 兼容托管端点） | - | C | https://docs.haystack.deepset.ai/reference/integrations-stackit |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（API Key，Bearer） | - | C | https://github.com/stackitcloud/n8n-nodes-stackit-ai-model-serving |
| URL/端点 | 无差异（base 与 registry 一致，Haystack/ models.dev 均确认） | `api_base_url="https://api.openai-compat.model-serving.eu01.onstackit.cloud/v1"` | C | https://models.dev/providers/stackit |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:1844-1852`
- **差距说明**：无 request 级特殊配置。
- **建议动作**：无需动作。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证（第三方集成文档 + models.dev）
- **存疑标记**：无

### stepfun — StepFun（阶跃星辰）

- **registry 现状**：profile=`full()` · base_url=`https://api.stepfun.com/v1` · env=`STEPFUN_API_KEY`
- **变体**：stepfun_ai_step_plan、stepfun_step_plan

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | **`max_tokens`**（官方参数表；无 max_completion_tokens；默认 INF 由模型决定） | `{"model":"step-3.5-flash","messages":[...],"max_tokens":2048}` | C | https://platform.stepfun.ai/docs/en/api-reference/chat/chat-completion-create |
| 能力支持 | `tools`/`tool_choice`/`response_format`（json_object）/`frequency_penalty`/`stop`/`n` 均支持 | - | C | 同上 |
| 思考机制 | **`reasoning_format`**：`general`（默认，返回 `reasoning` 字段）/ `deepseek-style`（返回 `reasoning_content`，DeepSeek 兼容）；**`reasoning_effort`**：low/medium/high（step-3.5-flash-2603 仅 low/high） | `{"reasoning_format":"deepseek-style","reasoning_effort":"high"}` → 响应 `choices[0].message.reasoning_content`；默认 `general` 时 `message.reasoning` | C | https://platform.stepfun.ai/docs/en/api-reference/chat/chat-completion-create；https://platform.stepfun.ai/docs/en/guides/developer/reasoning |
| 流式/usage | 无差异（标准 `chat.completion.chunk` SSE；`usage` 含可选 `cached_tokens`） | - | C | https://platform.stepfun.ai/docs/en/api-reference/chat/chat-completion-create |
| 消息格式 | 思考字段名是 `reasoning`（非 reasoning_content，除非 reasoning_format=deepseek-style）；多模态 content 类型含 `image_url`、`video_url`、`input_audio`（音频 base64） | 流式 `delta.reasoning`；`{"type":"video_url","video_url":{"url":"https://.../a.mp4"}}` | C | https://platform.stepfun.ai/docs/en/guides/developer/reasoning；https://platform.stepfun.ai/docs/en/api-reference/chat/chat-completion-create |
| 特殊字段 | `reasoning_format`（general/deepseek-style）为平台特有能力字段 | 见上 | C | https://platform.stepfun.ai/docs/en/api-reference/chat/chat-completion-create |
| headers/认证 | 无差异（Bearer，`STEPFUN_API_KEY`） | - | C | https://platform.stepfun.ai/docs/en/guides/developer/reasoning |
| URL/端点 | 无差异（China `https://api.stepfun.com/v1`、International `https://api.stepfun.ai/v1`，与 registry 一致；Step Plan 变体加 `/step_plan` 前缀） | `POST https://api.stepfun.com/v1/chat/completions` | C | https://platform.stepfun.ai/docs/en/api-reference/chat/chat-completion-create |
| 模型 ID | 无差异（`step-3.7-flash`、`step-3.5-flash` 等） | - | C | 同上 |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:1853-1861`；`convert.rs:1319-1324`（`reasoning_format` 仅 provider=="groq" 时发送）；`model.rs:558-563`（`reasoning`/`reasoning_content` 双键解析 ✅）
- **差距说明**：① `reasoning_format` 只有 groq 分支（convert.rs:1319-1324），stepfun 无法发送 `deepseek-style` → 默认 `general` 时返回 `reasoning` 字段（aimux 已解析 ✅）；② 推理模型分支 `max_completion_tokens` vs StepFun `max_tokens`（convert.rs:1122-1130）；③ `video_url`/`input_audio` 内容类型无转换支持；④ `reasoning_effort` 透传 ✅（值 low/medium/high 与 aimux 通用取值兼容）。
- **建议动作**：`reasoning_format` 发送条件从硬编码 groq 改为 profile 字段（或 bodyOverrides）；`max_tokens` 命名纳入候选；多模态 video/audio 输入留待消息层扩展。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证（官方 API 参考，含完整参数表与示例）
- **存疑标记**：无

### stepfun_ai_step_plan — StepFun Step Plan (Global)

- **registry 现状**：profile=`full()` · base_url=`https://api.stepfun.ai/step_plan/v1` · env=`STEPFUN_API_KEY`
- **变体**：stepfun、stepfun_step_plan

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 同 stepfun 主条目（`max_tokens`） | - | C | https://platform.stepfun.ai/docs/en/api-reference/chat/chat-completion-create |
| 能力支持 | 同 stepfun | - | - | - |
| 思考机制 | 同 stepfun（`reasoning_format`/`reasoning_effort`） | - | C | 同上 |
| 流式/usage | 同 stepfun | - | - | - |
| 消息格式 | 同 stepfun（`reasoning` 字段、video/audio 内容） | - | - | - |
| 特殊字段 | 同 stepfun | - | - | - |
| headers/认证 | 同 stepfun（Bearer） | - | - | - |
| URL/端点 | 无差异（Step Plan 订阅端点：Global `https://api.stepfun.ai/step_plan/v1`，registry 一致；Step Plan 是计费套餐前缀而非不同协议） | `POST https://api.stepfun.ai/step_plan/v1/chat/completions` | C | https://github.com/stepfun-ai/Step-3.5-Flash（Region 表） |
| 模型 ID | 同 stepfun | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖（同 stepfun 主条目）
- **aimux 代码位置**：`openai_compat_registry.rs:1862-1870`
- **差距说明**：差异全部继承自 stepfun 主条目（reasoning_format / max_tokens / 多模态内容）。
- **建议动作**：同 stepfun。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：无

### stepfun_step_plan — StepFun Step Plan (China)

- **registry 现状**：profile=`full()` · base_url=`https://api.stepfun.com/step_plan/v1` · env=`STEPFUN_API_KEY`
- **变体**：stepfun、stepfun_ai_step_plan

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 同 stepfun 主条目 | - | C | https://platform.stepfun.ai/docs/en/api-reference/chat/chat-completion-create |
| 能力支持 | 同 stepfun | - | - | - |
| 思考机制 | 同 stepfun | - | - | - |
| 流式/usage | 同 stepfun | - | - | - |
| 消息格式 | 同 stepfun | - | - | - |
| 特殊字段 | 同 stepfun | - | - | - |
| headers/认证 | 同 stepfun | - | - | - |
| URL/端点 | 无差异（China Step Plan：`https://api.stepfun.com/step_plan/v1`，registry 一致） | `POST https://api.stepfun.com/step_plan/v1/chat/completions` | C | https://github.com/stepfun-ai/Step-3.5-Flash（Region 表） |
| 模型 ID | 同 stepfun | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖（同 stepfun 主条目）
- **aimux 代码位置**：`openai_compat_registry.rs:1871-1879`
- **差距说明**：差异全部继承自 stepfun 主条目。
- **建议动作**：同 stepfun。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：无

### subconscious — Subconscious

- **registry 现状**：profile=`full()` · base_url=`https://api.subconscious.dev/v1` · env=`SUBCONSCIOUS_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（OpenAI SDK 直连） | - | C | https://docs.subconscious.dev/quickstart |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（`api_key` + base_url） | - | C | https://docs.subconscious.dev/quickstart |
| URL/端点 | 无差异（base `https://api.subconscious.dev/v1` 与 registry 一致） | - | C | https://www.subconscious.dev/ |
| 模型 ID | 无差异（如 `subconscious/tim-qwen3`，`/` 前缀为文档惯例） | `"model":"subconscious/tim-qwen3"` | C | https://github.com/subconscious-systems/TIMRUN |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:1880-1888`
- **差距说明**：无 request 级特殊配置。
- **建议动作**：无需动作。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证（仅文档引用）
- **存疑标记**：无

### submodel — SubModel

- **registry 现状**：profile=`full()` · base_url=`https://api.submodel.com/v1` · env=`SUBMODEL_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | ⚠️ 无证据 | - | ⚠️ | - |
| 能力支持 | ⚠️ 无证据 | - | ⚠️ | - |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | ⚠️ 无证据 | - | ⚠️ | - |
| URL/端点 | ⚠️ 未找到 `api.submodel.com` 官方 API 文档 | - | ⚠️ | - |
| 模型 ID | ⚠️ 无证据 | - | ⚠️ | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖（无证据显示差异）
- **aimux 代码位置**：`openai_compat_registry.rs:1889-1897`
- **差距说明**：查不到公开信息。
- **建议动作**：无需动作；标记为需人工确认厂商。

#### 3. 证据与验证

- **证据等级**：⚠️
- **验证状态**：🔲 未验证
- **存疑标记**：⚠️ 证据不足（查不到任何信息）

### synthetic — Synthetic

- **registry 现状**：profile=`full()` · base_url=`https://api.synthetic.new/openai/v1` · env=`SYNTHETIC_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（支持全部标准 OpenAI 参数，litellm 确认） | - | C | https://docs.litellm.ai/docs/providers/synthetic |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer） | - | C | https://dev.synthetic.new/docs/api/getting-started |
| URL/端点 | 无差异（base `https://api.synthetic.new/openai/v1`，路径含 `/openai` 前缀，registry 一致） | `POST https://api.synthetic.new/openai/v1/chat/completions` | C | https://dev.synthetic.new/ |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:1898-1906`
- **差距说明**：无 request 级特殊配置。
- **建议动作**：无需动作。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证（官方 dev 文档 + litellm provider 页）
- **存疑标记**：无

## 存疑归档

> 下列条目因证据不足或域名存疑被标记 ⚠️，不计入内置差距清单（按 README 方法论）。

| 厂商 | 存疑点 |
|------|--------|
| opencode | registry base_url 为非法字面量 `"opencode_zen.rs"`（确定性 bug，见 `openai_compat_registry.rs:1533`） |
| orcarouter | 官方域 orcarouter.ai，registry 用 api.orcarouter.com，未见官方使用 |
| petals | 查不到 api.petals.dev 托管服务的公开文档，端点真实性存疑 |
| pinstripes | 官方示例 api.pinstripes.ai，registry 用 api.pinstripes.io |
| predibase | 官方 OpenAPI base 为 `{tenant}/deployments/v2/llms/{model}` 形态，registry 用 serving.app.predibase.com/v1 |
| requesty | 官方示例 router.requesty.ai，registry 用 api.requesty.ai |
| qiniu_ai | 官方 FAQ 示例 api.qnaigc.com，registry 用 api.qiniu.com |
| qihoo360 | 无官方文档可核对（仅第三方汇总），认证/思考字段未确认 |
| perfxcloud | 无官方文档，思考机制未确认 |
| reve / submodel / privatemode_ai | 查不到任何公开信息 |
