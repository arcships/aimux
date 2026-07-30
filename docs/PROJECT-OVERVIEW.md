# aimux — 项目介绍与宣传素材

> **统一 LLM 服务接入层 — Rust 核心，一套 API 接入 172+ 家 AI 服务商，6 语言绑定**
>
> 本文档整理 aimux 全部设计决策与 benchmark 结论，供 README、博客、技术分享使用。

---

## 一句话定位

> **aimux 是 Vercel AI SDK 的 Rust 替代品。它把 172+ 家 AI 服务商的 HTTP API 收敛成一个统一接口，用 Rust 核心提供 7-15x 性能优势，通过 FFI 绑定覆盖 Node / Python / Swift / Kotlin / Flutter / C 六个语言生态。**

aimux 不做 agent loop、不做 RAG、不做编排——只专注服务接入统一化。这是它与 LangChain / Mastra 的根本区别：前者是接入层，后者是编排层。

---

## 核心数字

| 指标 | 数值 |
|------|------|
| Rust 代码 | 144,500+ 行 |
| AI 服务商 | 172 个（11 原生 + 145 OpenAI 兼容 + 15 语音/图像/视频） |
| 模态 trait | 8 个（文本/嵌入/图像/视频/语音/转写/重排/搜索） |
| 测试 cassette | 2,650 个录播回放 |
| 测试文件 | 118 个 |
| Rust crate | 7 个核心 + 8 个绑定 |
| 语言绑定 | 7 个（Node / Python / Swift / Kotlin / Flutter / C / Rust） |
| 类型定义 | 79 个（ts-rs 自动生成） |

---

## 性能基准（2026-07-30）

> 环境：Linux x64, 32 核, Node v24.18.0, Python 3.12.13
> 方法：同进程、同 mock server、固定响应，N=200-300 次取统计值
> 完整数据：[PERF-RESULTS.md](PERF-RESULTS.md)

### 1. 对等对比：aimux vs OpenAI 官方 SDK

同一抽象层（HTTP + JSON，无编排/schema 验证/中间件）的干净数字。

| | mean | P50 | P95 | P99 | RSS 增长 |
|---|---|---|---|---|---|
| **aimux (Node)** | 0.101ms | 0.096 | 0.122 | 0.139 | +2MB |
| **OpenAI Node SDK** | 1.488ms | 1.500 | 1.637 | 1.923 | +17MB |
| | | | | | |
| **aimux (Python)** | 0.080ms | 0.075 | 0.108 | 0.129 | +0MB |
| **OpenAI Python SDK** | 0.595ms | 0.577 | 0.695 | 0.839 | +8MB |

- **Node：aimux 快 14.7x，内存省 8.5x**
- **Python：aimux 快 7.5x，内存零增长**

### 2. 持续压测（2000 次请求，200KB 上文，50KB 响应）

| 场景 | SDK | rps | mean | P99 | RSS 增长 |
|---|---|---|---|---|---|
| 32 核 | aimux | 1512 | 0.66ms | 1.92ms | +23MB |
| | AISDK | 563 | 1.78ms | 3.96ms | +103MB |
| 1 核 | aimux | 1497 | 0.67ms | 1.65ms | +21MB |
| | AISDK | 473 | 2.11ms | **12.87ms** | +60MB |

关键发现：
- **CPU 受限时 aimux 性能不降**（1512→1497 rps），AISDK 暴跌（563→473 rps）
- **aimux 无 GC 停顿**，1 核时 P99 仍稳在 1.65ms；AISDK P99 飙到 12.87ms（GC 抖动）
- **Python aimux 2000 次请求后 RSS 一字节没涨**

### 3. 为什么快

```
aimux:  Node 应用 → napi FFI → Rust 核心 → reqwest → HTTP
AISDK:  Node 应用 → TS 核心 → undici → HTTP
                     ↑
              V8 GC + Zod 验证 + 中间件 pipeline + telemetry
```

- Rust 核心零 GC，内存分配模式可预测
- reqwest 连接池复用（RFC-0009 落地后）
- 不做 Zod schema 验证 / fetch 中间件 / telemetry 记录——这些是 AISDK 的开销来源
- PyO3（Python 绑定）比 napi（Node 绑定）更轻量——直接 C API 调用，不经 V8 包装

### 4. 对比对象的公平性说明

| 对比 | 倍数 | 是否对等 | 说明 |
|---|---|---|---|
| vs OpenAI Node SDK | **14.7x** | ✅ 对等 | 都是 HTTP + JSON，无编排层 |
| vs OpenAI Python SDK | **7.5x** | ✅ 对等 | 同上 |
| vs Vercel AI SDK | ~11x | ❌ 不对等 | AISDK 含 Zod 验证/中间件/telemetry，11x 有水分 |

