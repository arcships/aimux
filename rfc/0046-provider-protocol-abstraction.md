# RFC-0046：provider 协议的小重构与抽象设计

> **Status**: Draft
>
> **Date**: 2026-10-20
>
> **Scope**: `aimux-providers` 里各家协议实现（OpenAI Chat、OpenAI Responses、Anthropic Messages、Google generateContent、Bedrock Converse 等）怎么分层、怎么被别的厂商复用：通用函数下沉到 `aimux-provider-utils`，每种协议一个实现、用 `XxxConfig` 承载变体差异，托管变体只换配置和鉴权，厂商之间只经过 `internal` 入口单向依赖。目录、文件名和测试位置按这套分层落地，能逐个对照上游包。是对现有代码的小重构：不改变公开 API，不改变 wire 行为；第 3.4 节 Vertex Gemini 的合并会修正已经走样的行为，单独登记。crate 怎么拆见 RFC-0044，两份互不依赖。
>
> **Baseline**: `master@5743a40a`。上游对照：`fixtures/aisdk/VERSIONS.json` 锁定的包版本（`scripts/fetch-aisdk.sh` 取得的发布包），目录与命名另参考 `vercel/ai` main `fce9210`。
>
> **Related**: [对齐文档](../docs/aisdk-architecture-alignment.md) §2（D17 五层）、§3.2（fetch 链）、§9.1；[RFC-0042](0042-v4-type-boundaries.md) T2（共享流处理留在 providers）；[RFC-0044](0044-crate-layering-and-dependency-inversion.md)（crate 拆分与依赖反转，PR #279；`aimux-providers` 内部的目录和测试由本文定）；RFC-0045（上游测试迁移，引用本文的测试位置和 `layout.toml`）；[#275](https://github.com/arcships/aimux/issues/275)、[#276](https://github.com/arcships/aimux/issues/276)、[#278](https://github.com/arcships/aimux/issues/278)。

## 1. 决策摘要

| 编号 | 决定 | 后果 |
|---|---|---|
| V1 | provider 代码分三层：通用函数在 `aimux-provider-utils`；协议实现是一个模型加一个 `XxxConfig`（OpenAI Chat、OpenAI Responses、Anthropic Messages、Google generateContent、Bedrock Converse，加兼容层）；厂商只是工厂、Config 的取值和 Fetch 层的鉴权（§3.1） | 每种协议只有一份实现；厂商之间的差异都落在 Config 字段和 Fetch 层 |
| V2 | 上游放在 provider-utils 的通用函数，我们也放在 `aimux-provider-utils`；厂商代码不再自带一份，也不再去别的厂商目录拿 | 补齐 parseProviderOptions、流错误、响应元数据、推理强度映射等（§3.2） |
| V3 | 厂商之间只允许单向依赖，只能用对方 `internal` 模块导出的东西（对应上游 `@ai-sdk/<vendor>/internal`），CI 检查 | 解开现有 7 个环（§3.3） |
| V4 | 托管变体复用协议实现，只换配置：Anthropic 的模型 + `AnthropicConfig`，Google 的模型 + `GoogleConfig`，兼容层 + `OpenAICompatibleConfig`；类型名和字段名照上游 | Vertex Gemini 删掉复制的流处理和结果解析；AWS、Vertex 上的 Claude 不再有自己的模型（§3.4） |
| V5 | 厂商之间内部用的 Config 钩子是封闭枚举，每个变体是一种已知改写，逻辑在厂商目录的普通函数里，由 `match` 分发；用户能填的设置照上游保留回调（只有兼容层有）；鉴权和签名走 Fetch 层 | 配置可以打印、比较、写进录制，绑定和 preset 按名字选（§3.5） |
| V6 | 有独立上游包的厂商（Groq、DeepSeek、Mistral、Cohere、xAI、HuggingFace、Open Responses）保留自己的转换代码，和上游一样 | 不为了去重把它们硬并进兼容层或 OpenAI（§3.7） |
| V7 | 模态和搜索厂商的机械重复抽成公共结构和函数；各家的请求构造、响应解析不抽 | 约 2,500 行重复，抽掉其中 10 类完全重复（§3.6） |
| V8 | 按这套分层落目录：每个厂商一个目录 `src/<vendor>/`，`mod.rs` 只做声明和导出（对应上游 `index.ts`）；有上游包的厂商文件与上游包逐个对应，命名规则固定（§4.2） | 文件和测试能按名字对照上游 |
| V9 | 上游有、我们没实现的文件不建空文件，在 `layout.toml` 里记为未实现并挂 issue | 缺口可见，不制造占位代码 |
| V10 | 测内部函数的单元测试放在源文件旁边：`foo.rs` 里写 `#[cfg(test)] mod tests;`，测试在 `foo/tests.rs`（Rust 默认的模块路径，Rig 也这样）；走公开 API 的测试留在 `tests/`，按厂商分子目录，合成一个测试程序 | 上游测试能逐个对照，测试和代码一起搬、一起删；`tests/` 下 126 个测试程序合成一个 |
| V11 | 本文不拆 crate、不加按厂商的 cargo feature，厂商仍在一个 `aimux-providers` 里，以后再考虑；参考做法是只有厂商要引入很重的第三方库时才单独成 crate（Rig 的做法：27 个厂商在 rig-core，只有用 AWS SDK、Google Cloud SDK、gRPC、本地推理的拆出去） | 48 个厂商实现中文本模型占约 5.8 万行，多模态和搜索 31 家只有约 1.4 万行，按厂商开关省不了多少体积 |

## 2. 现状

### 2.1 协议实现和谁在用

48 个厂商实现跑在几种协议上，同一种协议现在有多份实现：

