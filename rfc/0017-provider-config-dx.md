# RFC-0017: 统一 provider 配置与 request override(DX 提升)

> **Status**: DRAFT (pending review)
> **Date**: 2026-08-01
> **Scope**: 统一 aimux 的 provider 配置层——把 Rust core 已有但 Node 未透出的能力(headers/org/project/retry)暴露出来,引入通用 `bodyOverrides`(JSON deep merge)替代闭包式 transform,并以**配置数据(而非代码枚举)**驱动 Qwen/Kimi/GLM 等厂商的思考开关翻译
> **Related**: [RFC-0016](0016-align-with-aisdk.md) 对齐 Vercel AI SDK 能力缺口(本 RFC 补齐 M1/M5 + 思考开关翻译层),[RFC-0009](0009-request-resilience.md) retry(本 RFC 透出 maxRetries)

---

## 1. 背景与动机

### 1.1 问题

aimux 的 Rust core 实现度很高(`OpenAIConfig` 有 headers/org/project/retry 的 builder,`apply_deepseek_override` 实现了 DeepSeek 的 thinking 字段),但存在两个 DX 断层:

1. **Node binding 不透出**:工厂函数 `openai(apiKey, modelId, baseUrl?)` 只有 3 个参数,Rust 的 `with_headers`/`with_org_id`/`with_project`/`with_retry_config` 全部用不到。用户无法设全局 header、无法关闭重试、无法设 org/project。
2. **厂商特化封闭且知识易过期**:`RequestBodyOverride` 是封闭枚举(只有 `DeepSeek`),Qwen/Kimi/GLM 等厂商的思考开关无法在不改 Rust 源码的情况下接入。更严重的是,思考开关的 wire 参数**按模型分代、不按厂商统一**(调研见 §2.3)——代码内置任何规则表都会过期,必须由配置数据驱动。

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
- **内置映射 + 用户兜底**:厂商思考开关由**内置映射**处理(用户不可见、不可配,仅稳定条目且必须有测试锁住);未覆盖厂商/自定义需求由 `bodyOverrides` 兜底;不可关模型发 warning 不静默

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

### 2.3 厂商思考开关:完全用户定义(不内置)

**设计原则**:aimux **不内置任何厂商思考映射**——`RequestBodyOverride` 枚举整体退役(含 DeepSeek)。用户 API 只有两个,均已存在:

- `reasoning`(7 档)——通用路径透传为 `reasoning_effort`,厂商自决(支持则生效,不支持则报错/忽略,aimux 不猜测)
- `body_overrides`(provider 级 + per-call)——用户定义一切厂商差异

**退役理由**:
1. 内置知识必然过期(Qwen/Kimi 一年换三套机制的先例)
2. 用户 `bodyOverrides` 完全可达——内置只是便利,不是能力
3. 内置需逐厂商维护测试 + provider 粒度误伤(同厂商 by-model 机制不同)
4. 阶段 3 调研证明映射是"知识"而非"机制"——知识放文档(用户手册),机制放代码

**证据:DeepSeek V4 官方机制**(2026-08,api-docs.deepseek.com/guides/thinking_mode/)——恰好证明知识在快速演化,内置必然过期:

| 参数 | 取值 |
|---|---|
| `thinking.type` | `"enabled"`/`"disabled"`(开关,默认 enabled) |
| `reasoning_effort` | `"low"`/`"high"`/`"max"` 三档(无 medium/minimal) |
| effort 映射 | 用户请求 `xhigh` → flash 实际 high / pro 实际 max(**按模型不同**) |
| 默认 | thinking 默认开启,默认 effort=high |
| 无效参数 | thinking 模式下 temperature/top_p/penalty 无效(不报错) |

**用户用法**(文档示例,来自用户手册):

```ts
// DeepSeek V4 关思考
await generateText(model, p, { bodyOverrides: { thinking: { type: 'disabled' } } })

// DeepSeek V4 开思考 + 档位(官方样例:两参数独立)
await generateText(model, p, {
  reasoning: 'xhigh',                       // → reasoning_effort:"xhigh",官方自己映射
  bodyOverrides: { thinking: { type: 'enabled' } },
})
```

**档位透传成立**:`xhigh` 是 DeepSeek 官方接受的有效输入(自行映射 flash→high/pro→max)——"透传 + 厂商自决"无需 aimux 归一化。

**warning 已删除**:v3 直传语义下"无映射未翻译"状态不存在(任何档位必然直传),warning 条件恒假为死代码——删除。`reasoning` 一律透传,厂商自决。

