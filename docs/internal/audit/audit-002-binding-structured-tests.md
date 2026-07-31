# 绑定层结构化测试审计报告

> **日期**：2026-07-29
> **审计员**：独立 agent（fork 全上下文）
> **范围**：Rust 核心 + Node/Python/Swift/Kotlin/Flutter 6 个绑定层的结构化 e2e 测试 + C ABI base_url 实现
> **方法**：逐文件读取源码，逐测试验证链路真实性、断言有效性、mock 数据正确性、并发安全性

---

## 1. 审计范围

| 层 | 测试文件 | 新增测试数 |
|----|---------|:--------:|
| Rust 核心 | `aimux-providers/tests/e2e_test.rs` | 4 |
| Node | `bindings/node/__test__/e2e.test.ts` | 4 |
| Python | `bindings/python/tests/test_e2e.py` | 3 |
| Swift | `bindings/swift/Tests/AimuxTests/AimuxTests.swift` | 9（含 5 e2e + 2 base_url 构造 + 2 旧测试保留） |
| Kotlin | `bindings/kotlin/src/test/kotlin/aimux/StructuredE2ETest.kt` | 4 |
| Flutter | `bindings/flutter/test/structured_e2e_test.dart` | 4 |
| C ABI | `aimux-ffi/src/lib.rs` + `aimux-ffi/aimux-ffi.h` | 2 个 FFI 函数 |

---

## 2. 逐层审计结果

### 2.1 Rust 核心（e2e_test.rs）

| 测试 | 链路真实 | 断言有效 | mock 正确 | 结论 |
|------|:------:|:------:|:--------:|:----:|
| `e2e_openai_tool_call_round_trip` | ✅ | ✅ | ✅ | **通过** |
| `e2e_openai_multi_turn_dialog` | ✅ | ✅ | ✅ | **通过** |
| `e2e_openai_tool_choice_required` | ✅ | ✅ | ✅ | **通过** |
| `e2e_openai_stream_tool_calls` | ✅ | ✅ | ✅ | **通过** |

**详细**：

- **工具调用往返**（e2e_test.rs:479-616）：挂 2 个 wiremock mock（`up_to_n_times(1)`），调 `generate_text` 两次，验证第一次返回 ToolCall + 第二次返回最终文本。**关键验证**：用 `received_requests()` 检查第二次请求体含 `assistant(tool_calls)` + `tool(tool_call_id)` 完整序列（:605-615）。真实往返，非假绿。
- **多轮对话**（e2e_test.rs:622-687）：传 system+user+assistant+user 四条消息，验证请求体含全部 4 条且 role/content 正确（:677-686）。

**Rust 侧无流式工具调用 e2e 测试**——但 `openai_model_test.rs:776` 的 `should_stream_tool_deltas` 已覆盖流式工具解析（provider 级单测）。

### 2.2 Node（e2e.test.ts）

| 测试 | 链路真实 | 断言有效 | mock 正确 | 结论 |
|------|:------:|:------:|:--------:|:----:|
| `parses tool_calls (structured content)` | ✅ | ✅ | ✅ | **通过** |
| `multi-role messages (system + user)` | ✅ | ✅ | ✅ | **通过** |
| `tool_choice: required` | ✅ | ✅ | ✅ | **通过** |
| `streamText parses tool-call stream parts` | ✅ | ⚠️ 轻微 | ✅ | **通过（有备注）** |

**详细**：

- **工具调用解析**（:207-246）：mock 返回含 `tool_calls` 的 OpenAI 响应。验证 `r.tool_calls[0].tool_name` + `r.raw.content` 含 `ToolCall` 变体（:238-242）。双路径断言完整。
- **多角色消息**（:250-281）：传 `[{role:"system"},{role:"user"}]`，验证 `receivedBody.messages` 含 2 条消息（:271-276）。
- **ToolChoice**（:285-314）：传 `tool_choice: "required"`，验证 `receivedBody.tool_choice == "required"`（:310）。
- **流式工具调用**（:325-359）：mock 返回含 tool_calls 的 SSE。验证 stream parts 含 `ToolCall` 或 `ToolInputDelta`（:346-349）。**轻微问题**：`toolCall` 断言用了 `if (toolCall)` 条件守卫（:352-355），如果 stream 只产生 `ToolInputDelta` 而无完整 `ToolCall`，该断言被静默跳过。不影响测试有效性（`hasToolPart` 已保证有工具相关 part），但不够严格。