| 协议 | aimux 的实现 | 跑在上面的 | 上游 |
|---|---|---|---|
| OpenAI Chat Completions | `openai/`（chat）、`openai_compatible/` | Azure（经 OpenAI）；282 个 preset、Vertex 上的合作模型（`vertex/partner_provider.rs:215`，经兼容层）；Groq、DeepSeek、Mistral、Cohere 各写一份 | `@ai-sdk/openai`、`@ai-sdk/openai-compatible`；Groq 等是独立包，各写一份 |
| OpenAI Responses | `openai/responses/` | Azure、Codex（`codex.rs:56-60` 直接用 OpenAI 的私有模块）；另有 `open_responses.rs`（1,430 行）、`xai/responses/`（2,314 行）、`huggingface/responses.rs`（1,157 行）各一份 | `@ai-sdk/open-responses`、`@ai-sdk/xai`、`@ai-sdk/huggingface` 也各自一份，只依赖 provider 和 provider-utils |
| Anthropic Messages | `anthropic/` | Anthropic AWS（`anthropic_aws/mod.rs:40-44`）、Vertex 上的 Claude；Bedrock 借用它的工具准备和 schema 清洗（`bedrock/tools.rs:67`、`bedrock/convert.rs:1348`） | 托管变体走 `@ai-sdk/anthropic/internal` |
| Google generateContent | `google/` | Vertex 上的 Gemini：流处理和结果解析整份复制（§2.4） | `@ai-sdk/google-vertex` 复用 `@ai-sdk/google` 的模型 |
| Bedrock Converse | `bedrock/` | — | `@ai-sdk/amazon-bedrock` |
| 各家自己的协议 | 31 个模态与搜索厂商 | — | 有包的照包 |

两条调用栈能看出现在的问题。基础厂商去别的厂商私有文件拿通用函数（Mistral）：

```text
aimux_providers::mistral::create_mistral           aimux-providers/src/mistral/mod.rs:161
 └ MistralProvider::language_model                 aimux-providers/src/mistral/mod.rs:270
   └ MistralModel::do_generate                     aimux-providers/src/mistral/model.rs:176
     └ build_request_body                          aimux-providers/src/mistral/convert.rs:225
       └ convert_prompt_to_mistral_messages        aimux-providers/src/mistral/convert.rs:105
         └ crate::openai::convert::tool_result_to_content
                                                    aimux-providers/src/mistral/convert.rs:167 → openai/convert.rs:512   ← 去 OpenAI 的私有文件拿
     └ aimux_provider_utils::post_json_to_api      aimux-providers/src/mistral/model.rs:180
```

上游 `@ai-sdk/mistral` 只依赖 `@ai-sdk/provider` 和 `@ai-sdk/provider-utils`，工具结果在 `convert-to-mistral-chat-messages.ts:167` 自己转换。

协议实现反过来认识托管变体（Vertex 上的 Claude）：

```text
Vertex provider 建 Claude 模型                      aimux-providers/src/vertex/anthropic_provider.rs:85
 └ vertex::anthropic_model::model                  aimux-providers/src/vertex/anthropic_model.rs:63
   └ AnthropicEndpoint::Vertex(resolver)           aimux-providers/src/vertex/anthropic_model.rs:86
     └ AnthropicModel::do_generate                 aimux-providers/src/anthropic/model.rs:152
       └ prepare → config.resolved()               aimux-providers/src/anthropic/model.rs:111 → :116
         └ AnthropicModelConfig::resolved          aimux-providers/src/anthropic/config.rs:156
           └ match AnthropicEndpoint::Vertex       aimux-providers/src/anthropic/config.rs:160
             └ crate::vertex::Publisher::Anthropic aimux-providers/src/anthropic/config.rs:162   ← Anthropic 认识 Vertex（环）
```

上游托管变体走协议包的 `/internal` 入口，协议包不认识变体：`amazon-bedrock/src/anthropic/amazon-bedrock-anthropic-provider.ts:17` 从 `@ai-sdk/anthropic/internal` 取模型。

### 2.2 厂商之间的引用

上游每个厂商包只依赖 `@ai-sdk/provider` 和 `@ai-sdk/provider-utils`；跨包复用只有托管变体，而且走对方的 `/internal` 入口（`amazon-bedrock` 用 `@ai-sdk/anthropic/internal`）。Groq、Mistral 的消息转换与 OpenAI 的结构相似度只有 0.44、0.46，各写各的。

我们反过来，厂商之间直接引用对方的私有文件：

| 被引用 | 引用方 | 性质 |
|---|---|---|
| `openai/convert.rs:512` `tool_result_to_content`、`:527` `tool_result_content_value` | Open Responses、xAI、兼容层、Mistral、Groq、DeepSeek、Cohere、Google | 通用函数放错了地方 |
| `openai/convert.rs:656` `parse_chat_provider_options`、`:636` `parse_option_fields` | Groq、DeepSeek、Anthropic | 上游的 `parseProviderOptions`，应在 provider-utils |
| `openai/model.rs` `list_models_once` | Codex、HuggingFace | 应在 `shared/discovery.rs` |
| `google/utils.rs` `GoogleJsonAccumulator` | Vertex | 合理复用，但要走导出入口 |
| `openai/tools.rs` `AzureOpenAITools` | Azure | Azure 专属类型放在了 OpenAI 里 |
| Anthropic、Google、OpenAI、兼容层的模型和配置 | AWS、Vertex、Bedrock、Azure、Codex、HuggingFace | 合理复用（上游也这样），要走导出入口 |

七个环：

