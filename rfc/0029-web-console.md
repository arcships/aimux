# RFC-0029: Web 控制台(aimux-web)—— 浏览器端的 model call 验证与 trace 可视化

> **Status**: DRAFT(设计定稿 2026-08-14;实施顺序见 §12)
> **Date**: 2026-08-14
> **Scope**: 新增 `tools/aimux-web` —— 本地 Web 服务(axum)+ 浏览器 SPA(Vue 3)。覆盖 Playground / Agent(前端 JS 驱动 loop)/ Traces / Sessions / Replay / Cache probe 六个页面,复用 RFC-0015 trace、RFC-0023 recording/replay、RFC-0024 session、RFC-0027 model catalogue
> **Related**: [RFC-0015](0015-cache-trace-audit.md)(cache trace)、[RFC-0023](0023-runtime-request-recording.md)(录制/回放)、[RFC-0024](0024-session-aggregation.md)(会话归组)、[RFC-0025](0025-aimux-cli-cache-probe.md)(CLI 三层拆分②)、[RFC-0027](0027-model-catalogue-and-list-api.md)(model catalogue)、[RFC-0014](0014-logging.md)(统一日志)

---

## 1. 背景与动机

aimux 现有两个命令行工具:`aimux-cli`(cache-probe,RFC-0025)与 `aimux-replay`(请求回放,RFC-0023 P4)。CLI 形态对开发者自己够用,但给使用者(验证 model call、看 trace)不直观:

- 验证各种 model call 需要改参数、重跑、看输出,CLI 每改一次参数都要敲一遍命令;
- agent 多步调用的 trace 只能读 jsonl,没有时间线/瀑布/对比视图;
- 回放、mock、缓存探测散在多个子命令里,缺乏交互式入口。

同时,aimux 在**观测与数据层已经齐备**:`Recording` 三层录制、`TraceLayer`/`RingTraceStore`/`verdict`、`SessionStore` 归组、`MockReplayModel`/`replay_with_model`/`rebuild_provider` 全部实现且经过测试。缺的只是**一个消费这些数据的交互式前端**。

### 1.1 定位边界

aimux 是 access layer(README 明确不做 agent loop / RAG / 编排)。**agent loop 属于工具层**——本工具是放 agent 循环的正确位置,不进 `aimux-core`。后端仅做"调用网关 + 录制",不内置编排逻辑。

### 1.2 与现有工具的关系

| 现有工具 | 本工具的处理 |
|---|---|
| `aimux-cli`(cache-probe) | **保留**给脚本/CI;交互式探测能力平移到 Cache probe 页 |
| `aimux-replay` | **保留**给脚本/CI;回放能力平移到 Replay 页,共用 `replay_with_model`/`rebuild_provider` |
| `bindings/*` | 无关,本工具直接用 Rust 核心,不走 FFI |

---

## 2. 设计目标

1. **交互式验证 model call**:选 provider/model、调参数、多轮消息、流式输出,一键重发。
2. **浏览器跑简单 agent loop**:循环逻辑在前端 JS 驱动(决策 D1),用户发消息,页面实时展示每步的模型调用/tool 调用/结果;agent 定义用 JSON,无需写 Rust。
3. **trace 可视化**:录制列表 + 单条三层详情(输入/provider 配置/HTTP exchanges)+ 会话瀑布图 + 双 Recording diff。
4. **最大化复用**:录制、trace、session、回放、mock、model catalogue 全部复用 core 现有实现,不重写。
5. **本地优先、凭据服务端托管**:默认绑 `127.0.0.1`;API key 支持网页内 Settings 保存(内存默认 + 显式 remember 落盘 `0600`,GET 仅掩码)或 `env:` 引用,明文永不回传前端,录制脱敏复用 RFC-0023 现成规则(见 §5.5)。

---

## 3. 已拍板决策

| 决策 | 结论 |
|---|---|
| **D1** agent loop 放哪 | **前端 JS**。循环状态在浏览器,Vue 组件驱动;每次 model call 走后端 `POST /api/calls`(带 `session_id`+`step`),后端负责调用 + 录制,不感知循环 |
| **D2** 前端技术栈 | **Vite + Vue 3 + TS + shadcn-vue + Tailwind CSS**(组件库 2026-08-14 定稿);类型由 ts-rs 从 Rust 生成,不手写;界面设计见 §8 |
| **D3** V1 范围 | **P1–P5 一次做完**(Playground / Agent / Traces / Sessions / Replay / Cache probe) |
| **D4** 凭据管理(**2026-08-14 修订**,替代原"凭据不落地前端") | **网页内 Settings + 服务端托管**:provider 下拉 + key 输入;内存为默认,显式勾选 remember 才落盘(配置目录 `keys.json`,`0600`);`GET /api/settings/keys` 只返回掩码(末 4 位),明文永不回传;非回环绑定时 PUT/DELETE 返回 403、请求内明文维持拒绝,回退 `env:` 引用。调用优先级:请求内显式 spec > Settings 已存 key > provider 注册 env var(§5.5) |

