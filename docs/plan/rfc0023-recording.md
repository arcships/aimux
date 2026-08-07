# RFC-0023 录制与回放 — 方案对齐记录

> 本文件是 #48(RFC-0023)的**对齐追踪入口**:先对齐方案,再实施。
> 关联:issue [#48](https://github.com/arcships/aimux/issues/48)、[RFC-0023](../../rfc/0023-runtime-request-recording.md)(DRAFT)
> 状态:2026-08-06 对齐完成 + 双模型评审完成,待修订 RFC 定稿后实施
> (P1-P3 曾实施后回滚,代码在 `backup/rfc0023-p1-p3` 分支,仅作参考)

## 流程状态

```
1. 对齐前置依赖与范围   ✅ 完成
2. 评审 RFC-0023 DRAFT  ✅ 完成(gpt-5.6-sol + glm-5.2,结论:需修订后定稿)
3. 逐项确认设计决策      ✅ 完成(B1-B4/C1/D1/D2/D3)
4. 对齐 RFC-0015 sink   ✅ 完成
5. 按评审修订 RFC       ← 当前步骤
6. 锁定实施草案 → 再实施
```

## 双模型评审结论(2026-08-06)

| 评审 | 结论 | 阻塞项 |
|---|---|---|
| gpt-5.6-sol(嘉衡) | 需修订后定稿 | 12 项(见下) |
| glm-5.2(牧云) | 需修订 | 4 项(B-1~B-4) |

**阻塞项(两份评审合并去重)**:

| # | 问题 | 修订决策 |
|---|---|---|
| R1 | RFC 正文与 B1/B3/D3 矛盾(§3.3 独立 trait、§3.5 OnceLock、§11 P6 可选) | 修订 RFC 对齐已定决策 |
| R2 | **敏感头脱敏不完整**:精确匹配漏 `x-goog-api-key`(Google/Vertex 明文落盘) | 脱敏改 contains 式(复用 logging.rs `is_sensitive_key`) |
| R3 | **per-attempt 录制落点**:重试在 `send_with_retry_raw` 内,`send` 只录 1 条 | 录制移入 `send_with_retry_raw` 循环,每次 attempt 一条 |
| R4 | **P4 crate 边界**:自动重建放 core 会循环依赖 | 拆层:`replay_with_model`(core,provider 无关)+ 自动构造(providers/CLI) |
| R5 | **流式终结竞态**:Outcome 即写删 pending,ExchangeUpdate 后到被丢 | writer 用 completion barrier,outcome/exchange 都到齐才写;ExchangeUpdate 带 attempt ID |
| R6 | **输入侧凭据泄露**:CallOptions.headers/provider_options 序列化含 Authorization | 输入侧 + provider_options 统一脱敏(与 HTTP 侧同规则) |
| R7 | **每次调用绑定 recorder 快照**:调用中途替换全局 recorder 会拆散 Recording | 层 A 入口取一次 Arc,随 call_id 传到底层;全局替换只影响新调用 |
| R8 | **mock 回放范围**:原生协议 wire 无法通用解析 | OpenAI 兼容 MVP,其他明确 Unsupported(用户拍板) |

## 已对齐结论(含评审修订)

### D1. P4 请求回放与 RFC-0020 解耦 + 拆层(2026-08-06 修订)

- RFC-0020 只覆盖 OpenAI 兼容协议(其 Non-Goal #1),即使落地也重建不了原生协议 provider——原依赖标注不成立。
- **拆层**(评审 R4 + 用户拍板):
  - `aimux-core::replay::replay_with_model(recording, model, overrides)` — provider 无关,只恢复输入并重发。
  - 自动构造(按 ProviderRecord 重建 provider)放 `aimux-providers`/CLI,后续按 D1 能力边界实现。
- 能力边界:`env:VAR`/`none` 可自动重建;`explicit` 补 key;原生协议传实例。

### B1. ConfigSnapshot = LanguageModel 默认方法(评审确认)

- `LanguageModel::config_snapshot()` 默认方法返回 `ProviderRecord::minimal`;provider 覆盖即完整。
- 装饰器规则(评审补充):transparent decorator(TraceLayer 等)必须转发 inner 快照。

### B2. 落盘线程 = 专用 writer thread(评审推翻 tokio task,用户拍板)

- **同步 `Recorder::flush` 与异步 writer 不匹配**(fire-and-forget 不保证落盘、无 runtime 生命周期问题)。
- 回到 std::thread + std::sync::mpsc(回滚前做法,已验证);flush 用 oneshot 回执确认落盘。
- aimux-core 已有 tokio dev-dep,无需为录制加正式依赖。

### B3. 门控 = RwLock + Option 签名(评审确认)

- `RwLock<Option<Arc<dyn Recorder>>>`,`init_recording(Option<Arc>)` 可替换/关闭。
- **每次调用绑定 recorder 快照**(R7):层 A 入口取一次 Arc,整次调用用同一实例。
- 热路径性能表述务实化(删除"~1ns"绝对数值)。
- 测试仍需 `#[serial]`(全局单例),`serial_test` 已是仓库 dev-dep。

### B4. 流式 outcome = 流结束时补全 + 终结状态(评审修订)

- 包装 stream:Finish → success+finish/usage;StreamPart::Error/item Err → failure;
  无 Finish EOF → incomplete;提前 drop → cancelled/incomplete(用 Drop guard)。
- OutcomeRecord 增加状态枚举(complete/truncated),不用裸 success: bool。

### C1. RingRecorder = 同款样式各自实现(评审确认)

- 同 RingTraceStore 模式(Mutex + VecDeque 有界 + with_capacity + export_jsonl)。
- 内部需 `pending_by_call_id + completed VecDeque`(分片合并后再入 ring);容量语义、单条上限、incomplete 导出规则进实施验收。

### D2. P5 绑定层 = Node + C ABI 本期(评审确认)

### D3. P6 RingRecorder = 纳入本期(评审确认)

## 修订后仍需实施验证的不确定项

- `ConfigSnapshot` api_key_source 的跨 provider 可分类性(回滚前仅 OpenAI 1 家实现)——S-1 建议 config 加来源追踪字段,或 D1 在分类不确定时回退手动兜底。
- `send_with_retry_raw` per-attempt 录制的错误体捕获点(read_error_body)需实现时确认。
- P5 Node/C ABI 的 model handle 所有权(C ABI 不直接表达借用型 Option<&dyn LanguageModel>)。

## 实施进度

| 阶段 | 状态 | 备注 |
|---|---|---|
| P1 录制核心 | ✅ 已合并(PR #85) | completion barrier + transport_closed、专用 writer thread + oneshot flush + Drop join、脱敏超集、call_id 统一、complete 标记、ISO 时间戳 |
| **P2 层 B HTTP 录制** | ✅ 已实施(双模型二次评审通过) | per-attempt exchange、流式骨架 + patch 补全(Drop 幂等)、R7 recorder 快照(RecordingContext 透传)、UTF-8 安全截断、OpenAI/Anthropic/Azure/Codex call_id 透传、429→200 per-attempt 测试 |
| **P3 mock 响应回放** | ✅ 已实施 | `MockReplayModel`(OpenAI 兼容 MVP)+ `ReplayMatcher`(`ExactMatcher`/`ScoreMatcher`)+ `from_jsonl` + 10 单测 |
| **P4 请求回放** | ✅ 已实施 | `replay_with_model`(core,provider 无关)+ `ReplayOverrides` + `rebuild_provider`(aimux-providers,OpenAI 兼容族)+ `tools/aimux-replay` CLI(`--dry-run`/`--api-key`/`--prompt`/`--call-id`) |
| **P5 绑定层透传 + matcher + 脱敏验证** | ✅ 已实施 | C ABI:`aimux_init_recording`/`aimux_init_recording_ring`/`aimux_recording_stop`/`aimux_recording_flush`/`aimux_mock_replay_new`;Node:`initRecording`/`initRecordingRing`/`recordingStop`/`recordingFlush`/`mockReplay`;`PrefixMatcher`(消息级前缀,最长命中)+ 录制边界脱敏端到端验证 |
| **P6 RingRecorder** | ✅ 已实施 | 内存有界 ring(默认 2048)+ FIFO 淘汰 + `dropped_count` 可查 + `pending`/`completed` 分片 + completion barrier + `export_jsonl`/`clear` + 5 单测;`RingTraceStore` 同款样式各自实现 |
| **config_snapshot 覆盖** | ✅ 已实施 | OpenAI 兼容族(OpenAIModel/OpenAIResponsesModel,经 OpenAIConfig `api_key_source` 字段,覆盖 251 注册表 provider)+ 20 本地 wrapper 标 `none` + Anthropic/Google/Azure/Codex/Mistral/Cohere 原生族;profile/provider_options 可重建,round-trip 测试 |
| 队列语义 | ✅ 已覆盖 | RingRecorder 即 bounded + drop-newest + 计数;JsonlRecorder 保持 unbounded(专用 writer thread,文档说明) |

## 参考

- 回滚前的 P1-P3 实施代码:分支 `backup/rfc0023-p1-p3`(3 个提交,仅作方案参考,不直接复用)