| 环 | 证据 |
|---|---|
| OpenAI ↔ Azure | `openai/config.rs:24` 用 `azure::UrlRules` |
| OpenAI ↔ 兼容层 | `openai/completion.rs:5` 把 completion 交给兼容层；兼容层 `completion.rs:25,201,479,573` 用 OpenAI 的配置、错误处理、流错误 |
| 兼容层 ↔ preset | `openai_compatible/config.rs:27` 用 `PresetDescriptor`；`preset.rs:39-41` 用兼容层 |
| OpenAI ↔ decision_support | `openai/decision.rs:19,150,470`；`decision_support.rs:78-79` |
| 兼容层 ↔ runtime_decision | `openai_compatible/mod.rs:342-346`；`runtime_decision.rs:14` |
| Anthropic ↔ Vertex | `anthropic/config.rs:50,160-163` 用 `vertex::Resolver`、`Publisher` |
| Anthropic ↔ Anthropic AWS | `anthropic/config.rs:54,68,172-179` 用 `Auth`、`AwsHeaders` |

另外 `stream/chat_tool_calls_tests.rs` 引用了 4 个厂商，公共层的测试依赖上层；生产代码里 `stream/`、`shared/` 不引用厂商。

### 2.3 provider-utils 的缺口

上游 provider-utils 导出、而我们没有公共实现的：

| 上游 | 我们现在 |
|---|---|
| `parseProviderOptions` | 约 25 处各自解析，3 家借 OpenAI 的 |
| `createProviderStreamError` | 9 个厂商各写，15 处直接拼 `StreamPart::Error` |
| `createLanguageModelResponseMetadata` | 15 个文件 16 次手拼 |
| `mapReasoningToProviderEffort` / `Budget` | 只在 `anthropic/convert.rs:1751`、`:1813`；Cohere `convert.rs:429` 抄了同一张表 |
| `resolveProviderReference` | `openai/convert.rs:255`、`xai/convert.rs:48`、`anthropic/convert.rs:1521` 各一份 |
| `convertToBase64`、`convertImageModelFileToDataUri` | 约 35 个文件直接用 base64 库，18 处手拼 data URL |
| 流式工具调用参数追踪 | 在 `aimux-providers/src/stream/`（RFC-0042 T2 决定留在 providers） |
| `ToolNameMapping` | 只在 `anthropic/tool_name_mapping.rs:62` |
| `isValidHostnamePart`、`loadOptionalSetting` + `validateBaseURL`、凭据、UA | 在 `aimux-providers/src/shared/mod.rs:35-300` |

### 2.4 托管变体

| 变体 | 和基础模型的差异 | 现状 |
|---|---|---|
| Anthropic AWS | 9 处：区域 URL、区域校验、key 或 SigV4、工作区 header、UA、名字、无 URL 来源、无 files/skills | 已复用 Anthropic 模型，但靠反向引用（环） |
| Vertex 上的 Claude | 10 处；只有 Google 令牌签发是独立代码（`vertex/auth*.rs`） | 同上 |
| Bedrock Converse | 信封、消息转换、AWS 二进制事件流都不同 | 独立实现；`bedrock/convert.rs:858-872`、`:961-975` 抄了 Anthropic 的能力表 |
| Vertex 上的 Gemini | 11 处 | 流处理（`vertex/model.rs:244-677` 对 `google/model.rs:256-610`）和结果解析（`:819-1028` 对 `:724-939`）整份复制且已走样；ID 规则不同；用量计算、端点拼接、embedding、video 各有一份 |

`AnthropicAwsFetch`（`anthropic_aws/model.rs:210-232`）和 `BedrockAuthFetch`（`bedrock/mod.rs:364-395`）是同一个 SigV4 鉴权层写了两次。

### 2.5 模态与搜索厂商

31 个厂商（21 个双形态 + 10 个单文件）里的重复，按行数：

| 重复 | 厂商数 | 约行数 |
|---|---|---|
| 厂商工厂：设置、Debug、create、默认实例、模型配置 | 31 | 900 |
| `options.rs` 只有 13 行，除名字外完全一样 | 20 | 260 |
| 异步任务的提交、轮询、下载 | 8 | 250 |
| 搜索请求的头尾和结果映射 | 11 | 200 |
| 错误体 `{"error":{message, code}}` 解析（6 个一字不差） | 12 | 150 |
| 响应尾部 headers、body、时间戳 | 29 | 150 |
| `Warning::Unsupported` 手拼（全仓 147 处） | — | 100 |
| 任务失败错误 | 8 | 100 |
| 多模态 provider 的 no_such_model 桩 | 6 | 100 |
| 下载生成结果 URL（带 SSRF 校验），5 处相同 | 5 | 90 |
| 音频转字节、multipart 前缀 | 7 | 80 |
| 图片、视频转 URL 或 data URL | 5 | 70 |
| 透传 providerOptions（去掉轮询键） | 6 | 40 |
| 完全相同的小函数（`gcd`、宽高比、`resolve_count`） | 4 | 30 |

各家的 `do_generate` 两两相似度只有 0.33–0.43，请求构造 0.43–0.49，这部分是真正的厂商差异。

### 2.6 目录

`aimux-providers/src` 有 37 个目录、44 个文件：

| 形态 | 数量 | 例子 |
|---|---|---|
| 带 `mod.rs` 的厂商目录 | 14 | `openai/`、`anthropic/`、`google/`、`vertex/`、`bedrock/` |
| `<vendor>.rs` + 只有 `options.rs` 的同名目录 | 21 | `fal.rs` + `fal/options.rs`、`elevenlabs.rs` + `elevenlabs/options.rs`；`huggingface/` 另有 `responses.rs` |
| 单文件厂商 | 13 | `codex.rs`、`jev.rs`、`tavily.rs`、`serper.rs`、`open_responses.rs` |
| 共用基础设施 | 2 | `shared/`、`stream/` |
| 非厂商顶层文件 | 9 | `catalogue.rs`、`default_providers.rs`、`preset.rs`、`provider_registry.json`、`replay.rs`、`decision_support.rs`、`decision_scoring.rs`、`runtime_decision.rs`、`systemone.rs` |