---

## 4. 架构

```
浏览器 (Vue 3 SPA)
 ├─ Playground 页 → POST /api/calls            ─┐
 ├─ Agent 页(loop 在前端 JS 里驱动)             │
 │   ├─ 每步     → POST /api/calls (session_id+step)  │
 │   ├─ tool 执行 → POST /api/tools/:name        ├→ Rust 后端 (axum, 127.0.0.1:port)
 │   └─ 消费 SSE StreamPart(文本/tool_call/Finish)│   ├─ generate_text / stream_text
 └─ Traces / Sessions / Replay / Cache 页        ─┘   ├─ RingRecorder + TraceLayer + SessionStore
                                                        └─ 每次调用自动落 Recording + TraceRecord
```

**核心分工**:后端 = 无状态模型调用网关 + 录制器;前端 = 交互、循环、可视化。后端进程内启动时接线:

```rust
// tools/aimux-web/src/state.rs(示意)
init_recording(Some(Arc::new(RingRecorder::default())));   // RFC-0023 P6,内存环形 2048
init_session_store(Arc::new(SessionStore::new()));          // RFC-0024
init_session_infer(false);                                   // 显式 session_id 为主
// TraceLayer 按调用包在 model 上(复用 provider.rs 的 build_model 思路)
```

### 4.1 后端模块

```
tools/aimux-web/
├── Cargo.toml            # 新增 axum / tower-http(静态服务)等依赖
└── src/
    ├── main.rs           # axum 启动:绑 127.0.0.1:随机端口,打印/自动打开 URL
    ├── state.rs          # 全局状态:录制/trace/session/model 缓存/agent 定义
    ├── wire.rs           # 前端消息 schema ↔ ModelPrompt 转换(§5.3)
    ├── agents.rs         # agent 定义 JSON 校验(§6.2)
    └── api/
        ├── mod.rs
        ├── calls.rs      # POST /api/calls,SSE 流
        ├── tools.rs      # POST /api/tools/:name(内置安全工具)
        ├── traces.rs     # Recording / TraceRecord 查询、导出/导入
        ├── sessions.rs   # 会话归组
        ├── replay.rs     # 请求回放 / mock 模式
        ├── cache_probe.rs# 在线缓存探测(平移 aimux-cli probe::provider)
        └── providers.rs  # provider/model 列表(model_catalogue)
```

### 4.2 前端结构

```
tools/aimux-web/web/
├── package.json / vite.config.ts
├── src/
│   ├── types/            # ts-rs 生成的 d.ts(§9.1)
│   ├── api/client.ts     # fetch + SSE reader 封装
│   ├── agent/engine.ts   # 前端 loop 引擎(§6.1)
│   ├── stores/           # Pinia:agent 运行状态、trace 缓存
│   ├── components/ui/    # shadcn-vue 生成的可定制组件(button/table/tabs…)
│   ├── components/       # 业务自研:瀑布图 / JSON 查看器 / diff;流式输出用 markstream-vue
│   └── views/
│       ├── Playground.vue
│       ├── Agent.vue
│       ├── Traces.vue
│       ├── Sessions.vue
│       ├── Replay.vue
│       ├── CacheProbe.vue
│       └── Settings.vue      # API key 管理(§5.5,决策 D4)
└── index.html
```

---

## 5. API 设计

### 5.1 端点总览