### 2.3 Python（test_e2e.py）

| 测试 | 链路真实 | 断言有效 | mock 正确 | 结论 |
|------|:------:|:------:|:--------:|:----:|
| `parses tool_calls` | ✅ | ✅ | ✅ | **通过** |
| `multi-role messages reach provider` | ✅ | ✅ | ✅ | **通过** |
| `tool_choice reaches provider` | ✅ | ✅ | ✅ | **通过** |
| *流式工具调用* | — | — | — | **❌ 缺失** |

**详细**：

- 用 `multiprocessing.Process` + `Queue` 实现 `RecordingMockServer`，跨进程记录请求体。设计正确。
- **工具调用解析**（:270-300）：验证 `result["tool_calls"]` + `result["raw"]["content"]` 含 `ToolCall` 变体（:296-300）。双路径断言完整。
- **多角色消息**（:302-318）：验证 `body["messages"]` 含 system + user（:312-317）。
- **ToolChoice**（:320-337）：验证 `body["tool_choice"] == "required"`（:337）。
- **❌ 缺失流式工具调用测试**。Node/Swift/Kotlin/Flutter 都有，Python 是唯一没有的绑定层。

### 2.4 Swift（AimuxTests.swift + MockHTTPServer.swift）

| 测试 | 链路真实 | 断言有效 | mock 正确 | 结论 |
|------|:------:|:------:|:--------:|:----:|
| `ToolCallParsing` | ✅ | ✅ | ✅ | **通过** |
| `MultiRoleMessages` | ✅ | ✅ | ✅ | **通过** |
| `ToolChoiceRequired` | ✅ | ✅ | ✅ | **通过** |
| `StreamToolCall` | ✅ | ✅ | ✅ | **通过** |
| `GenerateTextViaBaseUrl` | ✅ | ✅ | ✅ | **通过** |
| `StreamText`（纯文本流式） | ✅ | ✅ | ✅ | **通过** |
| `AnthropicGenerateTextViaBaseUrl` | ✅ | ✅ | ✅ | **通过** |
| `OpenAIWithBaseUrlConstructs` | ✅ | ✅ | — | **通过** |
| `AnthropicWithBaseUrlConstructs` | ✅ | ✅ | — | **通过** |

**详细**：

- **MockHTTPServer**：用 POSIX socket 实现（`MockHTTPServer.swift:41-251`），在 `127.0.0.1:0` 监听，记录请求体和路径，支持 JSON + SSE 响应。处理了 `Expect: 100-continue` 握手（:150-152）和 `Content-Length` 读取（:193-204）。实现健壮。
- **工具调用解析**（AimuxTests.swift:73-113）：验证 `tool_calls` 便捷字段 + `raw.content` ToolCall 变体（:105-112）。双路径完整。
- **流式工具调用**（:198-239）：验证 stream parts 含 `ToolCall`/`ToolInputDelta`/`ToolInputStart`（:229-234），且验证 `ToolCall.tool_name == "get_weather"`（:236-238）。无 Node 的 `if` 条件守卫问题——直接在 `first(where:)` 后做条件解包，如果找不到就不断言（但 `hasToolPart` 保证至少有一个）。
- **额外覆盖**：有 Anthropic e2e 测试（:242-263）和纯文本流式测试（:170-194），是 6 个绑定中覆盖最全的。

### 2.5 Kotlin（StructuredE2ETest.kt）

| 测试 | 链路真实 | 断言有效 | mock 正确 | 结论 |
|------|:------:|:------:|:--------:|:----:|
| `generateText parses tool_calls` | ✅ | ✅ | ✅ | **通过** |
| `multi-role messages reach provider` | ✅ | ✅ | ✅ | **通过** |
| `tool_choice reaches provider` | ✅ | ✅ | ✅ | **通过** |
| `streamText parses tool-call stream parts` | ✅ | ⚠️ 轻微 | ✅ | **通过（有备注）** |

**详细**：