文件名用通用名（`model.rs`、`convert.rs`、`types.rs`），不对应上游文件。282 个 preset 是 `provider_registry.json` 里的数据行，跑在兼容层上。

### 2.7 与上游对照

上游不是每个包文件数相同，而是命名规则固定、一个模型类一个文件、测试紧挨源文件：

| 规则 | 出现的包数 |
|---|---|
| `<x>-provider.ts`、`*-options.ts`、`version.ts`、`index.ts` | 13 / 13 |
| `<x>-error.ts`、`*-api-types.ts` 或 `*-api.ts` | 12 / 13 |
| `<x>-chat-language-model.ts` / `<x>-language-model.ts` | 有文本模型的全部 |
| `convert-to-<x>-*.ts`、`*prepare-tools.ts` | 有文本模型的全部 |
| 其余模态：每种模型一个 `<x>-<模态>-model.ts` + `-options.ts` | 有该模态的全部 |
| 测试 `foo.test.ts` 与 `foo.ts` 同目录 | 13 个包里 6 个全部如此，其余只有 1–5 个跨文件场景测试例外 |

13 个同名上游包逐文件对照（上游不计 `index.ts`、测试、`*.test-d.ts`、fixtures）：

| 厂商 | 上游文件 | aimux 文件 | 有对应 | aimux 文件要拆 | 上游有我们没有 | aimux 独有 |
|---|---|---|---|---|---|---|
| openai | 77 | 21 | 66 | 13 | 11 | 2 |
| google | 64 | 12 | 39 | 10 | 25 | 0 |
| anthropic | 44 | 14 | 43 | 7 | 1 | 0 |
| xai | 39 | 9 | 32 | 7 | 7 | 1 |
| amazon-bedrock | 32 | 12 | 30 | 6 | 2 | 1 |
| mistral | 22 | 8 | 17 | 5 | 5 | 0 |
| groq | 17 | 11 | 17 | 4 | 0 | 0 |
| fal | 17 | 2 | 14 | 2 | 3 | 0 |
| cohere | 15 | 7 | 15 | 3 | 0 | 0 |
| deepseek | 15 | 9 | 12 | 3 | 3 | 0 |
| deepgram | 12 | 2 | 8 | 2 | 4 | 0 |
| elevenlabs | 12 | 2 | 12 | 2 | 0 | 0 |
| luma | 5 | 2 | 5 | 1 | 0 | 0 |
| 合计 | 371 | 111 | 310 | 65 | 61 | 4 |

“上游有我们没有”主要是：Google Interactions（17 个文件）、各家 realtime 与 live、batch、speech-translation、xAI 和 DeepSeek 的文件接口、Mistral Conversation。其余 29 个厂商在上游没有包。

### 2.8 测试

`aimux-providers/tests/` 有 126 个文件，每个是一个独立的测试二进制：101 个能按前缀对应到厂商，8 个是 preset（alibaba、baseten、cerebras、deepinfra、fireworks、moonshotai、openrouter、perplexity），17 个是跨厂商场景（cassette、conformance、e2e）。另有 `tests/common/`（14 个文件 `mod common;`，22 个文件用 `#[path]` 引 `mock_fetch.rs`）、`tests/cassettes/`（2,799 个文件，176 MB，`Cargo.toml` 的 `exclude` 不打包）、`tests/fixtures/`、`tests/replay_fixtures/`。

搬进同一个测试二进制要处理的：

- 18 个文件改环境变量，共 88 个 `#[serial]`；
- 已有的冲突：`bedrock_user_agent_test.rs:109`、`sigv4_signed_headers_test.rs:74` 断言 `AWS_BEARER_TOKEN_BEDROCK` 未设置，而 `bedrock_remaining_test.rs:394` 会设置它；`vertex_upstream_e2e_test.rs:443`、`:649` 读 `GOOGLE_*` 却没有 `#[serial]`；
- `xai/responses/mod.rs:44` 的 `SOURCE_ID_COUNTER` 依赖执行顺序；`replay_test.rs:214` 用固定临时路径；
- 181 个测试函数名在多个文件里重名（搬进各自目录后在不同模块里，不冲突）。

`src/` 里已有 19 个内联测试模块，`stream/chat_tool_calls_tests.rs` 是唯一的独立测试文件（复数 `_tests`）。

## 3. 抽象设计

### 3.1 三层

```text
厂商（工厂、Config 取值、鉴权 Fetch）      vertex/、anthropic_aws/、azure/、codex/、preset 数据行 …
   │ 只经过 internal，单向（§3.3）
   ▼
协议实现（模型 + XxxConfig）               openai/（Chat、Responses）、openai_compatible/、anthropic/、google/、bedrock/
   │                                       独立上游包的厂商（Groq、Mistral、xAI …）各自是自己的协议实现
   ▼
通用函数（aimux-provider-utils）            parse_provider_options、流错误、响应元数据、工具结果转换、SigV4 …
```

改完后同一次调用的样子：

```text
Mistral：MistralModel::do_generate → convert_prompt_to_mistral_messages
 └ aimux_provider_utils::tool_result_to_content         ← 通用函数，不再进 openai/

Vertex 上的 Claude：vertex 工厂
 └ AnthropicModel + AnthropicConfig { endpoint: Arc<dyn ResolveEndpoint>, build_request_url: VertexRawPredict, … }
   └ vertex 实现 ResolveEndpoint；Google 令牌在 Fetch 层     ← Anthropic 不再认识 vertex::Resolver、Publisher
```