| 端点 | 方法 | 说明 |
|---|---|---|
| `/api/health` | GET | 存活检查 |
| `/api/calls` | POST | 单次 model call(generate/stream);`stream=true` 时响应为 SSE 流(§5.2) |
| `/api/tools/:name` | POST | 内置 tool 执行(calculator/echo/datetime/http_get 白名单) |
| `/api/traces` | GET | 录制列表(分页 + provider/model/session/状态过滤) |
| `/api/traces/:call_id` | GET | 单条 `Recording` 三层详情 |
| `/api/trace-records` | GET | cache `TraceRecord` 列表(verdict/命中率/usage) |
| `/api/sessions` | GET | 会话归组列表(`SessionStore::list_sessions`) |
| `/api/sessions/:id` | GET | 单会话调用链 |
| `/api/replay` | POST | `{call_id, overrides}` → `replay_with_model` 重发真实 API(返回新 `Recording` 供 diff) |
| `/api/mock/load` | POST | 从录制 jsonl 构造 `MockReplayModel`,切换离线 mock 模式 |
| `/api/recordings/export` | GET | 导出全部录制为 jsonl(与 `aimux-replay` 格式互通) |
| `/api/recordings/import` | POST | 导入 jsonl(RFC-0023 `from_jsonl` 兼容) |
| `/api/providers` | GET | provider 列表 + 可用模型(复用 RFC-0027 catalogue) |
| `/api/settings/keys` | GET | 已存 key 掩码列表(末 4 位)+ `plaintext_entry` 标志(**2026-08-14 修订新增**,§5.5) |
| `/api/settings/keys` | PUT | `{provider, key, remember?}` 保存 key(内存 + 可选落盘;非回环 403) |
| `/api/settings/keys/:provider` | DELETE | 移除该 provider 的 key(内存 + 磁盘;非回环 403) |
| `/api/cache-probe` | POST | 在线缓存探测(平移 `aimux-cli probe::provider`,含 `--dry-run` 等价语义) |

### 5.2 SSE 协议(`POST /api/calls?stream=1`)

流式响应按 `StreamPart`(已有 `#[ts(export)]`,见 [stream_part.rs](../aimux-core/src/stream_part.rs))逐条转发:

```
event: stream_part
data: {"TextDelta":{"id":"...","delta":"..."}}

event: stream_part
data: {"ToolCall":{"tool_call_id":"...","tool_name":"web_search","input":{...}}}

event: stream_part
data: {"Finish":{"finish_reason":"tool_calls","usage":{...},"provider_metadata":null}}

event: meta
data: {"call_id":"call-...","session_id":"sess-...","step":3,"outcome":"success"}
```

`meta` 事件是前端 trace 锚点:每次调用结束后带出 `call_id`/`session_id`/`step`,前端据此把运行中的 agent 步骤与 Traces 页的 Recording 对上。

非流式请求(`stream=false`)直接返回 `{result, meta}`。

### 5.3 wire schema(前端消息格式)

前端不碰 Rust 类型,后端 `wire.rs` 把下列 schema 映射为 `LanguageModelPrompt` / `CallOptions`:

```jsonc
POST /api/calls
{
  "provider": "openai",            // 注册表名或原生 provider 名
  "model": "gpt-4o",
  "api_key": "env:OPENAI_API_KEY", // env:VAR 引用 / 留空(Settings 已存 key 或 provider env var)/ 明文仅限回环绑定(§5.5)
  "base_url": null,
  "stream": true,
  "options": {
    "temperature": 0.7,
    "max_output_tokens": 1024,
    "tools": [                      // 与前端 agent 共用同一 tool 定义格式
      { "name": "calculator", "description": "...", "parameters": { /* JSON Schema */ } }
    ]
  },
  "session_id": "sess-xxx",         // RFC-0024,多步关联
  "step": 3,                        // 会话内步号,前端自增
  "messages": [
    { "role": "system", "content": [{ "type": "text", "text": "..." }] },
    { "role": "user",  "content": [{ "type": "text", "text": "..." }] },
    {
      "role": "assistant",
      "content": [{ "type": "text", "text": "..." }],
      "tool_calls": [{ "id": "call_1", "name": "calculator", "input": { "expr": "1+1" } }]
    },
    { "role": "tool", "tool_call_id": "call_1",
      "content": [{ "type": "tool_result", "result": { "value": 2 }, "is_error": false }] }
  ]
}
```

映射规则:
- `system/user/assistant/tool` → `ModelMessage`,`content` 数组映射 `ContentPart`(text / image / tool_result…);
- `tool_calls` 按 provider 能力转 `ToolChoice`/消息内嵌 tool call(OpenAI 兼容族);
- `api_key` 解析见 §5.5:显式 `env:VAR` 引用、Settings key store 回退、回环绑定下的明文直传;**浏览器永远见不到 key 明文**(Settings 页只显示掩码)。

### 5.4 多步关联(前端 loop 与 trace 的桥梁)

- 前端在 agent 运行时,每次 `POST /api/calls` 都带**同一个** `session_id` + 自增 `step`;
- 后端 `CallOptions.session_id` 透传(RFC-0024),`SessionStore.append` 归组,`RingRecorder` 落 `Recording`(`session_id`/`step` 字段由 RFC-0023 P1 已实现);
- Traces 页按 `session_id` 过滤即得**整个 agent 运行的调用链**,按 `step` 排序渲染瀑布图;
- 单条失败/异常调用不影响会话归组(录制在入口发生,失败也在会话内)。

### 5.5 Settings 与凭据(决策 D4,2026-08-14 修订)

