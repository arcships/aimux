# 调研:允许外部提供 provider 配置的可行性与设计

> **Status**: 调研报告(非 RFC,供决策)
> **Date**: 2026-08-05
> **Scope**: 评估"运行时/宿主应用/配置文件提供 provider 配置"对 aimux 的可行性、形态、成本收益,给出明确建议
> **Related**: [RFC-0017](../rfc/0017-provider-config-dx.md) 配置 DX、[RFC-0019](../rfc/0019-session-affinity.md) 会话亲和

---

## TL;DR(结论先行)

**建议:部分做(推荐)。** 核心结论——

1. **值得做且低成本的部分**:为 **OpenAI 兼容协议** 的 provider 提供外部配置能力(配置文件 + 编程式注册 + 覆盖内置条目)。这高度契合 aimux 现有架构——`provider_registry.json` 本就是"name → 数据"的查找表,`provider()` 工厂已能把条目数据 + `ProviderOptions` 组装成 `Box<dyn LanguageModel>`;外部配置只需在"查条目"这一步注入运行时数据,后端管线零改动。**最小可行形态约 200~400 行 Rust + 各绑定薄透传。**

2. **不建议(或远期)的部分**:动态注册**原生协议**(anthropic / google / bedrock / vertex / azure …)的新 provider。这些是**代码实现**(各自有独立 convert/stream/model 代码,见 [lib.rs:15-24](../aimux-providers/src/lib.rs#L15-L24)),无法用配置数据描述。配置文件驱动只能覆盖 OpenAI 兼容这一层(250 家薄封装的同类),与 RFC-0017 §2.6 的分层定论一致。

3. **关键架构事实**:aimux **从不存储 `dyn Provider`**(`Provider` trait 仅作工厂,`language_model()` 立即产出 `Box<dyn LanguageModel>`);系统唯一流通货币是 `Box<dyn LanguageModel>` / `Arc<dyn LanguageModel>`(已 object-safe、全链路使用)。因此"动态注册"**不需要** `dyn Provider` 注册表,只需一个运行时可变的 name→数据 覆盖层。

---

## 1. aimux 当前配置机制全貌

### 1.1 数据源:编译期嵌入的 registry JSON

- `provider_registry.json`(251 条,见 [provider.rs:292](../aimux-providers/src/provider.rs#L292) `assert_eq!(entries.len(), 251)`)是**唯一数据源**,通过 `include_str!` 在编译期嵌入([provider.rs:66](../aimux-providers/src/provider.rs#L66))。
- 每条 `RegistryEntry`([provider.rs:34-42](../aimux-providers/src/provider.rs#L34-L42)):`name` / `display` / `base_url` / `env_var` / `profile`。`profile` 是纯数据(`supports_top_k`/`supports_tools`/`supports_response_format`/`stream_usage_key`/`max_tokens_key`,见 [provider.rs:45-55](../aimux-providers/src/provider.rs#L45-L55))。
- `REGISTRY` 是 `OnceLock<Vec<RegistryEntry>>`([provider.rs:61](../aimux-providers/src/provider.rs#L61))——**初始化后不可变**,无运行时增删入口。

### 1.2 工厂入口:`provider(name, api_key, model_id, options)`

[provider.rs:119-168](../aimux-providers/src/provider.rs#L119-L168) 是唯一名字驱动入口:

1. `registry().iter().find(|e| e.name == name)`——找不到即 `UnknownProvider`([provider.rs:126-131](../aimux-providers/src/provider.rs#L126-L131))。
2. api_key 为 `None` 时读条目的 `env_var`([provider.rs:133-136](../aimux-providers/src/provider.rs#L133-L136))。
3. 用条目数据初始化 `OpenAIConfig`(base_url / provider / profile),再叠加 `ProviderOptions` 覆盖([provider.rs:138-165](../aimux-providers/src/provider.rs#L138-L165))。
4. `OpenAIProvider::new(config).language_model(model_id)` → `Box<dyn LanguageModel>`([provider.rs:167](../aimux-providers/src/provider.rs#L167))。

**注意**:这条路径**永远构造 `OpenAIProvider`**——registry 只服务 OpenAI 兼容协议。

### 1.3 覆盖能力:`ProviderOptions`(已存在)

[provider.rs:96-110](../aimux-providers/src/provider.rs#L96-L110) 已支持 per-call 覆盖:`base_url` / `headers` / `organization` / `project` / `max_retries` / `body_overrides`。**但只能覆盖已知条目的字段,不能注册新名字,也不能覆盖 `profile`**(profile 来自条目,不在 options 里)。

### 1.4 "与内置表无关"的逃生口:`OpenAIProvider` 基础类

RFC-0017 阶段 4 第 4 点([0017-provider-config-dx.md:359](../rfc/0017-provider-config-dx.md#L359))保留了 `OpenAIConfig`/`OpenAIProvider` 基础类作为"createProvider 等价能力"。即用户可直接:

```rust
let cfg = OpenAIConfig::new(key).with_base_url(url).with_profile(profile);
let model = OpenAIProvider::new(cfg).language_model("model-id")?; // Box<dyn LanguageModel>
```

Node `openai(apiKey, modelId, config)`([lib.rs:283-308](../bindings/node/src/lib.rs#L283-L308))也是此路径——但**不暴露 profile 字段**(`ProviderConfig` 只有 base_url/headers/org/project/max_retries/body_overrides,见 [lib.rs:218-236](../bindings/node/src/lib.rs#L218-L236))。

**现状缺口**:
- (a) 新名字无法被 `provider("my-relay", ...)` 查到——只能用基础类逐次构造,**无"name → 后续复用"机制**。
- (b) 绑定层不暴露 profile,自定义 provider 拿到的是 `full()` profile,无法表达 `max_tokens_key` 等差异。
- (c) 无配置文件加载;无运行时增删;无外部覆盖内置条目。

### 1.5 各绑定配置流

| 绑定 | 入口 | 配置形态 |
|---|---|---|
| Rust | `aimux_providers::provider(name, key, model, opts)` | `ProviderOptions` 结构 |
| Node | `provider(name, apiKey?, model, config?)` + `openai(...)` 等([lib.rs:649-664](../bindings/node/src/lib.rs#L649-L664)) | `ProviderConfig` 对象(JSON string 透传) |
| C ABI | `aimux_provider_new(name, key, model, config_json)`([lib.rs:738-770](../aimux-ffi/src/lib.rs#L738-L770)) | `ProviderOptions` 的 JSON |
| Python/Go/… | 经 FFI JSON 透传 | 同 C ABI |

所有绑定最终汇聚到同一个 `provider()` Rust 函数——**外部配置只需在 Rust core 一处落地,绑定层做薄透传即可,8 语言同步受益**(RFC-0017 阶段 4 已验证此分发模式)。

---

## 2. "外部提供配置"的几种可能形态

| 形态 | 描述 | aimux 现状 | 本调研判断 |
|---|---|---|---|
| **a. 配置文件注册** | YAML/TOML/JSON 声明 provider 列表,运行时加载 | 无 | ✅ **推荐做(最小可行形态的核心)** |
| **b. 编程式注册** | 宿主应用调 API 注册自定义 provider(name+url+key+headers+profile) | 仅基础类逐次构造,无注册表 | ✅ **推荐做(与 a 共享同一覆盖层)** |
| **c. 动态发现/增删** | 网关运行时不重启增删 provider | 无 | ⚠️ **部分做**:增删可行(覆盖层可变),但"动态发现"属网关职责,aimux 定位是接入层不做(见 RFC-0019 §1.4 边界) |
| **d. 外部覆盖/扩展内置 registry** | 外部条目替换或补充内置 251 条 | 无 | ✅ **推荐做(覆盖层 merge 语义天然支持)** |

a/b/d 三者**共用同一个运行时覆盖层**(见 §4),实现上是同一机制的三个入口;区别仅在数据来源(文件 / API 调用 / 两者皆有)。c 的"增删"也复用该层(可变 map),"动态发现"则超出 aimux 定位。

---

## 3. 业界方案对比

### 3.1 LiteLLM(config.yaml 驱动 + 运行时管理)

- **形态**:`config.yaml` 的 `model_list` 声明 `model_name`(用户面名)+ `litellm_params`(model/api_base/api_key/extra_headers/organization/temperature …),`litellm --config` 启动加载([docs.litellm.ai/docs/proxy/configs](https://docs.litellm.ai/docs/proxy/configs))。
- **动态性**:`store_model_in_db` 开启后,Admin UI / `/config/update` API 写数据库,运行时增删模型**无需重启**;DB 模型与 YAML 模型共存(load balance),不互相替换([docs.litellm.ai/docs/proxy/model_management](https://docs.litellm.ai/docs/proxy/model_management))。
- **协议覆盖**:`litellm_params.model` 前缀(`azure/`、`bedrock/`、`openai/`、`ollama/`…)决定走哪个协议实现——**因为 litellm 是 Python,协议实现可在运行时按字符串 dispatch**。
- **优点**:声明式、动态、协议全覆盖(靠 Python 动态分发)。
- **缺点**:配置 schema 庞大;DB 与 YAML 的 deep-merge 语义复杂(`null`/空列表的特判);安全面大(api_key 入库、master_key)。

### 3.2 cc-switch(配置文件/预设驱动 provider 切换)

- **形态**:TS 预设表(`claudeProviderPresets.ts` 等),每条预设含 `settingsConfig.env`(注入到目标应用的 env var)、`apiFormat`(`anthropic`/`openai_chat`/`openai_responses`/`gemini_native`)、`providerType`(`github_copilot`/`codex_oauth`/`xai_oauth` 特殊认证)([claudeProviderPresets.ts:25-74](../reference/cc-switch/src/config/claudeProviderPresets.ts#L25-L74))。
- **关键设计**:`apiFormat` 即**协议类型**字段——cc-switch 本身**不实现协议**,只给 Claude Code/Codex/Gemini 客户端喂 env var,由这些客户端各自处理协议转换。Universal Provider 预设(如 NewAPI)跨三应用同步配置([universalProviderPresets.ts:61-76](../reference/cc-switch/src/config/universalProviderPresets.ts#L61-L76))。
- **优点**:协议类型作为数据字段清晰;预设可被用户覆盖/扩展。
- **缺点**:依赖宿主客户端实现协议——这正是 aimux 与之的根本差异(aimux 自己用 Rust 实现协议)。

### 3.3 one-api / new-api(数据库存 provider 配置)

- **形态**:provider 配置(channel)存数据库,运行时管理界面增删改查;支持多协议(OpenAI/Anthropic/Gemini…)通过 relay 层转换。
- **优点**:完全动态、运营友好。
- **缺点**:是**网关产品**而非库——自带 HTTP 服务、计费、用户体系。aimux 是接入层库,不承担此定位(RFC-0019 §1.4 已明确"不做渠道路由/账号池")。

### 3.4 Vercel AI SDK(纯编程式)

- **形态**:`createOpenAI({ baseURL, apiKey, headers, ... })` / `createOpenAICompatible(...)` 工厂,纯代码传配置,无配置文件。`transformRequestBody` 闭包做请求体变换。
- **优点**:类型安全、无解析面、无文件 IO 风险。
- **缺点**:无声明式配置;无"name 复用"注册表(每次构造)。
- aimux 的 `OpenAIProvider::new(OpenAIConfig::new(...))` + `ProviderConfig` 已对齐此形态(RFC-0017 §1.2 已论证为何不引入闭包桥接,改用 `bodyOverrides` JSON merge)。

### 3.5 对比小结

| 维度 | litellm | cc-switch | one/new-api | Vercel AISDK | **aimux 现状** | **aimux 适合** |
|---|---|---|---|---|---|---|
| 配置文件 | ✅ yaml | ✅ 预设 TS/JSON | ❌ DB | ❌ | ❌ | ✅ 做 |
| 编程式注册 | ✅ | ✅(喂 env) | ✅ | ✅ | 🔶(基础类,无注册表) | ✅ 做 |
| 动态增删 | ✅ DB | 🔶(切预设) | ✅ | ❌ | ❌ | ⚠️(增删做,发现不做) |
| 协议类型字段 | 🔶(model 前缀) | ✅ apiFormat | ✅ | ❌ | ❌ | 🔶(仅 openai_compat 可数据化,见 §4.4) |
| 定位 | 网关 | 配置切换器 | 网关 | SDK | **接入层库** | — |

---

## 4. aimux 做这个的契合度

### 4.1 registry 是编译期嵌入的 JSON,如何支持运行时注入?

**完全可行,且改动集中。** 现状 `REGISTRY: OnceLock<Vec<RegistryEntry>>` 不可变([provider.rs:61](../aimux-providers/src/provider.rs#L61))。注入方案:

- 新增一个**运行时覆盖层** `OVERLAYS: RwLock<HashMap<String, RegistryEntry>>`(或 `OnceLock<...>` 包 `RwLock`)。
- `provider()` 查找顺序改为:**覆盖层 → 内置 registry**;找到条目后,后端组装管线(`OpenAIConfig` 构造 + `ProviderOptions` 叠加)完全复用,零改动。
- 内置 registry 仍是 `include_str!` 编译期嵌入(保留 RFC-0017 的"单一数据源 + 类型派生"不变);覆盖层只承载外部新增/覆盖条目。
- **这是纯增量改动**:不动 `provider_registry.json`、不动 `gen_provider_names.py`、不动 `ProviderName` 派生(外部名字不属于编译期枚举,走字符串路径即可)。

### 4.2 Provider / LanguageModel trait 是否 object-safe(能否动态注册)?

**是,且更优——aimux 根本不需要 `dyn Provider` 注册表。**

- `Provider` trait([provider.rs:9-14](../aimux-core/src/provider.rs#L9-L14))object-safe(全 `&self` 方法,返回 `&str` / `Box<dyn LanguageModel>`,无泛型/Self)。但全仓库**从不存储 `dyn Provider`**(`grep "dyn Provider"` 零命中)——`Provider` 仅作工厂,`language_model()` 立即产出 `Box<dyn LanguageModel>`。
- `LanguageModel` trait([language_model.rs:25-42](../aimux-core/src/language_model.rs#L25-L42))经 `#[async_trait]` 去糖为返回 `Pin<Box<dyn Future>>`,object-safe;**已是全链路通用货币**(`Box<dyn LanguageModel>` / `Arc<dyn LanguageModel>`,见 [lib.rs:31](../bindings/node/src/lib.rs#L31)、[lib.rs:767](../aimux-ffi/src/lib.rs#L767))。
- **推论**:动态注册一个外部 provider = 把它的配置数据塞进覆盖层;`provider()` 查到后照常 `OpenAIProvider::new(config).language_model(id)` → `Box<dyn LanguageModel>`。**无需任何 trait object 注册表,无需新抽象。** 这是 aimux 架构对此需求的天然友好点。

### 4.3 配置 schema 要不要包含 protocol type?

**需要,但只能取一个值:`openai_compat`(且为默认)。** 这是关键约束,理由:

- aimux 的薄封装 registry **只服务 OpenAI 兼容协议**——`provider()` 恒构造 `OpenAIProvider`([provider.rs:167](../aimux-providers/src/provider.rs#L167))。
- 原生协议(anthropic/google/bedrock/vertex/azure/cohere/mistral)是**独立代码实现**(各有 convert/model/stream 模块,[lib.rs:15-24](../aimux-providers/src/lib.rs#L15-L24)),**无法用配置数据描述**——这与 RFC-0017 §2.6 的分层定论一致("协议不兼容的走核心代码实现,协议兼容且差异可数据化的一律走 JSON")。
- 对比 cc-switch 的 `apiFormat` 四值——cc-switch 不实现协议(靠宿主客户端),所以能把协议类型当数据;aimux 自己实现协议,新协议=新代码,配置无法承载。
- **因此**:外部配置能注册"内置 registry 没有的新 provider"——**但仅限 OpenAI 兼容的那些**。要注册一个全新的原生协议 provider,只能走代码(实现 trait + 发 PR),无法靠配置。

schema 里仍**建议显式带 `protocol` 字段**(默认 `openai_compat`),为未来留扩展点(若某天加 plugin/code 扩展机制,可新增取值);当前非 `openai_compat` 直接报清晰错误。

### 4.4 与 RFC-0017 bodyOverrides 的关系

- `body_overrides`(provider 级 + per-call,JSON deep merge)解决的是**请求体字段差异**(关思考、字段重命名等),[0017-provider-config-dx.md:63-136](../rfc/0017-provider-config-dx.md#L63-L136)。
- 本调研的"外部 provider 配置"解决的是**provider 身份/连接/profile 差异**(name/base_url/env_var/profile)——是比 bodyOverrides **更外层**的配置:先确定"连哪个 provider",再决定"请求体怎么覆盖"。
- 二者正交且互补:外部配置条目可自带 `body_overrides`(provider 级,等价于把 §2.5 示例 2/3 的 relay/Qwen 配置声明式化),per-call `body_overrides` 仍在其后 merge 覆盖。**无冲突**。
- RFC-0017 阶段 4 已预留"用户覆盖内置条目"语义([0017-provider-config-dx.md:363](../rfc/0017-provider-config-dx.md#L363):"同名条目——用户 JSON 替换/merge 内置条目,仅在'查条目'一步生效")——**本调研即该语义的具体落地**,是对 RFC-0017 的延续而非偏离。

### 4.5 与"零内置厂商映射"原则(RFC-0017 v3)的关系

RFC-0017 v3 退役了所有内置厂商思考映射,改为用户 `bodyOverrides` 定义([0017-provider-config-dx.md:140-149](../rfc/0017-provider-config-dx.md#L140-L149))。外部配置**强化**而非削弱此原则:
- 外部 provider 条目可自带 `body_overrides`,把"某 relay 的专属字段""某厂商关思考"**声明式写进配置**,而非散落在调用点代码——更符合"知识放配置/文档,机制放代码"。
- profile 字段(`max_tokens_key` 等)是**机制数据**(aimux 内部推断用),非厂商映射知识,放入配置不违反原则。

---

## 5. 成本/收益与风险

### 5.1 实现难度

| 部分 | 难度 | 工作量估计 | 说明 |
|---|---|---|---|
| 运行时覆盖层(Rust core) | 低 | ~150 行 | `RwLock<HashMap>` + `provider()` 查找顺序调整 + 注册/加载 API |
| 配置文件解析(YAML/TOML/JSON) | 低 | ~100 行 | 复用 `RegistryEntry` schema;JSON 最省依赖(YAML 需加 `serde_yaml`/`serde_yml`) |
| 编程式注册 API(Rust) | 低 | ~50 行 | `register_provider(entry)` / `load_config_file(path)` |
| 绑定层透传(8 语言) | 低 | 每语言 ~30 行 | RFC-0017 阶段 4 已验证分发模式;Node `provider()` 加 config-file 参数或新增 `registerProvider` |
| profile 字段透出 | 低 | ~50 行 | 绑定 `ProviderConfig` 加 `profile` 子对象 |
| **合计(最小可行)** | **低** | **~200-400 行 Rust + 绑定薄改** | 无新抽象、无 trait 改动、无破坏性变更 |

对比"动态注册原生协议 provider"(需 plugin/trait 注册系统):**高难度、高复杂度、低边际价值**(原生协议稳定且仅个位数,已代码实现),**不建议纳入**。

### 5.2 收益

1. **"新增/修正 provider 从库行为变用户行为"**(RFC-0017 阶段 4 目标 [0017-provider-config-dx.md:345](../rfc/0017-provider-config-dx.md#L345))的**最后一块拼图**——目前改内置 provider 仍需改 JSON + 重新编译发版;外部配置让 relay/私有网关/新厂商接入**零编译、零发版**。
2. **宿主应用集成友好**:网关/IDE/agent 宿主可编程式注册其私有 provider,不必 fork aimux。
3. **声明式配置**与 litellm/cc-switch 生态对齐,降低迁移门槛;配置文件可版本管理、团队共享。
4. 覆盖内置条目 = 修 base_url 错误等不必等发版(配合 RFC-0017 阶段 4 已修的 7 处 base_url)。

### 5.3 风险与对策

| 风险 | 等级 | 对策 |
|---|---|---|
| **API key 注入到配置文件**(明文泄露) | 高 | (1) schema 支持 `api_key: "env:VAR_NAME"` 引用(默认从 env 读,与内置 `env_var` 一致);(2) 文档强警示;(3) 不把 key 写日志(RFC-0015 审计已关注) |
| **SSRF via base_url**(内网探测) | 中 | (1) `base_url` scheme 校验(默认仅 `http`/`https`);(2) 可选 allowlist/blocklist(宿主侧);(3) 文档警示"外部配置 = 信任边界" |
| **覆盖层与内置条目 merge 语义歧义** | 中 | 明确定义:外部条目**整体替换**内置同名条目(非 deep merge,简单可预期);`ProviderOptions` 仍在条目之上 per-call 覆盖(已有) |
| **配置文件解析错误延后到运行时** | 低 | 加载时即校验(schema:必填 name/base_url、profile 字段范围);校验失败返回明确错误(非 panic) |
| **profile 的 `&'static str` 字段**(`stream_usage_key`/`max_tokens_key`) | 低 | `profile_from_registry` 已用 `Box::leak` 处理运行时字符串([provider.rs:180-187](../aimux-providers/src/provider.rs#L180-L187));覆盖层条目复用同路径,无新问题 |
| **并发注册竞争** | 低 | `RwLock` 保护;注册应在启动期完成,运行期只读查(动态增删可选,文档建议启动期注册) |
| **对"零内置厂商映射"原则的冲击** | 低 | 无冲击——profile 是机制数据,厂商映射仍由用户 `body_overrides` 定义(§4.5) |

---

## 6. 推荐的最小可行形态

### 6.1 范围(MVP)

- ✅ 配置文件(JSON 起步,YAML 可选)加载 OpenAI 兼容 provider
- ✅ 编程式注册 API(`register_provider` / `load_providers_from_file`)
- ✅ 覆盖/扩展内置 registry(同名替换、新名新增)
- ✅ profile 字段在配置 schema 中可声明
- ✅ api_key 支持 `env:VAR` 引用(默认)与明文(不推荐)
- ❌ 原生协议动态注册(远期,需 plugin 机制)
- ❌ 动态发现/网关路由(超出定位)

### 6.2 推荐配置 schema(JSON;YAML 等价)

```json
{
  "$schema": "https://aimux.dev/schema/providers.v1.json",
  "providers": [
    {
      "name": "my-relay",
      "display": "My Team Relay",
      "base_url": "https://relay.internal.team/v1",
      "env_var": "MY_RELAY_API_KEY",
      "api_key": "env:MY_RELAY_API_KEY",
      "protocol": "openai_compat",
      "profile": {
        "supports_top_k": true,
        "supports_tools": true,
        "supports_response_format": true,
        "stream_usage_key": null,
        "max_tokens_key": null
      },
      "headers": { "X-Team": "platform" },
      "body_overrides": { "enable_thinking": false },
      "max_retries": 2
    },
    {
      "name": "groq",
      "base_url": "https://api.groq.com/openai/v1",
      "env_var": "GROQ_API_KEY",
      "protocol": "openai_compat",
      "comment": "覆盖内置 groq 条目的 base_url"
    }
  ]
}
```

**schema 设计要点**:
- 复用 `RegistryEntry` 字段(name/display/base_url/env_var/profile)+ 已有 `ProviderOptions` 字段(headers/body_overrides/max_retries/organization/project)→ **不发明新概念**,与 [provider.rs:34-110](../aimux-providers/src/provider.rs#L34-L110) 同构。
- `protocol` 字段:默认 `openai_compat`;其他值当前报错(为扩展预留)。
- `api_key`:`"env:VAR"` 引用优先(安全默认,与内置 `env_var` 语义一致);明文字符串技术支持但文档不推荐。
- `profile`:全可选,缺省 `full()`(与内置 `profile: {}` 等价)。
- `comment`:用户备注,库忽略(配置文件可读性)。

### 6.3 推荐的 Rust API(MVP)

```rust
// aimux-providers/src/provider.rs 增量

/// 外部 provider 配置条目(与 RegistryEntry + ProviderOptions 同构,可 Deserialize)。
#[derive(Debug, Clone, Deserialize)]
pub struct ExternalProviderEntry {
    pub name: String,
    pub display: Option<String>,
    pub base_url: String,
    pub env_var: Option<String>,
    pub api_key: Option<String>,        // "env:VAR" 或明文
    #[serde(default = "default_openai_compat")]
    pub protocol: String,               // 当前仅 "openai_compat"
    #[serde(default)]
    pub profile: RegistryProfile,
    pub headers: Option<HashMap<String, String>>,
    pub organization: Option<String>,
    pub project: Option<String>,
    pub max_retries: Option<u32>,
    pub body_overrides: Option<Value>,
}

/// 注册一个外部 provider(覆盖同名内置条目,或新增)。
pub fn register_provider(entry: ExternalProviderEntry) -> Result<(), AiMuxError> { /* 校验 + 写 OVERLAYS */ }

/// 从配置文件加载并注册多个 provider。
pub fn load_providers_from_file(path: &Path) -> Result<(), AiMuxError> { /* parse + register each */ }

/// 从 JSON 字符串加载(供绑定层透传)。
pub fn load_providers_from_json(json: &str) -> Result<(), AiMuxError> { /* ... */ }
```

`provider()` 查找改为:`OVERLAYS.read().get(name)` → 命中则用外部条目组装 `OpenAIConfig`(含 profile/headers/body_overrides);未命中走内置 `registry()`。后端 `OpenAIProvider::new(config).language_model(id)` 不变。

### 6.4 绑定层(Node 示例)

```ts
// 新增:编程式注册(透传到 Rust load_providers_from_json)
export function registerProviders(configJson: string): Promise<void>

// 或:provider() 工厂接受额外配置文件路径/对象
// (保持向后兼容:新参数可选)
```

C ABI:`aimux_register_providers(json: *const c_char) -> *mut c_char`,各 FFI 语言复用。

### 6.5 校验与安全(MVP 必须)

- 加载即校验:`name`/`base_url` 非空;`base_url` 是合法 `http(s)` URL;scheme 白名单;`protocol` 仅 `openai_compat`。
- `api_key` 解析:`env:VAR` → 读 env,失败报清晰错误;明文 → 警告日志。
- 校验失败 → 返回 `AiMuxError`,不 panic(与内置 registry 的 `panic` 不同——外部配置是用户输入,不可 panic)。

---

## 7. 实施建议(分阶段)

| 阶段 | 内容 | 风险 | 依赖 |
|---|---|---|---|
| **P1** | Rust core:覆盖层 + `register_provider` + `load_providers_from_json` + `provider()` 查找调整 + 校验 + 测试 | 低 | 无 |
| **P2** | 绑定层透传(Node/C ABI/Python/Go…)+ profile 字段透出 `ProviderConfig` | 低 | P1 |
| **P3**(可选) | 配置文件加载(`load_providers_from_file`,YAML 支持)+ 文档 | 低 | P1 |
| **P4**(远期,单独提案) | 原生协议动态注册(plugin/code 扩展机制) | 高 | 需新 RFC |

**建议先做 P1+P2**:覆盖 90% 真实需求(外部 relay/私有网关/新 OpenAI 兼容厂商),成本最低,与现有架构无缝衔接。P3 视用户反馈;P4 暂不做。

---

## 8. 证据索引(关键 file:line)

| 结论 | 证据 |
|---|---|
| registry 编译期嵌入、不可变 | [provider.rs:61](../aimux-providers/src/provider.rs#L61) `OnceLock`、[provider.rs:66](../aimux-providers/src/provider.rs#L66) `include_str!` |
| 251 条、name→数据 查找 | [provider.rs:126](../aimux-providers/src/provider.rs#L126)、[provider.rs:292](../aimux-providers/src/provider.rs#L292) |
| `provider()` 恒构造 OpenAIProvider | [provider.rs:167](../aimux-providers/src/provider.rs#L167) |
| `ProviderOptions` 已支持 per-call 覆盖(不含 profile) | [provider.rs:96-110](../aimux-providers/src/provider.rs#L96-L110)、[provider.rs:143-165](../aimux-providers/src/provider.rs#L143-L165) |
| `Provider` trait object-safe 但从不存 `dyn Provider` | [provider.rs:9-14](../aimux-core/src/provider.rs#L9-L14);`grep "dyn Provider"` 零命中 |
| `LanguageModel` object-safe、全链路通用货币 | [language_model.rs:25-42](../aimux-core/src/language_model.rs#L25-L42);[lib.rs:31](../bindings/node/src/lib.rs#L31)、[lib.rs:767](../aimux-ffi/src/lib.rs#L767) |
| profile 的 `&'static str` 已用 `Box::leak` 处理运行时串 | [provider.rs:180-187](../aimux-providers/src/provider.rs#L180-L187) |
| 绑定层不暴露 profile | [lib.rs:218-236](../bindings/node/src/lib.rs#L218-L236) |
| RFC-0017 已预留"用户覆盖内置条目"语义 | [0017-provider-config-dx.md:363](../rfc/0017-provider-config-dx.md#L363) |
| RFC-0017 §2.6 分层:原生协议=代码,OpenAI 兼容=JSON | [0017-provider-config-dx.md:230-242](../rfc/0017-provider-config-dx.md#L230-L242) |
| 原生协议是独立代码实现 | [lib.rs:15-24](../aimux-providers/src/lib.rs#L15-L24) |
| aimux 定位=接入层,不做网关路由 | [0019-session-affinity.md:41-44](../rfc/0019-session-affinity.md#L41-L44) |
| litellm config.yaml model_list 形态 | [docs.litellm.ai/docs/proxy/configs](https://docs.litellm.ai/docs/proxy/configs) |
| litellm 运行时 DB 增删、不替换 YAML | [docs.litellm.ai/docs/proxy/model_management](https://docs.litellm.ai/docs/proxy/model_management) |
| cc-switch apiFormat 协议类型字段 | [claudeProviderPresets.ts:49-58](../reference/cc-switch/src/config/claudeProviderPresets.ts#L49-L58) |
| cc-switch 不实现协议(喂 env var) | [claudeProviderPresets.ts:98-107](../reference/cc-switch/src/config/claudeProviderPresets.ts#L98-L107) |

---

## 9. 剩余不确定性

1. **配置文件格式选型**:JSON(零依赖,与 registry 同构)vs YAML(用户更友好,但加 `serde_yml` 依赖)。建议 MVP 用 JSON,P3 再加 YAML——需用户反馈确认偏好。
2. **覆盖语义:替换 vs deep-merge**:本报告推荐"整体替换"(简单可预期),但 litellm 用 deep-merge。若用户期望"只改 base_url、其余继承内置条目",需改为 merge——建议先替换,按反馈调整。
3. **是否需要"卸载/列出已注册外部 provider"的运维 API**:MVP 可不做(进程生命周期内注册即固定),但宿主应用管理界面可能需要——P2 后视需求加。
4. **`api_key: "env:VAR"` 的引用语法**是否与现有 `load_api_key(None, env_var, ...)`([provider.rs:135](../aimux-providers/src/provider.rs#L135))完全对齐,还是有独立解析路径——实现时需统一,避免两套 env 读取逻辑。
5. **原生协议动态注册**的真实需求强度:当前无证据表明用户需要"运行时注册全新原生协议 provider"(原生协议稳定且少);若有强需求,需另立 plugin/extension RFC——本报告不预设。
