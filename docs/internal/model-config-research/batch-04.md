# Batch 04 — Model Request Config 调研

> 状态: ✅ 已完成调研（42/42） · 厂商数: 42
> 模板见 [_template.md](_template.md) · 方法论见 [README.md](README.md)
> 证据硬性要求: 每条目必须有例子或证明(A/B/C 级),禁止编造来源。
> 完成日期: 2026-08-01 · 调研批次: batch-04（llmgateway → openaisb，网关/聚合/代理厂商为主）

## 厂商清单

| # | id | display | base_url | env_var | profile |
|---|---|---|---|---|---|
| 1 | llmgateway | LLM Gateway | https://api.llmgateway.io/v1 | LLM_GATEWAY_API_KEY | OpenAICompatProfile::full() |
| 2 | llmtr | LLMTR | https://llmtr.com/v1 | LLMTR_API_KEY | OpenAICompatProfile::full() |
| 3 | longcat | LongCat | https://api.longcat.chat/v1 | LONGCAT_API_KEY | OpenAICompatProfile::full() |
| 4 | lucidquery | LucidQuery | https://api.lucidquery.com/v1 | LUCIDQUERY_API_KEY | OpenAICompatProfile::full() |
| 5 | lynkr | Lynkr | http://localhost:8081/v1 | LYNKR_API_KEY | OpenAICompatProfile::full() |
| 6 | matterai | Matter AI | https://api.matterai.com/v1 | MATTERAI_API_KEY | OpenAICompatProfile::full() |
| 7 | meganova | Meganova | https://api.meganova.ai/v1 | MEGANOVA_API_KEY | OpenAICompatProfile::full() |
| 8 | merge_gateway | Merge Gateway | https://api-gateway.merge.dev/v1/openai | MERGE_GATEWAY_API_KEY | OpenAICompatProfile::full() |
| 9 | meta | Meta | https://api.meta.ai/v1 | MODEL_API_KEY | OpenAICompatProfile::full() |
| 10 | meta_llama | Meta Llama API | https://api.llama.com/compat/v1 | LLAMA_API_KEY | OpenAICompatProfile::full() |
| 11 | mimo | Mimo | https://api.xiaomimimo.com/v1 | MIMO_API_KEY | OpenAICompatProfile::full() |
| 12 | minimax | MiniMax | https://api.minimax.io/v1 | MINIMAX_API_KEY | OpenAICompatProfile::full() |
| 13 | minimax_cn | MiniMax (minimaxi.com) | https://api.minimaxi.com/v1 | MINIMAX_API_KEY | OpenAICompatProfile::full() |
| 14 | minimax_cn_coding_plan | MiniMax Token Plan (minimaxi.com) | https://api.minimaxi.com/v1 | MINIMAX_API_KEY | OpenAICompatProfile::full() |
| 15 | minimax_coding_plan | MiniMax Token Plan (minimax.io) | https://api.minimax.io/anthropic/v1 | MINIMAX_API_KEY | OpenAICompatProfile::full() |
| 16 | mira | Mira | https://api.mira.so/v1 | MIRA_API_KEY | OpenAICompatProfile::full() |
| 17 | mixlayer | Mixlayer | https://models.mixlayer.ai/v1 | MIXLAYER_API_KEY | OpenAICompatProfile::full() |
| 18 | moark | Moark | https://api.moark.com/v1 | MOARK_API_KEY | OpenAICompatProfile::full() |
| 19 | modal | Modal | https://modal.com/v1 | MODAL_API_KEY | OpenAICompatProfile::full() |
| 20 | model_oracle_ai | Model Oracle AI | https://api.modeloracle.com/api/v1 | MODEL_ORACLE_API_KEY | OpenAICompatProfile::full() |
| 21 | modelscope | ModelScope | https://api-inference.modelscope.cn/v1 | MODELSCOPE_API_KEY | OpenAICompatProfile::full() |
| 22 | moonshotai | Moonshot AI | https://api.moonshot.cn/v1 | MOONSHOT_API_KEY | OpenAICompatProfile::full() |
| 23 | moonshotai_cn | Moonshot AI (China) | https://api.moonshot.cn/anthropic/v1锛圓nthropic | MOONSHOT_API_KEY | OpenAICompatProfile::full() |
| 24 | morph | Morph LLM | https://api.morphllm.com/v1 | MORPH_API_KEY | OpenAICompatProfile::full() |
| 25 | nanogpt | NanoGPT | https://api.nanogpt.com/v1 | NANOGPT_API_KEY | OpenAICompatProfile::full() |
| 26 | ncompass | Ncompass | https://api.ncompass.tech/v1 | NCOMPASS_API_KEY | OpenAICompatProfile::full() |
| 27 | nearai | NEAR AI Cloud | https://cloud-api.near.ai/v1 | NEARAI_API_KEY | OpenAICompatProfile::full() |
| 28 | nebius | Nebius AI | https://api.studio.nebius.ai/v1 | NEBIUS_API_KEY | OpenAICompatProfile::full() |
| 29 | neon | Neon | https://<branch-host>/v1 | NEON_AI_GATEWAY_TOKEN | OpenAICompatProfile::full() |
| 30 | neuralwatt | Neuralwatt | https://api.neuralwatt.com/v1 | NEURALWATT_API_KEY | OpenAICompatProfile::full() |
| 31 | nextbit | NextBit | https://api.nextbit.ai/v1 | NEXTBIT_API_KEY | OpenAICompatProfile::full() |
| 32 | nlp_cloud | NLP Cloud | https://api.nlpcloud.io/v1 | NLPCLOUD_API_KEY | OpenAICompatProfile::full() |
| 33 | nous_research | Nous Research | https://api.nousresearch.com/v1 | NOUS_API_KEY | OpenAICompatProfile::full() |
| 34 | novita | Novita AI | https://api.novita.ai/v1 | NOVITA_API_KEY | OpenAICompatProfile::full() |
| 35 | nscale | Nscale | https://inference.api.nscale.com/v1 | NSCALE_API_KEY | OpenAICompatProfile::full() |
| 36 | nvidia_nim | NVIDIA NIM | https://integrate.api.nvidia.com/v1 | NVIDIA_API_KEY | OpenAICompatProfile::full() |
| 37 | oci | OCI | https://inference.generativeai.${region}.oci.oraclecloud.com/openai/v1 | OCI_API_KEY | OpenAICompatProfile::full() |
| 38 | ofox | OfoxAI | https://api.ofox.ai/v1 | OFOX_API_KEY | OpenAICompatProfile::full() |
| 39 | ohmygpt | OhMyGPT | https://api.ohmygpt.com/v1 | OHMYGPT_API_KEY | OpenAICompatProfile::full() |
| 40 | ollama_cloud | Ollama Cloud | https://api.ollama.com/v1 | OLLAMA_CLOUD_API_KEY | OpenAICompatProfile::full() |
| 41 | openaimax | OpenAIMax | https://api.openaimax.com/v1 | OPENAIMAX_API_KEY | OpenAICompatProfile::full() |
| 42 | openaisb | OpenAI-SB | https://api.openaisb.com/v1 | OPENAISB_API_KEY | OpenAICompatProfile::full() |

---

### llmgateway — LLM Gateway

- **registry 现状**: profile=`full()` · base_url=`https://api.llmgateway.io/v1` · env=`LLM_GATEWAY_API_KEY`
- **变体**: 无（llmgateway 同厂还有 llamagate/openai_router 等不属本批）

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（顶层 `max_tokens`/`max_completion_tokens` 均透传） | - | - | - |
| 能力支持 | 无差异（tools/json 等标准） | - | - | - |
| 思考机制 | 支持两套：顶层 `reasoning_effort`（`none/minimal/low/medium/high/xhigh/max` 7 档）或统一 `reasoning` 对象（`effort` + `max_tokens` 预算）；二者不可混用；`reasoning.max_tokens` 覆盖 effort | `{"model":"gpt-oss-120b","messages":[...],"reasoning_effort":"medium"}` / `{"model":"anthropic/claude-sonnet-4-20250514","reasoning":{"max_tokens":8000}}` | B | reference/llmgateway/apps/docs/content/features/reasoning.mdx:55-106 |
| 流式/usage | 响应 usage 含 `reasoning_tokens`；流式 delta 携带 `delta.reasoning`（思考内容字段名不是 `reasoning_content`） | `"message":{"content":"...","reasoning":"First, I need to..."}`；`"usage":{"prompt_tokens":20,"completion_tokens":45,"reasoning_tokens":35}` | B | reasoning.mdx:112-136, 267-286 |
| 消息格式 | 非流式思考内容在 `message.reasoning`（非 `reasoning_content`） | 见上 | B | reasoning.mdx:124 |
| 特殊字段 | `verbosity`（`low/medium/high`，GPT-5 系列）；缓存控制头 `x-no-cache: true` 可绕过网关缓存 | `{"model":"gpt-5","messages":[...],"verbosity":"low"}` | B | reasoning.mdx:207-221；features/anthropic-endpoint.mdx:183 |
| headers/认证 | `Authorization: Bearer $LLM_GATEWAY_API_KEY`（标准）；响应头 `x-llmgateway-cache: HIT` | `curl -H "Authorization: Bearer $LLM_GATEWAY_API_KEY"` | B | apps/docs/content/overview.mdx:30-32 |
| URL/端点 | 无差异（`https://api.llmgateway.io/v1` 与 registry 一致） | - | B | quick-start.mdx:12 |
| 模型 ID | 厂商前缀模型 ID：`openai/gpt-oss-20b`、`anthropic/claude-...`、自定义 `<provider>/<model>`（如 `internal-vllm/llama-4-maverick`）；auto-routing 可用不带前缀的 `gpt-5` | `"model":"anthropic/claude-sonnet-4-20250514"` | B | reasoning.mdx:156,274；learn/models.mdx:50 |

#### 2. aimux 现状对比

