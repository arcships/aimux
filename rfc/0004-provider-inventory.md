# Provider Inventory and Implementation Status

## 1. Providers Currently Implemented in aimux

**A total of 172 provider modules** (as of 2026-07-28).

### Native Protocol Implementations (with independent model/convert, handling provider-specific differences)

| Provider | Capabilities | Notes |
|------|------|------|
| openai | text, streaming, tools, embedding, image, speech, transcription, file | Reference implementation, 655 lines model + 1223 lines convert, includes Responses API |
| anthropic | text, streaming, tools, file, cache control | 1438 lines convert, includes prompt caching, schema sanitization |
| anthropic_aws | text, streaming | Anthropic via the AWS Bedrock side channel |
| google | text, streaming, embedding, image, video, file | 995 lines convert, Gemini native protocol |
| bedrock | text, streaming, embedding, image, reranking | 625 lines convert, Converse API + SigV4 signing + event stream |
| vertex | text, streaming, embedding, image, transcription, video | Google Vertex AI, GCP authentication |
| azure | text, streaming, Responses API | Azure OpenAI, multiple authentication methods |
| cohere | text, streaming, embedding, reranking | Cohere native protocol |
| mistral | text, streaming, embedding | Mistral native protocol |
| xai | text, streaming, tools, Responses API | 636 lines convert, handles reasoning content, citations, search parameters |
| deepseek | text, streaming, reasoning | 600 lines, independently handles reasoning_content and thinking fields |

### OpenAI-Compatible Thin Wrappers (only change the URL and environment variables, no customization points)

About 195. Includes early hand-written ones and batch-generated ones:

groq, fireworks, togetherai, perplexity, moonshotai, cerebras, openrouter (including Responses API), ollama, copilot, llamafile, mistralrs, doubleword, zai, github, siliconflow, lmstudio, sambanova, alibaba, baseten, bytedance, deepinfra, huggingface, vercel, ai21, ai302, aibadgr, aigc2d, aihubmix, ails, aiml, albert, antling, anyscale, api2d, api2gpt, apiserpent, atlascloud, baichuan, baidu, bedrock_mantle, bigmodel, bing, byteplus, bytez, canopywave, chatgpt, clarifai, cline_pass, closeai, codestral, cometapi, commandcode, compactifai, coze, cybertron, databricks, datarobot, deepbricks, deepl, dify, doc2x, docker_model_runner, doubaoaudio, embercloud, fastcrw, fastembed, fastgpt, fastrouter, featherless_ai, flux, friendliai, galadriel, gaudi, gdc, gigachat, gonka24, gradient_ai, helicone, heroku, hosted_vllm, hyperbolic, ideogram, inception, inference_net, infinity, inworld, jimeng, jina, jlama, kilo, kiro, kluster_ai, krutrim, lambda_ai, lemonfox_ai, lingyiwanwu, litellm_proxy, llamacpp, local, localai, longcat, matterai, meshy, meta_llama, midjourney, milvus, minimax, mira, mixedbread, mlx, modal, modelscope, mokaai, morph, murf, nanogpt, ncompass, nebius, nextbit, nlp_cloud, nomic, nous_research, novita, nscale, nvidia_nim, nvidia_riva, oci, ohmygpt, ollama_cloud, omlx, onnx, oobabooba, openaimax, openaisb, opencode_go, opencode_zen, openvino, orcarouter, ovhcloud, parasail, perfxcloud, petals, pg_vector, pioneer, playai, portkey, predibase, qdrant, qihoo360, qiniu_ai, recraft, reka_ai, requesty, reve, runware, runwayml, s3_vectors, sagemaker, sakana, sangforaicp, sap, sarvam, scaleway, scx_ai, segmind, sglang, skylark, slack, snowflake, soniox, sora, speechify, stability_ai, stepfun, streamlake, submodel, suno, tei, tencent, text_embeddings_inference, tokenpony, tripo3d, tundra, upstage, v0, vidu, vllm, wafer, watsonx, xiaomimimo, xinference, etc.

### Providers Dedicated to Non-Text Capabilities

| Provider | Capabilities |
|------|------|
| voyage | embedding, reranking |
| cartesia | speech, transcription |
| elevenlabs | speech, transcription |
| hume | speech |
| lmnt | speech |
| assemblyai | transcription |
| deepgram | transcription |
| fal | image, transcription, video |
| gladia | transcription |
| revai | transcription |
| black_forest_labs | image |
| luma | image |
| prodia | image, video |
| replicate | image, video |
| klingai | video |

### General Wrapper

| Module | Notes |
|------|------|
| open_responses | General Responses API wrapper, 1238 lines |

### Known Issues

Thin wrappers still lack a configuration descriptor structure (recorded in RFC-0002). Differences such as DeepSeek's reasoning field, Alibaba Tongyi's reasoning_content, and Groq's top_k limit are still being lost. This is the next step.

**Total: 161 provider modules.**

> Of these, 11 native protocol implementations + 1 general wrapper + 15 dedicated to non-text capabilities + 134 OpenAI-compatible thin wrappers.
> Note: vector databases (milvus/qdrant/pg_vector/s3_vectors), embedding-dedicated (jina/nomic/mixedbread/clip/fastembed/tei),
> image/video/music generation (recraft/ideogram/stability_ai/meshy/tripo3d/runwayml/sora/vidu/jimeng/midjourney/flux/suno),
> speech/transcription (murf/playai/speechify/inworld/aws_polly/nvidia_riva/soniox/doubaoaudio/mokaai),
> non-LLM services (bing/deepl/dify/slack/doc2x/streamlake/antling/sangforaicp/skylark),
> special authentication (watsonx/sagemaker/sap/oci/snowflake/bedrock_mantle), a total of 49 are not integrated —
> they are not OpenAI Chat Completions compatible and require their own traits or authentication methods.

---

## 2. Complete Provider Inventory of All Reference Projects

> Merged and de-duplicated all providers that appeared in the four scan results (Rust competitors / Python ecosystem / other languages / gateways).
> The "Gateway coverage" column marks gateway projects that support the provider (not exhaustive; representative ones are listed).

### Mainstream Large Model Providers

| Provider | aimux | rig | litellm | Gateway coverage | Other major projects |
|------|:---:|:---:|:---:|:---:|------|
| openai | ✅ native | ✅ | ✅ | all | almost all projects |
| anthropic | ✅ native | ✅ | ✅ | all | almost all projects |
| google (gemini) | ✅ native | ✅ | ✅ | all | almost all projects |
| azure (openai) | ✅ native | ✅ | ✅ | all | almost all projects |
| bedrock (aws) | ✅ native | ✅ | ✅ | all | almost all projects |
| vertex (gcp) | ✅ native | ✅ | ✅ | all | langchain4j, spring-ai |
| mistral | ✅ native | ✅ | ✅ | all | almost all projects |
| cohere | ✅ native | ✅ | ✅ | all | almost all projects |
| xai (grok) | ✅ native | ✅ | ✅ | all | genai, pydantic-ai, instructor |
| deepseek | ✅ native | ✅ | ✅ | all | almost all projects |

