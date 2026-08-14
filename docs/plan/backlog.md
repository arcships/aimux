# Backlog — 非阻塞发现 / 后续项

> 按计划评审与开发过程中产生的非阻塞项归档于此。阻塞项必须当轮修复，不得入此表。

## RFC-0017 阶段 2 相关（v3：完全用户定义）

| # | 来源 | 内容 | 状态 |
|---|---|---|---|
| B1 | 调研 batch-04 | `reasoning_effort` 档位差异（kimi-k3 low/high/max、perplexity 四档）——档位不翻译（透传+厂商自决），知识进用户手册 | 已闭合（文档化） |
| B2 | 调研 batch-05 | `reasoning_format`（stepfun general/deepseek-style）硬编码 `provider=="groq"`——用户 bodyOverrides 表达，手册文档化 | 已闭合（手册 §3 StepFun/Groq 行） |
| B3 | 调研 batch-06 | venice_parameters/hetzner chat_template_kwargs 等封闭字段——用户 bodyOverrides，手册文档化 | 已闭合（手册 §3 Venice/Hetzner 行） |
| B5 | 调研全局 | 7 处 registry base_url 确定性错误——**已修复**（stage2-004，merge 7a84fb4）；~20 处存疑（novita/nous_research/longcat/moonshotai_cn 等）仍待实测 | 🔶 存疑部分待实测 |
| B6 | 调研 batch-02 | DeepSeek 是否接受 `max_completion_tokens` 待实测——实测后决定 DeepSeek 是否加 `max_tokens_key="max_tokens"`（v3 下它已无特化，纯通用路径，优先级降低） | 🔶 未实测（2026-08-06 复核：仍走默认推断路径，手册 §4 已标注现状；实测需真实 API 调用） |
| B8 | 调研 batch-03 | Heroku 的 `allow_ignored_params` 机制——max_tokens_key 之外的特殊参数行为，用户 bodyOverrides + 手册文档化 | 已闭合（手册 §3 Heroku 行，含 bodyOverrides 表达） |
| B9 | verify stage2-001 F7 | 厂商级 `max_tokens_key` 数据接线未做（6 家 `"max_tokens"` + groq/heroku `"max_completion_tokens"`）——**须在 stage2-002/003 收尾前接线**，否则 groq/heroku 修复对真实流量不生效 | 已闭合（stage2-002 实施：registry 8 家接线 + provider.rs 测试锁定 + 手册 §4） |
| B10 | verify stage2-001 F8 | `Some("max_tokens")` + 显式 `maxCompletionTokens` 被静默丢弃——行为方向符合设计，但需在用户手册标注（用户应改用 `max_output_tokens`） | 已闭合（手册 §4 标注 2026-08-06） |

## 已闭合（v3 变更消除）

| # | 原条目 | 闭合原因 |
|---|---|---|
| B4 | 不可关模型按 model 前缀分派（kimi-k2.7-code 等） | 不内置任何映射——无需分派，用户 bodyOverrides + warning 兜底 |
| B7 | alibaba 内置 EnableThinking 对 qwen3-thinking-2507 的 by-model 误伤 | 不内置——无误伤面 |

## Round 4 质量审计（2026-08-14，基线 cf2cea5）

> 来源：docs/quality-audit/round4/（SUMMARY.md 汇总）。全部已 Issue 化（#110–#125）并逐项独立校验，校验修正已回写（SUMMARY §8）；下表数字为校验后口径。

| # | 优先级 | 内容 | 状态 |
|---|---|---|---|
| R4-H1/H2 | P0 | recording writer 磁盘 I/O 错误静默 + completion barrier 缺 input 条件（取证失真，recording.rs:829/94） | 🔶 待修 |
| R4-H3 | P0 | google 非流式 tool_result 用 tool_call_id 冒充 functionResponse.name → 多轮工具调用 400（google/convert.rs:228） | 🔶 待修 |
| R4-H4 | P0 | anthropic 流错误不发 Finish part，违反 Final chunk 契约（anthropic/stream.rs:481） | 🔶 待修 |
| R4-H5 | P1 | JSON 线格式解析器 FFI/Python/Node 三份等价、"空/null=默认"约定 20 份拷贝——建共享 wire 层（4d 规则表） | 🔶 待修 |
| R4-H6 | P1 | http.rs:177 Client 构建 expect（panic=abort 放大）；mutex "中毒 DoS" 经校验不成立，降为锁错误处理统一（#115） | 🔶 待修 |
| R4-H7 | P0 | bedrock 流式丢弃 reasoning signature（注释矛盾掩盖）；anthropic 流式同类缺陷（校验扩大，#113） | 🔶 待修 |
| R4-H8 | P1 | Kotlin nextPart 超时哨兵死代码，retryable 契约破坏（Multimodal.kt:303） | 🔶 待修 |
| R4-T1 | P1 | coverage CI job（报告模式）+ rustdoc 47 error（4 crate，校验修正）修复后入 CI（#117） | 🔶 待做 |
| R4-T2 | P1 | RFC-0028 错误路径补测（connect 失败/error 事件/peer close/双超时/abort；8 绑定无行为测试） | 🔶 待做 |
| R4-T3 | P1 | 统一 e2e 套件扩协议（各家已有独立 wiremock 测试）；FFI 全导出冒烟遍历；c 绑定零测试、swift e2e 命名统一（校验修正，#119） | 🔶 待做 |
| R4-T4 | P1 | clippy lint 子集永久开启（uninlined_format_args/must_use_candidate/return_self_not_must_use/redundant_closure_for_method_calls/missing_errors_doc，≈1,721 处；needless_lifetimes 校验为 0 处移出）（#120） | 🔶 待做 |
| R4-S1 | P2→P1 | Chat 族 3 份 SSE 引擎下沉（cohere 不宜并入）；vertex 功能缺失（工具可启用但流式结果静默丢弃，校验升级 medium）；top_level_media_type 9 处收敛；净删 600–900 行（#121） | 🔶 待做 |
| R4-S2 | P2 | ffi/lib.rs（3,767 行）/ recording.rs / http.rs 拆分；266 行 rebuild_stream_result | 🔶 待做 |
| R4-S3 | P2 | 依赖清理 5 处（bytes 校验为整体移除）；dead_code 清单校验修正（item 10 + 模块级 9 + 测试 5；store.rs:73 属过期属性）（#123） | 🔶 待做 |
| R4-S4 | P2 | 15 个零覆盖本地 provider registry 化或补 smoke；contract-tests fixtures 扩面 | 🔶 待做 |
| R4-F1 | P1 | SigV4 签名 host 头缺端口（真实签名缺陷）+ 环境代理触发回环 502；"时间敏感"机理经校验推翻；cassette_full 移出（#125） | 🔶 待做 |
| R4-D1 | P1→#135 | 绑定层跟进：reasoning signature round-trip 核验（8 语言）+ toolName 透传测试 + #128 旧断言核验 | 🔶 待做 |
| R4-D2 | P1→#136 | FFI 暴露 recording try-flush（aimux_recording_flush 恒 0，Write 错误对绑定用户不可见） | 🔶 待做 |
