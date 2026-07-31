# 语言绑定能力与类型覆盖审计

> 审计日期：2026-07-31  
> 审计基线：`docs/ffi-audit-report.md`、`aimux-core/bindings/*.ts`（80 个文件，含 `serde_json/JsonValue.ts`；顶层 79 个）  
> 审计对象：Node.js、Python、Swift、Kotlin、Flutter、Go、C/C++  
> 方法：静态逐文件审计；未修改任何绑定源文件。测试数量按仓库中测试声明计数，不等同于本次实际执行通过数。

## 1. 执行摘要

**一句话结论：Go 是唯一接入 C ABI 全部 24 个符号、覆盖全部 8 种多模态结果类型且有共享契约测试的绑定；Python 的原生能力面最接近 Node，但 Node/Python 的多模态公开方法仍停留在 raw JSON；Swift/Kotlin/Flutter 已有较成熟的 typed 文本层却只接入文本 FFI；C/C++ 仍只是无测试的示例。**

### 1.1 总体覆盖矩阵

| 语言 | 实现路径 | provider/操作能力 | typed 文本 API | typed 多模态结果 | typed stream | 测试数 | 契约 fixture | 总评 |
|---|---|---:|---:|---:|---:|---:|---:|---|
| Node.js | napi-rs 原生 | 文本 + 8 模态操作；无 Tavily 工厂 | ✅ | ⚠ 类型全导出、方法返回 JSON | ✅ | 33 | ❌ | 旗舰基线；能力广但多模态 wrapper/test 缺口明显 |
| Python | PyO3 原生 | 与 Node 基本相同；无 Tavily 工厂 | ✅ | ❌ wrapper 未实现九类多模态 typed result | ⚠ stream 默认返回 dict | 37 | ❌ | 原生能力最接近 Node，typed 多模态落后 |
| Swift | C ABI | 仅文本 OpenAI/Anthropic | ✅ | ❌ | ✅，但 async 吞错 | 23 | ❌ | 文本 typed 质量高，能力面窄 |
| Kotlin | C ABI/JNA | 仅文本 OpenAI/Anthropic | ✅ | ❌ | ✅ | 29 | ❌ | 文本类型较完整，资源关闭有重复 drop 风险 |
| Flutter | C ABI/dart:ffi | 仅文本 OpenAI/Anthropic | ✅ | ❌ | ✅ | 58 | ❌ | 测试数量多，但核心枚举/options 不完整 |
| Go | C ABI/cgo | ✅ 全部 24 个符号、8 模态 | ✅ | ✅ 九类 result 均有 struct | ⚠ 仅 tag+raw payload | 60 | ✅ | C ABI 绑定中最完整，最接近 Node 功能面 |
| C/C++ | C ABI 直接链接 | 示例仅文本；无 base_url | ❌ | ❌ | ⚠ raw callback JSON | 0 | ❌ | 示例而非产品级绑定 |

### 1.2 基线说明

- C ABI 的 24 个导出符号及其缺口见 `docs/ffi-audit-report.md:51-62`；全部 10 种业务操作见 `docs/ffi-audit-report.md:141-161`。
- 核心 `GenerateTextResult` 要求 `text/tool_calls/finish_reason/usage/warnings/raw`，见 `aimux-core/bindings/GenerateTextResult.ts:11-35`。
- 核心 `GenerateTextOptions` 有 15 个 optional 字段，见 `aimux-core/bindings/GenerateTextOptions.ts:8-22`。
- 核心多模态结果字段示例：`EmbeddingResult` 见 `aimux-core/bindings/EmbeddingResult.ts:12-32`，`SpeechResult` 见 `aimux-core/bindings/SpeechResult.ts:13-33`，`SearchResult` 见 `aimux-core/bindings/SearchResult.ts:10-31`。

## 2. FFI 覆盖度对比表

### 2.1 24 个 C ABI 符号逐项覆盖

图例：✅ 直接接入；≈ 原生路径有等价能力（不调用 C ABI）；◐ workaround/定义存在但非同一符号；❌ 未接入。Node/Python 是原生路径，因此其标记表示能力等价而非 FFI 调用。