### OpenAI-Compatible Cloud Providers

| Provider | aimux | rig | litellm | Gateway coverage | Other |
|------|:---:|:---:|:---:|:---:|------|
| groq | ✅ thin | ✅ | ✅ | portkey, bifrost, higress, one-hub, axonhub, ferro | pydantic-ai, instructor, aisuite, llama_index |
| fireworks | ✅ thin | — | ✅ | portkey, one-hub, higress, APIPark, ferro | langchain, llama_index, langchain4j |
| together | ✅ thin | ✅ | ✅ | portkey, higress, ferro | langchain, llama_index, langchain4j, aisuite |
| perplexity | ✅ thin | ✅ | ✅ | portkey, ferro | langchain, llama_index, langchainjs |
| moonshot (kimi) | ✅ thin | ✅ | ✅ | all domestic gateways | llama_index, genai |
| cerebras | ✅ thin | — | ✅ | portkey, bifrost, axonhub, ferro | pydantic-ai, instructor, aisuite, llama_index |
| openrouter | ✅ thin | ✅ | ✅ | almost all gateways | langchain, llama_index, genai, aisuite |
| huggingface | ✅ thin | ✅ | ✅ | portkey, one-hub, bifrost, APIPark, ferro | langchain, llama_index, langchain4j, semantic-kernel |
| deepinfra | ✅ thin | — | ✅ | portkey, llmgateway | llama_index, langchain |
| ovhcloud | ✅ thin | — | ✅ | portkey, llmgateway | llama_index, langchain4j |
| replicate | ✅ image | — | ✅ | one-hub, TokenHub, ferro | langchain, llama_index |
| novita | ✅ thin | — | ✅ | portkey, one-api, TokenHub, APIPark, ferro, llmgateway | llama_index |
| nebius | ✅ thin | — | ✅ | portkey, bifrost, ferro, llmgateway | genai, aisuite, llama_index |
| hyperbolic | ✅ thin | ✅ | ✅ | portkey | — |
| minimax | ✅ thin | ✅ | ✅ | all domestic gateways, portkey, claude-code-router, llmgateway | genai, llama_index |
| mistralrs | ✅ thin | ✅ local | — | — | — |
| llamafile | ✅ thin | ✅ local | ✅ | — | langchaingo |
| doubleword | ✅ thin | ✅ | — | — | — |
| mira | ✅ thin | ✅ | — | — | — |
| xiaomimimo (Xiaomi) | ✅ thin | ✅ | — | manifest, llmgateway, chats, TokenHub | — |
| zai (Zhipu/z.ai) | ✅ thin | ✅ | ✅ | all domestic gateways, portkey, claude-code-router, llmgateway | genai |
| github (models) | ✅ thin | — | ✅ | higress, one-hub, chats, TokenHub | langchain4j, llama_index |
| github_copilot | ✅ thin | ✅ | ✅ | axonhub, manifest, TokenHub, bifrost | genai |
| chatgpt (subscription) | ✅ thin | ✅ | ✅ | — | — |
| ai21 | ✅ thin | — | ✅ | portkey, ferro | langchain, llama_index |
| anyscale | ✅ thin | — | ✅ | portkey | langchain |
| sambanova | ✅ thin | — | ✅ | portkey, ferro | llama_index |
| predibase | ✅ thin | — | ✅ | portkey | — |
| triton (nvidia) | ✅ thin | — | ✅ | portkey, higress | — |
| databricks | ✅ thin | — | ✅ | portkey, ferro, TokenHub | langchain, llama_index |
| sagemaker (aws) | ❌ | — | ✅ | portkey | langchain, llm-chain |
| watsonx (ibm) | ❌ | — | ✅ | — | langchain, langchain4j, langchaingo |
| scaleway | ✅ thin | — | ✅ | TokenHub | — |
| snowflake | ❌ | — | ✅ | TokenHub | — |
| sap | ❌ | — | ✅ | — | — |
| oci (oracle) | ❌ | — | ✅ | TokenHub, portkey(oracle) | llama_index |
| nlp_cloud | ✅ thin | — | ✅ | — | langchain |
| friendliai | ✅ thin | — | ✅ | — | llama_index |
| clarifai | ✅ thin | — | ✅ | — | langchain, llama_index |
| gigachat (Samsung) | ✅ thin | — | ✅ | — | — |
| codestral | ✅ thin | — | ✅ | — | — |
| morph | ✅ thin | — | ✅ | TokenHub | — |
| v0 | ✅ thin | — | ✅ | — | — |
| aiml | ✅ thin | — | ✅ | TokenHub | — |
| heroku | ✅ thin | — | ✅ | — | — |
| hosted_vllm | ✅ thin | — | ✅ | — | — |
| nvidia_nim | ✅ thin | — | ✅ | manifest, ferro, APIPark, TokenHub | — |
| nscale | ✅ thin | — | ✅ | portkey, llmgateway | — |
| lambda_ai | ✅ thin | — | ✅ | portkey | — |
| databricks | ✅ thin | — | ✅ | portkey, ferro | langchain |
| petals | ✅ thin | — | ✅ | — | — |
| oobabooba | ✅ thin | — | ✅ | — | — |
| inception | ✅ thin | — | ✅ | TokenHub | — |
| galadriel | ✅ thin | — | ✅ | — | — |
| gdc | ✅ thin | — | ✅ | — | — |
| datarobot | ✅ thin | — | ✅ | — | — |
| infinity | ✅ thin | — | ✅ | — | — |
| kluster-ai | ✅ thin | — | — | portkey | — |
| featherless-ai | ✅ thin | — | ✅ | portkey | llama_index |
| krutrim | ✅ thin | — | — | portkey | — |
| bytez | ✅ thin | — | — | portkey | — |
| upstage | ✅ thin | — | — | portkey, APIPark, langchain4j | — |
| deepbricks | ✅ thin | — | — | portkey | — |
| lemonfox-ai | ✅ thin | — | — | portkey | — |
| inference-net | ✅ thin | — | — | portkey, llmgateway | — |
| 302ai | ✅ thin | — | — | portkey, TokenHub | — |
| cometapi | ✅ thin | — | ✅ | portkey, TokenHub | — |
| matterai | ✅ thin | — | — | portkey | — |
| meshy | ❌ | — | — | portkey | — |
| nextbit | ✅ thin | — | — | portkey | — |
| tripo3d | ❌ | — | — | portkey | — |
| modal | ✅ thin | — | — | portkey | — |
| aibadgr | ✅ thin | — | — | portkey, TokenHub, chats | — |
| ncompass | ✅ thin | — | — | portkey | — |
| reka-ai | ✅ thin | — | ✅ | portkey | llama_index |
| stability-ai | ❌ | — | ✅ | portkey, one-hub, spring-ai | — |
| segmind | ❌ | — | — | portkey | — |
| monsterapi | ✅ thin | — | — | portkey | llama_index |
| sgl (sglang) | ✅ thin | — | ✅ | bifrost | guidance, outlines, llama_index |
| parasail | ✅ thin | — | — | bifrost | — |
| wafer | ✅ thin | — | — | bifrost, axonhub | — |
| runway | ❌ | — | ✅(runwayml) | bifrost | — |
| runware | ❌ | — | — | bifrost | — |
| sarvam | ✅ thin | — | ✅ | bifrost, claude-code-router, TokenHub | — |
| opencode-go | ✅ thin | — | — | bifrost, manifest, axonhub | genai |
| opencode-zen | ✅ thin | — | — | bifrost, manifest | — |
| bedrock_mantle | ❌ | — | ✅ | bifrost, llmgateway | — |
| vllm | ✅ thin | — | ✅ | bifrost, higress | guidance, outlines, llama_index |
| nous (nousresearch) | ✅ thin | — | — | manifest | — |
| byteplus | ✅ thin | — | — | manifest | — |
| kiro | ✅ thin | — | — | manifest | — |
| pioneer | ✅ thin | — | — | manifest | — |
| kilo | ✅ thin | — | — | manifest, TokenHub | — |
| cline-pass | ✅ thin | — | — | manifest | — |
| commandcode | ✅ thin | — | — | manifest | — |
| copilot (github) | ✅ thin | ✅ | ✅ | manifest | genai |
| longcat | ✅ thin | — | — | higress, axonhub | — |
| yi (01.AI) | ✅ thin | — | — | higress, one-api, APIPark, TokenHub, chats | llama_index |
| deepl | ❌ | — | — | higress, one-api | — |
| dify | ❌ | — | — | higress, new-api, aiproxy | — |
| stepfun (StepFun) | ✅ thin | — | — | higress, one-api, APIPark, TokenHub | llama_index |
| baichuan | ✅ thin | — | — | higress, one-api, APIPark, TokenHub, coai | — |
| kling (Kling) | ✅ video | — | — | higress, new-api, one-hub, TokenHub | — |
| triton | ✅ thin | — | ✅ | higress | — |
| perfxcloud | ✅ thin | — | — | APIPark | — |
| doc2x | ❌ | — | — | aiproxy | — |
| sangforaicp | ❌ | — | — | aiproxy | — |
| streamlake | ❌ | — | — | aiproxy | — |
| antling | ❌ | — | — | aiproxy | — |
| text_embeddings_inference | ❌ | — | — | aiproxy | — |
| skylark (Skylark) | ❌ | — | — | coai | — |
| bing (new bing) | ❌ | — | — | coai | — |
| slack (slack claude) | ❌ | — | — | coai | — |
| ideogram | ❌ | — | — | one-hub | — |
| flux | ❌ | — | — | one-hub | — |
| suno (music) | ❌ | — | — | new-api, one-hub | — |
| midjourney | ❌ | — | — | new-api, one-hub, coai | — |
| sora | ❌ | — | — | new-api | — |
| vidu (video) | ❌ | — | — | new-api | — |
| jimeng (Jimeng) | ❌ | — | — | new-api | — |
| doubaoaudio | ❌ | — | — | aiproxy | — |
| mokaai | ❌ | — | — | new-api | — |
| recraft | ❌ | — | ✅ | one-hub, portkey, TokenHub | — |
| sakana | ✅ thin | — | — | llmgateway | — |
| meta | ✅ thin | — | ✅(meta_llama) | manifest, llmgateway, TokenHub | — |
| scx-ai | ✅ thin | — | — | llmgateway | — |
| atlascloud | ✅ thin | — | — | llmgateway, genai | — |
| canopywave | ✅ thin | — | — | llmgateway | — |
| embercloud | ✅ thin | — | — | llmgateway | — |
| tundra | ✅ thin | — | — | llmgateway | — |
| reve | ✅ thin | — | — | llmgateway | — |
| gonka24 | ✅ thin | — | — | llmgateway | — |
| albert | ✅ thin | — | — | OpenGateLLM | — |
| tei (text embeddings inference) | ❌ | — | — | OpenGateLLM, aiproxy | — |
| fastcrw | ✅ thin | — | ✅ | — | — |
| apiserpent | ✅ thin | — | ✅ | — | — |
| modelscope (ModelScope) | ✅ thin | — | ✅ | axonhub, TokenHub | llama_index |
| docker_model_runner | ✅ thin | — | ✅ | — | — |
| ollama_cloud | ✅ thin | — | — | bifrost, ferro, manifest | genai |
| aihubmix | ✅ thin | — | — | — | genai |
| bigmodel (Zhipu) | ✅ thin | — | — | — | genai |
| litellm_proxy | ✅ thin | — | ✅ | — | — |
| compactifai | ✅ thin | — | ✅ | — | — |
| fastembed | ❌ | — | — | — | swiftide (local) |
| docker_model_runner | ✅ thin | — | ✅ | — | — |

