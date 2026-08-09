# p4：v0.2.1 → HEAD 发布范围代码 Review（双模型：gpt-sol + glm-5.2）

> **快照声明**: 本报告是 **2026-08-07** 的时间点快照，基于 v0.2.1 tag 到当前 HEAD 的代码状态审查。
> 代码演进后部分发现可能过期；可追踪的工作项以 GitHub Issues 为准，本报告作为诊断参考归档。
>
> **日期**: 2026-08-07
> **方法**: 先界定变更范围（106 commits / 456 文件 / +35.3K 行），再派 5 个独立 agent 分区并行审查。
> 模型分配：**gpt-5.6-sol**（架构敏感区：录制/回放/会话核心、core 管线+providers、CLI 工具）与
> **glm-5.2**（大文件/长文档区：trace/openai 输出/模型目录、FFI+全语言绑定）。
> 每区独立读 `git diff v0.2.1..HEAD` 与当前源码，统一按**架构状况 / 代码整洁度 / 边界遵守情况 / 模块抽象正确度**
> 四维审查，产出 findings 后由主进程汇总并亲自复核关键项。
>
> **验证基线**: 各代理分别执行 `cargo check -p aimux-core/-p aimux-provider-utils/-p aimux-providers/-p aimux-ffi/-p aimux-cli/-p aimux-replay`
> 全部通过；`cargo test -p aimux-providers --test list_models_test` 22/22 通过；
> `gen_provider_names.py --check` 通过（8 文件 / 251 名称）；`git diff --check` 无 whitespace error。

---

## 1. 审查范围与分区

