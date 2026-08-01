# RFC-0017: 统一 provider 配置与 request override(DX 提升)

> **Status**: DRAFT (pending review)
> **Date**: 2026-08-01
> **Scope**: 统一 aimux 的 provider 配置层——把 Rust core 已有但 Node 未透出的能力(headers/org/project/retry)暴露出来,引入通用 `bodyOverrides`(JSON deep merge)替代闭包式 transform,并为 Qwen/Kimi/GLM 等厂商的思考开关提供可扩展的特化机制
> **Related**: [RFC-0016](0016-align-with-aisdk.md) 对齐 Vercel AI SDK 能力缺口(本 RFC 补齐 M1/M5 + 厂商思考开关特化),[RFC-0009](0009-request-resilience.md) retry(本 RFC 透出 maxRetries)

---

## 1. 背景与动机

### 1.1 问题

aimux 的 Rust core 实现度很高(`OpenAIConfig` 有 headers/org/project/retry 的 builder,`apply_deepseek_override` 实现了 DeepSeek 的 thinking 字段),但存在两个 DX 断层:

1. **Node binding 不透出**:工厂函数 `openai(apiKey, modelId, baseUrl?)` 只有 3 个参数,Rust 的 `with_headers`/`with_org_id`/`with_project`/`with_retry_config` 全部用不到。用户无法设全局 header、无法关闭重试、无法设 org/project。
2. **厂商特化封闭**:`RequestBodyOverride` 是封闭枚举(只有 `DeepSeek`),Qwen/Kimi/GLM 等厂商的思考开关(`enable_thinking`/`thinking_budget`/`thinking:{type}`)无法在不改 Rust 源码的情况下接入。

### 1.2 为什么不用闭包式 transformRequestBody

Vercel AI SDK 的 `createOpenAICompatible` 提供了 `transformRequestBody: (body) => body` 闭包钩子。但在 aimux(Rust core + napi binding)中桥接 JS 闭包到 Rust `Box<dyn Fn>` 需要 napi `ThreadsafeFunction` + JSON 序列化往返,复杂度高且性能差。

分析 Vercel `transformRequestBody` 的实际用途(Fireworks 为例),闭包做的三件事:
- **删除字段**(删 `reasoningHistory`/`promptCacheKey` 等)——这些字段是 Vercel providerOptions 注入的,aimux 不注入就不需要删
- **重命名字段**(`promptCacheKey`→`prompt_cache_key`)——aimux 的 `convert.rs` 已在构建时做了正确的字段命名
- **条件映射**(`minimal`→`low`,`xhigh`→`high`)——aimux 的 Groq profile 已内置

结论:闭包的"独家能力"(删除、条件逻辑)在 aimux 架构下由内置 profile 处理更合适。用户真正需要的是**注入/覆盖字段**——这用一个 JSON 对象 deep merge 就能实现,纯数据,不需要跨语言函数桥接。

### 1.3 设计原则

- **简单优先**:纯 JSON 数据,不引入闭包/函数桥接
- **两级覆盖**:provider 级(每次请求) + per-call 级(单次),后者在前者之后 merge
- **内置特化 + 用户兜底**:厂商思考开关由内置 `RequestBodyOverride` 处理(带 warnings),用户 `bodyOverrides` 做兜底注入

---

## 2. 设计

### 2.1 Provider 工厂:options 对象(透出 Rust 能力)

把工厂函数从固定参数改为接收可选的 options 对象,向后兼容:

```ts
// 之前
openai(apiKey: string, modelId: string, baseUrl?: string): Promise<Model>

// 之后(向后兼容:第 3 参数既接受 string 也接受 options 对象)
openai(apiKey: string, modelId: string, config?: string | ProviderConfig): Promise<Model>

interface ProviderConfig {
  baseUrl?: string
  headers?: Record<string, string>
  organization?: string        // OpenAI 专属
  project?: string             // OpenAI 专属
  maxRetries?: number          // 0 = 关闭重试
  bodyOverrides?: Record<string, any>   // 请求体覆盖(provider 级,每次请求 merge)
}
```