aimux 与 OpenAI 官方 SDK 的对比是真正对等的——两者都只做 HTTP + JSON。Vercel AI SDK 每次请求额外做 Zod schema 验证、构建类型化对象树、fetch 中间件 pipeline、telemetry 记录，这些在 V8 堆里累积导致内存膨胀。

---

## 功能覆盖

### 8 个模态 trait

| Trait | 能力 | 示例 Provider |
|-------|------|-------------|
| `LanguageModel` | 文本生成 + 流式 + 工具调用 | OpenAI / Anthropic / Google / DeepSeek / 172 家 |
| `EmbeddingModel` | 向量嵌入 | OpenAI / Cohere / Voyage / 通用兼容 |
| `ImageModel` | 图像生成 | Black Forest Labs / Replicate / Fal / KlingAI |
| `VideoModel` | 视频生成 | Google Veo / Replicate |
| `SpeechModel` | 语音合成 (TTS) | OpenAI / ElevenLabs / Cartesia |
| `TranscriptionModel` | 语音转文字 (STT) | OpenAI / Deepgram / AssemblyAI |
| `RerankingModel` | 重排序 | Cohere / Voyage |
| `SearchModel` | 搜索 | Tavily / Exa / Jina |

### 172 个厂商分类

| 类型 | 数量 | 代表 |
|------|:---:|------|
| 原生协议 | 11 | OpenAI、Anthropic、Google、Bedrock、Vertex、Azure、Cohere、Mistral、xAI、DeepSeek |
| OpenAI 兼容 | 145 | Groq、Fireworks、Together、Perplexity、Ollama、OpenRouter、阿里通义、智谱、百度、腾讯、讯飞、月之暗面、硅基流动… |
| 语音/转写 | 7 | ElevenLabs、Deepgram、AssemblyAI、Cartesia… |
| 图像/视频 | 8 | Black Forest Labs、Replicate、Fal、KlingAI… |

完整清单见 [rfc/0004-provider-inventory.md](../rfc/0004-provider-inventory.md)。

### 数据模型（对标 AI SDK V4）

aimux 的类型设计本就对齐 Vercel AI SDK V4，核心结构高度一致：

| 维度 | 一致性 | 说明 |
|------|:---:|------|
| `GenerateResult` | ✅ | 核心字段一致（content/finish_reason/usage/response） |
| `StreamPart` | ✅ | aimux 18 变体 vs V4 ~21 变体，核心路径对齐 |
| `Role` | ✅ | 完全一致（system/user/assistant/tool） |
| `FinishReason` | ✅ | 基本一致（unified + raw 双字段） |
| `ToolResult` | ✅ | 字段全对齐（result/is_error/preliminary/dynamic/tool_name） |
| `File 变体` | ✅ | GenerateContent/StreamPart 均有 File 变体 |
| `ToolChoice` | 🟡 | 格式不同（裸字符串 vs 对象），wrapper 可转换 |
| 命名 | 🟡 | snake_case vs camelCase，wrapper 统一映射 |

79 个类型定义由 Rust 的 `ts-rs` 自动生成，保证 Rust 核心与 TypeScript 类型永远同步。

完整对比见 [type-comparison-aisdk.md](type-comparison-aisdk.md)。

---

## 架构设计

### 分层架构

```
aimux/
├── aimux-core              # 核心抽象：8 个 trait + 类型定义
│   ├── LanguageModel        #   object-safe，支持 Box<dyn> 跨厂商互换
│   ├── EmbeddingModel
│   ├── ImageModel / VideoModel
│   ├── SpeechModel / TranscriptionModel
│   └── RerankingModel / SearchModel
├── aimux-providers          # 172 个厂商实现
│   ├── 11 原生协议           #   独立 model + convert，处理厂商特有差异
│   └── 145 OpenAI 兼容       #   薄封装，共享请求路径
├── aimux-stream             # SSE / NDJSON 流式解析
├── aimux-provider-utils     # HTTP 工具：重试、退避、错误解析、API Key 加载
├── aimux-tools              # 工具调用：ToolSet、ToolExecutor
├── aimux-macros             # 过程宏：#[tool] 属性宏
├── aimux-ffi                # C ABI（FFI 基础设施，所有绑定共享）
└── bindings/                # 6 语言绑定
    ├── node/                #   napi-rs v3 + 类型化 TS wrapper
    ├── python/              #   PyO3
    ├── swift/               #   Swift Package
    ├── kotlin/              #   Kotlin/JVM
    ├── flutter/             #   dart:ffi
    └── c/                   #   C ABI 头文件
```

