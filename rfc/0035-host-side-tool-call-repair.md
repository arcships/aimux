# RFC-0035: 宿主侧工具调用修复(host-side tool-call repair)

> **Status**: Implemented
> **Date**: 2026-09-22
> **Scope**: 让 7 个绑定拿到等价于 AI SDK `repairToolCall` 的能力,而不引入回调、句柄或会话
> **Related**: [RFC-0016](0016-align-with-aisdk.md) §7.5(agent loop 明确不做)、[#186](https://github.com/arcships/aimux/pull/186)(函数指针回调,已否决)、[#191](https://github.com/arcships/aimux/pull/191)(有状态 operation session,已否决)

---

## 1. 问题

`GenerateTextOptions.repair_tool_call` 是一个 `#[serde(skip)]` 闭包:只有 Rust 调用方能用。
绑定无法把函数塞进 JSON,于是 Node / Python / Go / Java / Kotlin / Swift / Flutter 七家全部缺失这一能力(见 [docs/api/gaps.md](../docs/api/gaps.md) §8)。

前两次尝试都失败在同一个地方——**把控制流搬过 ABI**:

- **#186:函数指针回调**。宿主注册 `extern "C"` 回调,core 在 `parse_tool_call` 里回调过去。问题:回调在 FFI 线程栈上同步执行,而宿主的修复通常是**再发一次 LLM 请求**(异步)。要么宿主在回调里阻塞(撞上 `ffi_block_on` 的重入保护),要么 core 得把 future 挂起并交回控制权——那就是 #191。
- **#191:有状态 operation session**。core 持有一个暂停中的生成操作,通过 `Event`/`Reply` 与宿主往返。问题:句柄生命周期、超时、取消、泄漏、7 家绑定各自的资源管理,全部翻倍;为一个一次性、纯数据的判断引入了一整套会话协议。

## 2. 为什么宿主侧后处理就够

三个既成事实合在一起,让"暂停生成"这件事根本没必要:

1. **core 的 `generate_text` / `stream_text` 是单次模型调用**。没有多步循环,没有工具执行——agent loop 按 RFC-0016 §7.5 明确不在 core 范围内。
2. **非法工具调用是数据,不是失败**。`parse_tool_call` 从不让生成失败:解析/校验不过的调用以 `ToolCall { invalid: true, error }` 的形态返回。
3. **同一次调用里没有任何后续逻辑依赖这个结果**。它只被放进 `GenerateTextResult.tool_calls`、`response_messages`(经 `ResponseMessageBuilder::tool_call`)和 `StreamPart::ToolCall`。

所以修复不需要发生在生成过程**之中**。宿主照常调用现有入口(不传任何钩子),在结果里看到 `invalid` 的调用,用自己的语言修,再调一个**纯函数**把结果算出来。
跨边界的只有 JSON:没有句柄,没有回调,没有会话,不新增 crate。

## 3. 线上形状

### 3.0 入参:宿主已经有的那两个字符串

三个导出都**不**让宿主自己凑 `tools` / `messages` / `instructions`——它们直接吃
宿主发给 `generate_text` / `stream_text` 的同一份 `prompt_json` + `opts_json`:

```
aimux_tool_call_repair_context(tool_call_json, prompt_json, opts_json, &out)
aimux_apply_tool_call_repair(tool_call_json, opts_json, reply_json, &out)
aimux_apply_tool_call_repair_to_result(result_json, opts_json, tool_call_id, reply_json, &out)
```

prompt → messages 的规则(含 `{"prompt": …}` 包装)、instructions 与 messages 的
并列关系、以及 `opts.tools` 的取法,全部由 core 的 `tool_call_repair_inputs`
(复用 `generate_text` 自己的 `split_prompt`)完成。否则这段逻辑要在 7 家绑定里各写一遍。

**没有 tool set 时不是错误**:`tool_call_repair_context` 返回 JSON `null`。
AI SDK 的规则是"没给工具集的调用从不修复",宿主看到 `null` 就跳过这个调用。
这个判断落在 core(`tools: Option<&[Tool]>` → `Option<Value>`,与 `parse_tool_call`
自身的参数形状一致),不在每个导出里重写一遍。两个 `apply_*` 反过来把
`tools == None` 视为 `InvalidArgument`——遵守了 `null` 的宿主根本走不到那里。

### 3.1 修复上下文(`tool_call_repair_context`)

对齐 AI SDK `repairToolCall` 的入参:

```json
{ "tool_call": { "tool_call_id": "call-1", "tool_name": "weather",
                 "input": "{\"town\":\"Singapore\"}", "dynamic": true },
  "error": { "InvalidToolInput": { "tool_name": "weather", "tool_input": "…", "cause": "…" } },
  "input_schema": { "type": "object", "required": ["city"], "…": "…" },
  "tools": [ … ], "messages": [ … ], "instructions": null }
```

`tool_call.input` 是 provider 的**原始参数文本**(模型吐出来的那串),不是解析后的对象——这正是修复函数要看的东西。原文保存在调用携带的 `InvalidToolInput.tool_input` / `NoSuchTool.tool_input` 中;不能从解析后的 JSON 值反推,因为解析会丢失字符串引号、重复 key、数字表示与空白。

`error` 直接取自调用自身的 `error` 字段,不重新推导。**这是宿主必须持有非法 `ToolCall` 原件的原因**,也是唯一的信息损失点:非法调用的 `dynamic` 一律被置为 `Some(true)`,原值不可恢复。对本流程无影响(原始 error 是读出来的,不是重算的)。

### 3.2 宿主答复(`ToolCallRepairReply`)

内部 tag、snake_case、`deny_unknown_fields`:

```json
{"type": "repaired", "tool_call": { "…": "RawToolCall" }}
{"type": "unchanged"}
{"type": "failed", "message": "repair model unavailable"}
```

语义与闭包路径逐字对应,**并且共用同一段分支代码**(`RepairOutcome` + `apply_repair_outcome`):

| reply | 闭包等价物 | 结果 |
|---|---|---|
| `repaired` | `Ok(Some(call))` | 重新解析+校验;通过则有效调用,不通过则 `ToolCallRepair { original_error, cause }` |
| `unchanged` | `Ok(None)` | 保留原始 error 的非法调用 |
| `failed` | `Err(e)` | `ToolCallRepair { original_error, cause: Other(message) }` |

`failed` 的 `cause` 只能是 `Other`:宿主语言的异常在这一侧没有类型对应物。 `message` 是宿主语言对该异常的常规文本表示:Node 复刻 AI SDK 的 `getErrorMessage`(字符串原样,`Error` 取 `toString()`,其余 JSON 序列化),其他语言用各自惯用的 message 访问器,Dart 没有通用访问器则用 `toString()`。控制流与 AI SDK `parseToolCall` 逐条一致:无工具集不进修复;抛错包成 `ToolCallRepair`;返回空保留原错误;修复结果重新校验,再失败也包成 `ToolCallRepair`。

### 3.3 结果补丁(`apply_tool_call_repair_to_result`)

输入整份 `GenerateTextResult`、`StreamTextResultAggregated`(两者顶层字段同形)或 `GenerateObjectResult`(自动识别其 `raw` 嵌套),同时改写 `tool_calls[]` 与 `response_messages` 中对应的 tool-call part——后者沿用 builder 的 `response_tool_call_input` 规则(仍然非法且输入是原始标量时不回放),并且连 `tool_call_id` 一起改:修复可以换掉调用 id,transcript 必须继续指向 `tool_calls` 里的同一条。**两处必须一起改**,否则下一轮对话会把未修复的参数发回模型。

`tool_call_id` 对不上、在结果中重复、目标调用本来就是合法的,或修复后的 id 与另一条调用冲突,都返回 `InvalidArgument` 而不是静默 no-op:补丁和 transcript 都按 id 对应,只有唯一 id 才能无歧义更新;静默选择任意一条只会破坏下一轮消息。

## 4. OpenAI 流式输出的决定:不反映修复

删除 `to_chat_completion_stream_with_deferred_tool_calls` 与 `defer_tool_calls` 标志(原先:配置了 repair 时扣住 `ToolInputDelta`,等 `StreamPart::ToolCall` 落地再一次性吐出)。

理由是与 AI SDK 对齐:上游**永远**立即转发工具输入增量,不会因为配置了修复就扣住。扣住还带来两个真实代价——首字节延迟变差,以及一个只在"配了 repair"时才走的独立代码路径。
`stream_text_as_openai` 的 rustdoc 现在写明:增量是 provider 原文,需要修复后的调用请用非流式 OpenAI 输出或原生 `stream_text`。

非流式 `ChatCompletion` 同样**不提供**补丁函数:它不携带 `invalid` / `error`,宿主根本无从判断哪个调用需要修——修复只能由原生结果驱动。为此 core 把 `generate_text_as_openai` 拆出纯转换函数 `generate_text_result_to_chat_completion`,C ABI 导出为 `aimux_generate_text_result_as_openai(handle, result_json, &out)`(handle 只提供 `ChatCompletion.model` 的回退值,不做任何运行时工作)。绑定的 `generateTextAsOpenAI` 在配了钩子时改走:原生 `generate_text` → 宿主修复循环 → 转换,得到的 `tool_calls` 是修复后的参数,与 Rust 调用方拿到的一致;不配钩子时仍走原来的一步调用。

## 5. 暴露面

| 层 | 名字 |
|---|---|
| core | `tool_call_repair_inputs` / `tool_call_repair_context` / `apply_tool_call_repair` / `apply_tool_call_repair_to_result` / `ToolCallRepairReply` |
| C ABI | `aimux_tool_call_repair_context` / `aimux_apply_tool_call_repair` / `aimux_apply_tool_call_repair_to_result`;OpenAI 转换 `aimux_generate_text_result_as_openai` |
| Node | `toolCallRepairContext` / `applyToolCallRepair` / `applyToolCallRepairToResult`;`Model.generateTextResultAsOpenai` |
| Python | 同 core 名;`Model.generate_text_result_as_openai` |

全部同步、纯函数:无 tokio、无 I/O、无全局状态。因此它们**不经过 `ffi_block_on`**,也就不会触发重入保护——从 `aimux_stream_text` 回调里调用是安全的。

现有 Rust 闭包 API(`ToolCallRepair`)原样保留,行为不变。

## 6. 契约

`contract-tests/fixtures/tool-call-repair.json` 固定每个分支的输入/期望输出,P2 的七家绑定对同一份文件断言。Rust 侧 `contract_fixture_matches_the_implementation` 回放同一份文件,防止其漂移。

## 7. 宿主循环的边界行为

- **钩子抛错不是流错误**:它是 `failed` 答复,core 把调用标成 `ToolCallRepair` 错误,照常作为数据投递。
- **边界失败**(解不开 context、FFI 拒绝答复这类绑定或 core 自身的 bug)在流中各绑定行为不同,均为既定行为:Java / Kotlin 先调 `onError` 再投递未修复的原 part;Swift 调 `onError` 后流继续,但不投递这个 part;Python / Go / Node / Dart 终止流。没有任何绑定会静默丢弃这个调用。
- **流等待钩子,不设超时**(与 AI SDK 一致):钩子挂起,流就挂起。Swift 在钩子运行期间 abort 不生效,钩子返回后才生效。
- **JVM**:钩子运行在借用了流读锁的工作线程上(daemon),可以回调同一个 Model;但在钩子里对正在流式输出的同一个 Model 调 `close()` 会死锁,与在流回调里调 `close()` 相同。
- **Swift / JVM 的线程跳转**必须是真正的新线程:GCD 的 `sync` 在可能时就地执行 block,会把钩子留在受重入保护的回调线程上(`AIMUX_E_FFI_REENTRANT_CALL`)。
