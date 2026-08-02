# Batch 03 — Model Request Config 调研

> 状态: ✅ 已完成调研 · 厂商数: 42（2026-08-01 完成）
> 模板见 [_template.md](_template.md) · 方法论见 [README.md](README.md)
> 证据硬性要求: 每条目必须有例子或证明(A/B/C 级),禁止编造来源。

## 厂商清单

| # | id | display | base_url | env_var | profile |
|---|---|---|---|---|---|
| 1 | freemodel | FreeModel | client.chat.completions.create | FREEMODEL_API_KEY | OpenAICompatProfile::full() |
| 2 | friendliai | FriendliAI | https://inference.friendli.ai/v1 | FRIENDLIAI_API_KEY | OpenAICompatProfile::full() |
| 3 | frogbot | FrogBot | https://app.frogbot.ai/api/v1 | FROGBOT_API_KEY | OpenAICompatProfile::full() |
| 4 | galadriel | Galadriel | https://api.galadriel.com/v1 | GALADRIEL_API_KEY | OpenAICompatProfile::full() |
| 5 | gdc | GDC | https://api.gdc.ai/v1 | GDC_API_KEY | OpenAICompatProfile::full() |
| 6 | gigachat | GigaChat (Sberbank) | https://gigachat.devices.sberbank.ru/api/v1 | GIGACHAT_API_KEY | OpenAICompatProfile::full() |
| 7 | github | GitHub Models | https://models.inference.ai.azure.com | GITHUB_TOKEN | OpenAICompatProfile::full() |
| 8 | gmi | GMI | https://api.gmi-serving.com/v1（与 | GMI_API_KEY | OpenAICompatProfile::full() |
| 9 | gmicloud | GMI Cloud | https://api.gmi-serving.com/v1 | GMI_API_KEY | OpenAICompatProfile::full() |
| 10 | gonka24 | Gonka24 | https://api.gonka24.com/v1 | GONKA24_API_KEY | OpenAICompatProfile::full() |
| 11 | gradient_ai | Gradient AI | https://inference.do-ai.run/v1 | GRADIENT_API_KEY | OpenAICompatProfile::full() |
| 12 | groq | Groq | https://api.groq.com/openai/v1 | GROQ_API_KEY | OpenAICompatProfile::groq() |
| 13 | helicone | Helicone | https://api.helicone.ai/v1 | HELICONE_API_KEY | OpenAICompatProfile::full() |
| 14 | heroku | Heroku AI | https://api.heroku.com/inference/v1 | HEROKU_API_KEY | OpenAICompatProfile::full() |
| 15 | hetzner | Hetzner | https://inference.hetzner.com/api/v1 | HETZNER_VLLM_API_KEY | OpenAICompatProfile::full() |
| 16 | hosted_vllm | Hosted vLLM | https://hosted-vllm-api.com/v1 | HOSTED_VLLM_API_KEY | OpenAICompatProfile::full() |
| 17 | hpc_ai | HPC-AI | https://api.hpc-ai.com/inference/v1 | INFERENCE_API_KEY | OpenAICompatProfile::full() |
| 18 | hyperbolic | Hyperbolic | https://api.hyperbolic.xyz/v1 | HYPERBOLIC_API_KEY | OpenAICompatProfile::full() |
| 19 | iflowcn | iFlow | https://apis.iflow.cn/v1（chat | IFLOW_API_KEY | OpenAICompatProfile::full() |
| 20 | inception | Inception Labs | https://api.inceptionlabs.ai/v1 | INCEPTION_API_KEY | OpenAICompatProfile::full() |
| 21 | inceptron | Inceptron | https://api.inceptron.io/v1 | INCEPTRON_API_KEY | OpenAICompatProfile::full() |
| 22 | inference_net | Inference.net | https://api.inference.net/v1 | INFERENCE_NET_API_KEY | OpenAICompatProfile::full() |
| 23 | inferx | InferX | https://model.inferx.net/v1 | INFERX_API_KEY | OpenAICompatProfile::full() |
| 24 | infinity | Infinity AI | https://infinity.ai/api/v1 | INFINITY_API_KEY | OpenAICompatProfile::full() |
| 25 | io_net | IO.NET | https://api.intelligence.io.solutions/api/v1 | IOINTELLIGENCE_API_KEY | OpenAICompatProfile::full() |
| 26 | jiekou | Jiekou.AI | https://api.highwayapi.ai/openai | JIEKOU_API_KEY | OpenAICompatProfile::full() |
| 27 | kenari | Kenari | https://kenari.id/v1 | KENARI_API_KEY | OpenAICompatProfile::full() |
| 28 | kilo | Kilo | https://api.kilo.ai/v1 | KILO_API_KEY | OpenAICompatProfile::full() |
| 29 | kimi | Kimi | https://api.moonshot.ai/v1 | MOONSHOT_API_KEY | OpenAICompatProfile::full() |
| 30 | kimi_for_coding | Kimi For Coding | https://api.kimi.com/coding/v1 | KIMI_API_KEY | OpenAICompatProfile::full() |
| 31 | kiro | Kiro | https://api.kiro.dev/v1 | KIRO_API_KEY | OpenAICompatProfile::full() |
| 32 | kluster_ai | Kluster AI | https://api.kluster.ai/v1 | KLUSTER_API_KEY | OpenAICompatProfile::full() |
| 33 | krutrim | Krutrim | https://api.krutrim.ai/v1 | KRUTRIM_API_KEY | OpenAICompatProfile::full() |
| 34 | kuae_cloud_coding_plan | KUAE Cloud Coding Plan | https://coding-plan-endpoint.kuaecloud.net/v1 | KUAE_API_KEY | OpenAICompatProfile::full() |
| 35 | lambda_ai | Lambda AI | https://api.lambda.ai/v1 | LAMBDA_API_KEY | OpenAICompatProfile::full() |
| 36 | lemonade | Lemonade | http://localhost:13305/v1 | LEMONADE_API_KEY | OpenAICompatProfile::full() |
| 37 | lemonfox_ai | Lemonfox AI | https://api.lemonfox.ai/v1 | LEMONFOX_API_KEY | OpenAICompatProfile::full() |
| 38 | libertai | Libertai | https://api.libertai.io/v1 | LIBERTAI_API_KEY | OpenAICompatProfile::full() |
| 39 | lilac | Lilac | https://api.getlilac.com/v1 | LILAC_API_KEY | OpenAICompatProfile::full() |
| 40 | lingyiwanwu | Lingyiwanwu (零一万物) | https://api.lingyiwanwu.com/v1 | LINGYIWANWU_API_KEY | OpenAICompatProfile::full() |
| 41 | llama | Llama | https://api.llama.com/compat/v1/ | LLAMA_API_KEY | OpenAICompatProfile::full() |
| 42 | llamagate | Llamagate | https://api.llamagate.dev/v1 | LLAMAGATE_API_KEY | OpenAICompatProfile::full() |

## 调研条目（按 id 字母序）

### freemodel — FreeModel

- **registry 现状**：profile=`full()` · base_url=`client.chat.completions.create`（⚠️ 明显是提取错误，这是代码片段不是 URL） · env=`FREEMODEL_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（官方示例用标准 OpenAI 参数 `model/messages`，支持 `model="auto"` 自动路由） | `client.chat.completions.create(model="auto", ...)` | C | https://freemodel.dev/ |
| 能力支持 | 无差异（宣称兼容 OpenAI SDK） | - | C | https://freemodel.dev/ |
| 思考机制 | 无法确认（未找到官方文档） | - | ⚠️ | - |
| 流式/usage | 无差异（宣称 OpenAI 兼容） | - | C | https://freemodel.dev/ |
| 消息格式 | 无差异 | - | C | https://freemodel.dev/ |
| 特殊字段 | 无法确认 | - | ⚠️ | - |
| headers/认证 | 无差异（Bearer key） | - | C | https://segmentfault.com/a/1190000047755983 |
| URL/端点 | ⚠️ registry base_url 是代码片段 `client.chat.completions.create`，非真实 URL；正确 base_url 未确认 | - | ⚠️ | https://freemodel.dev/ |
| 模型 ID | 支持 `model="auto"` 自动路由 | `"model": "auto"` | C | https://freemodel.dev/ |

> 证据等级: C=官方/权威网页 · ⚠️=证据不足

#### 2. aimux 现状对比

- **对比结论**：⚠️ 不一致（registry 数据错误，非协议差异）
- **aimux 代码位置**：`openai_compat_registry.rs:772-780`
- **差距说明**：base_url 字段被错误提取为代码片段；协议本身为 OpenAI 兼容，request 构造无差异。
- **建议动作**：修正 registry base_url（需确认官方端点，如 `https://api.freemodel.dev/v1` 之类）；request 层无需动作。

#### 3. 证据与验证

- **证据等级**：C（官网 + 第三方博客）
- **验证状态**：🔲 未验证（仅文档引用）
- **存疑标记**：⚠️ 正确 base_url 未确认

### friendliai — FriendliAI

- **registry 现状**：profile=`full()` · base_url=`https://inference.friendli.ai/v1` · env=`FRIENDLIAI_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（OpenAI Chat Completions 参数） | - | C | https://docs.litellm.ai/docs/providers/friendliai |
| 能力支持 | 无差异（支持 /chat/completions 与 /completions） | - | C | https://docs.litellm.ai/docs/providers/friendliai |
| 思考机制 | 未发现文档化差异 | - | - | - |
| 流式/usage | 无差异 | - | C | https://docs.litellm.ai/docs/providers/friendliai |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer token；litellm 侧环境变量名为 `FRIENDLI_TOKEN`，registry 用 `FRIENDLIAI_API_KEY`，属命名差异非协议差异） | - | C | https://docs.litellm.ai/docs/providers/friendliai |
| URL/端点 | 无差异 | - | C | https://docs.litellm.ai/docs/providers/friendliai |
| 模型 ID | 约定为 `org/model` 形式（如 `meta-llama-3.1-8b-instruct`） | `"model": "meta-llama-3.1-8b-instruct"` | C | https://docs.litellm.ai/docs/providers/friendliai |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:781-789`
- **差距说明**：协议无差异；模型 ID 直接透传，`org/model` 形式不影响 request 构造。
- **建议动作**：无需动作。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证（仅文档引用）
- **存疑标记**：-

