# Provider Config 用户手册（思考开关 + 字段差异）

> **原则**：aimux **不内置任何厂商映射**（RFC-0017 v3）。用户只有两个入口：
> - `reasoning`（7 档，直传 `reasoning_effort`，厂商自决——支持则生效，不支持则忽略/报错）
> - `bodyOverrides`（provider 级 + per-call，JSON deep-merge，用户定义一切厂商差异）
>
> 本手册是"知识"的归宿：各厂商的 wire 参数、配置示例、核实日期。知识会过期——以厂商官方文档为准，本手册仅作参考。
> 数据来源：[model-config-research/](internal/model-config-research/)（2026-08-01 全网调研，250 家）。

---

## 1. reasoning：7 档直传

```ts
await generateText(model, prompt, { reasoning: 'none' | 'minimal' | 'low' | 'medium' | 'high' | 'xhigh' })
// 注：枚举另有 provider-default（不传档位），实际可设档位 6 个
```

- aimux 把档位原样写成 `reasoning_effort` 字段（**无归一化**）
- 厂商认识哪个档位是厂商的事：
  - OpenAI 官方：minimal/low/medium/high 等（官方 API 枚举）
  - DeepSeek V4：官方接受 low/high/max/**xhigh**（自行映射，见 §2）
  - Perplexity：minimal/low/medium/high（四档）
  - Kimi k3：low/high/max（三档）
  - 不认识的厂商：忽略或报错，由用户自行处理（或用 bodyOverrides 直接控制）

## 2. DeepSeek V4（官方核实 2026-08-02）

来源：https://api-docs.deepseek.com/guides/thinking_mode/

| 参数 | 取值 | 说明 |
|---|---|---|
| `thinking.type` | `"enabled"` / `"disabled"` | 思考开关，**默认 enabled** |
| `reasoning_effort` | `"low"` / `"high"` / `"max"` | 三档（无 medium/minimal）；官方接受 `xhigh` |
| effort 映射 | `xhigh` → flash: `high` / pro: `max`；pro 的 `low` → 实际 `high` | **按模型不同**（官方表） |
| 默认 | thinking 开启，**默认 effort = high** | — |
| 无效参数 | temperature/top_p/penalty 在思考模式下无效 | 不报错但无效果 |

```ts
// 关思考（v3 迁移：旧版 reasoning:'none' 自动注入已退役）
await generateText(model, p, { bodyOverrides: { thinking: { type: 'disabled' } } })

// 开思考 + xhigh 档（官方样例：两参数独立）
await generateText(model, p, {
  reasoning: 'xhigh',
  bodyOverrides: { thinking: { type: 'enabled' } },
})
```

> ⚠️ 迁移说明：0.1.x 旧版对 DeepSeek 有内置特化（`reasoning:'none'` → 自动注入 `thinking:{type:"disabled"}`）。该内置已退役（2026-08），现在需要显式 `bodyOverrides`。

## 3. 思考开关配置示例（按调研，来源见各 batch 文件）

| 厂商 | 关思考 wire | 配置示例 | 备注/来源 |
|---|---|---|---|
| **GLM / Zhipu**（bigmodel/zai/zhipu_v4） | `thinking:{type:"disabled"}` | `bodyOverrides: { thinking: { type: 'disabled' } }` | 跨代稳定（batch-01/06） |
| **Qwen 系**（alibaba/baidu） | `enable_thinking: false`；预算 `thinking_budget` [100,16384] | `bodyOverrides: { enable_thinking: false }`；开思考 `{ enable_thinking: true, thinking_budget: 16384 }` | qwen3 混合（batch-01）；**纯思考版不可关**；新版混合用消息级 `/no_think` |
| **Kimi / Moonshot** | k3: `reasoning_effort`（low/high/max）；k2.5/k2.6: `thinking:{type:"disabled"}`；k2.7-code: **不可关** | `bodyOverrides: { thinking: { type: 'disabled' } }`（k2.5/k2.6 系） | by-model 三套（batch-03） |
| **MiniMax** | M3: `thinking:{type:"disabled"}`；M2.x: **不可关**（传 disabled 仍思考） | `bodyOverrides: { thinking: { type: 'disabled' } }` | 开启值 M3 用 `"adaptive"`（batch-04） |
| **方舟 / 豆包**（bytedance/byteplus） | `thinking:{type:"disabled"}`；带预算 `{type:"enabled",budget_tokens:N}` | `bodyOverrides: { thinking: { type: 'disabled' } }` | batch-01/02 |
| **DeepInfra** | `reasoning:{enabled:false}` | `bodyOverrides: { reasoning: { enabled: false } }` | 非 thinking 对象（batch-02） |
| **SiliconFlow** | `thinking_budget`（思维链 token 上限，Qwen3 系强制截断；**无 0=关 语义**，关思考走 qwen 系 `enable_thinking`） | `bodyOverrides: { thinking_budget: 1024 }`（调低预算） | batch-05 |
| **Perplexity** | `reasoning_effort` 四档 + `stream_mode`；推理 token **不可强制关闭** | `bodyOverrides: { stream_mode: 'concise' }` | batch-05 |
| **Groq** | `reasoning_format`（如 raw）+ effort **直传**（无归一化） | `bodyOverrides: { reasoning_format: 'raw' }` | batch-03 |
| **Heroku** | `extended_thinking:{enabled,budget_tokens,include_reasoning}`；未知参数需 `allow_ignored_params` | `bodyOverrides: { extended_thinking: { enabled: true, budget_tokens: 2000 } }`；非标准参数一并 `bodyOverrides: { allow_ignored_params: true, ... }` | batch-03 |
| **Hetzner** | `chat_template_kwargs:{enable_thinking:false}` | `bodyOverrides: { chat_template_kwargs: { enable_thinking: false } }` | 社区实测（batch-03） |
| **Venice** | `venice_parameters:{disable_thinking:true}` | `bodyOverrides: { venice_parameters: { disable_thinking: true } }` | 封闭字段（batch-06） |
| **腾讯 hy3**（TokenHub） | `thinking:{type:"enabled"}` + `reasoning_effort`（默认 low） | `bodyOverrides: { thinking: { type: 'enabled' }, reasoning_effort: 'low' }` | batch-06（C 级官方文档） |
| **StepFun** | `reasoning_format: "general"/"deepseek-style"` | `bodyOverrides: { reasoning_format: 'deepseek-style' }` | batch-05 |

## 4. max_tokens_key（内置修复，用户无感）

aimux 内置了 8 家厂商的 max tokens 字段名差异（**纯内部数据，用户不需要配置**）：

- 只认 `max_tokens`：stepfun / siliconflow / sarvam / reka_ai / publicai / perplexity
- 只认 `max_completion_tokens`：groq（max_tokens 已弃用）/ heroku（官方要求）

```ts
// 用户始终写 max_output_tokens，aimux 按厂商自动选字段名
await generateText(model, prompt, { max_output_tokens: 4096 })
// OpenAI 推理模型 → {"max_completion_tokens":4096}
// stepfun → {"max_tokens":4096}
```

> ⚠️ **不要用 `provider_options.maxCompletionTokens` 显式指定**：对只认 `max_tokens`
> 的厂商（stepfun / siliconflow / sarvam / reka_ai / publicai / perplexity），该
> 选项会被**静默丢弃**（不报错、无 warning，backlog B10）。请改用顶层
> `max_output_tokens`——它会被自动映射到正确字段名。
>
> 未内置 `max_tokens_key` 的厂商（含 **DeepSeek**）走默认推断：推理模型发
> `max_completion_tokens`，非推理发 `max_tokens`。DeepSeek 对
> `max_completion_tokens` 的接受性**尚未实测**（backlog B6）——当前行为
> 由默认推断路径决定，实测结论落地前请勿依赖其行为。

## 5. bodyOverrides 用法速查

```ts
// provider 级（创建时，每次请求生效）—— 适合固定字段
const model = await openai(key, 'qwen3-coder', {
  bodyOverrides: { enable_thinking: false },
})

// per-call 级（单次请求，覆盖 provider 级）
await generateText(model, prompt, { bodyOverrides: { enable_thinking: false } })

// null = 删除字段（删除通用路径注入的字段）
await generateText(model, prompt, { bodyOverrides: { 'reasoning_effort': null } })
```

## 6. 核实日期与来源

- 手册条目核实日期：2026-08-02
- 调研数据：[model-config-research/_global_table.md](internal/model-config-research/_global_table.md)（P1 差距 + batch-01~06）
- DeepSeek V4 官方：https://api-docs.deepseek.com/guides/thinking_mode/
- ⚠️ 标注条目的来源为推断（batch 文件存疑节），使用前以官方文档复核

## 7. Model List API 与模型配置补充（RFC-0027）

aimux 支持 `Provider::list_models()` 运行时发现可用模型,并用 `models.anya2a.com` 社区聚合数据补充模型配置/能力。

### 7.1 使用方式

```ts
// 1. 创建 provider 句柄
const p = await createProvider('deepseek', apiKey)

// 2. 列出可用模型(+ anya2a 补充的配置)
const models = await p.listModels()
// → [{ id: 'deepseek-v4', spec: { limits: { context: 1000000 }, reasoning: { effort: 'high' }, ... } }]

// 3. 用户读 spec,按业务自己定 options
const model = await p.model('deepseek-v4')
await generateText(model, prompt, { max_output_tokens: 8000, bodyOverrides: { thinking: { type: 'enabled' } } })
```

### 7.2 config 是咨询性的

`listModels` 返回的 `spec`(ModelSpec)是**纯咨询**信息——给用户读,用户按自己业务决定请求时填什么。aimux **不在请求路径自动套用** config(不自动填充默认值、不自动门控能力)。这保留用户自定义空间。

### 7.3 两层数据源

| 数据源 | 角色 | 特点 |
|---|---|---|
| provider `/models` | 可用性权威 | 账号级(这 key 能调什么),实时但稀疏(通常只有 id) |
| anya2a 缓存 | 补充配置/能力 | 社区知识(context/reasoning/cost),离线缓存、丰富但可能滞后 |

anya2a 只补 provider 列表里出现的 modelId,不作可用性依据。缺字段留空。

### 7.4 覆盖范围

292/325 家真 LLM provider 已覆盖(89.8%):251 registry + 23 standalone + 10 vertex MaaS + 8 native。33 家 modality-only(speech/image/embed/search)返回 `Unsupported`。

### 7.5 缓存与离线

- 默认缓存目录:`~/.cache/aimux/catalogue/`(或 `AIMUX_CATALOGUE_DIR`)
- 默认 TTL:24h
- 离线模式:`AIMUX_CATALOGUE_OFFLINE=1` 只读缓存不联网
- catalogue 未命中 = 回退现状(spec 为 null,行为与今天一致)

### 7.6 ModelSpec 字段 → 请求 options 对照

| ModelSpec 字段 | 请求时怎么用(options) |
|---|---|
| `limits.context` / `limits.output` | 用户自行做上下文截断;`max_output_tokens` 设多少 |
| `capabilities.tool_call` | 决定是否传 `tools` |
| `capabilities.structured_output` | 决定是否用 `response_format: Json` |
| `reasoning.effort_default` / `mode` | `bodyOverrides` 里填 `thinking:{enabled}` / `reasoning_effort` |
| `cost` | 发送前预估成本(用户自行算) |
| `modalities` | 决定能否传 image/audio 内容 |
