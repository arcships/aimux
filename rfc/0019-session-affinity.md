# RFC-0019: 会话亲和(session affinity)支持方案

> **Status**: ACCEPTED (2026-08-03 定稿为文档化方案——代码零改动)
> **Date**: 2026-08-02(初稿)/ 2026-08-03(定稿)
> **Scope**: 为 aimux 用户提供"会话亲和头怎么传、何时传、安全边界"的最佳实践集成指南与支持矩阵。
> 不新增 core 字段、不预置 registry 数据、不改任何绑定层代码——现有 `CallOptions.headers` 已可表达全部机制。
> **Related**: [RFC-0017](0017-provider-config-dx.md) 配置层、[RFC-0015](0015-cache-trace-audit.md) 缓存审计(session scope)、
> [RFC-0018](0018-codex-subscription.md) 订阅通道(私有会话头)

---

## 1. 背景与动机

### 1.1 审计发现

参考项目审计(104 项目)中,**15+ 项目实现会话亲和**,分三档形态:

| 形态 | 代表实现 | 机制 |
|---|---|---|
| 轻:固定 header/body 注入 | cline、pi、llmgateway、dynamo | `session_id` → `x-session-id` / `x-session-affinity` / `x-claude-code-session-id` |
| 中:哈希粘性路由 | TokenHub、claude-code-router、axonhub、plano | `hash(sessionId)%100`、SHA-256 rendezvous、`TraceStickyMode=prefer_previous_channel` |
| 重:账号/会话级绑定 | codex、opencodex、uni-api | `x-codex-turn-state`、账号池按会话选择 |

**价值**:同一会话固定同一上游/携带稳定会话标识,可提升网关侧粘性路由与上游提示缓存命中率,降低延迟与成本。

### 1.2 定稿前调研结论(2026-08-03)

对 reference/audit 全量扫描(22 项目命中)+ 联网核证官方文档后,三个事实改变了设计:

1. **会话头是"网关/部署"属性,不是"provider"属性**。同一个 OpenAI 兼容 provider,用户可能直连官方、或走 OpenRouter、或走自建 LiteLLM/网关——会话头名由**用户面对的网关**决定,不由 registry 里的 provider 决定。按 provider 预置(无论 registry 字段还是 core 常量表)都会误导,且大量自建网关根本不在 registry 里。因此**预置注入与集成方开发强耦合,正确形态是文档化最佳实践**。
2. **默认注入未知头整体安全但非零风险**。HTTP 语义上未知头被忽略(RFC 9110;OpenCode 对全部 provider 每请求发会话头为实证);但四类例外:非法字符/超长(400/431)、**AWS SigV4 签名链**(签名后加头 → SignatureDoesNotMatch)、严格白名单网关(LiteLLM 文档明言 "Some providers reject unknown headers")、私有后端(ChatGPT 订阅通道只放行白名单头)。另有意外语义:给 OpenRouter 发 `x-session-id` 会真实触发粘性钉住。→ **必须 opt-in,由集成方按自己的网关决定;库不做默认注入。**
3. **"支持 Responses API" ≠ "支持 session id"**。Responses 规范无 session id(官方用 `previous_response_id`/`conversation.id`/手动回灌);OpenRouter 的 Responses 实现 stateless 且**拒绝** `previous_response_id`(400);session id 是网关私有扩展。→ 不能以协议面作判据,只能逐家文档化。

### 1.3 aimux 现状