### frogbot — FrogBot

- **registry 现状**：profile=`full()` · base_url=`https://app.frogbot.ai/api/v1` · env=`FROGBOT_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | C | https://models.dev/providers/frogbot |
| 能力支持 | 无差异（26 个模型，`@ai-sdk/openai-compatible` 接入） | - | C | https://models.dev/providers/frogbot |
| 思考机制 | 未发现文档化差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（`Authorization: Bearer $FROGBOT_API_KEY`） | `-H "Authorization: Bearer $FROGBOT_API_KEY"` | C | https://firmware.mintlify.app/api-reference/images-generations |
| URL/端点 | 无差异 | - | C | https://models.dev/providers/frogbot |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:790-798`
- **差距说明**：纯 OpenAI 兼容。
- **建议动作**：无需动作。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：-

### galadriel — Galadriel

- **registry 现状**：profile=`full()` · base_url=`https://api.galadriel.com/v1` · env=`GALADRIEL_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | C | https://github.com/kreuzberg-dev/liter-llm/blob/main/schemas/providers.json |
| 能力支持 | 无差异（chat completions；Sentience SDK 面向 OpenAI 客户端库设计） | - | C | https://docs.galadriel.com/for-agents-developers/quickstart |
| 思考机制 | 未发现文档化差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异（另有 `/v1/verified` 可验证推理完整性，属附加端点） | `base_url="https://api.galadriel.com/v1/verified"` | C | https://docs.galadriel.com/for-agents-developers/quickstart |
| headers/认证 | 无差异（Bearer `GALADRIEL_API_KEY`） | - | C | https://github.com/kreuzberg-dev/liter-llm/blob/main/schemas/providers.json |
| URL/端点 | 无差异 | - | C | https://github.com/kreuzberg-dev/liter-llm/blob/main/schemas/providers.json |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:799-807`
- **差距说明**：纯 OpenAI 兼容。
- **建议动作**：无需动作。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：-

### gdc — GDC

- **registry 现状**：profile=`full()` · base_url=`https://api.gdc.ai/v1` · env=`GDC_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | ⚠️ 无法确认（查不到该厂商资料） | - | ⚠️ | - |
| 能力支持 | ⚠️ 无法确认 | - | ⚠️ | - |
| 思考机制 | ⚠️ 无法确认 | - | ⚠️ | - |
| 流式/usage | ⚠️ 无法确认 | - | ⚠️ | - |
| 消息格式 | ⚠️ 无法确认 | - | ⚠️ | - |
| 特殊字段 | ⚠️ 无法确认 | - | ⚠️ | - |
| headers/认证 | ⚠️ 无法确认 | - | ⚠️ | - |
| URL/端点 | ⚠️ 无法确认（`api.gdc.ai` 无公开文档；"GDC" 搜索主要命中 Google Distributed Cloud / GDC 大会，无法对应此端点） | - | ⚠️ | - |
| 模型 ID | ⚠️ 无法确认 | - | ⚠️ | - |

#### 2. aimux 现状对比

- **对比结论**：⚠️ 证据不足，无法对比
- **aimux 代码位置**：`openai_compat_registry.rs:808-816`
- **差距说明**：全库未查到该厂商任何官方文档或第三方适配；按默认 OpenAI 兼容处理，但无证据。
- **建议动作**：维持 full()，标记为"证据不足"，后续有人使用时补档。

#### 3. 证据与验证

- **证据等级**：无（D 级以下）
- **验证状态**：🔲 未验证
- **存疑标记**：⚠️ 存疑——查不到任何厂商信息

### gigachat — GigaChat (Sberbank)

- **registry 现状**：profile=`full()` · base_url=`https://gigachat.devices.sberbank.ru/api/v1` · env=`GIGACHAT_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（OpenAI SDK 可直接调用；`max_tokens` 等标准参数） | `client = OpenAI(api_key="<токен_доступа>", base_url="https://api.giga.chat/v1")` | C | https://developers.sber.ru/docs/ru/gigachat/guides/compatible-openai |
| 能力支持 | 🔶 部分：函数调用**每次请求仅限 1 个**；Structured Output 通过函数调用模拟实现；GigaChat-2-Max/Pro 支持图像输入（base64/URL） | - | C | https://docs.litellm.ai/docs/providers/gigachat |
| 思考机制 | 未发现文档化差异 | - | - | - |
| 流式/usage | 无差异（支持流式） | - | C | https://docs.litellm.ai/docs/providers/gigachat |
| 消息格式 | 无差异（"GigaChat API 的消息格式与 OpenAI API 部分兼容"） | - | C | https://developers.sber.ru/docs/ru/gigachat/guides/compatible-openai |
| 特殊字段 | ⚠️ TLS：官方 API 使用**自签名证书**，请求必须 `ssl_verify=False` | `ssl_verify=False` | C | https://docs.litellm.ai/docs/providers/gigachat |
| headers/认证 | ❌ **OAuth2 流程**：不能用裸 key 直接调；先 POST `https://ngw.devices.sberbank.ru:9443/api/v2/oauth`（scope=`GIGACHAT_API_PERS`，需 `RqUID` 头 + `Authorization: Bearer <授权密钥>`）换取 30 分钟有效 access token，再以 token 作 Bearer | `POST https://ngw.devices.sberbank.ru:9443/api/v2/oauth` + `headers={"RqUID": "...", "Authorization": "Bearer <ключ_авторизации>"}` `payload="scope=GIGACHAT_API_PERS"` | C | https://developers.sber.ru/docs/ru/gigachat/guides/compatible-openai |
| URL/端点 | ⚠️ registry base_url=`gigachat.devices.sberbank.ru/api/v1` 为旧端点；官方 OpenAI 兼容文档用 `https://api.giga.chat/v1` | `base_url="https://api.giga.chat/v1"` | C | https://developers.sber.ru/docs/ru/gigachat/guides/compatible-openai |
| 模型 ID | 官方模型名：`GigaChat`、`GigaChat-2-Max/Pro/Lite` | `"model": "GigaChat"` | C | https://developers.sber.ru/docs/ru/gigachat/guides/compatible-openai |

#### 2. aimux 现状对比

- **对比结论**：❌ 未覆盖（认证流程）+ ⚠️ base_url 过时
- **aimux 代码位置**：`openai_compat_registry.rs:817-825`；aimux 认证仅支持静态 Bearer key（`load_api_key`）
- **差距说明**：① GigaChat 需要 OAuth token 交换（静态 key 不能直接用）；② 自签名证书需要跳过 TLS 校验，aimux 无此配置；③ registry base_url 与官方文档端点不一致。
- **建议动作**：profile 层暂无法表达 OAuth 交换——建议从 registry 移除或降级为"需 bodyOverrides/手动 token"文档化；base_url 更新为 `https://api.giga.chat/v1`；TLS 校验关闭能力列入 RFC-0017 后续字段评估。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证（仅文档引用）
- **存疑标记**：⚠️ base_url/认证为确定差异；其余类别无证据显示差异

### github — GitHub Models

- **registry 现状**：profile=`full()` · base_url=`https://models.inference.ai.azure.com` · env=`GITHUB_TOKEN`
- **变体**：GitHub Copilot 见 batch-02 `copilot`

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（历史文档：标准 OpenAI Chat Completions） | - | C | https://docs.github.com/en/github-models/quickstart |
| 能力支持 | 无差异（历史：支持主流模型） | - | - | - |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（历史：`Authorization: Bearer <GITHUB_TOKEN>`，可用 PAT 或 OAuth） | - | C | https://docs.github.com/en/github-models/quickstart |
| URL/端点 | ⚠️ **服务已下线**：官方文档明确 GitHub Models 已于 **2026-07-30 全面退役**（playground/模型目录/inference API/BYOK 全部关闭），`models.inference.ai.azure.com` 不再可用 | - | C | https://docs.github.com/en/github-models/quickstart |
| 模型 ID | 无差异（历史：gpt-4o、o1、Llama 等公开模型 ID） | - | C | https://docs.github.com/en/github-models/quickstart |

#### 2. aimux 现状对比

- **对比结论**：⚠️ 不一致（registry 指向已退役服务）
- **aimux 代码位置**：`openai_compat_registry.rs:826-834`
- **差距说明**：服务已于 2026-07-30 退役（当前日期 2026-08-01），registry 条目指向死端点。官方建议迁移到 Azure AI Foundry。
- **建议动作**：从 registry 移除该条目或在文档标注"已退役"；如需替代接入 Azure AI Foundry（其 base_url/auth 为另一套，见 batch 后续）。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证（官方退役公告，无需实测）
- **存疑标记**：⚠️ 服务已退役

### gmi — GMI

- **registry 现状**：profile=`full()` · base_url=`https://api.gmi-serving.com/v1（与`（⚠️ 字符串被截断，含垃圾字符"（与"） · env=`GMI_API_KEY`
- **变体**：与 `gmicloud` 为同一服务（同一 base_url/env），建议合并

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | C | https://docs.litellm.ai/docs/providers/gmi |
| 能力支持 | 无差异（OpenAI-compatible drop-in，支持 /chat/completions） | - | C | https://docs.litellm.ai/docs/providers/gmi |
| 思考机制 | 未发现文档化差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer `GMI_API_KEY`） | - | C | https://docs.gmicloud.ai/quickstart |
| URL/端点 | ⚠️ registry base_url 截断损坏；正确端点为 `https://api.gmi-serving.com/v1` | `client = OpenAI(base_url="https://api.gmi-serving.com/v1")` | C | https://docs.gmicloud.ai/quickstart |
| 模型 ID | 约定为 `gmi/<vendor>/<model>`（如 `gmi/openai/gpt-5.6-sol`） | `"model": "gmi/openai/gpt-5.6-sol"` | C | https://docs.openclaw.ai/providers/gmi |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖（协议无差异；registry base_url 数据损坏）
- **aimux 代码位置**：`openai_compat_registry.rs:835-843`
- **差距说明**：base_url 字符串含截断垃圾"（与"；与 gmicloud 重复声明同一服务。
- **建议动作**：修正/合并 registry 条目（保留 gmicloud，删除 gmi 或统一 base_url）。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：⚠️ registry 数据损坏（非协议差异）

