# Round 2 D2c 探索报告:网关/聚合器/自托管引擎的缓存行为与 usage 上报(2026-08-01)

> Agent D2c(探索,第 2 轮)。每条:内容 + 来源 + 验证方式。术语保留英文。目标:判断哪些端点能报出可信 cached_tokens。

## 1. OpenRouter
- Prompt caching 支持面:OpenAI(自动,最低 1024)、Anthropic(自动/显式 cache_control,TTL 5m/1h)、DeepSeek(自动)、Gemini(implicit + 显式 cache_control)、Grok、Moonshot、Groq(仅 Kimi K2)、Alibaba Qwen(仅显式,快照端点不支持)、Z.AI(自动)。来源:https://openrouter.ai/docs/guides/best-practices/prompt-caching (WebFetch 全文)
- usage 透传:cached_tokens + cache_write_tokens(chat 在 prompt_tokens_details;responses 在 input_tokens_details);另附 cache_discount 字段。来源同上(官方示例 JSON)
- **sticky routing**:account+model+conversation 三级;conversation 默认 = hash(首条 system/dev + 首条非 system 消息);session_id(≤256 字符)/x-session-id 优先,其次 prompt_cache_key;仅在 provider cache 读价 < 普通价时激活;用户显式 provider.order 时失效。**无 session_id 时首次请求不粘(观察到命中后才粘)**。来源同上
- **OpenRouter 自有缓存层:response caching**(2026-04-30 beta,独立于 prompt caching):X-OpenRouter-Cache:true;cache key = hash(request body + model + key + streaming mode);TTL 默认 5min(1s-24h);命中返回 X-OpenRouter-Cache-Status: HIT/MISS;零 token 计费;80-300ms。来源:https://openrouter.ai/blog/announcements/response-caching/ (WebFetch)。**审计注意:完全相同请求命中该层时 usage 无 cached_tokens,是合法"响应缓存"而非掺水**
- "不支持 prompt caching 的 provider 完整清单":**未找到**(官方只列支持的;reddit 有抱怨但非官方)。来源:reddit r/openrouter "Openrouter should require input cache"

## 2. LiteLLM
- 透传:统一 OpenAI 格式 prompt_tokens_details.cached_tokens;Anthropic 专属 cache_creation_input_tokens 亦透传;prompt_tokens 含 hit+miss;cache_control 自动转发 Anthropic/Gemini/Vertex,并翻译为 Bedrock cachePoint;**低于最低 token 静默跳过不报错**(官方建议查 cache_creation_input_tokens 验证)。来源:https://docs.litellm.ai/docs/completion/prompt_caching (WebFetch)
- **自有 response caching**(Redis/内存/磁盘/Qdrant 语义)≠ provider 缓存:同请求第二次直接从缓存返回,不打 provider,无 cached_tokens;按请求可控制 cache.ttl/s-maxage/no-cache/no-store。来源:https://docs.litellm.ai/docs/proxy/caching (WebFetch)
- 已知 bug(均在 GitHub issue,WebFetch 全文):
  - **#9812**:cache_creation_input_tokens 被 input 与 creation 双计费,成本近翻倍($0.091 vs Anthropic 账单 $0.054);PR #9838 修复。https://github.com/BerriAI/litellm/issues/9812
  - **#27763**(2026-05 已关):/v1/messages 透传路径(Vertex/Bedrock 的 Claude)cache_read_input_tokens 不归一化进 prompt_tokens_details.cached_tokens,litellm_cached_tokens_metric_total 恒不涨;相关 #11364(Anthropic direct cached_tokens 不填充)、#7790(流式 logging callback 丢 cache 字段)、#11789(Anthropic 流式成本忽略 cache read)、#26625(Bedrock /v1/messages 缓存坏)。https://github.com/BerriAI/litellm/issues/27763
- 审计含义:经 LiteLLM 的 cached_tokens 可信度**按路由路径与流式/非流式而异**。

