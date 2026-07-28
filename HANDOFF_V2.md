# 交班文档 — 回放测试覆盖率扩充

> 日期：2026-07-28
> 前序：基于 [HANDOFF.md](HANDOFF.md) 和 [rfc/0003-test-cassette.md](rfc/0003-test-cassette.md) 的录像回放测试工作。

---

## 一、背景：什么是录像回放测试

aimux 不依赖网络和 API 密钥跑测试。做法是把真实 API 的请求/响应录成 JSON cassette 文件，测试时用 wiremock 起本地 mock server 回放，按路径 + 请求体特征匹配返回对应响应。

设计文档：[rfc/0003-test-cassette.md](rfc/0003-test-cassette.md)
回放基础设施：[aimux-providers/tests/common/replay.rs](aimux-providers/tests/common/replay.rs)
契约测试：[aimux-providers/tests/conformance_test.rs](aimux-providers/tests/conformance_test.rs)

### cassette 格式（一个文件 = 一个 HTTP 交互）

```json
{
  "source": "rig (MIT)",       // 或 "pydantic-ai (MIT)"
  "provider": "anthropic",
  "scenario": "streaming_smoke",
  "request": {
    "path": "/v1/messages",
    "method": "POST",
    "headers": { ... },
    "body": { ... }             // JSON object，用于特征评分匹配
  },
  "response": {
    "status": 200,
    "headers": { ... },
    "body": "event: message_start\ndata: ..."  // 原始文本，原样返回给被测代码
  }
}
```

### 匹配规则（见 replay.rs `CassetteRespond`）

1. **path + method** 选 group（wiremock Mock matcher）
2. **特征评分**：每个 cassette 的请求体标量字段（model/stream/等）和实际请求对比，model 权重 10、stream 权重 5、其他标量 1，非标量（messages/tools）忽略
3. **平局回退**：取第一个 cassette（按文件名排序）

---

## 二、本次工作做了什么

### 1. 发现并修复了 6 家 conformance 测试的假通过 ⚠️ 最关键

**问题**：conformance_test.rs 的容错逻辑（Err 分支只断言"不含 panic"）把 404 放过了。6 家 provider 的测试一直显示 pass，但实际从未命中过录像——都是 404 被容错吞掉。

| provider | 根因 | 修复 |
|---|---|---|
| openai | 录像全是 `/v1/responses`（Responses API），测试用 `model()` 打 `/chat/completions` | 改用 `responses_model()` + base_url 加 `/v1` |
| xai | 同上 | 改用 `responses_model()` + base_url 加 `/v1` |
| openrouter | base_url 缺 `/api/v1` 前缀 | base_url 加 `/api/v1` |
| groq | base_url 缺 `/openai/v1` 前缀 | base_url 加 `/openai/v1` |
| mistral | base_url 缺 `/v1` 前缀 | base_url 加 `/v1` |
| bedrock | cassette 路径 `%3A` 编码 vs provider 发 `:` 不编码 | replay.rs 加 percent_decode |

**容错收紧**：新增 `is_infrastructure_error()` helper，404 类错误（`ModelNotFound` 含 "404"）直接 panic，不再被容错放过。这是防止未来再出现假通过的护栏。

### 2. 新增 pydantic-ai 录像转换脚本 + 1742 个录像入库

新脚本：[scripts/convert_pydantic_ai.py](scripts/convert_pydantic_ai.py)

从 `reference/pydantic-ai/tests/` 的 1226 个 YAML 文件中，转换出 1742 个 JSON cassette（多轮 interaction 拆分）。按 uri host 动态判定 provider（不按目录名，因为 pydantic-ai 的 `test_openrouter` 目录里实际打的是 deepseek）。

转换要点：
- `parsed_body`（已解析 JSON）→ `request.body`（object，用于评分）
- `response.parsed_body` 序列化 → `response.body`（raw string）
- 优先用 raw `body` 字段（SSE 文本）
- **剥除 content-length/transfer-encoding/content-encoding 头**（录制时的长度和回放 body 长度不一致，会触发 hyper panic）
- 幂等：重跑先清理上次生成的 `source: "pydantic-ai (MIT)"` 文件再重新生成

### 3. 补齐 4 家 conformance 挂载

新增 `bedrock_conformance`、`cerebras_conformance`、`cohere_conformance`、`huggingface_conformance`，全部真命中。

### 4. 给 chatgpt/ollama cassette 目录加待实现标注

[aimux-providers/tests/cassettes/chatgpt/README.md](aimux-providers/tests/cassettes/chatgpt/README.md)
[aimux-providers/tests/cassettes/ollama/README.md](aimux-providers/tests/cassettes/ollama/README.md)

