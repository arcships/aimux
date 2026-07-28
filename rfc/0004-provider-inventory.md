# 厂商清单与实现现状

## 一、aimux 当前已实现的厂商

**共 172 个厂商模块**（截至 2026-07-28）。

### 原生协议实现（有独立 model/convert，处理厂商特有差异）

| 厂商 | 能力 | 说明 |
|------|------|------|
| openai | 文本、流式、工具、嵌入、图像、语音、转写、文件 | 参考实现，655 行 model + 1223 行 convert，含 Responses API |
| anthropic | 文本、流式、工具、文件、缓存控制 | 1438 行 convert，含 prompt caching、schema 净化 |
| anthropic_aws | 文本、流式 | Anthropic 走 AWS Bedrock 侧通道 |
| google | 文本、流式、嵌入、图像、视频、文件 | 995 行 convert，Gemini 原生协议 |
| bedrock | 文本、流式、嵌入、图像、重排序 | 625 行 convert，Converse API + SigV4 签名 + 事件流 |
| vertex | 文本、流式、嵌入、图像、转写、视频 | Google Vertex AI，GCP 认证 |
| azure | 文本、流式、Responses API | Azure OpenAI，多种认证方式 |
| cohere | 文本、流式、嵌入、重排序 | Cohere 原生协议 |
| mistral | 文本、流式、嵌入 | Mistral 原生协议 |
| xai | 文本、流式、工具、Responses API | 636 行 convert，处理推理内容、引用、搜索参数 |
| deepseek | 文本、流式、推理 | 600 行，独立处理 reasoning_content 和 thinking 字段 |

### OpenAI 兼容薄封装（只改网址和环境变量，无定制点）

约 195 个。包括早期手写的和批量生成的：

groq、fireworks、togetherai、perplexity、moonshotai、cerebras、openrouter（含 Responses API）、ollama、copilot、llamafile、mistralrs、doubleword、zai、github、siliconflow、lmstudio、sambanova、alibaba、baseten、bytedance、deepinfra、huggingface、vercel、ai21、ai302、aibadgr、aigc2d、aihubmix、ails、aiml、albert、antling、anyscale、api2d、api2gpt、apiserpent、atlascloud、baichuan、baidu、bedrock_mantle、bigmodel、bing、byteplus、bytez、canopywave、chatgpt、clarifai、cline_pass、closeai、codestral、cometapi、commandcode、compactifai、coze、cybertron、databricks、datarobot、deepbricks、deepl、dify、doc2x、docker_model_runner、doubaoaudio、embercloud、fastcrw、fastembed、fastgpt、fastrouter、featherless_ai、flux、friendliai、galadriel、gaudi、gdc、gigachat、gonka24、gradient_ai、helicone、heroku、hosted_vllm、hyperbolic、ideogram、inception、inference_net、infinity、inworld、jimeng、jina、jlama、kilo、kiro、kluster_ai、krutrim、lambda_ai、lemonfox_ai、lingyiwanwu、litellm_proxy、llamacpp、local、localai、longcat、matterai、meshy、meta_llama、midjourney、milvus、minimax、mira、mixedbread、mlx、modal、modelscope、mokaai、morph、murf、nanogpt、ncompass、nebius、nextbit、nlp_cloud、nomic、nous_research、novita、nscale、nvidia_nim、nvidia_riva、oci、ohmygpt、ollama_cloud、omlx、onnx、oobabooba、openaimax、openaisb、opencode_go、opencode_zen、openvino、orcarouter、ovhcloud、parasail、perfxcloud、petals、pg_vector、pioneer、playai、portkey、predibase、qdrant、qihoo360、qiniu_ai、recraft、reka_ai、requesty、reve、runware、runwayml、s3_vectors、sagemaker、sakana、sangforaicp、sap、sarvam、scaleway、scx_ai、segmind、sglang、skylark、slack、snowflake、soniox、sora、speechify、stability_ai、stepfun、streamlake、submodel、suno、tei、tencent、text_embeddings_inference、tokenpony、tripo3d、tundra、upstage、v0、vidu、vllm、wafer、watsonx、xiaomimimo、xinference 等。

### 非文本能力专用厂商

| 厂商 | 能力 |
|------|------|
| voyage | 嵌入、重排序 |
| cartesia | 语音、转写 |
| elevenlabs | 语音、转写 |
| hume | 语音 |
| lmnt | 语音 |
| assemblyai | 转写 |
| deepgram | 转写 |
| fal | 图像、转写、视频 |
| gladia | 转写 |
| revai | 转写 |
| black_forest_labs | 图像 |
| luma | 图像 |
| prodia | 图像、视频 |
| replicate | 图像、视频 |
| klingai | 视频 |

### 通用封装

| 模块 | 说明 |
|------|------|
| open_responses | 通用 Responses API 封装，1238 行 |

### 已知问题

薄封装仍无配置描述结构（RFC-0002 已记录）。深度求索的推理字段、阿里通义的 reasoning_content、Groq 的 top_k 限制等差异仍被丢失。这是下一步要做的。

**合计：161 个厂商模块。**

> 其中 11 个原生协议实现 + 1 个通用封装 + 15 个非文本能力专用 + 134 个 OpenAI 兼容薄封装。
> 注：向量数据库（milvus/qdrant/pg_vector/s3_vectors）、嵌入专用（jina/nomic/mixedbread/clip/fastembed/tei）、
> 图像/视频/音乐生成（recraft/ideogram/stability_ai/meshy/tripo3d/runwayml/sora/vidu/jimeng/midjourney/flux/suno）、
> 语音/转写（murf/playai/speechify/inworld/aws_polly/nvidia_riva/soniox/doubaoaudio/mokaai）、
> 非LLM服务（bing/deepl/dify/slack/doc2x/streamlake/antling/sangforaicp/skylark）、
> 特殊认证（watsonx/sagemaker/sap/oci/snowflake/bedrock_mantle）共 49 家未接入——
> 它们不是 OpenAI Chat Completions 兼容的，需要各自的 trait 或认证方式。

---

## 二、所有参考项目的厂商总清单

> 合并去重四个扫描结果（Rust 竞品 / Python 生态 / 其他语言 / 网关）中出现过的所有厂商。
> "网关覆盖"列标注支持该厂商的网关项目（不穷举，列代表性的）。

### 主流大模型厂商