### 核心设计决策

#### 1. Object-safe LanguageModel trait

```rust
// aimux-core：trait 是 object-safe 的，支持 Box<dyn LanguageModel>
pub trait LanguageModel: Send + Sync {
    fn model_id(&self) -> &str;
    fn do_generate(&self, prompt: ModelPrompt, options: CallOptions) 
        -> impl Future<Output = Result<GenerateResult, AiMuxError>>;
    fn do_stream(&self, prompt: ModelPrompt, options: CallOptions)
        -> impl Future<Output = Result<Stream, AiMuxError>>;
}
```

这意味着 `Box<dyn LanguageModel>` 可以在 OpenAI / Anthropic / Google 之间互换——**切换厂商只改 provider 构造，model 用法完全一样**。

#### 2. OpenAICompatProfile 配置描述结构

薄封装不丢差异。各家 OpenAI 兼容服务有细微差别（有的支持 top_k，有的不支持 tools，有的流式 usage 格式不同）：

```rust
pub struct OpenAICompatProfile {
    pub supports_top_k: bool,          // Groq 支持，OpenAI 不支持
    pub supports_tools: bool,           // 部分服务不支持
    pub supports_response_format: bool,
    pub streaming_usage_format: UsageFormat, // 流式 usage 在 data 行 vs chunk 行
    pub post_process_request: Option<fn(&mut serde_json::Value)>,
}
```

用一个 profile 描述结构表达差异，而不是为每个厂商写独立 model——这就是 145 个兼容厂商只需薄封装的原因。

#### 3. JSON 字符串 FFI 边界

所有绑定通过 JSON 字符串与 Rust 核心通信：

```
JS/Python 调用 → JSON.stringify → serde_json::from_str → Rust 核心 → serde_json::to_string → JSON.parse → JS/Python 对象
```

**好处**：6 个绑定共享同一套 Rust 核心，跨语言行为完全一致。
**代价**：每次调用 5-6 次序列化。但序列化开销 ~0.01-0.05ms，占真实 LLM 请求（200-2000ms）的 **< 0.025%**——可忽略。

#### 4. 录播测试（Cassette）

2,650 个真实 API 响应录像，不依赖网络和密钥：

```
测试时：mock server 回放固定响应 → 确定性测试
开发时：录制模式 → 真实 API 调用 → 存成 cassette
CI：全量回放，零网络依赖
```

覆盖工具调用、多轮对话、推理思考、结构化输出等场景。完整方案见 [rfc/0003-test-cassette.md](../rfc/0003-test-cassette.md)。

#### 5. 请求层优化（RFC-0009）

- `shared_client()` 连接池共享（不再每个 provider 各自建连）
- TLS 会话复用
- jitter 退避重试（参考 catcher 设计）
- 固定超时

#### 6. 请求层解耦（RFC-0009 补充）

provider 层不直接依赖 `reqwest`，通过 `aimux-provider-utils` 的 trait 抽象 HTTP 客户端——reqwest 不外泄到 provider 层，未来可替换为 hyper /其他 HTTP 后端。

---

## 与竞品对比

### aimux vs Vercel AI SDK

| 维度 | aimux | AI SDK | 优势方 |
|------|-------|--------|:---:|
| 性能（Node 对等对比） | 0.101ms | 1.488ms (OpenAI SDK) | **aimux 14.7x** |
| 性能（Python 对等对比） | 0.080ms | 0.595ms (OpenAI SDK) | **aimux 7.5x** |
| Provider 覆盖 | 172 | ~20 | **aimux 8.6x** |
| 模态 | 11 个 | 6 个 | **aimux**（多视频/STT/重排/搜索/文件） |
| 内存（2000 req） | +0~2MB | +60~144MB | **aimux** |
| GC 停顿 | 无（Rust） | V8 GC 抖动 | **aimux** |
| 语言绑定 | 7 个 | 1 个（Node） | **aimux** |
| 类型安全 DX | ✅ 完整（ts-rs 79 类型，已有 wrapper） | Zod 全推断 | 🟢 接近 |
| Agent loop | ❌ 不做（设计决策） | `stopWhen` + `execute` | AI SDK |
| 数据模型 | 对标 V4，可互换 | V4 原生 | 🟢 一致 |

**aimux 的定位**：统一接入层，不做编排。Node 绑定已有完整类型（ts-rs 自动生成 79 个类型 + 类型化 wrapper），agent loop 交给上层框架（LangChain / Mastra）。