### Domestic Providers (separate summary)

| Provider | aimux | rig | litellm | Gateway coverage | Other |
|------|:---:|:---:|:---:|:---:|------|
| alibaba (Tongyi/DashScope/Bailian) | ✅ thin | — | ✅(dashscope) | new-api, one-api, simple-one-api, portkey, higress, ferro, APIPark, claude-code-router, llmgateway | genai, swiftide, llm-connector, llama_index |
| baidu (Wenxin/ernie) | ✅ thin | — | — | new-api, one-api, simple-one-api, APIPark, TokenHub, chats | genai, langchaingo, langchain-swift |
| zhipu (Zhipu/glm) | ✅ thin | ✅(zai) | — | new-api, one-api, simple-one-api, portkey, higress, APIPark, axonhub, claude-code-router, llmgateway, aiproxy | llm-connector, langchain-swift, llama_index, genai |
| tencent (Hunyuan/hunyuan) | ✅ thin | — | ✅ | new-api, one-api, simple-one-api, coai, APIPark, chats, TokenHub | llm-connector |
| xunfei (Xunfei/spark) | ✅ thin | — | — | new-api, one-api, simple-one-api, higress, APIPark, chats | — |
| bytedance (Volcano/Doubao/volcengine) | ✅ thin | — | ✅(volcengine) | new-api, one-api, simple-one-api, axonhub, APIPark, llmgateway | — |
| baichuan | ✅ thin | — | — | one-api, higress, APIPark, TokenHub, coai | — |
| stepfun (StepFun) | ✅ thin | — | — | one-api, higress, APIPark, TokenHub | llama_index |
| minimax | ✅ thin | ✅ | ✅ | new-api, one-api, simple-one-api, portkey, claude-code-router, llmgateway, chats, TokenHub | genai, llama_index |
| moonshot (Moonshot AI/kimi) | ✅ thin | ✅ | ✅ | new-api, one-api, portkey, higress, ferro, claude-code-router, llmgateway, APIPark | genai |
| lingyiwanwu (01.AI/yi) | ✅ thin | — | — | new-api, one-api, higress, APIPark, TokenHub, chats | llama_index |
| 360 (ai360/zhinao) | ✅ thin | — | — | new-api, one-api, coai, APIPark, chats | — |
| coze (Coze) | ✅ thin | — | — | new-api, one-api, aiproxy | — |
| siliconflow (SiliconFlow) | ✅ thin | — | — | new-api, one-api, APIPark, chats, claude-code-router | llama_index |
| gigachat (Samsung) | ✅ thin | — | ✅ | — | — |
| qiniu-ai (Qiniu) | ✅ thin | — | — | claude-code-router | — |
| modelscope (ModelScope) | ✅ thin | — | ✅ | axonhub, TokenHub | llama_index |
| longcat | ✅ thin | — | — | higress, axonhub | — |

