# Session Affinity 集成指南

> **适用版本**: aimux 0.x(2026-08-03 定稿,[RFC-0019](../rfc/0019-session-affinity.md))
> **结论先行**: aimux **不预置、不自动注入**任何会话头——会话头是"你的网关"的属性,不是 provider 的属性。
> 机制已存在(`headers` 参数),本文档告诉你**传什么、怎么传、何时别传**。

---

## 1. 一句话背景

LLM 网关/上游普遍提供"会话亲和":同一会话的请求带上稳定标识,网关据此把请求路由到同一上游/实例,从而**命中提示缓存、降低延迟与成本**。aimux 是接入层,不做路由、不做账号池、不管理会话状态——只负责把你的标识**原样透传**。

## 2. 支持矩阵(2026-08-03 调研)

来源:reference/audit 全量扫描(22 项目命中)+ 官方文档核证。按**你面对的网关/上游**查表:

| 你的上游/网关 | 会话标识 | 注入方式 | 语义 | 证据 |
|---|---|---|---|---|
| **OpenRouter** | `session_id`(body 顶层,≤256 字符)或 `x-session-id` 头,body 优先 | header 或 body | **粘性路由键**(默认按首条消息哈希识别会话;显式传则直接用);兜底 `prompt_cache_key`;按 账户×模型×会话 粒度跟踪 | [OpenRouter prompt-caching 指南](https://openrouter.ai/docs/guides/best-practices/prompt-caching) |
| **Cloudflare Workers AI** | `x-session-affinity` | header | 路由到同一模型实例,提升前缀缓存命中 | [Workers AI prompt-caching](https://developers.cloudflare.com/workers-ai/features/prompt-caching/) |
| **Anthropic 自托管**(dynamo/SGLang 等) | `x-claude-code-session-id` / `x-claude-code-agent-id` / `x-claude-code-parent-agent-id` | header | Claude Code harness → 自托管会话路由(子 agent KV 隔离);**对 Anthropic 托管 API 的缓存不参与** | [dynamo.md](../reference/audit/dynamo.md#L102-L107) |
| **ChatGPT 订阅(Codex)** | `session_id`/`originator` 私有头 + `x-codex-turn-state`(服务端回传) | header | 私有约定、无文档;属 [RFC-0018](../rfc/0018-codex-subscription.md) 通道,只透传白名单头 | [plano.md](../reference/audit/plano.md#L88)、[uni-api.md](../reference/audit/uni-api.md#L57-L61) |
| **Kimi / Z.AI** | `prompt_cache_key`(透传不生成) | header/body | 缓存标识;值由你的上游体系决定 | [opencodex.md](../reference/audit/opencodex.md)、[cc-switch.md](../reference/audit/cc-switch.md) |
| **LiteLLM** | `x-litellm-session-id` / `x-litellm-trace-id` / `x-<vendor>-session-id` | header/body | 网关亲和/固定 deployment | [litellm.md](../reference/audit/litellm.md#L113-L120) |
| **TokenHub** | `x-tokenhub-session-id` / `session-id` / `thread-id` | header | rendezvous 缓存亲和 | [TokenHub.md](../reference/audit/TokenHub.md#L113-L131) |
| **llmgateway** | `x-session-id` → `prompt_cache_key` → `user`(解析优先级) | header/body | 钉 provider+region 保上游 prompt cache | [llmgateway.md](../reference/audit/llmgateway.md#L156-L160) |
| **OpenAI 官方 API** | **无 session id** | — | 会话用 `previous_response_id`/`conversation.id`(与本文档无关) | OpenAI conversation-state 指南 |
| 其他自建网关 | 看你的网关文档 | — | 无统一标准 | — |

> ⚠️ **"支持 Responses API" ≠ "支持 session id"**。Responses 规范无 session id 概念;OpenRouter 的 Responses 实现 stateless,甚至 400 拒绝 `previous_response_id`。判据只能是网关自己的文档。

## 3. 集成示例(各语言)

### Rust —— `GenerateTextOptions.headers`

```rust
use aimux_providers::openai;
use aimux_core::{generate::generate_text, options::GenerateTextOptions};
use std::collections::HashMap;

let model = openai("sk-...", "gpt-4o", None);

// 会话标识:由你(集成方)持有——agent 循环里生成一次,复用整轮
let session_id = "conv_abc123".to_string();

let mut headers = HashMap::new();
headers.insert("x-session-id".to_string(), session_id.clone());

let result = generate_text(model, "你好", GenerateTextOptions {
    headers: Some(headers),
    ..Default::default()
}).await?;
```

### Node —— 工厂级(所有请求)或调用级

```ts
import { openai, generateText } from 'aimux';

// 工厂级:整个 provider 实例都带
const model = openai('sk-...', 'gpt-4o', {
  headers: JSON.stringify({ 'x-session-id': 'conv_abc123' }),
});

// 调用级:仅本次调用(推荐——会话是调用级概念)
const result = await generateText(model, '你好', {
  headers: JSON.stringify({ 'x-session-id': 'conv_abc123' }),
});
```

### Python —— options 字典透传

```python
from aimux import openai, generate_text

model = openai("sk-...", "gpt-4o")
result = generate_text(model, "你好", {
    "headers": {"x-session-id": "conv_abc123"},
})
```

### FFI 语言(Swift/Kotlin/C/Go/Java/Flutter)

headers 经 options JSON 透传,无额外代码:

```jsonc
// opts_json 里加一个键即可
{ "headers": { "x-session-id": "conv_abc123" } }
```

## 4. 何时**别**传(安全规则)

1. **opt-in**:只在你有真实会话标识时传;aimux 不会替你生成/默认注入
2. **值校验**:≤256 字符、可打印 ASCII、无 CR/LF/NUL(非法值 → 400/431)
3. **AWS SigV4 通道**(bedrock/vertex 等):**不要**加会话头——签名链不一致 → SignatureDoesNotMatch(官方 codex 客户端就是在签名前剔除 `session_id` 等头的)
4. **私有后端**(ChatGPT 订阅等):只透传其放行的头(白名单)
5. **严格网关**:LiteLLM 官方文档明言 "Some providers reject unknown headers"——目标网关不认识的头,别发
6. **意外语义**:`x-session-id` 对 OpenRouter 是粘性路由键——传了就**真的会钉住路由**,值变长/变短会切换上游,请保持同一会话内稳定

## 5. FAQ

**Q: 默认给所有请求传一个 session id 会不会报错?**
A: 大概率不会(HTTP 语义下未知头被忽略,RFC 9110;OpenCode 对全部 provider 每请求发送为实证),但非零风险:SigV4 签名链、严格白名单网关、私有后端三类场景可能真实报错;且对 OpenRouter 会触发真实行为变更(粘性钉住)。**正确做法:opt-in,只对你的网关支持的头传。**

**Q: 为什么 aimux 不把会话头预置进 registry?**
A: 会话头是"你的网关"的属性,不是 provider 的属性。同一个 OpenAI 兼容 provider,直连官方、走 OpenRouter、走自建网关,要传的头完全不同;而大多数自建网关根本不在 registry 里。预置会误导,且与集成方的部署强耦合——所以 aimux 交付文档,不交付预置数据。

**Q: 我的上游是 OpenRouter,`session_id` 传 body 还是 header?**
A: OpenRouter 两者都接受,body 顶层 `session_id` 优先于 `x-session-id` 头。aimux 的 `body_overrides` 可传 body 字段,`headers` 可传头;二选一即可。

**Q: 会话 id 谁来生成?**
A: 你。aimux 不做会话状态管理(生命周期由你持有);同一会话内保持值稳定即可获得粘性/缓存收益。

## 6. 附:完整调研矩阵(22 项目)

| 项目 | 会话头/字段 | 注入方式 | 机制 | 证据 |
|---|---|---|---|---|
| cline | `session_id` | body(json-body) | 缓存标识 | cline.md#L82-L85 |
| pi | `x-session-affinity` | header | 前缀缓存折扣(Workers AI) | pi.md#L75-L79 |
| llmgateway | `x-session-id`→`prompt_cache_key`→`user` | header/body | sticky 路由 | llmgateway.md#L156-L160 |
| dynamo | `x-claude-code-session-id` 族 | header | 自托管 KV 隔离 | dynamo.md#L102-L107 |
| claude-code-router | `sessionId` | 内部脚本 | 哈希分桶路由 | claude-code-router.md#L188-L191 |
| TokenHub | `x-tokenhub-session-id`/`thread-id`/`prompt_cache_key` | header/字段 | rendezvous 亲和 | TokenHub.md#L113-L131 |
| axonhub | `traceID` | 内部 | 渠道级 sticky | axonhub.md#L149-L157 |
| plano | `X-Model-Affinity` | header | 路由结果缓存 | plano.md#L142-L147 |
| uni-api | `Session_id`/`Originator` | header | 账号绑定(ChatGPT 订阅) | uni-api.md#L57-L61 |
| opencodex | 白名单头(`authorization`/`chatgpt-account-id`/`beta`/`originator`/`session`);kimi/zai `prompt_cache_key` 透传 | header | 账号池亲和 | opencodex.md#L100-L104 |
| cc-switch | 稳定 `prompt_cache_key` + `session_id` | body | 缓存标识+用量 | cc-switch.md#L157-L161 |
| gemini-cli | `session_id`/`user_prompt_id` | body | 状态管理(Code Assist) | gemini-cli.md#L43-L44 |
| ai.rs | `session_id`+`x-client-request-id`+`x-session-affinity`(Openai)/`x-session-id`(Openrouter) | header/body | 缓存标识,**opt-in 开关** | ai.rs.md#L88-L93 |
| pydantic-ai | `openai_previous_response_id`/`openai_conversation_id` | body | 服务端会话复用(OpenAI) | pydantic-ai.md#L145-L150 |
| litellm | `x-litellm-session-id`/`x-<vendor>-session-id` | header/body | deployment 亲和 | litellm.md#L113-L120 |
| new-api | Channel Affinity 规则引擎 | 内部 | 渠道固定 | new-api.md#L110-L117 |
| awaken | thread/run id | 内部 | HRW 路由 | awaken.md#L113-L119 |
| codex(官方) | `x-codex-turn-state` | header | 官方粘性路由 | codex.md#L76-L81 |
| opencode | `X-Session-Id` + `x-session-affinity`(全量发送) | header | 缓存标识(被忽略无害实证) | opencode#39913 |
| claude-code-router / TokenHub / axonhub / awaken | 见上 | — | — | — |

> 以上行号基于 2026-08-01 审计快照,后续提交可能偏移。