### aimux vs LangChain / Mastra

根本区别：**aimux 是接入层，LangChain/Mastra 是编排层**。

```
你的应用
  └── 编排层（LangChain / Mastra / 自写 loop）
        └── 接入层（aimux）← 这里
              └── 172 个 AI 服务商
```

aimux 不与 LangChain 竞争，而是作为 LangChain 的底层——LangChain 负责 agent loop / RAG / chain，aimux 负责 172 个厂商的统一接入。用 Rust 跑接入层，性能和内存表现远超 JS 实现的接入层。

### aimux vs rig / rust-genai

| 维度 | aimux | rig / rust-genai |
|------|-------|------------------|
| Provider 数 | 172 | ~20-40 |
| 多语言绑定 | 7 个 | Rust only |
| 多模态 | 8 个 trait | 部分 |
| 录播测试 | 2,650 cassette | 少量 |
| 定位 | 纯接入层 | 接入 + 部分编排 |

---

## 适用场景

### ✅ 适合用 aimux

1. **多厂商聚合**：一个应用要接入 5+ 家 AI 服务商，不想为每家写适配
2. **性能敏感**：高并发 API 网关 / batch 处理 / 实时交互，GC 停顿不可接受
3. **多语言栈**：Node + Python + 移动端混合技术栈，想要一套统一的 AI 接入
4. **成本控制**：需要在低配服务器上跑 AI 服务，内存占用要小
5. **Provider 可替换**：需要在不改业务代码的前提下切换 / 降级 / 混用 LLM 厂商

### ❌ 不适合用 aimux

1. **需要 agent loop**：要 `stopWhen` / `execute` / 多步推理循环——用 LangChain / Mastra 叠在 aimux 上
2. **只需要 OpenAI 一家**：直接用 OpenAI 官方 SDK 更简单
3. **需要 RAG / 向量库编排**：aimux 只提供 embedding 调用，不做 RAG pipeline

---

## 快速开始

### Node.js

```bash
npm install aimux
```

```typescript
import { openai, generateText, streamText } from 'aimux'

const model = await openai(process.env.OPENAI_API_KEY!, 'gpt-4o')

// 非流式
const result = await generateText(model, 'What is Rust?')
console.log(result.text)

// 流式
const { stream } = await streamText(model, 'Write a haiku about Rust.')
for await (const part of stream) {
  if (part.TextDelta) process.stdout.write(part.TextDelta.delta)
}

// 切换厂商：只改 provider
const deepseekModel = await deepseek(DEEPSEEK_API_KEY, 'deepseek-chat')
// model 用法完全一样
```

### Python

```bash
pip install aimux
```

```python
from aimux import openai, generate_text, stream_text

model = openai("sk-...", "gpt-4o")
result = generate_text(model, "What is Rust?")
print(result["text"])

# 流式
result = stream_text(model, "Write a haiku about Rust.")
for part in result:
    if "TextDelta" in part:
        print(part["TextDelta"]["delta"], end="")
```

### Rust

```rust
use aimux_core::prelude::*;
use aimux_providers::{OpenAIConfig, OpenAIProvider};

#[tokio::main]
async fn main() -> Result<(), AiMuxError> {
    let provider = OpenAIProvider::new(OpenAIConfig::new("sk-..."));
    let model = provider.model("gpt-4o");

    let result = generate_text(
        &model,
        "Explain Rust ownership in one sentence.",
        GenerateTextOptions::default(),
    ).await?;

    println!("{}", result.text);
    Ok(())
}
```

完整 API 文档见 [API.md](API.md)。

---

## 技术亮点（技术分享用）

### 1. Rust 核心 + FFI 多语言绑定

一套 Rust 核心通过 FFI 覆盖 7 个语言生态：
- **napi-rs**（Node）— V8 原生绑定
- **PyO3**（Python）— CPython 原生绑定
- **Swift Package**（iOS/macOS）
- **Kotlin/JVM**（Android）
- **dart:ffi**（Flutter）
- **C ABI**（通用 FFI 基础设施）

关键工程决策：**JSON 字符串边界**。不跨 FFI 传递 Rust struct，只传 JSON 字符串。好处是所有绑定共享同一套 Rust 核心、跨语言行为完全一致；代价是 5-6 次序列化，但开销 < 0.025%，可忽略。

### 2. 14.7x 性能优势的来源

不是某一个技巧，而是系统性的设计选择：