### Local Inference

| Provider | aimux | rig | litellm | Gateway coverage | Other |
|------|:---:|:---:|:---:|:---:|------|
| ollama | ✅ thin | ✅ | ✅ | new-api, one-api, portkey, bifrost, one-hub, higress, ferro, APIPark, chats, TokenHub | almost all SDKs |
| llama.cpp | ✅ thin | ✅ local | — | — | guidance, outlines, llm-chain, langchain |
| lmstudio | ✅ thin | — | ✅ | APIPark | aisuite, langchain-swift, edgequake |
| vllm | ✅ thin | — | ✅ | bifrost, higress | guidance, outlines, llama_index, edgequake |
| sglang | ✅ thin | — | — | bifrost | guidance, outlines, llama_index |
| xinference | ✅ thin | — | ✅ | new-api, APIPark | llm-connector |
| mistralrs | ✅ thin | ✅ local | — | — | — |
| omlx / mlx_lm | ✅ thin | — | — | — | genai, edgequake |
| triton | ✅ thin | — | ✅ | higress, portkey | — |
| local (general local) | ✅ thin | — | — | — | langchaingo, langchain-swift, llm-chain |
| cybertron | ✅ thin | — | — | — | langchaingo |
| jlama | ✅ thin | — | — | — | langchain4j |
| localai | ✅ thin | — | — | — | langchain4j, llama_index |
| onnx | ✅ thin | — | — | — | semantic-kernel, langchain4j |
| openvino | ✅ thin | — | — | — | llama_index |
| mlx | ✅ thin | — | — | — | outlines, llama_index |
| gaudi | ✅ thin | — | — | — | llama_index |

### Embedding/Reranking Dedicated

| Provider | aimux | rig | litellm | Gateway coverage | Other |
|------|:---:|:---:|:---:|:---:|------|
| voyage | ✅ | ✅ | ✅ | portkey, ferro | langchainjs, langchain4j, llama_index |
| jina | ❌ | — | ✅(jina_ai) | portkey, one-api, axonhub, higress, langchain4j, llama_index | edgequake, langchaingo |
| nomic | ❌ | — | — | portkey | langchainjs, langchain4j |
| cohere embedding/rerank | ✅ | ✅ | ✅ | all | most projects |
| fastembed | ❌ | — | — | — | swiftide (local) |
| mixedbread | ❌ | — | — | — | LlamaIndexTS |
| clip | ❌ | — | — | — | LlamaIndexTS |
| pg_vector | ❌ | — | ✅ | — | — |
| s3_vectors | ❌ | — | ✅ | — | — |
| milvus (vector database) | ❌ | — | ✅ | portkey | — |
| qdrant (vector database) | ❌ | — | — | portkey | — |

### Speech/Transcription/Image/Video Dedicated

| Provider | aimux | Type | litellm | Gateway coverage | Other |
|------|:---:|------|:---:|:---:|------|
| elevenlabs | ✅ | speech | ✅ | bifrost, llmgateway | spring-ai |
| deepgram | ✅ | transcription | ✅ | — | aisuite, mastra |
| assemblyai | ✅ | transcription | — | — | llama_index |
| cartesia | ✅ | speech | — | — | — |
| fal | ✅ | image/video | ✅(fal_ai) | — | — |
| replicate | ✅ | image/video | ✅ | one-hub, TokenHub, ferro | langchain, llama_index |
| black_forest_labs | ✅ | image | ✅ | — | — |
| luma | ✅ | image | ✅(runwayml?) | — | — |
| prodia | ✅ | image/video | — | — | — |
| klingai | ✅ | video | — | new-api, higress, one-hub, TokenHub | — |
| stability | ❌ | image | ✅(stability) | portkey, one-hub, spring-ai | — |
| recraft | ❌ | image | ✅ | one-hub, portkey, TokenHub | — |
| runwayml | ❌ | video | ✅(runwayml) | bifrost | — |
| hume | ✅ | speech | — | — | — |
| lmnt | ✅ | speech | — | — | — |
| gladia | ✅ | transcription | — | — | — |
| revai | ✅ | transcription | — | — | — |
| aws_polly | ❌ | speech | ✅ | — | — |
| nvidia_riva | ❌ | speech | ✅ | — | — |
| soniox | ❌ | transcription | ✅ | — | — |
| midjourney | ❌ | image | — | new-api, one-hub, coai | — |
| sora | ❌ | video | — | new-api | — |
| vidu | ❌ | video | — | new-api | — |
| jimeng (Jimeng) | ❌ | image | — | new-api | — |
| ideogram | ❌ | image | — | one-hub | — |
| flux | ❌ | image | — | one-hub | — |
| suno | ❌ | music | — | new-api, one-hub | — |
| meshy | ❌ | 3D | — | portkey | — |
| tripo3d | ❌ | 3D | — | portkey | — |
| segmind | ❌ | image | — | portkey | — |
| runware | ❌ | image | — | bifrost | — |
| sarvam | ✅ thin | speech | ✅ | bifrost, claude-code-router, TokenHub | — |
| murf | ❌ | speech | — | mastra | — |
| playai | ❌ | speech | — | mastra | — |
| speechify | ❌ | speech | — | mastra | — |
| inworld | ❌ | speech | — | mastra | — |