- **对比结论**: 🔶 部分覆盖
- **aimux 代码位置**: `openai/convert.rs:1326-1329`（reasoning_effort 发送）、`convert.rs:1284-1317`（白名单）、`openai_compat_registry.rs:1151`
- **差距说明**: ① llmgateway 的 `reasoning` 对象（effort+max_tokens）与 `max`/`xhigh` 档位 aimux 未覆盖；② `verbosity` 不在白名单（aimux 只有 `textVerbosity`→`verbosity` 映射，见 convert.rs:1294-1296，字段名恰好一致但触发条件不同）；③ 响应侧 `message.reasoning` 别名未解析。
- **建议动作**: 无需改 profile；`reasoning.max_tokens` 可经 bodyOverrides 兜底；后续若做 reasoningMap 可把 llmgateway 列为"接受 OpenAI effort 全套档位"的厂商；补测试。

#### 3. 证据与验证

- **证据等级**: B
- **验证状态**: 🔲 未验证(仅 reference 文档引用)
- **存疑标记**: 无

---

### llmtr — LLMTR

- **registry 现状**: profile=`full()` · base_url=`https://llmtr.com/v1` · env=`LLMTR_API_KEY`
- **变体**: -

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异（标准 Chat Completions + streaming） | - | - | - |
| 思考机制 | 无差异（透传，按模型而定） | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（OpenAI SDK 标准 `base_url` + api key） | - | C | https://llmtr.com/?lang=en |
| URL/端点 | 无差异（`https://llmtr.com/v1`，土耳其 OpenAI 兼容网关） | - | C | https://llmtr.com/?lang=en；https://github.com/cline/cline/discussions/11492 |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖（纯透传网关）
- **aimux 代码位置**: `openai_compat_registry.rs:1160`
- **差距说明**: 官方声明"standard OpenAI Chat Completions"，无需特殊处理。
- **建议动作**: 无需动作。

#### 3. 证据与验证

- **证据等级**: C
- **验证状态**: 🔲 未验证
- **存疑标记**: 无

---

### longcat — LongCat

- **registry 现状**: profile=`full()` · base_url=`https://api.longcat.chat/v1` · env=`LONGCAT_API_KEY`
- **变体**: -

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 无差异（透传） | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer） | - | C | https://anx.anxcye.com/docs/en/ai/longcat |
| URL/端点 | ⚠️ 社区/文档实际使用的 OpenAI 兼容路径为 `https://api.longcat.chat/openai/v1`（SillyTavern 接入要求保留 `/openai` 前缀），registry 的 `/v1` 可能是原生端点 | `base_url = "https://api.longcat.chat/openai/v1"` | C | https://www.reddit.com/r/SillyTavernAI/comments/1pi2abh/how_do_i_use_longcat_api_it_never_connects/；https://anx.anxcye.com/docs/en/ai/longcat |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: ⚠️ 不一致（base_url 可能错误）
- **aimux 代码位置**: `openai_compat_registry.rs:1170`
- **差距说明**: 若 `/openai/v1` 才是 OpenAI 兼容前缀，registry 的 `https://api.longcat.chat/v1` 会 404/格式不符。⚠️ 存疑（两条社区来源指向 `/openai/v1`，官方文档未能直接访问验证）。
- **建议动作**: 用真实 API key 验证后修正 base_url；补 A 级测试。

#### 3. 证据与验证

- **证据等级**: C
- **验证状态**: 🔲 未验证
- **存疑标记**: ⚠️ base_url 前缀未确证（社区来源，官方文档需登录）

---

### lucidquery — LucidQuery

- **registry 现状**: profile=`full()` · base_url=`https://api.lucidquery.com/v1` · env=`LUCIDQUERY_API_KEY`
- **变体**: -

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（dashboard 生成 key，SDK 直插） | - | C | https://lucidquery.com/api |
| URL/端点 | 无差异（官方文档：`api.lucidquery.com/v1`） | - | C | https://lucidquery.com/api |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖
- **aimux 代码位置**: `openai_compat_registry.rs:1178`
- **差距说明**: 官方文档即"Set the base URL to api.lucidquery.com/v1"，标准 OpenAI 兼容。
- **建议动作**: 无需动作。

#### 3. 证据与验证

- **证据等级**: C
- **验证状态**: 🔲 未验证
- **存疑标记**: 无（文档信息少，仅 URL/认证确认）

---

### lynkr — Lynkr

- **registry 现状**: profile=`full()` · base_url=`http://localhost:8081/v1` · env=`LYNKR_API_KEY`
- **变体**: -

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 证据不足 | - | - | - |
| 能力支持 | 证据不足 | - | - | - |
| 思考机制 | 证据不足 | - | - | - |
| 流式/usage | 证据不足 | - | - | - |
| 消息格式 | 证据不足 | - | - | - |
| 特殊字段 | 证据不足 | - | - | - |
| headers/认证 | 证据不足 | - | - | - |
| URL/端点 | ⚠️ base_url 为 localhost（本地代理/自托管网关），registry 值与实际部署强绑定 | - | ⚠️ | 无公开文档可查 |
| 模型 ID | 证据不足 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: ⚠️ 证据不足（无法对比）
- **aimux 代码位置**: `openai_compat_registry.rs:1188`
- **差距说明**: 未检索到 lynkr 的任何公开 API 文档；localhost base_url 表明是本地自托管服务。
- **建议动作**: 保持 full()；待有真实接入案例再核对。

#### 3. 证据与验证

- **证据等级**: D（无公开来源）→ 计入存疑归档
- **验证状态**: 🔲 未验证
- **存疑标记**: ⚠️ 存疑

---

### matterai — Matter AI

- **registry 现状**: profile=`full()` · base_url=`https://api.matterai.com/v1` · env=`MATTERAI_API_KEY`
- **变体**: -

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 证据不足 | - | - | - |
| 能力支持 | 证据不足 | - | - | - |
| 思考机制 | 证据不足 | - | - | - |
| 流式/usage | 证据不足 | - | - | - |
| 消息格式 | 证据不足 | - | - | - |
| 特殊字段 | 证据不足 | - | - | - |
| headers/认证 | 证据不足 | - | - | - |
| URL/端点 | 证据不足（api.matterai.com 存在但未检索到 API 文档） | - | - | - |
| 模型 ID | 证据不足 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: ⚠️ 证据不足
- **aimux 代码位置**: `openai_compat_registry.rs:1196`
- **差距说明**: 未检索到官方 API 文档，无法确认是否纯 OpenAI 兼容。
- **建议动作**: 保持 full()；待有真实接入案例再核对。

#### 3. 证据与验证

- **证据等级**: D（无公开来源）→ 计入存疑归档
- **验证状态**: 🔲 未验证
- **存疑标记**: ⚠️ 存疑

---

### meganova — Meganova

- **registry 现状**: profile=`full()` · base_url=`https://api.meganova.ai/v1` · env=`MEGANOVA_API_KEY`
- **变体**: -

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（标准 OpenAI 兼容） | - | C | https://docs.meganova.ai/api-reference |
| URL/端点 | 无差异（官方："The Inference API is OpenAI compatible and available at https://api.meganova.ai/v1"） | - | C | https://docs.meganova.ai/inference-models |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖
- **aimux 代码位置**: `openai_compat_registry.rs:1205`
- **差距说明**: 官方文档明确 OpenAI 兼容 + 预配置端点。
- **建议动作**: 无需动作。

#### 3. 证据与验证

- **证据等级**: C
- **验证状态**: 🔲 未验证
- **存疑标记**: 无

---

### merge_gateway — Merge Gateway

- **registry 现状**: profile=`full()` · base_url=`https://api-gateway.merge.dev/v1/openai` · env=`MERGE_GATEWAY_API_KEY`
- **变体**: -

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异（标准 chat.completions） | - | - | - |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（`api_key` 经 OpenAI SDK 传） | - | C | https://docs.merge.dev/merge-gateway/get-started |
| URL/端点 | 无差异（官方示例 `base_url="https://api-gateway.merge.dev/v1/openai"`，与 registry 一致） | `client = OpenAI(api_key="...", base_url="https://api-gateway.merge.dev/v1/openai")` | C | https://docs.merge.dev/merge-gateway/get-started |
| 模型 ID | 无差异（官方："No provider prefix needed on the model name"） | - | C | https://www.merge.dev/gateway/fugu-ultra-api |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖
- **aimux 代码位置**: `openai_compat_registry.rs:1214`
- **差距说明**: 官方明确标准 OpenAI 兼容、模型名无需前缀，与 full() 无冲突。
- **建议动作**: 无需动作。

#### 3. 证据与验证

- **证据等级**: C
- **验证状态**: 🔲 未验证
- **存疑标记**: 无

---

### meta — Meta

- **registry 现状**: profile=`full()` · base_url=`https://api.meta.ai/v1` · env=`MODEL_API_KEY`
- **变体**: -

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 无差异（muse-spark 系列内部推理，未暴露开关） | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（`Bearer token (MODEL_API_KEY)`） | `Authorization: Bearer $MODEL_API_KEY` | C | https://ai.developer.meta.com/docs/overview/ |
| URL/端点 | 无差异（官方 Base URL `https://api.meta.ai/v1`，与 registry 一致） | - | C | https://ai.developer.meta.com/docs/overview/；https://dev.meta.ai/docs/getting-started/overview/ |
| 模型 ID | 官方唯一模型 `muse-spark-1.1`（1M 上下文） | `"model": "muse-spark-1.1"` | C | https://ai.developer.meta.com/docs/overview/ |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖
- **aimux 代码位置**: `openai_compat_registry.rs:1223`
- **差距说明**: Meta Model API 明确 OpenAI 兼容（"point the OpenAI Python SDK or any OpenAI-compatible client at the Model API base URL"）。
- **建议动作**: 无需动作；模型清单可在 registry 注释中补 `muse-spark-1.1`。

#### 3. 证据与验证

- **证据等级**: C
- **验证状态**: 🔲 未验证
- **存疑标记**: 无

---

### meta_llama — Meta Llama API

