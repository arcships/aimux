# P4a:aimux-core 大模块深审(recording / replay / session / trace-store)

- 基线:master @ cf2cea5(`/tmp/aimux-audit-master`,只读)
- 对象:`aimux-core/src/recording.rs`(2246 行,生产 1-1388)、`replay.rs`(2002 行,生产 1-1023)、`session.rs`(708 行,生产 1-444)、`trace/store.rs`(918 行,生产 1-682)
- 日期:2026-08-14(Round 4 子任务 4a)
- 上下文:workspace release profile `panic = "abort"`(根 Cargo.toml:30)——release 下任何 panic 直接终止宿主进程(FFI 致命);dev/test 默认 unwind,存在锁中毒级联面

## 概述

四个文件合计 5874 行(生产约 3535 行),承载 RFC-0015/0023/0024 的录制-回放-归组-审计链路。总体质量高于本仓库平均水平:

- **错误处理**:生产代码 34 处 `unwrap` 全部是 `lock().unwrap()`(锁中毒类)+ 2 处结构上不可能失败的 `get_mut().unwrap()`/`expect`;**没有任何一处可由外部数据直接触发的 unwrap/panic**。replay.rs 生产代码 0 unwrap / 0 expect / 0 let _ =,是四者中最干净的。
- **真正的风险不在 unwrap,而在静默**:writer 磁盘 I/O 错误被完全吞掉(违反 flush 落盘契约)、`JsonlRecorder::new` 失败降级后环境变量开启仍静默不录、completion barrier 与 RFC-0023 存在偏差(Input 丢失仍标 `complete=true`)。
- **精简度**:replay.rs 的 `rebuild_stream_result` 266 行是最大债务;writer 线程与 RingRecorder 双份实现同一套 C4-6/C4-7 合并语义;recording/session 逐字重复时间戳与 call_id 工具。
- **注释质量**:why 注释占比很高(抽样约 70%),大量引用评审决策编号(A4/A9/C4-7/N11…)并可交叉验证;但发现 3 处**与当前行为相反的过时注释**(replay.rs 两处脱敏描述、session.rs 一处功能边界声明)。

## 方法

1. 通读 4 个文件全部生产代码与内嵌测试;以 `#[cfg(test)]` 行切分生产/测试,`grep` 复核 unwrap/expect/let _ =/.ok()/unwrap_or_default 逐行清单。
2. 逐处判定 unwrap 触发条件:外部数据可达性 → 锁中毒可达性 → 结构不可失败;结合 `panic=abort` 区分 release/dev 两个 panic 语义。
3. 与 `rfc/0023-runtime-request-recording.md`(barrier 定义 §R5/行 156、ring 容量 §3.5、脱敏规则表、§3.6.1 请求回放)逐条比对代码与注释;与 `aimux-core/tests/`(recording_e2e_test.rs、session_test.rs、m7_aggregation_test.rs)比对覆盖面。
4. 函数长度以定义行区间计算;注释抽样 22 处按 why(解释动机/约束/教训)/what(描述行为)/漂移(与实现矛盾)分级。

## 发现列表

严重度:H = 正确性/panic 风险;M = 显著可维护性/潜在运行时风险;L = 风格。行号均指基线 cf2cea5。

### 一、错误处理(目标 5)

**F1 [H] writer 磁盘 I/O 错误全静默,`flush`/`try_flush` 违反落盘契约**
- `recording.rs:829-834`(`write_line`:`if let Ok(line) = to_string` 吞序列化失败;`let _ = writeln!`、`let _ = w.flush()` 吞 I/O 失败)、`recording.rs:782-787`(Flush 事件同样 `let _ = w.flush()` 后无条件 `ack.send(())`)。
- 证据:ENOSPC/权限丢失时,`try_flush()` 对调用方返回 `Ok(())` 但一行未写;`dropped_count()` 也不计入(A4 计数只覆盖通道溢出)。对以"崩溃前取证"为卖点的子系统,这是契约级失败。
- 建议:`write_line` 返回 `std::io::Result` 由 writer 累计 `write_errors` 计数(与 `dropped` 同级可查);`Flush` 事件把最后的 flush/写错误带回 ack(oneshot 改传 `Result`,超时语义不变)。改动局部于 writer 线程,风险低。

