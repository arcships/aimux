# RFC-0019: 会话亲和(session affinity)轻量支持

> **Status**: DRAFT (pending review)
> **Date**: 2026-08-02
> **Scope**: 在 aimux 请求层增加可选的 `session_id`,由 provider 自动注入各厂商的
> 会话/提示缓存头。轻量设计,不引入粘性路由。
> **Related**: [RFC-0017](0017-provider-config-dx.md) 配置层(本 RFC 的字段走同一 options 通道)、
> [RFC-0005](0005-protocol-conversion.md) 协议转换(头部注入点)

---

## 1. 背景与动机

### 1.1 审计发现

参考项目审计(104 项目)中,**15+ 项目实现会话亲和**,分三档形态:

| 形态 | 代表实现 | 机制 |
|---|---|---|
| 轻:固定 header/body 注入 | cline、pi、llmgateway、dynamo | `session_id` → `x-session-id` / `x-session-affinity` / `x-claude-code-session-id` |
| 中:哈希粘性路由 | TokenHub、claude-code-router、axonhub、plano | `hash(sessionId)%100`、SHA-256 rendezvous、`TraceStickyMode=prefer_previous_channel` |
| 重:账号/会话级绑定 | codex、opencodex、uni-api | `x-codex-turn-state`、账号池按会话选择 |

**价值**:同一会话固定同一上游/携带稳定会话标识,可提升上游提示缓存命中率
(Anthropic prompt caching、OpenAI prompt cache 均按会话/前缀命中),降低延迟与成本。

### 1.2 aimux 现状

- `CallOptions.headers: Option<HashMap<String, String>>` 已存在(options.rs:74)——**零代码即可表达**
  任意会话头(用户手动传 `x-session-id` 即可)
- 但无**一等公民**字段:各厂商头名不同(`x-session-id`/`x-session-affinity`/
  `x-claude-code-session-id`/`prompt_cache_key` 等),用户需自行查表,
  且流式/非流式/各 provider 行为由用户保证一致

### 1.3 边界(明确不做)

- **不做**哈希粘性/渠道路由/账号池(中/重形态)——属网关/账号层,aimux 定位是接入层
  (LEARNINGS.md 结论)
- **不做**会话状态管理(会话生命周期由用户持有,aimux 只透传)

---

## 2. 设计

### 2.1 用户面:复用 GenerateTextOptions 的 headers,或新增便捷字段

两条候选路径,倾向 **A**(最小侵入):

**路径 A(推荐)**:不新增核心字段,文档化 + 便捷常量

- 在 `aimux-core` 增加会话头常量表(供用户/绑定层查表):

```rust
/// 会话亲和 header 名(按 provider 分组,供用户选用)。
pub const SESSION_HEADERS: &[(&str, &str)] = &[
    ("openai", "x-session-id"),            // OpenAI 系(部分网关)
    ("anthropic", "x-claude-code-session-id"), // Anthropic 系(dynamo 透传)
    ("openrouter", "x-session-id"),        // OpenRouter sticky session
    // ...
];
```

- 用户通过现有 `headers` 字段注入;绑定层(Node/Python)在 options 里加
  `sessionId?: string` 便捷参数,自动填对应 provider 的 header
- 工作量:极小(常量表 + 绑定层一个字段)

**路径 B**:core 的 `CallOptions` 加 `session_id: Option<String>`,provider 侧自动注入。

- 优点:语义化、各语言绑定统一
- 缺点:header 名是 provider 专属知识,放 core 字段会造成"core 知道厂商头名"的耦合;
  且与 `headers` 重叠,两份入口易冲突(需定义合并优先级)

**结论**:路径 A 保持 core 纯净(厂商差异仍在 provider/绑定层表达,符合现有架构);
若后续发现 session 需要参与重试/粘性等**语义**,再升级为路径 B。

### 2.2 注入点

- Node/Python 绑定层:工厂函数/生成函数 options 增加 `sessionId`,映射到对应
  provider 的 header(查 §2.1 常量表)
- Rust 用户:直接 `with_headers`(现状已支持,无需改动)

### 2.3 行为契约

- `sessionId` 只做**透传**,不校验格式、不保证上游行为(上游缓存策略在厂商侧)
- 流式/非流式一致注入
- 与用户显式传的同名 header 冲突时:**用户显式 header 优先**(绑定层不覆盖)

---

## 3. 验收标准

- [ ] 常量表覆盖至少 3 个主流厂商/网关(openai 系、anthropic 系、openrouter)
- [ ] Node 绑定:`generateText({ sessionId })` 产出对应 header(cassette 断言)
- [ ] 与显式 headers 冲突时用户优先(测试)
- [ ] Rust 侧无 API 破坏(仅新增)

## 4. 参考资料

- 审计:`reference/audit/{cline,pi,llmgateway,dynamo,TokenHub,claude-code-router,axonhub,plano}.md`
- reference/audit/LEARNINGS.md 会话亲和三档形态表
