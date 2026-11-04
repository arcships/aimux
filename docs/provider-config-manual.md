# Provider Config 用户手册（思考开关 + 字段差异）

> **原则**：配置按 provider 包解释；OpenAI-compatible 的透传规则不适用于所有原生厂商包。用户有两个调用级入口：
> - `reasoning`（由 provider 包映射为其 wire 参数；不支持的设置产生 warning）
> - `provider_options.<provider>`（per-call；OpenAI-compatible 家族里，厂商自己命名空间下 schema 不认识的字段**原样写入请求体**，用户定义一切厂商差异）
>
> 声明式 `bodyOverrides`（provider 级 + per-call 的 JSON deep-merge）已删除，`ProviderConfig` / `config_json` 里传 `body_overrides` 会报 `InvalidArgument`。OpenAI-compatible settings 保留上游的 `transform_request_body` 闭包；OpenAI、Anthropic、Google settings 没有这个字段。
>
> 本手册是"知识"的归宿：各厂商的 wire 参数、配置示例、核实日期。知识会过期——以厂商官方文档为准，本手册仅作参考。
> 数据来源：[model-config-research/](internal/model-config-research/)（2026-08-01 全网调研，250 家）。
>
> **0.6 调用方式。** 绑定不再提供 `generateText` / `createProvider`。Node 用 `Client` 创建 provider，再 `model.generate(prompt, options)`。对外 JSON 是 camelCase：`maxOutputTokens`、`providerOptions`。Rust 源码字段仍是 `max_output_tokens`、`provider_options`，序列化后同为这两个 camelCase 键。下面示例里的对象就是 `providerOptions` 的值。

---

## 1. reasoning：按包映射

```ts
await model.generate(prompt, {
  reasoning: 'none', // or 'minimal' | 'low' | 'medium' | 'high' | 'xhigh'
})
// 枚举另有 provider-default（不传档位）。上面列出的是可设档位。
```

