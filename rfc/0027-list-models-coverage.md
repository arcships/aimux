# RFC-0027 list_models 覆盖跟踪

> 目标:覆盖全部**真 LLM provider**(有 chat/completions 能力、语义上有"模型列表"的 provider)。
> modality-only(speech/image/embed/search)不在范围,保留 trait 默认 `Unsupported`。
>
> 基线(2026-08-06):325 家宣称,真 LLM provider **292** 家。P1 已覆盖 251。

## 覆盖总览

| 层 | 数量 | P1 状态 | 说明 |
|---|---|---|---|
| registry OpenAI 兼容 | 251 | ✅ **已覆盖** | 共享 `OpenAIProvider::list_models`,`GET {base_url}/models` |
| standalone OpenAI 兼容(本地/自托管) | 23 | ⬜ 待做 | 内部都用 `OpenAIProvider`,重写 `list_models` 委托即可 |
| vertex_ai_* MaaS(OpenAI 协议) | 10 | ⬜ 待做 | OpenAI 协议,复用 `execute_list_models` |
| native protocol LLM | 8 | ⬜ 待做 | 端点各异,逐家适配(openai 已在 registry 覆盖,不重复) |
| **真 LLM 小计** | **292** | **251 / 292** | |
| modality-only(不在范围) | 33 | N/A | 返回 trait 默认 `Unsupported` |

---

## T1 — standalone OpenAI 兼容(23 家,低难度)

内部都用 `OpenAIProvider`,重写 `list_models` 一行委托 `execute_list_models` 即可。多数有 `/models` 端点。

### T1a — 本地推理引擎(13 家,均有 `/models` 或等价端点)

| # | provider | base_url 约定 | 备注 |
|---|---|---|---|
| 1 | ollama | `http://localhost:11434/v1` | OpenAI 兼容层;另有 `/api/tags` 原生端点 |
| 2 | vllm | `http://localhost:8000/v1` | 标准 OpenAI `/models` |
| 3 | sglang | `http://localhost:30000/v1` | 标准 OpenAI `/models` |
| 4 | llamacpp | `http://localhost:8080/v1` | OpenAI 兼容 |
| 5 | lmstudio | `http://localhost:1234/v1` | OpenAI 兼容 |
| 6 | xinference | `http://localhost:9997/v1` | OpenAI 兼容 |
| 7 | localai | `http://localhost:8080/v1` | OpenAI 兼容 |
| 8 | litellm_proxy | `http://localhost:4000/v1` | OpenAI 兼容代理 |
| 9 | mistralrs | `http://localhost:8080/v1` | OpenAI 兼容 |
| 10 | jlama | `http://localhost:8080/v1` | OpenAI 兼容 |
| 11 | mlx | `http://localhost:8080/v1` | OpenAI 兼容 |
| 12 | onnx | `http://localhost:8080/v1` | OpenAI 兼容 |
| 13 | openvino | `http://localhost:8080/v1` | OpenAI 兼容 |

### T1b — 其他本地/自托管(4 家)

| # | provider | 备注 |
|---|---|---|
| 14 | gaudi | OpenAI 兼容 |
| 15 | omlx | OpenAI 兼容 |
| 16 | oobabooba | OpenAI 兼容(text-generation-webui) |
| 17 | cybertron | OpenAI 兼容 |
| 18 | local | 通用本地 OpenAI 兼容 |
| 19 | docker_model_runner | Docker 内置模型服务 |
| 20 | llamafile | OpenAI 兼容 |

### T1c — 云端 OpenAI 兼容(3 家)

| # | provider | 备注 |
|---|---|---|
| 21 | huggingface | TGI OpenAI 兼容端点 |
| 22 | openrouter | `GET /api/v1/models`,富字段 |
| 23 | bedrock_mantle | OpenAI 兼容 |

---

## T2 — vertex_ai_* MaaS(10 家,低难度)