| C ABI 符号 | Node | Python | Swift | Kotlin | Flutter | Go | C/C++ |
|---|---:|---:|---:|---:|---:|---:|---:|
| `aimux_openai_new` | ≈ | ≈ | ✅ | ✅ | ✅ | ✅ | ✅ |
| `aimux_openai_new_with_base` | ≈ | ≈ | ✅ | ✅ | ✅ | ✅ | ❌ |
| `aimux_anthropic_new` | ≈ | ≈ | ✅ | ✅ | ✅ | ✅ | ✅¹ |
| `aimux_anthropic_new_with_base` | ≈ | ≈ | ✅ | ✅ | ✅ | ✅ | ❌ |
| `aimux_generate_text` | ≈ | ≈ | ✅ | ✅ | ✅ | ✅ | ✅ |
| `aimux_stream_text` | ≈ | ≈ | ✅ | ✅ | ✅ | ✅ | ✅ |
| `aimux_drop_handle` | ≈ GC | ≈ GC | ✅ | ✅ | ✅ | ✅ | ✅ |
| `aimux_free_string` | ≈ napi | ≈ PyO3 | ✅ | ✅ | ✅ | ✅ | ✅ |
| `aimux_openai_embedding_new` | ≈ | ≈ | ❌ | ❌ | ❌ | ✅ | ❌ |
| `aimux_embed` | ≈ | ≈ | ❌ | ❌ | ❌ | ✅ | ❌ |
| `aimux_openai_speech_new` | ≈ | ≈ | ❌ | ❌ | ❌ | ✅ | ❌ |
| `aimux_speech_generate` | ≈ | ≈ | ❌ | ❌ | ❌ | ✅ | ❌ |
| `aimux_openai_image_new` | ≈ | ≈ | ❌ | ❌ | ❌ | ✅ | ❌ |
| `aimux_image_generate` | ≈ | ≈ | ❌ | ❌ | ❌ | ✅ | ❌ |
| `aimux_openai_transcription_new` | ≈ | ≈ | ❌ | ❌ | ❌ | ✅ | ❌ |
| `aimux_transcription_generate` | ≈ | ≈ | ❌ | ❌ | ❌ | ✅ | ❌ |
| `aimux_openai_files_new` | ≈ | ≈ | ❌ | ❌ | ❌ | ✅ | ❌ |
| `aimux_file_upload` | ≈ | ≈ | ❌ | ❌ | ❌ | ✅ | ❌ |
| `aimux_cohere_reranking_new` | ≈ | ≈ | ❌ | ❌ | ❌ | ✅ | ❌ |
| `aimux_rerank` | ≈ | ≈ | ❌ | ❌ | ❌ | ✅ | ❌ |
| `aimux_google_video_new` | ≈ | ≈ | ❌ | ❌ | ❌ | ✅ | ❌ |
| `aimux_video_generate` | ≈ | ≈ | ❌ | ❌ | ❌ | ✅ | ❌ |
| `aimux_tavily_search_new` | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ |
| `aimux_search` | ◐ class 无公开工厂 | ◐ class 无公开工厂 | ❌ | ❌ | ❌ | ✅ | ❌ |

¹ C++ 类定义了 Anthropic 工厂，但示例 `main` 只实际创建 OpenAI：`bindings/c/example.cpp:24-29,87-103`。

证据：

- Node 文本工厂支持 OpenAI/Anthropic/DeepSeek 和 `base_url`：`bindings/node/src/lib.rs:163-230`；8 模态操作实现位于 `bindings/node/src/multimodal.rs:22-285`，工厂位于 `bindings/node/src/multimodal.rs:289-479`。
- Python 注册三种文本和十个多模态 provider 工厂：`bindings/python/src/lib.rs:202-231`；多模态操作见 `bindings/python/src/multimodal.rs:30-335`。
- Swift 只调用文本 8 符号：`bindings/swift/Sources/Aimux/Aimux.swift:50-95,105-180`。
- Kotlin JNA 接口只声明文本 8 符号：`bindings/kotlin/src/main/kotlin/aimux/Model.kt:20-38`。
- Flutter 只 lookup 文本 8 符号：`bindings/flutter/lib/aimux.dart:49-86`。
- Go 文本调用见 `bindings/go/aimux.go:202-273,376-407`，8 模态操作/构造器见 `bindings/go/multimodal.go:126-579`；它手工补了 header 遗漏的 6 个声明：`bindings/go/multimodal.go:19-25`。
- C/C++ 只包装文本：`bindings/c/example.c:34-56`、`bindings/c/example.cpp:13-80`。