原设计只接受 `env:VAR` 引用,用户必须先在 shell `export`——对"下载即用"的预编译二进制不友好。修订为**网页内设置**(Settings 页):

- **存储**(`settings.rs` `KeyStore`):每 provider 一把明文 key,内存 `HashMap` 为默认;勾选 "remember" 才落盘到配置目录(Linux/macOS `~/.config/aimux-web/keys.json`,Windows `%APPDATA%/aimux-web/keys.json`),文件权限 `0600`(unix `PermissionsExt`,Windows 尽力而为),启动时若文件存在则加载进内存。同 provider 重新保存但不勾 remember 时,磁盘旧条目同步移除(避免重启复活旧 key)。
- **API 面**(见 §5.1):`GET` 只返回 `{provider, status, hint}`(hint = 末 4 位,不足 4 位给长度),**明文永不回传**;`PUT`/`DELETE` 非回环绑定时返回 403。
- **回环门控**:后端启动时已知 bind host;非 `127.0.0.1`/`localhost`/`::1` 时 Settings 写接口 403(错误信息引导 `env:` 引用),`GET` 附 `plaintext_entry: false` 供前端隐藏输入框,请求内明文 spec 亦维持拒绝。
- **调用优先级**(`wire.rs` `resolve_api_key`,三个调用方 calls/cache-probe/replay 均带上 provider 名):
  1. **请求内显式 spec** —— `env:VAR` 引用(任意绑定可用);明文仅限回环绑定(Playground 快速用一次、不保存的场景);
  2. **Settings 已存该 provider 的 key**(KeyStore,含启动时从磁盘加载的);
  3. `None` —— provider 读自己注册的默认 env var。

**威胁模型**:信任边界 = 本机回环客户端。回环下浏览器 ↔ 后端信道视为可信,故允许网页内明文录入与请求内明文直传;非回环(如 `--host 0.0.0.0` LAN 共享)下任何局域网客户端都能 POST,故全部明文入口关闭。明文 key 永不进日志、永不回传前端、永不进 Recording(录制层脱敏复用 RFC-0023 规则)。

---

## 6. 前端 agent loop(决策 D1)

### 6.1 loop 引擎(`src/agent/engine.ts`)

前端驱动,纯 TS 状态机,不依赖后端编排:

```
runStep(messages, step):
  1. sse = POST /api/calls { messages, tools, session_id, step }
  2. 消费 StreamPart:
       TextDelta   → assistantText += delta(渲染)
       ToolCall    → pendingToolCalls.push({id, name, input})
       Finish      → 记录 usage / finish_reason
       Error       → 终止,标记失败
  3. 无 pendingToolCalls(或 finish_reason = stop)→ 结束
  4. 有 tool call 且 step < max_steps:
       results = await Promise.all(tools.map(t =>
                    POST /api/tools/{t.name} { input: t.input, tool_call_id: t.id }))
       messages.push(assistant(tool_calls))
       messages.push(...tool_results(results))
       return runStep(step + 1)
```

约束:循环上限 `max_steps`(默认 8);`stop` finish_reason 或"无 tool call"即终止;用户可随时中断(中止底层 fetch/SSE)。

### 6.2 agent 定义(JSON,免写 Rust)

UI 里可编辑,存 `localStorage` 或导出/导入 JSON:

```jsonc
{
  "name": "calculator-agent",
  "system_prompt": "You are a helpful calculator assistant. Always use tools.",
  "model": { "provider": "openai", "model_id": "gpt-4o", "api_key": "env:OPENAI_API_KEY" },
  "tools": ["calculator", "datetime"],
  "max_steps": 8,
  "temperature": 0.0
}
```

工具从服务端内置清单(§6.3)勾选;工具定义(JSON Schema)与模型调用共享同一份,后端 `wire.rs` 负责把工具 schema 注入 `CallOptions.tools`。

### 6.3 tool 执行桥(`POST /api/tools/:name`)

内置安全工具集,后端实现(不依赖 LLM):

| tool | 输入 | 说明 |
|---|---|---|
| `calculator` | `{expr}` | 算术表达式求值(自研解析器,不支持任意代码执行) |
| `datetime` | `{tz?}` | 当前时间 |
| `echo` | `{text}` | 原样返回(测循环用) |
| `http_get` | `{url}` | **白名单域名/协议受限**,防 SSRF;默认关闭 |

tool 调用本身也进录制上下文(用与 RFC-0023 §10 open-question 7 的 `composite_step` 类似语义标注 `tool:calculator`),便于在 trace 里看到工具入参出参。

---

## 7. trace 可视化

### 7.1 列表(Traces 页)