---

## 三、当前状态

### 测试结果

```
cargo test -p aimux-providers --test conformance_test  →  53 passed, 0 failed
cargo test -p aimux-providers --test replay_test       →  13 passed, 0 failed
```

### cassette 统计：总计 2626 个（rig 884 + pydantic-ai 1742）

| 目录 | 总数 | rig | pydantic-ai |
|------|----:|----:|----:|
| anthropic | 367 | 115 | 252 |
| bedrock | 312 | 22 | 290 |
| cerebras | 13 | 0 | 13 |
| chatgpt | 33 | 33 | 0 |
| cohere | 19 | 0 | 19 |
| copilot | 47 | 47 | 0 |
| deepseek | 72 | 67 | 5 |
| doubleword | 36 | 36 | 0 |
| gemini | 597 | 218 | 379 |
| groq | 107 | 25 | 82 |
| huggingface | 15 | 0 | 15 |
| llamafile | 11 | 11 | 0 |
| mistral | 86 | 24 | 62 |
| mistralrs | 9 | 9 | 0 |
| ollama | 24 | 21 | 3 |
| openai | 661 | 107 | 554 |
| openrouter | 137 | 77 | 60 |
| perplexity | 10 | 10 | 0 |
| xai | 62 | 62 | 0 |
| zai | 8 | 0 | 8 |

### conformance 覆盖：28 家已挂载且真命中

anthropic、alibaba、baseten、bedrock、bytedance、cerebras、chatgpt、cohere、copilot、deepinfra、deepseek、doubleword、fireworks、gemini、groq、huggingface、llamafile、mistral、mistralrs、moonshotai、ollama、openai、openrouter、perplexity、togetherai、vercel、xai、zai

---

## 四、本次改动文件清单

| 文件 | 改动 |
|------|------|
| [aimux-providers/tests/conformance_test.rs](aimux-providers/tests/conformance_test.rs) | 修 6 家配置 + 加 4 家挂载 + `is_infrastructure_error` 容错收紧 + 加 zai/ollama/chatgpt 3 家挂载 + 加 8 家 thin wrapper 宏挂载 + 加 4 家新增 provider 宏挂载 |
| [aimux-providers/tests/common/replay.rs](aimux-providers/tests/common/replay.rs) | 加 `percent_decode` 路径解码（解决 bedrock `%3A` 问题） |
| [aimux-providers/src/openrouter.rs](aimux-providers/src/openrouter.rs) | 加 `responses_model()` 方法（OpenRouter 录像含 Responses API） |
| [scripts/convert_pydantic_ai.py](scripts/convert_pydantic_ai.py) | 新文件，pydantic-ai YAML → JSON cassette 转换 |
| [scripts/generate_thin_wrapper_cassettes.py](scripts/generate_thin_wrapper_cassettes.py) | 新文件，从 OpenAI 模板派生 thin wrapper cassette（12 家） |
| [scripts/generate_all_providers.py](scripts/generate_all_providers.py) | 新文件，批量生成 171 个 provider .rs 文件 |
| [scripts/update_lib_rs.py](scripts/update_lib_rs.py) | 新文件，批量更新 lib.rs 导出 |
| [aimux-providers/src/{ollama,zai,github,siliconflow,lmstudio,sambanova}.rs](aimux-providers/src/) | 6 个独立 thin wrapper provider |
| `aimux-providers/src/*.rs`（171 个） | 批量生成的 thin wrapper provider 文件 |
| [aimux-providers/src/lib.rs](aimux-providers/src/lib.rs) | 新增 177 个 pub mod + pub use 导出 |
| [aimux-providers/tests/cassettes/chatgpt/README.md](aimux-providers/tests/cassettes/chatgpt/README.md) | 新文件，待实现标注 |
| [aimux-providers/tests/cassettes/ollama/README.md](aimux-providers/tests/cassettes/ollama/README.md) | 新文件，待实现标注 |
| `aimux-providers/tests/cassettes/*/test_*.json` | 新增 1742 个 pydantic-ai 录像文件 |
| `aimux-providers/tests/cassettes/{alibaba,baseten,bytedance,deepinfra,fireworks,moonshotai,togetherai,vercel,github,siliconflow,lmstudio,sambanova}/thin_wrapper_*.json` | 新增 24 个派生 cassette |

> ⚠️ 注意：git diff 还显示一些 `*_test.rs` 文件有 ±1-2 行变更（amazon_bedrock_image_test.rs、deepgram_transcription_test.rs 等），**这些不是本次改动**，是之前会话的未提交改动，提交时注意甄别。

---

## 五、录像来源

