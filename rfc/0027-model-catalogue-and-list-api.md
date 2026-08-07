# RFC-0027: Model List API 与模型配置补充

> 状态:已实现(P1 落地;host 侧合并)
> 日期:2026-08-06
> Issues:#79(model list API)、#80(anya2a 配置补充)
> 范围:为 aimux 引入 `Provider::list_models()`,运行时从 provider `/models` 发现可用模型;并用 `models.anya2a.com` 社区聚合数据补充每个模型的配置/能力。请求路径不变。**aimux 是库、不持有状态**:两个原语(`list_models` + `get_model_specs`)由 host 各自调用并按需合并。
> 依赖:RFC-0017(provider config & bodyOverrides)、RFC-0020(external provider config)、RFC-0003(cassette)
> 关联:RFC-0016(providerOptions 白名单)、`docs/internal/model-config-research/`(250 厂商调研)

---

## 1. 背景与动机

aimux 当前对"模型"的认知停留在**字符串 id**:`provider(name, key, model_id, options)` 里 `model_id` 是不透明 `&str`,aimux 从不内省它。两个缺口:

### 1.1 无"可用模型"发现

很多 provider 自带 `GET /v1/models`(OpenAI/DeepSeek/Ollama/OpenRouter/Gemini/Anthropic/Copilot…),返回该账号当前可调用的模型清单。aimux 仓库**已有 7 家 `list_models_smoke` cassette**(`aimux-providers/tests/cassettes/{openai,anthropic,deepseek,ollama,openrouter,gemini,copilot}/list_models_smoke.json`),但**无任何 Rust API 消费它们**——只被 `replay_test.rs` 当 wiremock 路由测试用([replay_test.rs:42](../aimux-providers/tests/replay_test.rs#L42)),`fn list_models` 全仓库 0 匹配。

用户/上游无法问 aimux"这个 key 能用哪些模型",只能凭文档硬编码 model_id,踩"模型不存在/无权访问"的 400。

### 1.2 provider 返回的配置通常不全

provider `/models` 多数只返回 `{id, owned_by, created}`,**不写** context length / reasoning 机制 / 能力开关 / 成本。即使写了也常不完整。用户拿到 id 后仍不知这模型能干什么、该怎么配。

`models.anya2a.com` 是现成的社区聚合知识库,可补充这些字段(详见 §3)。

### 1.3 目标与非目标

| 目标 | 非目标 |
|---|---|
| `Provider::list_models()`:从 provider `/models` 拿可用模型列表 | 不做账号配额/计费查询 |
| `get_model_specs()`:拉取 anya2a 社区画像(纯 fetch + parse) | 不在库里做缓存/TTL/FS 写入(状态由 host 管理) |
| **host 侧合并**:用户把两原语按需 merge(config 补 provider 没写清的字段) | aimux 库内不做合并,不自动套用 config(用户自定义空间) |
| provider 句柄化:`createProvider().listModels()` / `.model()` | 不自建模型评测/基准库 |
| 请求路径不变:options 里本来就有 max_tokens/bodyOverrides 等 | 不做自动路由/成本优化(RFC-0021/0022) |

**核心立场**:sync 产出的 config 是**咨询性**的——给用户读,用户按自己业务决定请求时填什么。aimux 请求路径不做自动填充/自动门控,保留用户自定义空间。

---

## 2. 主入口与业务流程

### 2.1 入口

```
createProvider(name, key)        → Provider 句柄        (新增)
provider.listModels()            → RuntimeModel[]       (新增;仅 provider /models 稀疏产出)
getModelSpecs()                  → Catalogue           (新增;anya2a 社区画像)
provider.model(modelId)          → Model                (现有 language_model 提到句柄上)
generateText(model, prompt, options) → 不变              (options 里带 config)
```

**host 负责 merge**:把 `listModels()` 返回的 `RuntimeModel[]` 与 `getModelSpecs()` 返回的 `Catalogue` 按 `(provider, modelId)` 合并,形成用户可读的完整画像。aimux 库内不做这步——库只给原语,不持有状态。

现有 one-shot `provider(name, key, modelId)` 保留为 `createProvider(name,key).model(id)` 的便捷封装,不破坏老用户。

### 2.2 数据源与输出

```
provider /models API ──→ 可用性权威(这 key 能调什么),config 通常不全   [RuntimeModel]
anya2a catalogue      ──→ 社区知识(补 provider 没写清的字段)           [Catalogue]
                              │
                host 按 (provider, modelId) merge
                              ▼
                (id, spec?)[]   — provider 没写的字段 = anya2a 补
                                  anya2a 也没的 = 留空
                                  provider 没列出的模型 = 不进列表(anya2a 不作可用性依据)
```

两个数据源完全独立:
- **provider `/models`** = 账号级真相(可用性),实时但稀疏
- **anya2a** = 全局知识(配置/能力),丰富但可能滞后;只补列表里出现的 modelId

### 2.3 业务流程

**流程 1 — 取可用模型 + 补充配置(host 侧合并):**

```
const p = createProvider("deepseek", key)
const list = await p.listModels()                    // → RuntimeModel[](仅 id/owned_by/created)
const cat = await getModelSpecs()                    // → Catalogue(anya2a 画像)
const merged = mergeByModelId(list, cat)             // host 合并
  → merged: [{id:"deepseek-chat", config:{context:128000,tools:true}},
              {id:"deepseek-v4",   config:{reasoning:{effort:"high"},context:1000000}}]
```

**流程 2 — use(用户读 config,自己定 options,发请求):**

```
用户从 merged 读到 deepseek-v4 的 config(reasoning effort=high, context=1M)
  → 按业务决定 options
const model = await p.model("deepseek-v4")
await generateText(model, prompt, {max_output_tokens:8000, bodyOverrides:{thinking:{enabled:true}}})
```

config 只给用户读;aimux 不在请求里自动套用。

---

## 3. 数据源调研:`models.anya2a.com`

### 3.1 它是什么

`models.anya2a.com` 是 DeepChat 的 `ThinkInAIXYZ/PublicProviderConf`(Apache-2.0)的 CDN 前端。它**聚合 `models.dev/api.json`** 作基底,叠加自有 provider 集成(ppinfra/tokenflux/groq live/aihubmix/ollama/siliconflow/burncloud),归一化能力标记后输出标准化 JSON。

分发产物(GitHub raw,镜像到 CDN):

| 文件 | 内容 | 大小 |
|---|---|---|
| `dist/all.json` | `{providers:{id→{api,name,doc,display_name,models:[…]}}, updated_at}` | 5.5 MB |
| `dist/{provider}.json` | 单 provider 归一化载荷 | 1.4 KB ~ 927 KB |
| `dist/dc_sync_version.json` | `{updated_at: <ms>}` 版本戳 | 28 B |

### 3.2 为何选 anya2a(对比 models.dev)

实测对比(2026-08-06):

| 维度 | models.dev/api.json | anya2a |
|---|---|---|
| provider 数 | 180 | 186 |
| model 数 | 6132 | 8321 |
| models 结构 | dict | list(归一化) |
| `reasoning` 字段 | bool | `{supported, default}` |
| `extra_capabilities.reasoning` 画像 | ❌ | ✅ effort/budget/interleaved/visibility |
| `type`(chat/embed/image/video/audio) | ❌ | ✅ |
| provider 级 `api`(base_url) | ❌ | ✅ 可与 aimux registry 交叉校验 |
| 版本化同步戳 | ❌ | ✅ `dc_sync_version.json` |

选 anya2a 为主源(models.dev 超集,补齐 base_url/type/reasoning 画像/版本戳)。models.dev 保留为 anya2a 不可达时的降级备选。

### 3.3 anya2a model 字段普查(8321 模型)

| 字段 | 覆盖率 | 补什么 |
|---|---|---|
| `id` / `display_name` | 100% | 模型名/展示名 |
| `type` | 100% | 路由到对应 modality(chat/embed/rerank/image/video) |
| `limit.context` / `limit.output` | ~99.5% | 上下文/输出上限 |
| `modalities.{input,output}` | ~92% | 多模态能力 |
| `tool_call` / `structured_output` | 100% / ~41% | 模型级能力 |
| `reasoning` / `reasoning_options` / `extra_capabilities.reasoning` | 100% / ~44% / ~46% | thinking 机制画像 |
| `cost` | ~74% | 成本(只读元数据) |
| `temperature` | ~67% | 是否接受 temperature |

`reasoning_options` 类型:`effort`(1850)/`toggle`(853)/`budget_tokens`(466)——与 `docs/internal/model-config-research/` P1-A/B 的 thinking 按模型换代清单同构。

### 3.4 provider 命名不一致(关键)

aimux registry 用 snake_case(`siliconflow`/`fireworks`),anya2a 用 kebab-case(`siliconflow`/`fireworks-ai`)。实测 251 vs 186,**精确名重合仅 99**。实现用 `catalogue.rs` 内联别名表(`PROVIDER_ALIASES`)+ 兜底规则(`-`→`_`)覆盖。映射失败不报错,该模型 config 留空。

---

## 4. 设计

### 4.1 Rust core

```rust
// aimux-core/src/provider.rs
pub trait Provider: Send + Sync {
    fn name(&self) -> &str;
    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError>;

    /// 运行时发现:该账号可用模型(仅 provider /models 稀疏数据,无 anya2a 富化)。
    /// 默认返回 Unsupported,由支持的 provider 覆盖。
    fn list_models(&self)
        -> Pin<Box<dyn Future<Output = Result<Vec<RuntimeModel>, AiMuxError>> + Send + '_>>;
}

// aimux-core/src/model_catalogue.rs (新模块)
pub struct RuntimeModel {              // provider /models 的稀疏产出
    pub id: String,
    pub owned_by: Option<String>,
    pub created: Option<u64>,
}

pub struct ModelSpec {                 // anya2a 补充的配置/能力(纯数据)
    pub display_name: Option<String>,
    pub r#type: ModelType,             // Chat | Embedding | Rerank | ImageGen | ...
    pub limits: ModelLimits,           // { context, output, input? }
    pub modalities: ModelModalities,   // { input:[…], output:[…] }
    pub capabilities: ModelCapabilities, // { tool_call, structured_output, temperature, attachment }
    pub reasoning: Option<ReasoningSpec>,
    pub cost: Option<ModelCost>,
    pub source: CatalogueSource,       // Anya2a | ModelsDev | Manual
    pub provider: Option<String>,      // 归一化后的 aimux provider 名
    pub raw: Option<serde_json::Value>,// 原始条目(宽松解析兜底)
}
```

`ModelSpec` 纯数据、宽松反序列化(`#[serde(default)]`),未知字段入 `raw`。`ReasoningSpec` 对齐 anya2a `extra_capabilities.reasoning`。

**库里没有 `ResolvedModel`,不做合并。** `list_models` 返回稀疏 `RuntimeModel`,`get_model_specs` 返回 `Catalogue`;两者由 host 各自调用、按 `(provider, modelId)` 合并(见 `aimux-core/src/model_catalogue.rs` 顶部注释 "deliberately not merged by aimux")。

### 4.2 aimux-providers

**list_models 实现**(OpenAI 兼容,覆盖 251 家):
- `GET {base_url}/models`,Bearer 鉴权,解析 `{data:[{id,object,created,owned_by}]}`
- 复用现有 `http::send` 与 shared client(RFC-0009)
- 返回稀疏 `Vec<RuntimeModel>`,**不查 anya2a、不合并**(合并是 host 的事)
- 已有 7 家 cassette 直接转真测试

**native provider** 各自映射(后续阶段):
- anthropic `GET /v1/models` → `{data:[{id,type,display_name}]}`
- google/vertex `GET /v1beta/models` → `{models:[{name,supportedGenerationMethods,…}]}`
- ollama `GET /api/tags` → `{models:[{name,…}]}`(非 OpenAI 格式,单独适配)

### 4.3 anya2a:薄函数 `get_model_specs`

```rust
// aimux-providers/src/catalogue.rs
pub const DEFAULT_ANYA2A_URL: &str = ".../ThinkInAIXYZ/PublicProviderConf/.../dist/all.json";

/// 纯 fetch + parse:拉 anya2a all.json → Catalogue。无缓存、无 FS 写入、无 TTL。
/// 状态(缓存/持久化)由 host 决定怎么管。
pub async fn get_model_specs(source_url: Option<&str>) -> Result<Catalogue, AiMuxError>;

pub struct Catalogue {
    pub updated_at: u64,
    pub specs: HashMap<String, HashMap<String, ModelSpec>>,  // provider → model_id → spec
}
```

**关键:catalogue 是独立原语,不是 `list_models` 内部依赖。** `get_model_specs` 只负责 fetch + parse;是否缓存、TTL 多少、离线怎么降级,都是 host 的策略,库不持有状态。源码 URL 可用参数覆盖(自建镜像/内网)。

### 4.4 provider 句柄化(binding 层)

binding 现状是 free function(`provider(name,key,modelId) → Model`,无 Provider 对象)。新增 Provider 句柄类型:

```ts
// Node typed wrapper
export class ProviderHandle {
  listModels(): Promise<RuntimeModel[]>
  model(modelId: string): Promise<Model>
}
export function createProvider(name: string, apiKey?: string, config?: ProviderConfig): ProviderHandle
export function getModelSpecs(sourceUrl?: string): Promise<Catalogue>   // 独立原语

// 便捷封装(保留兼容)
export function provider(name, apiKey, modelId, config?): Promise<Model>  // = createProvider().model()
```

8 个 binding 同构。FFI 加 `aimux_provider_new_handle` / `aimux_provider_list_models` / `aimux_provider_model`(句柄 + JSON 边界)。

### 4.5 请求路径:不变

`generateText(model, prompt, options)` 完全不动。options 里已有 `max_output_tokens` / `temperature` / `bodyOverrides` 等——**这就是 model config 在请求里存在的方式**。aimux 不在请求路径读 `ModelSpec`、不自动填充、不自动门控。config 只给用户读,用户自己决定填什么。

---

## 5. Model config 字段整理(咨询)

sync 返回的 `ModelSpec` 字段,对应请求时用户可在 options 里配的项:

| ModelSpec 字段 | 请求时怎么用(options) |
|---|---|
| `limits.context` / `limits.output` | 用户自行做上下文截断;`max_output_tokens` 设多少 |
| `capabilities.tool_call` | 决定是否传 `tools` |
| `capabilities.structured_output` | 决定是否用 `response_format: Json` |
| `reasoning.effort_default` / `mode` | `bodyOverrides` 里填 `thinking:{enabled}` / `reasoning_effort` |
| `cost` | 发送前预估成本(用户自行算) |
| `modalities` | 决定能否传 image/audio 内容 |

**三档配置机制(现状,本 RFC 不改):**
- 强类型 `CallOptions.*`(temperature/top_p/max_output_tokens/…)
- 白名单 `provider_options`(已知 key 才认,未知丢弃)
- 纯透传 `body_overrides`(任意 JSON deep-merge,`null` 删键)

`body_overrides` 已是纯透传(RFC-0017 验证),不另开通道。整理动作 = 文档化边界。

---

## 6. 实现计划

| 阶段 | 内容 | 状态 |
|---|---|---|
| **P1** | `RuntimeModel`/`ModelSpec` 结构(无 `ResolvedModel`);`Provider::list_models` trait 方法(默认 Unsupported,返回稀疏 `RuntimeModel`);OpenAI 兼容实现(251 家);7 家 cassette 转真测试;anya2a 薄函数 `get_model_specs`(纯 fetch+parse)+ 命名映射 | ✅ 已落地 |
| **P2** | binding provider 句柄化(`createProvider` + `listModels` + `model`)+ 独立 `getModelSpecs` 原语;FFI 句柄导出 + `aimux_get_model_specs`;host 侧合并示例 | ✅ 已落地 |
| **P3** | native provider `list_models`(anthropic/google/vertex/ollama/openrouter);host 侧缓存/TTL/离线降级策略(库不持有状态);manual 更新(config 字段表 + 三档机制) | 待做 |

P1 是核心(Rust 层 list_models + anya2a 原语);P2 是 binding 暴露;P3 是 native 覆盖面与 host 侧状态管理。**合并始终在 host,不在库里。**

---

## 7. 向后兼容

| 变更 | 兼容性 | 说明 |
|---|---|---|
| `Provider::list_models` 默认实现 | ✅ | 默认返回 Unsupported,不破坏现有 impl |
| 请求路径 | ✅ | 完全不动,无 config 自动填充/门控 |
| `provider(name,key,modelId)` one-shot | ✅ | 保留为 `createProvider().model()` 便捷封装 |
| `body_overrides`/`provider_options` | ✅ | 本 RFC 不改其语义 |
| FFI 新增导出 | ✅ | 只增不减 |

---

## 8. 与现有 RFC 关系

| RFC | 关系 |
|---|---|
| [0017](0017-provider-config-dx.md) | body_overrides 是请求路径的 config 载体,本 RFC 不动它;ModelSpec 是咨询信息,不进请求路径 |
| [0020](0020-external-provider-config.md) | external provider overlay 是厂商级;list_models 是账号级发现,可叠加 |
| [0016](0016-align-with-aisdk.md) | providerOptions 白名单不在本 RFC 解决 |
| [0003](0003-test-cassette.md) | list_models 复用已有 7 家 cassette + 新增 anya2a 同步 cassette |

---

## 9. 风险

| 风险 | 等级 | 对策 |
|---|---|---|
| anya2a 上游不稳定/停更 | 中 | models.dev 降级备选 + 自建镜像 + bundled 快照;config 留空=回退现状 |
| anya2a schema 变更 | 中 | `ModelSpec` 宽松反序列化,未知字段入 `raw` |
| provider 命名映射漏项 | 中 | 兜底规则 + 失败不报错(config 留空) |
| list_models 鉴权/限流 | 中 | 复用 RetryConfig;失败返回 Unsupported 不 panic |
| all.json 5.5MB 体积 | 低 | 版本化增量(per-provider 文件)+ 本地缓存 |
| 隐私:list_models 暴露账号模型清单 | 低 | 纯客户端拉取,数据不离开 aimux |

---

## 10. Open Questions

1. **bundled 快照 ship 不 ship?** 倾向 core 不 bundled,提供 `aimux-cli catalogue sync` 按需拉取(RFC-0025 已有 cache probe 先例)。
2. **native provider list_models 放 P1 还是 P3?** 倾向 P3(P1 先覆盖 251 家 OpenAI 兼容,价值最大)。
3. **anya2a `cost`/`extra_capabilities.reasoning` 的 visibility/continuation 全量建模还是最小集?** 倾向先取最小集(effort_default/mode/limits/caps),其余入 `raw`。
4. **models.dev 降级真做还是只声明 anya2a 单源?** 倾向后者(anya2a 已是超集),降低实现复杂度。