- OpenAI-compatible 默认把档位写成 `reasoning_effort`；原生 provider 按包规则映射。例如 Groq 将 minimal 映射为 low、xhigh 映射为 high；DeepSeek 将 minimal/low 映射为 low、medium/high 映射为 high、xhigh 映射为 max。
- 厂商认识哪个档位是厂商的事：
  - OpenAI 官方：minimal/low/medium/high 等（官方 API 枚举）
  - DeepSeek V4：官方接受 low/high/max/**xhigh**（自行映射，见 §2）
  - Perplexity：minimal/low/medium/high（四档）
  - Kimi k3：low/high/max（三档）
  - 不认识的厂商：忽略或报错，由用户自行处理（或用 `provider_options.<provider>` 直接控制）

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
// 显式关思考（优先于 reasoning）
await model.generate(p, {
  providerOptions: { deepseek: { thinking: { type: 'disabled' } } },
})

// 开思考 + xhigh 档（官方样例：两参数独立）
await model.generate(p, {
  reasoning: 'xhigh',
  providerOptions: { deepseek: { thinking: { type: 'enabled' } } },
})
```

> 当前 DeepSeek 包中，未显式指定 `thinking.type` 时，`reasoning: 'none'` 会关闭思考，其他显式档位会开启思考；`thinking.type` 优先。`reasoningEffort` 是该包的 option key，不能用 wire 字段名 `reasoning_effort` 替代。

## 3. 思考开关配置示例（按调研，来源见各 batch 文件）

配置示例写在 `provider_options.<key>` 里。OpenAI-compatible 的 `<key>` 接受 provider 名及其 camelCase 形式（`zhipu_v4` → `zhipuV4`），原名含下划线时会收到 deprecation warning；原生包的 namespace 和 option key 按各包 schema 定义。

| 厂商 | 关思考 wire | 配置示例 | 备注/来源 |
|---|---|---|---|
| **GLM / Zhipu**（bigmodel/zai/zhipu_v4） | `thinking:{type:"disabled"}` | `providerOptions: { zhipuV4: { thinking: { type: 'disabled' } } }` | 跨代稳定（batch-01/06） |
| **Qwen 系**（alibaba/baidu） | `enable_thinking: false`；预算 `thinking_budget` [100,16384] | `providerOptions: { alibaba: { enable_thinking: false } }`；开思考 `{ enable_thinking: true, thinking_budget: 16384 }` | qwen3 混合（batch-01）；**纯思考版不可关**；新版混合用消息级 `/no_think` |
| **Kimi / Moonshot** | k3: `reasoning_effort`（low/high/max）；k2.5/k2.6: `thinking:{type:"disabled"}`；k2.7-code: **不可关** | `providerOptions: { moonshotai: { thinking: { type: 'disabled' } } }`（k2.5/k2.6 系） | by-model 三套（batch-03） |
| **MiniMax** | M3: `thinking:{type:"disabled"}`；M2.x: **不可关**（传 disabled 仍思考） | `providerOptions: { minimax: { thinking: { type: 'disabled' } } }` | 开启值 M3 用 `"adaptive"`（batch-04） |
| **方舟 / 豆包**（bytedance/byteplus） | `thinking:{type:"disabled"}`；带预算 `{type:"enabled",budget_tokens:N}` | `providerOptions: { bytedance: { thinking: { type: 'disabled' } } }` | batch-01/02 |
| **DeepInfra** | `reasoning:{enabled:false}` | `providerOptions: { deepinfra: { reasoning: { enabled: false } } }` | 非 thinking 对象（batch-02） |
| **SiliconFlow** | `thinking_budget`（思维链 token 上限，Qwen3 系强制截断；**无 0=关 语义**，关思考走 qwen 系 `enable_thinking`） | `providerOptions: { siliconflow: { thinking_budget: 1024 } }`（调低预算） | batch-05 |
| **Perplexity** | `reasoning_effort` 四档 + `stream_mode`；推理 token **不可强制关闭** | `providerOptions: { perplexity: { stream_mode: 'concise' } }` | batch-05 |
| **Groq** | `reasoning_format`（如 raw）；effort 按包映射 | `providerOptions: { groq: { reasoningFormat: 'raw' } }` | `reasoningEffort` 接受 none/default/low/medium/high |
| **Heroku** | `extended_thinking:{enabled,budget_tokens,include_reasoning}`；未知参数需 `allow_ignored_params` | `providerOptions: { heroku: { extended_thinking: { enabled: true, budget_tokens: 2000 } } }`；非标准参数一并 `providerOptions: { heroku: { allow_ignored_params: true, ... } }` | batch-03 |
| **Hetzner** | `chat_template_kwargs:{enable_thinking:false}` | `providerOptions: { hetzner: { chat_template_kwargs: { enable_thinking: false } } }` | 社区实测（batch-03） |
| **Venice** | `venice_parameters:{disable_thinking:true}` | `providerOptions: { venice: { venice_parameters: { disable_thinking: true } } }` | 封闭字段（batch-06） |
| **腾讯 hy3**（TokenHub） | `thinking:{type:"enabled"}` + `reasoning_effort`（默认 low） | `providerOptions: { tencentTokenhub: { thinking: { type: 'enabled' }, reasoning_effort: 'low' } }` | batch-06（C 级官方文档） |
| **StepFun** | `reasoning_format: "general"/"deepseek-style"` | `providerOptions: { stepfun: { reasoning_format: 'deepseek-style' } }` | batch-05 |

## 4. max_tokens_key（内置修复，用户无感）

registry 内置了 7 家兼容厂商的 max tokens 字段名差异（**纯内部数据，用户不需要配置**）：

- 只认 `max_tokens`：stepfun / siliconflow / sarvam / reka_ai / publicai / perplexity
- 只认 `max_completion_tokens`：heroku（官方要求）
- 原生 Groq 和 DeepSeek 使用 `max_tokens`，不走 registry 推断。

```ts
// 绑定写 maxOutputTokens。Rust 写 max_output_tokens。aimux 按厂商选择 wire 字段名。
await model.generate(prompt, { maxOutputTokens: 4096 })
// OpenAI 推理模型 → {"max_completion_tokens":4096}
// stepfun → {"max_tokens":4096}
```

> 使用顶层 `maxOutputTokens`（Rust 字段 `max_output_tokens`），由包选择 wire 字段名。`providerOptions`
> 必须是 namespace → object 的结构；`providerOptions.maxCompletionTokens`
> 直接放数字不符合结构，不能作为 max tokens 的配置入口。
>
> 未内置 `max_tokens_key` 的兼容厂商走默认推断：推理模型发
> `max_completion_tokens`，非推理发 `max_tokens`。原生 DeepSeek 始终将
> `max_output_tokens` 写为 `max_tokens`。

## 5. `provider_options` 透传用法速查

```ts
// OpenAI-compatible per-call：厂商命名空间下 schema 不认识的字段原样进入请求体
await model.generate(prompt, {
  providerOptions: { alibaba: { enable_thinking: false } },
})

// OpenAI-compatible 已知字段（user / reasoningEffort / textVerbosity / strictJsonSchema）按 schema 处理，
// 其余字段（thinking、enable_thinking、extended_thinking …）直接透传。
// 通用命名空间 openaiCompatible 只认 schema 字段，未知字段会被丢弃——厂商专属字段请写在厂商自己的命名空间下。
```

Rust 的 OpenAI-compatible 包支持对**最终**请求体做整体改写（provider 级、每次请求生效、可删字段），用该包 settings 的 `transform_request_body`：

```rust
let provider = create_openai_compatible(OpenAICompatibleProviderSettings {
    name: "acme".into(),
    base_url: "https://api.acme.example/v1".into(),
    transform_request_body: Some(Arc::new(|mut body| {
        body["enable_thinking"] = json!(false);
        body.as_object_mut().unwrap().remove("reasoning_effort");
        body
    })),
    ..Default::default()
})?;
```

JS / Python / Go 等绑定没有闭包入口，provider 级的固定字段请在每次调用的 `provider_options` 里带上。

## 6. 核实日期与来源

- 手册条目核实日期：2026-08-02
- 调研数据：[model-config-research/_global_table.md](internal/model-config-research/_global_table.md)（P1 差距 + batch-01~06）
- DeepSeek V4 官方：https://api-docs.deepseek.com/guides/thinking_mode/
- ⚠️ 标注条目的来源为推断（batch 文件存疑节），使用前以官方文档复核

## 7. Model List API 与模型配置补充（RFC-0027）

aimux 提供两个**独立原语**,host 各自调用并按需合并(aimux 是库、不持有状态):
- Rust: `Provider::discovery()` → `ProviderDiscovery::list_models()` — 运行时从 provider `/models` 发现可用模型,返回 `RuntimeModel[]`(provider 官方数据,通常只有 id)。
- `get_model_specs()` — 独立拉取 `models.anya2a.com` 社区聚合数据,返回 `ModelSpec` 配置/能力。

aimux **不自动合并**二者:`list_models` 不带 anya2a 配置,`get_model_specs` 不带可用性。host 自行按 modelId 把 spec 合并进 `list_models` 的结果。

### 7.1 使用方式

```ts
const client = new Client()
const provider = await client.provider({
  package: 'deepseek',
  settings: { apiKey },
})

// 列出可用模型：provider.invoke method "listModels"（RuntimeModel[]，通常只有 id）
const models = await provider.invoke('listModels')

// 独立拉取 anya2a 社区配置（catalogue.fetch，无缓存）。host 自行按 modelId 合并。
const catalogue = await client.catalogue({})
// specs 按 provider.create 接受的名字分组；anya2a 中没有对应 aimux provider 的条目放在
// catalogue.unmapped，按 anya2a 自己的 provider id 分组，aimux 不猜名字。

const model = await provider.model('deepseek-chat', 'languageModel')
await model.generate(prompt, {
  maxOutputTokens: 8000,
  providerOptions: { deepseek: { thinking: { type: 'enabled' } } },
})
```

### 7.2 config 是咨询性的

`get_model_specs` 返回的 `spec`(ModelSpec)是**纯咨询**信息——给用户读,用户按自己业务决定请求时填什么。aimux **不在请求路径自动套用** config(不自动填充默认值、不自动门控能力)。这保留用户自定义空间。

### 7.3 两个独立数据源

| 数据源 | API | 角色 | 特点 |
|---|---|---|---|
| provider `/models` | `list_models()` | 可用性权威 | 账号级(这 key 能调什么),实时但稀疏(通常只有 id) |
| anya2a 社区聚合 | `get_model_specs()` | 补充配置/能力 | 社区知识(context/reasoning/cost),丰富但可能滞后 |

anya2a 只补 provider 列表里出现的 modelId,不作可用性依据。缺字段留空。两者**独立拉取、无内置缓存**:`list_models` 每次实时调 provider,`get_model_specs` 是 thin fetch(无 FS 写入、无 TTL);host 决定如何缓存/持久化。

### 7.4 覆盖范围

Rust 通过 `Provider::discovery()` 检查厂商是否提供模型发现,返回 `None` 表示未提供，再调用 `list_models()`。绑定没有 0.5 的 `listModels()` 方法：操作是 `provider.invoke`，`method` 为 `"listModels"`。`get_model_specs` 仍是 Rust 函数；绑定对应全局操作 `catalogue.fetch`。预设表的行数以生成的 [api/providers.md](api/providers.md) 为准。Groq 和 DeepSeek 由各自厂商包提供发现。

### 7.5 无内置缓存(不存在缓存/离线环境变量)

aimux 是库、不持有状态:`list_models` 与 `get_model_specs` 都是**实时拉取、无内置缓存**——没有缓存目录、没有 TTL、没有离线模式。如需缓存/离线,由 host 自行包装(例如把 `get_model_specs` 结果落盘并设过期)。

> ⚠️ 不存在 `~/.cache/aimux/catalogue/`、`AIMUX_CATALOGUE_DIR`、`AIMUX_CATALOGUE_OFFLINE`、24h TTL 等——这些并非 aimux 提供的能力。catalogue 未命中(或未调用 `get_model_specs`)= 无 spec,行为与不补充配置时一致。

### 7.6 ModelSpec 字段 → 请求 options 对照

| ModelSpec 字段 | 请求时怎么用(options) |
|---|---|
| `limits.context` / `limits.output` | 用户自行做上下文截断;`max_output_tokens` 设多少 |
| `capabilities.tool_call` | 决定是否传 `tools` |
| `capabilities.structured_output` | 决定是否用 `response_format: Json` |
| `reasoning.effort_default` / `mode` | `provider_options.<provider>` 里填 `thinking:{enabled}` / `reasoning_effort` |
| `cost` | 发送前预估成本(用户自行算) |
| `modalities` | 决定能否传 image/audio 内容 |
