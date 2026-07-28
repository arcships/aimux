# 测试用例可复用性分析

> 参考源码：`reference/ai/packages/openai/src/`、`packages/anthropic/src/`、`packages/provider-utils/src/`
> 复核状态：✅ 已复核（详见文末复核记录）

## OpenAI 测试用例清单

主文件 [openai-chat-language-model.test.ts](../reference/ai/packages/openai/src/chat/openai-chat-language-model.test.ts)（4048 行，89 个 `it`），用 `createTestServer`（MSW）+ fixture 文件。配套纯函数测试：`convert-to-openai-chat-messages.test.ts`（33）、`openai-chat-prepare-tools.test.ts`（12）。

| 类别 | 用例数 | 关键断言 | 可复用性 |
|------|--------|----------|----------|
| 响应解析（doGenerate） | ~12 | text/usage/partial usage/logprobs/finish reason/raw headers/annotations/tool results | ✅ mock JSON body → 断言反序列化结果 |
| 请求构造 | ~25 | `requestBodyJson` 严格匹配 model/messages/settings/reasoning_effort/tools/response_format | ✅ wiremock 录制请求体断言 |
| reasoning 模型 | ~10 | 清空 sampling 参数+warning、max_completion_tokens、systemMessageMode、reasoning tokens | ✅ |
| 扩展设置(store/metadata/promptCache/serviceTier) | ~18 | 请求体字段透传 | ✅ |
| 流式 doStream 文本/工具 | ~10 | stream-parts 序列快照（text-delta/tool-start/delta/end） | ✅ mock `data: ...\n\n` 分块 |
| 流式工具边界 | ~6 | 空 chunk 不重复、partial JSON 不提前 finalize、缺 type、单 chunk | ✅ 高价值边界用例 |
| 流式错误 | ~4 | 首 chunk 错误→statusCode/isRetryable、输出后错误、unparsable part | ✅ |
| 纯函数 convert-to-messages | 33 | system/user/image/file/tool 消息转换 | ✅ 无需 HTTP，最易翻译 |
| 纯函数 prepare-tools | 12 | strict/toolChoice 序列化 | ✅ |

## Anthropic 测试用例清单

主文件 [anthropic-language-model.test.ts](../reference/ai/packages/anthropic/src/anthropic-language-model.test.ts)（10395 行，277 个 `it`）。配套纯函数：`convert-to-anthropic-prompt.test.ts`（82）、`anthropic-prepare-tools.test.ts`（51）、`convert-anthropic-usage.test.ts`（15）。

| 类别 | 用例数 | 关键断言 | 可复用性 |
|------|--------|----------|----------|
| reasoning 映射(thinking/adaptive/顶层/providerOptions) | ~30 | thinking config、budget clamping、strip sampling params、effort 映射 | ✅ |
| JSON schema 响应格式 | ~20 | 工具回退 vs output_config.format、schema 净化 | ✅ |
| structured-outputs beta header | ~7 | beta header 条件触发 | ✅ |
| 基础(text/usage/stop_sequence/refusal) | ~10 | 内容/usage/stop 元数据 | ✅ |
| temperature/topP 互斥、max_tokens、clamping | ~12 | 互斥逻辑、clamping+warning | ✅ |
| cache control | ~4 | cache_control 字段 + providerMetadata | ✅ |
| citations(PDF/text) | ~4 | citation 解析 | ✅ |
| 服务端工具(web search/fetch/code exec/memory/skills) | ~70 | 请求体 include/tool + 响应 content 解析 | ✅ 但量大，需裁剪 |
| context management(compact/clear_*) | ~15 | 请求编辑 + 响应 iterations 解析 | ✅ |
| doStream(以上流式版) | ~50 | stream-parts 快照 | ✅ |
| getModelCapabilities | ~12 | 模型能力矩阵 | ✅ 纯函数 |
| transformRequestBody/custom provider name | ~10 | 请求体变换钩子 | ⚠️ 需设计 trait 钩子 |

## Conformance test

**不存在跨 provider 统一测试套件。** 全仓搜索 `conformance`/`shared-test`/`provider-test` 零命中。每个 provider 独立测试，`packages/openai-compatible` 是独立 provider 包而非一致性框架。`packages/test-server` 仅是共享 MSW mock 基础设施。

**结论：Rust 侧需自建 conformance harness**——建议抽象 `LanguageModel` trait 后用参数化测试跑统一 doGenerate/doStream 矩阵。

## SSE 解析测试

`parse-json-event-stream.ts` 依赖第三方 `eventsource-parser`，**无独立单测**。SSE 行为仅由 OpenAI/Anthropic 流式用例隐式覆盖：`[DONE]` 哨兵、多 chunk 工具调用、错误中途插入、Azure content filter 前缀 chunk、缺 type 字段。

`extract-lines.test.ts`（8 例）覆盖 `\n`/`\r\n`/`\r` 行尾、越界 endLine、单行。

**Rust 侧建议**：用 `eventsource-stream` crate 或自实现，并**补齐 TS 缺失的独立 SSE 单测**（半行跨 chunk、心跳、字段大小写、`[DONE]` 后多余数据）。

## 错误处理测试