- **registry 现状**: profile=`full()` · base_url=`https://api.llama.com/compat/v1` · env=`LLAMA_API_KEY`
- **变体**: -

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异（tools/json 标准；Llama 4 支持图像输入） | - | - | - |
| 思考机制 | 无差异（无专有 thinking 参数） | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 多模态：Llama 4 原生支持 text+image 输入（OpenAI 多模态格式） | `{"role":"user","content":[{"type":"text","text":"..."},{"type":"image_url","image_url":{"url":"..."}}]}` | C | https://llama.developer.meta.com/docs/models |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（`Authorization: Bearer <LLAMA_API_KEY>`） | `client = OpenAI(base_url="https://api.llama.com/compat/v1/", api_key=...)` | C | https://michaelsolati.com/blog/metas-llama-api-open-models-meet-developer-convenience |
| URL/端点 | 无差异（`/compat/v1` 前缀与 registry 一致） | - | C | 同上 |
| 模型 ID | 官方模型 ID 无前缀：`Llama-3.3-70B-Instruct`、`Llama-4-Scout-17B-16E-Instruct-FP8`、`Llama-4-Maverick-17B-128E-Instruct-FP8` 等 | `"model":"Llama-3.3-70B-Instruct"` | C | https://llama.developer.meta.com/docs/models |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖（多模态输入 aimux 已支持 OpenAI image_url 格式）
- **aimux 代码位置**: `openai_compat_registry.rs:1232`、`openai/convert.rs`（messages 转换）
- **差距说明**: 模型名透传无差异；无专有参数。
- **建议动作**: 无需动作。

#### 3. 证据与验证

- **证据等级**: C
- **验证状态**: 🔲 未验证
- **存疑标记**: 无

---

### mimo — Mimo

- **registry 现状**: profile=`full()` · base_url=`https://api.xiaomimimo.com/v1` · env=`MIMO_API_KEY`
- **变体**: -

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异（image_url 等标准多模态格式） | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（API Key 格式 `sk-xxxxx`，Bearer） | `Authorization: Bearer sk-...` | C | https://mimo.mi.com/docs/en-US/api/chat/openai-api |
| URL/端点 | 无差异（官方 Request Address `https://api.xiaomimimo.com/v1/chat/completions`；另有 `/anthropic` 协议端点） | - | C | https://mimo.mi.com/docs/en-US/api/chat/openai-api |
| 模型 ID | 无差异（`mimo-*` 系列，如 mimo-v2-tts；模型名透传） | - | C | https://github.com/QuantumNous/new-api/issues/3353 |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖
- **aimux 代码位置**: `openai_compat_registry.rs:1242`
- **差距说明**: 官方明确 "compatible with the OpenAI Chat Completions API"。
- **建议动作**: 无需动作。

#### 3. 证据与验证

- **证据等级**: C
- **验证状态**: 🔲 未验证
- **存疑标记**: 无

---

### minimax — MiniMax

- **registry 现状**: profile=`full()` · base_url=`https://api.minimax.io/v1` · env=`MINIMAX_API_KEY`
- **变体**: minimax_cn（api.minimaxi.com 域名变体）、minimax_cn_coding_plan、minimax_coding_plan（见下条）

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | `max_completion_tokens` 为推荐参数；`max_tokens` 标记 **deprecated**（仍接受）；`temperature` 范围 `[0,2]` 默认 1 | `{"model":"MiniMax-M3","max_completion_tokens":500,"messages":[...]}` | C | https://platform.minimax.io/docs/api-reference/text-chat-openai |
| 能力支持 | ⚠️ `presence_penalty`/`frequency_penalty`/`logit_bias` 等 OpenAI 参数**被静默忽略**；`n` 仅支持 1；`function_call`(deprecated) 不支持 | - | C | https://platform.minimax.io/docs/api-reference/text-openai-api（Important Notes: "Some OpenAI parameters (such as presence_penalty, frequency_penalty, logit_bias, etc.) will be ignored"） |
| 思考机制 | 顶层 `thinking` 对象：M3 支持 `{"type":"adaptive"}`（默认开，等价 thinking on）与 `{"type":"disabled"}`；**M2.x 思考不可关闭**（传 disabled 仍思考）；`reasoning_split` 控制输出切分（不控制开关） | `{"model":"MiniMax-M3","thinking":{"type":"adaptive"}}` / `{"reasoning_split":true}` | C | https://platform.minimax.io/docs/api-reference/text-openai-api（Thinking Control 节） |
| 流式/usage | 无差异（`stream_options` 支持）；usage 含 `prompt_tokens_details.cached_tokens` | `"usage":{"total_tokens":1659,"prompt_tokens":1366,"completion_tokens":293,"prompt_tokens_details":{"cached_tokens":114}}` | C | https://platform.minimax.io/docs/api-reference/text-chat-openai |
| 消息格式 | `reasoning_split=false` 时思考内容直接嵌入 `content` 的 `<think>...</think>` 标签中；`true` 时输出到 `reasoning_content`/`reasoning_details`（非标准字段）；响应 message 含 `name:"MiniMax AI"`、`audio_content` | `"message":{"content":"<think>...推理...</think>最终回答","role":"assistant","name":"MiniMax AI"}` | C | https://platform.minimax.io/docs/api-reference/text-chat-openai（Image Understanding 示例响应） |
| 特殊字段 | `service_tier`: `standard`（默认）/`priority`（1.5 倍价、优先准入）；`input_sensitive`/`output_sensitive` 等响应字段 | `{"model":"MiniMax-M3","service_tier":"priority"}` | C | https://platform.minimax.io/docs/api-reference/text-chat-openai |
| headers/认证 | 无差异（`Authorization: Bearer <token>`） | - | C | https://platform.minimax.io/docs/api-reference/text-chat-openai |
| URL/端点 | 无差异（`https://api.minimax.io/v1/chat/completions`） | - | C | 同上 |
| 模型 ID | 无差异（`MiniMax-M3`、`MiniMax-M2.7`、`MiniMax-M2.7-highspeed` 等，无前缀） | `"model":"MiniMax-M3"` | C | 同上 |

#### 2. aimux 现状对比

- **对比结论**: 🔶 部分覆盖（thinking 机制 ❌ 未覆盖）
- **aimux 代码位置**: `openai/convert.rs:1326-1329`（reasoning_effort）、`convert.rs:1118-1138`（max_tokens/max_completion_tokens）、`convert.rs:1331-1371`（service_tier）、`openai_compat_registry.rs:1250`
- **差距说明**: ① MiniMax 思考用顶层 `thinking:{type:...}`（与 deepseek profile 的 `{type:enabled/disabled}` 同构但取值多 `adaptive`，且 M2.x 不可关），aimux 仅 deepseek 走 override，minimax 不会发 thinking；② `reasoning_split` 未覆盖（❌）；③ service_tier=`priority` 会被 aimux 的 `supports_priority_processing` 能力校验拦截丢弃（⚠️ 不一致，应透传）；④ presence/frequency_penalty 会被忽略——aimux 默认发送但无报错风险（⚠️ 行为差异）；⑤ `max_completion_tokens` 当模型被识别为 reasoning 时 ✅ 已覆盖。
- **建议动作**: 为 minimax 加 profile 变体（bodyOverrides 兜底 `thinking`/`reasoning_split`）；service_tier 校验改为"已知白名单厂商透传"或增加 profile 字段；补测试。

#### 3. 证据与验证

- **证据等级**: C
- **验证状态**: 🔲 未验证
- **存疑标记**: 能力支持类"ignored 参数"为官方文档明示，无存疑

---

### minimax_cn — MiniMax (minimaxi.com)

- **registry 现状**: profile=`full()` · base_url=`https://api.minimaxi.com/v1` · env=`MINIMAX_API_KEY`
- **变体**: minimax_cn_coding_plan（同域 Token Plan 变体）

#### 1. request 差异发现

与 minimax（api.minimax.io）完全一致的 API 面（同一平台文档 `platform.minimax.io`/`platform.minimaxi.com`），仅域名不同：

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 同 minimax：`max_completion_tokens` 推荐、`max_tokens` deprecated | `{"model":"MiniMax-M3","max_completion_tokens":500}` | C | https://platform.minimax.io/docs/api-reference/text-chat-openai |
| 能力支持 | 同 minimax：penalty/logit_bias 被忽略、n=1 | - | C | 同上 |
| 思考机制 | 同 minimax：`thinking:{type:"adaptive"/"disabled"}`、M2.x 不可关 | `{"thinking":{"type":"adaptive"}}` | C | 同上 |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 同 minimax：`<think>...</think>` / `reasoning_split` | - | C | 同上 |
| 特殊字段 | 同 minimax：`service_tier` standard/priority | - | C | 同上 |
| headers/认证 | 无差异（Bearer） | - | - | - |
| URL/端点 | 域名变体：`https://api.minimaxi.com/v1`（registry 已用） | - | C | https://platform.minimax.io/docs/api-reference/text-chat-openai（api.minimax.io 官方示例；minimaxi.com 为国内域名变体） |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: 🔶 部分覆盖（同 minimax）
- **aimux 代码位置**: `openai_compat_registry.rs:1259`
- **差距说明**: 与 minimax 完全相同（thinking/reasoning_split/service_tier 校验）。
- **建议动作**: 与 minimax 一并处理（同一 profile 变体）。

#### 3. 证据与验证

- **证据等级**: C
- **验证状态**: 🔲 未验证
- **存疑标记**: minimaxi.com 域名本身未见官方示例 URL，为域名变体推断（与 minimax.io 同文档平台）

---

### minimax_cn_coding_plan — MiniMax Token Plan (minimaxi.com)