### Programming Subscription to API (coding plan)

A category that emerged in 2026, converting programming tool subscription quotas into APIs. OAuth authentication, account pools, specific endpoints, with account-ban risk.

| Provider/Type | rig | new-api | axonhub | claude-code-router | Other gateways |
|------|:---:|:---:|:---:|:---:|:---:|
| chatgpt subscription (codex) | ✅ | ✅ | ✅ | — | — |
| github copilot | ✅ | — | ✅ | — | portkey, bifrost, manifest, TokenHub |
| claude code subscription | — | — | ✅ | ✅ | — |
| cline | — | — | ✅ | — | manifest(cline-pass) |
| nanogpt | — | — | ✅ | — | llmgateway, TokenHub |
| kimi code | — | — | ✅ | — | — |
| opencode_go | — | — | ✅ | — | bifrost, manifest |
| antigravity | — | — | ✅ | — | — |
| aimux (axonhub built-in) | — | — | ✅ | — | — |
| synthetic | — | — | ✅ | — | TokenHub |
| neuralwatt | — | — | ✅ | — | — |
| apertis | — | — | ✅ | — | — |
| wafer | — | — | ✅ | — | bifrost |
| zhipu coding | — | — | — | ✅(zhipu-cn-coding) | aiproxy(ZhipuCoding) |
| zai coding | — | — | — | ✅(zai-global-coding) | — |
| fenno | — | — | — | ✅ | — |
| runapi | — | — | — | ✅ | — |
| teamorouter | — | — | — | ✅ | — |
| unity2 | — | — | — | ✅ | — |
| code0 | — | — | — | ✅ | — |
| claudeapi | — | — | — | ✅ | — |
| qiniu-ai | — | — | — | ✅ | — |
| kiro | — | — | — | — | manifest |
| pioneer | — | — | — | — | manifest |
| kilo | — | — | — | — | manifest, TokenHub |
| commandcode | — | — | — | — | manifest |
| TokenHub 100+ proxies | — | — | — | — | TokenHub(requesty, helicone, poe, submodel, morph, nearai, neon, poolside, wandb, clarifai, etc.) |

aimux currently does not support any coding plan integration.

### Gateway/Aggregation/Proxy Type

| Provider | aimux | rig | litellm | Gateway coverage |
|------|:---:|:---:|:---:|:---:|
| openrouter | ✅ thin | ✅ | ✅ | almost all gateways |
| vercel (ai gateway) | ✅ thin | — | ✅(vercel_ai_gateway) | — |
| portkey | ✅ thin | — | — | ✅portkey-gateway |
| helicone | ✅ thin | — | — | TokenHub |
| requesty | ✅ thin | — | — | TokenHub |
| 302ai | ✅ thin | — | — | portkey, TokenHub |
| cometapi | ✅ thin | — | ✅ | portkey, TokenHub |
| novita | ✅ thin | — | ✅ | portkey, one-api, TokenHub, APIPark |
| siliconflow | ✅ thin | — | — | new-api, one-api, APIPark, chats, claude-code-router |
| submodel | ✅ thin | — | — | new-api, TokenHub |
| api2d | ✅ thin | — | — | new-api, one-api |
| ohmygpt | ✅ thin | — | — | new-api, one-api |
| closeai | ✅ thin | — | — | one-api |
| openaisb | ✅ thin | — | — | one-api |
| openaimax | ✅ thin | — | — | new-api, one-api |
| ails | ✅ thin | — | — | new-api, one-api |
| api2gpt | ✅ thin | — | — | new-api, one-api |
| aigc2d | ✅ thin | — | — | new-api, one-api |
| fastgpt | ✅ thin | — | — | new-api, one-api |
| tokenpony | ✅ thin | — | — | chats |
| fastrouter | ✅ thin | — | — | TokenHub |
| orcarouter | ✅ thin | — | — | TokenHub |

### General/Custom Channels

Almost all gateway projects have a "custom/OpenAI-compatible/general" channel that allows filling in an arbitrary endpoint. aimux's `OpenAIProvider::with_base_url` is equivalent to this capability.

---

## 3. Providers Missing from aimux

> Update (2026-07-28): OpenAI-compatible LLM chat providers have all been integrated as thin wrappers. The following providers are not integrated, for various reasons:

### Non-OpenAI Chat Completions Protocol (requires its own trait or API format)

| Provider | Type | Reason for not integrating |
|------|------|---------|
| milvus / qdrant / pg_vector / s3_vectors | vector database | Not an LLM API; it is vector storage/retrieval |
| clip / fastembed / tei / text_embeddings_inference | embedding inference | Not a chat API; requires an EmbeddingModel trait implementation |
| jina / nomic / mixedbread | embedding/reranking | Not a chat API; requires an EmbeddingModel/RerankingModel implementation |
| recraft / ideogram / stability_ai / segmind / runware | image generation | Not a chat API; requires an ImageModel trait implementation |
| meshy / tripo3d | 3D generation | Not a chat API |
| runwayml / sora / vidu / jimeng / midjourney | video generation | Not a chat API; requires a VideoModel trait implementation |
| flux | image generation | Not a chat API |
| suno | music generation | Not a chat API; no corresponding trait |
| murf / playai / speechify / inworld | speech synthesis | Not a chat API; requires a SpeechModel trait implementation |
| aws_polly | speech synthesis | AWS SigV4 authentication; requires SpeechModel |
| nvidia_riva | speech | Not OpenAI-compatible |
| soniox | speech transcription | WebSocket protocol; requires TranscriptionModel |
| doubaoaudio / mokaai | audio | Not a chat API |
| bing / deepl / dify / slack / doc2x | non-LLM service | Search/translation/platform/messaging; not an LLM API |
| streamlake / antling / sangforaicp / skylark | non-LLM service | Video streaming/unknown/enterprise platform/unknown |

### Special Authentication (cannot simply use API key + Bearer)

| Provider | Authentication method | Reason for not integrating |
|------|---------|---------|
| watsonx (ibm) | IBM IAM token | Requires an independent authentication implementation |
| sagemaker (aws) | AWS SigV4 | Requires an independent authentication implementation |
| sap | SAP auth | Requires an independent authentication implementation |
| oci (oracle) | Oracle auth | Requires an independent authentication implementation |
| snowflake | Snowflake auth | Requires an independent authentication implementation |
| bedrock_mantle | AWS auth | Requires an independent authentication implementation |