### gmicloud — GMI Cloud

- **registry 现状**：profile=`full()` · base_url=`https://api.gmi-serving.com/v1` · env=`GMI_API_KEY`
- **变体**：`gmi` 为同一服务（registry 重复声明）

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | C | https://docs.gmicloud.ai/quickstart |
| 能力支持 | 无差异（OpenAI-compatible drop-in） | - | C | https://docs.litellm.ai/docs/providers/gmi |
| 思考机制 | 未发现文档化差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer `GMI_API_KEY`） | - | C | https://docs.gmicloud.ai/quickstart |
| URL/端点 | 无差异 | - | C | https://docs.gmicloud.ai/quickstart |
| 模型 ID | `gmi/<vendor>/<model>` 形式（透传即可） | - | C | https://docs.openclaw.ai/providers/gmi |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:844-852`
- **差距说明**：无差异。
- **建议动作**：无需动作（可考虑与 gmi 合并）。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：-

### gonka24 — Gonka24

- **registry 现状**：profile=`full()` · base_url=`https://api.gonka24.com/v1` · env=`GONKA24_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（OpenAI 兼容网关） | - | C | https://llmgateway.io/providers/gonka24 |
| 能力支持 | 无差异（提供 2 个开源权重模型） | - | C | https://llmgateway.io/providers/gonka24 |
| 思考机制 | 无法确认（资料极少） | - | ⚠️ | - |
| 流式/usage | 无差异（OpenAI 兼容） | - | C | https://llmgateway.io/providers/gonka24 |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer key，官方站点 gonka.ai 提供 quickstart） | - | C | https://gonka.ai/docs/developer/quickstart/ |
| URL/端点 | 无差异 | - | C | https://llmgateway.io/providers/gonka24 |
| 模型 ID | 无差异（开源模型 ID 透传） | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖（证据较薄）
- **aimux 代码位置**：`openai_compat_registry.rs:853-861`
- **差距说明**：第三方资料确认 OpenAI 兼容，无特殊 request 配置。
- **建议动作**：无需动作。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：⚠️ 资料少，思考机制类未确认

### gradient_ai — Gradient AI

- **registry 现状**：profile=`full()` · base_url=`https://inference.do-ai.run/v1` · env=`GRADIENT_API_KEY`
- **变体**：DigitalOcean（batch-02）使用同一端点族（`inference.do-ai.run`）

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | C | https://www.digitalocean.com/community/tutorials/serverless-inference-openai-sdk |
| 能力支持 | 无差异（OpenAI API 兼容，支持 /v1/chat/completions） | `base_url="https://inference.do-ai.run/v1/"` | C | https://www.digitalocean.com/community/tutorials/serverless-inference-openai-sdk |
| 思考机制 | 未发现文档化差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer token） | - | C | https://github.com/anomalyco/models.dev/issues/1317 |
| URL/端点 | 无差异（`https://inference.do-ai.run/v1`） | - | C | https://www.digitalocean.com/community/tutorials/serverless-inference-openai-sdk |
| 模型 ID | 无差异（GPT-OSS 120b 等标准 ID） | - | C | https://www.digitalocean.com/community/tutorials/serverless-inference-openai-sdk |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:862-870`
- **差距说明**：即 DigitalOcean Gradient AI 平台的 OpenAI 兼容端点，无差异。
- **建议动作**：无需动作。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：-

### groq — Groq

- **registry 现状**：profile=`groq()`（supports_top_k=false · stream_usage_key=`x_groq`） · base_url=`https://api.groq.com/openai/v1` · env=`GROQ_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 🔶 `max_tokens` 已弃用，官方文档统一用 `max_completion_tokens`（API reference 注明 "Deprecated in favor of max_completion_tokens"）；aimux 对非 reasoning 模型发 `max_tokens` | `{"model":"llama-3.3-70b-versatile","max_completion_tokens":1024,"stream":true}` | C | https://console.groq.com/docs/text-chat ；https://console.groq.com/docs/api-reference |
| 能力支持 | 🔶 top_k 不支持（profile 已处理）；Structured Outputs 分 strict(true)/best-effort(false) 两档，`strict:true` 仅限部分模型（gpt-oss-20b/120b）；response_format 与流式/工具调用互斥 | `{"response_format":{"type":"json_schema","json_schema":{"name":"...","strict":true,"schema":{...}}}}` | C | https://console.groq.com/docs/structured-outputs |
| 思考机制 | 🔶 `reasoning_format` 字段（aimux 已实现）；新模型 ID 带 `openai/` 前缀（`openai/gpt-oss-120b`） | `{"reasoning_format":"raw"}` | C | https://console.groq.com/docs/text-chat |
| 流式/usage | ✅ usage 在 SSE 的 `x_groq.usage` 顶层（非顶层 `usage`）；aimux profile 已覆盖；Groq 不支持 `stream_options.include_usage`，aimux 已跳过（convert.rs:1106-1108） | `data: {... "x_groq":{"usage":{...}}}` | C | https://console.groq.com/docs/text-chat |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 🔶 `service_tier` 透传（aimux 已实现 convert.rs:1332-1336）；`parallel_tool_calls`、`tool_choice` 标准 | - | C | https://console.groq.com/docs/api-reference |
| headers/认证 | 无差异（Bearer GROQ_API_KEY） | - | - | - |
| URL/端点 | 无差异 | - | - | - |
| 模型 ID | 无差异（`llama-3.3-70b-versatile`、`openai/gpt-oss-20b` 等，透传即可） | - | C | https://console.groq.com/docs/text-chat |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖（max_tokens 弃用方向与 groq profile 有小冲突）
- **aimux 代码位置**：`openai/mod.rs:71-80`（groq() profile）· `convert.rs:1106-1108`（跳过 stream_options）· `convert.rs:1239-1250`（structuredOutputs/strictJsonSchema）· `convert.rs:1320-1324`（reasoning_format）· `convert.rs:1332-1336`（service_tier 透传）· `model.rs:510-518`（x_groq usage）
- **差距说明**：① 非 reasoning 模型路径下 aimux 发送 `max_tokens`，Groq 官方已弃用（仍接受，未移除）；② structured outputs 的 strict 档位语义已用 `structuredOutputs/strictJsonSchema` 表达，覆盖良好。
- **建议动作**：convert.rs 为 groq 补充"非 reasoning 也发 `max_completion_tokens`"（或 profile 增加 `max_tokens_key` 字段）；补 A 级测试。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证（现有 groq 行为有 aimux 单测覆盖，但 max_tokens 弃用方向未测）
- **存疑标记**：⚠️ max_tokens 仍被 Groq 接受（弃用未移除），影响为渐进性

### helicone — Helicone

- **registry 现状**：profile=`full()` · base_url=`https://api.helicone.ai/v1` · env=`HELICONE_API_KEY`
- **变体**：区域端点 `eu.api.helicone.ai`；OpenAI 兼容代理端点 `oai.helicone.ai`

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（透传上游 OpenAI 参数） | - | C | https://docs.helicone.ai/integrations/openai/python |
| 能力支持 | 无差异（透传） | - | - | - |
| 思考机制 | 无差异（透传） | - | - | - |
| 流式/usage | 无差异（透传） | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 🔶 支持 Helicone 专属头：`Helicone-Auth`、`Helicone-OpenAI-Api-Base`（代理到指定上游）、`Helicone-Property-*` 等；用于日志/缓存/属性 | `default_headers={"Helicone-Auth": f"Bearer {helicone_api_key}"}` | C | https://docs.helicone.ai/integrations/openai/python ；https://docs.helicone.ai/helicone-headers/header-directory |
| headers/认证 | 🔶 双 key：`api_key` 放你的**上游 key**（如 OPENAI_API_KEY），Helicone 自身 key 走 `Helicone-Auth` 头 | `client = OpenAI(api_key=openai_api_key, base_url="https://oai.helicone.ai/v1", default_headers={"Helicone-Auth": "Bearer <HELICONE_API_KEY>"})` | C | https://docs.helicone.ai/integrations/openai/python |
| URL/端点 | 无差异（registry 的 `api.helicone.ai/v1` 可用；官方 OpenAI SDK 示例用 `oai.helicone.ai/v1`） | - | C | https://docs.helicone.ai/integrations/openai/python |
| 模型 ID | 无差异（上游模型 ID） | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖（双 key 认证需手动配置）
- **aimux 代码位置**：`openai/mod.rs:96-119`（`OpenAIConfig.headers` 支持附加头）
- **差距说明**：aimux 单 key（Authorization=HELICONE_API_KEY）会直接调 Helicone 网关，日志归属 Heliicone 而非上游；要透传上游需在 config.headers 手加 `Helicone-OpenAI-Api-Base` 与 `Helicone-Auth`——可行但需要文档化。
- **建议动作**：无需 profile 改动；在 provider 文档标注 Helicone 双 key 用法（headers 配置）。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：-

### heroku — Heroku AI