- **registry 现状**: profile=`full()` · base_url=`https://api.minimaxi.com/v1` · env=`MINIMAX_API_KEY`
- **变体**: minimax_cn 的 Token Plan（套餐）变体

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 同 minimax | - | C | 同 minimax 来源 |
| 能力支持 | 同 minimax | - | C | 同上 |
| 思考机制 | 同 minimax（Token Plan 面向 M2/M3 套餐，thinking 行为一致） | - | C | 同上 |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 同 minimax | - | C | 同上 |
| 特殊字段 | 同 minimax（service_tier） | - | C | 同上 |
| headers/认证 | 无差异（Bearer） | - | - | - |
| URL/端点 | 同 minimax_cn（`https://api.minimaxi.com/v1`） | - | C | 同上 |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: 🔶 部分覆盖（同 minimax）
- **aimux 代码位置**: `openai_compat_registry.rs:1268`
- **差距说明**: Token Plan 与普通套餐 API 面一致（计费/额度差异，不影响 request 构造）。
- **建议动作**: 与 minimax 一并处理。

#### 3. 证据与验证

- **证据等级**: C
- **验证状态**: 🔲 未验证
- **存疑标记**: 套餐 API 面与普通 API 相同的推断基于官方同一文档（未见 Token Plan 专有参数文档）

---

### minimax_coding_plan — MiniMax Token Plan (minimax.io)

- **registry 现状**: profile=`full()` · base_url=`https://api.minimax.io/anthropic/v1` · env=`MINIMAX_API_KEY`
- **变体**: minimax 的 Token Plan 国际版

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | ⚠️ 端点为 **Anthropic 兼容**协议（`/anthropic/v1`），OpenAI 参数名（max_completion_tokens 等）不适用 | - | C | https://platform.minimax.io/docs/api-reference/text-anthropic-api（"Anthropic SDK - Models"） |
| 能力支持 | 同 minimax（M2.x 思考不可关） | - | C | https://platform.minimax.io/docs/api-reference/text-chat-openai |
| 思考机制 | Anthropic 协议用 `thinking`/`output_config`；官方推荐 Anthropic 兼容 API 用于 M2.x 的 thinking/reasoning | - | C | https://github.com/HKUDS/nanobot/issues/3068 |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer） | - | - | - |
| URL/端点 | ⚠️ registry 以 OpenAI 兼容 profile 登记了 Anthropic 协议端点（`https://api.minimax.io/anthropic/v1`）——协议与 profile 不匹配 | - | ⚠️ | https://platform.minimax.io/docs/api-reference/text-anthropic-api |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: ⚠️ 不一致（registry 声明错误）
- **aimux 代码位置**: `openai_compat_registry.rs:1277`
- **差距说明**: 该 name 的 base_url 是 Anthropic 协议端点，却声明为 OpenAI 兼容 full()——通过 aimux OpenAI provider 调用会失败（消息/参数格式不对）。
- **建议动作**: 修正 registry（要么改走 anthropic provider，要么改为 `https://api.minimax.io/v1` 的 OpenAI 兼容端点）；补测试。

#### 3. 证据与验证

- **证据等级**: C + ⚠️
- **验证状态**: 🔲 未验证
- **存疑标记**: ⚠️ registry base_url 与协议不匹配（高置信）

---

### mira — Mira

- **registry 现状**: profile=`full()` · base_url=`https://api.mira.so/v1` · env=`MIRA_API_KEY`
- **变体**: -

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 证据不足 | - | - | - |
| 能力支持 | 证据不足 | - | - | - |
| 思考机制 | 证据不足 | - | - | - |
| 流式/usage | 证据不足 | - | - | - |
| 消息格式 | 证据不足 | - | - | - |
| 特殊字段 | 证据不足 | - | - | - |
| headers/认证 | 证据不足 | - | - | - |
| URL/端点 | 证据不足（api.mira.so 未检索到 API 文档） | - | - | - |
| 模型 ID | 证据不足 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: ⚠️ 证据不足
- **aimux 代码位置**: `openai_compat_registry.rs:1286`
- **差距说明**: 未检索到官方 API 文档。
- **建议动作**: 保持 full()；待有真实接入案例再核对。

#### 3. 证据与验证

- **证据等级**: D → 计入存疑归档
- **验证状态**: 🔲 未验证
- **存疑标记**: ⚠️ 存疑

---

### mixlayer — Mixlayer

- **registry 现状**: profile=`full()` · base_url=`https://models.mixlayer.ai/v1` · env=`MIXLAYER_API_KEY`
- **变体**: -

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 证据不足 | - | - | - |
| 能力支持 | 证据不足（多模态 LLM API，具体参数未确认） | - | - | - |
| 思考机制 | 证据不足 | - | - | - |
| 流式/usage | 证据不足 | - | - | - |
| 消息格式 | 证据不足 | - | - | - |
| 特殊字段 | 证据不足 | - | - | - |
| headers/认证 | 证据不足 | - | - | - |
| URL/端点 | base_url `https://models.mixlayer.ai/v1` 被第三方免费模型清单收录（OpenAI 兼容端点） | - | C | https://github.com/velo4705/awesome-free-byok-models |
| 模型 ID | 证据不足 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: ⚠️ 证据不足
- **aimux 代码位置**: `openai_compat_registry.rs:1295`
- **差距说明**: 仅确认 base_url 存在且为 OpenAI 兼容，参数面未确认。
- **建议动作**: 保持 full()；待有真实接入案例再核对。

#### 3. 证据与验证

- **证据等级**: C（仅 URL）+ ⚠️
- **验证状态**: 🔲 未验证
- **存疑标记**: ⚠️ 参数面存疑

---

### moark — Moark

- **registry 现状**: profile=`full()` · base_url=`https://api.moark.com/v1` · env=`MOARK_API_KEY`
- **变体**: -

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异（开源模型聚合，标准 OpenAI 格式） | - | - | - |
| 思考机制 | 无差异（透传） | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer） | - | - | - |
| URL/端点 | 无差异（官方入门文档直接调 `https://api.moark.com/v1/chat/completions`） | `POST https://api.moark.com/v1/chat/completions` | C | https://moark.com/docs/getting-started |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖
- **aimux 代码位置**: `openai_compat_registry.rs:1304`
- **差距说明**: 聚合网关透传，无专有参数。
- **建议动作**: 无需动作。

#### 3. 证据与验证

- **证据等级**: C
- **验证状态**: 🔲 未验证
- **存疑标记**: 无

---

### modal — Modal

- **registry 现状**: profile=`full()` · base_url=`https://modal.com/v1` · env=`MODAL_API_KEY`
- **变体**: -

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（标准 OpenAI SDK 用法） | - | - | - |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 无差异（透传） | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（`api_key` 直插，Bearer） | `client = OpenAI(api_key="...", base_url="https://modal.com/v1")` | C | https://openairouter.net/site/modal-com |
| URL/端点 | 无差异（`https://modal.com/v1`，官方 docs 页面已下线/404，未能直接验证） | - | C | https://modal.com/docs（gateway 文档 404）；第三方：https://openairouter.net/site/modal-com |
| 模型 ID | ⚠️ 未确认：网关路由多家模型，模型命名约定未见官方文档（第三方示例用 `gpt-4o` 无前缀） | `"model":"gpt-4o"` | C | https://openairouter.net/site/modal-com |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖（证据弱）
- **aimux 代码位置**: `openai_compat_registry.rs:1313`
- **差距说明**: 仅确认 OpenAI 兼容 + base_url 一致；模型命名未确证。
- **建议动作**: 无需动作；后续可补官方文档验证。

#### 3. 证据与验证

- **证据等级**: C（弱，第三方来源）
- **验证状态**: 🔲 未验证
- **存疑标记**: ⚠️ 模型 ID 约定存疑

---

### model_oracle_ai — Model Oracle AI

- **registry 现状**: profile=`full()` · base_url=`https://api.modeloracle.com/api/v1` · env=`MODEL_ORACLE_API_KEY`
- **变体**: -

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer） | - | - | - |
| URL/端点 | 无差异（官方 Setup 页 `https://api.modeloracle.com/...`；models.dev 记录 `https://api.modeloracle.com/api/v1`） | - | C | https://modeloracle.com/setup/；https://models.dev/providers/model-oracle-ai |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖
- **aimux 代码位置**: `openai_compat_registry.rs:1322`
- **差距说明**: models.dev 将其标记为 `@ai-sdk/openai-compatible`（纯兼容包），无专有参数。
- **建议动作**: 无需动作。

#### 3. 证据与验证

- **证据等级**: C
- **验证状态**: 🔲 未验证
- **存疑标记**: 无

---

### modelscope — ModelScope

- **registry 现状**: profile=`full()` · base_url=`https://api-inference.modelscope.cn/v1` · env=`MODELSCOPE_API_KEY`
- **变体**: -

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异（OpenAI 兼容 API，支持 OpenAI SDK） | - | - | - |
| 思考机制 | 无差异（透传；Qwen 系列 thinking 由模型/平台处理） | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer，MODELSCOPE_API_KEY） | - | - | - |
| URL/端点 | 无差异（`https://api-inference.modelscope.cn/v1`，官方 API 推理文档） | - | C | https://modelscope.cn/docs/model-service/API-Inference/intro |
| 模型 ID | ⚠️ **模型名必须用 ModelScope Model Id（带 owner 前缀）**：如 `Qwen/Qwen2.5-Coder-32B-Instruct`、`Qwen/Qwen2.5-VL-72B-Instruct`，不带前缀会报错 | `"model":"Qwen/Qwen2.5-Coder-32B-Instruct"` | C | https://modelscope.cn/docs/model-service/API-Inference/intro（"模型名字(model):使用魔搭上开源模型的Model Id，例如 Qwen/Qwen2.5-Coder-32B-Instruct"） |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖（模型名透传，无转换逻辑差异）
- **aimux 代码位置**: `openai_compat_registry.rs:1331`、`openai/convert.rs:1099`（model 字段透传）
- **差距说明**: 模型 ID 需带 owner 前缀是**调用方模型名约定**，aimux 不做模型名改写，透传即可——无代码差异；仅需在文档/提示中说明。
- **建议动作**: 无需动作（可在厂商注释中补模型命名约定提示）。

#### 3. 证据与验证

- **证据等级**: C
- **验证状态**: 🔲 未验证
- **存疑标记**: 无

---

### moonshotai — Moonshot AI