### Need to Upgrade to Native Protocol Implementation (current thin wrapper may not handle provider-specific fields)

| Provider | Current status | What needs to be done |
|------|---------|---------|
| baidu (Wenxin) | ✅ thin (OpenAI-compatible endpoint) | Wenxin has a custom protocol (ERNIE-Bot); requires a native implementation to cover full capabilities |
| tencent (Hunyuan) | ✅ thin | Hunyuan has its own signing mechanism; requires a native implementation |
| xunfei (Xunfei) | ✅ thin | Xunfei uses the WebSocket protocol; requires an independent implementation |
| alibaba | ✅ thin | The reasoning_content field is dropped; reasoning fields need to be handled |
| groq | ✅ thin | top_k is bypassed by coincidence; needs a configuration descriptor to mark it as unsupported |
| all thin wrappers | ✅ thin | Add a configuration descriptor structure per RFC-0002 to handle the differences of each provider |

### Whether to Include Programming Subscriptions (pending)

The coding plan category (chatgpt subscription, copilot, claude code subscription, etc.) is very popular in 2026, but carries ToS risks and unstable protocols. As a unified service integration layer, whether aimux should include this kind of integration requires a decision. If it is to be included, refer to the implementations of axonhub and claude-code-router.

---

## 4. Points to Update or Handle During Implementation

### 1. Thin Wrapper Refactoring (recorded in RFC-0002)

13 OpenAI-compatible thin wrappers need a configuration descriptor structure added so that each provider's differences (reasoning fields, capability flags, usage statistics methods) can be expressed. This is a prerequisite for rolling out providers at scale.

### 2. Special Protocols of Domestic Providers

Baidu, Xunfei, and Zhipu do not fully follow the OpenAI-compatible protocol and require native implementations:
- Baidu Wenxin: custom protocol, has the ERNIE-Bot series
- Xunfei Spark: WebSocket protocol, special signing authentication
- Zhipu GLM: has its own protocol and also an OpenAI-compatible endpoint

### 3. Different Integration Methods for Local Inference Providers

ollama, llama.cpp, and lmstudio are local services without HTTPS or keys, and their integration method differs from cloud providers:
- ollama: HTTP localhost, no authentication
- llama.cpp: local process, uses GGUF files
- lmstudio: local HTTP, OpenAI-compatible

This affects the authentication and URL construction logic of the Provider trait.

### 4. Routing Semantics of Gateway-Type Providers

openrouter and vercel gateway are not ordinary providers — they internally route to other providers. Users may want to specify "route to anthropic/claude via openrouter". This affects how model_id is parsed (it may contain a slash-separated provider prefix).

### 5. Test Coverage

Per the cassette scheme of RFC-0003, each time a provider is added:
- Find the corresponding provider's cassette from rig's recordings (rig covers 16 providers)
- For those not covered by rig, record them yourself with llmtape
- Add them to the unified contract tests

### 6. Verification of Existing Implementations

The existing 11 native implementations need to be verified for correctness with cassette tests:
- openai, anthropic, google, bedrock, vertex, azure, cohere, mistral, xai, deepseek, anthropic_aws
- These implementations have a large amount of code (500-1500 lines per provider) and may contain undiscovered parsing bugs
- Cassette replay can uncover "the return format changed but the code did not keep up" issues

### 7. Specifics of coding plan Integration (if included)

OAuth authentication flows, account pool management, token refresh, and account-ban risk handling — these all differ from ordinary API key authentication. If it is decided to include them, a separate authentication abstraction layer needs to be designed.

---

## 5. High-User-Volume Coding Agents and Forwarding Services

> Previously only SDKs and gateways were scanned, missing the higher-volume coding agent tools and their accompanying forwarding/switching services.
> These projects are massive in scale (several with 100k+ stars) and are the actual source of coding plan integrations.

### coding agent (terminal/IDE type)

| Project | ★ | Language | Positioning |
|------|:---:|:---:|------|
| openai/codex | 101k | Rust | OpenAI's official coding agent, runs in the terminal |
| anthropics/claude-code | 139k | — | Anthropic's official coding agent, runs in the terminal |
| anomalyco/opencode | 190k | TypeScript | Open-source coding agent (unofficial)|
| earendil-works/pi | 79k | TypeScript | AI agent toolkit: unified LLM API + agent loop + TUI + coding CLI |
| google-gemini/gemini-cli | 106k | TypeScript | Google's official Gemini terminal agent |
| cline/cline | 65k | TypeScript | Autonomous coding agent, SDK/IDE/CLI |
| Aider-AI/aider | 48k | Python | Terminal AI pair programming |
| continuedev/continue | 35k | TypeScript | Open-source coding agent (IDE)|
| RooCodeInc/Roo-Code | 24k | TypeScript | Multi-agent team inside the editor |
| opencode-ai/opencode | 14k | Go | Terminal coding agent (another opencode)|

### Forwarding/Switching/Proxy Services (letting coding agents integrate any provider)

| Project | ★ | Language | Positioning |
|------|:---:|:---:|------|
| farion1231/cc-switch | 122k | Rust+TS | Cross-platform desktop assistant: unified management + provider switching for Claude Code/Codex/OpenCode/OpenClaw/Grok Build/Hermes Agent |
| musistudio/claude-code-router | 36k | TypeScript | Routes Claude Code to any model/provider |
| lidge-jun/opencodex | 5.2k | TypeScript | General provider proxy for Codex CLI + Claude Code |
| XueshiQiao/CCSwitcher | 160 | — | One-click Claude Code account switching |
| liuzhengming/ccswitch-deepseek | 296 | — | ccswitch forwarding to DeepSeek |
| nicremo/ccs | 11 | — | Switch Claude Code to MiniMax/Kimi/GLM/DeepSeek/Qwen |
| glidea/claude-worker-proxy | 274 | — | Claude Code proxy on Cloudflare Worker |

### coding agent Ecosystem Periphery

| Project | ★ | Positioning |
|------|:---:|------|
| awesome-opencode | 9.2k | opencode plugin/theme/agent resource collection |
| alvinunreal/oh-my-opencode-slim | 7.4k | opencode multi-agent suite, mixing any models |
| pinchbench/skill | 1.3k | LLM benchmark for the OpenClaw coding agent |
| kenryu42/cc-safety-net | 1.5k | coding agent CLI safety net (intercepts dangerous commands)|
| agent-of-empires/agent-of-empires | 2.9k | Unified management TUI+Web for multiple agents (Claude Code/OpenCode/Codex/Gemini/Pi/Copilot/Factory Droid) |