### 2.2 provider 构造器与 base_url

| 语言 | 文本 provider | 多模态 provider 工厂 | base_url | 主要缺失 |
|---|---|---|---|---|
| Node | OpenAI/Anthropic/DeepSeek | OpenAI embedding/speech/image/transcription/files；Cohere embedding/rerank；Google embedding/image/video | ✅ 所有原生工厂 | Tavily search 工厂 |
| Python | 同 Node | 同 Node | ✅ 所有原生工厂 | Tavily search 工厂 |
| Swift | OpenAI/Anthropic | 无 | ✅ 文本 2/2 | DeepSeek + 全部 8 模态 |
| Kotlin | OpenAI/Anthropic | 无 | ✅ 文本 2/2 | DeepSeek + 全部 8 模态 |
| Flutter | OpenAI/Anthropic | 无 | ✅ 文本 2/2 | DeepSeek + 全部 8 模态 |
| Go | OpenAI/Anthropic；DeepSeek 用 OpenAI-compatible workaround | FFI 暴露的 7 个工厂，覆盖 8 模态 | ✅ 仅文本；多模态 0/7 | Cohere/Google embedding、Google image；所有多模态 base_url |
| C/C++ | OpenAI；C++ 另定义 Anthropic | 无 | ❌ | base_url、DeepSeek、全部多模态 |

- Go DeepSeek workaround 固定 `https://api.deepseek.com/v1`：`bindings/go/multimodal.go:581-592`。
- C ABI 本身没有任何多模态 `_with_base`，所以 Go 无法补齐；基线见 `docs/ffi-audit-report.md:37-45,211-217`。

## 3. 类型实现覆盖对比表

图例：✅ typed；⚠ 类型存在但公开操作仍返回 raw JSON/Map，或关键 payload 为 raw；❌ 缺失。

### 3.1 关键输入、枚举和文本类型

| 语言 | `GenerateTextOptions` | `ModelMessage` | `GenerateTextResult` | `StreamPart` | `Role` | `FinishReasonUnified` | `ReasoningEffort` |
|---|---:|---:|---:|---:|---:|---:|---:|
| Node | ✅ canonical ts-rs | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Python | ✅ 15/15 | ✅ | ✅ | ✅ 模型；API 默认 dict | ✅ | ✅ | ✅ |
| Swift | ✅ 15/15 | ✅ | ⚠ 缺 `warnings` | ✅ 全 variant | ✅ | ✅ | ✅ |
| Kotlin | ✅ 15/15 | ✅ | ✅ | ✅ + Unknown | ✅ | ✅ | ✅ |
| Flutter | ⚠ 4/15 | ⚠ role/content 为 `String/Object` | ⚠ 缺 `warnings` | ✅ + Unknown | ❌ | ❌ | ❌ |
| Go | ✅ 15/15 | ⚠ `Content any` | ✅ | ⚠ tag + raw payload，仅 TextDelta typed | ✅ | ✅ | ✅ |
| C/C++ | ❌ raw JSON | ❌ | ❌ | ❌ raw JSON | ❌ | ❌ | ❌ |

关键证据与字段偏差：

- Node 直接重导出生成类型：`bindings/node/types.ts:1-77`；typed wrapper 的文本输入/输出签名见 `bindings/node/src/index.ts:94-102,124-133`。
- Python 三枚举精确覆盖 serde 字符串：`bindings/python/python/aimux/wrapper.py:65-80`；15 个 option 字段见 `bindings/python/python/aimux/wrapper.py:790-811`；结果字段含 `warnings`：`bindings/python/python/aimux/wrapper.py:827-835`。
- Swift 三枚举见 `bindings/swift/Sources/Aimux/Types.swift:102-125`，15 个 options 字段见 `bindings/swift/Sources/Aimux/Types.swift:909-955`。但 `GenerateTextResult` 没有核心要求的 `warnings`：`bindings/swift/Sources/Aimux/Types.swift:876-899` 对比核心 `aimux-core/bindings/GenerateTextResult.ts:27-35`。
- Kotlin 三枚举见 `bindings/kotlin/src/main/kotlin/aimux/Types.kt:55-81`；结果含 `warnings` 且 raw typed：`bindings/kotlin/src/main/kotlin/aimux/Types.kt:853-886`；StreamPart 有 unknown fallback：`bindings/kotlin/src/main/kotlin/aimux/Types.kt:888-905`。
- Flutter `GenerateTextOptions` 只有 `max_output_tokens/temperature/tools/tool_choice`：`bindings/flutter/lib/types.dart:547-582`，缺 stop/top-p/top-k/penalties/response_format/seed/headers/provider_options/reasoning/instructions。`ModelMessage.role` 是 String、content 是 Object：`bindings/flutter/lib/types.dart:589-623`；`GenerateTextResult` 同样漏 `warnings`：`bindings/flutter/lib/types.dart:523-545`。
- Go 枚举和 options 完整：`bindings/go/types.go:18-76,185-229`；但 `ContentPart` 是 `json.RawMessage`：`bindings/go/types.go:113-116`，StreamPart 只拆 tag/payload：`bindings/go/types.go:232-264`。