| 厂商 | aimux | rig | litellm | 网关覆盖 | 其他主要项目 |
|------|:---:|:---:|:---:|:---:|------|
| openai | ✅原生 | ✅ | ✅ | 全员 | 几乎所有项目 |
| anthropic | ✅原生 | ✅ | ✅ | 全员 | 几乎所有项目 |
| google (gemini) | ✅原生 | ✅ | ✅ | 全员 | 几乎所有项目 |
| azure (openai) | ✅原生 | ✅ | ✅ | 全员 | 几乎所有项目 |
| bedrock (aws) | ✅原生 | ✅ | ✅ | 全员 | 几乎所有项目 |
| vertex (gcp) | ✅原生 | ✅ | ✅ | 全员 | langchain4j, spring-ai |
| mistral | ✅原生 | ✅ | ✅ | 全员 | 几乎所有项目 |
| cohere | ✅原生 | ✅ | ✅ | 全员 | 几乎所有项目 |
| xai (grok) | ✅原生 | ✅ | ✅ | 全员 | genai, pydantic-ai, instructor |
| deepseek | ✅原生 | ✅ | ✅ | 全员 | 几乎所有项目 |

### OpenAI 兼容云厂商

| 厂商 | aimux | rig | litellm | 网关覆盖 | 其他 |
|------|:---:|:---:|:---:|:---:|------|
| groq | ✅薄 | ✅ | ✅ | portkey, bifrost, higress, one-hub, axonhub, ferro | pydantic-ai, instructor, aisuite, llama_index |
| fireworks | ✅薄 | — | ✅ | portkey, one-hub, higress, APIPark, ferro | langchain, llama_index, langchain4j |
| together | ✅薄 | ✅ | ✅ | portkey, higress, ferro | langchain, llama_index, langchain4j, aisuite |
| perplexity | ✅薄 | ✅ | ✅ | portkey, ferro | langchain, llama_index, langchainjs |
| moonshot (kimi) | ✅薄 | ✅ | ✅ | 全员国产网关 | llama_index, genai |
| cerebras | ✅薄 | — | ✅ | portkey, bifrost, axonhub, ferro | pydantic-ai, instructor, aisuite, llama_index |
| openrouter | ✅薄 | ✅ | ✅ | 几乎所有网关 | langchain, llama_index, genai, aisuite |
| huggingface | ✅薄 | ✅ | ✅ | portkey, one-hub, bifrost, APIPark, ferro | langchain, llama_index, langchain4j, semantic-kernel |
| deepinfra | ✅薄 | — | ✅ | portkey, llmgateway | llama_index, langchain |
| ovhcloud | ✅薄 | — | ✅ | portkey, llmgateway | llama_index, langchain4j |
| replicate | ✅图像 | — | ✅ | one-hub, TokenHub, ferro | langchain, llama_index |
| novita | ✅薄 | — | ✅ | portkey, one-api, TokenHub, APIPark, ferro, llmgateway | llama_index |
| nebius | ✅薄 | — | ✅ | portkey, bifrost, ferro, llmgateway | genai, aisuite, llama_index |
| hyperbolic | ✅薄 | ✅ | ✅ | portkey | — |
| minimax | ✅薄 | ✅ | ✅ | 全员国产网关, portkey, claude-code-router, llmgateway | genai, llama_index |
| mistralrs | ✅薄 | ✅本地 | — | — | — |
| llamafile | ✅薄 | ✅本地 | ✅ | — | langchaingo |
| doubleword | ✅薄 | ✅ | — | — | — |
| mira | ✅薄 | ✅ | — | — | — |
| xiaomimimo (小米) | ✅薄 | ✅ | — | manifest, llmgateway, chats, TokenHub | — |
| zai (智谱/z.ai) | ✅薄 | ✅ | ✅ | 全员国产网关, portkey, claude-code-router, llmgateway | genai |
| github (models) | ✅薄 | — | ✅ | higress, one-hub, chats, TokenHub | langchain4j, llama_index |
| github_copilot | ✅薄 | ✅ | ✅ | axonhub, manifest, TokenHub, bifrost | genai |
| chatgpt (订阅) | ✅薄 | ✅ | ✅ | — | — |
| ai21 | ✅薄 | — | ✅ | portkey, ferro | langchain, llama_index |
| anyscale | ✅薄 | — | ✅ | portkey | langchain |
| sambanova | ✅薄 | — | ✅ | portkey, ferro | llama_index |
| predibase | ✅薄 | — | ✅ | portkey | — |
| triton (nvidia) | ✅薄 | — | ✅ | portkey, higress | — |
| databricks | ✅薄 | — | ✅ | portkey, ferro, TokenHub | langchain, llama_index |
| sagemaker (aws) | ❌ | — | ✅ | portkey | langchain, llm-chain |
| watsonx (ibm) | ❌ | — | ✅ | — | langchain, langchain4j, langchaingo |
| scaleway | ✅薄 | — | ✅ | TokenHub | — |
| snowflake | ❌ | — | ✅ | TokenHub | — |
| sap | ❌ | — | ✅ | — | — |
| oci (oracle) | ❌ | — | ✅ | TokenHub, portkey(oracle) | llama_index |
| nlp_cloud | ✅薄 | — | ✅ | — | langchain |
| friendliai | ✅薄 | — | ✅ | — | llama_index |
| clarifai | ✅薄 | — | ✅ | — | langchain, llama_index |
| gigachat (三星) | ✅薄 | — | ✅ | — | — |
| codestral | ✅薄 | — | ✅ | — | — |
| morph | ✅薄 | — | ✅ | TokenHub | — |
| v0 | ✅薄 | — | ✅ | — | — |
| aiml | ✅薄 | — | ✅ | TokenHub | — |
| heroku | ✅薄 | — | ✅ | — | — |
| hosted_vllm | ✅薄 | — | ✅ | — | — |
| nvidia_nim | ✅薄 | — | ✅ | manifest, ferro, APIPark, TokenHub | — |
| nscale | ✅薄 | — | ✅ | portkey, llmgateway | — |
| lambda_ai | ✅薄 | — | ✅ | portkey | — |
| databricks | ✅薄 | — | ✅ | portkey, ferro | langchain |
| petals | ✅薄 | — | ✅ | — | — |
| oobabooba | ✅薄 | — | ✅ | — | — |
| inception | ✅薄 | — | ✅ | TokenHub | — |
| galadriel | ✅薄 | — | ✅ | — | — |
| gdc | ✅薄 | — | ✅ | — | — |
| datarobot | ✅薄 | — | ✅ | — | — |
| infinity | ✅薄 | — | ✅ | — | — |
| kluster-ai | ✅薄 | — | — | portkey | — |
| featherless-ai | ✅薄 | — | ✅ | portkey | llama_index |
| krutrim | ✅薄 | — | — | portkey | — |
| bytez | ✅薄 | — | — | portkey | — |
| upstage | ✅薄 | — | — | portkey, APIPark, langchain4j | — |
| deepbricks | ✅薄 | — | — | portkey | — |
| lemonfox-ai | ✅薄 | — | — | portkey | — |
| inference-net | ✅薄 | — | — | portkey, llmgateway | — |
| 302ai | ✅薄 | — | — | portkey, TokenHub | — |
| cometapi | ✅薄 | — | ✅ | portkey, TokenHub | — |
| matterai | ✅薄 | — | — | portkey | — |
| meshy | ❌ | — | — | portkey | — |
| nextbit | ✅薄 | — | — | portkey | — |
| tripo3d | ❌ | — | — | portkey | — |
| modal | ✅薄 | — | — | portkey | — |
| aibadgr | ✅薄 | — | — | portkey, TokenHub, chats | — |
| ncompass | ✅薄 | — | — | portkey | — |
| reka-ai | ✅薄 | — | ✅ | portkey | llama_index |
| stability-ai | ❌ | — | ✅ | portkey, one-hub, spring-ai | — |
| segmind | ❌ | — | — | portkey | — |
| monsterapi | ✅薄 | — | — | portkey | llama_index |
| sgl (sglang) | ✅薄 | — | ✅ | bifrost | guidance, outlines, llama_index |
| parasail | ✅薄 | — | — | bifrost | — |
| wafer | ✅薄 | — | — | bifrost, axonhub | — |
| runway | ❌ | — | ✅(runwayml) | bifrost | — |
| runware | ❌ | — | — | bifrost | — |
| sarvam | ✅薄 | — | ✅ | bifrost, claude-code-router, TokenHub | — |
| opencode-go | ✅薄 | — | — | bifrost, manifest, axonhub | genai |
| opencode-zen | ✅薄 | — | — | bifrost, manifest | — |
| bedrock_mantle | ❌ | — | ✅ | bifrost, llmgateway | — |
| vllm | ✅薄 | — | ✅ | bifrost, higress | guidance, outlines, llama_index |
| nous (nousresearch) | ✅薄 | — | — | manifest | — |
| byteplus | ✅薄 | — | — | manifest | — |
| kiro | ✅薄 | — | — | manifest | — |
| pioneer | ✅薄 | — | — | manifest | — |
| kilo | ✅薄 | — | — | manifest, TokenHub | — |
| cline-pass | ✅薄 | — | — | manifest | — |
| commandcode | ✅薄 | — | — | manifest | — |
| copilot (github) | ✅薄 | ✅ | ✅ | manifest | genai |
| longcat | ✅薄 | — | — | higress, axonhub | — |
| yi (零一) | ✅薄 | — | — | higress, one-api, APIPark, TokenHub, chats | llama_index |
| deepl | ❌ | — | — | higress, one-api | — |
| dify | ❌ | — | — | higress, new-api, aiproxy | — |
| stepfun (阶跃) | ✅薄 | — | — | higress, one-api, APIPark, TokenHub | llama_index |
| baichuan | ✅薄 | — | — | higress, one-api, APIPark, TokenHub, coai | — |
| kling (可灵) | ✅视频 | — | — | higress, new-api, one-hub, TokenHub | — |
| triton | ✅薄 | — | ✅ | higress | — |
| perfxcloud | ✅薄 | — | — | APIPark | — |
| doc2x | ❌ | — | — | aiproxy | — |
| sangforaicp | ❌ | — | — | aiproxy | — |
| streamlake | ❌ | — | — | aiproxy | — |
| antling | ❌ | — | — | aiproxy | — |
| text_embeddings_inference | ❌ | — | — | aiproxy | — |
| skylark (云雀) | ❌ | — | — | coai | — |
| bing (new bing) | ❌ | — | — | coai | — |
| slack (slack claude) | ❌ | — | — | coai | — |
| ideogram | ❌ | — | — | one-hub | — |
| flux | ❌ | — | — | one-hub | — |
| suno (音乐) | ❌ | — | — | new-api, one-hub | — |
| midjourney | ❌ | — | — | new-api, one-hub, coai | — |
| sora | ❌ | — | — | new-api | — |
| vidu (视频) | ❌ | — | — | new-api | — |
| jimeng (即梦) | ❌ | — | — | new-api | — |
| doubaoaudio | ❌ | — | — | aiproxy | — |
| mokaai | ❌ | — | — | new-api | — |
| recraft | ❌ | — | ✅ | one-hub, portkey, TokenHub | — |
| sakana | ✅薄 | — | — | llmgateway | — |
| meta | ✅薄 | — | ✅(meta_llama) | manifest, llmgateway, TokenHub | — |
| scx-ai | ✅薄 | — | — | llmgateway | — |
| atlascloud | ✅薄 | — | — | llmgateway, genai | — |
| canopywave | ✅薄 | — | — | llmgateway | — |
| embercloud | ✅薄 | — | — | llmgateway | — |
| tundra | ✅薄 | — | — | llmgateway | — |
| reve | ✅薄 | — | — | llmgateway | — |
| gonka24 | ✅薄 | — | — | llmgateway | — |
| albert | ✅薄 | — | — | OpenGateLLM | — |
| tei (text embeddings inference) | ❌ | — | — | OpenGateLLM, aiproxy | — |
| fastcrw | ✅薄 | — | ✅ | — | — |
| apiserpent | ✅薄 | — | ✅ | — | — |
| modelscope (魔搭) | ✅薄 | — | ✅ | axonhub, TokenHub | llama_index |
| docker_model_runner | ✅薄 | — | ✅ | — | — |
| ollama_cloud | ✅薄 | — | — | bifrost, ferro, manifest | genai |
| aihubmix | ✅薄 | — | — | — | genai |
| bigmodel (智谱) | ✅薄 | — | — | — | genai |
| litellm_proxy | ✅薄 | — | ✅ | — | — |
| compactifai | ✅薄 | — | ✅ | — | — |
| fastembed | ❌ | — | — | — | swiftide(本地) |
| docker_model_runner | ✅薄 | — | ✅ | — | — |