全部 OpenAI 协议,复用 `execute_list_models`,但鉴权用 GCP Bearer token(非 API key)。

| # | provider | 模型族 |
|---|---|---|
| 24 | vertex_ai_openai_models | OpenAI(Gemini via OpenAI API) |
| 25 | vertex_ai_anthropic_models | Anthropic(Claude) |
| 26 | vertex_ai_ai21_models | AI21 |
| 27 | vertex_ai_deepseek_models | DeepSeek |
| 28 | vertex_ai_llama_models | Llama |
| 29 | vertex_ai_minimax_models | MiniMax |
| 30 | vertex_ai_mistral_models | Mistral |
| 31 | vertex_ai_moonshot_models | Moonshot(Kimi) |
| 32 | vertex_ai_qwen_models | Qwen |
| 33 | vertex_ai_zai_models | ZAI |

---

## T3 — native protocol LLM(8 家,中难度,端点各异)

openai 已在 registry 覆盖(不重复)。剩余 8 家需逐家适配端点格式。

### T3a — 已有 cassette(4 家,优先)

| # | provider | 端点 | cassette |
|---|---|---|---|
| 34 | anthropic | `GET /v1/models` → `{data:[{id,type,display_name}]}` | ✅ `anthropic/list_models_smoke.json` |
| 35 | google | `GET /v1beta/models` → `{models:[{name,supportedGenerationMethods}]}` | ✅ `gemini/list_models_smoke.json` |
| 36 | ollama(独立) | `GET /api/tags` → `{models:[{name}]}` | ✅ `ollama/list_models_smoke.json` |
| 37 | codex | `GET /v1/models`(GitHub Copilot) | ✅ `copilot/list_models_smoke.json` |

### T3b — 需查文档确认端点(4 家)

| # | provider | 预期端点 | 备注 |
|---|---|---|---|
| 38 | azure | `GET /openai/deployments?api-version=...` | 列 deployment 非 model |
| 39 | bedrock | 无直接 list;用 `ListFoundationModels` API | AWS SigV4,需独立适配 |
| 40 | cohere | `GET /v1/models` → `{models:[{name,endpoints}]}` | |
| 41 | mistral | `GET /v1/models` → `{data:[{id}]}` | OpenAI-like |
| 42 | xai | `GET /v1/models` → `{data:[{id}]}` | OpenAI-like |
| 43 | anthropic_aws | 同 bedrock(无独立 list) | |
| 44 | vertex | 同 google(共享 `/v1beta/models`) | |

---

## 不在范围:modality-only(33 家)

保留 `Provider::list_models` trait 默认实现(返回 `Unsupported`)。这些 provider 语义上没有"模型列表":

- **Speech/transcription(10)**: elevenlabs, deepgram, assemblyai, aws_polly, cartesia, hume, gladia, revai, lmnt, fal
- **Image/video(8)**: black_forest_labs, replicate, luma, prodia, klingai, recraft, stability, runwayml
- **Embed/rerank/search(13)**: voyage, jina_ai, tavily, exa_ai, firecrawl, serper, searxng, you_com, dataforseo, google_pse, linkup, parallel_ai, tinyfish
- **Other(2)**: open_responses(Responses API,非 chat 模型列表)

---

## 实现优先级

| 阶段 | 范围 | 新增覆盖 | 累计 | 难度 |
|---|---|---|---|---|
| P1 ✅ | registry 251 家 | 251 | 251 | 完成 |
| **P2.5** | T1(23 standalone)+ T2(10 MaaS) | 33 | 284 | 低(复用 execute_list_models) |
| **P3** | T3a(4 家有 cassette) | 4 | 288 | 中(逐家适配端点) |
| **P4** | T3b(4 家需查文档) | 4 | 292 | 中(端点格式待验证) |
| — | modality-only 33 家 | 0 | 292 | N/A(trait 默认) |

**最终目标:292 / 325 = 89.8%**(剩余 33 家 modality-only 语义不适用)。