### 3.2 九类结果与模态 CallOptions

| 语言 | Text | Embedding | Speech | Image | Transcription | Reranking | Video | Search | UploadFile | 各模态 CallOptions |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Node | ✅ | ⚠ | ⚠ | ⚠ | ⚠ | ⚠ | ⚠ | ⚠ 无工厂 | ⚠ | ✅ ts-rs 全量导出 |
| Python | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ wrapper 未建模 |
| Swift | ⚠ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| Kotlin | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| Flutter | ⚠ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| Go | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅（部分复杂字段 raw） |
| C/C++ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |

说明：

- Node 的九类类型本身全部由 `bindings/node/types.ts:12-75` 导出，但 napi 多模态方法声明仍统一返回 `Promise<string>`，例如 embedding/image/search：`bindings/node/index.d.ts:12-18,30-36,65-70`，所以表中为 ⚠，不是端到端 typed API。
- Python 多模态原生方法也全部返回 JSON 字符串，如 embedding/speech/image：`bindings/python/src/multimodal.rs:41-67,81-96,110-125`；Pydantic wrapper 只实现文本类型。
- Go 九类 result 和 option structs 覆盖见 `bindings/go/types.go:139-229` 与 `bindings/go/multimodal_types.go:17-315`。但 tagged unions（如 `AudioData`、`ImageOutputs`、`VideoData`）用多个 optional 字段模拟，无法在编译期保证“恰好一个 variant”：`bindings/go/multimodal_types.go:47-54,89-95,205-231`。

## 4. 测试验证覆盖对比表

测试数量按测试函数/`test(...)` 声明统计。参数化子用例未展开；cassette exhaustive 的运行时动态用例只计一个顶层声明。

| 语言 | 总数 | 单元/构造/close/错误 | mock E2E 文本/工具/角色/流 | 多模态 | round-trip | 共享契约 fixture | 主要问题 |
|---|---:|---|---|---|---|---|---|
| Node | 33 | 6 个基础 surface/无效 prompt；另有 cassette | 17 个 raw+typed E2E，含双 provider、tool_calls、tool_choice、multi-role、stream、完整 tool round-trip | 0 | 无独立系统性 round-trip | 0 | 多模态完全未测；部分 stream tool 断言条件化 |
| Python | 37 | 6 个基础；cassette | 21 个 raw+typed E2E，同 Node 主要文本场景 | 0（multipart message 不等于模型模态） | 少量局部 round-trip | 0 | `test_generate_text_with_options` 未检查请求，只说“隐式”验证 |
| Swift | 23 | 6 个构造/invalid/base_url/stream surface | 17 个 raw+typed mock 场景 | 0 | 2 个明确 round-trip | 0 | async stream 吞错行为未作为失败验证 |
| Kotlin | 29 | 5 个基础 | 9 个 raw+typed mock 场景 | 0 | 15 个类型 round-trip | 0 | double-close 测试只断言“不 crash”，未验证只 drop 一次 |
| Flutter | 58 | 5 个基础 | 10 个 raw+typed mock 场景 | 0 | 43 个 content/stream/type 测试 | 0 | 无共享 fixture；options/枚举缺失未被发现 |
| Go | 60 | 7 个基础 + close/concurrency/invalid | 15 个 text typed/raw E2E | 20 个：8 构造、8 result parse、2 close、2 options/mock | 17 个 round-trip | 1 顶层、逐 fixture case | 多数多模态只是手写 JSON parse，不是实际 FFI→mock E2E |
| C/C++ | 0 | 0 | 0 | 0 | 0 | 0 | 仅示例，无法回归 ABI/内存/错误合同 |