- **registry 现状**: profile=`full()` · base_url=`https://api.moonshot.cn/v1` · env=`MOONSHOT_API_KEY`
- **变体**: moonshotai_cn（Anthropic 端点变体，见下条）

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | `max_tokens` **已弃用**，官方要求用 `max_completion_tokens`（K2/K3 系） | `{"model":"kimi-k2.6","max_completion_tokens":8192,...}` | C | https://platform.kimi.com/docs/api/chat（"max_tokens 已弃用，请使用 max_completion_tokens"） |
| 能力支持 | `logprobs`/`top_logprobs`（0-20）支持；多模态支持 `image_url`/`video_url` 内容类型 | `{"type":"video_url","video_url":{"url":"data:video/mp4;base64,..."}}` | C | https://platform.kimi.com/docs/api/chat |
| 思考机制 | **按模型两套机制**：① `kimi-k3` 始终思考 + Preserved Thinking，用顶层 `reasoning_effort`（仅 `low/high/max` 三档，默认 `max`，**无 medium**）；② `kimi-k2.6` 用 `thinking:{type:"enabled"|"disabled", keep:null|"all"}`（keep=保留历史 reasoning_content）；`kimi-k2.7-code` 固定 enabled 不可关 | `{"model":"kimi-k2.6","thinking":{"type":"enabled","keep":"all"}}` / `{"model":"kimi-k3","reasoning_effort":"high"}` | C | https://platform.kimi.com/docs/api/chat（思考模式与 Preserved Thinking 节）；https://platform.kimi.com/docs/guide/use-thinking-models |
| 流式/usage | 无差异（`stream_options.include_usage` 标准）；usage 含 `cached_tokens` | `"usage":{"prompt_tokens":19,"completion_tokens":13,"total_tokens":32,"cached_tokens":12}` | C | https://platform.kimi.com/docs/api/chat |
| 消息格式 | 响应 `message.reasoning_content` 返回推理过程；多轮必须把每轮 assistant 的 `reasoning_content` 原样回传；**Partial Mode**：assistant 消息可带 `partial:true` 预填输出前缀 | `{"role":"assistant","content":"```python\n","partial":true}` | C | https://platform.kimi.com/docs/api/chat（Partial Mode 节） |
| 特殊字段 | `prompt_cache_key`（缓存命中；Kimi Code Plan 必填）、`safety_identifier`、`prediction`（Predicted Output） | `{"model":"kimi-k3","prompt_cache_key":"session-123","safety_identifier":"u-8f2a"}` | C | https://platform.kimi.com/docs/api/chat |
| headers/认证 | 无差异（`Authorization: Bearer <MOONSHOT_API_KEY>`） | - | C | https://platform.kimi.com/docs/api/chat |
| URL/端点 | 无差异（`https://api.moonshot.cn/v1/chat/completions`） | - | C | 同上 |
| 模型 ID | 无差异（`kimi-k3`、`kimi-k2.6`、`kimi-k2.7-code`、`kimi-k2.5`、`moonshot-v1`，无前缀） | `"model":"kimi-k2.6"` | C | 同上 |

#### 2. aimux 现状对比

- **对比结论**: 🔶 部分覆盖（思考机制 ❌ 按模型区分；特殊字段 ✅）
- **aimux 代码位置**: `openai/convert.rs:1326-1329`（reasoning_effort 发送）、`convert.rs:1426-1433/1484-1552`（仅 deepseek 的 thinking override）、`convert.rs:1306-1317`（prompt_cache_key/safety_identifier/prediction 白名单 ✅）、`openai_compat_registry.rs:1340`
- **差距说明**: ① `thinking:{type,keep}` 机制 aimux 只在 deepseek profile 触发（格式恰好同构），moonshot 需 profile 或 bodyOverrides 注入；② `reasoning_effort` 档位 `low/high/max`（无 medium/minimal/xhigh）与 OpenAI 通用档位映射不同，直接透传 OpenAI 档位可能报 `unsupported_value`；③ `partial:true` 消息字段未覆盖（❌）；④ prompt_cache_key/safety_identifier/prediction 已被白名单覆盖 ✅。
- **建议动作**: moonshot 单列 profile（复用 DeepSeek 式 thinking override + effort 档位映射 + 可选 partial 透传）；补测试。现有 cassette（`tests/cassettes/moonshotai/`）为从 openai 派生的合成录制，非真实 Moonshot 响应，不构成 A 级证据。

#### 3. 证据与验证

- **证据等级**: C
- **验证状态**: 🔲 未验证（cassette 为合成派生，见 `tests/cassettes/moonshotai/thin_wrapper_nonstream.json:1-3`）
- **存疑标记**: 无

---

### moonshotai_cn — Moonshot AI (China)

- **registry 现状**: profile=`full()` · base_url=`https://api.moonshot.cn/anthropic/v1锛圓nthropic` · env=`MOONSHOT_API_KEY`
- **变体**: moonshotai 的 Anthropic 端点变体

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | ⚠️ 该端点走 **Anthropic Messages 协议**（`/anthropic/v1`），OpenAI 参数名不适用 | - | ⚠️ | https://platform.kimi.com/docs（Anthropic 兼容端点） |
| 能力支持 | 同 moonshotai | - | - | - |
| 思考机制 | Anthropic 协议用 `thinking`（budget）语义，与 OpenAI 端不同 | - | C | https://platform.kimi.com/docs/api/chat |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 同 moonshotai（reasoning_content/partial） | - | C | 同上 |
| 特殊字段 | 同 moonshotai | - | C | 同上 |
| headers/认证 | 无差异（Bearer） | - | - | - |
| URL/端点 | ⚠️ **registry 的 base_url 字符串损坏**：`https://api.moonshot.cn/anthropic/v1锛圓nthropic` 含乱码 `锛圓nthropic`（疑似注释被拼进 URL），实际应为 `https://api.moonshot.cn/anthropic/v1` | - | ⚠️ | registry 自证：`openai_compat_registry.rs:1353` |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: ⚠️ 不一致（registry base_url 数据错误 + 协议错配）
- **aimux 代码位置**: `openai_compat_registry.rs:1349-1356`
- **差距说明**: ① base_url 含乱码，直接请求必失败；② Anthropic 协议端点声明为 OpenAI 兼容 full()，协议不匹配。
- **建议动作**: 修正 base_url 字符串（去掉乱码后缀）；若保留 Anthropic 端点应改走 anthropic provider 或删除该 name。

#### 3. 证据与验证

- **证据等级**: ⚠️（registry 数据错误为自证）
- **验证状态**: 🔲 未验证
- **存疑标记**: ⚠️ 高置信数据错误

---

### morph — Morph LLM

- **registry 现状**: profile=`full()` · base_url=`https://api.morphllm.com/v1` · env=`MORPH_API_KEY`
- **变体**: -

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 无差异（透传） | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异（LLM Proxy 路由功能经 header/路由，非请求体字段） | - | - | - |
| headers/认证 | 无差异（OpenAI 兼容，Bearer） | - | - | - |
| URL/端点 | 无差异（官方："Point any OpenAI-compatible client at https://api.morphllm.com/v1"） | `client = OpenAI(base_url="https://api.morphllm.com/v1", ...)` | C | https://www.morphllm.com/kimi-k3-api |
| 模型 ID | 模型名带 `morph-` 前缀（`morph-glm52-744b` 等），与上游原名不同 | `"model":"morph-glm52-744b"` | C | https://www.morphllm.com/glm-5.2-api |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖
- **aimux 代码位置**: `openai_compat_registry.rs:1359`
- **差距说明**: 模型 ID 前缀为调用方命名约定，aimux 透传即可。
- **建议动作**: 无需动作。

#### 3. 证据与验证

- **证据等级**: C
- **验证状态**: 🔲 未验证
- **存疑标记**: 无

---

### nanogpt — NanoGPT

- **registry 现状**: profile=`full()` · base_url=`https://api.nanogpt.com/v1` · env=`NANOGPT_API_KEY`
- **变体**: -

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 证据不足 | - | - | - |
| 能力支持 | 证据不足 | - | - | - |
| 思考机制 | 证据不足 | - | - | - |
| 流式/usage | 证据不足 | - | - | - |
| 消息格式 | 证据不足 | - | - | - |
| 特殊字段 | 证据不足 | - | - | - |
| headers/认证 | 证据不足 | - | - | - |
| URL/端点 | 证据不足（api.nanogpt.com 存在但未检索到官方 API 文档） | - | - | - |
| 模型 ID | 证据不足 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: ⚠️ 证据不足
- **aimux 代码位置**: `openai_compat_registry.rs:1367`
- **差距说明**: 未检索到官方 API 文档。
- **建议动作**: 保持 full()；待有真实接入案例再核对。

#### 3. 证据与验证

- **证据等级**: D → 计入存疑归档
- **验证状态**: 🔲 未验证
- **存疑标记**: ⚠️ 存疑

---

### ncompass — Ncompass

- **registry 现状**: profile=`full()` · base_url=`https://api.ncompass.tech/v1` · env=`NCOMPASS_API_KEY`
- **变体**: -

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 证据不足 | - | - | - |
| 能力支持 | 证据不足 | - | - | - |
| 思考机制 | 证据不足 | - | - | - |
| 流式/usage | 证据不足 | - | - | - |
| 消息格式 | 证据不足 | - | - | - |
| 特殊字段 | 证据不足 | - | - | - |
| headers/认证 | 证据不足 | - | - | - |
| URL/端点 | 证据不足（api.ncompass.tech 未检索到 API 文档） | - | - | - |
| 模型 ID | 证据不足 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: ⚠️ 证据不足
- **aimux 代码位置**: `openai_compat_registry.rs:1376`
- **差距说明**: 未检索到官方 API 文档。
- **建议动作**: 保持 full()；待有真实接入案例再核对。

#### 3. 证据与验证

- **证据等级**: D → 计入存疑归档
- **验证状态**: 🔲 未验证
- **存疑标记**: ⚠️ 存疑

---

### nearai — NEAR AI Cloud

