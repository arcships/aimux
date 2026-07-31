# aimux

> 统一 LLM 服务接入层 — 一套 API 接入 172+ 家 AI 服务商

## 这是什么

aimux 是一个 Rust 写的 LLM 服务接入统一层。把各家 AI 服务商的 HTTP API 收敛成一个 `dyn LanguageModel` 接口，上游谁都能用。

和 rig、langchain 不同，aimux 不做 agent loop、不做 RAG、不做编排——只专注服务接入统一化。

## 核心能力

- **172 个厂商模块**：11 个原生协议实现（OpenAI/Anthropic/Google/Bedrock/Vertex/Azure/Cohere/Mistral/xAI/DeepSeek/Anthropic-AWS）+ 145 个 OpenAI 兼容薄封装 + 15 个语音/图像/视频专用 + 1 个通用 Responses API 封装
- **统一接口**：`LanguageModel` trait（object-safe，支持 `Box<dyn>` 跨厂商互换）
- **多模态**：文本、流式、工具调用、嵌入、图像、语音、转写、视频、重排序、文件
- **配置描述结构**：`OpenAICompatProfile` 描述各家差异（top_k/tools/response_format/流式usage/请求体后处理），薄封装不丢差异
- **录播测试**：2654 个 cassette 回放，不依赖网络和密钥

## 架构

```
aimux/
├── aimux-core           # 核心抽象：LanguageModel / Provider / Message / StreamPart
├── aimux-providers      # 172 个厂商实现
├── aimux-stream         # SSE / NDJSON 流式解析
├── aimux-provider-utils # HTTP 工具：重试、退避、错误解析、API Key 加载
├── aimux-tools          # 工具调用：ToolSet、ToolExecutor
└── aimux-macros         # 过程宏：#[tool] 属性宏
```

## 快速开始

```rust
use aimux_core::prelude::*;
use aimux_providers::{OpenAIConfig, OpenAIProvider};

#[tokio::main]
async fn main() -> Result<(), AiMuxError> {
    let provider = OpenAIProvider::new(
        OpenAIConfig::new(std::env::var("OPENAI_API_KEY")?)
    );
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

## 流式输出

```rust
use futures::StreamExt;

let result = stream_text(
    &model,
    "Write a haiku about Rust.",
    GenerateTextOptions::default(),
).await?;

let mut stream = result.stream;
while let Some(part) = stream.next().await {
    match part? {
        StreamPart::TextDelta { delta, .. } => print!("{}", delta),
        StreamPart::Finish { .. } => println!("\n[done]"),
        _ => {}
    }
}
```

## 切换厂商

```rust
// OpenAI → DeepSeek，只改 provider
let provider = DeepSeekProvider::new(
    DeepSeekConfig::from_env()?
);
let model = provider.model("deepseek-chat");
// model 用法完全一样，都是 dyn LanguageModel
```

## 厂商覆盖

| 类型 | 数量 | 代表 |
|------|:---:|------|
| 原生协议 | 11 | OpenAI、Anthropic、Google、Bedrock、Vertex、Azure、Cohere、Mistral、xAI、DeepSeek |
| OpenAI 兼容 | 145 | Groq、Fireworks、Together、Perplexity、Ollama、OpenRouter、阿里通义、智谱、百度、腾讯、讯飞、月之暗面、硅基流动… |
| 语音/转写 | 7 | ElevenLabs、Deepgram、AssemblyAI、Cartesia… |
| 图像/视频 | 8 | Black Forest Labs、Replicate、Fal、KlingAI… |

完整清单见 [rfc/0004-provider-inventory.md](rfc/0004-provider-inventory.md)。

## 测试

```bash
cargo test -p aimux-providers --tests
```

测试不依赖网络和密钥，用 cassette 回放真实 API 响应。详见 [rfc/0003-test-cassette.md](rfc/0003-test-cassette.md)。

## 多语言绑定

aimux 提供 7 种语言绑定，共享同一个 Rust 核心：

| 绑定 | 路径 | 工具 | 目录 |
|------|------|------|------|
| **Node.js** | 原生 | napi-rs v3 | [bindings/node/](bindings/node/) |
| **Python** | 原生 | PyO3 + maturin | [bindings/python/](bindings/python/) |
| **Swift** | C ABI | Swift Package | [bindings/swift/](bindings/swift/) |
| **Kotlin** | C ABI | JNA | [bindings/kotlin/](bindings/kotlin/) |
| **Flutter** | C ABI | dart:ffi | [bindings/flutter/](bindings/flutter/) |
| **Go** | C ABI | cgo（静态链接，单 binary） | [bindings/go/](bindings/go/) |
| **C / C++** | C ABI | 直接链接 | [bindings/c/](bindings/c/) |

详见 [bindings/README.md](bindings/README.md) 和 [API 文档](docs/API.md)。

## 设计文档

| 文档 | 内容 |
|------|------|
| [docs/API.md](docs/API.md) | **API 文档**（全部模态 × 全部语言） |
| [rfc/0001-multilang-bindings.md](rfc/0001-multilang-bindings.md) | 多语言绑定方案（Node/Swift/Kotlin/Flutter/Python） |
| [rfc/0002-provider-improvements.md](rfc/0002-provider-improvements.md) | 配置描述结构与薄封装改进 |
| [rfc/0003-test-cassette.md](rfc/0003-test-cassette.md) | 录播测试方案 |
| [rfc/0004-provider-inventory.md](rfc/0004-provider-inventory.md) | 全网厂商清单与实现现状 |
| [rfc/0005-protocol-conversion.md](rfc/0005-protocol-conversion.md) | 协议转换与适配层设计 |
| [rfc/0006-provider-development.md](rfc/0006-provider-development.md) | Provider 最小准入、实现路径、核心契约、按需测试与验收规范 |
| [rfc/0008-multimodal-bindings.md](rfc/0008-multimodal-bindings.md) | 多模态绑定设计 |
| [rfc/0009-request-resilience.md](rfc/0009-request-resilience.md) | 请求优化 — 参考 catcher 设计（共享 Client / jitter / 超时） |
| [rfc/0010-perf-benchmark-vs-aisdk.md](rfc/0010-perf-benchmark-vs-aisdk.md) | 请求性能对比 — aimux vs Vercel AI SDK 基准方案（速度 / 结构化开销 / 并发三维度，同进程同 mock） |
| [rfc/0011-golang-bindings.md](rfc/0011-golang-bindings.md) | Go 绑定设计 — cgo 静态链接 + push callback→channel 流式，单 binary 7.5MB |

## License

MIT