测试文件计数证据：

- Node：`bindings/node/__test__/index.test.ts` 6、`e2e.test.ts` 10、`wrapper.test.ts` 7、cassette 10。基础无效 JSON 见 `bindings/node/__test__/index.test.ts:34-47`；typed tool/role/stream 见 `bindings/node/__test__/wrapper.test.ts:140-318`；完整 tool round-trip 见 `bindings/node/__test__/e2e.test.ts:361-438`。
- Python：`tests/test_aimux.py` 6、`test_e2e.py` 10、`test_wrapper.py` 11、cassette 10。弱断言位于 `bindings/python/tests/test_e2e.py:161-166`；typed full round-trip 位于 `bindings/python/tests/test_wrapper.py:248-319`。
- Swift：`AimuxTests.swift` 14、`WrapperTests.swift` 9。typed stream/round-trip 见 `bindings/swift/Tests/AimuxTests/WrapperTests.swift:165-304`。
- Kotlin：`ModelTest.kt` 5、`StructuredE2ETest.kt` 5、`TypedModelTest.kt` 19。round-trip 见 `bindings/kotlin/src/test/kotlin/aimux/TypedModelTest.kt:243-490`。
- Flutter：`aimux_test.dart` 5、`content_part_test.dart` 16、`structured_e2e_test.dart` 5、`typed_model_test.dart` 5、`typed_round_trip_test.dart` 27。typed E2E 见 `bindings/flutter/test/typed_model_test.dart:280-452`。
- Go：`aimux_test.go` 7、`e2e_test.go` 7、`multimodal_test.go` 20、`roundtrip_test.go` 17、`typed_test.go` 8、`wire_format_test.go` 1。共享 fixture 实际读取见 `bindings/go/wire_format_test.go:21-47`。
- 只有 Go 引用了 `contract-tests/fixtures/wire-format.json`；其余六种绑定无引用。fixture 路径和合同范围见 `bindings/go/wire_format_test.go:1-10,30-46`。

### 4.1 明确测试/实现风险

1. **Node/Python 流式 tool-call 测试存在弱断言**：找到完整 `ToolCall` 才校验字段，因此只有 delta 也可通过。Node：`bindings/node/__test__/wrapper.test.ts:303-314`；Python：`bindings/python/tests/test_e2e.py:437-448`。
2. **Python options E2E 未验证请求体**：`bindings/python/tests/test_e2e.py:161-166`。
3. **Go “E2E embedding/files” 名称高估覆盖**：embedding 明确因无 base_url 只做 typed parse：`bindings/go/multimodal_test.go:101-138`；files 只请求 Go mock、未调用 FFI upload：`bindings/go/multimodal_test.go:317-345`。
4. **C/C++ 没有测试**，且 C++ `stream_text` 忽略调用者传入的三个 `std::function`，实际使用内部打印 lambda：`bindings/c/example.cpp:58-80`。

## 5. typed API 完整度对比表

| 语言 | string prompt | typed message[] | typed Options | typed Text Result | typed StreamPart | typed 多模态方法 |
|---|---:|---:|---:|---:|---:|---:|
| Node | ✅ | ✅ | ✅ | ✅ | ✅ | ❌（raw `Promise<string>`） |
| Python | ✅ | ✅（也允许 dict） | ✅ | ✅ | ⚠ iterator 返回 dict，需再调用 parser | ❌ |
| Swift | ✅ `ModelPrompt.text` | ✅ `ModelPrompt.messages` | ✅ | ⚠ typed 但缺 warnings | ✅ callback/AsyncStream | ❌ |
| Kotlin | ✅ overload | ✅ overload | ✅ | ✅ | ✅ callback/Sequence | ❌ |
| Flutter | ✅ | ✅ 独立 `generateTextMessages` | ⚠ 仅 4 字段 | ⚠ 缺 warnings | ✅ | ❌ |
| Go | ✅ `any` + runtime check | ✅ `[]ModelMessage` | ✅ | ✅ | ⚠ typed envelope、payload 多为 raw | ⚠ 方法 typed input，但返回 string + `ParseXResult` 两步 |
| C/C++ | ❌ JSON string | ❌ JSON string | ❌ JSON string | ❌ JSON string | ❌ JSON string | ❌ |