**F2 [H] completion barrier 与 RFC-0023 偏差:Input 丢失仍写 `complete=true` 行**
- `recording.rs:94-98`(`ready()` 只检查 `transport_closed && outcome != Pending && exchanges 全 finalized`,无 input 条件)+ `recording.rs:804-813`(`try_finalize` 置 `complete=true`)。
- RFC-0023 行 156 定稿 barrier 为 "input ✅ + outcome ✅ + 全部 exchange 已终结";代码与模块文档(recording.rs:13)都省略了 input。叠加 A4 drop-newest(通道 1024 满即丢),负载下 `Input` 事件被丢弃是**已文档化的正常工况**——此时写出的行以 `entry_or_init`(recording.rs:689-703)的空 prompt + `ProviderRecord::minimal("","")` 占位,却标 `complete=true`,且不进 `inconsistent` 集合(C4-7 只覆盖 attempt 重复/update 未命中)。取证数据失真且无标记。
- 建议:`ready()` 增加 `input 已填充` 判据(如 `InputRecord` 引入 `populated: bool` 或以 `!provider.provider.is_empty()` 近似);或至少在 Input 缺失时写行前 `mark_inconsistent`。同步修订 recording.rs:13 的 barrier 描述。

**F3 [M] replay 流式 tool_call 缺 `index` 时静默并入 0 号累积器,可拼接出错误的 ToolCall**
- `replay.rs:764-770`:`dtc.get("index").and_then(as_u64).map(|n| n as usize).unwrap_or(0)`。
- 证据:OpenAI 规范 chunk 必带 index,但录制文件是**外部输入**(可手写/可来自非规范兼容层);两条都缺 index 的不同 tool 的 `arguments` 会串接进同一个 accumulator,产出错误 `input`(JSON 失效再退化为字符串,全程无警告)。
- 建议:缺 index 或 index 非法时记入 parts 的 `warnings`(StreamStart 已有该字段)或直接返回明确错误,与文件内 A7 "不猜测解析" 的立场对齐。

**F4 [M] `JsonlRecorder::new` 降级 + env 初始化:`AIMUX_RECORD=1` 可静默完全不录**
- `recording.rs:544-558`(`new` 失败降级为 `tx=None` 的 no-op,`dropped` 不再计数)、`recording.rs:356-363`(`init_recording_from_env` 用 infallible `new`,且无论成败返回 `true`)。
- 证据:`AIMUX_RECORD_DIR` 指向非法路径(如某文件的下级)时,录制"开启成功"但零输出、零计数、零日志。A9 已提供 `try_new`,env 入口却未用。
- 建议:`init_recording_from_env` 改走 `try_new`,失败时走 RFC-0014 日志告警(或至少返回 false);`disabled` 状态在 `dropped_count` 之外暴露 `is_disabled()`。

**F5 [M] 锁中毒策略跨模块不一致;dev(unwind)下存在级联面**
- 34 处生产 unwrap 全景:recording.rs 15(全部 `lock().unwrap()`,另 :1113 `expect("just inserted or present")` 为插入后 `get_mut`,结构不可失败)、session.rs 10(:151 `get_mut().unwrap()` 同为插入后取值,**[justify:不可能失败且无外部输入]**;其余 9 处锁)、trace/store.rs 9(全锁)。**无一处由外部数据直接触发**;release(panic=abort)下锁中毒不可能发生(首个 panic 即 abort),判定 [justify:不可达];dev/test(unwind)下任一持锁 panic 会毒化锁并使后续每次调用级联 panic——但四个文件的持锁区均无 panic 源,实际可达性极低。
- 不一致证据:`recording.rs:349`(init_recording 用 `if let Ok(mut g)` 容忍中毒)、`recording.rs:913`(mark_inconsistent 同样容忍)vs `session.rs:323/328/336/341/369`、`trace/store.rs:394-591`、recording.rs 其余全部 `.unwrap()`。同一 crate 两种中毒哲学。
- 建议:统一策略并文档化——推荐全部保留 `unwrap()` + 在 crate 级文档声明"锁中毒仅在 unwind 且持锁区 panic 时发生,release 为 abort";或提供 `fn lock_unpoisoned<T>(m:&Mutex<T>) -> MutexGuard` 统一封装。禁止混用。