- **registry 现状**: profile=`full()` · base_url=`https://cloud-api.near.ai/v1` · env=`NEARAI_API_KEY`
- **变体**: -

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（官方示例用 `max_tokens`；`max_completion_tokens` 亦兼容） | - | - | - |
| 能力支持 | 无差异（tool calling、structured outputs、reasoning、images、embeddings、audio、Responses API 全套） | - | C | https://docs.near.ai/cloud/guides/openai-compatibility |
| 思考机制 | 无差异（reasoning 模型支持，透传） | - | C | https://docs.near.ai/cloud/reasoning-models |
| 流式/usage | 无差异（SSE 标准） | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异（`X-Request-Id` 为响应头） | - | C | https://docs.near.ai/cloud/guides/openai-compatibility |
| headers/认证 | 无差异（`Authorization: Bearer <NEAR AI API KEY>`，OpenAI SDK 直连） | `client = OpenAI(base_url="https://cloud-api.near.ai/v1", api_key="...")` | C | https://docs.near.ai/cloud/guides/openai-compatibility |
| URL/端点 | 无差异（Gateway `https://cloud-api.near.ai/v1`；另有 Direct Completions `https://{slug}.completions.near.ai/v1`） | - | C | 同上 |
| 模型 ID | 模型名带 owner 前缀：`zai-org/GLM-5.1-FP8` 等（HF 风格） | `"model":"zai-org/GLM-5.1-FP8"` | C | 同上 |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖
- **aimux 代码位置**: `openai_compat_registry.rs:1385`
- **差距说明**: 模型 ID 前缀为命名约定（透传）；其余参数面全标准。
- **建议动作**: 无需动作。

#### 3. 证据与验证

- **证据等级**: C
- **验证状态**: 🔲 未验证
- **存疑标记**: 无

---

### nebius — Nebius AI

- **registry 现状**: profile=`full()` · base_url=`https://api.studio.nebius.ai/v1` · env=`NEBIUS_API_KEY`
- **变体**: -

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（litellm 示例用 `max_tokens`；OpenAI 参数全支持） | - | - | - |
| 能力支持 | 无差异（frequency/presence_penalty、logit_bias、seed、stop、tools、response_format 等全参数） | - | C | https://docs.litellm.ai/docs/providers/nebius |
| 思考机制 | 无差异（DeepSeek-R1 等 reasoning 模型透传） | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer，NEBIUS_API_KEY） | - | C | https://docs.litellm.ai/docs/providers/nebius |
| URL/端点 | 无差异（litellm 文档指向 docs.nebius.com/studio/inference/quickstart；`https://api.studio.nebius.ai/v1` 与 registry 一致） | - | C | https://docs.litellm.ai/docs/providers/nebius |
| 模型 ID | 模型名带 owner 前缀：`Qwen/Qwen3-235B-A22B`、`deepseek-ai/DeepSeek-R1`、`BAAI/bge-en-icl`（HF 风格） | `"model":"Qwen/Qwen3-235B-A22B"` | C | https://docs.litellm.ai/docs/providers/nebius |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖
- **aimux 代码位置**: `openai_compat_registry.rs:1394`
- **差距说明**: 模型 ID 前缀为命名约定（透传）；参数面全标准。
- **建议动作**: 无需动作。

#### 3. 证据与验证

- **证据等级**: C
- **验证状态**: 🔲 未验证
- **存疑标记**: 无

---

### neon — Neon

- **registry 现状**: profile=`full()` · base_url=`https://<branch-host>/v1` · env=`NEON_AI_GATEWAY_TOKEN`
- **变体**: -

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（官方示例 `max_tokens`；"fully compatible with the OpenAI Chat Completions API"） | `client.chat.completions.create({model:'gpt-5-mini', messages:[...], max_tokens:256})` | C | https://neon.com/docs/ai-gateway/chat-completions |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 无差异（透传） | - | - | - |
| 流式/usage | 无差异（SSE 标准）；速率限制经 `Retry-After`/`X-Ratelimit-*` 响应头 | - | C | 同上 |
| 消息格式 | ⚠️ 部分模型 `message.content` 返回**内容块数组**而非纯字符串（"Content shape varies by model"） | `"message":{"content":[{...}],"role":"assistant"}` | C | https://neon.com/docs/ai-gateway/models（"For a few models, message.content comes back as an array of content blocks"） |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（`NEON_AI_GATEWAY_TOKEN` 作为 OpenAI `apiKey`，Bearer） | `new OpenAI({apiKey: process.env.NEON_AI_GATEWAY_TOKEN, baseURL: '${...}/v1'})` | C | https://neon.com/docs/ai-gateway/chat-completions |
| URL/端点 | 无差异（Base URL `https://<branch-host>/v1`，与 registry 一致；另有等价的 `/ai-gateway/mlflow/v1` 长路径） | - | C | 同上 |
| 模型 ID | 无前缀直用（`gpt-5-mini`、`gemini-3-flash`、`qwen3-next-80b-a3b-instruct`），切换厂商只改 model 字段 | `"model":"gpt-5-mini"` | C | 同上 |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖（⚠️ 消息格式数组块为响应侧风险）
- **aimux 代码位置**: `openai_compat_registry.rs:1403`
- **差距说明**: base_url 含 `<branch-host>` 占位符，属于用户部署环境（Neon branch）动态值——aimux 需支持占位符解析或用户自设 base_url；`content` 数组块解析依赖响应解析器对 OpenAI content 数组的兼容（aimux 目前按字符串/OpenAI 标准处理）。
- **建议动作**: 确认 aimux 响应解析对 content 数组的处理；文档注明 branch host 需由用户替换。

#### 3. 证据与验证

- **证据等级**: C
- **验证状态**: 🔲 未验证
- **存疑标记**: ⚠️ content 数组块仅部分模型，影响面未实测

---

### neuralwatt — Neuralwatt

- **registry 现状**: profile=`full()` · base_url=`https://api.neuralwatt.com/v1` · env=`NEURALWATT_API_KEY`
- **变体**: -

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 无差异（透传） | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（`Authorization: Bearer sk-xxxxx`） | `curl https://api.neuralwatt.com/v1/chat/completions -H "Authorization: ..."` | C | https://portal.neuralwatt.com/docs/api/overview |
| URL/端点 | 无差异（官方："OpenAI-compatible endpoint (https://api.neuralwatt.com/v1)"） | `client = OpenAI(base_url="https://api.neuralwatt.com/v1", api_key="sk-xxxxx")` | C | https://portal.neuralwatt.com/docs/quickstart |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖
- **aimux 代码位置**: `openai_compat_registry.rs:1412`
- **差距说明**: 官方明确 OpenAI 兼容。
- **建议动作**: 无需动作。

#### 3. 证据与验证

- **证据等级**: C
- **验证状态**: 🔲 未验证
- **存疑标记**: 无

---

### nextbit — NextBit

- **registry 现状**: profile=`full()` · base_url=`https://api.nextbit.ai/v1` · env=`NEXTBIT_API_KEY`
- **变体**: -

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 证据不足 | - | - | - |
| 能力支持 | 证据不足 | - | - | - |
| 思考机制 | 证据不足 | - | - | - |
| 流式/usage | 证据不足 | - | - | - |
| 消息格式 | 证据不足 | - | - | - |
| 特殊字段 | 证据不足 | - | - | - |
| headers/认证 | 证据不足 | - | - | - |
| URL/端点 | 证据不足（api.nextbit.ai 未检索到 API 文档） | - | - | - |
| 模型 ID | 证据不足 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: ⚠️ 证据不足
- **aimux 代码位置**: `openai_compat_registry.rs:1421`
- **差距说明**: 未检索到官方 API 文档。
- **建议动作**: 保持 full()；待有真实接入案例再核对。

#### 3. 证据与验证

- **证据等级**: D → 计入存疑归档
- **验证状态**: 🔲 未验证
- **存疑标记**: ⚠️ 存疑

---

### nlp_cloud — NLP Cloud

- **registry 现状**: profile=`full()` · base_url=`https://api.nlpcloud.io/v1` · env=`NLPCLOUD_API_KEY`
- **变体**: -

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 证据不足（原生 API 为每模型独立端点 `/v1/{model}/chatbot`，OpenAI 兼容模式存在但参数面未验证） | - | - | - |
| 能力支持 | 证据不足 | - | - | - |
| 思考机制 | 无差异（不支持 reasoning 模型） | - | - | - |
| 流式/usage | 证据不足 | - | - | - |
| 消息格式 | 证据不足 | - | - | - |
| 特殊字段 | 证据不足 | - | - | - |
| headers/认证 | ⚠️ 原生 API 认证为 **`Authorization: Token <key>`**（非 Bearer）；OpenAI 兼容模式是否同样用 Token 未确证 | `curl -H "Authorization: Token <API_KEY>"` | C | https://docs.nlpcloud.com/（"Add your API key after the Token keyword in an Authorization header"） |
| URL/端点 | ⚠️ registry base `https://api.nlpcloud.io/v1`；原生调用路径为 `/v1/{model}/...`，OpenAI 兼容模式路径（`/v1/chat/completions`）存在但未经官方文档直接确认 | - | ⚠️ | https://docs.nlpcloud.com/ |
| 模型 ID | 原生模型名为 slug（如 `finetuned-...`、`gpt-...`）；⚠️ 与 OpenAI 兼容模式关系未确认 | - | ⚠️ | https://docs.nlpcloud.com/ |

#### 2. aimux 现状对比

- **对比结论**: ⚠️ 证据不足/可能不一致（认证 scheme）
- **aimux 代码位置**: `openai_compat_registry.rs:1430`
- **差距说明**: aimux 统一发 `Authorization: Bearer`（provider 实现层），若 NLP Cloud OpenAI 兼容端点沿用 `Token` scheme 会认证失败；OpenAI 兼容模式参数面未验证。
- **建议动作**: 用真实 key 验证认证 scheme；必要时在 provider 层加 `auth_scheme` profile 字段或 bodyOverrides 兜底。