向后兼容策略:napi 层第 3 参数为 `Option<Either<String, ProviderConfigObj>>`。string 走旧路径,对象展开各字段。

### 2.2 bodyOverrides:JSON deep merge

#### Rust 层

`OpenAIConfig` 和 `CallOptions` 各加一个 `body_overrides` 字段:

```rust
pub struct OpenAIConfig {
    // ... 现有字段 ...
    /// 请求体覆盖(provider 级)。在标准请求体 + 内置 override 之后 deep merge。
    pub body_overrides: Option<Value>,
}

pub struct CallOptions {
    // ... 现有字段 ...
    /// 请求体覆盖(per-call 级)。在 provider 级 body_overrides 之后 deep merge。
    pub body_overrides: Option<Value>,
}
```

`build_request_body` 末尾执行顺序:

```rust
// 1. 标准 OpenAI 请求体构建(convert.rs 已有)
// 2. 内置厂商特化(DeepSeek/Qwen/Kimi... 的 RequestBodyOverride)
if let Some(ref override_kind) = profile.request_body_override { ... }

// 3. provider 级 body_overrides(deep merge)
if let Some(ref overrides) = provider_body_overrides {
    deep_merge_json(&mut body, overrides);
}

// 4. per-call body_overrides(deep merge,覆盖 provider 级)
if let Some(ref overrides) = options.body_overrides {
    deep_merge_json(&mut body, overrides);
}
```

#### deep merge 语义

```rust
/// 递归合并两个 JSON 对象。`patch` 中的值覆盖 `target`:
/// - 对象:递归合并
/// - 标量/数组:覆盖
/// - null:删除 target 中对应的 key(显式删除能力)
fn deep_merge_json(target: &mut Value, patch: &Value) {
    match (target, patch) {
        (Value::Object(t), Value::Object(p)) => {
            for (k, v) in p {
                match v {
                    Value::Null => { t.remove(k); }  // null = 删除
                    _ => {
                        deep_merge_json(t.entry(k).or_insert(Value::Null), v);
                    }
                }
            }
        }
        (target, patch) => *target = patch.clone(),  // 标量/数组:覆盖
    }
}
```

`null = 删除` 是唯一需要的"闭包能力"——用户可以用 `{ "reasoning_effort": null }` 删除某个字段。这覆盖了 Vercel 闭包删除字段的场景,且无需函数桥接。

#### Node 层

```ts
// GenerateTextOptions 也加 bodyOverrides(per-call)
interface GenerateTextOptions {
  // ... 现有字段 ...
  maxRetries?: number
  bodyOverrides?: Record<string, any>
}
```

### 2.3 厂商思考开关:内置特化

#### 新增 RequestBodyOverride variants

```rust
pub enum RequestBodyOverride {
    DeepSeek,       // 已有
    /// Qwen: enable_thinking: bool + thinking_budget
    Qwen,
    /// Kimi (Moonshot): thinking_budget(0 = 关闭)
    Kimi,
    /// GLM/Zhipu + MiMo/Minimax: thinking: {type: "enabled"|"disabled"}
    ThinkingToggle,
}
```

统一实现(复用 `reasoning → thinking` 映射):

