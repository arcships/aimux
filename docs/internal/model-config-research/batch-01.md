# Batch 01 — Model Request Config 调研

> 状态: ✅ 已调研（2026-08-01） · 厂商数: 42
> 模板见 [_template.md](_template.md) · 方法论见 [README.md](README.md)
> 证据硬性要求: 每条目必须有例子或证明(A/B/C 级),禁止编造来源。
> ⚠️ 存疑条目汇总见文末「存疑归档」节。

## 厂商清单

| # | id | display | base_url | env_var | profile |
|---|---|---|---|---|---|
| 1 | abacus | Abacus | https://routellm.abacus.ai/v1 | ABACUS_API_KEY | OpenAICompatProfile::full() |
| 2 | abliteration_ai | Abliteration AI | https://api.abliteration.ai/v1 | ABLIT_KEY | OpenAICompatProfile::full() |
| 3 | ai_router | AI-ROUTER | https://api.ai-router.dev/v1 | AI_ROUTER_API_KEY | OpenAICompatProfile::full() |
| 4 | ai21 | AI21 Labs | https://api.ai21.ai/v1 | AI21_API_KEY | OpenAICompatProfile::full() |
| 5 | ai302 | 302.AI | https://api.302.ai/v1 | AI302_API_KEY | OpenAICompatProfile::full() |
| 6 | aiand | AIand | https://api.aiand.com/v1 | AIAND_API_KEY | OpenAICompatProfile::full() |
| 7 | aibadgr | AI Badgr | https://api.aibadgr.com/v1 | AIBADGR_API_KEY | OpenAICompatProfile::full() |
| 8 | aigc2d | AIGC2D | https://api.aigc2d.com/v1 | AIGC2D_API_KEY | OpenAICompatProfile::full() |
| 9 | aihubmix | AIHubMix | https://aihubmix.com/v1 | AIHUBMIX_API_KEY | OpenAICompatProfile::full() |
| 10 | ails | AILS | https://api.caipacity.com/v1 | AILS_API_KEY | OpenAICompatProfile::full() |
| 11 | aiml | AI/ML API | https://api.aimlapi.com/v1 | AIML_API_KEY | OpenAICompatProfile::full() |
| 12 | aki_io | AKI.IO | https://aki.io/openai/v1 | AKI_IO_API_KEY | OpenAICompatProfile::full() |
| 13 | albert | Albert | https://api.albert.ai/v1 | ALBERT_API_KEY | OpenAICompatProfile::full() |
| 14 | alibaba | Alibaba Cloud (DashScope) | https://dashscope-intl.aliyuncs.com/compatible-mode/v1 | ALIBABA_API_KEY | OpenAICompatProfile::full() |
| 15 | alibaba_coding_plan | Alibaba Coding Plan | https://coding-intl.dashscope.aliyuncs.com/v1 | ALIBABA_CODING_PLAN_API_KEY | OpenAICompatProfile::full() |
| 16 | alibaba_coding_plan_cn | Alibaba Coding Plan (China) | https://coding.dashscope.aliyuncs.com/v1 | ALIBABA_CODING_PLAN_API_KEY | OpenAICompatProfile::full() |
| 17 | alibaba_token_plan | Alibaba Token Plan | https://token-plan.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1 | ALIBABA_TOKEN_PLAN_API_KEY | OpenAICompatProfile::full() |
| 18 | alibaba_token_plan_cn | Alibaba Token Plan (China) | https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1 | ALIBABA_TOKEN_PLAN_API_KEY | OpenAICompatProfile::full() |
| 19 | ambient | Ambient | https://api.ambient.xyz/v1 | AMBIENT_API_KEY | OpenAICompatProfile::full() |
| 20 | anyapi | AnyAPI | https://api.anyapi.ai/v1 | ANYAPI_KEY | OpenAICompatProfile::full() |
| 21 | anyscale | Anyscale | https://api.endpoints.anyscale.com/v1 | ANYSCALE_API_KEY | OpenAICompatProfile::full() |
| 22 | apertis | Apertis | https://api.stima.tech/v1 | STIMA_API_KEY | OpenAICompatProfile::full() |
| 23 | api2d | API2D | https://oa.api2d.net/v1 | API2D_API_KEY | OpenAICompatProfile::full() |
| 24 | api2gpt | API2GPT | https://api.api2gpt.com/v1 | API2GPT_API_KEY | OpenAICompatProfile::full() |
| 25 | apiserpent | API Serpent | https://api.apiserpent.com/v1 | APISERPENT_API_KEY | OpenAICompatProfile::full() |
| 26 | atlascloud | AtlasCloud | https://api.atlascloud.com/v1 | ATLASCLOUD_API_KEY | OpenAICompatProfile::full() |
| 27 | atomic_chat | Atomic Chat | http://127.0.0.1:1337/v1 | ATOMIC_CHAT_API_KEY | OpenAICompatProfile::full() |
| 28 | auriko | Auriko | https://api.auriko.ai/v1 | AURIKO_API_KEY | OpenAICompatProfile::full() |
| 29 | azure_ai | Azure AI | https://models.inference.ai.azure.com | AZURE_AI_API_KEY | OpenAICompatProfile::full() |
| 30 | baichuan | Baichuan AI | https://api.baichuan-ai.com/v1 | BAICHUAN_API_KEY | OpenAICompatProfile::full() |
| 31 | baidu | Baidu (文心/ERNIE) | https://qianfan.baidubce.com/v2 | BAIDU_API_KEY | OpenAICompatProfile::full() |
| 32 | baidu_v2 | BaiduV2 | https://qianfan.baidubce.com/v2 | QIANFAN_API_KEY | OpenAICompatProfile::full() |
| 33 | bailing | Bailing | https://api.ant-ling.com/v1 | BAILING_API_TOKEN | OpenAICompatProfile::full() |
| 34 | baseten | Baseten | https://inference.baseten.co/v1 | BASETEN_API_KEY | OpenAICompatProfile::full() |
| 35 | berget | Berget.AI | https://api.berget.ai/v1 | BERGET_API_KEY | OpenAICompatProfile::full() |
| 36 | bigmodel | BigModel (智谱) | https://open.bigmodel.cn/api/paas/v4 | BIGMODEL_API_KEY | OpenAICompatProfile::full() |
| 37 | blueclaw | Blue Claw | https://openai.blueclaw.network/v1 | BLUECLAW_API_KEY | OpenAICompatProfile::full() |
| 38 | bytedance | ByteDance | https://ark.cn-beijing.volces.com/api/v3 | ARK_API_KEY | OpenAICompatProfile::full() |
| 39 | byteplus | BytePlus (Volcano) | https://ark.bytepluses.com/api/v3 | BYTEPLUS_API_KEY | OpenAICompatProfile::full() |
| 40 | bytez | Bytez | https://api.bytez.com/v2 | BYTEZ_API_KEY | OpenAICompatProfile::full() |
| 41 | canopywave | Canopywave | https://api.canopywave.com/v1 | CANOPYWAVE_API_KEY | OpenAICompatProfile::full() |
| 42 | cerebras | Cerebras | https://api.cerebras.ai/v1 | CEREBRAS_API_KEY | OpenAICompatProfile::full() |

---

### abacus — Abacus (RouteLLM)