**max_tokens_key 保留**:修 aimux 自身推断 bug(推理模型推断错发 `max_completion_tokens` 给只认 `max_tokens` 的厂商),纯内部数据,非用户概念。

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

// 3. Qwen 等厂商:关思考 = 用户自己定义(bodyOverrides,不内置)
const qwen = await openai(apiKey, 'qwen3-coder', {
  baseUrl: 'https://dashscope.aliyuncs.com/compatible-mode/v1',
  bodyOverrides: { enable_thinking: false },   // provider 级:每次请求关思考
})
// 或 per-call:
// await generateText(qwen, prompt, { bodyOverrides: { enable_thinking: false } })

// 4. 任意厂商:per-call bodyOverrides 精细覆盖
await generateText(model, prompt, {
  bodyOverrides: {
    enable_thinking: false,        // 注入
    temperature: 0.5,              // 覆盖
    'reasoning_effort': null,      // 删除
  },
})
```

### 2.6 provider 分层原则:核心代码 vs JSON 可配置

调研(阶段 3)确认了 provider 的分层边界——**协议不兼容的走核心代码实现,协议兼容且差异可数据化的一律走 JSON**:

| 层 | 内容 | 形态 |
|---|---|---|
| **核心代码实现** | 原生协议 provider:anthropic/google/bedrock/azure/vertex/cohere/mistral/模态厂商——各有自己的 convert/model/stream 代码 | 代码,不可 JSON 化 |
| **JSON 可配置层** | OpenAI 兼容薄封装(250 家):base_url + env + profile 字段(supports_*/stream_usage_key/max_tokens_key 等)——调研已证明全部可数据化 | JSON,运行时加载 |
| **边界特例** | 认证流程特殊的兼容协议(GigaChat OAuth/copilot VSCode headers):协议可 JSON,认证需代码钩子或暂不支持;伪兼容(coze/zai_coding_plan):错误分类,补原生或移除 | 逐个决策 |

推论:
- deepseek/groq 等有特殊 profile 的薄封装,profile 完全数据化(阶段 2)后**同样归入 JSON 层**——registry 250 家无例外
- 核心实现只管协议逻辑;默认 base_url/env/模型清单等外围数据外置
- 单一事实来源:`provider-registry.json`(由阶段 3 调研数据生成),各语言消费同一份数据

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

### 阶段 2: 退役 RequestBodyOverride + max_tokens_key + warning(内部收尾)

**目标**:把厂商思考映射全部移出代码(完全用户定义),修复 max_tokens_key 推断 bug,补齐不静默 warning。用户 API 零变化。

**改动**:
- `aimux-providers/src/openai/mod.rs` — 删除 `RequestBodyOverride` 枚举 + `request_body_override` profile 字段;`deepseek()` profile 回归 `full()`;`OpenAICompatProfile` 加内部字段 `max_tokens_key`
- `aimux-providers/src/openai/convert.rs` — 删除 `apply_deepseek_override` + reasoning_effort 归一化(xhigh→max 等,改为直接透传);`reasoning: none` 无映射且未发 effort → warning(不静默);`max_tokens_key` 分支(6 家 `"max_tokens"` + groq/heroku `"max_completion_tokens"`)
- `aimux-providers/tests/` — 退役回归测试(DeepSeek 旧行为改为文档化)+ max_tokens_key 测试 + warning 测试
- **文档交付** — 调研矩阵 → `docs/provider-config-manual.md` 用户手册(每厂商"关思考/字段差异"的 bodyOverrides 配置示例,含 DeepSeek V4 官方三档 effort 机制)
- **不做**:不新增任何用户可见概念;不内置任何厂商映射

**实施状态(2026-08-02)**:✅ 完成
- 退役:`RequestBodyOverride`/`apply_deepseek_override`/effort 归一化已删除(含 groq 特化),`deepseek()`/`groq()` 回归 full() 语义
- `max_tokens_key`:内部字段 + 8 家厂商接线(6 家 `"max_tokens"` + groq/heroku `"max_completion_tokens"`,stage2-002)
- warning 块删除(直传语义下不可达,v3 无"未翻译"状态)
- 用户手册:`docs/provider-config-manual.md`(DeepSeek V4 官方机制 2026-08 核实)
- 测试:`cargo test --workspace` 全绿(EXIT=0);绑定层零改动

### 阶段 3(新任务): 全网 model request config 调研 + 与 aimux 现状对比

**目标**:统计**全部** OpenAI 兼容厂商(registry 中 ~120 家 thin wrapper + 原生协议厂商的 OpenAI 兼容入口)的 request 强相关特殊配置,输出可追溯的数据清单;并**逐项与 aimux 已实现对比**,得出差距清单,作为 `OpenAICompatProfile` 扩展(思考开关内置映射、`max_tokens_key` 等能力字段)的唯一依据。

**范围**(只收集与 request 构造强相关的,全类别):

| 类别 | 例子 |
|------|------|
| 参数命名差异 | `max_tokens` vs `max_completion_tokens`、`stop` vs `stop_sequences` |
| 能力支持差异 | top_k / tools / tool_choice / response_format / logprobs / parallel_tool_calls / seed / json_mode |
| 思考机制(by-model) | 开关字段/取值/档位映射/是否可关/换代历史(Kimi 三套、Qwen `/no_think`) |
| 流式与 usage 差异 | `stream_options.include_usage`、usage 字段位置(如 Groq `x_groq`)、SSE 事件格式 |
| 消息/内容格式 | `reasoning_content` 别名、tool_result 别名、多模态输入格式(image_url/input_image/file) |
| 特殊请求体字段 | cache 类(`prompt_cache_*`/`cache_control`)、`safety_identifier`、`store`、`metadata`、`prediction`、`service_tier` |
| headers / 认证 | `OpenAI-Organization`/`OpenAI-Project`、X-API-Key vs Authorization、SigV4/OAuth/Basic |
| URL / 端点 | 默认 base_url、路径前缀(`/openai/v1`、`/api/v3`、`/compatible-mode/v1`) |
| 模型 ID 约定 | 别名、前缀映射 |

无特殊配置的厂商也要有记录(一行"无差异"条目),避免调研盲区。

**数据源分层**(按类别分配,不是所有条目都从文档查):

| 类别 | 优先数据源 | 原因 |
|------|-----------|------|
| 稳定类(参数命名/usage 位置/tool 格式/base_url/headers) | `reference/` 完整 clone 项目(rig、pydantic-ai、async-openai 等)**优先**——已实践验证、有测试,比文档可信 | 换代慢,快照过时风险低 |
| 换代类(thinking 机制) | 官方文档**唯一权威**(厂商官网 + 阿里百炼/腾讯云聚合) | 快照必然滞后 |
| 盲区/小厂商 | 在线 litellm/one-api/new-api 最新仓库 + 官方文档 | reference 覆盖不全 |
| 交叉验证 | GitHub issue、社区(gateway 代码 vs 文档对照) | 文档没写的坑 |

**证据要求(硬性,每条目必须)**:每条目必须附**例子**或**证明**之一——禁止只有转述的条目:

- **例子**:实际请求体/响应体片段(JSON),如 `{"thinking":{"type":"disabled"}}` 或 `"max_completion_tokens": 4096`——能直接对照 wire 格式。**思考机制/参数命名/特殊字段类条目必须附请求体示例**,否则视为无证明
- **证明**:可查证证据,按可信度分级:

| 等级 | 形式 | 示例 |
|------|------|------|
| A | 仓库内可运行的测试/cassette(最硬,能跑) | `aimux-providers/tests/cassettes/moonshot/...json`、wiremock 测试名 |
| B | reference 项目代码 `file:line` | `reference/one-api/relay/channel/qwen.go:42` |
| C | 官方文档原文引用(URL + 引用片段) | `help.aliyun.com/...` + "将 enable_thinking 设为 false 关闭思考" |
| D | 仅转述、无出处 | **不允许**——条目标记 ⚠️ 存疑,不参与任何内置/对比 |

- 无例子且无 A/B/C 级证据的条目:只能以"存疑"状态入文档,禁止进入 registry 内置默认、禁止作为对比结论(⚠️ 不一致除外,但需单独复核)
- 验证状态列由证据等级 + 是否跑过测试共同决定(仅文档引用 = 🔲 未验证;有 A 级测试 = ✅ 已验证)

**与 aimux 现状对比**(每条目必做):对照现有实现——`OpenAICompatProfile` 5 字段(supports_top_k/supports_tools/supports_response_format/stream_usage_key/request_body_override)、`convert.rs` 白名单字段、`deep_merge_json`、`bodyOverrides`:

| 对比结论 | 含义 | 后续动作 |
|----------|------|---------|
| ✅ 已覆盖 | aimux 已有相同机制 | 补测试即可 |
| 🔶 部分覆盖 | 字段名/取值不一致(如命名差异) | 记入差距清单,评估 convert 调整 |
| ❌ 未覆盖 | aimux 无此机制 | 记入差距清单,评估 profile 新字段或 bodyOverrides 兜底 |
| ⚠️ 不一致 | aimux 实现与调研结论冲突 | 优先处理(可能是 bug) |

**输出物**:
- `docs/provider-model-config.md` — 全量清单(厂商/模型族/机制/**示例或证据(含等级)**/来源链接/验证状态/**aimux 现状**)
- 差距清单 → `OpenAICompatProfile` 扩展需求(新字段如 `max_tokens_key`、`usage_key` 等,由调研结果驱动,不在本 RFC 预设)
- 只有**带 A/B/C 级证据且已验证**的条目 → registry 数据行 + 对应测试(阶段 2 约束:每条内置规则必须有测试引用)
- 换代频繁的模型家族(如 Qwen/Kimi)不内置,仅记录在文档供用户配置参考
- 存疑条目(D 级无证据)单独一节归档,不混入主清单

**执行方式**(分批,每批可独立并行):
- 按 registry 分组分批(如:国内厂商 / 国际云厂商 / 本地推理 / 网关聚合 / 编程订阅 5 批,每批 ~25 家)
- 每批产出该组清单条目 + aimux 对比结论,合并到 `docs/internal/model-config-research/`(batch-XX.md + `_global_table.md` 差距清单)
- 每批完成后由人审:来源链接必须真实可点;每条目核对**证据等级**(无例子且无 A/B/C 证据 → 打回重做);对比结论需附 aimux 代码位置
- **现状**:调研已完成(2026-08-01,250/250 家),差距清单见 [docs/internal/model-config-research/_global_table.md](../docs/internal/model-config-research/_global_table.md)

### 阶段 4: registry JSON 化 + 通用工厂 + 各语言暴露(架构性,依赖阶段 2)

**目标**:把"新增/修正 provider"从库行为变成纯用户行为;补上绑定层(除 Rust 外)对 250 家 registry 的暴露缺口(现状:Node/Python/Go 等只有 openai/anthropic/deepseek 三个工厂)。

**改动**:
1. `provider-registry.json` — 单一事实来源(250 家:name/display/base_url/env_var/profile 全字段),由阶段 3 调研数据生成;Rust `include_str!` embed,各绑定打包同一份
2. Rust — 保留编译期类型(宏从 JSON 生成,体验不变)+ 新增动态入口 `create_provider(name, api_key, model_id, overrides?)`;用户同名配置覆盖内置数据(修 base_url 不用等发版)
3. 各语言绑定 — 统一暴露通用工厂 `provider(name, apiKey, modelId, config?)`,替代"每家一个工厂"方案;profile 全字段透出(如 max_tokens_key 等,补齐现状 Node 侧无法传 profile 的缺口)
4. 边界特例(§2.6):认证流程特殊者(GigaChat/copilot)标记 unsupported 或后续 auth 钩子;伪兼容(coze/zai_coding_plan)修正分类
5. 校验:启动时对 registry JSON 做 schema 校验(必填字段/base_url 合法性),替代编译期校验

**验收**:
- [ ] 任意语言 `provider("groq", key, "llama-3.3-70b")` 可用,profile 差异生效
- [ ] 用户覆盖内置条目(base_url 修正)生效,无需发版
- [ ] Rust 编译期类型行为不变(全量测试通过)
- [ ] 阶段 3 调研发现的 7 处 registry base_url 错误通过数据修正落地

---

## 4. 向后兼容

| 变更 | 兼容性 | 说明 |
|------|--------|------|
| 工厂第 3 参数 `string → string \| object` | ✅ | 旧代码传 string 不受影响 |
| `GenerateTextOptions` 加 `max_retries`/`body_overrides` | ✅ | 新字段默认 None |
| 退役 `RequestBodyOverride`(阶段 2) | ⚠️ **破坏性**:DeepSeek `reasoning:'none'` 不再自动注入 `thinking:{type:"disabled"}`——需用户 `bodyOverrides`;0.x minor bump 覆盖(该功能 2026-07-28 才上线) | 文档化迁移说明 |
| `OpenAICompatProfile` 加 `max_tokens_key`(内部) | ✅ | 默认 None,行为不变 |
| `OpenAIConfig` 加 `body_overrides` | ✅ | 默认 None |
| registry JSON 化(阶段 4) | ✅ | 编译期类型保留、行为不变;新增动态入口与用户覆盖,纯增量 |

---

## 5. 厂商思考开关(用户手册参考,非内置)

> 阶段 2 后 aimux **不内置**任何映射——下表是用户手册内容,配置方式一律 `bodyOverrides`(provider 级或 per-call)。来源:阶段 3 调研文档 + 官方文档核实。

| 厂商 | 关思考 wire 参数 | 用户配置示例 | 状态 |
|------|-----------------|-------------|------|
| DeepSeek V4 | `thinking:{type:"disabled"}` + `reasoning_effort` 三档 low/high/max(xhigh 官方接受,flash→high/pro→max) | `bodyOverrides: { thinking: { type: 'disabled' } }` | ✅ 官方文档核实(2026-08) |
| OpenAI 官方 | `reasoning_effort: "none"` | `reasoning: 'none'`(通用路径) | ✅ 已实现 |
| Anthropic | thinking config disabled | 独立 convert(现状保留) | ✅ 已实现 |
| Groq | 跳过 reasoning_effort | 现状 profile | ✅ 已实现 |
| Qwen | `enable_thinking: false`(qwen3 混合)/ 不可关(纯思考版)/ `/no_think` | `bodyOverrides: { enable_thinking: false }` | 📖 文档化 |
| Kimi (Moonshot) | `thinking_budget: 0`(k2)/ `thinking:{type}`(k2.5+)/ 不可关(k2.7-code) | `bodyOverrides: { thinking: { type: 'disabled' } }` | 📖 文档化 |
| GLM/Zhipu | `thinking: {type: "disabled"}` | `bodyOverrides: { thinking: { type: 'disabled' } }` | 📖 文档化 |
| MiMo/Minimax | `thinking: {type}`(M2.x)/ `"adaptive"`(M3) | `bodyOverrides: { thinking: { type: 'disabled' } }` | 📖 文档化 |
| **任意厂商/relay** | 用户自定义 | `bodyOverrides`(阶段 1) | ✅ 已实现 |

---

## 6. 与 RFC-0016 的关系

本 RFC 补齐 RFC-0016 中的:
- **M1**(工厂级 headers/org/project/retry 未透出)→ 阶段 1
- **M5**(transformRequestBody 通用钩子)→ 阶段 1(简化为 `bodyOverrides` JSON merge)
- **H2**(maxRetries 不可配)→ 阶段 1
- 新增:思考开关**内置映射**(内部实现,无新增用户 API;调研数据支撑)→ 阶段 2/3
- 新增:registry JSON 化 + 各语言通用工厂(补绑定层暴露缺口)→ 阶段 4

---

## 7. 开放问题

1. **deep merge 中 null=删除**:是否需要?Fireworks 用闭包删字段,aimux 不注入那些字段所以一般不需要删。但 `null=删除` 成本极低(一行 match),且给用户显式删除能力,建议保留。
2. ~~**GLM 与 DeepSeek 同形**~~:**已关闭**——不内置任何映射,全部用户 bodyOverrides。
3. **Qwen thinking_budget 档位映射**:`reasoning` 档位不翻译(现状透传,厂商自决——DeepSeek V4 官方即接受 xhigh 自行映射,验证了该路线);档位知识在用户手册文档化。
4. **bodyOverrides 是否够用**:相比 Vercel 闭包,merge body 不能做"读取 body 中某字段值后条件变换"。这种条件逻辑由用户应用层自行处理(如需条件注入,用户在自己的代码里判断后传不同 bodyOverrides)——merge 够用。
5. ~~**reasoningMap 与 providerOptions 的优先级**~~:**已关闭**——无 reasoningMap;`provider_options` 白名单机制随 RequestBodyOverride 退役,保留字段但仅通用已知 key 有效;`body_overrides` 是用户自定义的唯一通用入口。
6. **退役 DeepSeek 特化的迁移**:0.x minor bump + 用户手册迁移说明(reasoning:'none' → bodyOverrides)。
7. **registry JSON 化的校验时机**:启动时 schema 校验替代编译期校验,错误发现延后——是否可接受?校验规则集(必填字段/base_url 合法/profile 字段范围)需定义。
8. **认证流程特殊厂商**(GigaChat OAuth/copilot device-flow)的落地形态:auth 钩子(代码扩展点)还是暂不支持仅文档化。
9. **Rust 编译期类型与 JSON 的单一事实来源**:宏从 JSON 生成 vs 双源维护——选前者,JSON 为唯一源。