```rust
fn apply_vendor_override(body: &mut Value, warnings: &mut Vec<Warning>,
                         options: &CallOptions, vendor: RequestBodyOverride) {
    match vendor {
        RequestBodyOverride::DeepSeek => apply_deepseek_override(body, warnings, options),
        RequestBodyOverride::Qwen => {
            if options.reasoning == Some(ReasoningEffort::None) {
                body["enable_thinking"] = json!(false);
            } else if let Some(r) = options.reasoning {
                if r != ReasoningEffort::ProviderDefault {
                    body["enable_thinking"] = json!(true);
                    body["thinking_budget"] = json!(effort_to_budget(r));
                }
            }
        }
        RequestBodyOverride::Kimi => {
            if options.reasoning == Some(ReasoningEffort::None) {
                body["thinking_budget"] = json!(0);
            }
        }
        RequestBodyOverride::ThinkingToggle => {
            // 与 DeepSeek 同形: thinking: {type: "enabled"|"disabled"}
            if options.reasoning == Some(ReasoningEffort::None) {
                body["thinking"] = json!({ "type": "disabled" });
            } else if options.reasoning != Some(ReasoningEffort::ProviderDefault) {
                body["thinking"] = json!({ "type": "enabled" });
            }
        }
    }
}
```

#### providerOptions 细粒度控制

用户也可通过 `providerOptions` 传厂商原汁原味的参数(优先级高于顶层 reasoning,由 override 函数读取):

```ts
// 方式 1:顶层 reasoning(跨厂商通用)
generateText(model, prompt, { reasoning: "none" })

// 方式 2:providerOptions(厂商原汁原味)
generateText(model, prompt, {
  provider_options: { qwen: { enable_thinking: false } }
})

// 方式 3:bodyOverrides(任意字段注入/覆盖)
generateText(model, prompt, {
  bodyOverrides: { enable_thinking: false, thinking_budget: 4096 }
})
```

### 2.4 maxRetries(per-call)

`GenerateTextOptions` 和 `CallOptions` 新增 `max_retries`:

```rust
pub struct GenerateTextOptions {
    // ... 现有字段 ...
    pub max_retries: Option<u32>,
}
```

在 `generate_text`/`stream_text` 入口,如果 `max_retries` 非 None,覆盖 provider config 的 `RetryConfig.max_retries`。

### 2.5 统一的 DX 形态

```ts
import { openai, generateText } from 'aimux'

// 1. 标准厂商:options 对象配置
const model = await openai(apiKey, 'gpt-4o', {
  headers: { 'X-Custom': 'value' },
  organization: 'org-xxx',
  maxRetries: 0,
})

// 2. relay:bodyOverrides 注入 relay 专属字段(provider 级)
const relay = await openai(apiKey, 'deepseek-v4-flash', {
  baseUrl: 'https://my-relay.dev/v1',
  bodyOverrides: { 'X-Relay-Tag': 'my-team' },
})

// 3. Qwen 等厂商:顶层 reasoning 统一控制思考(内置特化)
const qwen = await openai(apiKey, 'qwen3-coder', {
  baseUrl: 'https://dashscope.aliyuncs.com/compatible-mode/v1',
})
await generateText(qwen, prompt, { reasoning: 'none' })  // → enable_thinking: false

// 4. 任意厂商:per-call bodyOverrides 精细覆盖
await generateText(model, prompt, {
  bodyOverrides: {
    enable_thinking: false,        // 注入
    temperature: 0.5,              // 覆盖
    'reasoning_effort': null,      // 删除
  },
})
```

---

## 3. 实现计划

### 阶段 1: binding 透出 + bodyOverrides(低风险)

**改动**:
- `aimux-core/src/options.rs` — `CallOptions` 加 `body_overrides`、`max_retries`
- `aimux-core/src/generate.rs` — `GenerateTextOptions` 加同名字段;`into_call_options` 透传
- `aimux-providers/src/openai/mod.rs` — `OpenAIConfig` 加 `body_overrides` + `with_body_overrides` builder
- `aimux-providers/src/openai/convert.rs` — 新增 `deep_merge_json`;`build_request_body` 末尾 merge(provider 级 + per-call 级)
- `aimux-core/src/util.rs`(或新文件)— `deep_merge_json` 工具函数
- `bindings/node/src/lib.rs` — 工厂函数第 3 参数改 `Option<Either<String, ProviderConfigObj>>`
- `bindings/node/src/types/` — 新增/更新类型