- Node 旗舰签名：`bindings/node/src/index.ts:94-102,124-133`。
- Python typed generate 正确返回模型，但 stream 声明为 `Iterator[dict]`：`bindings/python/python/aimux/wrapper.py:886-928`。
- Swift typed overload：`bindings/swift/Sources/Aimux/Types.swift:1174-1258`。
- Kotlin typed overload/stream：`bindings/kotlin/src/main/kotlin/aimux/TypedModel.kt:41-85,88-205`。
- Flutter typed facade：`bindings/flutter/lib/typed_model.dart:32-76`。
- Go typed text facade：`bindings/go/typed.go:16-64,66-132`；多模态 `Embed/Generate/Search` 返回 string，再由 `ParseXResult` 解析，例如 `bindings/go/multimodal.go:139-188,543-564`。

## 6. 逐语言详细分析

### 6.1 Node.js

**优点**

- 原生直连核心，文本 + 8 模态操作全部存在，且每个工厂支持 base_url：`bindings/node/src/lib.rs:163-230`、`bindings/node/src/multimodal.rs:289-479`。
- 唯一直接消费并重导出 canonical ts-rs 类型全集的绑定：`bindings/node/types.ts:1-77`。
- 文本 typed API 达到目标形态：string/message[] + typed options → typed result/stream：`bindings/node/src/index.ts:74-133`。
- 文本 E2E 覆盖 OpenAI/Anthropic、tools、tool_choice、多角色、stream、完整 tool round-trip。

**缺口**

- `src/index.ts` 只重导出文本工厂，未重导出任何多模态工厂/类，也没有多模态 typed wrapper：`bindings/node/src/index.ts:38-63`。
- napi 多模态方法均返回 JSON string：`bindings/node/index.d.ts:12-35,56-78,98-112`。
- 没有 Tavily search 工厂，且多模态无任何测试。

**建议**：先给九类多模态操作增加 typed facade 并从主入口导出；随后用 base_url mock server 为每种模态至少建立一个真实 provider-chain E2E；接入共享 fixture。

### 6.2 Python

**优点**

- PyO3 原生能力几乎复制 Node，所有原生工厂支持 base_url：`bindings/python/src/lib.rs:138-196`、`bindings/python/src/multimodal.rs:342-528`。
- Pydantic 文本类型覆盖深，包括外部 tag union、三枚举和 15 字段 options：`bindings/python/python/aimux/wrapper.py:65-80,136-572,790-835`。
- typed 文本 E2E 场景充分，含 multipart 和完整 tool round-trip。

**缺口**

- `aimux.wrapper` 没有任何多模态 result/call-options 类型；公开多模态仍是 JSON string。
- typed `stream_text` 实际返回 dict，不是 `Iterator[StreamPart]`：`bindings/python/python/aimux/wrapper.py:908-933`。
- 顶层 `aimux` 默认 convenience API 仍返回 dict：`bindings/python/python/aimux/__init__.py:78-115`，typed API 需要显式导入 `aimux.wrapper`。
- 无 Tavily 工厂、无多模态测试、无共享契约测试。

**建议**：把 `stream_text` 改为/新增 typed iterator；为 8 模态生成 Pydantic models 和 typed facade；将 typed API 提升为默认入口；补 fixture 与多模态 mock E2E。

### 6.3 Swift

**优点**

- Codable 文本层较完整，15 字段 options、typed tagged unions、typed callback 和 AsyncStream：`bindings/swift/Sources/Aimux/Types.swift:909-1258`。
- 文本 mock E2E/round-trip 覆盖质量高：`bindings/swift/Tests/AimuxTests/WrapperTests.swift:19-304`。
- ARC 自动释放 handle：`bindings/swift/Sources/Aimux/Aimux.swift:40-54`。

**缺口/风险**

- 只接入文本 FFI，无 DeepSeek/8 模态。
- `GenerateTextResult` 漏 `warnings`。
- raw/typed `streamTextAsync` 都吞掉错误并正常 finish：`bindings/swift/Sources/Aimux/Aimux.swift:194-208`、`bindings/swift/Sources/Aimux/Types.swift:1242-1256`。
- callback trampoline 使用单一静态 `StreamContext.current`，并发流会互相覆盖：`bindings/swift/Sources/Aimux/Aimux.swift:144-183,217-235`。