### 国产厂商（单独汇总）

| 厂商 | aimux | rig | litellm | 网关覆盖 | 其他 |
|------|:---:|:---:|:---:|:---:|------|
| alibaba (通义/DashScope/百炼) | ✅薄 | — | ✅(dashscope) | new-api, one-api, simple-one-api, portkey, higress, ferro, APIPark, claude-code-router, llmgateway | genai, swiftide, llm-connector, llama_index |
| baidu (文心/ernie) | ✅薄 | — | — | new-api, one-api, simple-one-api, APIPark, TokenHub, chats | genai, langchaingo, langchain-swift |
| zhipu (智谱/glm) | ✅薄 | ✅(zai) | — | new-api, one-api, simple-one-api, portkey, higress, APIPark, axonhub, claude-code-router, llmgateway, aiproxy | llm-connector, langchain-swift, llama_index, genai |
| tencent (混元/hunyuan) | ✅薄 | — | ✅ | new-api, one-api, simple-one-api, coai, APIPark, chats, TokenHub | llm-connector |
| xunfei (讯飞/spark) | ✅薄 | — | — | new-api, one-api, simple-one-api, higress, APIPark, chats | — |
| bytedance (火山/豆包/volcengine) | ✅薄 | — | ✅(volcengine) | new-api, one-api, simple-one-api, axonhub, APIPark, llmgateway | — |
| baichuan | ✅薄 | — | — | one-api, higress, APIPark, TokenHub, coai | — |
| stepfun (阶跃) | ✅薄 | — | — | one-api, higress, APIPark, TokenHub | llama_index |
| minimax | ✅薄 | ✅ | ✅ | new-api, one-api, simple-one-api, portkey, claude-code-router, llmgateway, chats, TokenHub | genai, llama_index |
| moonshot (月之暗面/kimi) | ✅薄 | ✅ | ✅ | new-api, one-api, portkey, higress, ferro, claude-code-router, llmgateway, APIPark | genai |
| lingyiwanwu (零一/yi) | ✅薄 | — | — | new-api, one-api, higress, APIPark, TokenHub, chats | llama_index |
| 360 (ai360/zhinao) | ✅薄 | — | — | new-api, one-api, coai, APIPark, chats | — |
| coze (扣子) | ✅薄 | — | — | new-api, one-api, aiproxy | — |
| siliconflow (硅基流动) | ✅薄 | — | — | new-api, one-api, APIPark, chats, claude-code-router | llama_index |
| gigachat (三星) | ✅薄 | — | ✅ | — | — |
| qiniu-ai (七牛) | ✅薄 | — | — | claude-code-router | — |
| modelscope (魔搭) | ✅薄 | — | ✅ | axonhub, TokenHub | llama_index |
| longcat | ✅薄 | — | — | higress, axonhub | — |