#### 3. 证据与验证

- **证据等级**: C + ⚠️
- **验证状态**: 🔲 未验证
- **存疑标记**: ⚠️ 认证 scheme 与 OpenAI 兼容端点存在性存疑

---

### nous_research — Nous Research

- **registry 现状**: profile=`full()` · base_url=`https://api.nousresearch.com/v1` · env=`NOUS_API_KEY`
- **变体**: -

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异（Hermes 模型 OpenAI 兼容 tool calls） | - | C | https://docs.firecrawl.dev/quickstarts/nous-research |
| 思考机制 | 无差异（透传） | - | - | - |
| 流式/usage | ⚠️ 社区报告推理 API 流式在 agent 规模负载下超时（非 request 格式差异） | - | C | https://github.com/NousResearch/hermes-agent/issues/29418 |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（`Authorization: Bearer <NOUS_API_KEY>`） | `new OpenAI({apiKey: process.env.NOUS_API_KEY, baseURL: 'https://inference-api.nousresearch.com/v1'})` | C | https://docs.firecrawl.dev/quickstarts/nous-research |
| URL/端点 | ⚠️ 官方/社区一致使用的 base 为 **`https://inference-api.nousresearch.com/v1`**（Nous Portal inference），registry 的 `api.nousresearch.com/v1` 未见出处 | - | C | https://docs.firecrawl.dev/quickstarts/nous-research；https://hermes-agent.nousresearch.com/docs/integrations/nous-portal |
| 模型 ID | 无差异（Hermes 系列等，透传） | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: ⚠️ base_url 可能不一致
- **aimux 代码位置**: `openai_compat_registry.rs:1439`
- **差距说明**: registry 的 `api.nousresearch.com` 与官方文档的 `inference-api.nousresearch.com` 不一致，需验证哪个有效（可能其一为旧域名）。
- **建议动作**: 用真实 key 验证域名；若为旧域名则修正 base_url。

#### 3. 证据与验证

- **证据等级**: C + ⚠️
- **验证状态**: 🔲 未验证
- **存疑标记**: ⚠️ base_url 域名存疑

---

### novita — Novita AI

- **registry 现状**: profile=`full()` · base_url=`https://api.novita.ai/v1` · env=`NOVITA_API_KEY`
- **变体**: -

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 官方 reasoning 模型示例用 `max_tokens`（非 max_completion_tokens） | `client.chat.completions.create(model="deepseek/deepseek-r1", stream=True, max_tokens=4096)` | C | https://novita.ai/docs/guides/llm-reasoning |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 无专有 thinking 参数（DeepSeek R1 等 reasoning 模型由模型名驱动，输出 `reasoning_content`） | `"model":"deepseek/deepseek-r1"` | C | https://novita.ai/docs/guides/llm-reasoning |
| 流式/usage | 无差异；流式 delta 含 `reasoning_content` | `if chunk.choices[0].delta.reasoning_content: ...` | C | https://novita.ai/docs/guides/llm-reasoning |
| 消息格式 | 响应 `message.reasoning_content` 携带推理过程；推理内容**不会**自动带入下一轮，需手动维护历史 | `content = response.choices[0].message.content; reasoning = response.choices[0].message.reasoning_content` | C | https://novita.ai/docs/guides/llm-reasoning |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer） | - | - | - |
| URL/端点 | ⚠️ 官方文档示例 base_url 为 **`https://api.novita.ai/openai`**（reasoning 指南）与 `https://api.novita.ai/openai/v1`（llm-api 指南），registry 用 `/v1`——三处路径需确认哪个有效 | `client = OpenAI(api_key="...", base_url="https://api.novita.ai/openai")` | C | https://novita.ai/docs/guides/llm-reasoning；https://novita.ai/docs/guides/llm-api |
| 模型 ID | 模型名带 provider 前缀：`deepseek/deepseek-r1`、`deepseek/deepseek-v3-0324` 等 | `"model":"deepseek/deepseek-r1"` | C | https://novita.ai/docs/guides/llm-reasoning |

#### 2. aimux 现状对比

- **对比结论**: 🔶 部分覆盖（URL ⚠️；reasoning_content 为响应侧）
- **aimux 代码位置**: `openai_compat_registry.rs:1448`
- **差距说明**: ① base_url 路径（`/v1` vs `/openai` vs `/openai/v1`）需实测确认；② `reasoning_content` 输出别名 aimux 响应解析未覆盖（❌ 输出侧）；③ 模型 ID 前缀为透传约定。
- **建议动作**: 用真实 key 验证 base_url 并修正；响应解析补 `reasoning_content`（同 moonshot 场景）。

#### 3. 证据与验证

- **证据等级**: C
- **验证状态**: 🔲 未验证
- **存疑标记**: ⚠️ base_url 路径三处不一致

---

### nscale — Nscale

- **registry 现状**: profile=`full()` · base_url=`https://inference.api.nscale.com/v1` · env=`NSCALE_API_KEY`
- **变体**: -

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 无差异（DeepSeek/GPT OSS 等 reasoning 模型透传） | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer，NSCALE_API_KEY） | - | - | - |
| URL/端点 | 无差异（官方："call https://inference.api.nscale.com/v1/*"，与 registry 一致） | `client = OpenAI(base_url="https://inference.api.nscale.com/v1", api_key=...)` | C | https://www.nscale.com/blog/from-idea-to-inference-in-minutes-welcome-to-nscale-serverless；https://docs.nscale.com/changelog/changelog |
| 模型 ID | 无差异（开放模型 Llama/Qwen/DeepSeek/GPT OSS，透传） | - | C | https://apis.io/providers/nscale/ |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖
- **aimux 代码位置**: `openai_compat_registry.rs:1457`
- **差距说明**: 标准 OpenAI 兼容。
- **建议动作**: 无需动作。

#### 3. 证据与验证

- **证据等级**: C
- **验证状态**: 🔲 未验证
- **存疑标记**: 无

---

### nvidia_nim — NVIDIA NIM

- **registry 现状**: profile=`full()` · base_url=`https://integrate.api.nvidia.com/v1` · env=`NVIDIA_API_KEY`
- **变体**: -

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（OpenAI 兼容；历史 bug 要求 max_token 为必填，近期版本已修复） | - | C | https://forums.developer.nvidia.com/t/openai-compatible-api-does-not-work/303942 |
| 能力支持 | 无差异（streaming、tool calling、json 等） | - | C | https://docs.nvidia.com/nim/large-language-models/latest/api-reference.html |
| 思考机制 | 无专有 thinking 参数（reasoning 模型由模型名驱动） | `"model":"deepseek-ai/deepseek-r1"` | C | https://build.nvidia.com/deepseek-ai/deepseek-v4-flash |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | reasoning 模型响应拆分出 `reasoning_content` 字段（NIM 已支持拆分） | `message = getattr(..., "reasoning_content", None)` | C | https://github.com/RooCodeInc/Roo-Code/issues/10969；https://forums.developer.nvidia.com/t/bug-report-nvidia-nim-hosted-endpoint-reliability-issues-bugs-requiring-extensive-client-side-workarounds/366612 |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（`Authorization: Bearer $NVIDIA_API_KEY`） | `client = OpenAI(base_url="https://integrate.api.nvidia.com/v1", api_key="$NVIDIA_API_KEY")` | C | https://build.nvidia.com/deepseek-ai/deepseek-v4-flash |
| URL/端点 | 无差异（`https://integrate.api.nvidia.com/v1`，与 registry 一致） | - | C | 同上 |
| 模型 ID | 模型名带 org 前缀：`deepseek-ai/deepseek-r1`、`meta/llama-3.3-70b-instruct`、`nvidia/llama-3.1-nemotron-...`（HF 风格） | `"model":"meta/llama-3.3-70b-instruct"` | C | https://build.nvidia.com/；https://github.com/RooCodeInc/Roo-Code/issues/10969 |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖（reasoning_content 输出侧 ❌）
- **aimux 代码位置**: `openai_compat_registry.rs:1466`
- **差距说明**: 模型 ID org 前缀为透传约定；`reasoning_content` 响应字段未解析（与 moonshot/novita 同类，建议统一补）。
- **建议动作**: 响应解析统一补 `reasoning_content`；其余无需动作。

#### 3. 证据与验证

- **证据等级**: C
- **验证状态**: 🔲 未验证
- **存疑标记**: 无

---

### oci — OCI

- **registry 现状**: profile=`full()` · base_url=`https://inference.generativeai.${region}.oci.oraclecloud.com/openai/v1` · env=`OCI_API_KEY`
- **变体**: -

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（Chat Completions 兼容） | - | - | - |
| 能力支持 | 无差异（Chat Completions + Responses API；Responses 为推荐主接口） | - | C | https://docs.oracle.com/en-us/iaas/Content/generative-ai/openai-compatible-api.htm |
| 思考机制 | 无差异（透传） | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（`Authorization: Bearer <OCI GenAI API key>`，非 OCI 签名认证） | `--header 'Authorization: Bearer sk-...'` | C | https://docs.oracle.com/en-us/iaas/Content/generative-ai/api-keys.htm |
| URL/端点 | 无差异（OpenAI 兼容 base `https://inference.generativeai.${region}.oci.oraclecloud.com/openai/v1`，与 registry 一致；`${region}` 为占位符需用户替换，如 `us-chicago-1`） | - | C | https://docs.oracle.com/en-us/iaas/Content/generative-ai/openai-compatible-api.htm |
| 模型 ID | ⚠️ 模型名带 **provider 前缀（点分隔）**：`openai.gpt-oss-120b`、`xai.grok-3`、`meta.llama-3.3-70b-instruct` | `"model":"xai.grok-3"` | C | https://docs.oracle.com/en-us/iaas/Content/generative-ai/api-keys.htm |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖（region 占位符需用户替换）
- **aimux 代码位置**: `openai_compat_registry.rs:1475`
- **差距说明**: `${region}` 是模板占位符，aimux 不会自动替换——用户须配置完整 base_url；模型名透传。
- **建议动作**: 在文档/registry 注释中说明 `${region}` 占位符；其余无需动作。