**建议**：P0 修正 `warnings` 和 async throw；P0 解决 callback user-data/并发隔离；P1 接入 8 模态和共享 fixture。

### 6.4 Kotlin

**优点**

- 文本 typed 类型较完整，包含 options、结果、StreamPart unknown fallback：`bindings/kotlin/src/main/kotlin/aimux/Types.kt:562-579,853-905`。
- typed overload 接受 string 或 `List<ModelMessage>`，返回 typed result/stream：`bindings/kotlin/src/main/kotlin/aimux/TypedModel.kt:43-85,88-205`。
- 29 个测试含 structured E2E 与系统性 round-trip。

**缺口/风险**

- JNA 只声明文本 8 符号：`bindings/kotlin/src/main/kotlin/aimux/Model.kt:20-38`。
- `Model.close()` 的 `handle` 是不可变且不置零，重复 close 会重复调用 `aimux_drop_handle`：`bindings/kotlin/src/main/kotlin/aimux/Model.kt:60-69`。当前测试只证明“不 crash”：`bindings/kotlin/src/test/kotlin/aimux/ModelTest.kt:43-49`。
- typed stream 解码失败被转换成 `Unknown("<parse-error>")`，可能掩盖 wire break：`bindings/kotlin/src/main/kotlin/aimux/TypedModel.kt:146-155`。

**建议**：P0 把 close 做成原子幂等；解析失败走 onError 而非 Unknown；P1 接入 8 模态及共享 fixture。

### 6.5 Flutter

**优点**

- typed content/stream 使用 sealed hierarchy 和 Unknown fallback：`bindings/flutter/lib/types.dart:215-256,629-700`。
- 58 个测试为七种语言最多，尤其对 content/stream variant 做了细粒度 round-trip：`bindings/flutter/test/typed_round_trip_test.dart:35-500`。
- raw 层正确检查 error envelope 和 closed state：`bindings/flutter/lib/aimux.dart:193-223,261-271`。

**缺口/风险**

- 只 lookup 文本 8 符号，无多模态。
- options 仅 4/15，Role/FinishReasonUnified/ReasoningEffort 都只是裸 String 或完全缺失；`GenerateTextResult` 漏 `warnings`。
- 全局 `_currentController` 只允许一个流，并发流不安全：`bindings/flutter/lib/aimux.dart:113-138,238-255`。
- FFI stream 同步阻塞 isolate：`bindings/flutter/lib/aimux.dart:226-258`；测试必须显式 worker isolate，见 `bindings/flutter/test/typed_model_test.dart:13-25`。

**建议**：P0 补齐三枚举、15 字段 options、warnings；P0 改为 per-stream callback context/NativeCallable 并提供 isolate-safe API；P1 接入多模态与 fixture。

### 6.6 Go

**优点**

- 唯一覆盖全部 24 个 C ABI 符号，含 header 缺失的 6 个：`bindings/go/multimodal.go:19-25`。
- 九类结果和各模态 options 都有 Go struct：`bindings/go/types.go:139-229`、`bindings/go/multimodal_types.go:17-315`。
- 文本 typed API 与 Node 目标形态接近：`bindings/go/typed.go:37-64,85-132`。
- 唯一真正消费共享 wire fixture：`bindings/go/wire_format_test.go:30-158`。
- handle 有锁、close 幂等、stream registry 有 `sync.Once`：`bindings/go/aimux.go:101-145,293-336`。

**缺口/风险**

- 多模态方法仍返回 JSON string，需手工再调 `ParseXResult`，不是真正一步 typed API。
- StreamPart payload 多数为 raw JSON，只有 TextDelta 提供 typed payload：`bindings/go/types.go:232-264`。
- 多模态测试大多只测构造和静态 JSON parse，因 C ABI 无 base_url 无法做真实 FFI mock E2E：`bindings/go/multimodal_test.go:99-138,315-345`。
- 多个 externally-tagged union 用 optional 字段模拟，允许非法多 variant 状态。

**建议**：P0 为多模态增加 `EmbedTyped/GenerateTyped/...`；P1 建模完整 StreamPart union；FFI 增加多模态 base_url 后立刻把静态 parse 测试升级为真实端到端。

### 6.7 C/C++

**优点**

- C 示例清楚展示构造、generate、stream、free/drop：`bindings/c/example.c:27-59`。
- C++ 用 RAII、禁 copy、支持 move：`bindings/c/example.cpp:13-43`。