- **registry 现状**：profile=`full()` · base_url=`https://api.heroku.com/inference/v1` · env=`HEROKU_API_KEY`
- **变体**：区域端点（如 `https://us.inference.heroku.com`）；`INFERENCE_URL`/`INFERENCE_KEY` 由 add-on 注入

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 🔶 官方支持表只有 `max_completion_tokens`（无 `max_tokens`）；aimux 对非 reasoning 模型发 `max_tokens` 会命中"未识别参数"报错 | `{"model": "...", "max_completion_tokens": 1024}` | C | https://devcenter.heroku.com/articles/openai-compatibility-chat-completions |
| 能力支持 | 🔶 大量参数被**忽略**但仍接受：`response_format`、`store`、`metadata`、`prediction`、`service_tier`、`safety_identifier`、`frequency_penalty`、`logit_bias`、`n`、`audio`、`verbosity` 等；`top_k` 不在支持表内 | - | C | https://devcenter.heroku.com/articles/openai-compatibility-chat-completions |
| 思考机制 | 🔶 Extended Thinking：支持 `reasoning_effort`（low/medium/high 映射固定预算），或专有 `extended_thinking` 对象（enabled/budget_tokens/include_reasoning）；Claude 系列模型 | `extra_body={"extended_thinking":{"enabled":True,"budget_tokens":2000,"include_reasoning":False}}` | C | https://devcenter.heroku.com/articles/openai-compatibility-chat-completions |
| 流式/usage | ✅ `stream_options` 支持；usage 标准 | - | C | https://devcenter.heroku.com/articles/openai-compatibility-chat-completions |
| 消息格式 | 无差异（developer/system/user/assistant/tool；user 不支持 audio/file 内容类型） | - | C | https://devcenter.heroku.com/articles/openai-compatibility-chat-completions |
| 特殊字段 | 🔶 未识别参数默认**报错**，需 `allow_ignored_params=true` 才静默忽略 | `extra_body={"allow_ignored_params": True}` | C | https://devcenter.heroku.com/articles/openai-compatibility-chat-completions |
| headers/认证 | 无差异（Bearer `INFERENCE_KEY`） | - | C | https://devcenter.heroku.com/articles/openai-compatibility-chat-completions |
| URL/端点 | ⚠️ registry base_url=`api.heroku.com/inference/v1` 与官方文档不符；官方为 add-on 注入的 `INFERENCE_URL`（如 `https://us.inference.heroku.com`）+ `/v1/` | `base_url = INFERENCE_URL + "/v1/"` | C | https://devcenter.heroku.com/articles/openai-compatibility-chat-completions |
| 模型 ID | 需用 Heroku 模型名（见 elements.heroku.com/addons/heroku-inference），与 OpenAI 官方 ID 不同 | - | C | https://devcenter.heroku.com/articles/openai-compatibility-chat-completions |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖 + ⚠️ base_url 不一致
- **aimux 代码位置**：`convert.rs:1118-1138`（max_tokens 分支）、`openai_compat_registry.rs:889-897`
- **差距说明**：① aimux 发 `max_tokens`+`top_k`，Heroku 未列入支持表→默认 400（除非 allow_ignored_params）；② `extended_thinking`/`allow_ignored_params` 需 bodyOverrides；③ base_url 错误。
- **建议动作**：profile 增加"非 reasoning 也用 max_completion_tokens"能力（与 groq 同源）；base_url 改为 add-on 注入形式并文档化；其余用 bodyOverrides 兜底。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：⚠️ max_tokens 不在支持表≠一定报错（表中未列"忽略"），建议实测

### hetzner — Hetzner

- **registry 现状**：profile=`full()` · base_url=`https://inference.hetzner.com/api/v1` · env=`HETZNER_VLLM_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | C | https://sliplane.io/blog/hetzner-inference |
| 能力支持 | 无差异（vLLM OpenAI 兼容；当前仅 1 个模型，支持文本+图像） | - | C | https://sliplane.io/blog/hetzner-inference |
| 思考机制 | 🔶 vLLM 的 `chat_template_kwargs.enable_thinking` 可关思考（Qwen 模板）；Hetzner 未官方文档化，社区实测有效 | `extra_body={"chat_template_kwargs":{"enable_thinking":False}}` | C | https://sliplane.io/blog/hetzner-inference |
| 流式/usage | 无差异（标准 SSE） | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异（实验性服务，无计费/SLA） | - | C | https://sliplane.io/blog/hetzner-inference |
| headers/认证 | 无差异（Bearer `HETZNER_VLLM_API_KEY`） | - | C | https://sliplane.io/blog/hetzner-inference |
| URL/端点 | 无差异 | - | C | https://sliplane.io/blog/hetzner-inference |
| 模型 ID | `org/model` 形式（`Qwen/Qwen3.6-35B-A3B-FP8`） | `"model": "Qwen/Qwen3.6-35B-A3B-FP8"` | C | https://sliplane.io/blog/hetzner-inference |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖（enable_thinking 需 bodyOverrides）
- **aimux 代码位置**：`openai_compat_registry.rs:898-906`
- **差距说明**：`chat_template_kwargs` 不在白名单，需 bodyOverrides；服务为实验性质（无 SLA）。
- **建议动作**：无需 profile 改动；bodyOverrides 兜底即可，文档标注"实验性"。

#### 3. 证据与验证

- **证据等级**：C（第三方实测，非官方文档）
- **验证状态**：🔲 未验证
- **存疑标记**：⚠️ enable_thinking 未获官方文档背书

### hosted_vllm — Hosted vLLM

- **registry 现状**：profile=`full()` · base_url=`https://hosted-vllm-api.com/v1` · env=`HOSTED_VLLM_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（vLLM OpenAI 兼容） | - | ⚠️ | - |
| 能力支持 | 无差异（支持 reasoning_effort 透传） | `reasoning_effort="medium"` + `allowed_openai_params=["reasoning_effort"]` | C | https://github.com/BerriAI/litellm/issues/18543 |
| 思考机制 | 无法确认（仅见 litellm issue 中作为演示端点出现） | - | ⚠️ | - |
| 流式/usage | 无差异（vLLM SSE） | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无法确认 | - | ⚠️ | - |
| headers/认证 | 无法确认 | - | ⚠️ | - |
| URL/端点 | ⚠️ `hosted-vllm-api.com` 未找到官方站点/文档，仅作为 litellm 示例端点出现；可能为演示/占位域名 | - | ⚠️ | https://github.com/BerriAI/litellm/issues/18543 |
| 模型 ID | 无差异（vLLM 标准模型 ID） | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：⚠️ 证据不足
- **aimux 代码位置**：`openai_compat_registry.rs:907-915`
- **差距说明**：域名本身缺乏权威资料；按 vLLM OpenAI 兼容处理是合理默认，但无证据。
- **建议动作**：标记"证据不足"；若为 vLLM 则 reasoning_effort/chat_template_kwargs 走 bodyOverrides。

#### 3. 证据与验证

- **证据等级**：C（仅 litellm issue 提及）
- **验证状态**：🔲 未验证
- **存疑标记**：⚠️ 端点真实性/协议未确认

### hpc_ai — HPC-AI

- **registry 现状**：profile=`full()` · base_url=`https://api.hpc-ai.com/inference/v1` · env=`INFERENCE_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | C | https://www.hpc-ai.com/doc/docs/Model-APIs/Integration/CC%20Switch/ |
| 能力支持 | 无差异（OpenAI 兼容，模型如 `zai-org/glm-5.1`） | `"model": "zai-org/glm-5.1"` | C | https://www.hpc-ai.com/doc/docs/Model-APIs/Integration/CC%20Switch/ |
| 思考机制 | 未发现文档化差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer key） | - | C | https://www.hpc-ai.com/blog/Helicone-Integration |
| URL/端点 | 无差异 | - | C | https://www.hpc-ai.com/blog/Helicone-Integration |
| 模型 ID | `org/model` 形式（`zai-org/glm-5.1`） | - | C | https://www.hpc-ai.com/doc/docs/Model-APIs/Integration/CC%20Switch/ |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:916-924`
- **差距说明**：纯 OpenAI 兼容。
- **建议动作**：无需动作。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：-

### hyperbolic — Hyperbolic

- **registry 现状**：profile=`full()` · base_url=`https://api.hyperbolic.xyz/v1` · env=`HYPERBOLIC_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（官方示例用 `max_tokens`） | `{"model":"deepseek-ai/DeepSeek-R1","max_tokens":512,"temperature":0.7}` | C | https://www.hyperbolic.ai/docs/inference/text-apis |
| 能力支持 | 🔶 工具调用仅限部分模型（`meta-llama/Llama-3.3-70B-Instruct`、`Qwen/Qwen3-Coder-480B-A35B-Instruct`）；参数表仅列 model/messages/max_tokens/temperature/top_p/stream/stop | - | C | https://www.hyperbolic.ai/docs/inference/text-apis |
| 思考机制 | 🔶 提供 DeepSeek-R1 等推理模型（模型侧思考），请求侧未文档化专用字段 | `"model": "deepseek-ai/DeepSeek-R1"` | C | https://www.hyperbolic.ai/docs/inference/text-apis |
| 流式/usage | 无差异（标准 SSE，顶层 usage） | - | C | https://www.hyperbolic.ai/docs/inference/text-apis |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer HYPERBOLIC_API_KEY） | - | C | https://www.hyperbolic.ai/docs/inference/text-apis |
| URL/端点 | 无差异 | - | C | https://www.hyperbolic.ai/docs/inference/text-apis |
| 模型 ID | `org/model` 形式（`deepseek-ai/DeepSeek-R1`、`meta-llama/Llama-3.3-70B-Instruct`） | - | C | https://www.hyperbolic.ai/docs/inference/text-apis |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:925-933`
- **差距说明**：request 构造无差异；模型 ID 透传；工具限制是模型级而非协议级。
- **建议动作**：无需动作。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：-

### iflowcn — iFlow

- **registry 现状**：profile=`full()` · base_url=`https://apis.iflow.cn/v1（chat`（⚠️ 字符串被截断，含垃圾字符"（chat"） · env=`IFLOW_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（OpenAI 兼容） | - | C | https://platform.iflow.cn/cli/configuration/settings |
| 能力支持 | 无差异（支持 Qwen3-Coder、Kimi 等模型） | `"modelName": "Qwen3-Coder"` | C | https://platform.iflow.cn/cli/configuration/settings |
| 思考机制 | 未发现文档化差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（`IFLOW_apiKey` Bearer） | `export IFLOW_apiKey="sk-..."` | C | https://platform.iflow.cn/cli/configuration/settings |
| URL/端点 | ⚠️ registry base_url 截断损坏；正确为 `https://apis.iflow.cn/v1` | `"baseUrl": "https://apis.iflow.cn/v1"` | C | https://platform.iflow.cn/cli/configuration/settings |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖（registry base_url 数据损坏）
- **aimux 代码位置**：`openai_compat_registry.rs:934-942`
- **差距说明**：base_url 含截断垃圾"（chat"；协议纯 OpenAI 兼容。
- **建议动作**：修正 registry base_url。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：⚠️ registry 数据损坏（非协议差异）

