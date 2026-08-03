# Analysis: Issue #9 — Google thinking 模型 thoughtSignature 往返 + Node `google()` 工厂

> 来源：[arcships/aimux#9](https://github.com/arcships/aimux/issues/9)（用户 @sinskl 报告）
> 状态：会诊定稿（2026-08-03），修复进行中
> 会诊参与：GLM-5.2（明远）、GPT-5.6-sol（思远）独立验证 + 合并裁决

## 1. 问题描述

### 问题 A（bug）：Gemini thinking 模型工具调用 follow-up 轮次必现 HTTP 400

Gemini **thinking** 模型（`gemini-2.5-pro`、`gemini-3.x-flash-thinking` 等）在响应中给 `functionCall` part 附带同级的 `thoughtSignature`，后续轮次必须**原样回传**，否则 API 拒绝：

```
HTTP 400: Function call is missing a thought_signature in functionCall parts.
This is required for tools to work correctly...
```

aimux 全链路（数据模型 → 解析 → 发送 → 用户面投影）均无该字段，因此 thinking 模型 + 工具调用的多轮对话 100% 失败。非 thinking 模型不受影响（响应中无签名，无需回传）。

**签名位置（最容易做错）**：`thoughtSignature` 是 **part 的同级字段**，与 `functionCall` 平级，不在 `functionCall` 对象内部：

```json
{
  "functionCall": { "args": { "city": "SF" }, "id": "ukrj0i46", "name": "foo" },
  "thoughtSignature": "EuIDCt8DARFN..."
}
```

### 问题 B（API 缺口）：Node 绑定没有 `google()` 语言模型工厂

Node 绑定有 `openai()` / `anthropic()` / `deepseek()` + 通用 `provider()`（registry），但只有 `googleEmbedding` / `googleImage` / `googleVideo`，**没有 Google 语言模型工厂**。Python 绑定同样不对称。JS/Python 用户无法创建 Gemini chat 模型。

## 2. 诊断验证（全部代码位点已逐行确认）

### 2.1 数据模型层 — 4 处 `ToolCall` 均无签名承载字段

| 位置 | 结构 | 备注 |
|---|---|---|
| `aimux-core/src/tool.rs:102` | `tool::ToolCall` | 公开便捷类型，无任何兜底字段 |
| `aimux-core/src/content.rs:97` | `ContentPart::ToolCall` | 有 `provider_options: Option<Value>` 兜底（未被 Google 使用） |
| `aimux-core/src/stream_part.rs:82` | `StreamPart::ToolCall` | 有 `provider_metadata`（被置 None） |
| `aimux-core/src/result.rs:28` | `GenerateContent::ToolCall` | 有 `provider_metadata`（被置 None） |

全仓 `thoughtSignature|thought_signature` 引用：**0 处**。

### 2.2 解析层 — 4 个位点丢弃签名

| 位置 | 路径 | 现状 |
|---|---|---|
| `aimux-providers/src/google/model.rs:354` | 流式 | 只读 `functionCall.name/id/args`，`provider_metadata: None` |
| `aimux-providers/src/google/model.rs:582` | 非流式 | 同上 |
| `aimux-providers/src/vertex/model.rs:302` | 流式 | 同上（**复制粘贴的重复实现**，vertex/model.rs:395 有自己的 `extract_content_from_candidate`） |
| `aimux-providers/src/vertex/model.rs:411` | 非流式 | 同上 |

⚠️ Vertex 同样受影响：发送侧复用 `google::convert`（vertex/model.rs:24、127），但**解析侧是独立副本**，必须 4 个位点全改。

### 2.3 发送层 — 1 个共享位点

`aimux-providers/src/google/convert.rs:158-188` `convert_assistant_parts` 输出 `{ functionCall: {id?, name, args} }`，不回传签名。该函数被 google + vertex 共享，改 1 处覆盖两家。

`convert.rs:51` 注释自认 "No thought-signature / server-tool-call handling"，且称 "the Rust `ContentPart` doesn't carry per-part provider metadata"——**后半句已过时**（`ContentPart::ToolCall` 有 `provider_options`），会误导维护者，需修正。

### 2.4 用户面投影 — 第三处丢失

`aimux-core/src/generate.rs:194-207` 把 `GenerateContent::ToolCall` 投影到 `tool::ToolCall`（`GenerateTextResult.tool_calls`）时用 `..` 丢弃 metadata，且目标类型无承载字段。**最常用的回放路径也会丢签名**。

### 2.5 问题 B 相关事实

- `bindings/node/src/lib.rs`：`openai`(:273)、`anthropic`(:302)、`deepseek`(:349)、`provider`(:366) —— 无 `google`。
- `bindings/node/src/multimodal.rs:443-493`：只有 `google_embedding` / `google_image` / `google_video`。
- **registry 路堵死**：`aimux-providers/src/provider.rs:138-167` 的 `provider()` 工厂**硬编码构造 `OpenAIProvider` + `OpenAIConfig`**，只适用 OpenAI 兼容端点；`provider_registry.json` 也没有 `"google"` 条目。即使加条目也会用 OpenAI 协议打 Gemini 原生端点 → 错误。必须照 `openai()` 模式加专用工厂。
- Python 绑定（`bindings/python/src/lib.rs`）同样只有 `google_embedding` / `google_image` / `google_video`，无语言模型工厂。

## 3. 影响面与边界

| 维度 | 结论 |
|---|---|
| 受影响 provider | **google + vertex**（同一 Gemini 协议）。Anthropic 的签名在 `thinking` block 上且已支持（`ContentPart::Reasoning.signature`，content.rs:88，`anthropic/convert.rs:418` 回传）；OpenAI/DeepSeek/xAI 等协议无签名机制 |
| 受影响绑定 | 全部（Node/Python/Go/Flutter/Java/Kotlin/Swift/C 共享 `aimux-providers` 转换逻辑） |
| 不受影响 | 非 thinking 模型；`vertex/anthropic_model.rs`（走 Anthropic `rawPredict` 协议，另一套） |
| 范围外（另开 issue） | Gemini 3 server-side `toolCall/toolResponse`（provider-executed 工具）的独立 thoughtSignature（测试 fixture 已存在但被 ignore，google_provider_tools_test.rs:1367、1882） |

## 4. 修复方案定稿

### 4.1 方案选择：显式 `thought_signature` 字段（两位专家一致推荐）

否决纯 `provider_options/provider_metadata` 兜底方案，致命缺陷：

1. `tool::ToolCall` **没有**兜底字段 → `GenerateTextResult.tool_calls` 常用路径必然丢签名；
2. 输出侧叫 `provider_metadata`、输入侧叫 `provider_options`，core 无自动映射，用户需手工搬运 Google 私有 JSON 路径；
3. TS/其他语言用户不可见，容易只复制 id/name/args 继续触发 400；
4. google 与 vertex 的 metadata namespace（`google` vs `googleVertex`）容易分裂。

**定稿（含分歧裁决）**：给**全部 4 处** `ToolCall` 加显式 `thought_signature: Option<String>`。

> 分歧记录：明远建议只加 3 个响应类型（`tool::ToolCall` 保持 lossy 投影语义）；思远建议 4 处全加（否则常用路径仍坏）。裁决：**站思远**——加字段导致的全仓构造点修改成本反正要付一次，`GenerateTextResult.tool_calls` 是文档化的回放路径，"简易多轮"必须可用。

### 4.2 字段规范

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub thought_signature: Option<String>,
```

- Rust wire 名用 snake_case `thought_signature`；Google API 的 camelCase `thoughtSignature` 映射只在 converter 内做，不引入 serde rename；
- 签名**原样透传**：不解析、不 base64、不截断、不生成；
- ⚠️ 不要照搬 `Reasoning.signature` 的缺陷——它没有 `#[serde(default)]`，TS 导出成必填 `string | null`（content.rs:88、anthropic/types.rs:43）。新字段必须 `serde(default)` + `skip_serializing_if`；
- 多工具调用必须逐 part 保存（签名绑定 part，不是响应级）；
- 流式：以最终 `StreamPart::ToolCall` 为签名权威载体（不重复放在 ToolInputStart/Delta/End）。

### 4.3 修复清单

| # | 改动 | 位置 |
|---|---|---|
| 1 | core 字段 ×4 | tool.rs:102、content.rs:97、stream_part.rs:82、result.rs:28 |
| 2 | 全仓显式构造点补 `None` | 编译错误驱动 |
| 3 | 解析 ×4 | google/model.rs:354、:582；vertex/model.rs:302、:411 |
| 4 | 发送 ×1（共享） | convert.rs `convert_assistant_parts` 读 `thought_signature` 回写 part 顶层；修 :51 注释 |
| 5 | 投影 | generate.rs:194-207 传递字段 |
| 6 | ts-rs 类型再生成 + 审计 | `aimux-core/bindings/*.ts`、`bindings/node/src/types/*.ts` |
| 7 | Node `google()` 工厂 | lib.rs napi 函数（照 openai 模式，直接构造 `GoogleProvider`，不走 registry）+ 再生成 index.d.ts + `src/index.ts:44` re-export |
| 8 | Python `google()` 工厂同步 | `bindings/python/src/lib.rs` + pymodule 注册 |
| 9 | Rust 测试 | `google_provider_tools_test.rs`（wiremock 内联）补：非流式解析、流式解析、请求回放（签名在 part 顶层且原样）、无签名兼容、多 tool call；vertex 对称 |
| 10 | Node replay 测试 | 照 `reasoning_replay.test.ts` 模板加 `thoughtSignature` replay 用例 |

### 4.4 测试事实（修正初诊误述）

- Rust 层测试是 **wiremock 内联 mock**（`MockServer` + `ResponseTemplate`），不是 cassette；补 fixture 只需在 mock 响应 JSON 里加 `"thoughtSignature"` 字段，**任意字符串即可**（mock 不校验）。
- cassette（VCR）机制在 Node 绑定层（`__test__/cassette*.test.ts`）。
- 现有 Gemini cassette（`test_google_prompted_output_with_tools.json`）无签名：保留作"无签名响应必须兼容"的负向 fixture，**不手工篡改**。
- 现成先例：`bindings/node/__test__/reasoning_replay.test.ts`（DeepSeek reasoning_content 回放）是 thoughtSignature replay 测试的精确模板。

## 5. 风险与待确认

1. **流式分块**：当前实现假定 functionCall 单 chunk 完整返回（google/model.rs:354 注释）。若未来签名与 functionCall 分属不同 chunk，需要 tracker 而不是即时读取。需以官方 fixture 或一次受控真实录制确认。
2. **`functionResponse.name` 回退 hack**（convert.rs:218）：`ToolResult` 缺 `tool_name` 时用 `tool_call_id` 当 name，Google 要求 name 与 functionCall.name 匹配，错误同样 400。与签名 bug 独立，但会掩盖修复效果——**建议同批修或明确另开**。
3. **构造点破坏面**：加字段后所有显式 `ToolCall { ... }` 构造编译失败，涉及多个 provider 与测试，必须逐一补齐（不能只改 4 个定义）。
4. **其他语言绑定**：Go/Swift/Kotlin/Flutter/C 的手写镜像类型需审计（ts-rs 只覆盖 TS）。若走 JSON 且未知字段可忽略，运行时兼容，但公开类型应更新。
5. **两套签名勿混**：`Reasoning.signature`（Anthropic thinking block）与 `thoughtSignature`（Google functionCall）是独立机制，同属"思考模型签名回放"类问题，前者已解决、后者是本 issue。
6. **安全**：thought signature 是 provider 端不透明 token，非凭据，但测试只用固定假值，不提交真实会话签名或 API key。

## 6. 后续行动

- [x] 按 §4.3 清单实施修复（2026-08-03 完成）
  - core 字段 ×4 + 全仓构造点（编译驱动）+ 解析 ×4 + 发送 ×1 + 投影 + ts-rs 类型再生成（`cargo test -p aimux-core` 的 `export_bindings_*` 测试即为生成器）+ Node/Python `google()` 工厂 + Go/Swift/Kotlin/Flutter 手写镜像同步 + Rust wiremock 测试 + Node replay 测试（ava，`__test__/thought_signature_replay.test.ts`）
  - 验证：`cargo test --workspace` 全绿；Node 端 3 项往返自测通过（解析→tool_calls、回放→part 顶层、无签名负向）；Node/Python binding `cargo check` 通过
- [ ] `functionResponse.name` hack 修复（convert.rs:218 用 tool_call_id 当 name；建议紧接修复，会掩盖签名问题）
- [ ] Gemini 3 server-side `toolCall/toolResponse` thoughtSignature（范围外，另开 issue）
- [ ] 真实 API 受控录制验证流式签名 chunk 行为
- [ ] Node `npm install` 后正式跑 ava 测试套件（当前环境 npm 不可用，已用等价自测脚本验证）