| 列 | 来源 |
|---|---|
| 时间 | `Recording.recorded_at` |
| provider / model | `Recording.provider.*` |
| session / step | `Recording.session_id` / `step` |
| 延迟 | `exchanges[..].timing.latency_ms`(含 TTFB) |
| token / 命中率 | `TraceRecord.usage` + `reported_hit_rate()` |
| 状态 | `Recording.outcome.status`(success/error/incomplete/cancelled) |
| verdict | `TraceRecord.verdict`(RFC-0015) |

支持按 provider / session / 状态过滤与搜索 call_id。

### 7.2 单条详情(三层 + 瀑布)

左侧**瀑布图**:agent 会话里每个 `step` 一条 bar(开始 → TTFB → 完成),tool 调用嵌套子条;失败红色、重试(attempt>0)标注。右侧 **tab 切换**:

1. **输入**(`Recording.input`):消息列表 + options(temperature/tools/headers…);
2. **Provider 配置**(`Recording.provider`):base_url / api_key_source / profile / provider_options;
3. **HTTP exchanges**(`Recording.exchanges`):per-attempt 原始 request/response body(JSON 语法高亮、可折叠),敏感字段显示 `[REDACTED]`,含 timing/error。

### 7.3 diff(重放对比)

Replay 页对同一 `Recording` 重发真实 API 后,新旧两条 `Recording` **并排 diff**:输入完全相同(高亮 overrides 改动),输出/usage/延迟差异高亮。复用 `replay_with_model` + `ReplayOverrides`(改 prompt/temperature/max_output_tokens)。

---

## 8. 界面设计(UI Design)

> 2026-08-14 补充:组件库选型定稿为 **shadcn-vue + Tailwind CSS**(决策 D2 补充,§3)。

### 8.1 设计基调

- **布局**:左侧窄导航栏(图标 + 文字,六页切换)+ 主内容区;单页应用,`hash` 路由,无路由刷新。
- **主题**:默认深色(dev 工具惯例),Tailwind CSS 变量令牌(shadcn 主题化),右上角亮/暗切换。
- **视觉风格**:shadcn 默认风格(圆角、细边框、zinc 色板),数据密集型页面用紧凑密度。
- **流式 Markdown 渲染**:`markstream-vue`(Vue 3 流式 Markdown 渲染器,适配 SSE/token 流、未完成 Markdown、流式代码块、Mermaid/KaTeX)。Playground 与 Agent 的助手输出用它渲染:`<MarkdownRender mode="chat" :content="accText" :final="done" />`,流式中 `final=false`、结束后 `true`。
- **代码/JSON 高亮**:shiki(按需加载 `json`/`shell` 语言,内置 dark 主题),用于 trace body 查看与 agent 定义编辑。
- **自研组件**(组件库不提供、业务定制):
  - `Waterfall.vue` — 会话瀑布图(SVG,步骤 bar + TTFB 标注 + 失败/重试着色);
  - `JsonViewer.vue` — 折叠树 + `[REDACTED]` 高亮 + 复制按钮;
  - `DiffPane.vue` — 双 Recording 并排 diff。

### 8.2 组件映射(shadcn-vue)

| 需求 | 组件 |
|---|---|
| provider/model 选择(带搜索) | `Command`(combobox)+ `Popover` |
| 参数表单 | `Form` / `Label` / `Input` / `Select` / `Textarea` |
| 滑块 / 开关 | `Slider` / `Switch` |
| 三层查看器 tab | `Tabs` |
| 列表 / 过滤栏 | `Table` + `Input` / `Select`(过滤逻辑自写,轻量) |
| 状态徽标 / 标签 | `Badge` / `Separator` |
| 重放确认 / 导入弹窗 | `Dialog` / `Alert` |
| 步骤折叠、JSON 折叠 | `Collapsible` / `ScrollArea` |
| 提示 / 悬浮说明 | `Sonner`(toast)/ `Tooltip` |
| 运行/停止按钮、骨架加载 | `Button` / `Skeleton` |
| 流式 Markdown 输出(助手消息) | `markstream-vue` `MarkdownRender`(第三方,非 shadcn;§8.1) |

### 8.3 页面线框

**Playground**(单次 model call 验证)

```
┌──────────┬───────────────────────────────────────────┬─────────────┐
│ ▦ 导航    │ [openai ▾][gpt-4o ▾][env:…▾][base_url]   │ ▸ 参数        │
│ Playground│ [stream ⚡]            [运行] [停止]      │ temperature  │
│ Agent    │──────────────────────────────────────────│ ─────●──    │
│ Traces   │ [user] 请解释 Rust 所有权                 │ max_tokens   │
│ Sessions │ [assistant] Rust 的所有权……(markstream) │ [1024]       │
│ Replay   │ [assistant·tool] calculator → 2          │ tools        │
│ Cache    │                                          │ ☑calculator  │
│          │ ┌─────────────────────────────────────┐  │ ☐http_get    │
│          │ │ 输入消息…                [发送]      │  │ response fmt │
│          │ └─────────────────────────────────────┘  │ view trace → │
└──────────┴───────────────────────────────────────────┴─────────────┘
```