### 本地推理

| 厂商 | aimux | rig | litellm | 网关覆盖 | 其他 |
|------|:---:|:---:|:---:|:---:|------|
| ollama | ✅薄 | ✅ | ✅ | new-api, one-api, portkey, bifrost, one-hub, higress, ferro, APIPark, chats, TokenHub | 几乎所有 SDK |
| llama.cpp | ✅薄 | ✅本地 | — | — | guidance, outlines, llm-chain, langchain |
| lmstudio | ✅薄 | — | ✅ | APIPark | aisuite, langchain-swift, edgequake |
| vllm | ✅薄 | — | ✅ | bifrost, higress | guidance, outlines, llama_index, edgequake |
| sglang | ✅薄 | — | — | bifrost | guidance, outlines, llama_index |
| xinference | ✅薄 | — | ✅ | new-api, APIPark | llm-connector |
| mistralrs | ✅薄 | ✅本地 | — | — | — |
| omlx / mlx_lm | ✅薄 | — | — | — | genai, edgequake |
| triton | ✅薄 | — | ✅ | higress, portkey | — |
| local (通用本地) | ✅薄 | — | — | — | langchaingo, langchain-swift, llm-chain |
| cybertron | ✅薄 | — | — | — | langchaingo |
| jlama | ✅薄 | — | — | — | langchain4j |
| localai | ✅薄 | — | — | — | langchain4j, llama_index |
| onnx | ✅薄 | — | — | — | semantic-kernel, langchain4j |
| openvino | ✅薄 | — | — | — | llama_index |
| mlx | ✅薄 | — | — | — | outlines, llama_index |
| gaudi | ✅薄 | — | — | — | llama_index |

### 嵌入/重排序专用

| 厂商 | aimux | rig | litellm | 网关覆盖 | 其他 |
|------|:---:|:---:|:---:|:---:|------|
| voyage | ✅ | ✅ | ✅ | portkey, ferro | langchainjs, langchain4j, llama_index |
| jina | ❌ | — | ✅(jina_ai) | portkey, one-api, axonhub, higress, langchain4j, llama_index | edgequake, langchaingo |
| nomic | ❌ | — | — | portkey | langchainjs, langchain4j |
| cohere embedding/rerank | ✅ | ✅ | ✅ | 全员 | 多数项目 |
| fastembed | ❌ | — | — | — | swiftide(本地) |
| mixedbread | ❌ | — | — | — | LlamaIndexTS |
| clip | ❌ | — | — | — | LlamaIndexTS |
| pg_vector | ❌ | — | ✅ | — | — |
| s3_vectors | ❌ | — | ✅ | — | — |
| milvus (向量库) | ❌ | — | ✅ | portkey | — |
| qdrant (向量库) | ❌ | — | — | portkey | — |

### 语音/转写/图像/视频专用

| 厂商 | aimux | 类型 | litellm | 网关覆盖 | 其他 |
|------|:---:|------|:---:|:---:|------|
| elevenlabs | ✅ | 语音 | ✅ | bifrost, llmgateway | spring-ai |
| deepgram | ✅ | 转写 | ✅ | — | aisuite, mastra |
| assemblyai | ✅ | 转写 | — | — | llama_index |
| cartesia | ✅ | 语音 | — | — | — |
| fal | ✅ | 图像/视频 | ✅(fal_ai) | — | — |
| replicate | ✅ | 图像/视频 | ✅ | one-hub, TokenHub, ferro | langchain, llama_index |
| black_forest_labs | ✅ | 图像 | ✅ | — | — |
| luma | ✅ | 图像 | ✅(runwayml?) | — | — |
| prodia | ✅ | 图像/视频 | — | — | — |
| klingai | ✅ | 视频 | — | new-api, higress, one-hub, TokenHub | — |
| stability | ❌ | 图像 | ✅(stability) | portkey, one-hub, spring-ai | — |
| recraft | ❌ | 图像 | ✅ | one-hub, portkey, TokenHub | — |
| runwayml | ❌ | 视频 | ✅(runwayml) | bifrost | — |
| hume | ✅ | 语音 | — | — | — |
| lmnt | ✅ | 语音 | — | — | — |
| gladia | ✅ | 转写 | — | — | — |
| revai | ✅ | 转写 | — | — | — |
| aws_polly | ❌ | 语音 | ✅ | — | — |
| nvidia_riva | ❌ | 语音 | ✅ | — | — |
| soniox | ❌ | 转写 | ✅ | — | — |
| midjourney | ❌ | 图像 | — | new-api, one-hub, coai | — |
| sora | ❌ | 视频 | — | new-api | — |
| vidu | ❌ | 视频 | — | new-api | — |
| jimeng (即梦) | ❌ | 图像 | — | new-api | — |
| ideogram | ❌ | 图像 | — | one-hub | — |
| flux | ❌ | 图像 | — | one-hub | — |
| suno | ❌ | 音乐 | — | new-api, one-hub | — |
| meshy | ❌ | 3D | — | portkey | — |
| tripo3d | ❌ | 3D | — | portkey | — |
| segmind | ❌ | 图像 | — | portkey | — |
| runware | ❌ | 图像 | — | bifrost | — |
| sarvam | ✅薄 | 语音 | ✅ | bifrost, claude-code-router, TokenHub | — |
| murf | ❌ | 语音 | — | mastra | — |
| playai | ❌ | 语音 | — | mastra | — |
| speechify | ❌ | 语音 | — | mastra | — |
| inworld | ❌ | 语音 | — | mastra | — |