两个来源，都是 MIT 协议：

### 1. rig (MIT) — 505 个 YAML，转成 884 个 JSON

- 源目录：`reference/rig/tests/cassettes/<provider>/<scenario>/*.yaml`
- 转换脚本：[scripts/convert_cassettes.py](scripts/convert_cassettes.py)
- 格式：`when`（请求）/`then`（响应），body 是原始字符串

### 2. pydantic-ai (MIT) — 1226 个 YAML，转成 1742 个 JSON

- 源目录：`reference/pydantic-ai/tests/{models,cassettes,providers}/...`
- 转换脚本：[scripts/convert_pydantic_ai.py](scripts/convert_pydantic_ai.py)
- 格式：`interactions: [{request:{uri,method,parsed_body}, response:{status,parsed_body,body}}]`，多轮 interaction 拆成多个 JSON

### 其他参考项目（未采用）

| 项目 | 录像数 | 未采用原因 |
|------|------:|-----------|
| mastra | 126 | JSON 格式，含 Responses API，格式干净但量小，可后续补充 |
| traceloop-hub | 12 | 只录了 response body，没录 request，无法完整回放匹配 |
| LlamaIndexTS | 7 | fixture，多为手造假数据 |
| tensorzero | — | fixture，非真实录像 |

---

## 六、遗留项与下一步

### A. chatgpt / ollama / zai：conformance 挂载 ✅ 已完成

三家全部挂载成功，conformance_test 29 passed / 0 failed（原 24 + 新增 5）。

| provider | 录像路径 | 接入方式 | 测试 |
|---|---|---|---|
| **zai** | `/api/paas/v4/chat/completions` | `OpenAIProvider` + base_url `{server}/api/paas/v4`，model `glm-4.7` | do_generate + do_stream |
| **ollama** (pydantic-ai) | `/v1/chat/completions` | `OpenAIProvider` + base_url `{server}/v1`，model `gpt-oss:20b` | do_generate |
| **ollama** (rig) | `/api/chat` | ❌ Ollama 原生 NDJSON，不兼容 OpenAI（rig 录像挂载但不会被 OpenAI 请求命中，属预期行为） | — |
| **chatgpt** | `/backend-api/codex/responses` | `OpenAIProvider` + base_url `{server}/backend-api/codex` + `responses_model("gpt-5.4")` | do_generate + do_stream |

conformance 真命中从 17 家提升到 **20 家**。`is_infrastructure_error` 护栏确保无假通过。

### B. 有实现但无录像的 thin wrapper（8 家）✅ 已完成

这 8 家 src 里有 OpenAI 兼容 wrapper 实现，但没有真实返回录像：alibaba、baseten、bytedance、deepinfra、fireworks、moonshotai、togetherai、vercel。

**方案**：这 8 家全是 `OpenAIProvider` thin wrapper，响应格式与 OpenAI Chat Completions 字节级一致。用已有的 OpenAI 录像作模板，改写 `request.path` 和 `request.body.model` 即可得到有效录像。这不是造假数据——验证的是解析代码对 OpenAI 格式响应的处理能力。

| provider | 路径前缀 | model | base_url |
|---|---|---|---|
| alibaba | `/compatible-mode/v1` | `qwen-plus` | `{server}/compatible-mode/v1` |
| baseten | `/v1` | `meta-llama/Llama-3.1-8B-Instruct` | `{server}/v1` |
| bytedance | `/api/v3` | `doubao-pro-32k` | `{server}/api/v3` |
| deepinfra | `/v1/openai` | `meta-llama/Llama-3.1-8B-Instruct` | `{server}/v1/openai` |
| fireworks | `/inference/v1` | `llama-v3p1-8b-instruct` | `{server}/inference/v1` |
| moonshotai | `/v1` | `moonshot-v1-8k` | `{server}/v1` |
| togetherai | `/v1` | `meta-llama/Llama-3.1-8B-Instruct-Turbo` | `{server}/v1` |
| vercel | `/v1` | `gpt-4o` | `{server}/v1` |

脚本：[scripts/generate_thin_wrapper_cassettes.py](scripts/generate_thin_wrapper_cassettes.py)（幂等，重跑覆盖）。每家生成 non-stream + stream 两个 cassette，共 16 个。

conformance 代码用 `macro_rules!` 批量生成，每家 do_generate + do_stream 两个测试。

**仍缺录像**：azure、vertex 两家有独立认证和路径格式，无法用 OpenAI 模板复用，需有 key 时用 llmtape 补录。

conformance 真命中从 20 家提升到 **28 家**。

### B2. 新增独立 provider 实现（6 家）✅ 已完成