布局:左导航 | 中间消息区(多轮,底部输入框) | 右侧参数面板(可折叠)。每次调用结束,消息右上角出现 `view trace →` 跳转 Traces 详情。

**Agent**(前端 JS 驱动 loop)

```
┌──────────────────────────────────────────────────────────┬─────────────┐
│ Agent · calculator-agent              [运行][停止][重置]   │ Agent 定义   │
├──────────────────────────────────────────────────────────┤ name        │
│ [user] 17*19=?                                           │ [txt]       │
│ [assistant] 我用工具算一下                                │ system_prompt│
│ ┌ tool: calculator ───────────────────┐                  │ [textarea]  │
│ │ expr: "17*19" → {value:323}         │                  │ model ▾ ▾   │
│ └─────────────────────────────────────┘                  │ tools ☑☐   │
│ [assistant] 17×19 = 323                                  │ max_steps 8 │
│                                                          │ temp 0.0    │
│ ▶ 步骤时间线(sidebar 底部)                                │ 导出定义 JSON│
│  step0 ✓ tool·calc  323ms  0.9k                          │─────────────│
│  step1 ✓ 答复       412ms  0.7k                          │ trace 链接   │
└──────────────────────────────────────────────────────────┴─────────────┘
```

主区 = 会话流(用户/助手消息 + tool 卡片,可折叠看入参出参);右侧 = agent 定义表单 + 步骤时间线;每步旁 `trace` 链接直达该步 Recording。运行中当前步高亮、助手输出用 markstream-vue 流式渲染。

**Traces**(录制列表 + 三层详情)

```
┌──────────────────────────────────────────────────────────────┐
│ Traces  [provider▾][session▾][status▾][🔍call_id]  [导出jsonl] │
├──────────────────────────────────────────────────────────────┤
│ 时间       provider   model      session step  延迟  tok 状态 │
│ 14:32:01  openai     gpt-4o     s-abc   2   412ms 1.1k ✓    │
│ 14:32:00  openai     gpt-4o     s-abc   1   323ms 0.9k ✓    │
│ 14:31:59  openai     gpt-4o     s-abc   0   301ms 1.2k ✓    │
│ 14:31:10  deepseek   deepseek…  s-def   0   810ms 2.3k ✗    │
├─────────────────────────────┬────────────────────────────────┤
│ 瀑布图(session 聚合)         │ Tabs:[输入][Provider][HTTP]    │
│ step0 ████────  TTFB 120ms   │ messages… / options…          │
│ step1 ████─────              │ attempt0 req 202 → resp 200   │
│ step2 ████──                 │ JsonViewer(shiki + REDACTED)  │
└─────────────────────────────┴────────────────────────────────┘
```

点行展开详情:左瀑布(该 session 全部 step),右三层 tab;HTTP tab 内 per-attempt 请求/响应用 `JsonViewer` 渲染,敏感字段黄色高亮 `[REDACTED]`。

**Sessions**

```
┌─────────────────────────────┬──────────────────────────────────┐
│ s-abc     explicit · 3 calls│ 调用链 s-abc                      │
│ s-def     explicit · 1 call │ step0 ✓ 301ms 1.2k  verdict pass  │
│ auto-9f…  inferred · 5 calls│ step1 ✓ 323ms 0.9k  verdict pass  │
│                             │ step2 ✓ 412ms 1.1k  verdict …     │
│                             │ (点击任一 step → Traces 详情)      │
└─────────────────────────────┴──────────────────────────────────┘
```

左侧会话列表(来源徽标 explicit/inferred + 调用数),右侧链视图;verdict 颜色编码(绿 pass / 黄 suspect / 红 fail)。

**Replay**(重放 + diff)

```
┌──────────────────────────────────────────────────────────────┐
│ Replay [录制▾][改prompt][temp][max_tokens] [重放→][⚠确认费用]   │
├────────────────────────────────┬─────────────────────────────┤
│ 原 Recording · call-xxx         │ 新 Recording · call-yyy      │
│ 输入(overrides 高亮)            │ 输入(同)                     │
│ 输出 / usage 1.2k+0.9k          │ 输出 / usage 1.3k+0.9k(差异) │
│ 延迟 301ms                      │ 延迟 320ms                   │
└────────────────────────────────┴─────────────────────────────┘
```