### 编程订阅转 API（coding plan）

2026 年兴起的一类，把编程工具订阅额度转成 API。OAuth 认证、账号池、特定端点、有封号风险。

| 厂商/类型 | rig | new-api | axonhub | claude-code-router | 其他网关 |
|------|:---:|:---:|:---:|:---:|:---:|
| chatgpt 订阅 (codex) | ✅ | ✅ | ✅ | — | — |
| github copilot | ✅ | — | ✅ | — | portkey, bifrost, manifest, TokenHub |
| claude code 订阅 | — | — | ✅ | ✅ | — |
| cline | — | — | ✅ | — | manifest(cline-pass) |
| nanogpt | — | — | ✅ | — | llmgateway, TokenHub |
| kimi code | — | — | ✅ | — | — |
| opencode_go | — | — | ✅ | — | bifrost, manifest |
| antigravity | — | — | ✅ | — | — |
| aimux (axonhub 内置) | — | — | ✅ | — | — |
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
| TokenHub 100+ 代理 | — | — | — | — | TokenHub(requesty, helicone, poe, submodel, morph, nearai, neon, poolside, wandb, clarifai 等) |

aimux 当前不支持任何 coding plan 接入。

### 网关/聚合/代理型

| 厂商 | aimux | rig | litellm | 网关覆盖 |
|------|:---:|:---:|:---:|:---:|
| openrouter | ✅薄 | ✅ | ✅ | 几乎所有网关 |
| vercel (ai gateway) | ✅薄 | — | ✅(vercel_ai_gateway) | — |
| portkey | ✅薄 | — | — | ✅portkey-gateway |
| helicone | ✅薄 | — | — | TokenHub |
| requesty | ✅薄 | — | — | TokenHub |
| 302ai | ✅薄 | — | — | portkey, TokenHub |
| cometapi | ✅薄 | — | ✅ | portkey, TokenHub |
| novita | ✅薄 | — | ✅ | portkey, one-api, TokenHub, APIPark |
| siliconflow | ✅薄 | — | — | new-api, one-api, APIPark, chats, claude-code-router |
| submodel | ✅薄 | — | — | new-api, TokenHub |
| api2d | ✅薄 | — | — | new-api, one-api |
| ohmygpt | ✅薄 | — | — | new-api, one-api |
| closeai | ✅薄 | — | — | one-api |
| openaisb | ✅薄 | — | — | one-api |
| openaimax | ✅薄 | — | — | new-api, one-api |
| ails | ✅薄 | — | — | new-api, one-api |
| api2gpt | ✅薄 | — | — | new-api, one-api |
| aigc2d | ✅薄 | — | — | new-api, one-api |
| fastgpt | ✅薄 | — | — | new-api, one-api |
| tokenpony | ✅薄 | — | — | chats |
| fastrouter | ✅薄 | — | — | TokenHub |
| orcarouter | ✅薄 | — | — | TokenHub |

### 通用/自定义渠道

几乎所有网关项目都有"自定义/OpenAI 兼容/通用"渠道，允许填入任意端点。aimux 的 `OpenAIProvider::with_base_url` 等价于此能力。

---

## 三、aimux 缺失的厂商

> 更新（2026-07-28）：OpenAI 兼容的 LLM chat 厂商已全部接入 thin wrapper。以下厂商未接入，原因各异：

### 非 OpenAI Chat Completions 协议（需要各自的 trait 或 API 格式）

| 厂商 | 类型 | 未接入原因 |
|------|------|---------|
| milvus / qdrant / pg_vector / s3_vectors | 向量数据库 | 不是 LLM API，是向量存储/检索 |
| clip / fastembed / tei / text_embeddings_inference | 嵌入推理 | 不是 chat API，需要 EmbeddingModel trait 实现 |
| jina / nomic / mixedbread | 嵌入/重排序 | 不是 chat API，需要 EmbeddingModel/RerankingModel 实现 |
| recraft / ideogram / stability_ai / segmind / runware | 图像生成 | 不是 chat API，需要 ImageModel trait 实现 |
| meshy / tripo3d | 3D 生成 | 不是 chat API |
| runwayml / sora / vidu / jimeng / midjourney | 视频生成 | 不是 chat API，需要 VideoModel trait 实现 |
| flux | 图像生成 | 不是 chat API |
| suno | 音乐生成 | 不是 chat API，无对应 trait |
| murf / playai / speechify / inworld | 语音合成 | 不是 chat API，需要 SpeechModel trait 实现 |
| aws_polly | 语音合成 | AWS SigV4 认证，需要 SpeechModel |
| nvidia_riva | 语音 | 不是 OpenAI 兼容 |
| soniox | 语音转写 | WebSocket 协议，需要 TranscriptionModel |
| doubaoaudio / mokaai | 音频 | 不是 chat API |
| bing / deepl / dify / slack / doc2x | 非LLM服务 | 搜索/翻译/平台/消息，不是 LLM API |
| streamlake / antling / sangforaicp / skylark | 非LLM服务 | 视频流/未知/企业平台/未知 |

### 特殊认证（不能简单用 API key + Bearer）

| 厂商 | 认证方式 | 未接入原因 |
|------|---------|---------|
| watsonx (ibm) | IBM IAM token | 需要独立认证实现 |
| sagemaker (aws) | AWS SigV4 | 需要独立认证实现 |
| sap | SAP auth | 需要独立认证实现 |
| oci (oracle) | Oracle auth | 需要独立认证实现 |
| snowflake | Snowflake auth | 需要独立认证实现 |
| bedrock_mantle | AWS auth | 需要独立认证实现 |

### 需升级为原生协议实现（当前 thin wrapper 可能无法处理厂商特有字段）