### Detailed Provider Inventory of coding agents and Forwarding Services

**codex** (OpenAI official, Rust): 4 built-in — OpenAI, Amazon Bedrock, Ollama, LM Studio. Only uses the OpenAI Responses API, with no cross-protocol conversion. Supports ChatGPT subscription OAuth.

**opencode** (190k stars, TS): Delegates to Vercel AI SDK, ~20 built-in — anthropic, openai, google, google-vertex, github-copilot, amazon-bedrock, azure, openrouter, mistral, gitlab, xai, groq, deepinfra, cerebras, cohere, togetherai, perplexity, vercel, alibaba, venice, bedrock/mantle. Supports GitHub Copilot OAuth.

**pi** (79k stars, TS): Self-implements 10 API adapters, 37 built-in providers — amazon-bedrock, ant-ling, anthropic, azure-openai-responses, cerebras, cloudflare-ai-gateway, cloudflare-workers-ai, deepseek, fireworks, github-copilot, google, google-vertex, groq, huggingface, kimi-coding, minimax, minimax-cn, mistral, moonshotai, moonshotai-cn, nvidia, openai, openai-codex, opencode, opencode-go, openrouter, qwen-token-plan, qwen-token-plan-cn, radius, together, vercel-ai-gateway, xai, xiaomi, xiaomi-token-plan(ams/cn/sgp), zai, zai-coding-cn. Supports three OAuth types: Codex subscription/GitHub Copilot/radius.

**cline** (65k stars, TS): ~55+ providers — anthropic, claude-code, cline, cline-pass, openai-compatible, openai-native, openai-codex, openai-codex-cli, opencode, bedrock, vertex, gemini, ollama, lmstudio, deepseek, xai, together, fireworks, groq, poolside, cerebras, sambanova, nebius, baseten, requesty, litellm, huggingface, vercel-ai-gateway, v0, aihubmix, hicap, nousResearch, huawei-cloud-maas, wandb, xiaomi, tencent-tokenhub, kilo, zai, zai-coding-plan, qwen, qwen-code, doubao, mistral, moonshot, asksage, minimax, dify, oca, sapaicore, openrouter. Supports four OAuth types: Claude Code/Cline/Codex/opencode.

**continue** (35k stars, TS): 66 provider classes — including Anthropic, OpenAI, Gemini, Bedrock, Azure, VertexAI, Cohere, Mistral, Deepseek, xAI, MiniMax, Groq, OpenRouter, Together, Fireworks, Cerebras, Cloudflare, DeepInfra, HuggingFace, LlamaCpp, LlamaStack, Llamafile, LMStudio, Ollama, Nvidia, Novita, Msty, Mimo, Moonshot, Nebius, NCompass, Nous, OVHcloud, Replicate, Relace, SambaNova, SageMaker, Scaleway, SiliconFlow, TARS, Tensorix, TextGenWebUI, Venice, Vllm, WatsonX, zAI, CometAPI, ClawRouter, Docker, Flowise, FunctionNetwork, Inception, Jina, Kindo, Lemonade, AskSage, etc.

**Roo-Code** (24k stars, TS): 30+ handlers — Anthropic, AwsBedrock, DeepSeek, Moonshot, Gemini, LiteLLM, LmStudio, Mistral, OpenAiCodex, OpenAiNative, OpenAi, OpenAICompatible, OpenRouter, Poe, QwenCode, Requesty, SambaNova, Unbound, Vertex, AnthropicVertex, VsCodeLm, XAI, ZAi, Fireworks, VercelAiGateway, MiniMax, Baseten, NativeOllama, FakeAI. Internal canonical format = Anthropic Messages. Supports Codex subscription OAuth.

**opencode-ai** (14k stars, Go): 11 — Copilot, Anthropic, OpenAI, Gemini, Bedrock, GROQ, Azure, VertexAI, OpenRouter, XAI, Local. Each provider has an independent Go SDK client. Supports GitHub Copilot token exchange.

**aider** (48k stars, Python): Delegates to litellm, supports all 134 of litellm's providers. No OAuth.

**opencodex** (5.2k stars, TS, forwarding service): 60 provider entries — including OpenAI (Codex subscription pool), Anthropic (Claude subscription), xAI/Grok, Kimi, Kiro, Google Antigravity, Cursor, GitHub Copilot, DeepSeek, Moonshot, Z.AI, Zhipu, Qwen, Alibaba, Tencent, Baidu, MiniMax, SiliconFlow, Groq, Cerebras, Together, Fireworks, FirePass, HuggingFace, NVIDIA NIM, Venice, NanoGPT, Synthetic, Mistral, OpenRouter, OrcaRouter, BizRouter, Parallel, ZenMux, Vercel AI Gateway, Cloudflare AI Gateway/Workers AI, GitLab Duo, Kilo, Umans, Neuralwatt, opencode-go/zen/free, Ollama, vLLM, LM Studio, LiteLLM, etc. Most complete protocol conversion (internal intermediate representation + bidirectional adapters).

**cc-switch** (122k stars, Rust+Tauri): 80+ presets — including AiHubMix, OpenRouter, TheRouter, Novita AI, DMXAPI, CrazyRouter, NewAPI, APIKEY.FUN, SubRouter, DeepSeek, Zhipu GLM, Bailian, Baidu Qianfan, StepFun, ModelScope, Longcat, MiniMax, BaiLing, Xiaomi MiMo, DouBaoSeed/BytePlus, Tencent, Kimi/Kimi For Coding, PackyCode, ZetaAPI, APINebula, AICodeMirror, PatewayAI, FennoAI, RunAPI, Unity2.ai, Shengsuanyun, AIGoCode, AICoding, Code0, TeamoRouter, ClaudeCN, ClaudeAPI, CCSub, SSSAiCode, Micu, RightCode, ETok.ai, Cubence, SudoCode, Amux, CherryIN, RelaxyCode, E-FlowCode, PIPELLM, NekoCode, AtlasCloud, Compshare, KAT-Coder, Nvidia, Together AI, Nous Research, Claude Official, OpenAI Official, Google Official, Grok Official, Codex, GitHub Copilot, AWS Bedrock, Azure OpenAI, Gemini Native, OpenCode Go, custom gateway. No protocol conversion, only configuration switching + model name mapping.

**claude-code-router** (36k stars, TS): 20+ presets — anthropic, openai, deepseek, gemini, bailian, claudeapi, code0, fenno, kimi-coding, minimax, mistral, moonshot, nvidia, openrouter, qiniu-ai, runapi, siliconflow, teamorouter, unity2, zai-global-coding, zai-global-general, zhipu-cn-coding, zhipu-cn-general. The core does not perform protocol conversion, relying on route scripts.