- `CallOptions.headers: Option<HashMap<String, String>>` 已存在([options.rs:93](../aimux-core/src/options.rs#L93))——**零代码即可表达**任意会话头
- Node 绑定:工厂级 `ProviderConfig.headers` + 每次调用 `opts.headers`([lib.rs:224/319](../bindings/node/src/lib.rs#L224));Python/FFI 经 options JSON 透传
- 但无统一文档:各厂商头名不同(`x-session-id`/`x-session-affinity`/`x-claude-code-session-id`/`prompt_cache_key` 等),用户需自行查表

### 1.4 边界(明确不做)

- **不做**哈希粘性/渠道路由/账号池(中/重形态)——属网关/账号层,aimux 定位是接入层(LEARNINGS.md 结论)
- **不做**会话状态管理(会话生命周期由用户持有,aimux 只透传)
- **不做**自动生成 session id、不默认注入、不按 provider 预置头名

---

## 2. 设计(定案:文档化方案,代码零改动)

### 2.1 核心决策

会话亲和的全部机制已存在于 `CallOptions.headers` / `ProviderConfig.headers`。
aimux 的交付物是**集成指南 + 支持矩阵 + 安全规则**,而非新代码:

1. 新增 `docs/session-affinity-guide.md` 集成指南(本 RFC 的实现物):
   - 支持矩阵(§2.2)+ 每个上游的官方语义、头名、注入方式、示例代码
   - 安全规则(§2.3)与校验清单
   - 各语言示例:Rust `with_headers`、Node `headers` 参数、Python/FFI JSON 透传
2. 绑定层 `sessionId` 便捷参数:**记为可选增强,不做**——用户有真实需求(如多语言一致体验)再议;届时实现为 wrapper 层纯 sugar(合并规则 `{...auto, ...user}`,用户显式优先),不碰 core/FFI wire

### 2.2 支持矩阵(2026-08-03 调研;完整 22 项目矩阵见集成指南)

| 上游/网关 | 会话标识 | 注入方式 | 语义 | 证据 |
|---|---|---|---|---|
| OpenRouter | `session_id`(body 顶层,≤256 字符)或 `x-session-id` 头,body 优先 | header/body | **粘性路由键**,兜底 `prompt_cache_key`;按 账户×模型×会话 粒度跟踪 | openrouter.ai/docs/guides/best-practices/prompt-caching |
| Cloudflare Workers AI | `x-session-affinity` | header | 路由到同一模型实例,提升前缀缓存命中 | developers.cloudflare.com/workers-ai/features/prompt-caching |
| Anthropic 生态 | `x-claude-code-session-id` / `-agent-id` / `-parent-agent-id` | header | Claude Code harness → **自托管**会话路由(SGLang 子 agent KV 隔离,dynamo);对托管 API 缓存**不参与** | [dynamo.md](../reference/audit/dynamo.md#L102-L107) |
| ChatGPT 订阅(Codex) | `session_id`/`originator` 私有头 + `x-codex-turn-state`(服务端回传) | header | 私有约定,无文档;属 [RFC-0018](0018-codex-subscription.md) 通道 | [plano.md](../reference/audit/plano.md#L88)、[uni-api.md](../reference/audit/uni-api.md#L57-L61) |
| Kimi / Z.AI | `prompt_cache_key`(仅透传不生成) | header/body | 缓存标识;值由用户/上游体系决定 | [opencodex.md](../reference/audit/opencodex.md)、[cc-switch.md](../reference/audit/cc-switch.md) |
| OpenAI 官方 API | **无 session id** | — | 会话用 `previous_response_id`/`conversation.id`;OpenRouter Responses 实现甚至 400 拒绝 previous_response_id | developers.openai.com 对话状态指南 |
| 自建网关约定 | litellm `x-litellm-session-id`、TokenHub `x-tokenhub-session-id`、llmgateway `x-session-id`→`prompt_cache_key`→`user` 等 | header/body | 各自约定,无统一标准;集成方按自己的网关选 | [litellm.md](../reference/audit/litellm.md)、[TokenHub.md](../reference/audit/TokenHub.md)、[llmgateway.md](../reference/audit/llmgateway.md) |

### 2.3 安全规则(opt-in 注入的边界)

1. **opt-in**:用户显式传 session 标识才注入;库不自动生成、不默认注入
2. **值校验**:≤256 字符、可打印 ASCII、无 CR/LF/NUL(非法值可能 400/431)
3. **SigV4 排除**:bedrock/vertex 等签名请求通道不注入会话头(签名链不一致 → SignatureDoesNotMatch;官方 codex 客户端即签名前剔头)
4. **私有后端仅白名单**:ChatGPT 订阅等通道只透传其放行的头
5. **意外语义提示**:对 OpenRouter 类网关传 `x-session-id` 会触发粘性钉住——这是功能,但文档需明示
6. **与显式 headers 冲突时用户优先**(若未来加 sugar,合并规则 `{...auto, ...user}`)

---

## 3. 验收标准(文档交付,代码零改动)

- [ ] `docs/session-affinity-guide.md` 落盘:支持矩阵(≥8 上游,含官方语义/头名/注入方式)+ 安全规则 + 校验清单
- [ ] 指南含 3 种以上语言示例(Rust/Node/Python 起步),FFI 语言注明 headers JSON 透传
- [ ] 指南明示:"支持 Responses API"不等于支持 session id(§1.2-3);OpenAI 官方 API 无 session id
- [ ] 指南引用全部证据位置(reference/audit 文件:行号 + 官方文档 URL)
- [ ] 代码零改动(本 RFC 定稿即验收);若后续加绑定层 sugar,需单独提案或本 RFC 修订

---

## 4. 参考资料

- 审计:`reference/audit/{cline,pi,llmgateway,dynamo,claude-code-router,TokenHub,axonhub,plano,uni-api,opencodex,cc-switch,gemini-cli,ai.rs,pydantic-ai,litellm,new-api,awaken}.md`、`reference/audit/LEARNINGS.md`
- 官方文档:OpenRouter prompt-caching、Cloudflare Workers AI prompt-caching、OpenAI conversation-state、LiteLLM request_headers/forward_client_headers、RFC 9110
- 调研存档:2026-08-03 会话亲和调研(22 项目矩阵 + 注入安全性 + Responses 会话机制三份结论,完整矩阵并入集成指南)