一个厂商要么是协议实现（有自己的模型），要么是托管变体（只有工厂和 Config 取值），不出现“半个模型复制一份”的中间形态。

### 3.2 provider-utils 补齐

| 加入 `aimux-provider-utils` | 取代 |
|---|---|
| `parse_provider_options` | 各家手写解析、`openai/convert.rs:636,656` |
| `create_provider_stream_error` | 9 个厂商的流错误构造；各家只留状态码表 |
| `create_language_model_response_metadata` | 16 处手拼 |
| `map_reasoning_to_provider_effort`、`map_reasoning_to_provider_budget` | Anthropic 和 Cohere 的两份表 |
| `resolve_provider_reference` | 3 份 |
| `convert_to_base64`、`convert_image_model_file_to_data_uri` | 直接调用 base64 库和手拼 data URL |
| `tool_result_to_content` 一族 | `openai/convert.rs:512,527` |
| `ToolNameMapping` | `anthropic/tool_name_mapping.rs` |
| 主机名校验、base URL 设置、凭据、UA 组装 | `shared/mod.rs:35-300` |

名字是上游函数的 snake_case。provider-utils 的死代码一并删除：`headers.rs:38` `with_user_agent_suffix_fn`、`url.rs:16` `without_trailing_slash_opt`（只有测试用）。

### 3.3 协议实现之间的依赖

1. 每个被别家复用的厂商有一个 `internal` 模块，只 `pub use` 允许别家用的东西，对应上游 `@ai-sdk/<vendor>/internal`。别家只能 `use crate::<vendor>::internal::...`。
2. 允许的方向只有托管变体到基础厂商：Azure → OpenAI，Codex → OpenAI，HuggingFace → OpenAI，Amazon Bedrock → Anthropic，Anthropic AWS → Anthropic，Google Vertex → Google / Anthropic / 兼容层，preset → 兼容层。其余厂商之间不允许引用。
3. 七个环的解法：

| 环 | 解法 |
|---|---|
| OpenAI ↔ Azure | `UrlRules` 移进 OpenAI 的配置，Azure 只填值 |
| OpenAI ↔ 兼容层 | OpenAI 照上游自己写 completion（上游 openai 包有自己的 `completion/`）；错误处理、流错误下沉到 provider-utils（§3.2） |
| 兼容层 ↔ preset | preset 产出 `OpenAICompatibleConfig`，兼容层不认识 preset |
| OpenAI ↔ decision、兼容层 ↔ runtime_decision | decision 代码依赖一个小接口（发请求所需的端点和 headers），OpenAI 和兼容层实现它 |
| Anthropic ↔ Vertex、Anthropic ↔ Anthropic AWS | `AnthropicConfig` 持有 `Arc<dyn ResolveEndpoint>`（`shared/exchange.rs:141-144` 已有，Vertex Gemini、Bedrock、Polly 在用），Vertex 和 AWS 各实现一个；删掉与 `EndpointHeaders` 重复的 `AnthropicHeaders::Aws`（`anthropic/config.rs:61-69`） |

4. CI 检查（扩展已在 CI 里的 `scripts/check_provider_boundaries.sh`，`ci.yml:714`）：厂商之间的 `use` 只能指向第 2 条允许的方向且只能经过 `internal`；`shared/`、`stream/`、`test_support/` 不引用厂商。脚本里现有的 `|| true` 会让改名后的路径静默通过，改成路径不存在即失败。

### 3.4 托管变体复用协议实现

| 变体 | 做法 |
|---|---|
| Anthropic AWS | Anthropic 的模型 + `AnthropicConfig`：`build_request_url` 指向区域端点，SigV4 或 key 在 Fetch 层 |
| Vertex 上的 Claude | Anthropic 的模型 + `AnthropicConfig`：`:rawPredict` 路径、`transform_request_body` 去掉 `model` 加 `anthropic_version`、不支持原生结构化输出和 strict tools；Google 令牌在 Fetch 层 |
| Vertex 上的 Gemini | 删掉 `vertex/model.rs` 里复制的流处理和结果解析，改用 Google 的模型 + `GoogleConfig`：端点解析、命名空间、一张字段剥离表；`sharedRequestType` 等 header 和 `streamFunctionCallArguments` 是 Config 字段。ID 生成改用注入的 `generate_id`（现在是计数器），这会改变 Vertex 输出的 ID，登记为修正 |
| Bedrock Converse | 另一种协议，保持独立；能力表改为调用 Anthropic 的 `get_model_capabilities`（`anthropic/convert.rs:1674-1720`） |
| Azure | OpenAI 的模型 + 配置，和上游一样 |

两个 SigV4 鉴权层合成一个，放在 provider-utils（和 `sigv4_fetch.rs` 一起）。

### 3.5 Config 的写法

类型名和字段名照上游：`OpenAICompatibleConfig`（`include_usage`、`supports_structured_outputs`、`error_structure`、`metadata_extractor`、`transform_request_body`）、`AnthropicConfig`（`build_request_url`、`transform_request_body`、`transform_request_headers`、`supports_native_structured_output`、`supports_strict_tools`）、`GoogleConfig`（`provider`、`base_url`、`headers`、`fetch`、`generate_id`、`supported_urls`）。aimux 自己加的字段放进同一个 Config。

上游这些钩子是回调函数，分两种：