**F6 [M] `let _ =` / `.ok()` / `unwrap_or_default` 信号盘点(9/7/29 处)**
- recording.rs 9 处 `let _ =`::312(trait 默认 no-op,无害)、608(Drop 中 `handle.join()` 忽略——writer 线程无 panic 源,可接受但 panic 时不可观测)、680(`flush()` 吞错,**已有 `try_flush` 补救,已注释,可接受**)、784/786/800/831/832(writer I/O,见 F1,**吞掉关键信号**)。
- recording.rs 7 处 `.ok()`::232/235/1328/1331(serde_json to_value,自有序列化,实际不可失败)、379(`RECORDER.read().ok()?` 静默把毒锁当"录制关闭"——与 F5 同源)。判定:无一处吞外部输入错误。
- replay.rs 24 处 `unwrap_or_default`:8 处在 `canonical_call_key`(:93-102)为自类型序列化兜底(实际不可失败,但失败会双侧同退化为 null 造成伪命中理论面);16 处在 `canonical_recording_key`/body 解析为**刻意的向后兼容缺省语义且已注释**(replay.rs:106-107),判定合理。
- trace/store.rs `:557` `u128::from_str_radix(h,16).unwrap_or(0)`:`TraceRecord` 可从外部 JSONL 反序列化,非法指纹静默变 0(进入索引但 128 位校验必拒,表现为 LCP 静默 miss)——建议计数或 debug_assert,见 F8。
- session.rs 4 处 unwrap_or_default 均为查询空缺省,合理。

**F7 [M] aggregate O(n²) 与 session 查询线性 find**
- `trace/store.rs:445-450`:每个 (provider,model) 分组再全量扫描 `inner.records`(2048 × 组数);`trace/store.rs:471-475` 与 `:519-522`:`records.iter().find(|r| &r.call_id == id)`,每 session 每步一次线性扫描(64 × 2048 次字符串比较)。
- 建议:`by_call_id: HashMap<String, usize>`(或直接存 Arc 索引)随 append/驱逐维护;aggregate 改单遍累计 per-group 的 est。收益:查询从平方降到线性;风险:驱逐联动需测试(现有 m7_aggregation_test.rs 可护航)。

**F8 [M] `from_str_radix` 失败静默为 0(外部反序列化数据)**
- `trace/store.rs:557`。证据见 F6。建议:`unwrap_or(0)` 改为过滤 + 计数(`fingerprint_parse_errors`),保持"宁可 abstain 不可错报"的 RFC-0015 立场。

### 二、精简优雅(目标 3)

**超长函数清单(>80 行)**
| 函数 | 位置 | 行数 | 问题 |
|---|---|---|---|
| `rebuild_stream_result` | replay.rs:647-913 | **266** | SSE 解析 + 文本/reasoning/tool 三状态机 + 收尾 + A7 判定混杂;finish 收尾块(818-855)与流末收尾块(860-890)逐字重复 |
| `writer_loop` | recording.rs:710-801 | 91 | 8 路事件 match + 合并 + 兜底;尚可读但已到阈值 |
| `rebuild_generate_result` | replay.rs:544-625 | 81 | 刚过线,tool_calls 解析可独立 |

**重复模式**
1. **writer 线程 vs RingRecorder 双实现同一合并语义**:`insert_exchange`/`apply_exchange_update`/`mark_inconsistent`/finalize barrier 在 `writer_loop`(recording.rs:749-781)与 `RingRecorder::record_*`(recording.rs:1174-1234)各写一遍,C4-6/C4-7 修复需双处同步——本次审计已核对两侧一致,但没有测试强制这一点。
2. **时间戳/call_id 工具逐字重复**:`new_call_id`/`rfc3339_now`(session.rs:397-439)与 `new_call_id`/`iso8601_now`/`format_rfc3339_utc`/`civil_from_days`(recording.rs:384-393, 930-970)同算法两份,且**各自持有独立 `static CALL_SEQ`**(recording.rs:384 / session.rs:397)——同格式 `call-{ns}-{seq}` 双计数器,理论撞 ID(recording.rs:944 自己承认"与 session::rfc3339_now 同格式同算法")。
3. replay.rs 三个 Matcher(Exact/Score/Prefix)重复 provider/model_id 字段与过滤前缀。
4. recording.rs 两个 Recorder 的 `record_provider` 脱敏块(631-635 / 1186-1189)重复。