| 分区 | Agent | 模型 | 覆盖文件 |
|---|---|---|---|
| A. 录制/回放/会话核心 | Mason | gpt-5.6-sol | core/recording.rs、replay.rs、session.rs + provider-utils/http.rs + 相关测试 |
| B. trace/缓存探测/openai 输出/模型目录 | Zephyr | glm-5.2 | core/trace/*、openai_output.rs、model_catalogue.rs + 测试 |
| C. core 管线改造 + providers config_snapshot | Nova | gpt-5.6-sol | core/error、generate、provider、options、types 等 + providers/src、tests |
| D. FFI + 全语言 bindings 透传面 | Iris | glm-5.2 | aimux-ffi + bindings/{node,python,go,swift,kotlin,java,flutter,c} |
| E. CLI 工具 + workspace 基建 | Quill | gpt-5.6-sol | tools/aimux-cli、aimux-replay + scripts + Cargo/rustfmt |

涉及 RFC：0015（缓存探测）、0016（对齐 AISDK）、0023（录制/回放）、0024（session 归组）、0025（CLI 探测）、0026（OpenAI 兼容输出）、0027（模型目录/list API）。

---

## 2. 严重度总览

> 第一轮后追加第二轮审计（4 个 agent：原生 provider 深挖 / API·semver·文档 / unsafe·panic 安全 / 未覆盖模块+抽查验证）。
> 第二轮全部关键声明已由主进程复核。第一轮抽查验证结果：**8 个抽查 major 全部属实，无误报**。
> 第三轮再追加 4 个 agent（8 语言 binding 逐语言 / 并发·异步·性能 / 四大文件对抗性逻辑 / RFC 状态+生成文件），
> 发现 1 个新 blocker（Kotlin binding 无法编译）与 13 个新 major，并复核第二轮 5 组结论（B3/R2/R5/R10/R7/R8 全部属实，M2b 一处子论断需修正）。详见 §11。

| 级别 | 数量 | 摘要 |
|---|---|---|
| 🔴 blocker | **4** | ① URL query 凭据明文入库 ② Python `get_model_specs` NameError ③ 版本仍 0.2.1 但含破坏性变更且 `[Unreleased]` 为空 ④ **Kotlin Model.kt 源码结构损坏无法编译** |
| 🟠 major | **32** | matcher 正确性、内存有界失效、FFI runtime 重入、快照身份丢失、契约漂移、max_retries 静默忽略、STS 令牌泄漏、LCP 跨记录拼接、流式 id 中途变化、replay 丢 tool calls、binding 透传断层、close 竞态等 |
| 🟡 minor | ~50 | 契约/文档漂移、跨语言不一致、脱敏规则分叉、错误模型半结构化、热路径性能、SSE 标准边界 |
| 🔵 nit | ~20 | 死字段、注释误导、O(n²)、时间格式、trailing whitespace |

---

## 3. Blocker（均已主进程复核确认）

### B1. [blocker] HTTP 录制把 URL query 中的 API key 明文入库
- **位置**: [`aimux-provider-utils/src/http.rs:986`](../..//aimux-provider-utils/src/http.rs#L986)（`to_http_record`）
- **问题**: headers 经 `is_sensitive_key` 脱敏，但 `url: request.url.clone()` 原样保存整条 URL（含 `?key=...`/`?api_key=...`）。RFC-0023 将"api_key 永不录明文"定义为强制安全要求。
- **佐证**: 仓库日志侧已有 `request_url_no_query`（承认 query 可能含 key），录制侧未用等价脱敏。
- **修复方向**: 解析 URL 后按敏感键脱敏 query value；解析失败至少丢弃整个 query；保留非敏感 query。

### B2. [blocker] Python `get_model_specs()` 必抛 NameError
- **位置**: [`bindings/python/python/aimux/__init__.py:163`](../..//bindings/python/python/aimux/__init__.py#L163)
- **问题**: 函数调用 `_native_get_model_specs(source_url)`，但 import 块（L10-50）从未导入该名（原生函数存在并已注册 `lib.rs:827`）。已 export 进 `__all__`（L67）、有文档（L156），却完全不可用；`bindings/python/tests/` 零覆盖导致漏网。
- **修复方向**: import 块补 `get_model_specs as _native_get_model_specs`，并补离线测试。

---

## 4. Major findings 汇总

### 4.1 录制/回放/会话核心（分区 A）

| # | 级别 | 问题 | 位置 |
|---|---|---|---|
| A1 | major | **`ScoreMatcher` 可命中完全无关请求**：temperature 一致即 +1 分，prompt 相关性为 0 也能命中；RFC 规定零分不命中 | [replay.rs:132-163](..//aimux-core/src/replay.rs#L132) |
| A2 | major | **空 prompt 触发 `usize` 下溢**：`min(len) - 1`，debug panic / release wrap 到 usize::MAX，行为不一致 | [replay.rs:142-145](..//aimux-core/src/replay.rs#L142) |
| A3 | major | **`RingRecorder` 并非真正有界**：容量只约束 completed，pending HashMap 无上限/超时/淘汰/incomplete 导出 | [recording.rs:674-691](..//aimux-core/src/recording.rs#L674) |
| A4 | major | **`JsonlRecorder` 无界 channel**：`mpsc::channel` 无队列上限/丢弃策略/积压指标，writer 落后则内存无限增长，违背 RFC-0023 §3.5 | [recording.rs:404-435](..//aimux-core/src/recording.rs#L404) |
| A5 | major | **非 2xx 响应丢失结构化 response**：已收到合法 HTTP 响应仍记 `response: None`，status/headers/retry-after/body 只拼进 error 字符串 | [http.rs:1019-1035](..//aimux-provider-utils/src/http.rs#L1019) |
| A6 | major | **流式录制按网络 chunk 逐个 lossy decode**：UTF-8 多字节字符跨 chunk 边界被拆成 `�`，wire 内容失真、SSE 可能损坏 | [http.rs:503-507](..//aimux-provider-utils/src/http.rs#L503) |
| A7 | major | **Mock replay 未守"非 OpenAI 协议返回 Unsupported"**：未知格式降级为文本/Raw + 人工补 Finish，与模块文档自述矛盾 | [replay.rs:389-403](..//aimux-core/src/replay.rs#L389)、[:472-487](..//aimux-core/src/replay.rs#L472) |
| A8 | major | **`ExactMatcher` 名不副实**：只比 provider/model_id/prompt，忽略 temperature/tools/response_format/seed/max_output_tokens 等全部生成参数 | [replay.rs:41-75](..//aimux-core/src/replay.rs#L41) |
| A9 | major | **JsonlRecorder 初始化/写入失败被静默吞掉**：目录创建、文件打开、send、序列化、flush 错误大多忽略，调用方以为录制开启实际无落盘 | [recording.rs:415/543/431/651](..//aimux-core/src/recording.rs#L415) |
| A10 | minor | `recorded_at` 手拼 Duration debug 串，非 ISO8601/RFC3339，与 session 模块时间格式不一致 | [recording.rs:660-667](..//aimux-core/src/recording.rs#L660) |
| A11 | minor | replay token 计数 `as u64 as u32` 静默截断（外部可加载数据） | [replay.rs:334-360](..//aimux-core/src/replay.rs#L334) |
| A12 | minor | `SessionCall.step` u32 达到上限后无限重复 `u32::MAX`，非严格单调 | [session.rs:152-157](..//aimux-core/src/session.rs#L152) |
| A13 | minor | `HttpRequest.call_id` 与 `recording_context.call_id` 双关联源，实际只读后者，可能不一致 | [http.rs:204-209](..//aimux-provider-utils/src/http.rs#L204) |
| A14 | nit | RingRecorder 注释写 drop-newest，实现是 drop-oldest | [recording.rs:690-691](..//aimux-core/src/recording.rs#L690) |

### 4.2 trace / openai 输出 / 模型目录（分区 B）

| # | 级别 | 问题 | 位置 |
|---|---|---|---|
| B1 | major | **scope_key 未按 RFC §6 纳入 base_url/api_key**：仅用 model_id；同进程内两个 TraceLayer 包装不同 provider/key 但 model_id 相同且共享 store 时会串域，产生伪前缀匹配 | [layer.rs:161](..//aimux-core/src/trace/layer.rs#L161)、[:116](..//aimux-core/src/trace/layer.rs#L116) |
| B2 | minor | `convert_usage`（providers）与 `usage_to_openai`（core）跨 crate 逆映射不对称：Moonshot 顶层 `cached_tokens` 往返被移到嵌套字段，无顶层字段往返测试 | [model.rs:138](..//aimux-providers/src/openai/model.rs#L138) ↔ [openai_output.rs:920](..//aimux-core/src/openai_output.rs#L920) |
| B3 | minor | 单 Mutex 替代 RFC §6 的 16 片分片锁，异 scope 不并行；`evict_oldest_in_scope` O(cap) | [store.rs:323](..//aimux-core/src/trace/store.rs#L323)、[:147](..//aimux-core/src/trace/store.rs#L147) |
| B4 | minor | 未实现 RFC §6 两阶段占位记录，并发在途调用互不可见（判定正确性无伤，属并发可视性简化） | [layer.rs:422](..//aimux-core/src/trace/layer.rs#L422) |
| B5 | minor | 自定义（非 RingTraceStore）sink + auditor 时 LCP lookup 恒空、`first` 恒 true，判定退化，未文档化 | [layer.rs:231](..//aimux-core/src/trace/layer.rs#L231)、[:309](..//aimux-core/src/trace/layer.rs#L309) |
| B6 | minor | `system_tokens` 恒为 0，R-2.1 跨 session 系统段实际关闭 | [layer.rs:345](..//aimux-core/src/trace/layer.rs#L345)、[verdict.rs:491](..//aimux-core/src/trace/verdict.rs#L491) |
| B7 | minor | 非流式 Error finish_reason 未按 RFC §4.3 在 content 追加错误说明（流式路径有） | [openai_output.rs:911](..//aimux-core/src/openai_output.rs#L911)、[:338](..//aimux-core/src/openai_output.rs#L338) |
| B8 | minor | cache_probe e2e call 3 注释误导：实际由 R-1.6b 触发而非 LCP overclaim（R-1.1），LCP overclaim 无端到端覆盖 | [cache_probe_test.rs:468](..//aimux-core/tests/cache_probe_test.rs#L468) |
| B9 | minor | `aggregate` 对每组重新全量 filter，O(groups×records) | [store.rs:404-409](..//aimux-core/src/trace/store.rs#L404) |
| B10 | minor | `ResolvedModel` 合并产物未实现（已文档化为有意简化），但 RFC §4.1 仍定义其为 list_models 返回类型 | [model_catalogue.rs:9](..//aimux-core/src/model_catalogue.rs#L9) |
| B11 | nit | `ToolCallAccum` 用 `#[allow(dead_code)]` 存 id/name 从不读 | [openai_output.rs:471](..//aimux-core/src/openai_output.rs#L471) |
| B12 | nit | hash.rs 自纠 RFC 乐观碰撞率表述（"~2⁻¹²⁸"、非 HMAC），建议同步 RFC 措辞 | [hash.rs:6-9](..//aimux-core/src/trace/hash.rs#L6) |

### 4.3 core 管线改造 + providers config_snapshot（分区 C）

| # | 级别 | 问题 | 位置 |
|---|---|---|---|
| C1 | major | **RFC-0027 `list_models` 契约漂移**：RFC 规定返回 `ResolvedModel[]`（内部合并 catalogue），实现改为 `RuntimeModel` + 宿主自行调 `get_model_specs`；`ResolvedModel` 类型不存在但文档引用 | [provider.rs:30-46](..//aimux-core/src/provider.rs#L30)、[model_catalogue.rs:20-26](..//aimux-core/src/model_catalogue.rs#L20)、RFC-0027 L46-49/L154-166/L196-222 |
| C2 | major | **OpenAI 兼容注册表族丢失真实 provider 身份**：`provider_handle("deepseek"...)` 把 registry 名写入 `OpenAIConfig.provider`，但 `provider()`/`config_snapshot()` 硬编码 `"openai"`；`list_models` 却知道用 `config.provider` 区分——内部不一致，快照无法支撑按录制配置重建 | [openai/model.rs:203-215](..//aimux-providers/src/openai/model.rs#L203)、[provider.rs:162-177](..//aimux-providers/src/provider.rs#L162) |
| C3 | major | **原生 provider 的 env key 被误记成 `explicit`**：Google/Anthropic/Mistral/Cohere 有 `from_env()` 但 `config_snapshot()` 无条件写 explicit，回放时会要求再次显式给 key；OpenAIConfig 已有正确的 `api_key_source` 模式可复用 | [google/model.rs:112](..//aimux-providers/src/google/model.rs#L112)、[anthropic/model.rs:92](..//aimux-providers/src/anthropic/model.rs#L92)、[mistral/model.rs:215](..//aimux-providers/src/mistral/model.rs#L215)、[cohere/model.rs:111](..//aimux-providers/src/cohere/model.rs#L111) |
| C4 | minor | 原生 `list_models` retry 策略分叉：Google/Cohere/Mistral 硬编码 `RetryConfig::default()`，Anthropic/OpenAI 用 config | google/cohere/mistral mod.rs |
| C5 | minor | `RuntimeModel.owned_by` 被原生 provider 塞 display_name（"Gemini 2.5 Flash"），字段语义被破坏 | [anthropic/mod.rs:297](..//aimux-providers/src/anthropic/mod.rs#L297)、[google/mod.rs:166](..//aimux-providers/src/google/mod.rs#L166) |
| C6 | minor | Anthropic `config_snapshot` 序列化裸 header map 而非 `ProviderOptions` 形状，且漏 api_version/retry_config/body_overrides | [anthropic/model.rs:92-101](..//aimux-providers/src/anthropic/model.rs#L92) |
| C7 | minor | `AiMuxError::status_code()` 靠解析 `"HTTP {status}:"` 字符串前缀反向取值，文案一变即静默丢失 status | [error.rs:143-164](..//aimux-core/src/error.rs#L143) |

### 4.4 FFI + 全语言 bindings（分区 D）

| # | 级别 | 问题 | 位置 |
|---|---|---|---|
| D1 | major | **FFI `list_models`/`get_model_specs` 绕过共享 runtime 与 M7 重入守卫**：不走 `run_and_serialize`→`ffi_block_on`→共享 `runtime()`，自建 `tokio::runtime::Handle::try_current()`；宿主已跑 tokio 时嵌套 `block_on` panic；临时 Runtime drop 后持 handle 调 block_on 行为存疑；happy path 未被测试覆盖 | [lib.rs:948-980](..//aimux-ffi/src/lib.rs#L948)、[:1016-1033](..//aimux-ffi/src/lib.rs#L1016) |
| D2 | major | **trace(RFC-0015)/session(RFC-0024) 透传覆盖断层**：Go 无 session、Swift/Kotlin/Java/Flutter 无 trace 也无 session——这些语言能设 `session_id` 却查不了（Node/Python 齐全，Go 有 trace.go） | FFI `aimux_trace_*`/`aimux_session_*` vs 各 binding |
| D3 | major | **Swift `Model.generate(prompt:)` 手拼 JSON 不转义**：`"\"\(prompt)\""` 对含引号/反斜杠/控制字符的 prompt 产出非法 JSON，其余 binding 均正确转义 | [Aimux.swift:632-633](..//bindings/swift/Sources/Aimux/Aimux.swift#L632) |
| D4 | minor | `list_models`/`get_model_specs` 序列化失败信封缺 `error_type`/`status_code`，违反 header:30-33 全信封契约 | lib.rs:974-975、:1027-1028 |
| D5 | minor | invalid-handle 的 error_type 跨函数不一致：provider handle 系列返回 `"InvalidHandle"`，generate/stream 系列返回 `"Other"` | lib.rs:952/992 vs :1056 |
| D6 | minor | Python `session_calls`/`list_sessions` 失败不抛异常、返回半信封（`{"error":...}`）而非 PyRuntimeError，调用方静默拿到 dict 而非 list | [lib.rs:722-732](..//bindings/python/src/lib.rs#L722) |
| D7 | minor | Go `InitRecordingRing(cap=0)` 静默改写为 2048，与 FFI(-1)/Node/Python(no-op)/Swift(拒绝)/Kotlin(IllegalArgumentException) 行为分叉 | [aimux.go:326-333](..//bindings/go/aimux.go#L326) |
| D8 | minor | Go `Trace()` 返回的 model 未设 `runtime.SetFinalizer`（base Model 有），忘 Close 即双泄漏 | [trace.go:61](..//bindings/go/trace.go#L61) |
| D9 | nit | Node `getModelSpecs` 返回 `Promise<unknown>`，文档描述不存在的 `lookup()` 方法 | [index.ts:354](..//bindings/node/src/index.ts#L354) |
| D10 | nit | 序列化错误消息前缀不统一（`serialize list_models:` vs `[Json] serialize result:`）；cap 类型宽度不一（u32 vs u64） | 跨语言 |

### 4.5 CLI 工具 + workspace（分区 E）

| # | 级别 | 问题 | 位置 |
|---|---|---|---|
| E1 | major | **provider probe 请求序列非 append-only**：每轮第一条 user 消息不同、follow-up 含当前 i，上一轮请求不是下一轮前缀，无法验证会话式前缀缓存（RFC-0025 核心前提） | [provider.rs:167](..//tools/aimux-cli/src/probe/provider.rs#L167) |
| E2 | major | **stats 按 CLI 传入的 provider 名过滤**：Google 记录用 `google.generative-ai`、XAI 用 `xai.chat`，而 CLI 收 `--provider google`，原生模型 stats 恒空（请求成功但报告 0 命中） | [provider.rs:226](..//tools/aimux-cli/src/probe/provider.rs#L226) |
| E3 | minor | replay `--call-id` 未命中静默退出 0（回归/CI 场景会误判成功） | [main.rs:51](..//tools/aimux-replay/src/main.rs#L51) |
| E4 | minor | replay 失败汇总分母用整个文件记录数而非实际筛选记录数（1/100 误导） | [main.rs:75](..//tools/aimux-replay/src/main.rs#L75) |
| E5 | minor | replay dry-run 不应用 prompt override、不重建 provider，展示的不是真实将发送的 plan | [main.rs:58](..//tools/aimux-replay/src/main.rs#L58) |
| E6 | minor | `gen_providers_doc.py` 无 `--check` 且已漂移（non-registry 76→78、新模块未反映），且把所有 `pub mod` 当 provider（replay/catalogue 被误算） | [gen_providers_doc.py:129](..//scripts/gen_providers_doc.py#L129) |
| E7 | minor | `local-ci.sh` contract 阶段漏 `gen_ts_types.py --check`（GitHub CI 有），本地 gate 未完全镜像远端 | [local-ci.sh:73](..//scripts/local-ci.sh#L73) |
| E8 | nit | offline/session "无有效结果"统一退出 0，脚本无法区分"空结果"与"输入无效" | offline.rs:47、session.rs:15 |
| E9 | nit | local-ci.sh 注释称 "Stops at the first failing gate"，实现是收集所有失败后统一退出 | local-ci.sh:20 |

---

## 5. 按四维审查的总体判断

### 5.1 架构状况 — 有隐患
- **分层方向正确**：core 保持 provider 无关（`openai_output.rs` 不 import providers、catalogue 类型在 core/网络在 providers、自动 rebuild 在 providers 避免循环依赖）；binding 层只透传，无业务逻辑泄漏；CLI/replay 正确复用 core API。
- **主要隐患**：① 内存有界语义失效（A3/A4，ring 与 channel 均非真正有界）；② FFI 新入口绕过重入守卫（D1）；③ RFC 与实现契约漂移（C1、A7、B10）。

### 5.2 代码整洁度 — 中等偏好
- 命名/模块拆分总体清晰；生产路径 unwrap 可控；错误路径多数含上下文（文件/行号）。
- **主要不足**：provider `list_models` 大规模复制粘贴（C4/C5 即复制分叉的体现）；时间格式两套（A10）；字符串编码状态码（C7）；跨语言手写类型漂移风险（D9/D10）。

### 5.3 边界遵守情况 — 核心边界良好，安全边界有漏洞
- **良好**：provider-utils 无特定 provider 硬编码；FFI/C ABI 契约统一（除 D1/D4 外）；脱敏集中在 recorder 边界统一递归执行。
- **漏洞**：① URL query 凭据（B1）；② 非 OpenAI 协议降级而非 Unsupported（A7）；③ 快照身份丢失（C2/C3）；④ trace/session 透传断层（D2）；⑤ scope_key 串域（B1）；⑥ CLI stats 过滤不一致（E2）。

### 5.4 模块抽象正确度 — 核心抽象扎实，接口有漂移
- **扎实**：verdict/confidence 模型、fingerprint 块链、LCP 上界、单调时钟域、Recorder/barrier、SessionStore 基本形态；`config_snapshot`/`list_models` 默认方法设计合理。
- **漂移**：ExactMatcher/ScoreMatcher 语义（A8/A1/A2）；ResolvedModel（C1/B10）；RuntimeModel.owned_by 语义（C5）；step 单调性（A12）。

---

## 6. 测试覆盖缺口

| 缺口 | 关联 finding |
|---|---|
| URL query API key 脱敏 | B1 |
| ScoreMatcher 对"双 None temperature + 零相关性"必须 miss | A1 |
| 空 prompt 不 panic | A2 |
| 非 OpenAI body/SSE 必须返回 Unsupported | A7 |
| UTF-8 跨 chunk 边界录制保真 | A6 |
| 429/5xx 录制保留 status/headers/body | A5 |
| RingRecorder pending 容量与淘汰 | A3 |
| JsonlRecorder 初始化/写入失败可观测 | A9 |
| LCP overclaim（R-1.1）端到端 | B8 |
| Python 新透传 API（get_model_specs 等） | B2 |
| FFI list_models/get_model_specs happy path（宿主 runtime 嵌套场景） | D1 |

---

## 7. 优先修复行动计划

### 阶段一：Blocker（建议立即）
- [ ] **B1** URL query 凭据脱敏（http.rs:986）+ key/api_key/api-key 测试
- [ ] **B2** Python import 补 `get_model_specs as _native_get_model_specs` + 离线测试

### 阶段二：Major（建议下次 release 前）
- [ ] **D1** FFI list_models/get_model_specs 改走 `ffi_block_on`/共享 runtime + 全错误信封
- [ ] **A3/A4** JsonlRecorder 换 bounded channel + 丢弃计数；RingRecorder pending 上限/超时淘汰 + incomplete 导出
- [ ] **A1/A2/A8** matcher 正确性：temperature 不得单独命中、空 prompt 防下溢、ExactMatcher 纳入生成参数
- [ ] **C2/C3** 快照身份修复：OpenAI 兼容族用 `config.provider`；原生族引入 `api_key_source`
- [ ] **C1** 定稿 RFC-0027 契约（ResolvedModel vs RuntimeModel 二选一，同步 RFC/bindings 文档）
- [ ] **A5/A6** HTTP 错误响应结构化录制 + 流式按字节累积解码 + truncated 标记
- [ ] **A7** 非 OpenAI 协议返回 Unsupported（或单独命名 raw debug API）
- [ ] **B1(trace)** scope_key 并入 provider/base_url
- [ ] **D2** 补齐 Go(session) 与 Swift/Kotlin/Java/Flutter(trace+session) 透传，或文档化有意范围
- [ ] **E1/E2** CLI probe 请求序列改 append-only；stats 用 `model.provider()` 实际值过滤
- [ ] **D3** Swift JSON 转义

### 阶段三：Minor（排期）
- [ ] **A10/A11/A12/A13** 时间格式统一、token 截断 fail-fast、step u64、call_id 单一来源
- [ ] **B2-B10** 逆一致性测试、锁分片、占位记录、降级文档化、R-2.1 语义、Error content、e2e 测试注释、aggregate 合并、RFC 同步
- [ ] **C4-C7** retry 统一、owned_by 语义、Anthropic snapshot 形状、状态码结构化
- [ ] **D4-D8** 信封契约、error_type 统一、Python 异常、cap=0 跨语言对齐、Go finalizer
- [ ] **E3-E7** replay 计数/退出码/dry-run plan、gen_providers_doc --check + 接入 CI、local-ci 镜像

### 阶段四：Nit（随手清理）
- [ ] **A14/B11/B12/D9/D10/E8/E9** 注释修正、死字段删除、RFC 措辞同步、文档漂移

---

## 8. 总体评价

本变更集（RFC-0015/16/23/24/25/26/27 落地）**工程总量大但分层正确**：判定/录制/回放引擎的抽象设计扎实、测试充分（cache_probe/recording_e2e/session/golden 等），FFI 错误传播模型（自包含 envelope、M7 重入防护）设计正确。主要风险集中在三处：

1. **录制管道的安全与内存边界**（B1 blocker、A3/A4/A5/A6）——凭据泄漏 + 有界承诺失效是发布前必须处理的；
2. **跨语言透传的不一致与缺失**（B2 blocker、D2、D3、D7）——同一能力在 8 语言间行为分叉；
3. **RFC 与实现的契约漂移**（C1、A7、B10、C2/C3）——文档承诺与代码事实不一致，会误导调用方。

建议按阶段一（2 个 blocker）→ 阶段二（major）的顺序推进，并补全 §6 的测试缺口后发布。

---

## 9. 报告索引

| 相关报告 | 内容 |
|---|---|
| [SUMMARY.md](SUMMARY.md) | 上一轮 P0-P3 质量总报告（2026-08-06） |
| [p2-error-handling-layering.md](p2-error-handling-layering.md) | 错误分层（M3/M4/M6 与本报告 C7/A5 同源） |
| [p1-ffi-soundness-review.md](p1-ffi-soundness-review.md) | FFI soundness（M7 与本报告 D1 同源） |
| [p2-provider-abstraction-audit.md](p2-provider-abstraction-audit.md) | provider 抽象（M9/M10 与本报告 C2/C4 同源） |
| [p5-native-providers-round2-review.md](p5-native-providers-round2-review.md) | 第二轮：原生 provider 深挖（config_snapshot 覆盖矩阵 + max_retries） |
| [p6-concurrency-async-round3-review.md](p6-concurrency-async-round3-review.md) | 第三轮：并发/异步正确性 + 热路径性能（Yara） |
| [p6-rfc-consistency-round3-review.md](p6-rfc-consistency-round3-review.md) | 第三轮：RFC 状态矩阵 + 生成文件漂移（Otto） |
| [rfc/0023-runtime-request-recording.md](../../rfc/0023-runtime-request-recording.md) | 录制/回放意图 |
| [rfc/0027-model-catalogue-and-list-api.md](../../rfc/0027-model-catalogue-and-list-api.md) | list API 意图 |

---

## 10. 第二轮审计补充（2026-08-07）

> 第二轮派出 4 个 agent：**Harper**(glm-5.2，原生 provider 深挖)、**Rowan**(gpt-5.6-sol，API 兼容性/semver/文档漂移)、
> **Vega**(glm-5.2，新增 unsafe/panic 路径安全)、**Piper**(gpt-5.6-sol，未覆盖模块 + 第一轮抽查验证)。
> 全部新声明已由主进程复核源码确认。Harper 完整报告见 p5。

### 10.1 第二轮新 findings

#### 🔴 Blocker（第二轮新增）

| # | 问题 | 位置 |
|---|---|---|
| B3 | **HEAD 包含 Rust/C 破坏性变更，但 workspace 版本仍为已发布的 0.2.1，且 CHANGELOG `[Unreleased]` 为空**（无任何记录、compare link 仍从 v0.1.0 开始）。按 semver 不应以 0.2.1 发布不同内容，应至少升 0.3.0 并补全 changelog | [Cargo.toml:15](..//Cargo.toml#L15)、[CHANGELOG.md:8](..//CHANGELOG.md#L8) |

#### 🟠 Major（第二轮新增）

| # | 问题 | 位置 |
|---|---|---|
| N1 | **录制脱敏漏掉 AWS STS 临时令牌 `x-amz-security-token` 明文落盘**：录制侧 `is_sensitive_key` 无 `token` needle；Bedrock `from_env()` 读 `AWS_SESSION_TOKEN` → sigv4 写入 `x-amz-security-token` 头 → 录制原样保存。与 B1 同属凭据落盘 | [recording.rs:339-347](..//aimux-core/src/recording.rs#L339)、[sigv4.rs:80-81](..//aimux-providers/src/bedrock/sigv4.rs#L80)、[mod.rs:127](..//aimux-providers/src/bedrock/mod.rs#L127) |
| M1b | **`CallOptions.max_retries`（RFC-0017 per-call override）在 google/cohere/mistral/azure/bedrock 的 do_generate+do_stream+list_models 全路径静默忽略**：硬编码 `RetryConfig::default()`，且这 5 个 Config 无 `retry_config` 字段；OpenAI/Anthropic 正确用 `resolve_*_retry` 合并。用户可见契约违反。**升级并扩展第一轮 C4**（minor→major，list_models→全路径） | [google/model.rs:139/218](..//aimux-providers/src/google/model.rs#L139)、cohere/mistral/azure model.rs |
| M2b | **config_snapshot 覆盖缺口：8 个 LanguageModel 回落 `minimal`**（base_url=None、source="unknown"）：bedrock/anthropic_aws/vertex×2/xai×2/open_responses/huggingface_responses；其中 xai/open_responses/huggingface_responses 的 list_models 已构造 OpenAIConfig 却不复用 snapshot helper | providers 各 mod.rs |
| P1 | **contract-tests 的 Rust "全 fixture 匹配"测试只验证 JSON 可解析**：`all_fixtures_have_matching_rust_serialization` 仅 `serde_json::from_str`，不构造/序列化任何 Rust 值；`Fixture.ty` 是死代码；`assert_roundtrip` 未被调用。已出现实例：新增 session fixture 缺 `include_raw_chunks:null`（真实 serde 会输出）而测试仍绿 | [contract_test.rs:205-214](..//aimux-core/tests/contract_test.rs#L205)、[wire-format.json:62](..//contract-tests/fixtures/wire-format.json#L62) |
| R2 | **`AiMuxError::RateLimited` 变体形状破坏性变化**：v0.2.1 为 `{ retry_after_ms: u64 }`，HEAD 为 `{ retry_after_ms, message: String }`；外部模式匹配与 JSON 输出形状均破坏，`#[serde(default)]` 只兼容反序列化 | [error.rs:41-47](..//aimux-core/src/error.rs#L41) |
| R3 | **公开 struct 新增字段破坏完整 struct literal 构造**：`GenerateTextOptions`(+session_id/include_raw_chunks)、`CallOptions`(+session_id/call_id/recording_context/include_raw_chunks)、`StreamTextResult`(+request_body/response_headers)，均无 `#[non_exhaustive]` | [generate.rs:64](..//aimux-core/src/generate.rs#L64)、[options.rs:122](..//aimux-core/src/options.rs#L122) |
| R4 | **C header 删除 `aimux_last_error` 声明**，属 C 源码兼容破坏，未记录迁移路径 | [aimux-ffi.h:380](..//aimux-ffi/aimux-ffi.h#L380) |
| R5 | **`docs/provider-config-manual.md` 仍声称 `listModels()` 自动合并 anya2a 并带缓存/TTL**，代码实际返回裸 `RuntimeModel`、宿主自行 `get_model_specs` 合并；用户照手册访问 `models[i].spec` 会得到不存在字段 | [provider-config-manual.md:119-158](..//docs/provider-config-manual.md#L119) ↔ [provider.rs:30-46](..//aimux-core/src/provider.rs#L30) |
| R6 | **`aimux-ffi.h` 对取消能力自相矛盾**：顶部声明 "C ABI 没有 abort/cancel entry point"，同文件紧接公开 `aimux_abort_signal_new`/`aimux_abort_signal_abort`/`aimux_stream_text_with_abort` | [aimux-ffi.h:23-26](..//aimux-ffi/aimux-ffi.h#L23) vs [:284-326](..//aimux-ffi/aimux-ffi.h#L284) |
| R7 | **RFC-0023 仍标记 Draft/全部"待实施"**，但 Recorder/RingRecorder/FFI 入口/MockReplayModel 均已落地；维护者可能重复实施或误判 API 未公开 | [rfc/0023:3](..//rfc/0023-runtime-request-recording.md#L3)、[:474-481](..//rfc/0023-runtime-request-recording.md#L474) |
| R8 | **RFC-0026 仍标记草案**，但 Rust/C ABI 的 `generate_text_as_openai`/`stream_text_as_openai` 已公开 | [rfc/0026:3](..//rfc/0026-openai-compatible-output.md#L3) ↔ [generate.rs:514](..//aimux-core/src/generate.rs#L514)、[lib.rs:1190](..//aimux-ffi/src/lib.rs#L1190) |

#### 🟡 Minor（第二轮新增）

| # | 问题 | 位置 |
|---|---|---|
| N2 | `aimux_provider_list_models` 的 `map_err`+`expect` 是死代码：runtime 创建失败时构造的 `*mut c_char` 被 expect 丢弃（内存泄漏）+ 直接 abort，错误 JSON 永远到不了调用方 | [lib.rs:958-971](..//aimux-ffi/src/lib.rs#L958) |
| N3 | **脱敏规则分叉根因**：recording.rs 注释称"与 logging.rs 同规则"，实际 needle 集不同（录制缺 `token`/`key`/`apikey`，日志缺 cookie/set-cookie），建议合并为一个共享函数 | [recording.rs:336](..//aimux-core/src/recording.rs#L336) vs [logging.rs:151](..//aimux-provider-utils/src/logging.rs#L151) |
| N4 | `JsonlRecorder::new` 线程派生 `.expect()` 在 FFI 可达路径上 abort 宿主进程（`panic=abort`+`extern "C"`）；与同函数 `create_dir_all().ok()` 静默吞错语义自相矛盾 | [recording.rs:420-423](..//aimux-core/src/recording.rs#L420)、[lib.rs:2300](..//aimux-ffi/src/lib.rs#L2300) |
| P2 | aimux-stream/SSE 不支持纯 CR 行分隔符（SSE 标准允许 CRLF/LF/CR），`data: x\r\r` 被当未完成 partial 丢弃 | [sse.rs:153-165](..//aimux-stream/src/sse.rs#L153) |
| P3 | aimux-stream/SSE 不支持无冒号字段的标准语义（`data\n\n` 应派发空 data event），当前按未知字段丢弃 | [sse.rs:181-212](..//aimux-stream/src/sse.rs#L181) |
| P4 | Node contract runner 是 fixture 自证：fixture 的 producer 与 oracle 是同一 JSON 字符串，native 部分只检查函数存在，不验证序列化形状 | [run-node.ts:27-155](..//contract-tests/run-node.ts#L27) |
| R9 | `docs/api/reference.md` 自称 public API surface index，但 catalogue/recording/replay/session/trace/OpenAI output/C ABI 新 API 几乎全部缺席 | [reference.md:8-77](..//docs/api/reference.md#L8) |
| R10 | C header 对 `aimux_provider_list_models` 描述为 "runtime discovery + anya2a enrichment + ResolvedModel[]"，实际返回 `Vec<RuntimeModel>` 且不做 enrichment | [aimux-ffi.h:229-233](..//aimux-ffi/aimux-ffi.h#L229) |
| R11 | CI 无 semver/public API 门禁、无 C header/导出符号一致性检查、无 `[Unreleased]` 非空校验、无 `cargo publish --dry-run`；本轮实际出现的 break 均不会被 CI 阻止 | [ci.yml:21-26](..//.github/workflows/ci.yml#L21) |
| Hf3 | C2 半修复：OpenAI **Chat** 路径仍硬编码 `"openai"`，**Responses** 路径已用 `config.provider` | openai/model.rs ↔ openai/responses |
| Hf4 | C5 扩展：vertex list_models 也把 display_name 塞进 owned_by | vertex mod.rs |
| Hf5 | C3 扩展：azure×2 + codex 也硬编码 explicit；codex 还丢失 OAuth Subscription 模式 | azure/codex model.rs |
| Hf6 | bedrock list_models 用 `send`（非 `send_timed`），无超时，与同类分叉 | bedrock/mod.rs |
| Hf7 | list_models 在 6 个原生 provider 近乎逐字复制（~300 行） | 各 provider |

#### 🔵 Nit（第二轮新增）

| # | 问题 | 位置 |
|---|---|---|
| N5 | 录制 request/response body 只截断不做 JSON 敏感键脱敏（当前 provider 凭据在 header 未触发，潜在） | [http.rs:974-983](..//aimux-provider-utils/src/http.rs#L974) |
| P5 | contract fixture 无类型驱动/未知 fixture 必须失败机制；session fixture 已漂移而测试绿色 | contract-tests |
| R12 | binding Rust crate 版本 metadata 不同步（python 0.2.0、node 0.1.0 vs workspace 0.2.1） | bindings/*/Cargo.toml |
| R13 | 全范围 `git diff --check v0.2.1..HEAD` 不干净（node types/*.ts 等多处 trailing whitespace，CI 无此门禁） | bindings/node/src/types |
| Hf8 | google/utils.rs unwrap 仍在且 GoogleJsonAccumulator 仍未接生产（p1 结论仍成立） | google/utils.rs |
| Hf9 | anthropic 的 panicking 转换包装实为 test-only（精细化 p1：生产 panic 风险 0） | anthropic |
| Hf10 | anthropic usage `as u32` 截断 | anthropic/convert.rs |

### 10.2 第一轮抽查验证表（Piper 复核，全部属实）

| 第一轮 finding | 结论 | 依据 |
|---|---|---|
| A1 ScoreMatcher temperature 单独命中 | 属实 | replay.rs:153-162，LCP=0 但 temperature 相等仍 +1 |
| A2 空 prompt usize 下溢 | 属实 | replay.rs:142-145，debug panic / release wrap |
| A3 RingRecorder pending 无界 | 属实 | recording.rs:684-792，capacity 只作用于 completed |
| A4 JsonlRecorder 无界 channel | 属实 | recording.rs:412-435 |
| D1 FFI 绕过共享 runtime | 属实且更危险 | lib.rs:181-229（正确路径）vs :948-979/:1016-1032（绕过） |
| C2 OpenAI 兼容族身份硬编码 | 属实 | provider.rs:168-177 存真名 ↔ model.rs:203-215 硬编码 |
| E1 CLI probe 非 append-only | 属实 | provider.rs:167-183 |
| B1-trace scope_key 只含 model_id | 属实 | layer.rs:107-162 |

**结论：第一轮 8 个抽查 major 无一处误报。**

### 10.3 旧审计（P0-P3）结论核对（Vega 复核）

| 项 | 状态 | 证据 |
|---|---|---|
| H1 回调 panic 跨 extern "C" UB | **缓解（非 catch_unwind）**：`panic=abort` + nounwind ABI 下转为进程终止而非 UB | Cargo.toml:30、lib.rs:24-30 |
| H2 convert 静默吞错 | **已修复**：`build_request_body` 返回 Result，fail-fast | openai/convert.rs:866-879 |
| H3 google set_nested_value 7 expect | **已修复**：返回 Result + `?` 传播 | google/utils.rs:195-222 |
| M1 into_cstring_raw null | **已修复**：NUL→U+FFFD 替换，永不返回 null | lib.rs:286-299 |
| M2 文档重入误述 | **已修复**：改为 "process terminates" | lib.rs:24-30 |
| M7 FFI 重入防护 | **原路径已修，但被 D1 新代码部分倒退**（list_models/get_model_specs 绕过守卫） | lib.rs:189-230 |

### 10.4 第二轮优先修复追加（在 §7 基础上）

1. **B3 版本策略**：先定版本（建议 0.3.0），填 CHANGELOG `[Unreleased]`（breaking/additive/迁移示例），修 compare link。
2. **N1+B1 统一脱敏**：`is_sensitive_key` 增 `token` needle；B1 URL query 脱敏；合并 recording/logging 两份脱敏为一个共享函数（同解 N3）。
3. **M1b max_retries**：google/cohere/mistral/azure/bedrock 引入 retry 合并（仿 OpenAI resolve_*_retry），补 per-call override 测试。
4. **M2b config_snapshot 覆盖**：为 8 个缺失 LanguageModel 补 snapshot（xai/open_responses/huggingface 至少复用 OpenAI helper）。
5. **R2-R6 契约统一**：RateLimited 兼容策略（或明确 0.3.0 breaking）；struct 加 `#[non_exhaustive]` 或补 changelog；C header 修复/记录；修正 provider 手册与 header 的 list_models 描述；删 header 取消能力矛盾声明。
6. **P1 重做 contract test**：Rust 按 fixture type 构造真实值逐条比较、禁止未消费 fixture；Node 从 native 获取值再比；修正 session fixture。
7. **R7/R8**：RFC-0023/0026 状态头与实施表同步实现事实。
8. **R11**：CI 加 `cargo-semver-checks` / C header 编译与符号检查 / changelog 校验。
9. **D1+N2+N4**：list_models/get_model_specs 走 `ffi_block_on`；删死 map_err；JsonlRecorder::new 返回 Result。
10. **SSE 标准边界**：补纯 CR 分隔符与无冒号字段支持（P2/P3）。

---

## 11. 第三轮审计补充（2026-08-07）

> 第三轮派出 4 个 agent：**Sage**(gpt-5.6-sol，8 语言 binding 逐语言深度审查)、**Yara**(glm-5.2，并发/异步正确性+热路径性能)、
> **Cyrus**(gpt-5.6-sol，四大文件对抗性逻辑审查)、**Otto**(glm-5.2，RFC 状态全量核对+生成文件漂移+第二轮复核)。
> Sage/Cyrus 为只读审查；Otto/Yara 各自产出独立报告（见下）。
> 主进程已亲自复核 Kotlin 编译阻断（T1）、Flutter 流式同步阻塞（T3）。Yara 完整报告见 p6-concurrency，Otto 见 p6-rfc。

### 11.1 第三轮新 findings

#### 🔴 Blocker（第三轮新增）

| # | 问题 | 位置 |
|---|---|---|
| B4 | **Kotlin `Model.kt` 源码结构损坏，无法编译**：`Model` 类在 L397 闭合、`ProviderHandle` 在 L446 闭合后，L465+ 的 `generateText`/`streamText`/OpenAI 方法变成**顶层函数**却调用 `Model` 私有 `requireHandle()`（L198）；文件末尾还有多余闭合括号（L662）。Kotlin 顶层函数无法访问类私有成员。✅ 主进程已复核源码结构确认 | [Model.kt:396-397](..//bindings/kotlin/src/main/kotlin/ai/arcships/aimux/Model.kt#L396)、[:475](..//bindings/kotlin/src/main/kotlin/ai/arcships/aimux/Model.kt#L475) |

#### 🟠 Major（第三轮新增）

| # | 问题 | 位置 |
|---|---|---|
| T2 | **Kotlin 两个 Sequence 流适配器正常结束必向 `LinkedBlockingQueue` 写 `null`**（`onDone = { parts.put(null) }`），JDK 队列禁止 null，正常完成即 NPE；即使修复 T1 也需处理 | [Model.kt:545-553](..//bindings/kotlin/src/main/kotlin/ai/arcships/aimux/Model.kt#L545)、[:644-652](..//bindings/kotlin/src/main/kotlin/ai/arcships/aimux/Model.kt#L644) |
| T3 | **Flutter `streamText()`/`streamTextAsOpenAI()` 同步阻塞跑完整个原生流后才返回 Stream**：FFI 调用同步、callback 上下文是全局变量；调用方拿到 Stream 时所有 chunk 已产生完毕，主 isolate 全程被阻塞、无取消路径。✅ 主进程已复核（方法文档自认 "Blocks the current isolate"） | [aimux.dart:603-635](..//bindings/flutter/lib/aimux.dart#L603)、[:318-343](..//bindings/flutter/lib/aimux.dart#L318) |
| T4 | **Node recording/mock replay 原生已实现但发布入口完全漏出**：`index.js` 导出列表（L702-743）、`index.d.ts`、typed wrapper 均无 `initRecording*`/`recordingStop`/`recordingFlush`/`mockReplay` | [lib.rs:505-561](..//bindings/node/src/lib.rs#L505)、[index.js:702](..//bindings/node/index.js#L702) |
| T5 | **Python recording/mock replay 已注册到 extension 却未从包入口公开**：`__init__.py` import 与 `__all__` 均无 `init_recording*`/`recording_stop`/`recording_flush`/`mock_replay`（与 B2 是不同断层） | [__init__.py:10-96](..//bindings/python/python/aimux/__init__.py#L10) |
| T6 | **Java/Kotlin `close()` 可与在途 FFI 调用并发**：`requireHandle()` 瞬时读取后 FFI 调用，`close()` 原子交换后立即 drop handle；线程 A 读取句柄 → 线程 B close → A 用已释放句柄。Go 已用 RWLock 正确解决，JVM 无等价保障；ProviderHandle 同类 | [Model.java:45-65](..//bindings/java/src/main/java/ai/arcships/aimux/Model.java#L45)、[Model.kt:191-200](..//bindings/kotlin/src/main/kotlin/ai/arcships/aimux/Model.kt#L191) |
| C4-1 | **TraceStore LCP 可由不同历史记录逐块拼接**：`lookup()` 每轮独立选候选、不要求同一 `(slot,generation)`，可构造不存在的"历史前缀"并扩大 lcp_upper_bytes，可能把 overclaim 判成 Trusted | [store.rs:169-212](..//aimux-core/src/trace/store.rs#L169) |
| C4-2 | **byte-proxy 用 `bytes/4` 作 token 上界不安全**：ASCII/数字/标点可接近 1 byte/token，`bytes/4` 可能低估、方向性错误地把真实命中判成 overclaim（应只做必要条件返回 Unknown 或真正保守上界） | [verdict.rs:501-505](..//aimux-core/src/trace/verdict.rs#L501)、[layer.rs:360-366](..//aimux-core/src/trace/layer.rs#L360) |
| C4-3 | **OpenAI 流式输出 id/model 中途变化**：`StreamStart` 立即发首帧，后到的 `ResponseMetadata` 覆盖 state 的 id/model；OpenAI provider 真实顺序正是先 StreamStart 后 metadata，chunk identity 前后不一致可能被严格客户端拒绝 | [openai_output.rs:536-549](..//aimux-core/src/openai_output.rs#L536)、[openai/model.rs:580-589](..//aimux-providers/src/openai/model.rs#L580) |
| C4-4 | **非流式 mock replay 丢弃全部 OpenAI tool calls**：只读 `choices[0].message.content`，`content:null + tool_calls:[...]` 的合法响应重建出空 content 但 finish_reason=ToolCalls，工具型 agent 回放不可用 | [replay.rs:370-403](..//aimux-core/src/replay.rs#L370) |
| F1 | **`TraceLayer::do_stream` 流被提前 drop 时 trace 记录永不写入**：`rec_ctx.record` 在流循环结束后唯一落库；abort/take(N)/超时在 yield 挂起点丢弃生成器，与录制层 `RecordingOutcomeStream` 的 Drop 兜底行为不一致 | [layer.rs:471-488](..//aimux-core/src/trace/layer.rs#L471) |
| N3-1 | **`docs/plan/rfc0023-recording.md` 自相矛盾**：头部写"待修订 RFC 定稿后实施"，自身进度表却显示 P1-P6 + config_snapshot 全部 ✅ 已实施 | [rfc0023-recording.md:5](..//docs/plan/rfc0023-recording.md#L5) vs [:92-98](..//docs/plan/rfc0023-recording.md#L92) |
| N3-2 | **provider-config-manual §7 整段与代码不符**（.spec 合并/缓存/TTL/offline 均不存在，catalogue.rs 明确 no caching）——与 R5 同源，但这里是**整段设计承诺** | [provider-config-manual.md:119-158](..//docs/provider-config-manual.md#L119) |

#### 🟡 Minor（第三轮新增）

| # | 问题 | 位置 |
|---|---|---|
| C4-5 | `StreamPart::Error` 不终止流：后续 Finish 可把错误覆盖成成功 stop，终态误导 | [openai_output.rs:815-819](..//aimux-core/src/openai_output.rs#L815) |
| C4-6 | Recording `Provider` 事件先于 `Input` 时被静默丢弃（未用 `entry_or_init`），配置快照不可恢复 | [recording.rs:568-571](..//aimux-core/src/recording.rs#L568) |
| C4-7 | Recording 重复 attempt 未拒绝，ExchangeUpdate 总 patch 第一条，可能卡死 barrier 且 response 串到错误 request | [recording.rs:585-593](..//aimux-core/src/recording.rs#L585) |
| C4-8 | 流式 replay 只重建文本：tool_calls/reasoning/metadata/usage-only 末帧全丢 | [replay.rs:430-469](..//aimux-core/src/replay.rs#L430) |
| C4-9 | replay `last_response` 可能选"有响应但失败"的 attempt（不检查 status/finalized/error） | [replay.rs:313-318](..//aimux-core/src/replay.rs#L313) |
| T7 | Node `index.d.ts` 声明 `getModelSpecs` 但 `index.js` 未导出，类型检查允许、运行时 undefined | [index.d.ts:243](..//bindings/node/index.d.ts#L243) vs [index.js:702](..//bindings/node/index.js#L702) |
| T8 | Go `MockReplay()` 返回的 Model 未设 finalizer（同 D8 的 Trace()） | [aimux.go:346-357](..//bindings/go/aimux.go#L346) |
| T9 | Flutter Model/ProviderHandle 无 NativeFinalizer/Dart Finalizer，忘 close 永久泄漏 | [aimux.dart:712-718](..//bindings/flutter/lib/aimux.dart#L712) |
| T10 | Swift recording 初始化完全忽略 C 返回码（cap=0 时 C 返回 -1 但 Swift 静默） | [Aimux.swift:247-260](..//bindings/swift/Sources/Aimux/Aimux.swift#L247) |
| T11 | Java `initRecordingRing(long)` 未拒绝负数，JNA 传成巨大 u64 可能触发大容量分配失败 | [Aimux.java:49-58](..//bindings/java/src/main/java/ai/arcships/aimux/Aimux.java#L49) |
| T12 | Flutter recording ring 接受负 int 按无符号大值跨 FFI | [aimux.dart:952-956](..//bindings/flutter/lib/aimux.dart#L952) |
| T13 | **Go/Java/Kotlin/Swift/Flutter 错误信封统一只留 message，丢弃 error_type/status_code**：调用方无法按 Auth/RateLimited/Timeout 稳定分支 | [aimux.go:791](..//bindings/go/aimux.go#L791)、[AimuxResult.java:63](..//bindings/java/src/main/java/ai/arcships/aimux/AimuxResult.java#L63)、[Model.kt:163](..//bindings/kotlin/src/main/kotlin/ai/arcships/aimux/Model.kt#L163) 等 |
| F2 | `aimux_recording_flush` 恒返回 0，`recv_timeout(30s)` 超时后仍报成功，无可观测性 | [recording.rs:513-522](..//aimux-core/src/recording.rs#L513)、[lib.rs:2334](..//aimux-ffi/src/lib.rs#L2334) |
| F3 | `TraceStore::lookup` 是 `&mut self` → LCP 查询与插入抢同一把排他锁，并发调用串行化（扩展 p4 B3） | [store.rs:169](..//aimux-core/src/trace/store.rs#L169) |
| F4 | `export_jsonl`/`aggregate`/`session_chain` 持全局 Mutex 做全量遍历/IO，可阻塞热路径 | [store.rs:469](..//aimux-core/src/trace/store.rs#L469) |
| F5 | `session_chain` 对每个 call_id 做 `records.iter().find`，O(records×session_len) | [store.rs:430](..//aimux-core/src/trace/store.rs#L430) |
| F6 | TraceLayer `record()` 在 async 线程做 denoise 深拷贝+序列化+哈希+双锁，未 spawn_blocking | [layer.rs:192](..//aimux-core/src/trace/layer.rs#L192) |
| F7 | SessionStore LRU `touch` O(sessions)，每次 generate 入口持锁执行 | [session.rs:216](..//aimux-core/src/session.rs#L216) |
| F8 | `to_http_record` 先物化整段请求体再截断 1 MiB，每条 exchange 在 async 线程 | [http.rs:961](..//aimux-provider-utils/src/http.rs#L961) |
| N3-3 | `aimux-ffi.h` 取消能力矛盾声明（同 R6，C ABI 侧确认） | aimux-ffi.h:23-26 |
| N3-4 | RFC-0027 P3 `catalogue sync` CLI 命令未实现（RFC 已承诺） | aimux-cli |
| N3-5 | local-ci.sh 弱于 CI：缺 `gen_ts_types.py --check` + node drift 检查（确认 E7） | local-ci.sh:73 |

#### 🔵 Nit（第三轮新增）

| # | 问题 | 位置 |
|---|---|---|
| C4-10 | trace 非法 fingerprint hash 被 `unwrap_or(0)` 静默变 0，可能制造假匹配 | [store.rs:486-492](..//aimux-core/src/trace/store.rs#L486) |
| T14 | 5 种 binding 的 provider-list 文档仍宣称 ResolvedModel/anya2a enrichment（Go/Swift/Kotlin/Java/Flutter） | 各 binding |
| F9 | `append` 全局锁内逐块 hex→u128 解析 | [store.rs:487](..//aimux-core/src/trace/store.rs#L487) |
| F10 | 两份独立 `new_call_id()` 工厂（recording.rs:325 + session.rs:397） | 两文件 |
| N3-6/7/8 | 0027-coverage 缺状态头；0005-rename 状态滞后；0015/0024"待依赖 0023"依赖现已过期 | rfc/ 各文件 |

### 11.2 第二轮结论独立复核（Otto 复核）

| 第二轮 finding | 结论 | 依据 |
|---|---|---|
| B3 版本/CHANGELOG | **属实（更严重）**：连 `[0.2.1]` 段都没有，compare link 仍从 v0.1.0 起 | Cargo.toml、CHANGELOG.md |
| R2 RateLimited 变体改形 | 属实 | error.rs 双字段 |
| R5/R10 list_models 契约 | 属实 | manual/header vs provider.rs |
| R7/R8 RFC-0023/0026 状态 | 属实 | rfc 状态头 vs 代码 |
| M2b config_snapshot 覆盖缺口 | **部分属实**：8 个 LanguageModel 缺 config_snapshot 属实；但子论断"open_responses/huggingface_responses 的 list_models 已构造 OpenAIConfig"**不准确**——二者无 `fn list_models`（用 trait 默认），该子论断仅对 xai 属实 | 各 mod.rs |

**结论：第二轮无完全误报，仅 M2b 一处子论断需修正（已在此修正）。**

### 11.3 生成文件漂移检查（Otto，实测通过）

| 检查 | 结果 |
|---|---|
| `gen_provider_names.py --check` | ✅ 通过（8 文件 / 251 名称） |
| `gen_ts_types.py --check` | ✅ 通过（123 文件 up to date） |
| node `index.js`/`index.d.ts` 工作树 | ✅ 干净（CI 有 `git diff --exit-code` 门禁） |
| registry 251 = RFC-0027 coverage 计数 | ✅ 一致 |
| node types 118 行 trailing whitespace | ⚠️ 无 CI 门禁（R13 确认） |

### 11.4 RFC 状态矩阵（Otto，28 文件全量）

- **17 一致 / 4 基本·部分一致 / 3 明显不一致**（0023、0026 状态滞后——已 P1-P6 全部实施仍标 DRAFT/草案；0005-rename 滞后）/ **4 无状态头**（含 0027-coverage）。
- 完整矩阵见 [p6-rfc-consistency-round3-review.md](p6-rfc-consistency-round3-review.md)。

### 11.5 第三轮优先修复追加（在 §7/§10.4 基础上）

1. **B4/T2 Kotlin 阻断**：把 generation/OpenAI 方法移回 `Model` 类、删多余闭合括号、Sequence 终止改 sealed sentinel/Flow；并给 Kotlin binding 加**编译门禁**（gradle compile check 进 CI/local-ci）。
2. **T4/T5/T7 发布产物对齐**：Node 重新生成 `index.js`/`index.d.ts` 并让 typed wrapper 一致公开 recording/mock replay/getModelSpecs；Python 补 recording/mock replay 与 `_native_get_model_specs` 导入 + `__all__` + `hasattr` 测试。
3. **T3 Flutter 流式重构**：FFI 流移到 worker isolate/native port，拿到首块前就返回 Stream，移除全局 `_currentController`，提供取消。
4. **T6/T8/T9 资源模型统一**：Java/Kotlin 在途调用加 RWLock/ref-count lease（对齐 Go）；Go trace/mock replay 补 finalizer；Flutter 加 NativeFinalizer。
5. **C4-1/C4-2 判定正确性**：LCP 候选固定同一 record 身份；byte-proxy 不得用 bytes/4 做 overclaim 上界。
6. **C4-3/C4-5 流式稳定**：首帧前等 ResponseMetadata 或 metadata 不得改已公开 id/model；Error 后立即终止。
7. **C4-4/C4-8/C4-9 replay 完整性**：非流式解析 tool_calls、流式支持 tool_calls/reasoning/usage-only、last_response 只选 finalized+2xx+无 error。
8. **F1 trace Drop 兜底**：`do_stream` 流式提前 drop 也要落 record（对齐 RecordingOutcomeStream）。
9. **N3-1/N3-2/R7/R8 文档一致性**：修 rfc0023 plan 矛盾、provider manual §7、RFC 状态头。
10. **F2-F8 性能**：flush 可观测、缩小 TraceStore 持锁面、热点 offload spawn_blocking。

---

## 12. 新旧归属分类（2026-08-07，基于 git 证据）

> 回答"这些问题是否都是本次变更引入的"。分类依据：对关键项逐一用 `git show v0.2.1:<file>` 与 HEAD 对比。
> 结论：约 **85% 为本次引入**（本次变更是大量新模块）；**约 6-8 项为存量问题**（扫新代码时顺带发现），不应阻塞本次发布。

### 12.1 本次变更引入（发布前必改）

| 项 | 证据 |
|---|---|
| A 系列（recording/replay/session 核心全部） | 全新文件（recording.rs +1420 / replay.rs +1028 / session.rs +708） |
| B-trace / C4 / F 系列（trace/*、openai_output.rs、model_catalogue.rs） | 全新文件 |
| B1 URL query 泄露、N1 STS token、N3/N4/N5 | 录制模块是新增代码 |
| B2 Python NameError、T4/T5 Node/Python 透传断层 | 新增 API 的接入断层 |
| B4 Kotlin 编译断裂 | v0.2.1 Model.kt 结构正常（方法在类内），HEAD 断裂 |
| C2/C3/M2b config_snapshot 身份与覆盖 | config_snapshot 是本次新方法（C2 底层 provider() 硬编码为存量，见 12.2） |
| D 系列 FFI（D1/N2）、R2/R3/R4（RateLimited、struct 字段、删 aimux_last_error） | 本次修改的公开签名与新增入口 |
| E 系列 CLI 工具、P1 contract-tests、R5-R13 文档/CI | 新增工具、新增测试、本次同步缺失 |
| N3-1/N3-2/R7/R8（RFC/plan/手册滞后） | 本次实现后未同步文档 |

### 12.2 存量问题（本次未引入，可排 backlog，不应阻塞本发布）

| 项 | git 证据 | 备注 |
|---|---|---|
| P2/P3 SSE 纯 CR/无冒号字段 | `git diff v0.2.1..HEAD -- aimux-stream/src/sse.rs` 为空 | 标准边界，老代码 |
| C7 status_code 字符串解析 | v0.2.1 error.rs:130 已存在 | 旧审计 M4 已记录同项 |
| M1b max_retries 被 google/cohere/mistral 忽略（do_generate/do_stream） | v0.2.1 google/model.rs:125/202 已有 `RetryConfig::default()`；options.rs:108 `max_retries` 字段 v0.2.1 已有 | 本次仅新增 list_models 路径延续；核心行为存量 |
| T6 Java/Kotlin close 竞态（base Model） | v0.2.1 Kotlin Model.kt:163/165 已有 AtomicLong+close 模式 | 新增 ProviderHandle 复用老设计 |
| C2 的 provider() 硬编码 "openai" | v0.2.1 openai/model.rs:190 已有 | 新 config_snapshot() 继承存量硬编码 |
| H1/H3（旧审计高危） | 存量 | **本次已修复/缓解**（Vega 复核） |

### 12.3 发布策略建议

1. **本次 blocker 清单** = B1/B2/B3/B4（均为本次引入）必须在本发布前处理。
2. **存量问题（§12.2）单独建 backlog issue**，注明"非本次回归"，避免被误认为发布阻断。
3. **新功能门禁补漏**：Kotlin/Java/Flutter/Node/Python binding 的编译/导入门禁（本次已因缺此门禁漏出 B4、B2、T4、T5）。
---

## 13. 第四轮审计补充 — 远程同步变更 review（2026-08-07，c0fceba）

> 远程 master 更新 15 个提交（4dac8ce..c0fceba，98 文件 / +9080/-5561），本地已 fast-forward 同步。
> 核心变更：**FFI 错误传输重构（breaking，`AimuxError` out-param）**、RFC-0024 session 收尾（P3/P4/P5）、RFC-0027 文档对齐、Node 产物重生成、Kotlin 重写。
> 第四轮 4 个 agent：**Reed**(gpt-sol，FFI 错误传输)、**Wren**(glm-5.2，六语言错误模型)、**Faye**(gpt-sol，Node/Python)、**Aldo**(glm-5.2，core+RFC)。
> 主进程自查确认：B4/T4/T7 已修，B2/B1/B 未修（见 §13.2）。

### 13.1 远程变更内容

| 提交 | 内容 |
|---|---|
| fe0bb2c | feat(ffi)!: AimuxError out-param 错误传输（breaking） |
| 52383ee | feat(node,python)!: typed error surfaces |
| a9eac52 | feat(bindings)!: 六种 C-ABI 语言错误模型 |
| 70b5a57 / 65b3db8 | RFC-0024 P3+P4+P5：录制集成 session_id/step、session_cache_trajectory、各 binding session_id typed options + 契约测试 |
| 92e7125 / e24368e | RFC-0027 文档对齐 host-side merge + Node 产物重生成 |
| 3b3063e | 修复 binding 遗留同步问题 |

### 13.2 旧 findings 状态总表（远程是否修复）

#### ✅ 已修复（10 项）

| 项 | 状态 | 依据 |
|---|---|---|
| B4 Kotlin 结构断裂 | 已修 | `class Model`(216-565) 结构完整，无孤立顶层方法；✅ 主进程自查 |
| T4/T7 Node recording 导出 + getModelSpecs | 已修 | index.js 已导出 initRecording/initRecordingRing/mockReplay；✅ 主进程自查 |
| T2 Kotlin Sequence null | 已修 | 改为 `Any()` sentinel 哨兵（Model.kt:305-307） |
| T8/D8 Go MockReplay/Trace finalizer | 已修 | 统一走 `wrapHandleU64` 设 SetFinalizer |
| T13 错误信封丢 error_type/status | 已修（5 语言） | Go/Java/Kotlin/Swift/Flutter 均升级为 code+status+retryMs+message+errorValue 五字段；C 枚举与 Rust 18 变体 1:1 且编译器强制 |
| D1 FFI list_models/get_model_specs 绕过共享 runtime | 已修 | 改走 `ffi_block_on` 共享 runtime（lib.rs:1095/1143）；N2 死 map_err 随之消失 |
| D4/D5 错误信封/InvalidHandle 不一致 | 已修 | JSON envelope 移除、统一 `AIMUX_E_INVALID_ARGUMENT` |
| C1 RFC-0027 ResolvedModel 契约 | 已修 | RFC 重写为 RuntimeModel[]，core 零 ResolvedModel 残留 |
| N3-8 RFC-0024 状态 | 已修 | 状态头更新为 IMPLEMENTED (P1-P5) |
| 流式 on_error 旧模型 | 已修 | 新 ABI 移除 on_error 回调，失败走 0+err |

#### ❌ 仍未修（远程未触及）

| 项 | 状态 |
|---|---|
| B1 URL query 凭据泄露 | 未修（http.rs 零改动） |
| B2 Python get_model_specs NameError | 未修（import 块仍缺，L10-69） |
| B3 版本 0.2.1 + [Unreleased] 空 | 未修 |
| T3 Flutter 流式同步阻塞 | 未修（文档自认 follow-up，全局 controller 仍在） |
| T6 Java/Kotlin close 竞态 | 未修（仅原子操作无读锁；Go RWMutex 为对照） |
| T9 Flutter finalizer | 未修 |
| T10/T11/T12 Swift/Java/Flutter recording 校验 | 未修 |
| D7 cap=0 跨语言分叉 | 未修（行为分叉仍在） |
| D6 Python session 半信封 | 未修 |
| A3/A4 Ring/Jsonl 有界性 | 未修 |
| C4-1..C4-9（matcher/流式/replay/trace 逻辑） | 未修（replay.rs 仅 +2 行编译补丁） |
| F1-F5 trace 流 drop/锁/性能 | 未修（layer.rs 零改动） |
| R5/R10 manual/header list_models 漂移 | 未修（RFC-0027 已对齐但下游未跟进） |
| R7/R8 RFC-0023/0026 状态 | 未修 |
| N3-1 rfc0023 plan 矛盾 | 未修 |

#### 🆕 第四轮新 findings

| # | 级别 | 问题 | 位置 |
|---|---|---|---|
| R4-1 | major | **多模态 FFI 构造函数绕过新错误契约**：embedding/speech/image/transcription/files/reranking 构造失败仍无条件 `intern_handle` 返回非零句柄，err 不填充（语言模型路径正确，多模态未跟进） | [lib.rs:1662/1675/1693/1724/1796/1849/1933/2007/2071](aimux-ffi/src/lib.rs#L1662) |
| R4-2 | minor | Swift/Go 错误 status 不补默认值（429/401/404），Java/Kotlin/Flutter 补了——同源错误跨语言 getStatusCode() 不同 | [Aimux.swift:202](bindings/flutter/../swift/Sources/Aimux/Aimux.swift#L202)、[aimux.go:954](bindings/go/aimux.go#L954) |
| R4-3 | minor | Java/Kotlin/Swift 流式实为全量缓冲物化（阻塞调用 + queue），文档暗示增量 | Model.java:381-424、Model.kt:301-329 |
| R4-4 | minor | `session_cache_trajectory` 的 step 是记录到达序（enumerate），与 Recording/SessionStore 的入口步序在不同场景发散，且 ring 驱逐后重编号；文档误导 | [store.rs:498-503](aimux-core/src/trace/store.rs#L498) |
| R4-5 | minor | trajectory 复制 F5 反模式：逐 call_id `records.iter().find` + 持全局 Mutex 全量遍历 | [store.rs:501](aimux-core/src/trace/store.rs#L501) |
| R4-6 | minor | 跨语言错误 model 是各 binding 手写适配（非共享生成），字段名已分叉（Node retryMs vs Python retry_ms）、默认值语义分叉 | 各 binding error 文件 |
| R4-7 | nit | wire-format fixture `with_session_id` 缺 `include_raw_chunks` 字段（存量漂移，本轮改动了相邻字段） | [wire-format.json:64](contract-tests/fixtures/wire-format.json#L64) |

### 13.3 第四轮总体判断

**远程变更质量整体良好**：错误传输重构设计正确（错误码 1:1 枚举 + 编译器强制 + 测试覆盖、所有权契约清晰、UTF-8/NUL 处理到位、无双重释放），Kotlin 重写修复编译阻断，Node/Python typed error 方向正确（async reject 闭环、流式错误不吞）。

**但作用域仅限错误传输**：前三轮 32 项 major 中仅 10 项被修，22 项（含 3 个 blocker：B1/B2/B3）未动；且错误模型是各语言手写、无共享 schema，已出现字段分叉。**发布了错误传输却仍带着 B1/B2/B3 和全部判定/录制正确性 major**，发布风险并未移除。

### 13.4 第四轮优先修复追加

1. **R4-1 多模态构造函数错误契约统一**（与 Reed 建议一致）：所有 `(err out-param) -> u64` 构造函数失败必须返回 0 + fail_ai，补各模态失败测试。
2. **B2 仍是 Python 用户可见 blocker**：一行 import 即可修，强烈建议随错误传输一并发布前修掉。
3. **T6 JVM close 竞态**：引入 Go 式 RWMutex/lease，Java/Kotlin 各三处。
4. **Swift/Go status 默认值对齐**（R4-2）。
5. **trajectory step 语义修正 + 索引化**（R4-4/R4-5）。