| 厂商 | 当前状态 | 需要做的事 |
|------|---------|---------|
| baidu (文心) | ✅薄（OpenAI 兼容端点） | 文心有自定义协议（ERNIE-Bot），需原生实现以覆盖完整能力 |
| tencent (混元) | ✅薄 | 混元有自有签名机制，需原生实现 |
| xunfei (讯飞) | ✅薄 | 讯飞用 WebSocket 协议，需独立实现 |
| alibaba | ✅薄 | reasoning_content 字段被丢弃，需处理推理字段 |
| groq | ✅薄 | top_k 靠巧合绕过，需配置描述标记不支持 |
| 所有薄封装 | ✅薄 | 按 RFC-0002 加配置描述结构，处理各家差异 |

### 编程订阅是否纳入（待定）

coding plan 类（chatgpt 订阅、copilot、claude code 订阅等）在 2026 年很火，但有 ToS 风险、协议不稳定。aimux 作为服务接入统一层，是否纳入这类接入需要决策。如果要纳入，参考 axonhub 和 claude-code-router 的实现。

---

## 四、实现过程中需要更新或处理的点

### 1. 薄封装改造（RFC-0002 已记录）

13 个 OpenAI 兼容薄封装需要加配置描述结构，让各家差异（推理字段、能力标记、用量统计方式）能被表达。这是放量铺厂商的前提。

### 2. 国产厂商的特殊协议

百度、讯飞、智谱不完全走 OpenAI 兼容协议，需要原生实现：
- 百度文心：自定义协议，有 ERNIE-Bot 系列
- 讯飞星火：WebSocket 协议，签名鉴权特殊
- 智谱 GLM：有自有协议，也有 OpenAI 兼容端点

### 3. 本地推理厂商的接入方式不同

ollama、llama.cpp、lmstudio 是本地服务，没有 HTTPS 和密钥，接入方式和云厂商不同：
- ollama：HTTP localhost，无认证
- llama.cpp：本地进程，走 GGUF 文件
- lmstudio：本地 HTTP，OpenAI 兼容

这影响 Provider trait 的认证和 URL 构造逻辑。

### 4. 网关型厂商的路由语义

openrouter、vercel gateway 不是普通厂商——它们内部再路由到其他厂商。用户可能要指定"用 openrouter 路由到 anthropic/claude"。这影响 model_id 的解析方式（可能含斜杠分隔的厂商前缀）。

### 5. 测试覆盖

按 RFC-0003 的录播方案，每补一个厂商就要：
- 从 rig 的录像里找对应厂商的录像（rig 覆盖 16 家）
- rig 没覆盖的用 llmtape 自己录
- 加进统一契约测试

### 6. 已有实现的验证

现有 11 个原生实现需要用录播测试验证正确性：
- openai、anthropic、google、bedrock、vertex、azure、cohere、mistral、xai、deepseek、anthropic_aws
- 这些实现代码量大（单家 500-1500 行），可能有未发现的解析 bug
- 录播回放能发现"返回格式变了但代码没跟上"的问题

### 7. coding plan 接入的特殊性（如纳入）

OAuth 认证流程、账号池管理、令牌刷新、封号风险处理——这些都和普通 API key 认证不同。如果决定纳入，需要单独设计一层认证抽象。

---

## 五、高用户量 coding agent 与转发服务

> 之前只扫了 SDK 和网关，漏了用户量更大的 coding agent 工具和配套的转发/切换服务。
> 这些项目体量巨大（多个 10 万+ star），是 coding plan 类接入的实际来源。

### coding agent（终端/IDE 类）

| 项目 | ★ | 语言 | 定位 |
|------|:---:|:---:|------|
| openai/codex | 101k | Rust | OpenAI 官方 coding agent，终端运行 |
| anthropics/claude-code | 139k | — | Anthropic 官方 coding agent，终端运行 |
| anomalyco/opencode | 190k | TypeScript | 开源 coding agent（非官方）|
| earendil-works/pi | 79k | TypeScript | AI agent 工具包：统一 LLM API + agent loop + TUI + coding CLI |
| google-gemini/gemini-cli | 106k | TypeScript | Google 官方 Gemini 终端 agent |
| cline/cline | 65k | TypeScript | 自治 coding agent，SDK/IDE/CLI |
| Aider-AI/aider | 48k | Python | 终端 AI 配对编程 |
| continuedev/continue | 35k | TypeScript | 开源 coding agent（IDE）|
| RooCodeInc/Roo-Code | 24k | TypeScript | 编辑器内多 agent 团队 |
| opencode-ai/opencode | 14k | Go | 终端 coding agent（另一个 opencode）|

### 转发/切换/代理服务（让 coding agent 接入任意厂商）

| 项目 | ★ | 语言 | 定位 |
|------|:---:|:---:|------|
| farion1231/cc-switch | 122k | Rust+TS | 跨平台桌面助手：Claude Code/Codex/OpenCode/OpenClaw/Grok Build/Hermes Agent 统一管理+厂商切换 |
| musistudio/claude-code-router | 36k | TypeScript | Claude Code 路由到任意模型/provider |
| lidge-jun/opencodex | 5.2k | TypeScript | Codex CLI + Claude Code 的通用 provider 代理 |
| XueshiQiao/CCSwitcher | 160 | — | Claude Code 账号一键切换 |
| liuzhengming/ccswitch-deepseek | 296 | — | ccswitch 转发到 DeepSeek |
| nicremo/ccs | 11 | — | Claude Code 切到 MiniMax/Kimi/GLM/DeepSeek/Qwen |
| glidea/claude-worker-proxy | 274 | — | Cloudflare Worker 上的 Claude Code 代理 |

### coding agent 生态周边

| 项目 | ★ | 定位 |
|------|:---:|------|
| awesome-opencode | 9.2k | opencode 插件/主题/agent 资源集 |
| alvinunreal/oh-my-opencode-slim | 7.4k | opencode 多 agent 套件，混合任意模型 |
| pinchbench/skill | 1.3k | OpenClaw coding agent 的 LLM 基准测试 |
| kenryu42/cc-safety-net | 1.5k | coding agent CLI 安全网（拦截危险命令）|
| agent-of-empires/agent-of-empires | 2.9k | 多 agent（Claude Code/OpenCode/Codex/Gemini/Pi/Copilot/Factory Droid）统一管理 TUI+Web |

### coding agent 与转发服务的详细厂商清单

**codex**（OpenAI 官方，Rust）：内置 4 家——OpenAI、Amazon Bedrock、Ollama、LM Studio。仅走 OpenAI Responses API，无跨协议转换。支持 ChatGPT 订阅 OAuth。