#### 3. 证据与验证

- **证据等级**: C
- **验证状态**: 🔲 未验证
- **存疑标记**: 无

---

### ofox — OfoxAI

- **registry 现状**: profile=`full()` · base_url=`https://api.ofox.ai/v1` · env=`OFOX_API_KEY`
- **变体**: -

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异（OpenAI/Anthropic/Gemini 三协议，OpenAI 协议"supports all models"） | - | C | https://ofox.ai/docs/api |
| 思考机制 | 无差异（透传） | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（API Key 认证，Bearer） | `client = OpenAI(base_url="https://api.ofox.ai/v1", api_key="<OFOXAI_API_KEY>")` | C | https://ofox.ai/docs/develop/authentication |
| URL/端点 | 无差异（官方 OpenAI 兼容 base `https://api.ofox.ai/v1`，与 registry 一致；Anthropic 端点为 `/anthropic`） | - | C | https://ofox.ai/docs/api |
| 模型 ID | 模型名带 provider 前缀：`openai/...`、`anthropic/...`、`google/...` 等 | `"model":"openai/gpt-4o"` | C | https://ofox.ai/ |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖
- **aimux 代码位置**: `openai_compat_registry.rs:1484`
- **差距说明**: 模型 ID 前缀为透传约定；无专有参数。
- **建议动作**: 无需动作。

#### 3. 证据与验证

- **证据等级**: C
- **验证状态**: 🔲 未验证
- **存疑标记**: 无

---

### ohmygpt — OhMyGPT

- **registry 现状**: profile=`full()` · base_url=`https://api.ohmygpt.com/v1` · env=`OHMYGPT_API_KEY`
- **变体**: -

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（OpenAI 兼容中转站） | - | - | - |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 无差异（透传） | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer） | - | - | - |
| URL/端点 | 无差异（one-api 默认渠道表收录 `https://api.ohmygpt.com` 为 OpenAI 兼容渠道） | - | C | https://github.com/songquanpeng/one-api/blob/main/relay/channeltype/url.go |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖
- **aimux 代码位置**: `openai_compat_registry.rs:1493`
- **差距说明**: 经典中转站，纯透传；官方 API 文档未检索到，one-api 渠道表为间接证据。
- **建议动作**: 无需动作。

#### 3. 证据与验证

- **证据等级**: C（间接）
- **验证状态**: 🔲 未验证
- **存疑标记**: 官方文档缺失，参数面为推断

---

### ollama_cloud — Ollama Cloud

- **registry 现状**: profile=`full()` · base_url=`https://api.ollama.com/v1` · env=`OLLAMA_CLOUD_API_KEY`
- **变体**: -

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | ⚠️ `logprobs`/`top_logprobs` 请求被接受但响应返回 `null`（Cloud 端不支持，本地 Ollama 支持） | `{"model":"deepseek-v3.2","logprobs":true,"top_logprobs":5}` → `"message":{"content":"4"}` 无 logprobs 字段 | C | https://github.com/ollama/ollama/issues/13638 |
| 思考机制 | 无差异（透传） | - | - | - |
| 流式/usage | 无差异（`/v1/chat/completions` SSE 标准） | - | C | 同上 |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（`Authorization: Bearer $OLLAMA_API_KEY`） | `curl 'https://ollama.com/api/chat' -H "Authorization: Bearer $OLLAMA_API_KEY"` | C | https://github.com/ollama/ollama/issues/13638；https://pypi.org/project/ollama/ |
| URL/端点 | 无差异（OpenAI 兼容 `POST /v1/chat/completions`；`https://ollama.com` 与 `https://api.ollama.com` 均可用） | - | C | 同上 |
| 模型 ID | 无差异（`deepseek-v3.2`、`gpt-oss:120b-cloud` 等，无前缀） | `"model":"gpt-oss:120b-cloud"` | C | https://github.com/ollama/ollama/issues/13638 |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖（logprobs 无影响——aimux 白名单不含 logprobs，不会发送）
- **aimux 代码位置**: `openai_compat_registry.rs:1502`、`openai/convert.rs:1284-1317`
- **差距说明**: aimux 从不发送 logprobs，故 Cloud 端不支持无影响；其余参数面标准。
- **建议动作**: 无需动作；若未来支持 logprobs 需注意 Cloud 端差异。

#### 3. 证据与验证

- **证据等级**: C
- **验证状态**: 🔲 未验证
- **存疑标记**: 无

---

### openaimax — OpenAIMax

- **registry 现状**: profile=`full()` · base_url=`https://api.openaimax.com/v1` · env=`OPENAIMAX_API_KEY`
- **变体**: -

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（OpenAI 兼容中转站） | - | - | - |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 无差异（透传） | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer） | - | - | - |
| URL/端点 | 无差异（one-api/new-api 默认渠道表收录 `https://api.openaimax.com` 为 OpenAI 兼容渠道） | - | C | https://github.com/songquanpeng/one-api/blob/main/relay/channeltype/url.go；https://pkg.go.dev/github.com/skylinebear/new-api/constant |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖
- **aimux 代码位置**: `openai_compat_registry.rs:1511`
- **差距说明**: 经典中转站，纯透传；官方文档未检索到。
- **建议动作**: 无需动作。

#### 3. 证据与验证

- **证据等级**: C（间接）
- **验证状态**: 🔲 未验证
- **存疑标记**: 官方文档缺失，参数面为推断

---

### openaisb — OpenAI-SB

- **registry 现状**: profile=`full()` · base_url=`https://api.openaisb.com/v1` · env=`OPENAISB_API_KEY`
- **变体**: -

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（OpenAI 兼容中转站） | - | - | - |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 无差异（透传） | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer） | - | - | - |
| URL/端点 | 无差异（one-api/new-api 默认渠道表收录 `https://api.openai-sb.com`；社区文档使用 `https://api.openaisb.com/v1`） | - | C | https://github.com/songquanpeng/one-api/blob/main/relay/channeltype/url.go；https://wenku.csdn.net/answer/bjs3ufq2cgm8 |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**: ✅ 已覆盖
- **aimux 代码位置**: `openai_compat_registry.rs:1520`
- **差距说明**: 经典中转站，纯透传；官方文档未检索到。
- **建议动作**: 无需动作。

#### 3. 证据与验证

- **证据等级**: C（间接）
- **验证状态**: 🔲 未验证
- **存疑标记**: 官方文档缺失，参数面为推断

---

## 存疑归档（⚠️ 条目汇总）

| id | 存疑点 | 等级 |
|----|--------|------|
| lynkr | 无任何公开信息（localhost base_url，自托管） | D |
| matterai | 无任何公开信息 | D |
| mira | 无任何公开信息 | D |
| nanogpt | 无任何公开信息 | D |
| ncompass | 无任何公开信息 | D |
| nextbit | 无任何公开信息 | D |
| mixlayer | 仅确认 base_url，参数面未确认 | C 弱 |
| modal | 官方 gateway 文档 404，模型命名未确认 | C 弱 |
| longcat | OpenAI 兼容路径可能是 `/openai/v1` 而非 `/v1` | C |
| nous_research | registry base `api.nousresearch.com` vs 官方 `inference-api.nousresearch.com` | C |
| novita | base_url 三处路径不一致（`/v1` vs `/openai` vs `/openai/v1`） | C |
| nlp_cloud | 认证 scheme（`Authorization: Token` vs Bearer）与 OpenAI 兼容端点存在性 | C |
| minimax_coding_plan | Anthropic 协议端点登记为 OpenAI 兼容 profile | C |
| moonshotai_cn | registry base_url 含乱码（`锛圓nthropic`） | 自证 |
| neon | 部分模型 content 返回数组块（响应侧） | C |
| minimax | `service_tier=priority` 会被 aimux 能力校验拦截；penalty/logit_bias 被忽略 | C |

## 批次小结

- 完成厂商数：**42 / 42**
- 有差异（含 ⚠️ 存疑差异）的厂商数：**16 家**（llmgateway、minimax/minimax_cn/minimax_cn_coding_plan/minimax_coding_plan、moonshotai/moonshotai_cn、modelscope、novita、nvidia_nim、oci、ollama_cloud、nlp_cloud、neon、nous_research、longcat）；其余经确认无差异或证据不足
- 证据不足（计入存疑归档）：**6 家**（lynkr、matterai、mira、nanogpt、ncompass、nextbit）+ 弱证据 2 家（mixlayer、modal）
- 重要发现（Top 5）：
  1. **Moonshot 思考机制按模型两套**（`kimi-k3` → `reasoning_effort` 仅 low/high/max 三档默认 max；`kimi-k2.6` → `thinking:{type,keep}`）——aimux 现有 OpenAI effort 档位（medium 等）与 deepseek 式 thinking override 均不能直接复用，需新 profile。
  2. **MiniMax thinking 三态**（`adaptive`/`disabled`，M3 默认开、M2.x 不可关）+ `reasoning_split` 输出切分 + `<think>` 内嵌格式 + `service_tier=priority` 被 aimux 校验拦截——同类问题：能力校验应区分"OpenAI 官方模型"与"第三方厂商"。
  3. **6 家厂商 base_url 存疑/错误**：novita（`/openai` vs `/v1`）、nous_research（`api.` vs `inference-api.`）、longcat（`/openai/v1` vs `/v1`）、moonshotai_cn（乱码）、minimax_coding_plan（Anthropic 端点）、oci（`${region}` 占位符）——registry 数据质量需统一校对。
  4. **多家 reasoning 模型厂商（moonshot/novita/nvidia_nim）响应统一用 `reasoning_content`**，aimux 响应解析未覆盖该别名——建议统一补一个 reasoning_content 解析。
  5. **llmgateway 是"网关类"模板**：厂商前缀模型 ID + `reasoning` 对象/max 档位/`verbosity`/`x-no-cache`——与 OpenRouter 类同，可作为后续 gateway 类厂商 profile 扩展的样板。
