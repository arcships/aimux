# RFC-0020: 外部 Provider 配置

> **Status**: DRAFT (pending review)
> **Date**: 2026-08-05
> **Scope**: `aimux-providers` 新增运行时覆盖层,允许外部(配置文件 / 编程式 API)注册、覆盖 OpenAI 兼容 provider 条目;各 binding 薄透传
> **Related**: [RFC-0017](0017-provider-config-dx.md) 配置 DX(本 RFC 落地其 §363 预留的"用户覆盖内置条目"语义)、[RFC-0019](0019-session-affinity.md) 会话亲和、[调研报告](../docs/external-provider-config-research.md)

---

## 1. Motivation

aimux 当前有 251 个 OpenAI 兼容 provider 通过 `provider_registry.json` 编译期嵌入(`include_str!` + `OnceLock`),初始化后不可变。新增或修正一个 provider 需要改 JSON + 重新编译发版。

三个真实痛点:

1. **私有 relay / 内网网关无法接入**:企业自建 OpenAI 兼容网关(base_url 指向内网)无法被 `provider("my-relay", ...)` 查到,只能用 `OpenAIProvider::new(OpenAIConfig::new(...))` 逐次手工构造,无"name → 后续复用"机制。
2. **内置条目修正需等发版**:RFC-0017 阶段 4 修了 7 处 base_url 错误,但仍有 ~20 处存疑(novita/nous_research/longcat 等)待实测——外部覆盖能让用户不等发版自行修正。
3. **宿主应用集成不友好**:网关 / IDE / agent 宿主想注册其私有 provider,目前必须 fork aimux 改 JSON。