### inception — Inception Labs

- **registry 现状**：profile=`full()` · base_url=`https://api.inceptionlabs.ai/v1` · env=`INCEPTION_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（官方示例用 `max_tokens`） | `{"model":"mercury-coder-small","messages":[...],"max_tokens":100}` | C | https://www.inceptionlabs.ai/blog/introducing-inception-api |
| 能力支持 | 🔶 Mercury 2 支持 Tool Calling + Structured Outputs；响应侧（流式）注意 diffusing 噪声块不产生计费 token | - | C | https://docs.inceptionlabs.ai/get-started/models |
| 思考机制 | 🔶 Mercury 2 自称 "fastest reasoning LLM"，但请求侧无文档化思考开关字段 | - | C | https://docs.inceptionlabs.ai/get-started/models |
| 流式/usage | 无差异（支持流式） | - | C | https://www.inceptionlabs.ai/blog/introducing-inception-api |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 🔶 专有 `diffusing` 布尔字段：开启时流式返回逐步去噪的噪声 token（可视化扩散过程，噪声 token 不计费） | `{"model":"mercury-coder-small","diffusing":true}` | C | https://www.inceptionlabs.ai/blog/introducing-inception-api |
| headers/认证 | 无差异（Bearer INCEPTION_API_KEY） | - | C | https://www.inceptionlabs.ai/blog/introducing-inception-api |
| URL/端点 | 🔶 除 /chat/completions 外还有 `/v1/fim/completions` 与 `/v1/edit/completions`（FIM/编辑端点，Mercury Edit 2） | `POST https://api.inceptionlabs.ai/v1/fim/completions` `{"model":"mercury-edit-2","prompt":"def fibonacci(","suffix":"return a+b","max_tokens":1000}` | C | https://docs.inceptionlabs.ai/get-started/models |
| 模型 ID | `mercury-coder-small`、`mercury-2`、`mercury-edit-2` 等专有 ID | - | C | https://docs.inceptionlabs.ai/get-started/models |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖（diffusing/FIM 需 bodyOverrides）
- **aimux 代码位置**：`openai_compat_registry.rs:943-951`
- **差距说明**：`diffusing` 不在白名单（bodyOverrides 可注入）；FIM/Edit 端点为独立 API 面，aimux chat 流不覆盖。
- **建议动作**：diffusing 用 bodyOverrides 兜底；FIM 端点列入后续能力评估（非本 9 类范围）。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：-

### inceptron — Inceptron

- **registry 现状**：profile=`full()` · base_url=`https://api.inceptron.io/v1` · env=`INCEPTRON_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | C | https://www.inceptron.io/ |
| 能力支持 | 无差异（"deploy any model"，OpenAI 兼容 chat completions） | `curl https://api.inceptron.io/v1/chat/completions` | C | https://www.inceptron.io/ |
| 思考机制 | 未发现文档化差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer key） | - | - | - |
| URL/端点 | 无差异 | - | C | https://www.inceptron.io/ |
| 模型 ID | 无差异（可部署任意模型） | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:952-960`
- **差距说明**：OpenAI 兼容，无特殊 request 配置。
- **建议动作**：无需动作。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：-

### inference_net — Inference.net

- **registry 现状**：profile=`full()` · base_url=`https://api.inference.net/v1` · env=`INFERENCE_NET_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（支持表：model/messages/stream/max_tokens/temperature/top_p/frequency_penalty/presence_penalty/response_format/tools） | `{"model":"glm-5.2","stream":true}` | C | https://docs.inference.net/api/api-quickstart |
| 能力支持 | 无差异（response_format 支持 json_object 与 json_schema；工具调用支持） | - | C | https://docs.inference.net/api/api-quickstart |
| 思考机制 | 未发现文档化差异（平台侧支持推理模型） | - | - | - |
| 流式/usage | 无差异 | - | C | https://docs.inference.net/api/api-quickstart |
| 消息格式 | 无差异（另支持 Anthropic Messages 格式，非 OpenAI 侧） | - | C | https://docs.inference.net/api/api-quickstart |
| 特殊字段 | 🔶 代理（Catalyst）模式下有专有请求头：`x-inference-provider`（路由到 openai/anthropic/groq/cerebras 等）、`x-inference-provider-api-key`（下游 key）、`x-inference-provider-url`（按 base_url 路由）、`x-inference-environment`、`x-inference-task-id`、`x-inference-metadata-*` | `headers={"x-inference-provider":"openai","x-inference-provider-api-key":"<OPENAI_KEY>"}` `"model":"gpt-4.1"` | C | https://docs.inference.net/api/api-quickstart |
| headers/认证 | 无差异（`Authorization: Bearer <INFERENCE_API_KEY>`；注意 registry env 名 `INFERENCE_NET_API_KEY`，官方文档用 `INFERENCE_API_KEY`） | - | C | https://docs.inference.net/api/api-quickstart |
| URL/端点 | 无差异 | - | C | https://docs.inference.net/api/api-quickstart |
| 模型 ID | 混合：开源模型裸 ID（`glm-5.2`）、闭源模型 `claude-haiku-4-5`/`gpt-5-mini`/`gemini-3.5-flash`、自部署 `your-team/your-model` | - | C | https://docs.inference.net/api/api-quickstart |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖（x-inference-* 头需 config.headers 手动配置）
- **aimux 代码位置**：`openai_compat_registry.rs:961-969`；`openai/mod.rs:154-158`（`with_headers`）
- **差距说明**：普通 serverless 调用无差异；Catalyst 代理模式需在 provider config 加 x-inference-* 头（aimux `OpenAIConfig.headers` 支持，但 profile 层无法表达）。
- **建议动作**：无需 profile 改动；文档化 headers 配置。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：-

### inferx — InferX

- **registry 现状**：profile=`full()` · base_url=`https://model.inferx.net/v1` · env=`INFERX_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | ⚠️ 无法确认（查不到资料） | - | ⚠️ | - |
| 能力支持 | ⚠️ 无法确认 | - | ⚠️ | - |
| 思考机制 | ⚠️ 无法确认 | - | ⚠️ | - |
| 流式/usage | ⚠️ 无法确认 | - | ⚠️ | - |
| 消息格式 | ⚠️ 无法确认 | - | ⚠️ | - |
| 特殊字段 | ⚠️ 无法确认 | - | ⚠️ | - |
| headers/认证 | ⚠️ 无法确认 | - | ⚠️ | - |
| URL/端点 | ⚠️ 无法确认（`model.inferx.net` 无公开文档） | - | ⚠️ | - |
| 模型 ID | ⚠️ 无法确认 | - | ⚠️ | - |

#### 2. aimux 现状对比

- **对比结论**：⚠️ 证据不足，无法对比
- **aimux 代码位置**：`openai_compat_registry.rs:970-978`
- **差距说明**：全库未查到该厂商资料。
- **建议动作**：维持 full()，标记证据不足。

#### 3. 证据与验证

- **证据等级**：无
- **验证状态**：🔲 未验证
- **存疑标记**：⚠️ 存疑——查不到任何厂商信息

### infinity — Infinity AI

- **registry 现状**：profile=`full()` · base_url=`https://infinity.ai/api/v1` · env=`INFINITY_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | ⚠️ 无法确认 | - | ⚠️ | - |
| 能力支持 | ⚠️ 无法确认（注意 "Infinity" 常见指向向量库/嵌入服务，与 registry 的 LLM 端点关系不明） | - | ⚠️ | - |
| 思考机制 | ⚠️ 无法确认 | - | ⚠️ | - |
| 流式/usage | ⚠️ 无法确认 | - | ⚠️ | - |
| 消息格式 | ⚠️ 无法确认 | - | ⚠️ | - |
| 特殊字段 | ⚠️ 无法确认 | - | ⚠️ | - |
| headers/认证 | ⚠️ 无法确认 | - | ⚠️ | - |
| URL/端点 | ⚠️ 无法确认（`infinity.ai` 无匹配的 LLM API 文档） | - | ⚠️ | - |
| 模型 ID | ⚠️ 无法确认 | - | ⚠️ | - |

#### 2. aimux 现状对比

- **对比结论**：⚠️ 证据不足，无法对比
- **aimux 代码位置**：`openai_compat_registry.rs:979-987`
- **差距说明**：查不到对应 LLM 服务文档；可能是名称歧义（向量库/嵌入服务）。
- **建议动作**：标记证据不足，建议人工确认该条目的服务性质。

#### 3. 证据与验证

- **证据等级**：无
- **验证状态**：🔲 未验证
- **存疑标记**：⚠️ 存疑——名称歧义，无法确认对应服务

### io_net — IO.NET

- **registry 现状**：profile=`full()` · base_url=`https://api.intelligence.io.solutions/api/v1` · env=`IOINTELLIGENCE_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | C | https://io.net/blog/what-is-io-intelligence-unlocking-the-power-of-ai-driven-insights |
| 能力支持 | 无差异（15+ 开源模型，OpenAI 兼容 chat completions） | - | C | https://www.truefoundry.com/docs/ai-gateway/io-net |
| 思考机制 | 未发现文档化差异 | - | - | - |
| 流式/usage | 无差异（/v1/models、/v1/chat/completions 标准） | - | C | https://github.com/api-evangelist/io-net |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer `$IOINTELLIGENCE_API_KEY`） | - | C | https://io.net/blog/what-is-io-intelligence-unlocking-the-power-of-ai-driven-insights |
| URL/端点 | 无差异 | - | C | https://io.net/blog/what-is-io-intelligence-unlocking-the-power-of-ai-driven-insights |
| 模型 ID | 无差异（开源模型 ID） | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:988-996`
- **差距说明**：纯 OpenAI 兼容。
- **建议动作**：无需动作。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：-

### jiekou — Jiekou.AI