- 用户能填的：只有兼容层，`createOpenAICompatible` 的设置里可以传 `transformRequestBody`、`metadataExtractor`、`supportedUrls`、`convertUsage` 这些函数。Rust 版 `OpenAICompatibleProviderSettings`（`openai_compatible/mod.rs:64`）已经照样开放，保留不动；绑定传不了函数，只能用 preset 里按名字选的改写。
- 厂商之间内部用的：Anthropic、Google 的 `transformRequestBody`、`buildRequestUrl` 等只给上游自己的 Bedrock、Vertex 包用，用户设置里没有（上游 `AnthropicProviderSettings` 只开放 `baseURL`、`apiKey`、`authToken`、`headers`、`fetch`、`generateId`、`name`）。这一类我们写成封闭枚举：

```rust
pub enum TransformRequestBody {
    VertexMaas,
    VertexXai,
    VertexAnthropic,
    BedrockAnthropic,
}

pub enum BuildRequestUrl {
    Messages,          // {base_url}/messages
    VertexRawPredict,  // …/{model}:rawPredict 或 :streamRawPredict
    AwsRegional,       // aws-external-anthropic.{region}.api.aws/v1/messages
}

pub struct AnthropicConfig {
    pub provider: String,
    pub endpoint: Arc<dyn ResolveEndpoint>,
    pub build_request_url: BuildRequestUrl,
    pub transform_request_body: Option<TransformRequestBody>,
    pub supports_native_structured_output: bool,
    pub supports_strict_tools: bool,
}

// 模型里只有一个 match，逻辑在厂商目录的普通函数里
match &config.transform_request_body {
    Some(TransformRequestBody::VertexAnthropic) => vertex::anthropic::transform_request_body(&mut body),
    …
}
```

兼容层内部的模型配置（`CompatModelConfig`）同时接受两种来源：用户传的回调原样保留，我们自己的厂商（Vertex 上的合作模型，`vertex/partner_provider.rs:219,310,311`）改用枚举。端点解析 `ResolveEndpoint` 保留为接口，因为它要在请求时读区域、项目和令牌，这是行为不是配置。

鉴权、签名不进 Config，在 Fetch 链里（对齐文档 §3.2 的 auth fetch 位置）。

### 3.6 模态与搜索厂商

抽成公共部分：

| 抽成 | 取代 |
|---|---|
| 厂商工厂的公共结构和宏（设置、Debug、默认实例、模型配置），扩展已有的 `impl_single_modality_provider!` 到多模态 | 31 个厂商的工厂代码、6 个多模态的 no_such_model 桩 |
| 一个读命名空间选项的函数 | 20 个只有 13 行的 `options.rs`（删掉） |
| 搜索请求函数（端点、请求、错误、结果映射） | 11 个搜索厂商的头尾 |
| 标准错误体解析 | 12 个厂商的同形闭包 |
| 异步任务执行器（提交、轮询、下载），轮询参数是数据 | 8 个厂商的任务代码和各自声明的轮询常量 |
| 生成结果下载（走 RFC-0044 的单一下载路径） | 5 处相同代码 |
| `Warning::unsupported`、任务失败错误、响应尾部的构造函数 | 147 处手拼告警等 |
| 音频、图片、视频输入上的转换方法 | 12 处 |
| 完全相同的小函数合并 | `black_forest_labs.rs:230` 与 `stability.rs:245`、`you_com.rs:178` 与 `tinyfish.rs:177` |

### 3.7 不抽象的

- 各厂商的消息转换、finish reason、用量、工具准备、选项结构、响应结构、请求构造，和上游一样各写各的。
- Groq、DeepSeek、Mistral、Cohere 保留自己的目录和代码。逐项比较后只有 Groq 的差异全部能写成配置（8 项数据、1 项钩子），但上游是独立包、测试按上游文件对照，所以不并进兼容层；§3.2 下沉后它自然变薄。
- xAI、HuggingFace 只有 Responses 实现，HuggingFace 的 `responses.rs`（1,165 行）和 OpenAI Responses 有 12 个同名函数，是否复用 OpenAI 的 Responses 实现在它自己的 PR 里比较后决定。

## 4. 目录与文件

按第 3 节的分层落目录。

### 4.1 厂商目录

1. 每个厂商一个目录 `src/<vendor>/`，`mod.rs` 只做 `mod` 声明和 `pub use`，对应上游 `index.ts`。
2. 厂商目录名保留现有名字（`bedrock/`、`vertex/` 等），不改成上游包名：目录名就是 Rust 的公开模块路径（`aimux_providers::bedrock::…`），改名是破坏性变更，换来的只是名字一致。对应哪个上游包写在 `layout.toml` 里。上游包内的子目录照建，如上游 `amazon-bedrock/src/anthropic/` 对应 `bedrock/anthropic/`。
3. 非厂商的顶层文件：`catalogue.rs`、`default_providers.rs`、`preset/`（`preset.rs` + `provider_registry.json`）留在顶层，它们是注册表和目录，不是厂商；`replay.rs` 按 RFC-0044 C6 移出；`systemone.rs`（Jev 的 wire 编解码）进 `jev/`；`decision_support.rs`、`decision_scoring.rs`、`runtime_decision.rs` 进 `decision/`，只依赖 §3.3 定义的小接口。

### 4.2 有上游包的厂商