## 3. 网关代码盘点(reference/ 本地源码,直接阅读)
- **one-api**:Usage 仅 prompt/completion/total + completion_tokens_details,无 prompt_tokens_details → cached_tokens 恒被剥(响应重造)。[relay/model/misc.go:3-15](reference/one-api/relay/model/misc.go#L3)
- **new-api**:reference 检出近乎为空(仅 constant/),无法源码验证;Web 未检索到 cached_tokens 支持证据 → [UNVERIFIED/未找到]
- **simple-one-api**:非流式 OpenAI 适配器只拷贝 prompt/completion/total,自定义 Usage 无 prompt_tokens_details → **非流式 cached_tokens 被剥**;[pkg/adapter/openai_openai.go:47-51](reference/simple-one-api/pkg/adapter/openai_openai.go#L47);流式路径直接 re-marshal go-openai v1.37(含 PromptTokensDetails)→ 理论透传 [依赖 go-openai 解析,go.mod:18];Claude 适配器只留 input/output_tokens → **Anthropic cache 字段全丢**。[pkg/llm/claude/claude_response.go](reference/simple-one-api/pkg/llm/claude/claude_response.go)
- **Portkey**:OpenAI 非流式 = 原样透传(cached_tokens 保留);**OpenAI 流式 = 重建 usage 只留 prompt/completion/total,cache 字段仅 provider===ANTHROPIC 才带 → OpenAI 流式 cached_tokens 被剥**;[src/providers/openai/chatComplete.ts:164-197](reference/portkey-gateway/src/providers/openai/chatComplete.ts#L164);Anthropic 映射 cache_read→cached_tokens 且 total=input+output+creation+read(口径不同);Gemini/Vertex/Bedrock 各自映射 cachedContentTokenCount / cacheReadInputTokens;Cohere types 含 cached_tokens
- **ferro-ai-gateway**:Usage 反序列化把 prompt_tokens_details.cached_tokens 与 DeepSeek 扁平 prompt_cache_hit_tokens 折叠为**扁平 cache_read_tokens 字段**输出(非 OpenAI 嵌套格式)→ 改写式透传,OpenAI 客户端按标准字段读会丢;[providers/core/chat.go:309-352](reference/ferro-ai-gateway/providers/core/chat.go#L309)

## 4. 自托管引擎
- **vLLM**:
  - block 哈希 = hash(parent hash, block tokens, extra hashes[LoRA ID/多模态 image hash/**cache_salt**]);只缓存完整 block;v0.11+ 默认 sha256;--prefix-caching-hash-algo sha256_cbor 可跨环境复现。https://docs.vllm.ai/en/stable/design/prefix_caching/ (WebFetch)
  - **--enable-prefix-caching:V0 默认关,V1 默认开**。https://docs.vllm.ai/en/v0.8.5/serving/engine_args.html
  - cache_salt:请求体 per-request 字段,注入首 block 哈希,同 salt 才可复用 → 多租户隔离/防时序侧信道;**不同 salt 请求不命中是正常行为**。来源同上
  - **usage 上报缺陷:V1 引擎即使 --enable-prefix-caching + --enable-prompt-tokens-details,prompt_tokens_details 恒为 null**(PR #18149 只修 engine 内部,未映射到 OpenAI serving 层;14+ 个月,2026-06 nightly 仍坏);V0 正常;**vllm:prefix_cache_hits 指标正常**。https://github.com/vllm-project/vllm/issues/44961 (WebFetch 全文)
- **SGLang**:
  - RadixAttention:page_size=1 时真正 token 级;命中只 prefill 新 token。https://sgl-project-sglang-93.mintlify.app/concepts/prefix-caching
  - **--enable-cache-report + extra_body return_cached_tokens_details 才输出 usage.prompt_tokens_details.cached_tokens**(默认无)。https://sgl-project-sglang-93.mintlify.app/backend/openai-compatible-api (WebFetch)
  - bug **#25972**:PD disaggregation + decode radix cache 双重计数,污染 OpenAI usage.cached_tokens。https://github.com/sgl-project/sglang/issues/25972
- **Ollama**:
  - /api/chat、/api/generate 只报 prompt_eval_count/eval_count/各 duration,无 cached token 字段;流式在 done chunk。https://docs.ollama.com/api/usage (WebFetch)
  - KV 缓存自动,keep_alive 控制驻留(默认 5 分钟卸载)。https://docs.ollama.com/faq
  - **官方 issue #15758:"后台用缓存加速但永远报 0 cached tokens"**(2026-04);Cloud issue #16714 同。https://github.com/ollama/ollama/issues/15758 → 审计必须降级
  - "Ollama 3 的 prompt caching 现状":未找到独立信息(仅 leanpub 书论 keep_alive)
- **llama.cpp server**:/v1/chat/completions usage = {prompt_tokens, completion_tokens, total_tokens}(README 示例),无 prompt_tokens_details;KV 复用(cache_prompt/n_past)存在但不报命中数。https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md
- **TensorRT-LLM trtllm-serve**:OpenAI 兼容 /v1/chat/completions + /v1/responses;KV block reuse(kv_cache_reuse/enable_block_reuse)存在;**未找到 usage 含缓存字段的文档** → [未找到/降级]。https://nvidia.github.io/TensorRT-LLM/commands/trtllm-serve/trtllm-serve.html

## 5. Cohere / Mistral / xAI 原生 API
- **Cohere v2**:usage.cached_tokens("number of prompt tokens that hit the inference cache",cohere-python ≥5 SDK 类型;docs.cohere.com/v2/reference/chat 有文档)。旁证:pydantic-ai #5945(该字段被其 _map_usage 丢弃,PR #5957 修复)→ 字段确实存在。https://github.com/pydantic/pydantic-ai/issues/5945 (WebFetch 全文)
- **Mistral**:prompt_cache_key 提升命中(不保证);usage.prompt_tokens_details.cached_tokens;**64-token block 粒度,cached_tokens 为 64 倍数,<64 token 无命中**;读价 = 标准价 10%。https://docs.mistral.ai/studio-api/conversations/advanced/prompt-caching (WebFetch)
- **xAI/Grok**:自动缓存;x-grok-conv-id / prompt_cache_key 提升命中(缓存按 server 存储);chat: usage.prompt_tokens_details.cached_tokens;responses: usage.input_tokens_details.cached_tokens;官方给出 cached_tokens=0/>0/=prompt_tokens 的判定表。https://docs.x.ai/developers/advanced-api-usage/prompt-caching/usage-and-pricing (WebFetch)

## 6. 结论:可信度分级与审计降级
- **A 级(可直接用服务端 cached_tokens)**:OpenRouter 透传、Cohere v2、Mistral(注意 64 粒度)、xAI、SGLang(--enable-cache-report 开启时)、vLLM V0(--enable-prompt-tokens-details)
- **B 级(可信但口径/路径有坑)**:LiteLLM(流式与 /v1/messages 透传路径有丢字段/双计费史)、Portkey(非流式 OpenAI 透传 OK;流式 OpenAI 被剥;Anthropic/Gemini/Bedrock 映射 OK)、ferro(输出扁平 cache_read_tokens,非 OpenAI 标准)
- **C 级(报不出 → 客户端前缀对比 + prefix_cache_hits 指标 + TTFT 降级)**:vLLM V1(prompt_tokens_details=null,默认引擎)、Ollama(恒 0)、llama.cpp、TensorRT-LLM、one-api、simple-one-api 非流式
- **额外审计信号**:OpenRouter response cache HIT(usage 零但毫秒级返回 + 响应头)、LiteLLM Redis 响应缓存(usage 零);OpenRouter 无 session_id 时首请求不粘路由(预热期 0 命中正常)

## 剩余不确定性
1. vLLM V1 prompt_tokens_details null 在 2026-08 是否已修复(issue 快照 2026-06 仍 broken)[UNVERIFIED]
2. OpenRouter"不支持 caching 的 provider"官方清单未找到
3. new-api 上游 cached_tokens 行为(本地检出不完整)[UNVERIFIED]
4. Ollama /v1 端点 usage 精确字段未逐一实测
5. TRT-LLM 为"未找到"而非"确认无"缓存字段
