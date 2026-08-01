# Round 3: Provider 缓存审计矩阵(设计 agent 第 3 轮,2026-08-01)

> 汇总 D1/D2a/D2b/D2c/D3-D5 全部 provider 事实为统一矩阵;4 项缺口已 Web 补查(带 URL)。aimux 接入路径经代码确认(aimux-providers/src)。

## 一、审计矩阵(19 行)

列:Provider | aimux 接入 | 机制 | 最低门槛 | 粒度/对齐 | TTL | 上报→aimux 映射 | 客户端可对账不变量 | 已知缺陷 | 审计等级 | 落地注意

| Provider | aimux 接入 | 机制 | 最低门槛 | 粒度/对齐 | TTL | 上报→映射 | 不变量 | 已知缺陷 | 等级 | 落地注意 |
|---|---|---|---|---|---|---|---|---|---|---|
| OpenAI Chat | 原生 openai/chat(model.rs,convert.rs) | 自动;5.6+ 显式 options/breakpoint | ≥1024(5.6+ 严格);旧 1024-2048 可变 | 5.6- 128-token;5.6+ 字节精确(128 对齐失效) | mem 5-10min(≤1h);extended 24h;5.6+ ttl=30m | cached_tokens→cache_read;cache_write→write;no_cache=差值 | <1024→0;5.6- cached%128==0;cached≤已发前缀 LCP | 2025-01 计费事故(报高实锤);图片 token 不计 usage;5-nano/mini 不命中;流式断丢 | A | 5.6+ implicit 断点在最新 user/tool 消息:agent loop 尾部变→cached=0 合法,勿判掺水 |
| OpenAI Responses | 原生 openai/responses(mod+responses_convert) | 自动;5.6+ 显式 | ≥1024 | 同 Chat | 同 Chat | input_tokens_details.cached_tokens→cache_read | 同 Chat | 流式 include_usage 才出、断流丢 | A | 字段路径异于 chat(prompt_ vs input_),映射器按端点分派 |
| Azure OpenAI | 原生 azure(responses.rs,Chat+Responses) | 自动不可关;5.6+ prompt_cache_key;options/breakpoint 不支持 | ≥1024 且前 1024 完全一致(单字符差⇒0) | 128-token(2026-07 文档仍如此,异于 OpenAI 5.6)[补查1] | mem 5-10min/24h;5.6+ 默认 24h,不支持 in_memory | cached_tokens→cache_read;5.6 不报 cache_write_tokens | 前 1024 差 1 字⇒0;之后每 128 命中;路由 hash≈前 256 tok | prompt_cache_key 非确定性需预热,~15 RPM/key 漏命中;Azure Responses 实测 0 命中;5.6 无写侧 | B | 粒度与 OpenAI 原生不同源:量化校验用 128 对齐放宽版;5.6 无法对账写侧 |
| Anthropic | 原生 anthropic(convert.rs,stream.rs) | 自动+显式 cache_control(5m/1h) | 按模型 512-4096 | content-block 断点≤4;20-block 回看 | 5m 默认(2026-03 静默回归);1h 可选写价 2× | cache_read→read;cache_creation(ephemeral_5m/1h)→write;input=末断点后 | total=read+creation+input;首请求 read==0;read≤历史写过未过期;TTL 档位=请求参数 | litellm#9812 双计费;#10249 流式双计;TTL 回归(#46829);server tool 自动 5m 断点混入 | A(字段全) | aimux 生产路径只填 total(stream.rs:180-192),先补 cache 映射;建议对接官方 Cache diagnostics header |
| Anthropic-Bedrock | 原生 bedrock(Anthropic 消息体,SigV4 字节签名) | 显式 cachePoint;Claude 单 checkpoint+20-block 回看 | 按模型 1024/4096 | checkpoint≤4(简化 1) | 5m 默认,部分 1h | cacheReadInputTokenCount→read;cacheWriteInputTokenCount→write;inputTokens=非缓存 | total=input+read+write;首请求 write>0/read==0 | 顺序 tools→system→messages 破坏即 miss;ConverseStream usage 位置待查 | A | 字节签名⇒request_body 字节=发送字节,前缀对比零转换 |
| Bedrock Converse(Claude/Nova) | 原生 bedrock/model.rs(Converse) | 可选特性;Claude 显式;Nova 自动(只省延迟) | 1024/4096 按模型 | checkpoint≤4 | 5m 默认/1h | 同 Anthropic-Bedrock(flat camelCase) | quota=input+write+output,read 不占;input+read+write=total | 三字段混算易虚报(B4);Nova 省钱需显式 | B | flat 字段名与 Anthropic 嵌套不同,两套解析;开缓存后 inputTokens 语义变化 |
| Gemini API | 原生 google/model.rs | implicit 默认开(2.5+);Interactions 仅 implicit | 2.5=2048;3.1 Pro Preview/3.5 Flash=4096[补查4] | 未公开 | 未公开 | total_cached_tokens→cache_read | 低于门槛无 implicit 命中 | preview 账目异常(G16)/命中不稳 40-60%(G15);流式 usage 在流末尾不稳 | B | 流式对账用非流式复测;preview 模型降低预期区间 |
| Vertex | 原生 vertex/model.rs+anthropic_model.rs | implicit+显式 CachedContent | 3.x=4096;2.5=2048[补查4] | 未公开 | implicit ≤24h(依负载);显式默认 60min | total_cached_tokens/cachedContentTokenCount→read | 低于门槛 0;显式 model 必填不可变 | implicit 清除时账目异常;preview 池小 | B | 显式与 implicit 字段名不同;显式 TTL/命中重置语义按文档实现 |
| DeepSeek | compat(deepseek profile) | 自动,磁盘缓存 | 64 token(整单元) | 64-token 单元;前缀须从 token 0 | 闲置数小时-数天 | prompt_cache_hit_tokens→read(扁平字段) | prompt=hit+miss(官方);cached%64==0 | 模型版本升级全失效;user_id 隔离;第二轮 0 命中合法 | A | 扁平字段名勿用 prompt_tokens_details 解析;多轮看公共前缀落盘后第 3 轮命中 |
| Cohere | 原生 cohere/model.rs(v2) | 自动 | 未公开 | 未公开 | 未公开 | usage.cached_tokens→read | cached≤prompt_tokens | 无写侧字段;pydantic-ai 曾丢(#5945) | A | 只对账读侧;无粒度信息→期望区间放宽 |
| Mistral | 原生 mistral/model.rs | 自动;prompt_cache_key 提升(不保证) | 64 | 64-token block | 未公开 | prompt_tokens_details.cached_tokens→read | cached%64==0;<64→0 | 预热期 0 常态 | A | 64 整除是强不变量;区间判定按 64 对齐 |
| xAI Grok | 原生 xai/model.rs+responses/mod.rs | 自动;x-grok-conv-id/prompt_cache_key 提升 | 未公开 | 未公开 | 未公开 | chat=prompt_tokens_details;responses=input_tokens_details→read | 官方判定表 cached=0/>0/=prompt;aimux 有 read>input 兜底 | 无写侧;命中不保证 | A | 双端点字段路径不同,沿用 OpenAI 映射按端点分派 |
| OpenRouter | 薄封装 openrouter.rs | 透传上游+sticky routing;自有 response caching(beta) | 随上游 | 随上游 | 随上游;response cache 默认 5min(1s-24h) | cached_tokens/cache_write_tokens/cache_discount 透传;response HIT 时 usage 全零+X-OpenRouter-Cache-Status | 透传口径随上游;response cache=字节全等+头 HIT | 无 session_id 首请求不粘→预热 0 命中;两类缓存需区分 | A(透传) | 先按响应头分缓存类型:prompt 缓存走 LCP,response 缓存走字节全等(usage 零合法) |
| LiteLLM | 薄封装 litellm_proxy.rs | 透传+自动翻译(cache_control→Bedrock cachePoint);自有 Redis/语义缓存 | 随上游,低于门槛静默跳过 | 随上游 | 随上游 | cached_tokens 透传;/v1/messages 路径可能不归一化 | 同上游;经 bug 史 | #9812 双计费;#27763 指标恒不涨;流式丢字段史 | B | 按路由路径分级信任;Bedrock/Vertex 的 /v1/messages 透传降级为客户端对账 |
| Portkey | compat 注册表(api.portkey.ai/v1) | 透传上游 | 随上游 | 随上游 | 随上游 | OpenAI 非流式原样透传;流式重建剥 cache;Anthropic 映射 read→cached_tokens | 同上游(非流式) | OpenAI 流式剥字段;Anthropic total 口径含 creation+read | B | OpenAI 流式 cached_tokens 不可信,用非流式对账或 TTFT 旁证 |
| vLLM | 薄封装 vllm.rs(VLLM_BASE_URL) | 自动(block 哈希;LoRA/salt 入哈希) | 16-token(1 block) | 16-token block 对齐 | LRU+free queue,引用中不淘汰 | V0=prompt_tokens_details.cached_tokens;V1 恒 null(#44961);指标 prefix_cache_hits | cached%16==0;cached≤已发前缀;salt 不同 0 命中合法 | V1(默认引擎)usage null,14+ 个月未修 | B(V0)/C(V1) | V1 部署用 prefix_cache_hits+TTFT 端到端验证,不信 usage |
| SGLang | 薄封装 sglang.rs | RadixAttention;--enable-cache-report+extra_body return_cached_tokens_details | 1 token(page_size=1) | token 级(radix tree) | LRU | prompt_tokens_details.cached_tokens(需开启) | cached≤已发前缀 LCP | #25972 PD disaggregation 双重计数 | B | 先查部署是否 PD disaggregation;是则降级 |
| Ollama | 薄封装 ollama.rs | KV 自动(keep_alive 5min 默认) | 无公开 | 未公开 | keep_alive | 无缓存字段,官方自认恒报 0(#15758) | 无可对账字段 | 官方 issue 承认缓存存在但不报数 | C | 只用 eval_count/TTFT 旁证+客户端 LCP 期望,无服务端数 |
| one-api/new-api | 无专行;OpenAI 兼容自定义 base_url | 无(转发上游) | 随上游 | 随上游 | 随上游 | one-api 重造 usage 剥 cached_tokens(misc.go);new-api 未检到[UNVERIFIED] | 无字段可对账 | 非流式+流式均剥;new-api 源码检出不全 | C | 网关后 usage 不可信:直连上游或纯客户端 LCP |

## 二、补查结果(带 URL)

1. **Azure gpt-5.6 缓存粒度**:Azure 官方文档(2026-07-17 更新)仍写"前 1024 token 完全一致,之后每 128 个附加 token 命中一次"——**保持 128-token 粒度,未采用 OpenAI 原生 5.6 的字节精确查找**;并明确:5.6 的 usage **不单独上报 cache_write_tokens**;不支持 prompt_cache_options/breakpoint;prompt_cache_key 与前缀 hash 组合,同 key 超 ~15 RPM 部分请求漏命中;5.6+ 默认 24h 保留且不支持 in_memory。URL:https://learn.microsoft.com/en-us/azure/foundry/openai/how-to/prompt-caching ; 旁证(社区,2026-07-12):显式缓存字段"接受但从不生效" https://community.openai.com/t/prompt-caching-is-a-core-gpt-5-6-feature-why-are-customers-still-reverse-engineering-it/1386612
2. **Groq**:官方 docs:自动前缀缓存、命中不保证、volatile 存储 **2h 无使用过期**、缓存 token 50% 折扣、cached 不计 rate limit;支持模型目前**仅 GPT-OSS 3 个**(openai/gpt-oss-20b/120b/safeguard);usage 报 **cached_tokens**(OpenAI 兼容,pydantic-ai #5981 证实存在但曾被其丢弃)。与 round-2-d2c 中 OpenRouter 缓存页"Groq(仅 Kimi K2)"不矛盾——那是 OpenRouter 上 Groq 托管 Kimi 的缓存,非 Groq API 自身。URL:https://console.groq.com/docs/prompt-caching ; https://github.com/pydantic/pydantic-ai/issues/5981
3. **Qwen(Alibaba Model Studio/DashScope)**:2026-07-11 文档:**显式+implicit 双模且互斥**。显式=Anthropic 式 cache_control,≤4 断点+20-block 回看,块≥1024 token,TTL 5min(命中重置),写 125%/读 10%;implicit=自动,≥256 token,读 20%,命中不保证。上报:prompt_tokens_details.**cache_creation_input_tokens** + cached_tokens(Anthropic 命名);支持 Qwen 3.5-3.7 全系+DeepSeek-v3.2+Kimi+GLM-5.1,另有 OpenAI Responses 兼容 session cache。URL:https://www.alibabacloud.com/help/en/model-studio/context-cache ; 注意:OpenRouter 侧 Qwen 仅显式且快照端点不支持(round-2-d2c),与原生双模不同。
4. **Vertex/Gemini 3.x 门槛(2026-07-30)**:无下调,Gemini 3 家族=4096(3.5 Flash、3.1 Pro Preview 均 4096),2.5 Flash/Pro=2048;usage=total_cached_tokens。URL:https://ai.google.dev/gemini-api/docs/caching ; https://docs.cloud.google.com/gemini-enterprise-agent-platform/models/context-cache/context-cache-overview

**附带补查 Moonshot/Kimi**:自动缓存全模型、无需配置、系统管理 TTL,前序请求 >256 token 才入缓存;上报 cached_tokens(OpenAI 兼容,AI Gateway 仪表盘证实正确);缺陷:tool+interleaved thinking 时丢命中(reasoning_content 须完整保留,forum.moonshot.ai/t/216);K2.6 via OpenRouter 成本异常(社区)。URL:https://platform.kimi.ai/docs/guide/use-context-caching-feature-of-kimi-api ; https://github.com/vercel/ai/issues/13907

Groq/Kimi/DashScope 在 aimux 均为 compat 注册行(OpenAICompatProfile),补查后可直接并入矩阵正式行。

## 三、关键结论

- 完成标准达成:矩阵 19 行 ≥14 家;4 项补查均有官方文档 URL。
- 最重要的新增事实:**Azure 与 OpenAI 原生对 gpt-5.6 粒度口径不同(128-token vs 字节精确),且 Azure 5.6 不报写侧** → 审计量化规则必须按 endpoint 分派,不能全局一套。
- 剩余不确定性:vLLM V1 null 是否已修(快照 2026-06 仍坏);Gemini API 侧 TTL 未公开;new-api cached_tokens 未检出;Cohere/Mistral/xAI 粒度未公开;Groq 模型面极窄(仅 GPT-OSS)。