顶部重放表单 + 费用确认(`Dialog`);下方 `DiffPane` 双列对比,差异行高亮。

**CacheProbe**(缓存探测)

```
┌──────────────────────────────────────────────────────────────┐
│ Cache Probe [provider▾][model▾][key▾][base_url][dry-run☐]    │
│             [max_requests 4][prompt 覆盖][运行→]              │
├──────────────────────────────────────────────────────────────┤
│ 汇总卡片: 命中率 82% · verdict 分布 · 平均延迟 507ms           │
│ #  cache_read  input  output  verdict   TTFB   延迟           │
│ 0  0          1024   128     no_data  812ms  812ms           │
│ 1  1024       1024   96      pass     120ms  410ms           │
│ 2  2048       2048   110     pass     118ms  405ms           │
└──────────────────────────────────────────────────────────────┘
```

表单 + 汇总卡 + 请求明细表,逻辑平移 `aimux-cli probe::provider`。

---

## 9. 复用与新增

### 9.1 ts-rs 类型导出(新增)

| 类型 | 现状 | 动作 |
|---|---|---|
| `StreamPart` | 已有 `#[ts(export)]` | 无需改 |
| `TraceRecord` / `UsageSnapshot` / `Verdict` 系 | 已有 `#[ts(export)]` | 无需改 |
| `SessionView` / `SessionCall` | 已有 `#[ts(export)]` | 无需改 |
| `Recording` 系(`InputRecord`/`ProviderRecord`/`HttpExchange`/`HttpRecord`/`ResponseRecord`/`TimingRecord`/`OutcomeRecord`/`OutcomeStatus`) | **只有 serde,无 ts** | 加 `#[ts(export)]`(与现有 trace 类型同模式) |
| wire schema 类型(§5.3 请求/响应) | 新建于 `tools/aimux-web` | `#[ts(export)]` |

生成的 d.ts 输出到 `tools/aimux-web/web/src/types/`(复用 `bindings/node` 的 ts-rs 导出管线方式,`TS_RS_EXPORT_DIR` 指向 web 目录)。

### 9.2 复用的 core 能力(不改 core)

| 能力 | 复用点 |
|---|---|
| 录制 | `init_recording` + `RingRecorder` + `Recording` 数据模型 |
| 回放 | `replay_with_model` + `ReplayOverrides` + `rebuild_provider` |
| mock | `MockReplayModel` + `ScoreMatcher`/`ExactMatcher`/`PrefixMatcher` + `from_jsonl` |
| cache trace | `TraceLayer` + `RingTraceStore` + `TraceFilter`/`TraceStats` + `RuleAuditor` |
| 会话 | `SessionStore` + `resolve_session_id` |
| provider 构造 | 平移 `aimux-cli` `probe::provider::build_model` 的 native/registry 双路径 |
| model 列表 | RFC-0027 `model_catalogue` |

### 9.3 新增依赖(仅 `tools/aimux-web`)

后端:axum、tower-http(静态文件/SSE/CORS-localhost)、可选 `open`(自动开浏览器,不引入则打印 URL)。前端:Vue 3、Pinia、**shadcn-vue + Tailwind CSS(reka-ui 原语,按需拷贝组件进仓库)**、**markstream-vue(流式 Markdown 渲染,§8.1)**、shiki(代码/JSON 高亮);瀑布图与 JSON 查看器为自研组件(§8.1)。

---

## 10. 安全

| 维度 | 设计 |
|---|---|
| 绑定 | 默认 `127.0.0.1`;`--host 0.0.0.0` 需显式开启(LAN 共享用,文档警示) |
| API key | 网页内 Settings 保存(内存默认 + 显式 remember 落盘 `0600`)或 `env:VAR` 引用;`GET` 仅掩码,**明文永不回传浏览器**;非回环绑定关闭一切明文入口(403 / 拒绝),回退 `env:` 引用(§5.5,决策 D4) |
| 录制脱敏 | 复用 RFC-0023 现成规则:api_key/Authorization/含 api-key 头恒 `[REDACTED]` |
| `http_get` 工具 | 白名单域名 + 仅 http/https,防 SSRF;默认关闭 |
| 请求回放 | 重发真实 API 消耗费用,UI 明确警示 + 需用户确认(等价 CLI `--dry-run` 的精神) |
| 前端依赖面 | 最小化:shadcn-vue 组件按需拷贝进仓库、npm 依赖锁定;shiki 按需加载语言 |

---

## 11. Scope of Changes