- **registry 现状**：profile=`full()` · base_url=`https://api.highwayapi.ai/openai` · env=`JIEKOU_API_KEY`
- **变体**：海外端点 `https://api.jiekou.ai/openai`

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（OpenAI 兼容协议） | - | C | https://jiekou.ai/blog/jiekou-ai-quickstart |
| 能力支持 | 无差异（聚合站，兼容 OpenAI SDK） | `client = OpenAI(base_url="https://api.highwayapi.ai/openai")`（官方示例） | C | https://jiekou.ai/blog/jiekou-ai-quickstart |
| 思考机制 | 无差异（透传上游模型能力） | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer key） | - | C | https://docs.jiekou.ai/docs/announcement/announcement |
| URL/端点 | 无差异（国内直连 `https://api.highwayapi.ai/openai`；海外 `https://api.jiekou.ai/openai`） | - | C | https://docs.jiekou.ai/docs/announcement/announcement |
| 模型 ID | `vendor-model` 形式（`claude-opus-4-7`、`minimax-minimax-m2.7`） | - | C | https://jiekou.ai/models-console/model-detail/minimax-minimax-m2.7 |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:997-1005`
- **差距说明**：纯 OpenAI 兼容聚合站。
- **建议动作**：无需动作。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：-

### kenari — Kenari

- **registry 现状**：profile=`full()` · base_url=`https://kenari.id/v1` · env=`KENARI_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | C | https://models.dev/providers/kenari |
| 能力支持 | 无差异（38+ 模型：Claude、GPT-5、DeepSeek、Qwen、Kimi、GLM、MiniMax 等；支持工具调用/结构化输出/推理） | - | C | https://models.dev/providers/kenari |
| 思考机制 | 无差异（透传上游） | - | - | - |
| 流式/usage | 无差异 | - | C | https://models.dev/providers/kenari |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（`Authorization: Bearer kn-...`） | - | C | https://github.com/diegosouzapw/OmniRoute/blob/release/v3.8.50/docs/reference/PROVIDER_REFERENCE.md |
| URL/端点 | 无差异 | - | C | https://models.dev/providers/kenari |
| 模型 ID | 无差异（上游模型 ID 透传） | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:1006-1014`
- **差距说明**：OpenAI 兼容网关。
- **建议动作**：无需动作。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：-

### kilo — Kilo

- **registry 现状**：profile=`full()` · base_url=`https://api.kilo.ai/v1` · env=`KILO_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | C | https://kilo.ai/docs/gateway/sdks-and-frameworks |
| 能力支持 | 无差异（Kilo Code API Gateway，OpenAI 兼容） | - | C | https://kilo.ai/docs/gateway/sdks-and-frameworks |
| 思考机制 | 无法确认（网关透传） | - | ⚠️ | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（`Authorization: Bearer $KILO_API_KEY`） | - | C | https://kilo.ai/docs/gateway/sdks-and-frameworks |
| URL/端点 | ⚠️ registry base_url=`https://api.kilo.ai/v1`；官方文档 chat completions 端点为 `https://api.kilo.ai/api/gateway/chat/completions`（前缀 `/api/gateway`），`/v1` 是否可用未确认 | `POST https://api.kilo.ai/api/gateway/chat/completions` | C | https://kilo.ai/docs/gateway/sdks-and-frameworks |
| 模型 ID | 无差异（透传） | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：⚠️ base_url 可能不一致（+✅ 协议无差异）
- **aimux 代码位置**：`openai_compat_registry.rs:1015-1023`
- **差距说明**：官方文档端点含 `/api/gateway` 前缀；registry 的 `/v1` 是否可用存疑。
- **建议动作**：核实 base_url（大概率应为 `https://api.kilo.ai/api/gateway`）；协议层无需动作。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：⚠️ `/v1` 路径正确性未确认

### kimi — Kimi

- **registry 现状**：profile=`full()` · base_url=`https://api.moonshot.ai/v1` · env=`MOONSHOT_API_KEY`
- **变体**：国内端点 `https://api.moonshot.cn/v1`；`kimi_for_coding` 为另一服务（见下条）

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（`max_tokens` 与 `max_completion_tokens` 官方均接受；文档示例混用） | `{"model":"kimi-k2.6","max_tokens":1024*32,"stream":true}` | C | https://platform.kimi.ai/docs/api/chat ；https://platform.kimi.ai/docs/guide/use-thinking-models |
| 能力支持 | ✅ response_format（text/json_object/json_schema）与 tools 标准支持；思考模型 temperature 不可调 | - | C | https://platform.kimi.ai/docs/api/chat |
| 思考机制 | 🔶 **by-model 三套机制**：① `kimi-k3`：永远思考，顶层 `reasoning_effort`（`"low"/"high"/"max"`，默认 `"max"`），**不接受** `thinking` 参数；② `kimi-k2.7-code`：永远思考，`thinking.type` 仅允许 `"enabled"`，传 `"disabled"` 报错；③ `kimi-k2.6`/`k2.5`：`thinking.type` `"enabled"(默认)/"disabled"`，k2.6 另有 `thinking.keep`（`null` 默认 / `"all"` 保留历史 reasoning_content） | `{"model":"kimi-k2.6","thinking":{"type":"disabled"}}` · `{"model":"kimi-k3","reasoning_effort":"high"}` | C | https://platform.kimi.ai/docs/guide/use-thinking-models |
| 流式/usage | ✅ 流式 usage 顶层返回，含 `cached_tokens`；`reasoning_content` delta 先于 `content` 输出 | `data: {... "usage":{"prompt_tokens":19,"completion_tokens":13,"total_tokens":32,"cached_tokens":12}}` | C | https://platform.kimi.ai/docs/api/chat |
| 消息格式 | 🔶 响应 `message.reasoning_content` 与 `content` 同级；多轮必须原样回传历史 assistant 的 `reasoning_content`（Preserved Thinking）；多模态 content 支持 `image_url`/`video_url` | `{"role":"assistant","content":"...","reasoning_content":"..."}` | C | https://platform.kimi.ai/docs/guide/use-thinking-models |
| 特殊字段 | 无差异（无 cache/safety 类请求字段） | - | - | - |
| headers/认证 | 无差异（Bearer MOONSHOT_API_KEY） | - | C | https://platform.kimi.ai/docs/api/chat |
| URL/端点 | 无差异（`https://api.moonshot.ai/v1/chat/completions`） | - | C | https://platform.kimi.ai/docs/api/chat |
| 模型 ID | `kimi-k3`、`kimi-k2.7-code`、`kimi-k2.7-code-highspeed`、`kimi-k2.6`、`kimi-k2.5` 等；国内平台另有 `moonshot-v1-*` | - | C | https://platform.kimi.ai/docs/guide/use-thinking-models |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖
- **aimux 代码位置**：`openai/mod.rs:35-49`（profile 无 thinking 字段）· `convert.rs:1326-1329`（reasoning_effort 直传）· `types.rs:37-40,146-149` + `model.rs:299-302,558-561`（reasoning_content 解析）· `types.rs:69-74` + `model.rs:127-134`（Moonshot 顶层 cached_tokens 已支持）· `convert.rs:684-738`（assistant reasoning_content 回放，支持 Preserved Thinking）
- **差距说明**：① `thinking.type/thinking.keep` 不在白名单，需 bodyOverrides 注入；② kimi-k3 的 reasoning_effort 与 OpenAI 语义一致，aimux 直传可用；③ reasoning_content 解析/回放与顶层 cached_tokens 已覆盖 ✅。
- **建议动作**：评估 profile 新字段 `thinking`（或文档化 bodyOverrides 模板）；为 kimi 补 A 级测试（cached_tokens + reasoning_content 流式）。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证（aimux 的 cached_tokens/reasoning_content 逻辑有单测，但无 kimi 线上 cassette）
- **存疑标记**：-

### kimi_for_coding — Kimi For Coding

- **registry 现状**：profile=`full()` · base_url=`https://api.kimi.com/coding/v1` · env=`KIMI_API_KEY`
- **变体**：Anthropic 兼容端点 `https://api.kimi.com/coding/`（非 OpenAI 面）

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（OpenAI 兼容；`max_completion_tokens` 为标准） | - | C | https://www.kimi.com/code/docs/en/kimi-code/models |
| 能力支持 | 🔶 部分参数报 400：未知 `reasoning_effort` 取值直接 HTTP 400；`k3` 需工具侧手动设 context window 到 1048576 才用满 1M | - | C | https://www.kimi.com/code/docs/en/kimi-code/models |
| 思考机制 | 🔶 **`reasoning_effort` 取值映射表**（K3）：`null/undefined→high`；`ultra/max/xhigh→max`；`high/medium→high`；`low/minimum/light→low`；`none→thinking.type disabled`；**未知值→HTTP 400**。`kimi-for-coding`（K2.7 Code）为 `Thinking:ON`（恒思考） | `{"model":"k3","reasoning_effort":"medium"}` → 按 high 处理 | C | https://www.kimi.com/code/docs/en/kimi-code/models |
| 流式/usage | ✅ 自动上下文缓存（context cache），**切换模型 ID 使缓存失效**（重新 prefill 计费更高） | - | C | https://www.kimi.com/code/docs/en/kimi-code/models |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 🔶 K3 恒思考 + Preserved Thinking（切换 reasoning_effort 使缓存失效） | - | C | https://www.kimi.com/code/docs/en/kimi-code/models |
| headers/认证 | 无差异（Bearer KIMI_API_KEY，与 Moonshot Open Platform key 不通用） | - | C | https://www.kimi.com/code/docs/en/kimi-code/models |
| URL/端点 | 无差异（`https://api.kimi.com/coding/v1`；另有 Anthropic 面 `/coding/`） | - | C | https://www.kimi.com/code/docs/en/kimi-code/models |
| 模型 ID | 固定 4 个：`k3`、`k3-256k`、`kimi-for-coding`、`kimi-for-coding-highspeed`（填"模型版本名"如 `Kimi K3` 会调用失败；highspeed 拼错静默回退普通版） | `"model": "kimi-for-coding"` | C | https://www.kimi.com/code/docs/en/kimi-code/models |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖
- **aimux 代码位置**：`convert.rs:1326-1329`（reasoning_effort 直传，不做取值映射/400 校验）
- **差距说明**：① aimux 直传 `reasoning_effort`，若用户传 `ultra/xhigh/minimum/light` 等值，Kimi 端可接受（有映射），但 `medium`（OpenAI 标准档）在 kimi-k3 会映射为 high（语义 OK）；真正风险是**未知值 400**——aimux 不校验；② `thinking.type disabled`（reasoning_effort=none）需 bodyOverrides；③ 模型 ID 需严格透传。
- **建议动作**：文档化模型 ID 清单与 reasoning_effort 映射表；thinking disabled 用 bodyOverrides；不建议在 profile 层做映射（映射属于 Kimi 端行为）。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：-