### 阶段 2: 厂商思考开关特化(中等 core 改动)

**改动**:
- `aimux-providers/src/openai/mod.rs` — `RequestBodyOverride` 加 `Qwen`/`Kimi`/`ThinkingToggle`
- `aimux-providers/src/openai/convert.rs` — 新增 `apply_vendor_override`,match 分发
- `aimux-providers/src/openai_compat_registry.rs` — Qwen/Kimi/GLM 厂商声明改用新 profile
- `aimux-providers/tests/` — 新增厂商思考开关测试

---

## 4. 向后兼容

| 变更 | 兼容性 | 说明 |
|------|--------|------|
| 工厂第 3 参数 `string → string \| object` | ✅ | 旧代码传 string 不受影响 |
| `GenerateTextOptions` 加 `max_retries`/`body_overrides` | ✅ | 新字段默认 None |
| `RequestBodyOverride` 加新 variants | ✅ | 枚举加 variant 不破坏现有 DeepSeek 分支 |
| `OpenAIConfig` 加 `body_overrides` | ✅ | 默认 None |

---

## 5. 待支持的厂商思考开关(清单)

| 厂商 | 关闭思考的 wire 参数 | 实现方式 | 状态 |
|------|---------------------|---------|------|
| DeepSeek | `thinking: {type: "disabled"}` | `RequestBodyOverride::DeepSeek` | ✅ 已实现 |
| OpenAI 官方 | `reasoning_effort: "none"` | 通用路径 | ✅ 已实现 |
| Anthropic | thinking config disabled | 独立 convert | ✅ 已实现 |
| Groq | 跳过 reasoning_effort | `groq()` profile | ✅ 已实现 |
| **Qwen** | `enable_thinking: false` | `RequestBodyOverride::Qwen`(阶段 2) | 🔲 待支持 |
| **Kimi (Moonshot)** | `thinking_budget: 0` | `RequestBodyOverride::Kimi`(阶段 2) | 🔲 待支持 |
| **GLM/Zhipu** | `thinking: {type: "disabled"}` | `RequestBodyOverride::ThinkingToggle`(阶段 2) | 🔲 待支持 |
| **MiMo/Minimax** | `thinking: {type: "disabled"}` | `RequestBodyOverride::ThinkingToggle`(阶段 2) | 🔲 待支持 |
| **任意厂商/relay** | 用户自定义 | `bodyOverrides`(阶段 1) | 🔲 待支持 |

---

## 6. 与 RFC-0016 的关系

本 RFC 补齐 RFC-0016 中的:
- **M1**(工厂级 headers/org/project/retry 未透出)→ 阶段 1
- **M5**(transformRequestBody 通用钩子)→ 阶段 1(简化为 `bodyOverrides` JSON merge)
- **H2**(maxRetries 不可配)→ 阶段 1
- 新增:厂商思考开关特化(Qwen/Kimi/GLM/MiMo)→ 阶段 2

---

## 7. 开放问题

1. **deep merge 中 null=删除**:是否需要?Fireworks 用闭包删字段,aimux 不注入那些字段所以一般不需要删。但 `null=删除` 成本极低(一行 match),且给用户显式删除能力,建议保留。
2. **GLM 与 DeepSeek 同形**:`thinking:{type}` 格式相同,合并为 `ThinkingToggle` 还是按厂商分开?建议合并(格式相同,差异由 profile 其他字段表达)。
3. **Qwen thinking_budget 档位映射**:`reasoning` 的 7 档映射到 `thinking_budget` 的具体 token 数需查 Qwen 文档确认。
4. **bodyOverrides 是否够用**:相比 Vercel 闭包,merge body 不能做"读取 body 中某字段值后条件变换"。但这种逻辑性特化由内置 profile 做,用户层只需注入/覆盖——merge 够用。
