# Codex 订阅通道集成指南（RFC-0018）

> **适用版本**: aimux 0.x（2026-08-03，[RFC-0018](../rfc/0018-codex-subscription.md)）
> **核心分工**: **OAuth 由集成方做，aimux 库只做协议面**。
> 库不执行设备码登录、不持久化 token、不自动刷新——它提供订阅模式的请求协议、
> 一个无状态的 `codex_refresh` 纯函数，以及 401 → `AiMuxError::TokenExpired` 的类型化错误。

---

## 1. 两种模式怎么选

| | API key 模式（Path A） | 订阅模式（Path B） |
|---|---|---|
| 端点 | `api.openai.com/v1/responses` | `chatgpt.com/backend-api/codex/responses` |
| 凭据 | API key | ChatGPT 订阅账号的 OAuth access token |
| 状态 | 官方、有文档、支持 CI/自动化 | **无文档、无 SLA、best-effort** |
| 适用 | 生产/自动化（推荐） | 个人订阅账号自用 |
| ToS 边界 | 正常使用 | **单账号自用**；账号池/多账号轮换/转售违反 OpenAI ToU |

**默认用 API key 模式**。订阅模式仅当你确实持有 ChatGPT 订阅且愿意承担端点漂移风险时使用。

## 2. API key 模式（Rust）

```rust
use aimux_providers::{CodexMode, CodexProviderSettings, codex, create_codex};

// 显式 key（`Some("")` 原样发送，不回退环境变量）：
let provider = create_codex(CodexProviderSettings {
    mode: CodexMode::ApiKey(Some("sk-...".to_string().into())),
    ..Default::default()
})?;
let model = provider.responses("gpt-5.2-codex");
// 或默认实例：每次请求时读 CODEX_API_KEY，缺失则该次调用返回 `AiMuxError::LoadApiKey`
let model = codex().responses("gpt-5.2-codex");
// 之后就是标准 LanguageModel 用法：generate_text / stream_text 均可；model.provider() == "codex.responses"
```

可用模型（2026-08 核验）：`gpt-5.2-codex`、`gpt-5.1-codex`、`gpt-5.1-codex-mini`、`gpt-5-codex`。
Codex 模型**只走 Responses API**——不要在 chat-completions 通道使用。

## 3. 订阅模式：职责边界

**集成方负责**（交互与状态）：
1. 设备码登录 UI（展示 user_code，轮询取 token）
2. token 持久化（`~/.codex/auth.json` 之类的自有存储）
3. 刷新编排：收到 `TokenExpired` → 调 `codex_refresh` → 持久化新 refresh token → 重试

**aimux 库负责**（协议面，全部无状态）：
1. 订阅模式 provider：端点、`Originator`/`ChatGPT-Account-Id` 头、**强制 `stream:true` + `store:false`**（`generate_text` 内部自动走流式采集——该端点不接受非流式请求）
2. `codex_refresh(refresh_token, client_id)`：一次 `/oauth/token` 调用，无持久化
3. 401 → `AiMuxError::TokenExpired`（`error_type` = `"TokenExpired"`，不可重试）

## 4. 集成方示例：登录 → 调用 → 刷新编排

### 4.1 设备码登录（集成方实现，协议细节来自官方 Codex CLI 源码）

```rust,ignore
// 伪代码——设备码流程属于集成方，aimux 不提供
async fn device_login(client_id: &str) -> Tokens {
    // 1. 申请设备码
    let resp = post("https://auth.openai.com/api/accounts/deviceauth/usercode",
        json!({ "client_id": client_id })).await?;
    // → { user_code, device_auth_id, interval }

    // 2. 展示: 让用户到 https://auth.openai.com/codex/device 输入 user_code（15 分钟有效）
    println!("Open https://chatgpt.com/codex/device and enter: {}", resp.user_code);

    // 3. 轮询（按 interval）
    loop {
        let r = post("https://auth.openai.com/api/accounts/deviceauth/token",
            json!({ "device_auth_id": resp.device_auth_id, "user_code": resp.user_code })).await?;
        if let Some(auth_code) = r.authorization_code {
            // 4. 用 authorization_code + PKCE code_verifier 换 token
            return post("https://auth.openai.com/oauth/token", json!({
                "grant_type": "authorization_code",
                "code": auth_code,
                "client_id": client_id,
                "code_verifier": verifier,
            })).await?; // → { access_token, refresh_token, expires_in }
        }
        sleep(resp.interval).await;
    }
}
```

> 细节：PKCE S256、scope=`openid profile email offline_access ...`、refresh token **一次性轮换**。
> 完整协议以 `openai/codex` 官方源码（`login/src/device_code_auth.rs`）为准。

### 4.2 调用（库）