### kiro — Kiro

- **registry 现状**：profile=`full()` · base_url=`https://api.kiro.dev/v1` · env=`KIRO_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | ⚠️ 无法确认 | - | ⚠️ | - |
| 能力支持 | ⚠️ 无法确认（kiro.dev 是 AWS 的 AI IDE/编码代理，提供跨区域推理；`api.kiro.dev/v1` 的 OpenAI 端点无文档） | - | ⚠️ | https://kiro.dev/ ；https://kiro.dev/docs/models/ |
| 思考机制 | ⚠️ 无法确认 | - | ⚠️ | - |
| 流式/usage | ⚠️ 无法确认 | - | ⚠️ | - |
| 消息格式 | ⚠️ 无法确认 | - | ⚠️ | - |
| 特殊字段 | ⚠️ 无法确认 | - | ⚠️ | - |
| headers/认证 | ⚠️ 无法确认 | - | ⚠️ | - |
| URL/端点 | ⚠️ 无法确认（`api.kiro.dev/v1` 未在 kiro.dev 文档中找到对应说明） | - | ⚠️ | https://kiro.dev/docs/models/ |
| 模型 ID | ⚠️ 无法确认 | - | ⚠️ | - |

#### 2. aimux 现状对比

- **对比结论**：⚠️ 证据不足，无法对比
- **aimux 代码位置**：`openai_compat_registry.rs:1042-1050`
- **差距说明**：kiro.dev 主体是 AWS AI IDE；`api.kiro.dev/v1` 端点无公开文档。
- **建议动作**：标记证据不足；如确认是 Kiro 的 OpenAI 兼容代理端点则维持 full()。

#### 3. 证据与验证

- **证据等级**：无
- **验证状态**：🔲 未验证
- **存疑标记**：⚠️ 存疑——端点与产品形态未确认

### kluster_ai — Kluster AI

- **registry 现状**：profile=`full()` · base_url=`https://api.kluster.ai/v1` · env=`KLUSTER_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（历史：OpenAI 兼容，`baseURL: "https://api.kluster.ai/v1/"`） | - | C | https://raw.githubusercontent.com/LibreChat-AI/librechat-config-yaml/main/librechat-env-f.yaml |
| 能力支持 | 无差异（历史：vLLM 系 OpenAI 兼容） | - | C | https://raw.githubusercontent.com/LibreChat-AI/librechat-config-yaml/main/librechat-env-f.yaml |
| 思考机制 | 无法确认 | - | ⚠️ | - |
| 流式/usage | 无差异（历史） | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（历史：Bearer KLUSTER_API_KEY） | - | C | https://raw.githubusercontent.com/LibreChat-AI/librechat-config-yaml/main/librechat-env-f.yaml |
| URL/端点 | ⚠️ **服务已转型**：官方 docs.kluster.ai 声明 "kluster.ai has joined MITO"，原 LLM 推理服务已随团队转做 AI 视频；inference API 大概率已停 | - | C | https://docs.kluster.ai/getting-started/quickstart |
| 模型 ID | 无法确认（历史：vLLM 标准 ID） | - | ⚠️ | - |

#### 2. aimux 现状对比

- **对比结论**：⚠️ 不一致（服务疑似已停）
- **aimux 代码位置**：`openai_compat_registry.rs:1051-1059`
- **差距说明**：docs.kluster.ai 首页声明团队并入 MITO（AI 视频），原推理服务状态存疑；API key 领取入口 `platform.kluster.ai/apikeys` 仍被第三方引用。
- **建议动作**：人工确认服务是否下线；若下线则移除/标注条目。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：⚠️ 服务存续状态存疑（2026-07 文档声明转型）

### krutrim — Krutrim

- **registry 现状**：profile=`full()` · base_url=`https://api.krutrim.ai/v1` · env=`KRUTRIM_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 🔶 官方参数表为 `max_tokens`（非 `max_completion_tokens`）：temperature/top_p/max_tokens/frequency_penalty/presence_penalty/logit_bias/stop/stream | - | C | https://docs.cloud.olakrutrim.com/basics/ai-studio/ai-jobs/inferencing.md |
| 能力支持 | 无差异（OpenAI-compatible，支持 openai SDK；模型按 KMS key 分组授权） | - | C | https://docs.cloud.olakrutrim.com/basics/ai-studio/ai-jobs/inferencing.md |
| 思考机制 | 未发现文档化差异 | - | - | - |
| 流式/usage | 无差异（支持 stream） | - | C | https://docs.cloud.olakrutrim.com/basics/ai-studio/ai-jobs/inferencing.md |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（`Authorization: Bearer <api-key>`；KMS 生成，模型组作用域） | `curl https://cloud.olakrutrim.com/v1/chat/completions -H "Authorization: Bearer ..."` | C | https://docs.cloud.olakrutrim.com/basics/key-management-system/model-api-keys |
| URL/端点 | ⚠️ registry base_url=`api.krutrim.ai/v1`；官方文档为 `https://cloud.olakrutrim.com/v1` | `client = OpenAI(api_key="...", base_url="https://cloud.olakrutrim.com/v1")` | C | https://docs.cloud.olakrutrim.com/basics/ai-studio/ai-jobs/inferencing.md |
| 模型 ID | 官方示例 `krutrim-1`；以 Model Card 中字符串为准（填错报 invalid model） | `"model": "krutrim-1"` | C | https://docs.cloud.olakrutrim.com/basics/ai-studio/ai-jobs/inferencing.md |

#### 2. aimux 现状对比

- **对比结论**：✅ 协议已覆盖（max_tokens 路径匹配 aimux 非 reasoning 分支）+ ⚠️ base_url 不一致
- **aimux 代码位置**：`convert.rs:1131-1137`（非 reasoning 发 max_tokens ✓）；`openai_compat_registry.rs:1060-1068`
- **差距说明**：Krutrim 用 `max_tokens`，aimux 非 reasoning 模型路径正好发 `max_tokens` ✓；但 base_url 与官方文档不符。
- **建议动作**：registry base_url 更新为 `https://cloud.olakrutrim.com/v1`。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：⚠️ api.krutrim.ai 是否为合法别名未确认（以官方 cloud.olakrutrim.com 为准）

### kuae_cloud_coding_plan — KUAE Cloud Coding Plan

- **registry 现状**：profile=`full()` · base_url=`https://coding-plan-endpoint.kuaecloud.net/v1` · env=`KUAE_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | C | https://mastra.ai/models/providers/kuae-cloud-coding-plan |
| 能力支持 | 无差异（@ai-sdk/openai-compatible） | - | C | https://models.dev/providers/kuae-cloud-coding-plan |
| 思考机制 | 无法确认（1 个模型：GLM-4.7 系） | - | ⚠️ | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer KUAE_API_KEY） | - | C | https://mastra.ai/models/providers/kuae-cloud-coding-plan |
| URL/端点 | 无差异 | - | C | https://models.dev/providers/kuae-cloud-coding-plan |
| 模型 ID | `kuae-cloud-coding-plan/GLM-4.7` 形式 | `"model": "kuae-cloud-coding-plan/GLM-4.7"` | C | https://mastra.ai/models/providers/kuae-cloud-coding-plan |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:1069-1077`
- **差距说明**：OpenAI 兼容。
- **建议动作**：无需动作。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：-

### lambda_ai — Lambda AI

- **registry 现状**：profile=`full()` · base_url=`https://api.lambda.ai/v1` · env=`LAMBDA_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | C | https://lambda.ai/blog/deepseek-r1-0528-on-lambda-inference-api |
| 能力支持 | 无差异（OpenAI 兼容 chat completions；/v1/models 可用） | - | C | https://docs.litellm.ai/docs/providers/lambda_ai |
| 思考机制 | 无差异（提供 DeepSeek-R1 等推理模型，请求侧无专用字段） | `model="deepseek-r1-0528"` | C | https://lambda.ai/blog/deepseek-r1-0528-on-lambda-inference-api |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer LAMBDA_API_KEY） | - | C | https://docs.litellm.ai/docs/providers/lambda_ai |
| URL/端点 | 无差异 | - | C | https://lambda.ai/blog/deepseek-r1-0528-on-lambda-inference-api |
| 模型 ID | 简短 ID（`deepseek-r1-0528`、`hermes-4-405b` 等） | - | C | https://lambda.ai/blog/deepseek-r1-0528-on-lambda-inference-api |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:1078-1086`
- **差距说明**：纯 OpenAI 兼容。
- **建议动作**：无需动作。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：-

### lemonade — Lemonade