### Newly Discovered Providers (previously omitted from the inventory)

The following providers appear in coding agents/forwarding services and were not previously recorded in the inventory:

| Provider | Source | Type |
|------|------|------|
| ant-ling | pi | LLM |
| radius | pi | LLM (OAuth) |
| kimi-coding | pi/opencodex | coding plan |
| qwen-token-plan / qwen-code | pi/cline | coding plan |
| zai-coding-cn / zai-coding-plan | pi/cline | coding plan |
| poolside | cline | LLM |
| hicap | cline | LLM |
| asksage | cline/continue | LLM |
| dify | cline | platform |
| oca | cline | LLM |
| sapaicore | cline | LLM |
| huawei-cloud-maas | cline | LLM |
| msty | continue | local |
| tars | continue | LLM |
| tensorix | continue | LLM |
| relace | continue | LLM |
| llamastack | continue | local |
| kindo | continue | LLM |
| lemonade | continue | LLM |
| flowise | continue | platform |
| functionnetwork | continue | LLM |
| poe | Roo-Code | aggregation |
| unbound | Roo-Code | LLM |
| firepass | opencodex | LLM |
| orcarouter | opencodex | proxy |
| bizrouter | opencodex | proxy |
| parallel | opencodex | proxy |
| zenmux | opencodex | proxy |
| umans | opencodex | LLM |
| neuralwatt | opencodex/axonhub | LLM |
| opencode-free | opencodex | free |
| kiro | opencodex/manifest | coding plan |
| google antigravity | opencodex/axonhub | coding plan |
| cursor | opencodex | coding plan |
| packycode | cc-switch | proxy |
| zetaapi | cc-switch | proxy |
| apinebula | cc-switch | proxy |
| aicodemirror | cc-switch | proxy |
| patewayai | cc-switch | proxy |
| fennoai | cc-switch | proxy |
| runapi | cc-switch/claude-code-router | proxy |
| unity2.ai | cc-switch/claude-code-router | proxy |
| shengsuanyun | cc-switch | proxy |
| aigocode | cc-switch | proxy |
| aicoding | cc-switch | proxy |
| code0 | cc-switch/claude-code-router | proxy |
| teamorouter | cc-switch/claude-code-router | proxy |
| claudecn | cc-switch | proxy |
| ccsub | cc-switch | proxy |
| sssaicode | cc-switch | proxy |
| micu | cc-switch | proxy |
| rightcode | cc-switch | proxy |
| etok.ai | cc-switch | proxy |
| cubence | cc-switch | proxy |
| sudocode | cc-switch | proxy |
| amux | cc-switch | proxy |
| cherryin | cc-switch | proxy |
| relaxycode | cc-switch | proxy |
| e-flowcode | cc-switch | proxy |
| pipllm | cc-switch | proxy |
| nekocode | cc-switch | proxy |
| compshare | cc-switch | proxy |
| kat-coder | cc-switch | proxy |
| dmxapi | cc-switch | proxy |
| crazyrouter | cc-switch | proxy |
| subrouter | cc-switch | proxy |
| apikey.fun | cc-switch | proxy |
| therouter | cc-switch | proxy |
| clawrouter | continue | proxy |

> The vast majority of the newly added providers above are domestic coding agent proxy/relay services, which emerged in large numbers in 2026. For protocol conversion logic, see [0005-protocol-conversion.md](0005-protocol-conversion.md).

### Implications for aimux

1. **cc-switch is written in Rust** (a Tauri desktop application) — it needs to uniformly manage the authentication and switching of multiple providers, which is exactly what aimux's provider abstraction can provide.
2. **Both opencode and pi come with a "unified LLM API" layer** — pi explicitly states "unified LLM API", which directly overlaps with aimux's positioning.
3. **Forwarding services (claude-code-router/opencodex) are essentially micro-gateways** — forwarding coding agent requests to any provider, of the same kind as the gateway projects scanned earlier.
4. **These projects have far more users than SDK projects** (opencode 190k vs rig 8k) — if aimux wants to be widely used, the coding agent ecosystem is a larger market than the SDK ecosystem.

aimux currently does not support any coding agent integration. Whether to provide support for such scenarios (OAuth authentication, subscription quota management, provider switching) requires a decision.

---

## 6. Data Source Notes

The provider inventory in this document comes from the following scans (2026-07-27):

- **Rust competitors**: rig(28), rust-genai(31), langchain-rust(5), kalosm, swiftide(9), graniet-llm(15), rllm(7), edgequake-llm, llm-connector(8), ai.rs, litellm-rust, unia(14), multi-llm, llmrust, rust_ai_sdk, llm-chain
- **Python ecosystem**: litellm(134), langchain, llama_index(99), dspy, haystack, guidance, pydantic-ai(14), outlines, instructor(14), aisuite(28), textgrad, AutoGPT, OpenHands, autogen, crewAI, mirascope
- **Other languages**: mastra, langchainjs, LlamaIndexTS, eino (interfaces only), langchaingo, langchain4j, spring-ai, semantic-kernel, semantic-kernel-java, dotnet-extensions, LangChain-csharp, langchain-swift
- **Gateways**: new-api(54 channels), one-api(38 adapters), portkey-gateway(72), bifrost(29), coai(18), manifest(33), higress(36), one-hub(41), simple-one-api(18), uni-api(12 adapters), axonhub(20), TokenHub(35+ direct/100+ proxies), aiproxy(37), chats(22), otari (configuration-driven), ferro-ai-gateway(29), OpenGateLLM(5), llmgateway(33), APIPark(35), envoy-ai-gateway(7), claude-code-router(20)
- **coding agents and forwarding services**: openai/codex(4 built-in), anthropics/claude-code (docs only), anomalyco/opencode(20), earendil-works/pi(37), google-gemini/gemini-cli (Google only), cline(55+), Aider-AI/aider (litellm passthrough), continuedev/continue(66), RooCodeInc/Roo-Code(30+), opencode-ai/opencode(11), farion1231/cc-switch(80+), musistudio/claude-code-router(20+), lidge-jun/opencodex(60), XueshiQiao/CCSwitcher (Claude account pool), glidea/claude-worker-proxy (arbitrary OpenAI/Gemini), liuzhengming/ccswitch-deepseek (DeepSeek), nicremo/ccs (5 domestic), agent-of-empires(2.9k), oh-my-opencode-slim(7.4k), pinchbench(1.3k), cc-safety-net(1.5k)
- **Protocol conversion logic**: see [0005-protocol-conversion.md](0005-protocol-conversion.md), covering SDK adapter layer design, gateway cross-protocol mutual conversion, and coding agent/forwarding service protocol conversion