**Top-5 重构建议(收益/风险)**
1. **抽取 `ShardMerger`**:`HashMap<String, Recording>` + `insert_exchange`/`apply_update`/`finalize`/`mark_inconsistent` 收敛为单结构,writer_loop 与 RingRecorder 共用。收益:消除双实现漂移(直接服务 F2 修复);风险:低,纯内部,现有 C4-6/C4-7 测试可直接复用。
2. **拆 `rebuild_stream_result`** 为 `parse_sse_blocks`(事件循环)+ `StreamRebuildState`(text/reasoning/tool 累积)+ `emit_pending_closures`(消除重复收尾)。收益:266 行 → 3×~80,收尾逻辑单点;风险:中,流语义需 C4-8 系列测试护航(现已较全)。
3. **时间戳/call_id 下沉 `util.rs`**:合并两份 civil_from_days 与 CALL_SEQ。收益:DRY + 单一 ID 计数器消除理论撞号;风险:低(两个调用点,测试已有 RFC3339 金测)。
4. **trace/store.rs 查询索引化**(对应 F7)。收益:大 ring 下查询可用性;风险:低-中,驱逐联动。
5. **统一 Recorder 脱敏/入队样板**:provider 快照脱敏 + send_ev 包装为 helper 或宏。收益:小;风险:低。

### 三、注释质量(目标 4)

抽样 22 处分级(why = 解释动机/约束/教训;what = 仅描述行为):

| # | 位置 | 内容摘要 | 分级 |
|---|---|---|---|
| 1 | recording.rs:6-15 | 模块头:设计性质逐条 + 定稿日期 | why |
| 2 | recording.rs:91-93 | barrier 防早写 | why |
| 3 | recording.rs:397-410 | token 脱敏教训(误伤用量字段) | why(优秀) |
| 4 | recording.rs:484-488 | 有界通道 drop-newest 动机 | why |
| 5 | recording.rs:505-507 | try_new 替代 expect 的历史 | why |
| 6 | recording.rs:688 | 兜底建条目原因(乱序) | why |
| 7 | recording.rs:724-727 | C4-6 防覆盖 | why |
| 8 | recording.rs:875 | finalized 不回退 | why |
| 9 | recording.rs:940-944 | N11 时间戳教训 | why |
| 10 | recording.rs:594-597 | path() 描述 | what |
| 11 | recording.rs:30-31 | schema 用途 | what |
| 12 | replay.rs:127-140 | 脱敏感知比较保守原则 | why |
| 13 | replay.rs:172-175 | "max_output_tokens 会被脱敏" | **漂移(见 F9)** |
| 14 | replay.rs:528-536 | 非法 JSON 回退与正向一致 | why(优秀) |
| 15 | replay.rs:679-700 | C4-8 流式语义 | why |
| 16 | session.rs:152-155 | step 单调不随淘汰回退 | why |
| 17 | session.rs:174-177 | 未知 session 不 touch 的原因 | why |
| 18 | session.rs:1-18 | 模块边界声明 | why |
| 19 | session.rs:389-391 | "trajectory 不在本里程碑" | **漂移(见 F9)** |
| 20 | store.rs:166-175 | C4-1 不跨记录拼接 | why |
| 21 | store.rs:253-256 | 墓碑 25% 重建阈值 | why |
| 22 | store.rs:373 | Arc2 命名原因 | what(略怪,见 L6) |

比例:why 18 / what 3 / 漂移 2 ≈ **why 占 82%**,且多数绑定可追溯的评审编号——显著优于常见水平。问题集中在过时注释:

**F9 [M] 三处注释与当前行为相反**
- `replay.rs:130`:脱敏键清单含 "token"——recording.rs:405-410 已移除 token contains(仅 `x-amz-security-token` 精确匹配),清单过时。
- `replay.rs:172-175`:断言 "`max_output_tokens` 等含 token 子串字段在录制侧会被 `redact_json` 脱敏"——**与 recording.rs 现行为直接矛盾**(该注释描述的是移除 contains("token") 之前的旧行为;测试 replay.rs:1262-1264 注释同病)。
- `session.rs:389-391`:"`session_cache_trajectory` 有意不在本里程碑"——已落地于 trace/store.rs:513(RFC-0024 §10 P4),边界声明过时。
- 另:recording.rs:1 模块头标题仍写 "(P1 — 数据模型 + Recorder trait + 门控 + JsonlRecorder)",实际已含 P3 session 字段、P6 RingRecorder、B4 流观测——范围漂移(L 级)。
- 建议:修正三处主文 + 模块头;后续约定"行为修正 PR 必须同步 grep 旧断言"(本仓已有该习惯,如 N11,此三处是漏网)。

**RFC-0023 一致性核对**
- 一致:writer thread + oneshot flush(§3.5 行 262)、RingRecorder 默认 2048(行 263/489)、env 变量(行 255)、§3.6.1 请求回放语义与 `[REDACTED]` 重发警示、脱敏"超集"自述(行 484)。
- 偏差:①barrier 缺 input 条件(= F2);②脱敏实现相对 RFC §377 "禁止精确匹配清单" 使用了两个 exact 匹配(`key`、`x-amz-security-token`)——代码注释已给出充分理由(避免误伤 X-Key/monkey、用量字段)且无泄露路径,但 RFC 表格未回写该修订。