**opencode**（190k star，TS）：委托 Vercel AI SDK，内置 ~20 家——anthropic、openai、google、google-vertex、github-copilot、amazon-bedrock、azure、openrouter、mistral、gitlab、xai、groq、deepinfra、cerebras、cohere、togetherai、perplexity、vercel、alibaba、venice、bedrock/mantle。支持 GitHub Copilot OAuth。

**pi**（79k star，TS）：自实现 10 种 API 适配器，37 个内置 provider——amazon-bedrock、ant-ling、anthropic、azure-openai-responses、cerebras、cloudflare-ai-gateway、cloudflare-workers-ai、deepseek、fireworks、github-copilot、google、google-vertex、groq、huggingface、kimi-coding、minimax、minimax-cn、mistral、moonshotai、moonshotai-cn、nvidia、openai、openai-codex、opencode、opencode-go、openrouter、qwen-token-plan、qwen-token-plan-cn、radius、together、vercel-ai-gateway、xai、xiaomi、xiaomi-token-plan(ams/cn/sgp)、zai、zai-coding-cn。支持 Codex 订阅/GitHub Copilot/radius 三种 OAuth。

**cline**（65k star，TS）：~55+ provider——anthropic、claude-code、cline、cline-pass、openai-compatible、openai-native、openai-codex、openai-codex-cli、opencode、bedrock、vertex、gemini、ollama、lmstudio、deepseek、xai、together、fireworks、groq、poolside、cerebras、sambanova、nebius、baseten、requesty、litellm、huggingface、vercel-ai-gateway、v0、aihubmix、hicap、nousResearch、huawei-cloud-maas、wandb、xiaomi、tencent-tokenhub、kilo、zai、zai-coding-plan、qwen、qwen-code、doubao、mistral、moonshot、asksage、minimax、dify、oca、sapaicore、openrouter。支持 Claude Code/Cline/Codex/opencode 四种 OAuth。

**continue**（35k star，TS）：66 个 provider 类——含 Anthropic、OpenAI、Gemini、Bedrock、Azure、VertexAI、Cohere、Mistral、Deepseek、xAI、MiniMax、Groq、OpenRouter、Together、Fireworks、Cerebras、Cloudflare、DeepInfra、HuggingFace、LlamaCpp、LlamaStack、Llamafile、LMStudio、Ollama、Nvidia、Novita、Msty、Mimo、Moonshot、Nebius、NCompass、Nous、OVHcloud、Replicate、Relace、SambaNova、SageMaker、Scaleway、SiliconFlow、TARS、Tensorix、TextGenWebUI、Venice、Vllm、WatsonX、zAI、CometAPI、ClawRouter、Docker、Flowise、FunctionNetwork、Inception、Jina、Kindo、Lemonade、AskSage 等。

**Roo-Code**（24k star，TS）：30+ handler——Anthropic、AwsBedrock、DeepSeek、Moonshot、Gemini、LiteLLM、LmStudio、Mistral、OpenAiCodex、OpenAiNative、OpenAi、OpenAICompatible、OpenRouter、Poe、QwenCode、Requesty、SambaNova、Unbound、Vertex、AnthropicVertex、VsCodeLm、XAI、ZAi、Fireworks、VercelAiGateway、MiniMax、Baseten、NativeOllama、FakeAI。内部规范格式 = Anthropic Messages。支持 Codex 订阅 OAuth。

**opencode-ai**（14k star，Go）：11 家——Copilot、Anthropic、OpenAI、Gemini、Bedrock、GROQ、Azure、VertexAI、OpenRouter、XAI、Local。每厂商独立 Go SDK client。支持 GitHub Copilot token exchange。

**aider**（48k star，Python）：委托 litellm，支持 litellm 全部 134 家。无 OAuth。

**opencodex**（5.2k star，TS，转发服务）：60 个 provider entry——含 OpenAI(Codex 订阅池)、Anthropic(Claude 订阅)、xAI/Grok、Kimi、Kiro、Google Antigravity、Cursor、GitHub Copilot、DeepSeek、Moonshot、Z.AI、Zhipu、Qwen、Alibaba、Tencent、Baidu、MiniMax、SiliconFlow、Groq、Cerebras、Together、Fireworks、FirePass、HuggingFace、NVIDIA NIM、Venice、NanoGPT、Synthetic、Mistral、OpenRouter、OrcaRouter、BizRouter、Parallel、ZenMux、Vercel AI Gateway、Cloudflare AI Gateway/Workers AI、GitLab Duo、Kilo、Umans、Neuralwatt、opencode-go/zen/free、Ollama、vLLM、LM Studio、LiteLLM 等。协议转换最完整（内部中间表示 + 双向适配器）。

**cc-switch**（122k star，Rust+Tauri）：80+ 家 preset——含 AiHubMix、OpenRouter、TheRouter、Novita AI、DMXAPI、CrazyRouter、NewAPI、APIKEY.FUN、SubRouter、DeepSeek、Zhipu GLM、Bailian、Baidu Qianfan、StepFun、ModelScope、Longcat、MiniMax、BaiLing、Xiaomi MiMo、DouBaoSeed/BytePlus、Tencent、Kimi/Kimi For Coding、PackyCode、ZetaAPI、APINebula、AICodeMirror、PatewayAI、FennoAI、RunAPI、Unity2.ai、Shengsuanyun、AIGoCode、AICoding、Code0、TeamoRouter、ClaudeCN、ClaudeAPI、CCSub、SSSAiCode、Micu、RightCode、ETok.ai、Cubence、SudoCode、Amux、CherryIN、RelaxyCode、E-FlowCode、PIPELLM、NekoCode、AtlasCloud、Compshare、KAT-Coder、Nvidia、Together AI、Nous Research、Claude Official、OpenAI Official、Google Official、Grok Official、Codex、GitHub Copilot、AWS Bedrock、Azure OpenAI、Gemini Native、OpenCode Go、自定义网关。无协议转换，仅配置切换 + 模型名映射。

**claude-code-router**（36k star，TS）：20+ preset——anthropic、openai、deepseek、gemini、bailian、claudeapi、code0、fenno、kimi-coding、minimax、mistral、moonshot、nvidia、openrouter、qiniu-ai、runapi、siliconflow、teamorouter、unity2、zai-global-coding、zai-global-general、zhipu-cn-coding、zhipu-cn-general。内核不做协议转换，靠 route script。

### 新发现的厂商（之前清单遗漏）

以下厂商在 coding agent/转发服务中出现，之前清单未记录：

