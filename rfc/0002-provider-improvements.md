# 厂商适配层改进

## 现在的问题

薄封装的厂商只有网址和环境变量不同，没有定制点。深度求索的推理字段被直接丢掉，代码注释自己都承认了。

## 建议

### 加一个配置描述结构

把各家差异写成数据——推理字段叫什么名、支不支持工具调用、流式返回带不带用量统计。请求数据的构造和流式解析读这份配置决定行为。这样既保持运行时可以换厂商，又能表达差异。

### 测试改成录播模式

现在每个厂商的测试手写一遍，重复且容易漏。改成录一次真实返回存成文件，以后跑测试直接回放这个文件，不用联网、不用密钥。再写一套统一测试，所有厂商跑同样的输入、断言同样的行为。这两样到位了才敢放量铺厂商。

## 实现进度（2026-07-28）

### 已完成

1. **reasoning_content 字段解析**：openai/types.rs 的 `reasoning` 字段加了 `alias = "reasoning_content"`。DeepSeek/阿里通义等厂商返回的 `reasoning_content` 字段现在能被共享 OpenAI 解析器自动解析，不再丢失。

2. **OpenAICompatProfile 结构**：在 openai/mod.rs 新建，描述厂商差异：
   - `supports_top_k`：是否支持 top_k 参数
   - `supports_tools`：是否支持工具调用
   - `supports_response_format`：是否支持 response_format
   - `stream_usage_key`：流式 usage 的特殊 key（如 Groq 的 "x_groq"）
   - `request_body_override`：请求体后处理（如 DeepSeek 的 thinking 字段）

3. **OpenAIConfig 加 profile 字段**：通过 `with_profile()` 设置。

4. **convert.rs 接入 profile**：`build_request_body_with_warnings` 接收 profile 参数，top_k 现在由 profile 控制——`supports_top_k=true` 的厂商发送 top_k，`false` 的发 warning。

5. **model.rs 接入 profile**：`execute_generate`/`execute_stream` 接收 profile 参数并传递给 convert。

6. **DeepSeek 从独立实现改为薄封装**：删掉了 668 行独立 model.rs + convert.rs + types.rs，改成 70 行薄封装。thinking 字段和 reasoning_effort 重映射通过 `RequestBodyOverride::DeepSeek` 在共享 convert.rs 里处理。reasoning_content 靠 serde alias 自动解析。

7. **全部 145 个薄封装已接入 profile**：Groq 用 `groq()`，DeepSeek 用 `deepseek()`，其余 143 个用 `full()`。Azure 也已接入。

8. **测试全过**：所有测试文件 0 failures。

### 待完成

1. **model.rs 流式 x_groq 硬编码**：流式解析的 x_groq 处理目前仍硬编码，应该读 `stream_usage_key` 决定。影响较小（x_groq 只有 Groq 用，当前硬编码结果正确）。

2. **supports_tools / supports_response_format 接入 convert**：目前定义了但 convert.rs 还没读——当前所有厂商都默认支持。如果有厂商不支持，需要接入。