**缺口/风险**

- 无 typed 类型层、无 JSON parser、无 base_url、多模态、错误 envelope 处理或测试。
- C++ stream API 的回调参数被完全忽略：`bindings/c/example.cpp:58-80`。
- 头文件仍缺 rerank/video/search 六个声明，直接限制标准 C 消费者：`docs/ffi-audit-report.md:49-76`。

**建议**：先修 header；把示例升级为小型可复用 C++ wrapper，正确转发 callback 并解析 error envelope；增加 ABI smoke/ASan/invalid input/double-drop 测试。

## 7. 修复优先级清单

### P0：合同或正确性

1. **Swift**：`GenerateTextResult.warnings` 对齐核心；AsyncThrowingStream 传递错误；移除全局单 stream context。
2. **Kotlin**：close 后原子置零/只 drop 一次；StreamPart 解码失败不可伪装成 forward-compatible Unknown。
3. **Flutter**：补 `warnings`、三枚举及缺失的 11 个 options 字段；修复全局 callback controller 和阻塞 isolate API。
4. **C/C++**：修复 C++ callback 参数被忽略；补 `aimux-ffi.h` 六个声明并建立 ABI smoke tests。

### P1：能力对等

5. **Go**：让九类多模态方法直接返回 typed result；完整建模 typed StreamPart payload。
6. **Node**：从主入口公开多模态工厂并提供 typed wrappers；增加 Tavily search 工厂。
7. **Python**：新增九类多模态 Pydantic types/facade；typed stream 直接 yield `StreamPart`；增加 Tavily search 工厂。
8. **Swift/Kotlin/Flutter**：接入 FFI 已有的 8 模态；DeepSeek 暂可像 Go 一样通过 OpenAIWithBase 兼容入口实现。

### P2：验证与可维护性

9. 所有非 Go 绑定接入 `contract-tests/fixtures/wire-format.json`；不要各自手写同一合同。
10. Node/Python 增加多模态测试；Go 在 FFI 补多模态 base_url 后把 parse-only 测试升级为真实链路。
11. 统一由 schema/codegen 派生 Swift/Kotlin/Dart/Python/Go 类型，避免 Flutter/Swift 已出现的字段漂移。
12. 给所有绑定增加“未知 enum/variant、错误 envelope、并发 stream、close 与 in-flight call”回归测试。

## 8. 总体评价

### 8.1 谁最接近 Node 旗舰水准？

- **按功能覆盖：Go 最接近，甚至在 Tavily search 上超过 Node**。它是唯一完整消费全部 24 个 C ABI 符号、8 模态、九类 result、共享 wire fixture 的绑定。
- **按原生 provider 能力：Python 最接近 Node**。PyO3 层几乎逐行镜像 Node 的工厂和操作，但 typed wrapper 只覆盖文本，因此用户体验仍有明显差距。
- **按纯文本 typed DX：Kotlin 与 Swift 已接近 Node**；Kotlin 类型合同更完整，Swift API 更原生，但二者都受限于仅文本 FFI。

### 8.2 明显差距

1. **C/C++**：没有产品级类型层或测试，只是示例。
2. **Flutter**：测试很多，但核心枚举/options/result 字段仍漂移，说明测试没有锚定共享合同。
3. **Swift/Kotlin/Flutter**：FFI 可用的 8 模态全部未接入，能力覆盖只有文本。
4. **Node/Python**：底层多模态能力广，但缺 typed 方法层与测试，旗舰能力与旗舰 DX 尚未统一。

### 8.3 剩余不确定性

- 本审计是静态审计，**没有声明上述 33/37/23/29/58/60 个测试本次全部执行通过**；不同绑定依赖各自 native artifact/toolchain。
- Node/Python 的 cassette exhaustive 会在运行时动态生成更多子用例，报告只按顶层测试声明计数。
- `aimux-core/bindings` 当前 glob 得到 80 个 `.ts` 文件，其中 79 个顶层类型文件 + `serde_json/JsonValue.ts`；背景中的“共 79 个”应理解为顶层生成类型数。
- C ABI 缺少多模态 base_url，使 Go 多模态无法用本地 mock 完成真正 provider-chain E2E；目前静态 parse 测试不能证明实际 provider request/response 合同。

---

*本审计只新增本报告，没有修改任何绑定源文件。*
