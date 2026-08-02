# RFC-0018: Codex 订阅通道 provider 集成评估

> **Status**: PROPOSED (pending protocol verification)
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

## 2. 必须核验的协议事实(阻塞项)

按 [RFC-0006 §2.1](../rfc/0006-provider-development.md) 证据裁决顺序,核验以下事实:

| # | 事实 | 来源 | 当前状态 |
|---|---|---|---|
| V1 | Codex 官方 API 是否存在 API key 模式(端点/模型 ID/定价) | OpenAI 官方文档 | 未核验 |
| V2 | 订阅通道接入方式(端点、鉴权流程、条款) | OpenAI 官方文档/官方 SDK | 未核验 |
| V3 | 订阅 OAuth 设备码流程细节(scope/刷新/失效) | 参考实现(pi/opencodex/codex) | 参考实现有,官方未确认 |
| V4 | 协议形态(Responses API?WebSocket?) | 官方 + 参考实现 | 参考实现不一致,待确认 |
| V5 | 流式与工具调用支持 | 官方 + 参考实现 | 待确认 |

**核验完成后**更新本 RFC 的 §3/§4 并进入 review;若 V1 成立(有官方 API key 模式),
建议实现路径改为"标准 Responses 薄封装 + 订阅通道降级为 backlog"。

---

## 3. 设计草案(待核验后定稿)

### 3.1 路径 A:标准 API key 模式(若 V1 成立)

- 走现有 `openai` provider 的 Responses API 通道(codex 模型 + API key)
- 无需新架构;`OpenAICompatProfile`/`OpenAIResponsesModel` 复用
- 工作量:小(模型 ID + 验证测试)

### 3.2 路径 B:订阅通道(若 V2/V3 成立且条款允许)

新增 `aimux-providers/src/codex/`:

```
CodexConfig {
  mode: ApiKey | Subscription,
  // ApiKey 模式复用 OpenAIConfig 能力
  // Subscription 模式:
  account_token: Option<String>,   // OAuth 产物
  refresh_token: Option<String>,
  base_url: String,                // 默认 https://chatgpt.com/backend-api
}
```

- **凭证管理**:OAuth 设备码(登录一次,存 refresh_token,自动刷新);
  参考实现:pi(`openai-codex` OAuth)、opencodex(账号池 + cooldown/failover)
- **协议**:Responses API 透传 + `ChatGPT-Account-Id`/`x-codex-turn-state` 头
  (实现细节以 V4 核验结果为准)
- **明确不做**:账号池/多账号负载均衡(网关系,见 [LEARNINGS](../reference/audit/LEARNINGS.md) 定位结论)

---

## 4. 验收标准

- [ ] V1-V5 核验记录附于本 RFC(官方文档 URL + 结论)
- [ ] 核验结论决定路径 A 或 B,更新 §3 后 review
- [ ] 实现后:cassette 测试覆盖非流式/流式/工具调用(复用 RFC-0003 测试方案)
- [ ] 订阅模式凭证刷新有测试(401 → 刷新 → 重试)

## 5. 参考资料

- 审计:`reference/audit/{pi,opencodex,cline,Roo-Code,agent-of-empires,axonhub,CCSwitcher}.md`
- inventory 候选:codex(high,7 项目,`special:5, none:2`)
- 参考实现:pi `openai-codex`、opencodex `accounts`、codex CLI `[model_providers]`