| 位置 | 改动 | 工作量 |
|---|---|---|
| `tools/aimux-web/Cargo.toml` + `src/main.rs` + `src/state.rs` | 新 crate:axum 服务 + 静态资源 + 录制/trace/session 接线 | ~300 行 |
| `tools/aimux-web/src/wire.rs` | 前端消息 schema ↔ `ModelPrompt`/`CallOptions` 转换 + 工具 schema 注入 | ~250 行 |
| `tools/aimux-web/src/api/*` | 6 组端点(§5.1)+ SSE 转发 + `meta` 事件 | ~600 行 |
| `tools/aimux-web/src/agents.rs` | agent 定义 JSON 校验 + 内置工具集(calculator/datetime/echo/http_get) | ~200 行 |
| `aimux-core/src/recording.rs` 等 | `Recording` 系类型补 `#[ts(export)]`(9.1) | ~10 行 |
| `tools/aimux-web/web/**` | Vue 3 + shadcn-vue SPA:6 页面 + agent 引擎 + 瀑布/JSON/diff 组件 + markstream-vue + Pinia | ~2500 行 |
| ts-rs 导出管线 | 生成 d.ts 到 `web/src/types/` | ~30 行(脚本) |
| 测试 | wire schema 往返、SSE 转发、agent 引擎(离线 fixture)、tool 白名单、回放/diff | ~400 行 |
| 文档 | README 段落 + docs/api 指引 | ~50 行 |

**合计:~4300 行。core 仅补 `#[ts(export)]` 注解,零行为改动;CLI 工具保留不动。**

---

## 12. 实施顺序(V1 一次做完)

| 阶段 | 内容 | 依赖 |
|---|---|---|
| **P1** | `aimux-web` 骨架(axum + 静态服务 + `/api/calls` SSE + `wire.rs`)+ Playground 页 | 无 |
| **P2** | `state.rs` 接线(RingRecorder/TraceLayer/SessionStore)+ Traces 列表/详情/三层查看器 | P1 |
| **P3** | Agent 页 + `engine.ts` loop 引擎 + `agents.rs` 定义 + `/api/tools/*` | P1 |
| **P4** | Replay / mock / Cache probe 页 + diff 对比 | P2 |
| **P5** | Sessions 页、jsonl 导入导出、瀑布图打磨、ts-rs 导出收尾、文档 | P2-P4 |

---

## 13. 风险与缓解

| 风险 | 等级 | 对策 |
|---|---|---|
| 前端 loop 与 provider tool 语义差异(OpenAI-compat vs 原生) | 中 | `wire.rs` 收敛 tool call 映射;V1 以 OpenAI 兼容族为主(与 mock 回放范围一致),原生 provider 逐步补 |
| 前端消息 schema 与 `ModelPrompt` 往返丢字段 | 中 | wire 往返单测(round-trip)覆盖;未知字段显式报错而非静默丢弃 |
| SSE 中断 / 浏览器 fetch reader 兼容 | 低 | fetch streaming + TextDecoder 解析;断流重试 + 失败态展示 |
| key 误入前端(用户手填明文) | 低 | Settings 页与请求内明文仅回环可用;`GET` 只回掩码;非回环一律拒绝并提示 `env:VAR` |
| `http_get` SSRF | 中 | 白名单 + 协议限制,默认关闭 |
| 请求回放消耗真实费用 | 中 | UI 确认弹窗 + 每请求显示预估 token;与 CLI `--dry-run` 精神一致 |
| 前端体量膨胀 | 中 | shadcn-vue 组件按需拷贝进仓库(不用即删);业务组件(JSON 查看器/瀑布/Schema form)复用;shiki 按需加载语言 |

---

## 14. Open Questions

1. **agent 定义是否要支持"客户端自定义 tool"(浏览器 JS 里注册)?** V1 只做服务端内置工具集;客户端 tool 需把执行代码上传到后端执行,安全面大,V1 不做。
2. **录制持久化**:V1 用 `RingRecorder` 内存环形(重启即清),导出 jsonl 由用户手动;是否要本地自动持久化目录(复用 `aimux-replay` 的 jsonl 格式)在 P5 评估。
3. **多用户/鉴权**:默认单机信任(127.0.0.1);如需 LAN 共享,是否加简单 token 鉴权在后续版本评估。
4. **瀑布图粒度**:V1 按 `step`(每次 model call)一条 bar;是否下沉到 `StreamPart` 粒度(每 delta)在 P5 打磨时评估。
5. **provider 覆盖**:V1 以 OpenAI 兼容族(注册表)+ 原生单 key provider(openai/anthropic/google/mistral/xai/cohere)为主;azure/bedrock/vertex 需要额外参数,沿用 `aimux-cli` 的处理(暂不直接支持,可用兼容镜像)。