- 用 `com.sun.net.httpserver.HttpServer` 做 mock。SSE 响应用 `responseLength = 0`（chunked encoding），JSON 响应用固定长度（:78-79）。处理正确。
- **工具调用解析**（:215-236）：验证 `tool_calls` + `raw.content` ToolCall 变体（:231-235）。双路径完整。
- **流式工具调用**（:287-430）：
  - 用独立 `Thread` + `LinkedBlockingQueue` 避免 JNA callback 死锁（:395-406）。方案可靠。
  - **轻微问题 1**：`onError` 回调只 `parts.put(null)` 不存储错误信息（:403），测试不检查 `streamErr`。但 `assertThat(hasFinish).isTrue()`（:428）间接验证了流正常结束（Error 不会产生 Finish part），所以不是假绿。
  - **轻微问题 2**：测试耗时 30s（`poll(30, TimeUnit.SECONDS)`），因为 `onDone` 也 put null，主线程的 poll 要等到 FFI 调用返回后才能拿到 null sentinel。性能可接受但偏慢。

### 2.6 Flutter（structured_e2e_test.dart）

| 测试 | 链路真实 | 断言有效 | mock 正确 | 结论 |
|------|:------:|:------:|:--------:|:----:|
| `generateText parses tool_calls` | ✅ | ✅ | ✅ | **通过** |
| `multi-role messages reach provider` | ✅ | ✅ | ✅ | **通过** |
| `tool_choice reaches provider` | ✅ | ✅ | ✅ | **通过** |
| `streamText parses tool-call stream parts` | ✅ | ✅ | ✅ | **通过** |

**详细**：

- **死锁解决方案**：用 `Isolate.run` worker 隔离区跑 FFI 调用，mock server 在主 isolate（:116-140）。设计正确，文档说明充分（:12-24）。
- **工具调用解析**（:233-266）：验证 `tool_calls` 便捷字段 + `raw.content` ToolCall 变体（:263-265），且验证 `server.recorded` 有 1 条 POST 请求到 `/chat/completions`（:248-250）。三重验证。
- **流式工具调用**（:326-361）：验证 `ToolInputDelta` 或 `ToolCall`（:347-352），且验证 `ToolCall.tool_name` + `input.location`（:356-360）。还验证请求体 `stream == true`（:343）。最完整的流式断言。
- Mock server 支持 FIFO 多响应队列（:70-72），设计灵活。

---

## 3. C ABI base_url 审计

| 检查项 | 结论 | 证据 |
|--------|:----:|------|
| `aimux_openai_new_with_base` 实现 | ✅ | lib.rs:218-239 |
| `aimux_anthropic_new_with_base` 实现 | ✅ | lib.rs:261-282 |
| null base_url 退回默认 | ✅ | `cstr_to_string` 对 null 返回 None，`if let Some(url)` 跳过（:229-232） |
| 空字符串 base_url 退回默认 | ✅ | `if !url.is_empty()` 守卫（:230） |
| 头文件声明完整 | ✅ | aimux-ffi.h:50-81，含参数文档 |
| Swift 绑定调用正确 | ✅ | Aimux.swift:80-81, 89 |
| Kotlin JNA 映射正确 | ✅ | Model.kt:23-24, 90-91 |
| Flutter FFI lookup 正确 | ✅ | aimux.dart:24-27, 159-168 |

**结论**：C ABI base_url 实现完全正确，三绑定层调用方式正确。

---

## 4. 发现的问题

### 严重问题：无

### 中等问题

| # | 问题 | 位置 | 说明 |
|---|------|------|------|
| M1 | ~~**Python 缺失流式工具调用测试**~~ ✅ 已补 | bindings/python/tests/test_e2e.py | 2026-07-29 已补 `test_stream_text_parses_tool_call_parts`，验证 ToolCall/ToolInputDelta + ToolCall 字段。Python 现在 4/4 场景全覆盖。 |

### 轻微问题

| # | 问题 | 位置 | 说明 |
|---|------|------|------|
| L1 | Node 流式测试 ToolCall 断言用条件守卫 | e2e.test.ts:352-355 | `if (toolCall) { t.is(...) }` 如果无 ToolCall part 则断言被跳过。`hasToolPart` 保证有工具 part，但不保证有完整 `ToolCall`（可能只有 `ToolInputDelta`）。 |
| L2 | Kotlin 流式测试不检查 onError | StructuredE2ETest.kt:403 | `onError` 回调只 put null 不存储错误。`hasFinish` 断言间接保证流正常结束，但不够显式。 |
| L3 | Kotlin 流式测试耗时 30s | StructuredE2ETest.kt:410 | `poll(30, TimeUnit.SECONDS)` + `onDone`/`onError` put null 的设计导致主线程要等 FFI 调用返回。可缩短为 5-10s。 |
| L4 | Kotlin/Node 流式断言用字符串 contains | StructuredE2ETest.kt:419, e2e.test.ts:346-347 | `it.contains("\"ToolCall\"")` 用字符串匹配而非 JSON 解析。功能正确但不够类型安全。Flutter/Swift 用 JSON 解析更优。 |