- **registry 现状**：profile=`full()` · base_url=`https://routellm.abacus.ai/v1` · env=`ABACUS_API_KEY`（[openai_compat_registry.rs](D:\code\aimux\aimux-providers\src\openai_compat_registry.rs#L16)）
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异（工具/流式/多模态/PDF 均走 OpenAI 兼容通道） | - | C | abacus.ai RouteLLM 文档 |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（`Authorization: Bearer <api_key>`） | - | C | https://abacus.ai/help/developer-platform/route-llm/ |
| URL/端点 | 无差异（self-serve 用 `/v1`；企业版 base_url 为 `https://<workspace>.abacus.ai/v1`，属部署差异） | - | C | https://abacus.ai/help/developer-platform/route-llm/ |
| 模型 ID | 特殊模型 ID `route-llm`：填入该 ID 时由系统自动按成本/速度/质量路由，其余模型 ID 直接透传 | `"model": "route-llm"` | C | https://abacus.ai/help/developer-platform/route-llm/ |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖（无差异；`route-llm` 模型 ID 只是字符串透传）
- **aimux 代码位置**：`openai_compat_registry.rs:16-24`
- **差距说明**：无
- **建议动作**：无需动作

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证（仅文档引用）
- **存疑标记**：-

---

### abliteration_ai — Abliteration AI

- **registry 现状**：profile=`full()` · base_url=`https://api.abliteration.ai/v1` · env=`ABLIT_KEY`（[openai_compat_registry.rs](D:\code\aimux\aimux-providers\src\openai_compat_registry.rs#L25)）
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
| headers/认证 | 无差异 | - | - | - |
| URL/端点 | 无差异（官方自述为 OpenAI-compatible API） | - | C | https://abliteration.ai/abliterated-ai |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖（无差异）
- **aimux 代码位置**：`openai_compat_registry.rs:25-33`
- **差距说明**：官方文档仅确认「behind an OpenAI-compatible API」，未披露私有参数
- **建议动作**：无需动作

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证（仅文档引用）
- **存疑标记**：-

---

### ai_router — AI-ROUTER

- **registry 现状**：profile=`full()` · base_url=`https://api.ai-router.dev/v1` · env=`AI_ROUTER_API_KEY`（[openai_compat_registry.rs](D:\code\aimux\aimux-providers\src\openai_compat_registry.rs#L52)）
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
| headers/认证 | 无差异 | - | - | - |
| URL/端点 | 无差异 | - | - | - |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖（按 OpenAI 兼容假设，无差异）
- **aimux 代码位置**：`openai_compat_registry.rs:52-60`
- **差距说明**：**⚠️ 证据不足**：未能找到 ai-router.dev 任何官方文档/站点，无法排除私有参数
- **建议动作**：保持现状，列入存疑归档等待补充证据

#### 3. 证据与验证

- **证据等级**：-（无公开证据）
- **验证状态**：🔲 未验证
- **存疑标记**：⚠️ 存疑（无出处，仅按 OpenAI 兼容假设）

---

### ai21 — AI21 Labs

- **registry 现状**：profile=`full()` · base_url=`https://api.ai21.ai/v1` · env=`AI21_API_KEY`（[openai_compat_registry.rs](D:\code\aimux\aimux-providers\src\openai_compat_registry.rs#L34)）
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 输出上限字段为 `max_tokens`（Jamba 上限 4096），无 `max_completion_tokens` | `client.chat.completions.create(messages=..., model="jamba-large", max_tokens=1024)` | C | https://docs.ai21.com/reference/jamba-1-6-api-ref |
| 能力支持 | `response_format` 仅 JSON mode `{"type":"json_object"}`；tools/function calling 支持；`n` 支持 1-16；temperature 默认 0.4 | `"response_format": {"type": "json_object"}` | C | https://docs.ai21.com/reference/jamba-1-6-api-ref · https://docs.ai21.com/docs/function-calling |
| 思考机制 | Jamba 1.6/1.7 有 thinking 模式（OpenRouter 标注 reasoning tokens），但请求参数名未在官方文档中定位到 | - | ⚠️ | https://openrouter.ai/ai21/jamba-large-1.7 |
| 流式/usage | 无差异（SSE；`stream=True` 时 `n` 必须为 1；`tools` 与 `stream` 不能同用） | - | C | https://docs.ai21.com/reference/jamba-1-6-api-ref |
| 消息格式 | 无差异（system/user/assistant/tool 消息，`tool_call_id` 标准） | - | C | https://docs.ai21.com/reference/jamba-1-6-api-ref |
| 特殊字段 | 私有字段 `documents`：注入外部文档供模型引用 | `"documents": [{"content": "hello world", "metadata": [{"key": "author", "value": "ishaan"}]}]` | C | https://docs.litellm.ai/docs/providers/ai21 |
| headers/认证 | 无差异（Bearer） | - | C | https://docs.litellm.ai/docs/providers/ai21 |
| URL/端点 | ⚠️ 注册表 base_url=`api.ai21.ai/v1`；第三方集成文档普遍使用 `https://api.ai21.com/studio/v1`（API2D 风格 studio 前缀），jamba-1-6 参考文档请求路径为 `POST /studio/v1/chat/completions`，域名/路径与注册表不完全一致 | `POST https://api.ai21.com/studio/v1/chat/completions` | C | https://docs.ai21.com/reference/jamba-1-6-api-ref · https://igris.mintlify.app/connect/providers |
| 模型 ID | 无差异（`jamba-large` / `jamba-mini` 短名 + 版本快照名 `jamba-large-1.6-2025-03`） | `"model": "jamba-large"` | C | https://docs.ai21.com/docs/jamba-foundation-models |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:34-42`；`convert.rs:1118-1138`（max_tokens 白名单已含）、`convert.rs:1285-1317`（特殊字段白名单）
- **差距说明**：`max_tokens` ✅ 已覆盖；`response_format.json_object` ✅；**`documents` 字段 ❌ 未在白名单**，需要 `bodyOverrides` 或新增白名单；`n` 参数未在 aimux CallOptions 白名单中（convert.rs:1285-1317 无 `n`）
- **建议动作**：bodyOverrides 兜底 `{"documents": [...]}`；调研是否把 `documents` 加入白名单；核对 base_url（`api.ai21.ai/v1` vs `api.ai21.com/studio/v1`）

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证（仅文档引用）
- **存疑标记**：⚠️ 思考机制参数名未证实；base_url 域名存疑

---

### ai302 — 302.AI

- **registry 现状**：profile=`full()` · base_url=`https://api.302.ai/v1` · env=`AI302_API_KEY`（[openai_compat_registry.rs](D:\code\aimux\aimux-providers\src\openai_compat_registry.rs#L43)）
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异（全面兼容 OpenAI 协议，150+ 模型） | - | C | https://segmentfault.com/a/1190000047843591（第三方评测） |
| 思考机制 | 无差异（透传） | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer） | - | - | - |
| URL/端点 | 无差异（`https://302.ai` 聚合网关，OpenAI 兼容 `/v1`） | - | C | https://help.302.ai/docs/geng-xin-ri-zhi-VmVs |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖（无差异）
- **aimux 代码位置**：`openai_compat_registry.rs:43-51`
- **差距说明**：聚合网关按 OpenAI 协议透传，无私有 request 字段证据
- **建议动作**：无需动作

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：-

---

### aiand — AIand

- **registry 现状**：profile=`full()` · base_url=`https://api.aiand.com/v1` · env=`AIAND_API_KEY`（[openai_compat_registry.rs](D:\code\aimux\aimux-providers\src\openai_compat_registry.rs#L61)）
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
| headers/认证 | 无差异 | - | - | - |
| URL/端点 | 无差异 | - | - | - |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖（按 OpenAI 兼容假设）
- **aimux 代码位置**：`openai_compat_registry.rs:61-69`
- **差距说明**：**⚠️ 证据不足**：未找到 aiand.com 官方 API 文档
- **建议动作**：保持现状，列入存疑归档

#### 3. 证据与验证

- **证据等级**：-
- **验证状态**：🔲 未验证
- **存疑标记**：⚠️ 存疑（无出处）

---

### aibadgr — AI Badgr

- **registry 现状**：profile=`full()` · base_url=`https://api.aibadgr.com/v1` · env=`AIBADGR_API_KEY`（[openai_compat_registry.rs](D:\code\aimux\aimux-providers\src\openai_compat_registry.rs#L70)）
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
| headers/认证 | 无差异 | - | - | - |
| URL/端点 | 无差异 | - | - | - |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖（按 OpenAI 兼容假设）
- **aimux 代码位置**：`openai_compat_registry.rs:70-78`
- **差距说明**：**⚠️ 证据不足**：未找到 aibadgr.com 官方 API 文档
- **建议动作**：保持现状，列入存疑归档

#### 3. 证据与验证

- **证据等级**：-
- **验证状态**：🔲 未验证
- **存疑标记**：⚠️ 存疑（无出处）

---

### aigc2d — AIGC2D

- **registry 现状**：profile=`full()` · base_url=`https://api.aigc2d.com/v1` · env=`AIGC2D_API_KEY`（[openai_compat_registry.rs](D:\code\aimux\aimux-providers\src\openai_compat_registry.rs#L79)）
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
| headers/认证 | 无差异 | - | - | - |
| URL/端点 | 无差异 | - | - | - |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖（按 OpenAI 兼容假设）
- **aimux 代码位置**：`openai_compat_registry.rs:79-87`
- **差距说明**：**⚠️ 证据不足**：未找到 aigc2d.com 官方 API 文档
- **建议动作**：保持现状，列入存疑归档

#### 3. 证据与验证

- **证据等级**：-
- **验证状态**：🔲 未验证
- **存疑标记**：⚠️ 存疑（无出处）

---

### aihubmix — AIHubMix

- **registry 现状**：profile=`full()` · base_url=`https://aihubmix.com/v1` · env=`AIHUBMIX_API_KEY`（[openai_compat_registry.rs](D:\code\aimux\aimux-providers\src\openai_compat_registry.rs#L88)）
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异（OpenAI 兼容，500+ 模型） | - | C | https://docs.aihubmix.com/en |
| 思考机制 | 无差异（透传） | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer） | - | C | https://docs.aihubmix.com/cn/api/Crush |
| URL/端点 | OpenAI 协议 base 为 `https://aihubmix.com/v1`；Anthropic 协议为 `https://aihubmix.com`（协议分流，与注册表 OpenAI 用法无关） | - | C | https://docs.aihubmix.com/cn/api/Crush |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖（无差异）
- **aimux 代码位置**：`openai_compat_registry.rs:88-96`
- **差距说明**：无
- **建议动作**：无需动作

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：-

---

### ails — AILS

- **registry 现状**：profile=`full()` · base_url=`https://api.caipacity.com/v1` · env=`AILS_API_KEY`（[openai_compat_registry.rs](D:\code\aimux\aimux-providers\src\openai_compat_registry.rs#L97)）
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
| headers/认证 | 无差异 | - | - | - |
| URL/端点 | 无差异 | - | - | - |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖（按 OpenAI 兼容假设）
- **aimux 代码位置**：`openai_compat_registry.rs:97-105`
- **差距说明**：**⚠️ 证据不足**：caipacity.com 无公开 API 文档可查
- **建议动作**：保持现状，列入存疑归档

#### 3. 证据与验证

- **证据等级**：-
- **验证状态**：🔲 未验证
- **存疑标记**：⚠️ 存疑（无出处）

---

### aiml — AI/ML API

- **registry 现状**：profile=`full()` · base_url=`https://api.aimlapi.com/v1` · env=`AIML_API_KEY`（[openai_compat_registry.rs](D:\code\aimux\aimux-providers\src\openai_compat_registry.rs#L106)）
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（`max_tokens` 等标准参数） | - | C | https://docs.agno.com/models/providers/gateways/aimlapi/overview |
| 能力支持 | 无差异（「extends the OpenAI-compatible interface and supports most parameters」） | - | C | https://docs.agno.com/models/providers/gateways/aimlapi/overview |
| 思考机制 | 无差异（透传） | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer） | - | C | https://www.promptfoo.dev/docs/providers/aimlapi/ |
| URL/端点 | 无差异（base_url=`https://api.aimlapi.com/v1`） | - | C | https://docs.agno.com/models/providers/gateways/aimlapi/overview |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖（无差异）
- **aimux 代码位置**：`openai_compat_registry.rs:106-114`
- **差距说明**：无
- **建议动作**：无需动作

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：-

---

### aki_io — AKI.IO

- **registry 现状**：profile=`full()` · base_url=`https://aki.io/openai/v1` · env=`AKI_IO_API_KEY`（[openai_compat_registry.rs](D:\code\aimux\aimux-providers\src\openai_compat_registry.rs#L115)）
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异（OpenAI 风格 drop-in replacement） | - | C | https://aki.io/ |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer） | - | C | https://aki.io/ |
| URL/端点 | 路径前缀 `/openai/v1`（OpenAI 协议）与 `/anthropic`（Anthropic 协议）分流，与 OpenAI 默认 `/v1` 不同但注册表已正确设置 | `https://aki.io/openai/v1/chat/completions` | C | https://aki.io/ |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖（无差异；base_url 已含 `/openai/v1` 前缀）
- **aimux 代码位置**：`openai_compat_registry.rs:115-123`
- **差距说明**：无
- **建议动作**：无需动作

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：-

---

### albert — Albert

- **registry 现状**：profile=`full()` · base_url=`https://api.albert.ai/v1` · env=`ALBERT_API_KEY`（[openai_compat_registry.rs](D:\code\aimux\aimux-providers\src\openai_compat_registry.rs#L124)）
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
| headers/认证 | 无差异 | - | - | - |
| URL/端点 | 无差异 | - | - | - |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖（按 OpenAI 兼容假设）
- **aimux 代码位置**：`openai_compat_registry.rs:124-132`
- **差距说明**：**⚠️ 证据不足**：未能定位 albert.ai 官方 API 文档（albert.ai 为瑞士 EPFL 系创业公司，站点当前无可抓取 API 文档）
- **建议动作**：保持现状，列入存疑归档

#### 3. 证据与验证

- **证据等级**：-
- **验证状态**：🔲 未验证
- **存疑标记**：⚠️ 存疑（无出处）

---

### alibaba — Alibaba Cloud (DashScope)

- **registry 现状**：profile=`full()` · base_url=`https://dashscope-intl.aliyuncs.com/compatible-mode/v1` · env=`ALIBABA_API_KEY`（[openai_compat_registry.rs](D:\code\aimux\aimux-providers\src\openai_compat_registry.rs#L133)）
- **变体**：alibaba_coding_plan（`https://coding-intl.dashscope.aliyuncs.com/v1`）、alibaba_coding_plan_cn（`https://coding.dashscope.aliyuncs.com/v1`）、alibaba_token_plan（`https://token-plan.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1`）、alibaba_token_plan_cn（`https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1`）——同一 compatible-mode 协议，仅套餐/区域端点不同

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | `max_tokens`（OpenAI 兼容层标准）；无 `max_completion_tokens` 相关特例 | - | C | https://www.alibabacloud.com/help/en/model-studio/qwen-api-via-openai-chat-completions |
| 能力支持 | top_p/temperature/stop/response_format/tools 标准支持；`stream_options.include_usage` 支持 | `"stream_options": {"include_usage": true}` | C | https://www.alibabacloud.com/help/en/model-studio/deep-thinking |
| 思考机制 | **by-model 两套机制**：(1) 混合思考模式用 `enable_thinking`（非标准参数，需 extra_body）开关；`thinking_budget` 控制思考预算；(2) thinking-only 模型不可关。**默认值按模型族不同**：qwen3 开源系默认开、qwen-plus 商业系默认关。**消息级开关**：prompt 追加 `/no_think`、`/think` 逐轮切换 | `extra_body={"enable_thinking": True}`；curl 体 `{"model":"qwen-plus","messages":[...],"stream":true,"stream_options":{"include_usage":true},"enable_thinking":true}` | C | https://www.alibabacloud.com/help/en/model-studio/deep-thinking · https://qwen.readthedocs.io/en/latest/getting_started/quickstart.html |
| 流式/usage | 流式最后一块为 `choices:[]` + 顶层 `usage`（OpenAI 标准）；思考输出走 `delta.reasoning_content`，正文走 `delta.content` | `data: {"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":360,"total_tokens":370},...}` | C | https://www.alibabacloud.com/help/en/model-studio/deep-thinking |
| 消息格式 | 思考内容在响应 `reasoning_content` 字段；历史回传时**必须完整回传 reasoning_content**（官方提示）；**兼容层不支持 document 内容块**（pydantic-ai 明确报 UserError）；`qwen3-*` 非流式请求被强制 `enable_thinking=false`，`qwq-*` 强制 `stream=true` | `"messages":[{"role":"assistant","content":null,"reasoning_content":"..."}]` 回传 | B+C | [reference/pydantic-ai/docs/models/openai.md](D:\code\aimux\reference\pydantic-ai\docs\models\openai.md#L615) · [reference/aiproxy/docs/REASONING_COMPATIBILITY.md](D:\code\aimux\reference\aiproxy\docs\REASONING_COMPATIBILITY.md#L433) |
| 特殊字段 | `thinking_budget`（Qwen 系，范围参考 [100,16384]，不被 max_tokens 钳制） | `{"enable_thinking":true,"thinking_budget":16384}` | B+C | [reference/aiproxy/docs/REASONING_COMPATIBILITY.md](D:\code\aimux\reference\aiproxy\docs\REASONING_COMPATIBILITY.md#L399) · https://www.alibabacloud.com/help/en/model-studio/deep-thinking |
| headers/认证 | 无差异（`Authorization: Bearer`） | - | C | https://www.alibabacloud.com/help/en/model-studio/deep-thinking |
| URL/端点 | 注册表用 `dashscope-intl.aliyuncs.com/compatible-mode/v1`（仍可用）；官方推荐迁移到 workspace 专属域名（新加坡 `{WorkspaceId}.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1`、北京、美东 `dashscope-us.aliyuncs.com/compatible-mode/v1` 等） | `POST https://{WorkspaceId}.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1/chat/completions` | C | https://www.alibabacloud.com/help/en/model-studio/qwen-api-via-openai-chat-completions |
| 模型 ID | 无差异（`qwen-plus`、`qwen3-max`、`qwq-plus` 等模型名直接透传；编码套餐走独立域名） | - | C | https://www.alibabacloud.com/help/en/model-studio/deep-thinking |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖（思考机制 ❌）
- **aimux 代码位置**：`openai_compat_registry.rs:133-177`（5 个条目均 `full()`）；`convert.rs:1098-1441`（白名单无 enable_thinking/thinking_budget）；`convert.rs:1439-1441`（body_overrides deep-merge 兜底）
- **差距说明**：`enable_thinking`/`thinking_budget` 不在白名单；`reasoning_effort` 直通（convert.rs:1327）但**不映射**为 qwen 的 `enable_thinking`（qwen 系不认 `reasoning_effort`）；`/no_think` 消息级开关无处理。已有 cassette 仅覆盖 thin wrapper 无思考（[cassettes/alibaba](D:\code\aimux\aimux-providers\tests\cassettes\alibaba)）
- **建议动作**：profile 新增字段（如 `thinking` 开关字段名/取值映射）或在文档中明确用 `bodyOverrides: {"enable_thinking": true, "thinking_budget": 16384}` 兜底；补 qwen-plus 流式思考 cassette

#### 3. 证据与验证

- **证据等级**：B + C（本地 aiproxy 适配代码 + 官方文档）
- **验证状态**：🔲 未验证（有 thin wrapper cassette，无思考路径）
- **存疑标记**：-

---

### ambient — Ambient

- **registry 现状**：profile=`full()` · base_url=`https://api.ambient.xyz/v1` · env=`AMBIENT_API_KEY`（[openai_compat_registry.rs](D:\code\aimux\aimux-providers\src\openai_compat_registry.rs#L178)）
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
| headers/认证 | 无差异（Bearer） | - | C | https://ambient.xyz/ |
| URL/端点 | 无差异（官方宣称「Point your OpenAI or Anthropic SDK at Ambient」零改写） | - | C | https://ambient.xyz/ |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖（无差异）
- **aimux 代码位置**：`openai_compat_registry.rs:178-186`
- **差距说明**：无
- **建议动作**：无需动作

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：-

---

### anyapi — AnyAPI

- **registry 现状**：profile=`full()` · base_url=`https://api.anyapi.ai/v1` · env=`ANYAPI_KEY`（[openai_compat_registry.rs](D:\code\aimux\aimux-providers\src\openai_compat_registry.rs#L187)）
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异（标准 /models 端点，OpenAI-compatible 网关） | - | C | https://github.com/Kilo-Org/kilocode/issues/10588 |
| 思考机制 | 无差异（透传） | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer） | - | C | https://docs.anyapi.ai |
| URL/端点 | 无差异（base_url=`https://api.anyapi.ai/v1`） | - | C | https://github.com/Kilo-Org/kilocode/issues/10588 |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖（无差异）
- **aimux 代码位置**：`openai_compat_registry.rs:187-195`
- **差距说明**：无
- **建议动作**：无需动作

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：-

---

### anyscale — Anyscale

- **registry 现状**：profile=`full()` · base_url=`https://api.endpoints.anyscale.com/v1` · env=`ANYSCALE_API_KEY`（[openai_compat_registry.rs](D:\code\aimux\aimux-providers\src\openai_compat_registry.rs#L196)）
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（OpenAI 标准） | - | C | https://docs.litellm.ai/docs/providers/anyscale |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer，`ANYSCALE_API_KEY`） | - | C | https://docs.litellm.ai/docs/providers/anyscale |
| URL/端点 | 无差异 | - | - | - |
| 模型 ID | 模型 ID 带组织前缀 `<org>/<model>`（HuggingFace 风格），如 `meta-llama/Llama-2-7b-chat-hf`、`mistralai/Mistral-7B-Instruct-v0.1` | `"model": "meta-llama/Llama-2-7b-chat-hf"` | C | https://docs.litellm.ai/docs/providers/anyscale |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖（无差异；模型 ID 为字符串透传）
- **aimux 代码位置**：`openai_compat_registry.rs:196-204`
- **差距说明**：无
- **建议动作**：无需动作

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：-

---

### apertis — Apertis

- **registry 现状**：profile=`full()` · base_url=`https://api.stima.tech/v1` · env=`STIMA_API_KEY`（[openai_compat_registry.rs](D:\code\aimux\aimux-providers\src\openai_compat_registry.rs#L205)）
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
| headers/认证 | 无差异 | - | - | - |
| URL/端点 | 无差异 | - | - | - |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖（按 OpenAI 兼容假设）
- **aimux 代码位置**：`openai_compat_registry.rs:205-213`
- **差距说明**：**⚠️ 证据不足**：stima.tech/apertis 无公开 API 文档可查
- **建议动作**：保持现状，列入存疑归档

#### 3. 证据与验证

- **证据等级**：-
- **验证状态**：🔲 未验证
- **存疑标记**：⚠️ 存疑（无出处）

---

### api2d — API2D

- **registry 现状**：profile=`full()` · base_url=`https://oa.api2d.net/v1` · env=`API2D_API_KEY`（[openai_compat_registry.rs](D:\code\aimux\aimux-providers\src\openai_compat_registry.rs#L214)）
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
| headers/认证 | 无差异（Bearer + fk- 前缀 key） | - | C | https://api2d.com/doc/doc |
| URL/端点 | 无差异（官方指定 `https://oa.api2d.net` 或 `https://openai.api2d.net`，**结尾无 `/`**；注册表带 `/v1`，OpenAI SDK 拼 `/chat/completions` 后为 `oa.api2d.net/v1/chat/completions`，与官方兼容用法一致） | - | C | https://api2d.com/doc/doc |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖（无差异）
- **aimux 代码位置**：`openai_compat_registry.rs:214-222`
- **差距说明**：无
- **建议动作**：无需动作

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：-

---

### api2gpt — API2GPT

- **registry 现状**：profile=`full()` · base_url=`https://api.api2gpt.com/v1` · env=`API2GPT_API_KEY`（[openai_compat_registry.rs](D:\code\aimux\aimux-providers\src\openai_compat_registry.rs#L223)）
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
| headers/认证 | 无差异 | - | - | - |
| URL/端点 | 无差异 | - | - | - |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖（按 OpenAI 兼容假设）
- **aimux 代码位置**：`openai_compat_registry.rs:223-231`
- **差距说明**：**⚠️ 证据不足**：未找到 api2gpt.com 官方 API 文档
- **建议动作**：保持现状，列入存疑归档

#### 3. 证据与验证

- **证据等级**：-
- **验证状态**：🔲 未验证
- **存疑标记**：⚠️ 存疑（无出处）

---

### apiserpent — API Serpent

- **registry 现状**：profile=`full()` · base_url=`https://api.apiserpent.com/v1` · env=`APISERPENT_API_KEY`（[openai_compat_registry.rs](D:\code\aimux\aimux-providers\src\openai_compat_registry.rs#L232)）
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | ⚠️ 该厂商实为 **SERP（搜索引擎结果）API**，非 LLM 供应商；官方文档仅见搜索端点 | `https://apiserpent.com` base URL，搜索端点无 /v1/chat/completions | C | https://apiserpent.com/docs |
| 能力支持 | ⚠️ 未发现 LLM chat/completions 能力文档 | - | C | https://docs.litellm.ai/docs/search/apiserpent |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异 | - | - | - |
| URL/端点 | ⚠️ 官方 base 为 `https://apiserpent.com`（无 /v1）；注册表 `api.apiserpent.com/v1` 与之不一致 | - | C | https://apiserpent.com/docs |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：⚠️ 不一致（厂商定位存疑）
- **aimux 代码位置**：`openai_compat_registry.rs:232-240`
- **差距说明**：apiserpent.com 官方文档只描述 SERP 搜索 API（Google/Bing/Yahoo/DDG），未发现 LLM 对话端点；注册表按 LLM 供应商登记，可能为失效/误登记条目
- **建议动作**：核实厂商是否提供 OpenAI 兼容 LLM 端点；若无则应从 registry 移除或标记废弃

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：⚠️ 存疑（厂商类型与 registry 定位矛盾）

---

### atlascloud — AtlasCloud

- **registry 现状**：profile=`full()` · base_url=`https://api.atlascloud.com/v1` · env=`ATLASCLOUD_API_KEY`（[openai_compat_registry.rs](D:\code\aimux\aimux-providers\src\openai_compat_registry.rs#L241)）
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异（聚合多家 LLM，兼容 OpenAI/Anthropic/DeepSeek 客户端） | - | C | https://www.atlascloud.ai/blog/guides/The-Ultimate-Guide-Deploying-ClawdBot-MoltBot-with-AtlasCloud |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer） | - | C | 同上 |
| URL/端点 | 无差异 | - | - | - |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖（无差异）
- **aimux 代码位置**：`openai_compat_registry.rs:241-249`
- **差距说明**：聚合网关，无私有 request 字段证据
- **建议动作**：无需动作

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：-

---

### atomic_chat — Atomic Chat

- **registry 现状**：profile=`full()` · base_url=`http://127.0.0.1:1337/v1` · env=`ATOMIC_CHAT_API_KEY`（[openai_compat_registry.rs](D:\code\aimux\aimux-providers\src\openai_compat_registry.rs#L250)）
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异（本地 OpenAI-compatible 服务） | - | C | https://models.dev/providers（registry 索引） |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异 | - | - | - |
| URL/端点 | 本地回环地址 `http://127.0.0.1:1337/v1`（用户自建，非公有云） | - | C | https://models.dev/providers |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖（无差异）
- **aimux 代码位置**：`openai_compat_registry.rs:250-258`
- **差距说明**：无
- **建议动作**：无需动作

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：-

---

### auriko — Auriko

- **registry 现状**：profile=`full()` · base_url=`https://api.auriko.ai/v1` · env=`AURIKO_API_KEY`（[openai_compat_registry.rs](D:\code\aimux\aimux-providers\src\openai_compat_registry.rs#L259)）
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异（OpenAI-compatible drop-in 网关，数百模型） | - | C | https://www.auriko.ai/ |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer） | - | C | https://www.auriko.ai/ |
| URL/端点 | 无差异（base_url=`https://api.auriko.ai/v1`） | - | C | https://www.auriko.ai/ |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖（无差异）
- **aimux 代码位置**：`openai_compat_registry.rs:259-267`
- **差距说明**：无
- **建议动作**：无需动作

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：-

---

### azure_ai — Azure AI

- **registry 现状**：profile=`full()` · base_url=`https://models.inference.ai.azure.com`（**无 /v1**） · env=`AZURE_AI_API_KEY`（[openai_compat_registry.rs](D:\code\aimux\aimux-providers\src\openai_compat_registry.rs#L268)）
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（max_tokens/temperature/top_p/response_format 标准；`response_format.type` 取值含 `"text"`） | `{"response_format": {"type": "text"}}` | C | https://learn.microsoft.com/en-us/rest/api/microsoft-foundry/modelinference/ |
| 能力支持 | 标准 chat completions/embeddings/image embeddings；**未知参数默认报错**，需 `extra-parameters: pass-through` 头才会透传给底层模型 | `extra-parameters: pass-through` 头 + `"safe_prompt": true`（Mistral-Large） | C | https://learn.microsoft.com/en-us/rest/api/microsoft-foundry/modelinference/ |
| 思考机制 | 无差异（透传由底层模型决定；Azure 官方不定义 thinking 字段） | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异（extra-parameters 头属透传机制） | - | C | https://learn.microsoft.com/en-us/rest/api/microsoft-foundry/modelinference/ |
| headers/认证 | **认证头为 `api-key`（AzureKeyCredential）**，也可 `Authorization: Bearer`；GitHub token（`github_pat_` 前缀）可直接作 key。aimux 默认只发 `Authorization: Bearer`，需通过 headers 配置 `api-key` | `client = ChatCompletionsClient(endpoint="https://models.inference.ai.azure.com", credential=AzureKeyCredential(github_token))` | C | https://github.com/Azure/azure-sdk-for-python/blob/main/sdk/ai/azure-ai-inference/README.md |
| URL/端点 | **端点无 `/v1` 前缀**：`https://models.inference.ai.azure.com/chat/completions`（aimux model.rs:73 直接拼接 `/chat/completions`，路径正确）；⚠️ **GitHub Models 服务已于 2026-07-30 退役**，注册表指向的免费端点可能失效，Azure AI Foundry 托管端点格式为 `https://<host>.<region>.models.ai.azure.com` | `POST https://models.inference.ai.azure.com/chat/completions` | C | https://docs.github.com/github-models/prototyping-with-ai-models（退役公告）· https://github.com/Azure/azure-sdk-for-python/blob/main/sdk/ai/azure-ai-inference/README.md |
| 模型 ID | 无差异（`gpt-4o`、`mistral-large` 等直接透传） | - | C | 同上 |

#### 2. aimux 现状对比

- **对比结论**：⚠️ 部分不一致（认证头 + 端点退役）
- **aimux 代码位置**：`openai_compat_registry.rs:268-276`；`model.rs:73`（URL 拼接）
- **差距说明**：(1) URL 拼接正确（无 /v1）；(2) `api-key` 认证头未内置——需用户 `with_headers({"api-key": ...})` 或在 profile 增加认证头支持；(3) GitHub Models 2026-07-30 退役，`models.inference.ai.azure.com` 免费端点在失效风险
- **建议动作**：profile 或配置层支持 `api-key` 头；核实端点可用性并更新 registry 说明；`extra-parameters: pass-through` 可经 headers 透传

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：⚠️ 端点退役状态需实测确认

---

### baichuan — Baichuan AI

- **registry 现状**：profile=`full()` · base_url=`https://api.baichuan-ai.com/v1` · env=`BAICHUAN_API_KEY`（[openai_compat_registry.rs](D:\code\aimux\aimux-providers\src\openai_compat_registry.rs#L277)）
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（OpenAI 兼容） | - | C | https://platform.baichuan-ai.com/docs/api |
| 能力支持 | 无差异（chat/completions + 搜索增强/知识库为控制台能力，非 request 字段） | - | C | https://platform.baichuan-ai.com/docs/api |
| 思考机制 | 无差异（官方文档未见私有 thinking 参数；`reasoning_content` 按 OpenAI 兼容层透传，未获官方专门文档） | - | ⚠️ | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer） | - | C | https://platform.baichuan-ai.com/docs/api |
| URL/端点 | 无差异（`https://api.baichuan-ai.com/v1/chat/completions`） | - | C | https://cloud.baidu.com/article/3339742 |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖（无差异）
- **aimux 代码位置**：`openai_compat_registry.rs:277-285`
- **差距说明**：无
- **建议动作**：无需动作

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：-

---

### baidu — Baidu (文心/ERNIE)

- **registry 现状**：profile=`full()` · base_url=`https://qianfan.baidubce.com/v2` · env=`BAIDU_API_KEY`（[openai_compat_registry.rs](D:\code\aimux\aimux-providers\src\openai_compat_registry.rs#L286)）
- **变体**：baidu_v2（同 `https://qianfan.baidubce.com/v2`，仅 env 不同 `QIANFAN_API_KEY`，`openai_compat_registry.rs:295-303`）——同一 v2 OpenAI 兼容端点

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | `max_tokens`（OpenAI 兼容 v2）；对 ERNIE X1 系 `max_tokens` 仅限制正文 content 长度、不含思维链 | `"max_tokens": 8192` | C | https://cloud.baidu.com/doc/qianfan-api/s/3m7of64lb |
| 能力支持 | tools/response_format 标准；视觉模型支持 image_url、视频支持 `video_url` 输入块（非标准模态）；`n` 等标准 | `"content": [{"type":"image_url","image_url":{"url":"...","detail":"high"}}]`、`{"type":"video_url","video_url":{"url":"...","fps":...}}` | C | https://ai.baidu.com/ai-doc/WENXINWORKSHOP/4mchtzl8s |
| 思考机制 | **by-model 多字段族**：`enable_thinking`（默认 false 关闭；支持模型可开）、`thinking_budget`、`reasoning_effort`（仅接受 `high`/`max`）、原生 `thinking` 对象（`{"type":"enabled|disabled"}`，DeepSeek-v4 系用此）。模型能力检测按 `qwen3-*`/`deepseek-v4-*`/`*think*`/`*vl*` 关键字分派字段族 | `{"model":"ernie-4.5-vl-28b-a3b","enable_thinking":true,"max_tokens":8192,"messages":[...]}` | B+C | [reference/aiproxy/docs/REASONING_COMPATIBILITY.md](D:\code\aimux\reference\aiproxy\docs\REASONING_COMPATIBILITY.md#L470) · https://ai.baidu.com/ai-doc/WENXINWORKSHOP/4mchtzl8s |
| 流式/usage | 无差异（SSE；usage 标准字段；V2 升级后 usage 支持 prompt_tokens_details 等） | - | C | https://cloud.baidu.com/doc/qianfan/s/Kmh4stnjp |
| 消息格式 | 响应含 `reasoning_content`（与 content 同级）；响应多一个 `flag` 字段（非标准）；历史回传需带 reasoning_content | `"message": {"role":"assistant","content":"...","reasoning_content":"..."}`，响应根级 `"flag": 0` | C | https://cloud.baidu.com/doc/qianfan-docs/s/Wm95lyynv · https://ai.baidu.com/ai-doc/WENXINWORKSHOP/4mchtzl8s |
| 特殊字段 | `thinking_budget`（Qwen3/ERNIE 思考预算，官方文档区间 [100,16384]）；Batch/OpenAI 兼容请求体含 `enable_thinking`、`thinking_budget` | `{"model":"qwen3-14b","enable_thinking":true,"thinking_budget":2048}` | B+C | [reference/aiproxy/docs/REASONING_COMPATIBILITY.md](D:\code\aimux\reference\aiproxy\docs\REASONING_COMPATIBILITY.md#L500) · https://cloud.baidu.com/doc/qianfan/s/Oml4r78ea |
| headers/认证 | 无差异（`Authorization: Bearer`，Qianfan API key） | - | C | https://ai.baidu.com/ai-doc/WENXINWORKSHOP/4mchtzl8s |
| URL/端点 | **v2 OpenAI 兼容端点**：`https://qianfan.baidubce.com/v2/chat/completions`（注册表 base_url 正确；旧 v1 rpc 接口非 OpenAI 兼容） | `POST https://qianfan.baidubce.com/v2/chat/completions` | C | https://cloud.baidu.com/doc/qianfan-api/s/3m7of64lb |
| 模型 ID | 无差异（`ernie-4.5-0.3b`、`ernie-4.5-turbo-128k-preview`、`qwen3-14b` 等透传） | - | C | https://ai.baidu.com/ai-doc/WENXINWORKSHOP/4mchtzl8s |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖（思考机制 ❌）
- **aimux 代码位置**：`openai_compat_registry.rs:286-303`（均 `full()`）；`convert.rs:1327`（reasoning_effort 直通但无 high/max 限制）
- **差距说明**：`enable_thinking`/`thinking_budget`/`thinking` 对象均不在白名单；`reasoning_effort` 若传 `low`/`medium` 会被百度拒绝（只收 high/max）；`video_url` 模态无内置支持（可 bodyOverrides）
- **建议动作**：profile 新增思考映射（如 DeepSeek override 同类机制，按模型关键字分派 enable_thinking/thinking/reasoning_effort）；或文档指引 bodyOverrides

#### 3. 证据与验证

- **证据等级**：B + C
- **验证状态**：🔲 未验证（仅文档/适配代码引用）
- **存疑标记**：-

---

### bailing — Bailing (蚂蚁百灵 Ant Ling)

- **registry 现状**：profile=`full()` · base_url=`https://api.ant-ling.com/v1` · env=`BAILING_API_TOKEN`（[openai_compat_registry.rs](D:\code\aimux\aimux-providers\src\openai_compat_registry.rs#L304)）
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 标准 OpenAI 命名；`temperature` 范围 [0,1]、`top_p` (0,1]（略异于 OpenAI 但同构）；非流式调用 90s 超时建议开 stream | - | C | https://developer.ant-ling.com/zh-CN/docs/api-reference/openai/ |
| 能力支持 | tools（function calling）标准；**无 top_k/logprobs 提及** | - | C | 同上 |
| 思考机制 | **by-model 两个私有对象**：`thinking.type`（`enabled`/`disabled`，默认 enabled，仅 Ling-3.0-flash）；`reasoning.effort`（`high`/`xhigh`，默认 high，仅 Ring-2.6-1T） | `{"model":"Ling-3.0-flash","thinking":{"type":"disabled"}}`；`{"model":"Ring-2.6-1T","reasoning":{"effort":"xhigh"}}` | C | https://developer.ant-ling.com/zh-CN/docs/api-reference/openai/ |
| 流式/usage | 无差异（SSE，`data:` 逐块；usage 在无 choices 块） | `data:{"id":"...","choices":[{"delta":{"content":"你好","role":"assistant"},"index":0}],"created":...,"model":"Ling-3.0-flash","object":"chat.completion.chunk","usage":null}` | C | 同上 |
| 消息格式 | 无差异（system/user/assistant 纯文本） | - | C | 同上 |
| 特殊字段 | **联网搜索字段**：`enable_search`（bool，默认 false）、`search_options.forced_search`（bool，默认 false）——非标准 request 字段 | `{"model":"Ling-3.0-flash","enable_search":true,"search_options":{"forced_search":true}}` | C | 同上 |
| headers/认证 | 无差异（`Authorization: Bearer <token>`） | - | C | 同上 |
| URL/端点 | 无差异（`POST https://api.ant-ling.com/v1/chat/completions`） | - | C | 同上 |
| 模型 ID | 模型 ID 含命名空间风格：`Ling-3.0-flash`、`Ling-2.6-1T`、`Ring-2.6-1T`、`AntAngelMed\...` | `"model": "Ring-2.6-1T"` | C | 同上 |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖（thinking/reasoning/enable_search ❌）
- **aimux 代码位置**：`openai_compat_registry.rs:304-312`；`convert.rs:1285-1317`（白名单无 enable_search/search_options/reasoning/thinking）
- **差距说明**：`thinking.type`/`reasoning.effort`/`enable_search`/`search_options` 均需 bodyOverrides；`thinking.type` 与 DeepSeek override 的 `thinking` 对象结构（convert.rs:1500-1503）同构，可复用机制
- **建议动作**：复用 DeepSeek override 的 thinking 对象生成逻辑（按 provider 映射）；enable_search 走 bodyOverrides；补 cassette

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证（仅官方文档引用）
- **存疑标记**：-

---

### baseten — Baseten

- **registry 现状**：profile=`full()` · base_url=`https://inference.baseten.co/v1` · env=`BASETEN_API_KEY`（[openai_compat_registry.rs](D:\code\aimux\aimux-providers\src\openai_compat_registry.rs#L313)）
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（OpenAI 兼容） | - | C | https://docs.baseten.co/inference/model-apis/overview |
| 能力支持 | 无差异（结构化输出/工具调用走 OpenAI 参数） | - | C | https://docs.baseten.co/inference/model-apis/overview |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer） | - | C | https://docs.baseten.co/reference/inference-api/chat-completions |
| URL/端点 | 无差异（`/v1/chat/completions` 与 `/v1/completions` 都提供） | - | C | https://docs.baseten.co/reference/inference-api/chat-completions |
| 模型 ID | 模型 ID = **部署 ID/模型 ID**（用户自部署模型） | - | C | https://docs.baseten.co/inference/model-apis/overview |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖（无差异；已有 thin wrapper cassette [cassettes/baseten](D:\code\aimux\aimux-providers\tests\cassettes\baseten)）
- **aimux 代码位置**：`openai_compat_registry.rs:313-321`
- **差距说明**：无
- **建议动作**：无需动作

#### 3. 证据与验证

- **证据等级**：A（thin wrapper cassette）+ C
- **验证状态**：✅ 已验证（有 A 级 cassette）
- **存疑标记**：-

---

### berget — Berget.AI

- **registry 现状**：profile=`full()` · base_url=`https://api.berget.ai/v1` · env=`BERGET_API_KEY`（[openai_compat_registry.rs](D:\code\aimux\aimux-providers\src\openai_compat_registry.rs#L322)）
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
| headers/认证 | 无差异（Bearer） | - | C | https://api.berget.ai/ |
| URL/端点 | 无差异（`POST https://api.berget.ai/v1/chat/completions`） | - | C | https://api.berget.ai/（OpenAPI） |
| 模型 ID | 无差异（OpenRouter 风格前缀 `berget/google/gemma-4-31B-it`，透传即可） | - | C | https://mastra.ai/models/providers/berget |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖（无差异）
- **aimux 代码位置**：`openai_compat_registry.rs:322-330`
- **差距说明**：无
- **建议动作**：无需动作

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：-

---

### bigmodel — BigModel (智谱 GLM)

- **registry 现状**：profile=`full()` · base_url=`https://open.bigmodel.cn/api/paas/v4` · env=`BIGMODEL_API_KEY`（[openai_compat_registry.rs](D:\code\aimux\aimux-providers\src\openai_compat_registry.rs#L331)）
- **变体**：-（智谱国际站 Z.ai 为独立注册项 `zai`，同协议 `https://api.z.ai/api/paas/v4`，`openai_compat_registry.rs:2213-2226`）

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | `max_tokens` 标准；`temperature` 默认 0.6、`top_p` 默认 0.95；`do_sample=false`（temperature=0）在 OpenAI 调用中不适用 | - | C | https://docs.bigmodel.cn/cn/guide/develop/openai/introduction |
| 能力支持 | tools/function calling ✓；图像理解 `image_url`（base64）✓；response_format json_object ✓ | `{"content":[{"type":"image_url","image_url":{"url":"data:image/jpeg;base64,..."}}]}` | C | https://docs.bigmodel.cn/cn/guide/develop/openai/introduction |
| 思考机制 | **thinking 对象**：`{"thinking":{"type":"enabled"}}` 开思考（可加 `budget_tokens` 如 1024）；`reasoning_content` 在流式 delta 中返回。兼容层**不支持 reasoning_effort**（阿里聚合 GLM 文档明确「兼容方式不支持 reasoning_effort 参数」） | `extra_body={"thinking":{"type":"enabled"}}`；`{"thinking":{"type":"enabled","budget_tokens":1024}}` | C | https://docs.bigmodel.cn/cn/guide/develop/openai/introduction · https://help.aliyun.com/zh/model-studio/glm |
| 流式/usage | 无差异（SSE；`delta.reasoning_content` 先于 `delta.content`） | `delta.reasoning_content: "..."` | C | https://docs.bigmodel.cn/cn/guide/develop/openai/introduction |
| 消息格式 | 无差异（历史回传 reasoning_content 的约定同 OpenAI 思考模型） | - | - | - |
| 特殊字段 | `thinking` 对象（见上）；无 cache/store/metadata 特例 | - | C | 同上 |
| headers/认证 | 无差异（Bearer） | - | C | https://docs.bigmodel.cn/cn/guide/develop/openai/introduction |
| URL/端点 | **路径 `/api/paas/v4`（无 /v1 段）**：`https://open.bigmodel.cn/api/paas/v4/chat/completions`；社区踩坑：OpenAI SDK 若强制拼 `/v1` 会 404。aimux model.rs:73 直接拼接 `/chat/completions`，路径正确 | `POST https://open.bigmodel.cn/api/paas/v4/chat/completions` | C | https://docs.bigmodel.cn/cn/guide/develop/openai/introduction · https://github.com/farion1231/cc-switch/issues/1013 |
| 模型 ID | 无差异（`glm-5.2`、`glm-4.5-air`、`glm-4v-plus` 透传） | - | C | 同上 |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:331-339`（`full()`）；`convert.rs:1485-1504`（DeepSeek override 已实现 `{"thinking":{"type":"enabled|disabled"}}` 同构机制，但仅绑定 deepseek profile）；[cassettes/zai/test_zai_thinking_mode.json](D:\code\aimux\aimux-providers\tests\cassettes\zai\test_zai_thinking_mode.json)（A 级证据：`{"thinking":{"type":"enabled","clear_thinking":false}}` 请求形状）
- **差距说明**：bigmodel 未绑定任何 override，`thinking` 对象只能靠用户 bodyOverrides 注入；zai 条目同协议已有 thinking cassette 但同样是通用通道（无自动映射）
- **建议动作**：把 DeepSeek 的 thinking 对象 override 泛化为 provider 映射（bigmodel/zai/baidu/ark 复用）；补 bigmodel thinking cassette；`budget_tokens` 透传

#### 3. 证据与验证

- **证据等级**：A（zai cassette）+ C（官方文档）
- **验证状态**：🔲 未验证（cassette 存在但思考路径未全部接线测试）
- **存疑标记**：-

---

### blueclaw — Blue Claw

- **registry 现状**：profile=`full()` · base_url=`https://openai.blueclaw.network/v1` · env=`BLUECLAW_API_KEY`（[openai_compat_registry.rs](D:\code\aimux\aimux-providers\src\openai_compat_registry.rs#L340)）
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
| headers/认证 | 无差异 | - | - | - |
| URL/端点 | 无差异（子域名 `openai.` 前缀为协议分流，无 request 影响） | - | - | - |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖（按 OpenAI 兼容假设）
- **aimux 代码位置**：`openai_compat_registry.rs:340-348`
- **差距说明**：**⚠️ 证据不足**：未找到 blueclaw.network 官方 API 文档
- **建议动作**：保持现状，列入存疑归档

#### 3. 证据与验证

- **证据等级**：-
- **验证状态**：🔲 未验证
- **存疑标记**：⚠️ 存疑（无出处）

---

### bytedance — ByteDance (火山方舟 Ark)

- **registry 现状**：profile=`full()` · base_url=`https://ark.cn-beijing.volces.com/api/v3` · env=`ARK_API_KEY`（[openai_compat_registry.rs](D:\code\aimux\aimux-providers\src\openai_compat_registry.rs#L349)）
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | `max_tokens` 标准；top_p/temperature/stop 标准 | - | C | https://doubao.apifox.cn/265897481e0（官方文档镜像） |
| 能力支持 | tools ✓（⚠️ 工具 schema 中 `minLength/maxLength/minItems/maxItems/minContains/maxContains` 关键字会被 Ark 拒绝，需剔除）；多模态：image_url 多图、video、audio | - | C | https://docs.openclaw.ai/zh-CN/providers/volcengine |
| 思考机制 | **thinking 对象**：`{"thinking":{"type":"enabled|disabled"}}`（Doubao 深度思考开关，仅保留开/关语义）；`deepseek-reasoner` 系模型在 Ark 上通过注入「回答前先用 <think></think> 输出思考过程」system 提示词模拟思考 | `{"model":"doubao-seed-1-6","reasoning_effort":"none"}` → `{"thinking":{"type":"disabled"}}`；system 注入示例 `{"role":"system","content":"回答前，都先用 <think></think> 输出你的思考过程。"}` | B | [reference/aiproxy/docs/REASONING_COMPATIBILITY.md](D:\code\aimux\reference\aiproxy\docs\REASONING_COMPATIBILITY.md#L436) · [reference/aiproxy/docs/REASONING_COMPATIBILITY.md](D:\code\aimux\reference\aiproxy\docs\REASONING_COMPATIBILITY.md#L1280) |
| 流式/usage | 无差异（SSE）；usage 含 `prompt_tokens_details.cached_tokens`（缓存计费字段） | `"usage":{"completion_tokens":601,"prompt_tokens":989,"total_tokens":1590,"prompt_tokens_details":{"cached_tokens":0}}` | C | https://doubao.apifox.cn/265897481e0 |
| 消息格式 | 无差异（多模态 content 数组标准） | - | C | 同上 |
| 特殊字段 | 无 cache/store/metadata 特例（Ark 缓存为服务端自动，无请求字段） | - | - | - |
| headers/认证 | 无差异（`Authorization: Bearer <token>`） | - | C | https://doubao.apifox.cn/265897481e0 |
| URL/端点 | **`/api/v3`** 前缀：`https://ark.cn-beijing.volces.com/api/v3/chat/completions`；Coding Plan 走 `/api/coding/v3`；推理接入点（ep-xxx）与模型 ID 同字段 | `POST https://ark.cn-beijing.volces.com/api/v3/chat/completions` | C | https://doubao.apifox.cn/265897481e0 · https://docs.openclaw.ai/zh-CN/providers/volcengine |
| 模型 ID | **带日期的快照 ID**：`doubao-vision-pro-32k-2410128`、`doubao-seed-1-8-251228`；`ep-<id>` 推理接入点 ID 直接作 model | `"model": "doubao-vision-pro-32k-2410128"` | C | https://doubao.apifox.cn/265897481e0 |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖（thinking ❌）
- **aimux 代码位置**：`openai_compat_registry.rs:349-357`；[cassettes/bytedance](D:\code\aimux\aimux-providers\tests\cassettes\bytedance)（thin wrapper，无 thinking）；`convert.rs:1098-1441`
- **差距说明**：`thinking` 对象未内置（同 bigmodel，需 bodyOverrides 或 override 泛化）；工具 schema 关键字剔除（minLength 等）未实现——Groq 的 `prepare_tools_groq` 是现成模式（convert.rs:1401-1407）可参考扩展
- **建议动作**：thinking 对象 override 泛化（bytedance/byteplus）；工具 schema 清洗可加 provider 特定逻辑；补 doubao thinking cassette

#### 3. 证据与验证

- **证据等级**：A（thin wrapper cassette）+ B + C
- **验证状态**：🔲 未验证（thinking 路径未覆盖）
- **存疑标记**：-

---

### byteplus — BytePlus (Volcano 国际版 Ark)

- **registry 现状**：profile=`full()` · base_url=`https://ark.bytepluses.com/api/v3` · env=`BYTEPLUS_API_KEY`（[openai_compat_registry.rs](D:\code\aimux\aimux-providers\src\openai_compat_registry.rs#L358)）
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 同 bytedance（Ark 同协议）：`{"thinking":{"type":"enabled|disabled"}}` | `{"thinking":{"type":"enabled"}}` | B | [reference/aiproxy/docs/REASONING_COMPATIBILITY.md](D:\code\aimux\reference\aiproxy\docs\REASONING_COMPATIBILITY.md#L436) |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer） | - | - | - |
| URL/端点 | 国际域 `ark.bytepluses.com/api/v3`（与国内 ark.cn-beijing.volces.com 同协议）；Seed Speech TTS 为另一服务 | `POST https://ark.bytepluses.com/api/v3/chat/completions` | C | https://docs.openclaw.ai/zh-CN/providers/volcengine |
| 模型 ID | 无差异（同 Ark 风格） | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖（thinking ❌，同 bytedance）
- **aimux 代码位置**：`openai_compat_registry.rs:358-366`
- **差距说明**：同 bytedance
- **建议动作**：与 bytedance 合并处理（同一 Ark 协议，profile/override 可共用）

#### 3. 证据与验证

- **证据等级**：B + C
- **验证状态**：🔲 未验证
- **存疑标记**：-

---

### bytez — Bytez

- **registry 现状**：profile=`full()` · base_url=`https://api.bytez.com/v2` · env=`BYTEZ_API_KEY`（[openai_compat_registry.rs](D:\code\aimux\aimux-providers\src\openai_compat_registry.rs#L367)）
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（`max_tokens` 示例） | `"max_tokens": 150` | C | https://docs.bytez.com/http-reference/examples/openai-compliant/chatCompletionsExample |
| 能力支持 | 无差异（流式/自定义参数支持；**logprobs 全模型支持**）；chat 支持开源与闭源两类模型 | - | C | 同上 |
| 思考机制 | 无差异（透传） | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer）；cohere 模型需额外 `provider-key` 头 | `provider-key: <cohere-key>` | C | 同上 |
| URL/端点 | ⚠️ **注册表 base_url=`https://api.bytez.com/v2` 与官方文档不一致**：官方 OpenAI 兼容 baseURL 为 `https://api.bytez.com/models/v2/openai/v1`（chat/completions 路径） | `baseURL: "https://api.bytez.com/models/v2/openai/v1"` | C | https://docs.bytez.com/http-reference/examples/openai-compliant/chatCompletionsExample |
| 模型 ID | **闭源模型需加供应商前缀**：`openai/gpt-4`、`anthropic/claude-...`；开源模型为 org/name（`Qwen/Qwen3-4B`） | `"model": "openai/gpt-4"`、`"model": "Qwen/Qwen3-4B"` | C | 同上 |

#### 2. aimux 现状对比

- **对比结论**：⚠️ 不一致（base_url）
- **aimux 代码位置**：`openai_compat_registry.rs:367-375`
- **差距说明**：base_url 需核对为 `https://api.bytez.com/models/v2/openai/v1`（或确认 `/v2` 是否为官方别名）；模型 ID 前缀/`provider-key` 头为透传+headers 配置即可覆盖
- **建议动作**：用官方文档 baseURL 实测一次；若 `/v2` 无效则修 registry

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：⚠️ base_url 差异存疑

---

### canopywave — Canopywave

- **registry 现状**：profile=`full()` · base_url=`https://api.canopywave.com/v1` · env=`CANOPYWAVE_API_KEY`（[openai_compat_registry.rs](D:\code\aimux\aimux-providers\src\openai_compat_registry.rs#L376)）
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异（开源模型推理平台） | - | C | https://canopywave.com/ |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer） | - | C | https://canopywave.com/docs/get-started/openai-compatible |
| URL/端点 | 无差异（官方提供 OpenAI client 接入说明：改 base_url 即可） | - | C | https://canopywave.com/docs/get-started/openai-compatible |
| 模型 ID | 无差异 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖（无差异）
- **aimux 代码位置**：`openai_compat_registry.rs:376-384`
- **差距说明**：无
- **建议动作**：无需动作

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证
- **存疑标记**：-

---

### cerebras — Cerebras

- **registry 现状**：profile=`full()` · base_url=`https://api.cerebras.ai/v1` · env=`CEREBRAS_API_KEY`（[openai_compat_registry.rs](D:\code\aimux\aimux-providers\src\openai_compat_registry.rs#L385)）
- **变体**：-

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | **`max_completion_tokens`**（含推理 token 的总输出上限，reasoning 模型默认走此字段）；`max_tokens` 是否兼容随模型 | `"max_completion_tokens": 100` | C | https://inference-docs.cerebras.ai/api-reference/chat-completions |
| 能力支持 | tools ✓ / parallel_tool_calls（默认 true）✓ / response_format ✓ / logprobs ✓；⚠️ **`gpt-oss-120b` 同时传 tools+response_format 会被拒绝**；`developer` 角色与 `system` 同层（gpt-oss-120b） | - | C | https://inference-docs.cerebras.ai/resources/openai |
| 思考机制 | `reasoning_effort` 标准参数直接生效（`none/low/medium/high`）；**非标准参数 `clear_thinking`**（zai-glm-4.7 专属，默认 true，false=保留前轮思考上下文）需 extra_body；gpt-oss 系 always-on 思考（`thinking=False` 会被忽略） | `client.chat.completions.create(model="zai-glm-4.7", reasoning_effort="none", extra_body={"clear_thinking": False})` | B+C | https://inference-docs.cerebras.ai/resources/openai · [reference/pydantic-ai/docs/capabilities/thinking.md](D:\code\aimux\reference\pydantic-ai\docs\capabilities\thinking.md#L51) |
| 流式/usage | 无差异（SSE + usage） | - | - | - |
| 消息格式 | 无差异（含 developer 角色消息类型） | - | C | https://inference-docs.cerebras.ai/api-reference/chat-completions |
| 特殊字段 | `prompt_cache_key`（提示词缓存分组）；`prediction`（Predicted Outputs，content 类型，gpt-oss-120b Public Preview）；头部 `queue_threshold`（flex/auto 服务层排队阈值，50-20000ms，Private Preview）；`Content-Encoding: gzip` 支持 | `{"prediction":{"type":"content","content":"..."}}`、`{"prompt_cache_key":"session-123"}` | C | https://inference-docs.cerebras.ai/api-reference/chat-completions |
| headers/认证 | 无差异（`Authorization: Bearer`）；可选 gzip 压缩头 | - | C | 同上 |
| URL/端点 | 无差异（`https://api.cerebras.ai/v1`） | - | C | https://inference-docs.cerebras.ai/resources/openai |
| 模型 ID | 无差异（`gpt-oss-120b`、`zai-glm-4.7`、`qwen3-coder` 等透传） | - | C | https://inference-docs.cerebras.ai/api-reference/chat-completions |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖
- **aimux 代码位置**：`openai_compat_registry.rs:385-393`；`convert.rs:1118-1130`（max_completion_tokens 已有）；`convert.rs:1297-1308`（prediction/promptCacheKey 已在白名单）；`convert.rs:1327`（reasoning_effort 直通）；[cassettes/cerebras](D:\code\aimux\aimux-providers\tests\cassettes\cerebras)（A 级：thinking wire contract / clear-thinking / disable-reasoning 等）
- **差距说明**：`clear_thinking` ❌ 未内置（需 bodyOverrides）；`queue_threshold` 头可经 headers 配置；tools+response_format 互斥校验未实现（可复用 groq 风格 provider 分支）
- **建议动作**：cassette 已较全（cerebras reasoning wire contract）；补 `clear_thinking` 白名单或文档说明 bodyOverrides；评估 tools+response_format 互斥 warning

#### 3. 证据与验证

- **证据等级**：A（cerebras 多 cassette）+ B + C
- **验证状态**：🔶 部分已验证（reasoning/thinking 有 A 级 cassette，clear_thinking 接线待确认）
- **存疑标记**：-

---

## 存疑归档

以下条目证据不足（未能定位任何官方 API 文档），按 OpenAI 兼容假设保持 `full()`，但需后续补充验证；不参与内置字段差异决策：

| id | display | 问题 |
|----|---------|------|
| ai_router | AI-ROUTER | ai-router.dev 无公开文档 |
| aiand | AIand | aiand.com 无公开 API 文档 |
| aibadgr | AI Badgr | aibadgr.com 无公开 API 文档 |
| aigc2d | AIGC2D | aigc2d.com 无公开 API 文档 |
| ails | AILS | caipacity.com 无公开 API 文档 |
| albert | Albert | albert.ai 无公开 API 文档 |
| apertis | Apertis | stima.tech 无公开 API 文档 |
| api2gpt | API2GPT | api2gpt.com 无公开 API 文档 |
| atlascloud | AtlasCloud | 仅有第三方博客佐证聚合网关定位，无官方 API 文档 |
| blueclaw | Blue Claw | blueclaw.network 无公开 API 文档 |
| apiserpent | API Serpent | 官方定位为 SERP 搜索 API，LLM chat/completions 端点存疑（见条目） |

其他 ⚠️ 点（有证据但需实测/复核）：
- ai21：base_url 域名（api.ai21.ai/v1 vs 集成文档 api.ai21.com/studio/v1）；思考参数名未证实
- azure_ai：GitHub Models 2026-07-30 退役，端点可用性需实测；`api-key` 认证头需配置
- bytez：base_url（/v2 vs /models/v2/openai/v1）需实测
- baichuan：reasoning_content 无官方专门文档（⚠️ 低风险）