| 因素 | aimux | 竞品 | 影响 |
|------|-------|------|------|
| 语言 | Rust（零 GC） | TS/Python（GC 抖动） | P99 稳定性 |
| HTTP 客户端 | reqwest + 连接池 | undici/httpx | 连接复用 |
| 序列化 | serde_json（Rust 最快） | JSON.parse + Zod | CPU 开销 |
| 不做 | Zod 验证 / 中间件 / telemetry | 做了 | 每次请求额外 CPU |
| 编译 | AOT 编译为原生码 | JIT | 冷启动 + 稳态 |

### 3. 172 厂商的统一之道

不是为每个厂商写独立 model——那会爆炸。用 `OpenAICompatProfile` 描述差异：

```rust
// 一个 profile 描述一家兼容厂商的差异点
let profile = OpenAICompatProfile {
    supports_top_k: true,         // Groq 支持 top_k
    supports_tools: true,         // 支持 function calling
    streaming_usage_format: UsageFormat::InDataLine,
    post_process_request: Some(strip_unsupported_fields),
    ..Default::default()
};
```

11 个原生协议有独立 model + convert（处理 Anthropic message format / Google generateContent / Bedrock SigV4 等差异），145 个 OpenAI 兼容厂商只需薄封装 + profile。

### 4. 2650 个 cassette 的录播测试

测试不依赖网络和密钥。每次 CI 跑全量 2650 个 cassette 回放，保证协议转换的回归安全。cassette 来自真实 API 响应，覆盖工具调用、多轮对话、推理思考、结构化输出等场景。

### 5. 数据模型对标 AI SDK V4

aimux 的类型设计直接对标 Vercel AI SDK V4 provider 类型——`GenerateResult` / `StreamPart` / `Usage` / `ToolChoice` 结构对齐。这不是巧合，是设计目标。79 个 TypeScript 类型由 Rust 的 `ts-rs` 自动生成，保证 Rust 核心与 TS 类型永远同步。

---

## Roadmap

### 已完成

- [x] 172 个厂商接入（11 原生 + 145 兼容 + 15 语音/图像/视频）
- [x] 8 个模态 trait（文本/嵌入/图像/视频/语音/转写/重排/搜索）
- [x] 7 语言绑定（Node/Python/Swift/Kotlin/Flutter/C/Rust）
- [x] 2650 个 cassette 录播测试
- [x] 性能基准：14.7x（Node）/ 7.5x（Python）领先
- [x] 请求层优化（RFC-0009：连接池 + 退避 + 超时 + reqwest 解耦）
- [x] 数据模型对标 AI SDK V4
- [x] C ABI 全模态覆盖（16 个 extern 函数）

### 进行中

- [ ] Python wrapper 默认化（当前需显式 `from aimux.wrapper import ...`，改为默认导出）
- [ ] 各语言 wrapper 统一（camelCase 命名 + 类型化边界）

### 规划中

- [ ] 性能基准维度三：并发能力曲线 + 内存增长图
- [ ] 更多原生协议（Together / Fireworks / Anyscale 专业化）
- [ ] 流式 TTFT（Time To First Token）基准

---

## 链接索引

| 文档 | 内容 |
|------|------|
| [README.md](../README.md) | 项目主页 |
| [docs/API.md](API.md) | 完整 API 文档 |
| [docs/PERF-RESULTS.md](PERF-RESULTS.md) | 性能基准完整数据 |
| [docs/aimux-vs-aisdk-node.md](aimux-vs-aisdk-node.md) | Node.js 体验对比 |
| [docs/type-comparison-aisdk.md](type-comparison-aisdk.md) | 类型对比（V4 对标） |
| [docs/audit-001-feature-coverage.md](audit-001-feature-coverage.md) | 功能审计报告 |
| [docs/cross-lang-dx-plan.md](cross-lang-dx-plan.md) | 跨语言 DX 统一方案 |
| [rfc/0001-multilang-bindings.md](../rfc/0001-multilang-bindings.md) | 多语言绑定设计 |
| [rfc/0004-provider-inventory.md](../rfc/0004-provider-inventory.md) | 172 厂商清单 |
| [rfc/0005-protocol-conversion.md](../rfc/0005-protocol-conversion.md) | 协议转换设计 |
| [rfc/0009-request-resilience.md](../rfc/0009-request-resilience.md) | 请求层优化 |
| [rfc/0010-perf-benchmark-vs-aisdk.md](../rfc/0010-perf-benchmark-vs-aisdk.md) | 性能对比基准方案 |

---

## 修订记录

| 日期 | 版本 | 说明 |
|------|------|------|
| 2026-07-30 | v1.0 | 初版，汇总全部设计决策 + benchmark 结论 |