1. 目录内文件与上游包逐个对应：kebab-case 换成 snake_case，去掉和目录重复的厂商前缀。`mistral-chat-language-model.ts` → `mistral/chat_language_model.rs`；`map-mistral-finish-reason.ts` → `mistral/map_finish_reason.rs`；`convert-to-mistral-chat-messages.ts` → `mistral/convert_to_chat_messages.rs`。
2. 上游有子目录就照建（openai 的 `chat/`、`responses/`、`tool/`，google 的 `tool/`），任意深度。
3. 去掉前缀后重名时保留区分词：deepgram、elevenlabs 都有 `<x>-api-types.ts` 和 `speech-api-types.ts`，分别是 `api_types.rs`、`speech_api_types.rs`。
4. 上游写在模型文件里的 zod 响应 schema，aimux 放进 `api_types.rs`（上游 12/13 的包本来就有这个文件）。
5. `version.ts` 不建文件，用 crate 版本。
6. 上游有、我们没实现的文件不建空文件，在 `layout.toml` 里记为 `missing` 并写 issue（#275 或 #276）。
7. 上游的实现被我们放在公共层的（Bedrock 的 `sigv4-fetch` 在 provider-utils，OpenAI completion 若按 §3.3 仍由兼容层承担），在 `layout.toml` 里记为 `elsewhere` 并写路径。
8. 上游没有、aimux 独有的文件（`list_models` 发现、SigV4 GET 签名等已登记扩展）和上游对应的文件放在同一目录，不另建子目录；文件第一行注明“aimux 扩展”和对应的登记差异，`layout.toml` 里记为 `aimux`。

### 4.3 没有上游包的厂商

29 个厂商没有上游包，整个目录都是 aimux 自己的。按模态命名：`provider.rs`、`<模态>_model.rs`、`<模态>_model_options.rs`、`api_types.rs`；没有内容的文件不建（§3.6 抽掉公共部分后，大部分厂商只剩 `provider.rs` 和一个模型文件）。

preset 仍是 `provider_registry.json` 里的数据行，不建目录；preset 的测试放在 `preset/` 下。

### 4.4 `layout.toml`

每个有上游包的厂商一个表，逐个列上游源文件：

```toml
[mistral]
package = "@ai-sdk/mistral"           # 版本以 VERSIONS.json 为准

[[mistral.files]]
upstream = "src/mistral-chat-language-model.ts"
rust = "mistral/chat_language_model.rs"
status = "mapped"                     # mapped | missing | elsewhere | not-applicable | aimux

[mistral.files.tests]                 # RFC-0045 §3.2：一律是表；空表表示没有手写排除
ts-only = [
  { case = "MistralChatLanguageModel › …", reason = "…" },
]

[[mistral.files]]
upstream = "src/mistral-conversation-language-model.ts"
status = "missing"
issue = 275
```

`status` 决定这个文件的上游测试迁不迁（`mapped`、`elsewhere` 迁；`missing`、`not-applicable` 不迁）；`mapped`、`elsewhere` 的条目必须有 `tests` 表，表里只列手写排除：`ts-only`、`registered`、`sub-feature` 三类，每条是 `{ case, reason }`，`registered` 另写 `ref`，`sub-feature` 另写 `issue`；single-step、telemetry 由 RFC-0045 的 `check.mjs` 按 API 名判定，不写进表。模块按比锁定版本更新的上游对齐时，条目另写 `version`（目前只有 `aimux` 那张表里的 decide，`ai@7.0.128`）。aimux 独有的文件也进表，记为 `status = "aimux"` 并写 `reason`（对应的登记差异）；没有上游包的厂商不进表。`aimux-provider-utils` 和 `aimux` 照同样格式各有一张（RFC-0044 定目录，本文定格式）。

## 5. 测试位置

参考的 Rust 项目：Rig 的单元测试一律是 `foo/tests.rs`（`#[cfg(test)] mod tests;`，CI 用 `cargo xtask check-test-layout` 强制），录制回放测试在 `rig-cassette/tests/<厂商>.rs` 和 `tests/providers/<厂商>/`；genai、reqwest、axum 的单元测试写在源文件里，`tests/` 放走公开 API 的测试，genai 按厂商一个文件。Rust 的 `tests/` 只能用公开 API，而上游测试大多测内部函数（消息转换、用量转换、finish reason、工具准备），这些只能放在 crate 里面。

1. 测内部函数的单元测试放在源文件旁边：`chat_language_model.rs` 里写 `#[cfg(test)] mod tests;`，测试在 `chat_language_model/tests.rs`，对应上游 `mistral-chat-language-model.test.ts`。不用 `#[path]`；`stream/chat_tool_calls_tests.rs` 改成 `stream/chat_tool_calls/tests.rs`，对齐文档的引用一起改；已有的内联 `mod tests { … }` 搬到对应的 `tests.rs`。
2. 测试数据照上游的 `__fixtures__/`、`__snapshots__/` 放进厂商目录下的 `fixtures/`、`snapshots/`，大的 cassette 仍留在 crate 根目录按路径读取。
3. 走公开 API 的测试（provider → 模型 → mock fetch、cassette、conformance、e2e、数据丢失回归）留在 `tests/`，按厂商分子目录 `tests/it/<vendor>/`，跨厂商的放在 `tests/it/` 顶层，preset 的放在 `tests/it/preset/`，整个合成一个测试程序 `tests/it/main.rs`。
4. 现有 `tests/` 里 101 个按厂商的文件逐个分类：调内部函数、或能对上上游某个内部函数测试文件的，搬进对应的 `foo/tests.rs`；走公开 API 的搬进 `tests/it/<vendor>/`。
5. 测试共用的 mock fetch、回放 helper（现 `tests/common/`）移到 `src/test_support/`，`#[cfg(any(test, feature = "test-support"))]` 编译，正常构建不带，不进发布产物；`tests/it/` 经自引用的 dev 依赖开启 `test-support` 使用同一份（与 RFC-0044 §3.3 第 4 条相同）。`tests/cassettes/` 原地不动，`exclude` 不变。
6. 公共层的单元测试不引用厂商：`stream/chat_tool_calls_tests.rs` 里用到 4 个厂商的用例移到 `tests/it/`。
7. 合成一个测试程序前先去掉对进程环境的依赖：测试显式传设置，不靠 `set_var`；必须读环境变量的用例统一 `#[serial(env)]`。§2.8 列出的冲突、计数器和固定路径在搬移 PR 里修掉。
8. 依赖录制、回放的测试（`decision_recording_test.rs`、`runtime_decision_test.rs`、`decision_failure_e2e_test.rs` 等）先随厂商搬；RFC-0044 C6 之后 providers 不能依赖 devtools，届时移到 devtools 或 ops 的测试。

