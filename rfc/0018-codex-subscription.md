# RFC-0018: Codex 订阅通道 provider 集成评估

> **Status**: IMPLEMENTED (2026-08-03 核验→定案→实现完成:Path A + Path B + codex_refresh + C ABI,测试全绿)
> **Date**: 2026-08-02
> **Scope**: 评估并设计 Codex(OpenAI 编程模型/订阅通道)在 aimux 的集成路径。
> 本 RFC 基于 104 项目参考审计的发现,先给出动机与现状,再列出**必须核验的协议事实**,
> 核验通过后按本 RFC 的设计草案实现;核验不通过则降级为 backlog。
> **Related**: [RFC-0016](0016-align-with-aisdk.md) AISDK 对齐、[RFC-0006](0006-provider-development.md) provider 开发规范、
> [provider-inventory/INTEGRATION-PRIORITY.md](../provider-inventory/INTEGRATION-PRIORITY.md)

---

## 1. 背景与动机

### 1.1 审计发现

参考项目审计(104 项目,2026-08-01)中,**7 个编码 agent/网关项目**支持 Codex 通道:
agent-of-empires、axonhub、CCSwitcher、cline、pi、Roo-Code、uni-api。
接入判定 **5 个 special + 2 个 none**,即参考实现全部走**特殊接入**(订阅 OAuth/私有端点),
没有任何一个走标准 API key 薄封装。

参考实现要点(见各项目审计文件):
- **pi**: `openai-codex` provider,ChatGPT 订阅 OAuth(device flow/account_id + refresh_token),私有端点 `chatgpt.com/backend-api/codex/responses`
- **opencodex**: Responses 透传 + 账号池(affinity/quota/cooldown/failover),`x-codex-turn-state` 粘性路由
- **codex CLI**: 官方 CLI,`[model_providers.<id>]` 自定义 + 订阅 token,Responses-over-WebSocket
- **cline/pi/Roo-Code**: OAuth 设备码登录 + token 刷新

### 1.2 为什么值得评估

- Codex 是 OpenAI 官方编程模型,7 个参考项目背书,是"订阅通道"这一类
  (`subscription_proxy`:github_copilot/chatgpt/codex/kiro/cursor)中支持面最广的
- 订阅通道的**用户价值**与 API key 不同:固定订阅费、无需按 token 计费
- aimux 现有 provider 全部是 API key 模式,订阅通道是**全新的接入形态**,
  涉及 OAuth 设备码 + token 刷新 + 私有端点,需要独立的配置与凭证模型

### 1.3 风险与不确定

- OpenAI 官方对第三方接入订阅通道的**条款限制**(历史上 ChatGPT 订阅 API 为逆向/非官方路径),
  必须核验官方文档是否允许
- 若官方已提供 **Codex API key 模式**(Responses API + `codex-1` 模型),则大多数用户走
  标准路径即可,订阅通道价值下降——**先核验再决定**

---

## 2. 协议事实核验结果(2026-08-03)

按 [RFC-0006 §2.1](0006-provider-development.md) 证据裁决顺序核验完毕。
证据来源:OpenAI 官方文档(developers.openai.com / learn.chatgpt.com / openai.com)+
`openai/codex` 官方源码(当日 main 分支稀疏克隆核验)+ 本地参考实现交叉验证。