| 厂商 | 来源 | 类型 |
|------|------|------|
| ant-ling | pi | LLM |
| radius | pi | LLM（OAuth） |
| kimi-coding | pi/opencodex | coding plan |
| qwen-token-plan / qwen-code | pi/cline | coding plan |
| zai-coding-cn / zai-coding-plan | pi/cline | coding plan |
| poolside | cline | LLM |
| hicap | cline | LLM |
| asksage | cline/continue | LLM |
| dify | cline | 平台 |
| oca | cline | LLM |
| sapaicore | cline | LLM |
| huawei-cloud-maas | cline | LLM |
| msty | continue | 本地 |
| tars | continue | LLM |
| tensorix | continue | LLM |
| relace | continue | LLM |
| llamastack | continue | 本地 |
| kindo | continue | LLM |
| lemonade | continue | LLM |
| flowise | continue | 平台 |
| functionnetwork | continue | LLM |
| poe | Roo-Code | 聚合 |
| unbound | Roo-Code | LLM |
| firepass | opencodex | LLM |
| orcarouter | opencodex | 代理 |
| bizrouter | opencodex | 代理 |
| parallel | opencodex | 代理 |
| zenmux | opencodex | 代理 |
| umans | opencodex | LLM |
| neuralwatt | opencodex/axonhub | LLM |
| opencode-free | opencodex | 免费 |
| kiro | opencodex/manifest | coding plan |
| google antigravity | opencodex/axonhub | coding plan |
| cursor | opencodex | coding plan |
| packycode | cc-switch | 代理 |
| zetaapi | cc-switch | 代理 |
| apinebula | cc-switch | 代理 |
| aicodemirror | cc-switch | 代理 |
| patewayai | cc-switch | 代理 |
| fennoai | cc-switch | 代理 |
| runapi | cc-switch/claude-code-router | 代理 |
| unity2.ai | cc-switch/claude-code-router | 代理 |
| shengsuanyun | cc-switch | 代理 |
| aigocode | cc-switch | 代理 |
| aicoding | cc-switch | 代理 |
| code0 | cc-switch/claude-code-router | 代理 |
| teamorouter | cc-switch/claude-code-router | 代理 |
| claudecn | cc-switch | 代理 |
| ccsub | cc-switch | 代理 |
| sssaicode | cc-switch | 代理 |
| micu | cc-switch | 代理 |
| rightcode | cc-switch | 代理 |
| etok.ai | cc-switch | 代理 |
| cubence | cc-switch | 代理 |
| sudocode | cc-switch | 代理 |
| amux | cc-switch | 代理 |
| cherryin | cc-switch | 代理 |
| relaxycode | cc-switch | 代理 |
| e-flowcode | cc-switch | 代理 |
| pipllm | cc-switch | 代理 |
| nekocode | cc-switch | 代理 |
| compshare | cc-switch | 代理 |
| kat-coder | cc-switch | 代理 |
| dmxapi | cc-switch | 代理 |
| crazyrouter | cc-switch | 代理 |
| subrouter | cc-switch | 代理 |
| apikey.fun | cc-switch | 代理 |
| therouter | cc-switch | 代理 |
| clawrouter | continue | 代理 |

> 以上新增厂商绝大多数是国内 coding agent 代理/中转服务，2026 年大量涌现。协议转换逻辑详见 [0005-protocol-conversion.md](0005-protocol-conversion.md)。

### 对 aimux 的意义

1. **cc-switch 用 Rust 写的**（Tauri 桌面应用）——它需要统一管理多家厂商的认证和切换，这正是 aimux 的 provider 抽象能提供的。
2. **opencode 和 pi 都自带"统一 LLM API"层**——pi 明确写了"unified LLM API"，这和 aimux 定位直接重合。
3. **转发服务（claude-code-router/opencodex）本质就是微型网关**——把 coding agent 的请求转发到任意厂商，和前面扫的网关项目同类。
4. **这些项目用户量远超 SDK 项目**（opencode 19 万 vs rig 8 千）——如果 aimux 想被广泛使用，coding agent 生态是比 SDK 生态更大的市场。

aimux 当前不支持任何 coding agent 接入。是否为这类场景提供支持（OAuth 认证、订阅额度管理、厂商切换）需要决策。

---

## 六、数据来源说明

本文档的厂商清单来自以下扫描（2026-07-27）：

- **Rust 竞品**：rig(28家)、rust-genai(31家)、langchain-rust(5家)、kalosm、swiftide(9家)、graniet-llm(15家)、rllm(7家)、edgequake-llm、llm-connector(8家)、ai.rs、litellm-rust、unia(14家)、multi-llm、llmrust、rust_ai_sdk、llm-chain
- **Python 生态**：litellm(134家)、langchain、llama_index(99家)、dspy、haystack、guidance、pydantic-ai(14家)、outlines、instructor(14家)、aisuite(28家)、textgrad、AutoGPT、OpenHands、autogen、crewAI、mirascope
- **其他语言**：mastra、langchainjs、LlamaIndexTS、eino(仅接口)、langchaingo、langchain4j、spring-ai、semantic-kernel、semantic-kernel-java、dotnet-extensions、LangChain-csharp、langchain-swift
- **网关**：new-api(54渠道)、one-api(38适配器)、portkey-gateway(72)、bifrost(29)、coai(18)、manifest(33)、higress(36)、one-hub(41)、simple-one-api(18)、uni-api(12适配器)、axonhub(20)、TokenHub(35+直连/100+代理)、aiproxy(37)、chats(22)、otari(配置驱动)、ferro-ai-gateway(29)、OpenGateLLM(5)、llmgateway(33)、APIPark(35)、envoy-ai-gateway(7)、claude-code-router(20)
- **coding agent 与转发服务**：openai/codex(4家内置)、anthropics/claude-code(仅文档)、anomalyco/opencode(20家)、earendil-works/pi(37家)、google-gemini/gemini-cli(仅Google)、cline(55+家)、Aider-AI/aider(litellm透传)、continuedev/continue(66家)、RooCodeInc/Roo-Code(30+家)、opencode-ai/opencode(11家)、farion1231/cc-switch(80+家)、musistudio/claude-code-router(20+家)、lidge-jun/opencodex(60家)、XueshiQiao/CCSwitcher(Claude账号池)、glidea/claude-worker-proxy(任意OpenAI/Gemini)、liuzhengming/ccswitch-deepseek(DeepSeek)、nicremo/ccs(5家国产)、agent-of-empires(2.9k)、oh-my-opencode-slim(7.4k)、pinchbench(1.3k)、cc-safety-net(1.5k)
- **协议转换逻辑**：详见 [0005-protocol-conversion.md](0005-protocol-conversion.md)，覆盖 SDK 适配层设计、网关跨协议互转、coding agent/转发服务协议转换