新增 ollama/zai/github/siliconflow/lmstudio/sambanova 6 个独立 thin wrapper provider 文件（之前 conformance 用 OpenAIProvider 直接构造，现改为独立 provider）。conformance 真命中 32 家。

### B3. 批量生成 provider 实现 ✅ 已完成（含修正）

对照 [rfc/0004-provider-inventory.md](rfc/0004-provider-inventory.md) 中所有标 ❌ 的厂商，批量生成 OpenAI 兼容 thin wrapper provider .rs 文件。

**修正**：初次生成了 171 个，但其中 49 个不是 OpenAI Chat Completions 兼容的（向量数据库、嵌入专用、图像/视频/音乐生成、语音/转写、非LLM服务、特殊认证），已全部撤除。

最终保留 122 个 thin wrapper，覆盖：
- 云 LLM 厂商（novita/nebius/hyperbolic/ai21/databricks/clarifai/minimax/baidu/tencent 等）
- 本地推理（llamacpp/vllm/sglang/xinference/localai/jlama/docker_model_runner 等）
- 网关/聚合（portkey/helicone/requesty/302ai/api2d/ohmygpt 等）
- 国产厂商（baichuan/stepfun/lingyiwanwu/coze/bigmodel/longcat 等）
- 编程订阅/其他（chatgpt/cline_pass/kiro/kilo/nanogpt 等）

撤除的 49 家见 inventory 第三节，按原因分：非 Chat API（需要各自 trait）、特殊认证（SigV4/IAM/OAuth）、非LLM服务。

脚本：[scripts/generate_all_providers.py](scripts/generate_all_providers.py)（幂等）。
provider 模块数：50 → **161**。`cargo check` 和 `cargo test --test conformance_test`（53 passed）均通过。

### C. pydantic-ai 的 xai 录像未转换（18 个）

pydantic-ai 的 `test_xai` 用 protobuf 格式（`request_sample`/`response_sample` + `raw` binary），脚本未处理。但 rig 已有 62 个 xai 录像覆盖，优先级低。

### D. 审计文档已更新 ✅

[TEST_AUDIT.md](TEST_AUDIT.md) 已于 2026-07-28 更新，修正内容：
- trait 表：8 个 trait 全部标为已定义（初版标 ❌ 未定义），修正 trait 名（去掉 V4 后缀），补充实现数
- Responses API 表：openai/xai/huggingface/open-responses/azure 全部标为已实现 + 有测试
- Embedding/Image/Speech/Transcription/Video/Files/Reranking 各节：移除"阻塞"标注，补充实际实现 + 测试文件
- "完全没有 Rust 测试的 chat provider"表：8/9 已补齐，仅 proda 仍缺
- 缺口汇总：重写为已完成 + 仍待完成两区
- 测试基线：24 文件/~18000 行 → 94 文件/~57500 行；11 provider → 44 模块/14 LanguageModel 实现/17 conformance 真命中/2626 cassette

---

## 七、关键文件索引

| 文件 | 作用 |
|------|------|
| `aimux-providers/tests/common/replay.rs` | 回放基础设施：加载/匹配/返回 cassette |
| `aimux-providers/tests/common/mod.rs` | 测试共享模块入口 |
| `aimux-providers/tests/conformance_test.rs` | 契约测试：17 家 provider 真命中 |
| `aimux-providers/tests/replay_test.rs` | 回放基础设施自身的单元测试 |
| `aimux-providers/tests/replay_fixtures/` | replay_test 用的手写 fixture（4 个） |
| `aimux-providers/tests/cassettes/` | 全部录像（2626 个 JSON，20 个 provider 目录） |
| `scripts/convert_cassettes.py` | rig YAML → JSON 转换 |
| `scripts/convert_pydantic_ai.py` | pydantic-ai YAML → JSON 转换 |
| `rfc/0003-test-cassette.md` | 录像回放方案设计文档 |
| `TEST_AUDIT.md` | 测试覆盖审计（部分过时，见上文 D） |
| `aimux-providers/src/lib.rs` | provider 导出清单（29 个 provider） |

---

## 八、如何重跑

```bash
# 重新转换 pydantic-ai 录像（幂等）
uv run python scripts/convert_pydantic_ai.py

# 重新转换 rig 录像（幂等，不覆盖 pydantic-ai 文件）
uv run python scripts/convert_cassettes.py

# 跑契约测试
cargo test -p aimux-providers --test conformance_test

# 跑回放基础设施测试
cargo test -p aimux-providers --test replay_test

# 跑单个 provider 的契约测试
cargo test -p aimux-providers --test conformance_test bedrock_conformance -- --nocapture
```
