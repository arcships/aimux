# aimux API 文档

> 统一 LLM 服务接入层 — 一套 API 接入 172+ 家 AI 服务商

## 目录

- [快速开始](#快速开始)
- [文本生成](#文本生成)
- [流式生成](#流式生成)
- [向量嵌入](#向量嵌入)
- [语音合成 (TTS)](#语音合成-tts)
- [语音转文字 (STT)](#语音转文字-stt)
- [图像生成](#图像生成)
- [视频生成](#视频生成)
- [重排序](#重排序)
- [搜索](#搜索)
- [文件上传](#文件上传)
- [Provider 工厂函数](#provider-工厂函数)
- [工具调用](#工具调用)
- [Rust API](#rust-api)
- [C ABI (aimux-ffi)](#c-abi-aimux-ffi)

---

## 快速开始

### Node.js

```bash
npm install aimux
```

```typescript
import { openai, generateText } from 'aimux'

const model = await openai(process.env.OPENAI_API_KEY!, 'gpt-4o')
const result = await generateText(model, 'What is Rust?')
console.log(result.text)
```

### Python

```bash
pip install aimux
```

```python
from aimux import openai, generate_text

model = openai("sk-...", "gpt-4o")
result = generate_text(model, "What is Rust?")
print(result["text"])
```

### Rust

```rust
use aimux_core::prelude::*;
use aimux_providers::{OpenAIConfig, OpenAIProvider};

#[tokio::main]
async fn main() -> Result<(), AiMuxError> {
    let provider = OpenAIProvider::new(OpenAIConfig::new("sk-..."));
    let model = provider.model("gpt-4o");
    let result = generate_text(&model, "What is Rust?", GenerateTextOptions::default()).await?;
    println!("{}", result.text);
    Ok(())
}
```

---

## 文本生成

非流式文本生成，返回完整结果。

### Node.js

```typescript
const { openai, generateText } = require('aimux')

const model = await openai('sk-...', 'gpt-4o', 'https://api.openai.com/v1')
const result = await generateText(model, 'Explain Rust ownership.', {
  max_output_tokens: 100,
  temperature: 0.7,
})

console.log(result.text)           // 生成文本
console.log(result.usage)          // token 用量
console.log(result.finish_reason)  // 停止原因
console.log(result.tool_calls)     // 工具调用（如有）
```

### Python

```python
from aimux import openai, generate_text

model = openai("sk-...", "gpt-4o")
result = generate_text(model, "Explain Rust ownership.", {
    "max_output_tokens": 100,
    "temperature": 0.7,
})

print(result["text"])
print(result["usage"])
print(result["finish_reason"])
```

### Rust

```rust
let result = generate_text(
    &model,
    "Explain Rust ownership.",
    GenerateTextOptions {
        max_output_tokens: Some(100),
        temperature: Some(0.7),
        ..Default::default()
    },
).await?;
```

### 参数

| 参数 | 类型 | 说明 |
|------|------|------|
| `prompt` | `string` / `Message[]` | 提示词或消息数组 |
| `max_output_tokens` | `number?` | 最大生成 token 数 |
| `temperature` | `number?` | 采样温度 |
| `top_p` | `number?` | 核采样 |
| `stop_sequences` | `string[]?` | 停止序列 |
| `tools` | `Tool[]?` | 可用工具列表 |
| `tool_choice` | `ToolChoice?` | 工具选择策略 |
| `instructions` | `string?` | 系统指令 |
| `reasoning` | `ReasoningEffort?` | 推理强度 |

### 返回值

```typescript
interface GenerateTextResult {
  text: string                  // 生成的文本
  tool_calls: ToolCall[]        // 工具调用列表
  finish_reason: FinishReason    // 停止原因
  usage: Usage                  // token 用量
  warnings: Warning[]           // 警告
  raw: GenerateResult           // 原始 provider 结果
}
```

---

## 流式生成

流式返回生成内容，逐块输出。

### Node.js

```typescript
const { openai, streamText } = require('aimux')

const model = await openai('sk-...', 'gpt-4o')
for await (const part of streamText(model, 'Write a haiku about Rust.')) {
  if (part.TextDelta) {
    process.stdout.write(part.TextDelta.delta)
  }
  if (part.Finish) {
    console.log('\n[done]')
  }
}
```

### Python

```python
from aimux import openai, stream_text

model = openai("sk-...", "gpt-4o")
for part in stream_text(model, "Write a haiku about Rust."):
    if "TextDelta" in part:
        print(part["TextDelta"]["delta"], end="")
    if "Finish" in part:
        print("\n[done]")
```

### Rust

```rust
use futures::StreamExt;

let result = stream_text(&model, "Write a haiku.", GenerateTextOptions::default()).await?;
let mut stream = result.stream;
while let Some(part) = stream.next().await {
    match part? {
        StreamPart::TextDelta { delta, .. } => print!("{}", delta),
        StreamPart::Finish { .. } => println!("\n[done]"),
        _ => {}
    }
}
```

### StreamPart 类型

| 变体 | 说明 |
|------|------|
| `StreamStart` | 流开始（携带 warnings） |
| `TextStart` / `TextDelta` / `TextEnd` | 文本段 |
| `ToolInputStart` / `ToolInputDelta` / `ToolInputEnd` | 工具调用输入流 |
| `ToolCall` | 完整工具调用 |
| `ReasoningStart` / `ReasoningDelta` / `ReasoningEnd` | 推理段 |
| `Finish` | 流结束（携带 usage + finish_reason） |
| `Error` | 流错误 |

---

## 向量嵌入

将文本转为向量表示。

### Node.js

```typescript
const { openaiEmbedding } = require('aimux')

const embedder = await openaiEmbedding('sk-...', 'text-embedding-3-small')
const resultJson = await embedder.embed(JSON.stringify(['hello', 'world']))
const result = JSON.parse(resultJson)

console.log(result.embeddings.length)  // 2
console.log(result.embeddings[0].length)  // 1536（维度取决于模型）
console.log(result.usage.tokens)  // 输入 token 数
```

### Python

```python
from aimux import openai_embedding

embedder = openai_embedding("sk-...", "text-embedding-3-small")
# embed() 接收 JSON 字符串，返回 JSON 字符串
result = json.loads(embedder.embed(json.dumps(["hello", "world"])))
print(len(result["embeddings"]))      # 2
print(len(result["embeddings"][0]))   # 1536
```

### Rust

```rust
use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};

let model = provider.embedding_model("text-embedding-3-small");
let opts = EmbeddingCallOptions::new("hello");
let result = model.do_embed(&opts).await?;
// result.embeddings[0] 是 Vec<f32>
```

### 支持的 Provider

| 工厂函数 | Provider | 代表模型 |
|---------|---------|---------|
| `openaiEmbedding` | OpenAI | text-embedding-3-small/large |
| `cohereEmbedding` | Cohere | embed-english-v3.0 |
| `googleEmbedding` | Google | gemini-embedding-001 |

---

## 语音合成 (TTS)

将文本转为语音音频。

### Node.js

```typescript
const { openaiSpeech } = require('aimux')
const fs = require('fs')

const speaker = await openaiSpeech('sk-...', 'tts-1')
const resultJson = await speaker.generate(JSON.stringify({
  text: 'Hello world!',
  voice: 'alloy',
  output_format: 'mp3',
}))
const result = JSON.parse(resultJson)

// 音频在 result.audio 中（base64 或 binary）
if (result.audio.Base64) {
  fs.writeFileSync('out.mp3', Buffer.from(result.audio.Base64, 'base64'))
}
```

### Python

```python
from aimux import openai_speech
import json, base64

speaker = openai_speech("sk-...", "tts-1")
result = json.loads(speaker.generate(json.dumps({
    "text": "Hello world!",
    "voice": "alloy",
    "output_format": "mp3",
})))

if "Base64" in result["audio"]:
    audio_bytes = base64.b64decode(result["audio"]["Base64"])
    with open("out.mp3", "wb") as f:
        f.write(audio_bytes)
```

### Rust

```rust
use aimux_core::speech_model::{SpeechCallOptions, SpeechModel};

let model = provider.speech("tts-1");
let opts = SpeechCallOptions::new("Hello world!");
let result = model.do_generate(&opts).await?;
// result.audio 是 AudioData::Base64(String) 或 AudioData::Binary(Vec<u8>)
```

### 支持的 Provider

| 工厂函数 | Provider | 代表模型 |
|---------|---------|---------|
| `openaiSpeech` | OpenAI | tts-1, tts-1-hd |

---

## 语音转文字 (STT)

将音频转为文字（非流式）。

### Node.js

```typescript
const { openaiTranscription } = require('aimux')
const fs = require('fs')

const transcriber = await openaiTranscription('sk-...', 'whisper-1')
const audioBase64 = fs.readFileSync('audio.mp3').toString('base64')
const resultJson = await transcriber.generate(audioBase64, 'audio/mp3')
const result = JSON.parse(resultJson)

console.log(result.text)       // 转录文本
console.log(result.segments)   // 带时间戳的分段
console.log(result.language)   // 检测到的语言
```

### Python

```python
from aimux import openai_transcription
import base64, json

transcriber = openai_transcription("sk-...", "whisper-1")
audio_b64 = base64.b64encode(open("audio.mp3", "rb").read()).decode()
result = json.loads(transcriber.generate(audio_b64, "audio/mp3"))

print(result["text"])
print(result["segments"])
```

### Rust

```rust
use aimux_core::transcription_model::{AudioInput, TranscriptionCallOptions, TranscriptionModel};

let model = provider.transcription("whisper-1");
let opts = TranscriptionCallOptions::new(
    AudioInput::Base64(audio_base64),
    "audio/mp3",
);
let result = model.do_generate(&opts).await?;
// result.text, result.segments, result.language
```

---

## 图像生成

### Node.js

```typescript
const { openaiImage } = require('aimux')
const fs = require('fs')

const imager = await openaiImage('sk-...', 'dall-e-3')
const resultJson = await imager.generate(JSON.stringify({
  prompt: 'A cute baby sea otter',
  n: 1,
}))
const result = JSON.parse(resultJson)

if (result.images.Base64) {
  fs.writeFileSync('out.png', Buffer.from(result.images.Base64[0], 'base64'))
}
```

### Rust

```rust
use aimux_core::image_model::{ImageCallOptions, ImageModel};

let model = provider.image("dall-e-3");
let opts = ImageCallOptions { prompt: Some("A cute sea otter".into()), n: 1, .. };
let result = model.do_generate(&opts).await?;
// result.images 是 ImageOutputs::Base64(Vec<String>) 或 Binary(Vec<Vec<u8>>)
```

### 支持的 Provider

| 工厂函数 | Provider | 代表模型 |
|---------|---------|---------|
| `openaiImage` | OpenAI | dall-e-3 |
| `googleImage` | Google | gemini-2.5-flash-image |

---

## 视频生成

视频生成通常返回 URL（非二进制）。

### Node.js

```typescript
const { googleVideo } = require('aimux')

const videor = await googleVideo('sk-...', 'veo-3.0')
const resultJson = await videor.generate(JSON.stringify({
  prompt: 'A cat playing piano',
  n: 1,
}))
const result = JSON.parse(resultJson)

// result.videos 通常是 { Url: { url, media_type } }
if (result.videos[0].Url) {
  console.log('Video URL:', result.videos[0].Url.url)
}
```

### Rust

```rust
use aimux_core::video_model::{VideoCallOptions, VideoModel};

let model = provider.video("veo-3.0");
let opts = VideoCallOptions { prompt: Some("A cat".into()), n: 1, .. };
let result = model.do_generate(&opts).await?;
// result.videos[0] 是 VideoData::Url { url, media_type }
```

---

## 重排序

对文档列表按相关性重新排序。

### Node.js

```typescript
const { cohereReranking } = require('aimux')

const reranker = await cohereReranking('sk-...', 'rerank-v3.0')
const resultJson = await reranker.rerank(
  'What is Rust?',
  JSON.stringify([
    { text: 'Rust is a systems programming language.' },
    { text: 'Rust is a chemical element.' },
  ]),
)
const result = JSON.parse(resultJson)

// result.ranks 按相关性排序
result.ranks.forEach(r => console.log(r.index, r.score))
```

### Rust

```rust
use aimux_core::reranking_model::{RerankingCallOptions, RerankingDocuments, RerankingModel};

let model = provider.reranking_model("rerank-v3.0");
let opts = RerankingCallOptions::new("What is Rust?", docs);
let result = model.do_rerank(&opts).await?;
// result.ranks 按 score 排序
```

---

## 搜索

调用搜索 provider 获取结果。

### Rust

```rust
use aimux_core::search_model::{SearchCallOptions, SearchModel};

let model = provider.search_model("tavily-search");
let opts = SearchCallOptions::new("What is Rust?");
let result = model.do_search(&opts).await?;
// result.results 是 Vec<SearchResultItem>
```

---

## 文件上传

上传文件到 provider，返回文件 ID。

### Node.js

```typescript
const { openaiFiles } = require('aimux')
const fs = require('fs')

const files = await openaiFiles('sk-...')
const fileBase64 = fs.readFileSync('doc.pdf').toString('base64')
const resultJson = await files.uploadFile(fileBase64, 'application/pdf')
const result = JSON.parse(resultJson)

console.log(result.provider_reference)  // { openai: 'file-xxx' }
```

### Rust

```rust
use aimux_core::files_model::{Files, UploadFileCallOptions, UploadFileData};
use aimux_core::shared::FileBytes;

let files = provider.files();
let opts = UploadFileCallOptions::new(
    UploadFileData::Data { data: FileBytes::Base64(file_b64) },
    "application/pdf",
);
let result = files.upload_file(opts).await?;
// result.provider_reference 是 HashMap<String, String>
```

---

## Provider 工厂函数

### 文本生成

| 函数 | Provider | 示例 modelId |
|---------|---------|-------------|
| `openai(apiKey, modelId, baseUrl?)` | OpenAI | gpt-4o |
| `anthropic(apiKey, modelId, baseUrl?)` | Anthropic | claude-3-5-sonnet-20241022 |
| `deepseek(apiKey, modelId, baseUrl?)` | DeepSeek | deepseek-chat |

### 向量嵌入

| 函数 | Provider | 示例 modelId |
|---------|---------|-------------|
| `openaiEmbedding(apiKey, modelId, baseUrl?)` | OpenAI | text-embedding-3-small |
| `cohereEmbedding(apiKey, modelId, baseUrl?)` | Cohere | embed-english-v3.0 |
| `googleEmbedding(apiKey, modelId, baseUrl?)` | Google | gemini-embedding-001 |

### 语音合成

| 函数 | Provider | 示例 modelId |
|---------|---------|-------------|
| `openaiSpeech(apiKey, modelId, baseUrl?)` | OpenAI | tts-1 |

### 语音转文字

| 函数 | Provider | 示例 modelId |
|---------|---------|-------------|
| `openaiTranscription(apiKey, modelId, baseUrl?)` | OpenAI | whisper-1 |

### 图像生成

| 函数 | Provider | 示例 modelId |
|---------|---------|-------------|
| `openaiImage(apiKey, modelId, baseUrl?)` | OpenAI | dall-e-3 |
| `googleImage(apiKey, modelId, baseUrl?)` | Google | gemini-2.5-flash-image |

### 视频生成

| 函数 | Provider | 示例 modelId |
|---------|---------|-------------|
| `googleVideo(apiKey, modelId, baseUrl?)` | Google | veo-3.0 |

### 重排序

| 函数 | Provider | 示例 modelId |
|---------|---------|-------------|
| `cohereReranking(apiKey, modelId, baseUrl?)` | Cohere | rerank-v3.0 |

### 文件上传

| 函数 | Provider |
|---------|---------|
| `openaiFiles(apiKey, baseUrl?)` | OpenAI |

> 所有工厂函数的 `baseUrl?` 参数可选，默认使用各 provider 的官方 API 地址。测试时传本地 mock server URL。

---

## 工具调用

工具定义是语言无关的数据描述（JSON Schema），不需要宏。

### 定义工具

```typescript
// Node.js — 直接构造数据对象
const tools = [{
  type: 'function',
  name: 'get_weather',
  description: 'Get current weather',
  input_schema: {
    type: 'object',
    properties: {
      location: { type: 'string', description: 'City name' }
    },
    required: ['location']
  }
}]

const result = await generateText(model, "What's the weather in Tokyo?", { tools })
if (result.tool_calls.length > 0) {
  const call = result.tool_calls[0]
  console.log(call.tool_name)     // get_weather
  console.log(call.input)         // { location: "Tokyo" }
}
```

```rust
// Rust — 使用 #[tool] 宏（语法糖）或直接构造 FunctionTool
use aimux_core::tool::FunctionTool;
use serde_json::json;

let tool = FunctionTool::new("get_weather", json!({
    "type": "object",
    "properties": {
        "location": { "type": "string" }
    },
    "required": ["location"]
}));
```

### 工具选择策略

```typescript
const opts = {
  tools,
  tool_choice: 'auto'        // 'auto' | 'none' | 'required' | { type: 'tool', toolName: 'get_weather' }
}
```

---

## Rust API

Rust 核心提供 8 个 trait，各 provider 按需实现：

| Trait | 方法 | 语义 |
|-------|------|------|
| `LanguageModel` | `do_generate`, `do_stream` | 文本生成 |
| `EmbeddingModel` | `do_embed` | 向量嵌入 |
| `SpeechModel` | `do_generate` | 语音合成 |
| `TranscriptionModel` | `do_generate`, `do_stream` | 语音转文字 |
| `ImageModel` | `do_generate` | 图像生成 |
| `RerankingModel` | `do_rerank` | 重排序 |
| `VideoModel` | `do_generate` | 视频生成 |
| `SearchModel` | `do_search` | 搜索 |
| `Files` | `upload_file` | 文件上传 |

用户面 API 是 `generate_text()` / `stream_text()` 自由函数，内部调用 trait 方法。

---

## C ABI (aimux-ffi)

C ABI 边界为 Swift / Kotlin / Flutter / C++ 提供 FFI 接口。所有函数通过 JSON 字符串通信。

### 函数列表

| 函数 | 说明 |
|------|------|
| `aimux_openai_new(api_key, model_id)` | 创建 OpenAI 语言模型 |
| `aimux_anthropic_new(api_key, model_id)` | 创建 Anthropic 语言模型 |
| `aimux_generate_text(handle, prompt_json, opts_json)` | 非流式生成（返回 JSON 字符串） |
| `aimux_stream_text(handle, prompt_json, opts_json, on_part, on_done, on_error)` | 流式生成（push 回调） |
| `aimux_openai_embedding_new(api_key, model_id)` | 创建 embedding 模型 |
| `aimux_embed(handle, values_json, opts_json)` | 生成向量嵌入 |
| `aimux_openai_speech_new(api_key, model_id)` | 创建 TTS 模型 |
| `aimux_speech_generate(handle, opts_json)` | 生成语音 |
| `aimux_openai_image_new(api_key, model_id)` | 创建图像模型 |
| `aimux_image_generate(handle, opts_json)` | 生成图像 |
| `aimux_openai_transcription_new(api_key, model_id)` | 创建 STT 模型 |
| `aimux_transcription_generate(handle, audio_base64, media_type, opts_json)` | 转录音频 |
| `aimux_openai_files_new(api_key)` | 创建文件管理器 |
| `aimux_file_upload(handle, data_base64, media_type, opts_json)` | 上传文件 |
| `aimux_drop_handle(handle)` | 释放模型句柄 |
| `aimux_free_string(ptr)` | 释放返回的字符串 |

### 内存管理

- `aimux_generate_text` 等返回 `char*`，调用方必须用 `aimux_free_string` 释放
- `aimux_stream_text` 的回调收到的 `const char*` 仅在回调期间有效，回调内必须同步拷贝
- `aimux_drop_handle` 释放模型句柄（0 是 no-op）

### 头文件

`aimux-ffi/aimux-ffi.h` — 完整 C 头文件，C++ 包裹 `extern "C"` 即可直接使用。

---

## 设计文档

| 文档 | 内容 |
|------|------|
| [RFC-0001](rfc/0001-multilang-bindings.md) | 多语言绑定方案 |
| [RFC-0003](rfc/0003-test-cassette.md) | 录播测试方案 |
| [RFC-0008](rfc/0008-multimodal-bindings.md) | 多模态绑定设计 |

## License

MIT