- **registry 现状**：profile=`full()` · base_url=`http://localhost:13305/v1` · env=`LEMONADE_API_KEY`
- **变体**：litellm 侧默认端点为 `localhost:8000/api/v1`（registry 端口 13305 为 Lemonade SDK 本地服务，两者同为本地服务形态）

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（`max_tokens`/`max_completion_tokens` 均支持） | - | C | https://docs.litellm.ai/docs/providers/lemonade |
| 能力支持 | 无差异（支持 top_k、tools、response_format(json_schema)、logit_bias、presence_penalty、repeat_penalty 等） | `temperature=0.7, top_k=50, repeat_penalty=1.1` | C | https://docs.litellm.ai/docs/providers/lemonade |
| 思考机制 | 无法确认（本地推理，依赖模型） | - | ⚠️ | - |
| 流式/usage | 无差异 | - | C | https://docs.litellm.ai/docs/providers/lemonade |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异（不强制校验 key：`doesn't require strict API key validation`） | - | C | https://docs.litellm.ai/docs/providers/lemonade |
| headers/认证 | 无差异（本地服务，key 非强校验） | - | C | https://docs.litellm.ai/docs/providers/lemonade |
| URL/端点 | 🔶 registry `localhost:13305/v1`；官方 litellm 默认 `localhost:8000/api/v1`（端口可配 `LEMONADE_API_BASE`）——registry 端口来源未确认 | `os.environ['LEMONADE_API_BASE'] = "http://localhost:8000/api/v1"` | C | https://docs.litellm.ai/docs/providers/lemonade |
| 模型 ID | 由 `/models` 端点动态列出（GGUF 模型 ID） | - | C | https://docs.litellm.ai/docs/providers/lemonade |

#### 2. aimux 现状对比

- **对比结论**：✅ 协议已覆盖（top_k 支持匹配 full() profile）+ ⚠️ 本地端口存疑
- **aimux 代码位置**：`openai_compat_registry.rs:1087-1095`
- **差距说明**：协议完全 OpenAI 兼容且支持 top_k；registry 端口 13305 与 litellm 默认 8000 不同（Lemonade SDK 自身默认端口待确认）。
- **建议动作**：确认 Lemonade SDK 本地端口；协议层无需动作。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：⚠️ 本地端口 13305 来源未确认

### lemonfox_ai — Lemonfox AI

- **registry 现状**：profile=`full()` · base_url=`https://api.lemonfox.ai/v1` · env=`LEMONFOX_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | C | https://igris.mintlify.app/connect/providers |
| 能力支持 | 无差异（chat.completions；主打低价 Whisper 转录 + 部分开源模型） | `curl https://api.lemonfox.ai/v1/chat/completions` | C | https://blog.gordonbuchan.com/blog/index.php/2025/01/28/open-source-llm-models-and-open-source-inference-software-building-blocks-of-a-commoditized-llm-inference-hosting-market/ |
| 思考机制 | 无法确认 | - | ⚠️ | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（bearer） | - | C | https://igris.mintlify.app/connect/providers |
| URL/端点 | 无差异 | - | C | https://igris.mintlify.app/connect/providers |
| 模型 ID | 无差异（开源模型 ID） | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:1096-1104`
- **差距说明**：OpenAI 兼容（Whisper 转录也走 /v1/audio/transcriptions，标准路径）。
- **建议动作**：无需动作。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：-

### libertai — Libertai

- **registry 现状**：profile=`full()` · base_url=`https://api.libertai.io/v1` · env=`LIBERTAI_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | C | https://docs.libertai.io/quickstart.html |
| 能力支持 | 无差异（OpenAI 兼容；支持流式与工具调用） | - | C | https://libertai.io/ |
| 思考机制 | 未发现文档化差异 | - | - | - |
| 流式/usage | 无差异 | - | C | https://libertai.io/ |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（`Authorization: Bearer $LIBERTAI_API_KEY`） | `curl -X POST https://api.libertai.io/v1/chat/completions -H "Authorization: Bearer $LIBERTAI_API_KEY"` | C | https://docs.libertai.io/quickstart.html |
| URL/端点 | 无差异 | - | C | https://blog.libertai.io/unleash-decentralized-ai-your-ultimate-guide-to-the-libertai-inference-api |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:1105-1113`
- **差距说明**：去中心化推理，协议 OpenAI 兼容。
- **建议动作**：无需动作。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：-

### lilac — Lilac

- **registry 现状**：profile=`full()` · base_url=`https://api.getlilac.com/v1` · env=`LILAC_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（/v1/models 公开可探测，OpenAI schema） | - | C | https://llm24.net/model-api-check |
| 能力支持 | 无差异（4 个模型走 OpenAI schema） | - | C | https://llm24.net/model-api-check |
| 思考机制 | 无法确认 | - | ⚠️ | - |
| 流式/usage | 无法确认 | - | ⚠️ | - |
| 消息格式 | 无法确认 | - | ⚠️ | - |
| 特殊字段 | 无法确认 | - | ⚠️ | - |
| headers/认证 | 无差异（GPU 云服务，API key 认证） | - | - | - |
| URL/端点 | 无差异（`https://api.getlilac.com/v1`，`/v1/models` 可达） | - | C | https://llm24.net/model-api-check |
| 模型 ID | 无差异（标准模型 ID） | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖（证据较薄）
- **aimux 代码位置**：`openai_compat_registry.rs:1114-1122`
- **差距说明**：getlilac.com 为 GPU 云（边缘推理），/v1/models 走 OpenAI schema。
- **建议动作**：无需动作。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：⚠️ 详细参数（stream/usage/思考）未确认

### lingyiwanwu — Lingyiwanwu (零一万物)

- **registry 现状**：profile=`full()` · base_url=`https://api.lingyiwanwu.com/v1` · env=`LINGYIWANWU_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | C | https://docs.emqx.com/zh/neuronex/latest/best-practise/llm-example.html |
| 能力支持 | 无差异（OpenAI SDK 直接调用，标准参数） | `client = OpenAI(api_key=..., base_url="https://api.lingyiwanwu.com/v1")` | C | https://developer.aliyun.com/article/1586831 |
| 思考机制 | 未发现文档化差异（Yi-Lightning 等无思考开关文档） | - | ⚠️ | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer LINGYIWANWU_API_KEY） | - | C | https://developer.aliyun.com/article/1586831 |
| URL/端点 | 无差异 | - | C | https://platform.lingyiwanwu.com/docs |
| 模型 ID | `yi-large`、`yi-lightning`、`yi-medium`、`yi-spark`、`yi-vision` 等 | `"model": "yi-lightning"` | C | https://docs.emqx.com/zh/neuronex/latest/best-practise/llm-example.html |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:1123-1131`
- **差距说明**：纯 OpenAI 兼容。
- **建议动作**：无需动作。

#### 3. 证据与验证

- **证据等级**：C（官方 docs 页为 SPA 未能抓取原文，引用了官方 platform 域名 + 多个可靠第三方示例）
- **验证状态**：🔲 未验证
- **存疑标记**：⚠️ 思考机制未确认（官方文档未公开 Yi 思考开关）

### llama — Llama

- **registry 现状**：profile=`full()` · base_url=`https://api.llama.com/compat/v1/` · env=`LLAMA_API_KEY`
- **变体**：Meta Model API（`api.meta.ai`，Muse 模型）已被官方称为 Llama API 的继任者

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | C | https://developer.puter.com/tutorials/how-to-get-llama-api-key/ |
| 能力支持 | 无差异（OpenAI 兼容模式） | - | C | https://muddyterrain.com/docs/genai-unreal/openai-compatible-mode/ |
| 思考机制 | 无法确认（模型侧） | - | ⚠️ | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（`Authorization: Bearer <LLAMA_API_KEY>`；Meta 开发者平台签发） | - | C | https://developer.puter.com/tutorials/how-to-get-llama-api-key/ |
| URL/端点 | 无差异（`https://api.llama.com/compat/v1/`；注意 registry 无尾斜杠，官方示例带尾斜杠，一般无影响） | `new OpenAI({ baseURL: "https://api.llama.com/compat/v1/", apiKey: process.env.LLAMA_API_KEY })` | C | https://developer.puter.com/tutorials/how-to-get-llama-api-key/ |
| 模型 ID | `meta-llama/Llama-*` 形式（`meta-llama/Llama-4-Scout` 等） | - | C | https://muddyterrain.com/docs/genai-unreal/openai-compatible-mode/ |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:1132-1140`
- **差距说明**：OpenAI 兼容；模型 ID org/model 形式透传。
- **建议动作**：无需动作（可关注 Meta Model API `api.meta.ai` 迁移）。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：-

### llamagate — Llamagate

- **registry 现状**：profile=`full()` · base_url=`https://api.llamagate.dev/v1` · env=`LLAMAGATE_API_KEY`
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | C | https://docs.litellm.ai/docs/providers/llamagate |
| 能力支持 | 无差异（26+ 开源 LLM 的 OpenAI 兼容网关） | - | C | https://ai-sdk.dev/providers/community-providers/llamagate |
| 思考机制 | 未发现文档化差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer LLAMAGATE_API_KEY） | - | C | https://docs.litellm.ai/docs/providers/llamagate |
| URL/端点 | 无差异 | - | C | https://docs.litellm.ai/docs/providers/llamagate |
| 模型 ID | 无差异（开源模型 ID） | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:1141-1149`
- **差距说明**：OpenAI 兼容网关。
- **建议动作**：无需动作。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：-

---

## 汇总

- **完成条目**：42/42（每家均有 9 类差异表 + aimux 现状对比 + 证据状态）
- **协议级差异（request 构造相关）11 家**：groq（max_tokens 弃用方向）、gigachat（OAuth 认证 + TLS + 端点）、heroku（仅 max_completion_tokens + allow_ignored_params + extended_thinking）、hetzner（chat_template_kwargs.enable_thinking）、inception（diffusing 专有字段 + FIM 端点）、inference_net（x-inference-* 代理头）、kimi（thinking.type/thinking.keep by-model）、kimi_for_coding（reasoning_effort 映射/400 + 固定模型 ID）、helicone（双 key 认证头）、github（服务已退役）、kluster_ai（服务疑似下线）
- **registry 数据问题 6 家**：freemodel/gmi/iflowcn（base_url 被截断成垃圾字符串）、krutrim（官方端点为 cloud.olakrutrim.com/v1）、kilo（官方端点为 /api/gateway）、lemonade（本地端口待确认）
- **存疑条目 7 家**：gdc、inferx、infinity、kiro、hosted_vllm、lilac（资料不足）、lingyiwanwu（仅思考机制未确认）
- **证据等级分布**：全部为 C（官方/权威网页）；本批无 A 级（仓库 cassette）与 B 级（reference 代码，因 reference/ 多为文档剪枝版，未含厂商适配代码）