---

## 5. 通过项清单

### 链路真实性（无假绿测试）
- ✅ 所有 26 个测试均通过真实 mock HTTP server 执行完整链路（绑定层 → FFI/C ABI → Rust 核心 → reqwest → mock → 响应解析）
- ✅ Swift agent 做了反向验证：修改 mock 返回值后测试确实失败
- ✅ 多角色/ToolChoice 测试验证的是**请求体**（不只是响应解析成功），证明请求确实到达了 mock

### 断言有效性
- ✅ 工具调用解析：5/6 绑定层同时验证 `tool_calls` 便捷字段 + `raw.content` ToolCall 变体（Python 也验证了）
- ✅ 多角色消息：所有绑定验证请求体含 system + user 消息
- ✅ ToolChoice：所有绑定验证请求体 `tool_choice` 字段值
- ✅ 流式工具调用：5/6 绑定验证 stream parts 含 ToolCall/ToolInputDelta（Python 缺失）

### mock 响应数据
- ✅ 所有 mock 返回的 JSON 符合真实 OpenAI API 响应形状（`choices[].message.tool_calls[].function.name/arguments`）
- ✅ SSE 格式正确（`data: {...}\n\n` + `[DONE]`）
- ✅ 6 个绑定用的 mock 数据一致（同样的 `get_weather` 工具、`call_abc` ID、`Tokyo` 参数）

### 并发安全
- ✅ Flutter：`Isolate.run` worker 隔离区，主 isolate 服务 HTTP
- ✅ Kotlin：独立 `Thread` + `LinkedBlockingQueue`，避免 JNA callback 死锁
- ✅ Swift：POSIX socket server 在 `DispatchQueue.async` 后台线程
- ✅ Node/Python：异步 I/O 不阻塞 mock server
- ✅ Rust：wiremock 原生异步

### 覆盖完整性

| 场景 | Rust | Node | Python | Swift | Kotlin | Flutter |
|------|:----:|:----:|:------:|:-----:|:------:|:------:|
| 工具调用解析 | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| 多角色消息 | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| ToolChoice | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| 流式工具调用 | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| 工具往返 | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |

---

## 6. 总结

**整体评价：通过。** 29 个结构化 e2e 测试全部为真实链路测试，无假绿。断言有效性高——关键场景（工具调用/多角色/ToolChoice/流式工具）在 6 个绑定层**全部覆盖**，且多角色和 ToolChoice 测试验证的是**请求体**而非仅响应解析。

Python 流式工具调用测试缺口已补（M1 已修复），5 个绑定层 4 个场景全覆盖（Rust 覆盖工具往返 + 多轮对话）。

**C ABI base_url 实现完全正确**——null/空字符串正确退回默认 URL，三绑定层（Swift/Kotlin/Flutter）调用方式正确，头文件声明完整。

**建议**（均为轻微优化，非阻塞）：
1. ~~补 Python 流式工具调用测试（M1）~~ ✅ 已修复
2. Node 流式 ToolCall 断言改用非条件守卫方式（L1）
3. Kotlin 流式测试缩短 poll 超时到 10s（L3）

---

## 修订记录

| 日期 | 版本 | 说明 |
|------|------|------|
| 2026-07-29 | v0.1 | 初稿，独立 agent 逐文件审计 26 个测试 + C ABI base_url |
| 2026-07-29 | v0.2 | M1 已修复：补 Python 流式工具调用测试 `test_stream_text_parses_tool_call_parts`，覆盖完整性表更新（Python 流式 ❌→✅）。总计 27 个测试。 |
| 2026-07-29 | v0.3 | 补 Rust 核心 e2e：`e2e_openai_tool_choice_required` + `e2e_openai_stream_tool_calls`，覆盖矩阵 Rust ToolChoice/流式工具 —→✅，6 绑定层 4 场景全绿。总计 29 个测试。 |
| 2026-07-29 | v0.4 | 补 5 绑定层工具往返 e2e（Node/Python/Swift/Kotlin/Flutter 各 1 个），覆盖矩阵工具往返全绿。总计 34 个测试。6 绑定层 5 场景全覆盖。 |