**module `//!` 与行为漂移**:recording.rs(标题 P1 范围,见 F9)、replay.rs(无漂移)、session.rs(F9 第三条)、store.rs(无漂移)。

### 四、语言规范(目标 1)

**F10 [L] 不惯用写法清单**
- `replay.rs:395` `from_jsonl(path: &str)` → 应为 `impl AsRef<Path>`;同文件 `MockReplayModel` 把 provider/model_id 同时存于自身与 matcher(:367-375,`provider.clone()` 双份)。
- `replay.rs:244-249` `message_text` 返回 `String`(每次比较两次分配),可 `Cow<'_, str>` 或 `&str`。
- `trace/store.rs:374` `use std::sync::Arc as Arc2;` 别名避讳枚举变体名,不如重命名冲突源。
- `trace/store.rs:95/383` `assert!(cap > 0)` 无消息;session.rs:110/254 带消息——同 crate 风格不一。
- `recording.rs:933` 函数名 `iso8601_now` 实产 RFC 3339(自身文档 :932 也说 RFC 3339),与 session.rs `rfc3339_now` 命名分叉。
- `replay.rs:176-196` `canonical_keys_match` 对象键集比较手写(serde_json::Map 已有 keys 迭代,可 `map.keys().eq`),可读性小胜;非必须。
- `recording.rs:726-728` or_insert_with 闭包内 `input.clone()/provider.clone()` 后立即覆盖 `rec.input = input`——正确但多一次深拷贝,可用 `entry().or_default()` + 条件初始化。
- store.rs `insert`(:132)`rec.clone()` 后又迭代 `rec`——可先取出 hashes 避免(微)。
- 测试侧:replay.rs 测试 `rec.exchanges[0].response.as_mut().unwrap()` 重复 12+ 次,应抽 helper(不影响生产,顺带)。

**F11 [L] `rebuild_stream_result` 未见 finish_reason 时默认补 Stop 未注释**
- `replay.rs:899-906` `final_finish.unwrap_or(Stop)`。建议一行 why(与 provider 对齐或保守默认),避免读者误以为是遗漏。

**F12 [L] per-scope 淘汰与 `records` 队列不联动的语义耦合未声明**
- `trace/store.rs:147-164`(TraceStore 内淘汰只清 ring 槽,不清 `Inner.records`/`by_session`)——aggregate/session 查询仍可见该记录。行为上可辩护(索引淘汰 ≠ 保留淘汰),但依赖读者推断;建议在 `append`(:545)加两行 why。

## 统计

| 维度 | 数值 |
|---|---|
| 审查文件 / 生产行数 | 4 / ~3535(recording 1388、replay 1023、session 444、store 682) |
| 生产 unwrap / expect | 34(全为 `lock().unwrap()`)/ 2(均结构不可失败);其中 replay.rs 0/0 |
| 可由外部数据触发的 panic | **0** |
| let _ = / .ok() / unwrap_or_default(生产) | 9 / 7 / 29(replay 24、session 4、recording 1) |
| 吞掉关键信号的静默点 | writer I/O(F1)、new 降级(F4)、radix 失败(F8)共 3 处 |
| 超长函数(>80 行) | 3(rebuild_stream_result 266、writer_loop 91、rebuild_generate_result 81) |
| 注释抽样 | 22 处:why 18 / what 3 / 漂移 2(另模块头范围漂移 1) |
| 发现计数 | **H 2 / M 9 / L 3**(F1-F12,其中 F5/F6 为盘点型) |
| 测试现状 | recording 单测 30+ 用例 + e2e(recording_e2e_test.rs 235 行)覆盖 barrier/脱敏/溢出/乱序;session_test.rs 覆盖归组主链路;store.rs 的 lookup 语义(C4-1/TTL/墓碑)覆盖良好,aggregate 由 m7_aggregation_test.rs 覆盖;缺口:通道溢出导致的 Input 丢失端到端、try_flush 超时、writer 磁盘错误 |

### 结论优先级

1. 先修 F1/F2(取证契约,改动小、测试可加);2. F4 env 静默与 F3 replay 防御;3. F9 注释纠偏(半小时级);4. F5 锁策略统一与 F7 查询索引随下一轮重构;5. 二节 Top-5 重构按序排期,`ShardMerger` 与 util 合并风险最低可先行。