| # | 事实 | 核验结论 | 证据 | 可靠度 |
|---|---|---|---|---|
| V1 | Codex 官方 API(API key 模式)是否存在 | **存在且官方推荐**。Codex 模型**只走 Responses API**(端点 `https://api.openai.com/v1/responses`)。当前模型与定价:gpt-5.2-codex $1.75/$14(400K ctx)、gpt-5.1-codex $1.25/$10、gpt-5.1-codex-mini $0.25/$2、gpt-5-codex $1.25/$10。**"codex-1" 不是 API 模型 ID**——是 ChatGPT/Codex 云端内部模型名;API 侧 ID 为 gpt-5.x-codex 系列 | developers.openai.com/api/docs/models/gpt-5.2-codex 等 | 官方文档 ✅ |
| V2 | 订阅通道接入方式与条款 | 订阅登录**是官方支持功能**(Codex CLI/IDE/桌面 "Sign in with ChatGPT for subscription access");但 `chatgpt.com/backend-api/codex/responses` **无任何公开文档**(仅官方客户端自用)。消费者 ToU 红线:禁止程序化提取输出、禁止绕过限流、**禁止共享账号凭据**——账号池/多账号轮换/转售踩线;单账号自用为官方允许边界 | learn.chatgpt.com/docs/auth;openai.com/policies/row-terms-of-use(2026-01-01 版) | 官方文档 ✅(端点无官方表述=第三方推断 ⚠️) |
| V3 | 订阅 OAuth 设备码流程 | 官方流程(源码):issuer `auth.openai.com`;①`/api/accounts/deviceauth/usercode` → user_code/device_auth_id/interval;②用户输码(user_code 15 分钟过期);③轮询 `/api/accounts/deviceauth/token` → authorization_code + PKCE verifier;④`/oauth/token` grant_type=authorization_code 换 id/access/refresh token。scope=`openid profile email offline_access api.connectors.read api.connectors.invoke`,PKCE S256。**刷新**:`/oauth/token` grant_type=refresh_token,到期前 5 分钟窗口每 8 分钟自动检查;**refresh token 一次性轮换**(reused/expired/invalidated 均需重新登录)。凭据存 `~/.codex/auth.json` 或 keyring | openai/codex `login/src/device_code_auth.rs`、`login/src/auth/manager.rs`;learn.chatgpt.com/docs/auth | 官方源码 ✅(与 plano/uni-api 参考实现一致) |
| V4 | 协议形态 | **Responses-over-HTTP**:订阅通道 `POST https://chatgpt.com/backend-api/codex/responses`(官方常量 `CHATGPT_CODEX_BASE_URL` + `/responses`)。**存在 WS 变体**(同路径 wss,官方 CLI 优先 WS、重试预算耗尽回退 HTTP)。另有 `/responses/compact`、`/backend-api/codex/models`。参考实现(opencodex/plano/uni-api)全部指向同一端点 | openai/codex `model-provider-info/src/lib.rs`、`core/src/client.rs` | 官方源码 ✅ |
| V5 | 流式与工具调用 | 官方客户端**恒发 `stream:true` + `store:false`**;"非流式请求报错"无官方原文(plano 第三方记录),按"订阅模式仅支持流式"设计并容错。工具调用完整支持(Responses 工具事件流);**代码执行是客户端侧能力**(模型出工具调用,CLI 本地沙箱执行);订阅端另有 `x-codex-turn-state` 头做服务端同会话粘性路由 | openai/codex `core/src/client.rs`;本地 [plano.md](reference/audit/plano.md#L88) | 官方源码 ✅(非流式报错=第三方推断 ⚠️) |

**核验结论**:V1 成立 → **路径 A(API key 薄封装)必做**,是默认通道;路径 B(订阅通道)做单账号自用形态。

---

## 3. 设计(定案)

### 3.1 路径 A:标准 API key 模式(默认通道,必做)

- 走现有 OpenAI Responses 通道(codex 模型 + API key),`OpenAIResponsesModel` 复用
- **实现形态(2026-08-03)**:新增原生模块 `aimux-providers/src/codex.rs`(对齐 openrouter.rs 模式),导出 `CodexConfig`/`CodexProvider`/`CodexModel` + `CODEX_API_KEY_ENV_VAR`(读 `CODEX_API_KEY`);**registry 条目不可行**——registry 只表达 OpenAI 兼容 chat-completions 通道(`provider()` 统一走 `OpenAIProvider.language_model`),而 Codex 模型只接受 Responses API
- 模型表:gpt-5.2-codex / gpt-5.1-codex / gpt-5.1-codex-mini / gpt-5-codex(退役日期不硬编码)
- 工作量:小(模块 + 模型 ID + 测试,已完成)

### 3.2 路径 B:订阅通道(可选模式,单账号自用)

新增 `aimux-providers/src/codex/`(Subscription 模式):

```
CodexConfig {
  mode: ApiKey | Subscription,
  // ApiKey 模式复用 OpenAIConfig 能力
  // Subscription 模式:
  account_token: Option<String>,   // 集成方 OAuth 产物(access token)
  base_url: String,                // 默认 https://chatgpt.com/backend-api
}
```

**职责分离(2026-08-03 定案):OAuth 由集成方做,库只做协议面。**

| 层 | 职责 | 形态 |
|---|---|---|
| 库(协议面,无状态) | 订阅模式 provider:端点/头/流式解析,**强制 `stream:true` + `store:false`**(generate_text 内部走流式采集 shim;非流式请求报错);`codex_refresh(refresh_token, client_id) -> Tokens` **纯函数**(一次 `/oauth/token` 调用,无持久化);401 → `AiMuxError::TokenExpired` 类型化错误,**不自动重试刷新** | Rust API + C ABI,8 语言可调 |
| 集成方(交互与状态) | 设备码登录 UI(user_code 展示 → 轮询 → 取 token);token 持久化(库零明文落盘);刷新编排(收到 TokenExpired → 调 `codex_refresh` → 存新 token → 重试) | 参考示例脚本(库外) |

**明确不做(ToS/定位红线)**:
- 账号池/多账号负载均衡/配额轮换(opencodex pool、uni-api 多账号)——违反 ToU"禁止共享账号凭据/绕过限流"
- 转售或对外暴露共享 API 面
- v1 不做 WS 变体(记未来);不做 `x-codex-turn-state` 透传(服务端 opaque token,官方自述"对第三方网关无意义",记未来);不做 `/responses/compact`
- 订阅端配额窗口/模型可用性不硬编码(随产品策略变动)

**风险**:端点无文档、无 SLA,协议细节以官方 CLI 源码为唯一权威,随官方客户端演进而变——订阅模式标注"best-effort,非稳定承诺"。

---

## 4. 验收标准

- [x] V1-V5 核验记录(本 RFC §2,官方文档 URL + 源码位置)
- [x] 核验结论定案(2026-08-03):Path A 必做 + Path B 单账号自用(§3)
- [x] Path A 实现:非流式/流式/工具调用测试覆盖(wiremock,复用 openai_responses_test.rs 模式的真实响应形状;真实 codex cassette 后续可补)
- [x] 订阅模式:`codex_refresh`/`codex_refresh_at` 纯函数单测(wiremock mock `/oauth/token`);401 → `TokenExpired` 错误测试(do_generate + do_stream);强制流式 shim 测试(stream:true + store:false 断言)
- [x] 集成方示例文档:[docs/codex-subscription-guide.md](../docs/codex-subscription-guide.md)(设备码登录 → 取 token → 调用 → 刷新编排 完整流程)
- [x] C ABI:`aimux_codex_refresh` 导出 + 头文件声明(2026-08-03 追加)

## 5. 实施状态(2026-08-03)

| 项 | 状态 | 位置 |
|---|---|---|
| `AiMuxError::TokenExpired` 变体 | ✅ | aimux-core/src/error.rs(error_type="TokenExpired",不可重试) |
| `codex.rs` 模块(ApiKey + Subscription) | ✅ | aimux-providers/src/codex.rs |
| 订阅强制流式 shim(generate 采集 response.completed) | ✅ | codex.rs `subscription_generate` |
| 订阅 401 → TokenExpired 映射 | ✅ | codex.rs `map_subscription_401`(订阅模式任何 `Auth` 错误即 token 问题) |
| `codex_refresh` / `codex_refresh_at`(OAuth 刷新纯函数,零重试) | ✅ | codex.rs(重试禁用理由:refresh token 一次性轮换) |
| C ABI `aimux_codex_refresh` | ✅ | aimux-ffi/src/lib.rs + aimux-ffi.h |
| 测试 10 项 | ✅ | aimux-providers/tests/codex_test.rs(全绿) |
| Node/Python 绑定工厂(codex 构造入口) | ⏳ 后续 | 绑定层按需补 `codex()` 工厂(与 openai/anthropic 同模式) |

**实现偏差记录**:§3.1 原计划"registry 条目",实现改为原生模块(理由见 §3.1);订阅端点无文档,
`Originator` 默认值 `"aimux"`、头语义以参考实现(plano/uni-api/opencodex)为准,best-effort。

## 6. 参考资料

- 审计:`reference/audit/{pi,opencodex,cline,Roo-Code,agent-of-empires,axonhub,CCSwitcher}.md`
- inventory 候选:codex(high,7 项目,`special:5, none:2`)
- 参考实现:pi `openai-codex`、opencodex `accounts`、codex CLI `[model_providers]`
- 官方文档:developers.openai.com/api/docs/models/gpt-5.2-codex、learn.chatgpt.com/docs/auth、learn.chatgpt.com/codex/pricing、openai.com/policies
- 官方源码:openai/codex(`login/src/device_code_auth.rs`、`model-provider-info/src/lib.rs`、`core/src/client.rs`、`codex-api/src/lib.rs`)

## 7. 决策记录

| 日期 | 决策 | 理由 |
|---|---|---|
| 2026-08-03 | V1-V5 核验完成;Path A 必做、Path B 单账号自用 | API key 模式官方且支持 CI/自动化;订阅端点无文档、ToU 红线禁止账号池/共享凭据 |
| 2026-08-03 | OAuth(设备码 UI/持久化/刷新编排)归集成方;库只做协议面(provider + `codex_refresh` 纯函数 + `TokenExpired` 错误) | refresh token 一次性轮换,持久化天然是集成方职责;库保持无状态、零落盘、8 语言一致 |