迁移只搬动测试，不改断言、不删测试。每个 PR 前后比较 `cargo test -p aimux-providers -- --list` 的测试数量（模块路径会变，比数量和函数名多重集，不比完整路径），cassette 文件数不变。

## 6. 实施顺序

每一步一个 PR，只动 `aimux-providers` 和 `aimux-provider-utils`，合入时 workspace 全绿；搬移类 PR 测试数量前后相等。

| 步骤 | 内容 | 前置 |
|---|---|---|
| R1 | provider-utils 补齐通用函数（§3.2），各厂商改用；删除死代码 | 无 |
| R2 | 解开七个环（§3.3 第 3 条），加 `internal` 模块和 CI 检查 | R1 |
| R3 | 托管变体：Anthropic AWS、Vertex Claude 改用 `AnthropicConfig`；两个 SigV4 层合并；Bedrock 能力表 | R2 |
| R4 | Vertex Gemini 改用 Google 模型 + `GoogleConfig`（行为修正，单独评审） | R2 |
| R5 | Config 钩子改成枚举（§3.5） | R3、R4 |
| R6 | 模态与搜索厂商去重（§3.6） | R1 |
| R7 | 目录、文件名、测试位置（§4、§5），`layout.toml`，按厂商家族分批：OpenAI 一族与兼容层、Anthropic、Google、Bedrock、其余有上游包的、没有上游包的与 preset | 可与 R1–R6 交错；同一厂商先去重再搬，避免搬两次 |

和其他工作的关系：

- #268 只改 providers 的 2 个文件，不冲突。
- RFC-0044：R1 里依赖 core 下载和重试的部分（生成结果下载、重试退避）等 RFC-0044 C3、C4；其余不依赖。RFC-0044 的 C2/C3 改 import 时对着本文整理好的目录一次改完。
- RFC-0045：测试位置和 `layout.toml` 的 `tests` 字段按本文；搬移测试时同时把出处注释改成它定义的标记。

## 7. 文档同步与登记差异

- RFC-0044 已把 provider 目录与测试部分移出（其 §2.4、§3 写“不在本文范围”），本文合入时 RFC-0044 的这几处改为引用本文。
- 对齐文档 §9.1 的顺序是“core 类型 → provider 工厂与请求层 → 厂商包”；本文的整理不依赖 core 拆分，可以先做，登记这一顺序差异。
- 对齐文档引用 `stream/chat_tool_calls_tests.rs` 的地方改成 `stream/chat_tool_calls/tests.rs`。
- `docs/contributing/adding-a-provider.md` 按 §4、§5 重写。
- ROADMAP 里“tracker 移入 aimux-provider-utils（#204）”一行已被 RFC-0042 T2 取代，同步删除。
- 登记：Vertex Gemini 的 ID 生成和流处理改为与 Google 一致（§3.4）；厂商之间内部用的 Config 钩子是枚举而不是回调（§3.5）。

## 8. 验收

1. 每个厂商一个目录；`src/` 顶层只剩 `lib.rs`、`catalogue.rs`、`default_providers.rs`、`preset/`、`decision/`、`shared/`、`stream/`、`test_support/` 和厂商目录。
2. `layout.toml` 覆盖锁定版本里每个上游源文件，CI 校验：表里的 Rust 文件存在、厂商目录里每个 `.rs` 都在表里（对应上游文件，或记为 `aimux`）、表里的上游文件在锁定版本的发布包里存在（CI 用 `scripts/fetch-aisdk.sh` 取包）。
3. `check_provider_boundaries.sh` 通过：没有厂商之间的环，跨厂商引用只经过 `internal`。
4. §2.3 列出的函数在 provider-utils 里各只有一份，厂商目录里没有同名实现。
5. `aimux-providers/tests/` 只剩 `it/`（按厂商分子目录，一个测试程序）、`cassettes/`、`fixtures/`、`replay_fixtures/`；单元测试都是 `foo/tests.rs`，没有 `#[path]`；测试数量与搬移前相等；cassette 文件数不变。
6. 产物体积不超过 RFC-0043 的当前值。

## 9. 开放问题

- **厂商目录名（待讨论）。** 本文的方案是保留现有名字（`bedrock/`、`vertex/` 等，§4.1 第 2 条），对应的上游包写在 `layout.toml`。另一种做法是改成上游包名（`amazon_bedrock/`、`google_vertex/`），名字完全一致，但公开模块路径会变。
- **Config 钩子的写法（待讨论）。** 本文的方案是厂商之间内部用的钩子改成封闭枚举，用户能填的兼容层回调照上游保留（§3.5）。另一种做法是全部保留回调，和上游写法一致，但配置不能打印、比较、写进录制，绑定也选不了。
- **扫描中发现的行为偏差**由单独的线程核实和修复，不放进搬移 PR。原列 6 处，其中 xAI 拒绝文本、Anthropic 显式 `outputFormat` 两处核对后与上游一致，不是偏差；剩下 4 处：HuggingFace 丢掉 `ToolChoice::None`、Vertex 上的 Grok 用量 total、preset 强制 `supports_structured_outputs`、Vertex Gemini 的 ID 规则（§3.4）。