RFC-0017 阶段 4 已预留语义([0017-provider-config-dx.md:363](0017-provider-config-dx.md#L363):"同名条目——用户 JSON 替换/merge 内置条目,仅在'查条目'一步生效")。本 RFC 是该语义的具体落地。

---

## 2. Design Goals

1. **零破坏性**:不动 `provider_registry.json`、不动 `gen_provider_names.py`、不动 `ProviderName` 派生、不动 `provider()` 签名(新参数全 `Option`)。
2. **后端管线零改动**:外部配置只影响"查条目"一步;查到后 `OpenAIConfig` 构造 + `ProviderOptions` 叠加 + `OpenAIProvider::new(config).language_model(id)` 完全复用。
3. **8 语言同步受益**:Rust core 一处落地,各 binding 薄透传(RFC-0017 阶段 4 已验证此分发模式)。
4. **仅 OpenAI 兼容协议**:原生协议(anthropic/google/bedrock…)是代码实现,无法用配置数据描述(RFC-0017 §2.6 分层定论)。
5. **安全默认**:api_key 优先 env 引用;base_url scheme 校验;外部配置是用户输入,校验失败返回错误不 panic。

---

## 3. Design

### 3.1 运行时覆盖层

在 `aimux-providers/src/provider.rs` 新增覆盖层,`provider()` 查找顺序改为 **覆盖层 → 内置 registry**:

```rust
use std::sync::RwLock;
use once_cell::sync::Lazy;

static OVERLAYS: Lazy<RwLock<HashMap<String, ExternalProviderEntry>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

// provider() 查找改为:
// 1. OVERLAYS.read().get(name) → 命中则用外部条目组装 OpenAIConfig
// 2. registry().iter().find(|e| e.name == name) → 走内置路径(不变)
// 3. 未命中 → NoSuchProvider
```

内置 registry 仍是 `include_str!` 编译期嵌入(`OnceLock<Vec<RegistryEntry>>` 不变);覆盖层只承载外部新增/覆盖条目。外部名字不属于编译期 `ProviderName` 枚举,走字符串路径——`provider()` 本就接受 `&str`,无需改签名。

### 3.2 配置条目 schema

```rust
/// 外部 provider 配置条目。与 RegistryEntry + ProviderOptions 同构,可 Deserialize。
#[derive(Debug, Clone, Deserialize)]
pub struct ExternalProviderEntry {
    /// provider 名(用于 provider("name", ...) 查找)。必填。
    pub name: String,
    /// 显示名。可选,缺省用 name。
    pub display: Option<String>,
    /// API base URL。必填。必须是合法 http(s) URL。
    pub base_url: String,
    /// 环境变量名(用于读 api_key)。可选。
    pub env_var: Option<String>,
    /// api_key:"env:VAR_NAME" 引用(安全默认)或明文字符串(不推荐)。可选。
    pub api_key: Option<String>,
    /// 协议类型。当前仅 "openai_compat"(默认)。其他值报错。
    #[serde(default = "default_openai_compat")]
    pub protocol: String,
    /// provider 能力差异 profile。全可选,缺省 full()。
    #[serde(default)]
    pub profile: RegistryProfile,
    // --- 以下等价于 ProviderOptions 的字段 ---
    pub headers: Option<HashMap<String, String>>,
    pub organization: Option<String>,
    pub project: Option<String>,
    pub max_retries: Option<u32>,
    pub body_overrides: Option<serde_json::Value>,
    /// 用户备注,库忽略。
    pub comment: Option<String>,
}

fn default_openai_compat() -> String { "openai_compat".to_string() }
```

**schema 设计要点**:
- 复用 `RegistryEntry` 字段(name/display/base_url/env_var/profile)+ `ProviderOptions` 字段(headers/body_overrides/max_retries/organization/project)→ 不发明新概念。
- `protocol` 字段默认 `openai_compat`;非此值当前报清晰错误(为未来 plugin 扩展预留)。
- `api_key`:`"env:VAR"` 引用优先(与内置 `env_var` 语义一致);明文技术支持但文档不推荐。
- `profile`:`RegistryProfile` 已存在([provider.rs:45-55](../aimux-providers/src/provider.rs#L45)),全可选,缺省 `full()`。

### 3.3 JSON 配置文件格式

```json
{
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
      "comment": "覆盖内置 groq 条目的 base_url"
    }
  ]
}
```

### 3.4 Rust API

```rust
// aimux-providers/src/provider.rs 增量

/// 注册一个外部 provider(覆盖同名内置条目,或新增)。
/// 校验失败返回 AiMuxError,不 panic(区别于内置 registry 的 panic)。
pub fn register_provider(entry: ExternalProviderEntry) -> Result<(), AiMuxError> {
    validate_entry(&entry)?;           // name/base_url 非空、scheme 校验、protocol 校验
    let mut overlays = OVERLAYS.write().unwrap();
    overlays.insert(entry.name.clone(), entry);
    Ok(())
}

/// 从 JSON 字符串加载并注册多个 provider(供 binding 层透传)。
pub fn load_providers_from_json(json: &str) -> Result<(), AiMuxError> {
    let config: ProvidersConfig = serde_json::from_str(json)?;
    for entry in config.providers {
        register_provider(entry)?;
    }
    Ok(())
}

/// 从配置文件加载并注册。
pub fn load_providers_from_file(path: &std::path::Path) -> Result<(), AiMuxError> {
    let content = std::fs::read_to_string(path)?;
    load_providers_from_json(&content)
}

#[derive(Deserialize)]
struct ProvidersConfig {
    providers: Vec<ExternalProviderEntry>,
}
```

### 3.5 覆盖语义

外部条目**整体替换**内置同名条目(非 deep merge,简单可预期)。理由:
- 简单可预期:用户写什么就是什么,无隐式继承。
- litellm 用 deep-merge 但其 schema 庞大且 null/空列表特判复杂,aimux 不需要。
- 若用户只想"改 base_url 其余继承",可从内置 registry 拷贝条目改后注册(文档示例说明)。

`ProviderOptions`(per-call)仍在条目之上覆盖,已有机制不变。

---

## 4. Integration Approach

### 4.1 provider() 查找路径调整

```rust
pub fn provider(
    name: &str,
    api_key: Option<String>,
    model_id: &str,
    options: Option<ProviderOptions>,
) -> Result<Box<dyn LanguageModel>, AiMuxError> {
    // 1. 覆盖层优先
    if let Some(entry) = OVERLAYS.read().unwrap().get(name) {
        return build_from_external_entry(entry, api_key, model_id, options);
    }
    // 2. 内置 registry(原逻辑不变)
    let entry = registry().iter().find(|e| e.name == name)
        .ok_or_else(|| AiMuxError::NoSuchProvider(...))?;
    build_from_registry_entry(entry, api_key, model_id, options)
}

fn build_from_external_entry(
    entry: &ExternalProviderEntry,
    api_key: Option<String>,
    model_id: &str,
    options: Option<ProviderOptions>,
) -> Result<Box<dyn LanguageModel>, AiMuxError> {
    // 解析 api_key:"env:VAR" → 读 env;明文 → 直接用;None → 读 entry.env_var
    let key = resolve_api_key(&api_key, &entry.api_key, &entry.env_var)?;
    let mut config = OpenAIConfig::new(key)
        .with_base_url(&entry.base_url)
        .with_provider(&entry.name)
        .with_profile(entry.profile.clone());
    // 叠加 entry 自带的 provider 级配置
    if let Some(h) = &entry.headers { config = config.with_headers(h.clone()); }
    if let Some(o) = &entry.organization { config = config.with_org_id(o.clone()); }
    if let Some(p) = &entry.project { config = config.with_project(p.clone()); }
    if let Some(r) = entry.max_retries { config = config.with_retry_config(RetryConfig { max_retries: r, ..Default::default() }); }
    if let Some(b) = &entry.body_overrides { config = config.with_body_overrides(b.clone()); }
    // 叠加 per-call ProviderOptions(已有逻辑)
    if let Some(opts) = options { config = apply_provider_options(config, opts); }
    Ok(OpenAIProvider::new(config).language_model(model_id)?)
}
```

### 4.2 绑定层透传

**Node**:
```ts
// 新增:编程式注册(透传到 Rust load_providers_from_json)
export function registerProviders(configJson: string): Promise<void>
```

**C ABI**:
```c
// 新增:注册外部 provider(JSON 字符串)
const char* aimux_register_providers(const char* config_json);
```

各 FFI 语言(Go/Java/Kotlin/Swift/Flutter/Python)复用同一 C ABI 入口。Python(pyo3 native)直接调 Rust 函数。

**profile 透出**:各 binding 的 `ProviderConfig` 加可选 `profile` 子对象(Node `ProviderConfig` 当前缺此字段,[lib.rs:218-236](../bindings/node/src/lib.rs#L218))。

### 4.3 校验与安全

加载即校验(校验失败返回 `AiMuxError`,不 panic):
- `name` / `base_url` 非空
- `base_url` 是合法 `http(s)` URL(scheme 白名单,防 SSRF)
- `protocol` 仅 `openai_compat`
- `api_key` 解析:`env:VAR` → 读 env,失败报清晰错误;明文 → warn 日志

---

## 5. Relationship with Existing RFCs

| RFC | 关系 |
|-----|------|
| [RFC-0017](0017-provider-config-dx.md) | **延续**。本 RFC 落地 §363 预留的"用户覆盖内置条目"语义;与 `body_overrides`(per-call + provider 级)正交互补——外部配置条目可自带 `body_overrides`(声明式化),per-call `body_overrides` 仍在其后 merge 覆盖。强化"零内置厂商映射"原则:厂商专属字段声明式写进配置而非散落调用点。 |
| [RFC-0019](0019-session-affinity.md) | **正交**。会话亲和靠 `CallOptions.headers` 透传;外部配置条目可自带 `headers`(含会话亲和头),是更外层的"provider 身份"配置。 |
| [RFC-0009](0009-request-resilience.md) | **正交**。外部配置条目的 `max_retries` 透传到 `RetryConfig`,复用已有 retry 机制。 |

---

## 6. Non-Goals

1. **不动态注册原生协议 provider**(anthropic/google/bedrock/vertex/azure/cohere/mistral)。这些是代码实现(各有独立 convert/model/stream 模块),无法用配置数据描述。若未来有强需求,另立 plugin/extension RFC。
2. **不做动态发现 / 网关路由**。增删 provider 可行(覆盖层可变),但"动态发现"(运行时探测可用 provider)属网关职责,aimux 定位是接入层不做(RFC-0019 §1.4)。
3. **不做 deep-merge 覆盖语义**。整体替换,简单可预期。
4. **不做配置热重载 / 文件监听**。MVP 是启动期加载注册,进程生命周期内固定。
5. **不改 `ProviderName` 枚举**。外部名字走字符串路径,不属于编译期派生。

---

## 7. Scope of Changes

| 位置 | 改动 | 工作量 |
|------|------|--------|
| `aimux-providers/src/provider.rs` | 新增 `OVERLAYS` 覆盖层 + `ExternalProviderEntry` + `register_provider` / `load_providers_from_json` / `load_providers_from_file` + `provider()` 查找调整 + 校验 | ~200 行 |
| `bindings/node/src/lib.rs` | 新增 `registerProviders` 函数 + `ProviderConfig` 加 `profile` 字段 | ~30 行 |
| `aimux-ffi/src/lib.rs` | 新增 `aimux_register_providers` C ABI | ~20 行 |
| `bindings/{python,go,java,kotlin,swift,flutter}` | 各自薄透传 `register_providers` | 每语言 ~20 行 |
| `docs/provider-config-manual.md` | 外部配置章节 + 示例 | 文档 |

**合计:~200-400 行 Rust + 绑定薄改。无新抽象、无 trait 改动、无破坏性变更。**

---

## 8. Risks

| 风险 | 等级 | 对策 |
|------|------|------|
| **API key 明文泄露到配置文件** | 高 | schema 支持 `api_key: "env:VAR"` 引用(默认);文档强警示;不把 key 写日志 |
| **SSRF via base_url**(内网探测) | 中 | `base_url` scheme 校验(默认仅 `http`/`https`);可选 allowlist;文档警示"外部配置 = 信任边界" |
| **覆盖层与内置条目 merge 语义歧义** | 中 | 明确整体替换(非 deep merge);`ProviderOptions` 仍在条目之上 per-call 覆盖 |
| **配置解析错误延后到运行时** | 低 | 加载即校验;失败返回 `AiMuxError` 不 panic |
| **profile 的 `&'static str` 字段** | 低 | `profile_from_registry` 已用 `Box::leak` 处理运行时字符串([provider.rs:180-187](../aimux-providers/src/provider.rs#L180));覆盖层复用同路径 |
| **并发注册竞争** | 低 | `RwLock` 保护;建议启动期注册,运行期只读查 |
| **对"零内置厂商映射"原则的冲击** | 低 | 无冲击——profile 是机制数据,厂商映射仍由用户 `body_overrides` 定义 |

---

## 9. Open Questions

1. ~~配置文件格式 JSON vs YAML?~~ **MVP 用 JSON**(零依赖,与 registry 同构)。YAML 需加 `serde_yml` 依赖,视用户反馈在 P3 加。
2. ~~覆盖语义替换 vs deep-merge?~~ **整体替换**(简单可预期)。若用户反馈需要"只改 base_url 其余继承",再评估 merge。
3. **是否需要"卸载/列出已注册外部 provider"的运维 API?** MVP 可不做(进程内固定),宿主管理界面需要时再加 `unregister_provider` / `list_external_providers`。
4. **`api_key: "env:VAR"` 引用语法**是否与现有 `load_api_key(None, env_var, ...)`([provider.rs:135](../aimux-providers/src/provider.rs#L135))完全对齐?实现时需统一,避免两套 env 读取逻辑。

---

## 10. Implementation Order

| 阶段 | 内容 | 依赖 | 状态 |
|------|------|------|------|
| **P1** | Rust core:覆盖层 + `register_provider` + `load_providers_from_json` + `provider()` 查找调整 + 校验 + 测试 | 无 | 待实施 |
| **P2** | 绑定层透传(Node/C ABI/Python/Go/…)+ profile 字段透出 `ProviderConfig` | P1 | 待实施 |
| **P3**(可选) | 配置文件加载(`load_providers_from_file`)+ 文档 + YAML 支持 | P1 | 待实施 |
| **P4**(远期,单独提案) | 原生协议动态注册(plugin/code 扩展机制) | 需新 RFC | 不做 |

**建议先做 P1+P2**:覆盖 90% 真实需求(外部 relay/私有网关/新 OpenAI 兼容厂商/修正内置 base_url),成本最低。