```rust
use aimux_core::error::AiMuxError;
use aimux_core::generate::{generate_text, GenerateTextOptions};
use aimux_providers::{CodexMode, CodexProviderSettings, create_codex};

let provider = create_codex(CodexProviderSettings {
    mode: CodexMode::ChatGptAccount {
        token: access_token.into(),                 // OAuth 产物；也可传 Resolvable::from_async_fn 每次请求取最新 token
        account_id: Some("acct_...".to_string()),   // 可选
    },
    originator: Some("my-client".to_string()),      // 可选，默认 "aimux"
    ..Default::default()                            // base_url 默认 https://chatgpt.com/backend-api/codex
})?;
let model = provider.responses("gpt-5.2-codex");

let result = generate_text(model, "Hello", GenerateTextOptions::default()).await?;
```

#### 图片模型（仅订阅模式）

`provider.image(id)` / `Provider::image_model(id)`，`provider()` 为 `"codex.image"`：

| 模型 | 路由 | 说明 |
|---|---|---|
| `gpt-image-2` | `POST /images/generations`、`POST /images/edits`（均为 JSON，编辑用 `images: [{image_url: "data:..."}]`） | 不接受 `size` |
| `gpt-image-2.5-flare`、`gpt-image-2.5-sunburst` 及 `-2026-09-08` 快照 | `POST /responses` 流式，`image_generation` 工具（外层模型 `gpt-5.6-luna`） | `size` 可选：16 的倍数、边长 ≤ 3840、长短边比 ≤ 3、总像素 655,360–8,294,400 |

- 只发探测确认过的字段：`quality`、`outputFormat` 等 provider options 一律报错；不支持 `mask`；编辑至少一张输入图；每次一张图（`n` 必须为 1）。
- **失败不重放**：超时、连接重置、5xx、格式异常都可能已被服务端接受并计费，这些模型返回的错误都不可重试，`generate_image` 不会自动重发。直接接口的交换超时放宽到 600 秒，外层上限用 `ImageCallOptions.timeout`。
- 401 → `TokenExpired`（同语言模型）；403 表示账号无图片权限。
- API key 模式不提供图片模型（返回 `NoSuchModel`），公开 Image API 请用 `openai` 包。

### 4.3 刷新编排（集成方）

```rust
use aimux_providers::{CodexMode, CodexProviderSettings, codex_refresh, create_codex};

/// 订阅通道的 provider：token 由调用方给（刷新后重新创建，或用 Resolvable::from_async_fn 动态取）。
fn subscription(token: impl Into<String>) -> Result<aimux_providers::CodexProvider, AiMuxError> {
    create_codex(CodexProviderSettings {
        mode: CodexMode::ChatGptAccount { token: token.into().into(), account_id: None },
        ..Default::default()
    })
}

const CLIENT_ID: &str = "your-oauth-client-id";

async fn call_with_refresh(access: &str, refresh: &str, refresh_store: &mut String) -> Result<(), AiMuxError> {
    let model = subscription(access)?.responses("gpt-5.2-codex");
    match generate_text(model, "Hello", Default::default()).await {
        Ok(_) => Ok(()),
        Err(AiMuxError::TokenExpired(_)) => {
            // 1. 刷新（库的纯函数，无状态）
            let tokens = codex_refresh(refresh, CLIENT_ID).await?;
            // 2. 持久化轮换后的新 refresh token（一次性！）
            if let Some(new_refresh) = &tokens.refresh_token {
                *refresh_store = new_refresh.clone();
            }
            // 3. 用新 access token 重试一次
            let model = subscription(tokens.access_token)?.responses("gpt-5.2-codex");
            generate_text(model, "Hello", Default::default()).await?;
            Ok(())
        }
        Err(e) => Err(e),
    }
}
```

### 4.4 C ABI（Swift/Kotlin/C/Go/...）

```c
// 刷新：返回 JSON 或错误 JSON，用完 aimux_free_string 释放
char *json = aimux_codex_refresh("old-refresh", "client-id");
// → {"access_token":"...","refresh_token":"...","expires_in_secs":3600}
```

错误判别：JSON 里的 `"error_type":"TokenExpired"`（`error.rs` 的 `error_type()`）——
集成方据此触发刷新流程。`TokenExpired` 不可重试（`is_retryable() == false`），
库不会在 401 上自动重试，刷新后由你重发请求。

## 5. 注意事项（best-effort 边界）

- 订阅端点**无公开文档**，协议细节以官方 Codex CLI 源码为唯一权威，可能随官方客户端演进而变
- 该端点强制流式：库在订阅模式下**总是** `stream:true` + `store:false`，非流式请求会报错
- 配额/冷却按订阅计划（5 小时窗口），**不做账号池**——多账号轮换违反 ToU
- 订阅模式只适合个人自用；生产/CI 用 API key 模式
- WS 变体、`x-codex-turn-state` 透传、`/responses/compact` 不在 v1 范围