| 文件 | 用例 | 覆盖 | 可复用性 |
|------|------|------|----------|
| `handle-fetch-error.test.ts` | 7 | abort、Node fetch failed、browser Failed to fetch、Bun 各类、unknown | ⚠️ abort/unknown 可翻译；Bun/浏览器分支 ❌ |
| `get-from-api.test.ts` | ~16 | 200 解析、404 API error、网络错误、abort、header 清理、URL 重定向安全 | ✅ 核心几例 |
| `retry-with-exponential-backoff.test.ts` | 19 | retry-after(秒/ms/HTTP date)、指数退避、负/非法 header、多次重试、优先 ms、Gateway 限流/认证(不重试)、APICallError cause | ✅ **最高价值**，几乎 1:1 翻译 |
| 模型内流式错误 | ~8 | 首 chunk 错、statusCode 保留、输出后错误、529 overloaded、流中 overloaded | ✅ |

## 可直接翻译的测试汇总

### 优先级排序

| 优先级 | 类别 | 用例数 | 翻译方式 |
|--------|------|--------|----------|
| **P0** | 纯函数（消息转换/工具准备） | ~213 | `#[test]` 直接翻译，无 HTTP |
| **P0** | 错误/重试 | ~36 | mock HTTP 状态码 + header |
| **P1** | 请求构造断言 | ~65 | wiremock 录制请求体 JSON 比对 |
| **P1** | 响应解析 doGenerate | ~32 | mock JSON 响应体 |
| **P2** | 流式 doStream | ~70 | mock `data: ...\n\n` 分块，断言 `Vec<StreamPart>` |
| **P3** | 服务端工具 | ~85 | 量大但重复，各抽 2-3 例 |

### 估算

- **P0+P1 约 346 例可直接翻译**
- **加 P2 约 416 例**
- 需放弃：Bun/浏览器 fetch 错误分支（~4 例）、依赖 JS DOM 流的细节
- 需自建：conformance harness、SSE 独立单测（TS 缺失）

### Fixture 文件可直接复用

`__fixtures__/*.json`（API 响应快照）和 `*.chunks.txt`（SSE 分块）可原样拷贝到 Rust 测试的 `tests/fixtures/` 目录，用 `include_str!` 加载。

### 需要替换的技术栈

| TS 原版 | Rust 替代 | 说明 |
|---------|-----------|------|
| MSW (Mock Service Worker) | `wiremock` | 真起 HTTP server |
| `ReadableStream` | `tokio_stream` / `futures::stream` | 流式测试 |
| `inlineSnapshot` | `insta` crate | 快照断言 |
| vitest `vi.mock` | `cfg(test)` | 版本注入 |

---

## 复核记录

**复核员**：Zara | **复核日期**：2026-07-25

| # | 声明 | 结论 | 证据 |
|---|------|------|------|
| 1 | OpenAI 89 个 it + 配套 45 纯函数 | ✅ | 数字本身准确，但覆盖范围严重不完整（见审计） |
| 2 | Anthropic 277 个 it + 配套 148 纯函数 | ✅ | 4 文件数字全对，但漏 10 文件 47 it |
| 3 | 无 conformance test 套件 | ✅ | 全仓搜索零命中 |
| 4 | SSE 无独立单测 | ✅ | 确认 |
| 5 | retry 19 例最高价值 | ❌ | provider-utils 无 retry 测试；全仓唯一 retry 测试在 ai/src/util，15 it |
| 6 | P0+P1 约 346 例可翻译 | ❌ | 与自身分项之和(609)矛盾；实测仅 4 核心包就 4866 用例 |

### 测试统计审计（Zoe，2026-07-25）

**原报告统计严重不完整**，完整审计见 [13-test-audit.md](13-test-audit.md)。关键修正：

| 维度 | 原报告 | 实测 | 偏差 |
|------|--------|------|------|
| OpenAI | 134 it / 3 文件 | **611 it / 26 文件** | 漏整个 responses/ 子目录(357 it) |
| Anthropic | 425 it / 4 文件 | **474 it / 14 文件** | 4 文件数字全对，漏 10 文件 47 it |
| provider-utils | 50 it / 4 文件 | **808 it / 88 文件** | 漏 84 文件；retry 项不存在 |
| packages/ai/src | 只列文件名 | **2973 it / 143 文件** | 用例数此前完全缺失 |
| 其他 provider | 未提 | **2287 it / 110 文件** | 13 个包全部遗漏 |
| **审计总计** | **346** | **7153 用例 / 381 文件** | **低估约 20 倍** |

原报告具体错误：
1. **OpenAI 漏整个 responses/ 子目录**（357 it）——最严重遗漏
2. **retry 19 例不存在**——provider-utils 无 retry 测试，全仓唯一 retry 测试在 ai/src/util，15 it
3. **handle-fetch-error 7→8**、**get-from-api 16→17** 各少算 1
4. **346 总数不可用**——与自身分项之和(609)矛盾
5. **provider-utils 漏 84/88 文件**——仅点了 4 个且 1 个不存在
6. **其他 provider 全部未提**——google(701)/xai(416)/bedrock(410) 等共 2285 it
