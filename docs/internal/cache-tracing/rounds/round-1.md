# Round 1 探索报告存档(2026-08-01)

> 存档各 agent 原始发现,供追溯。完整原始文本在会话记录中;此处为核心内容 + 来源 URL。

## Agent 研究A — D1 缓存机制原理

### KV cache 物理机制
- KV cache = prefill 产生的 attention K/V 张量;缓存复用相同前缀 K/V,跳过重复 prefill。OpenAI 官方:"Key/value tensors are the intermediate representation from the model's attention layers produced during prefill"(extended retention 只卸载 KV 不存原文)。来源:https://developers.openai.com/api/docs/guides/prompt-caching
- vLLM:"cache the kv-cache blocks of processed requests, and reuse these blocks when a new request comes in with the same prefix"。来源:https://docs.vllm.ai/en/stable/design/prefix_caching/
- SGLang RadixAttention:token 级 radix tree,命中时只算新增 token prefill。来源:https://sgl-project-sglang-93.mintlify.app/concepts/radix-attention

### 粒度
- vLLM:block 哈希,默认 block size=16 tokens(源码 vllm/config/cache.py DEFAULT_BLOCK_SIZE=16);hash(parent_hash, block_tokens, extra_hashes)含 LoRA ID/多模态 hash/cache_salt;只缓存完整 block,partial block 不命中;v0.11 默认 sha256。来源:vLLM docs + GitHub raw
- OpenAI:最低 1024 token,128-token 增量(2024-10 公告原文:"starting at 1,024 tokens and increasing in 128-token increments");路由 hash 用前 ~256 tokens。来源:https://openai.com/index/api-prompt-caching/ + Azure 文档
- Anthropic:content-block 断点 + 累积前缀哈希;读时回看 20 block;最多 4 断点;最低可缓存长度按模型 512/1024/2048/4096。来源:https://docs.anthropic.com/en/docs/build-with-claude/prompt-caching
- DeepSeek:cache prefix unit 整单元匹配;64-token 存储单元(研究C 补);SWA 影响。来源:https://api-docs.deepseek.com/guides/kv_cache
- Gemini:门槛 2.5 Flash/Pro 2048、3.1 Pro Preview 4096、3.5 Flash 4096(研究C)。来源:https://ai.google.dev/gemini-api/docs/caching

### 命中条件与失效
- vLLM:采样参数不进哈希;extra hashes 含 LoRA/多模态/cache_salt
- Anthropic 失效表:tool 定义改→tools/system/messages 全失效;tool_choice 改→tools+system;图片增删→tools+system;thinking/effort 模型相关;web search toggle→tools 段
- OpenAI 可缓存内容:messages+图片(detail 一致)+tools+schema(注入 system 前缀)

### TTL
- OpenAI in-memory 5-10min 不活跃淘汰,off-peak 最长 1h;extended 24h;5.6+ ttl=30m
- Anthropic 5m 默认/1h 可选;命中刷新免费
- Gemini/Vertex implicit ≤24h;Gemini API 侧 TTL 未公开 [UNVERIFIED]
- DeepSeek 磁盘缓存数小时-数天;vLLM LRU + free queue

### agent loop 场景
- Anthropic 20-block lookback:每轮新增 ≥20 block 则断链
- **GPT-5.6+ implicit breakpoint 默认在最新 user/tool 消息处断点,不回退最长匹配前缀 → agent loop cached_tokens=0 正常**
- DeepSeek A+C 第二轮不命中但 A 落盘第三轮命中
- vLLM/SGLang 追加式自动命中

### TTFT 第二信号
- 命中跳过 prefill → TTFT 降(定性);OpenAI 官方:prompt 减半延迟只改善 1-5%,TTFT 噪声大 → 只做同会话相对比较
- vLLM 指标:vllm:prefix_cache_hits / vllm:prefix_cache_queries / vllm:time_to_first_token_seconds

### 审计启示
- 期望上限公式:expected_cached ≤ max(prefix_match(serialize(req_i), serialize(req_j))) − 粒度损失 − TTL 窗口
- 合法 0 命中 4 种:DeepSeek 单元、5.6 implicit 断点、跨机路由、低于门槛
- 需每 provider canonical serializer;容差窗口而非严格相等

## Agent 研究B1 — OpenAI 缓存细节

来源:https://developers.openai.com/api/docs/guides/prompt-caching (全文) / https://openai.com/index/api-prompt-caching/ (wayback) / Azure 文档 / community.openai.com 4 帖

- 触发:≥1024(5.6 strict minimum);旧模型 1024-2048;128-token 增量 5.6 已失效(精确字节级查找,计数非 128 对齐,社区帖 1386887)
- 写缓存:5.6+ 1.25× uncached 价,cache_write_tokens 计费;5.6 前免费
- prompt_cache_key:5.6+ 启用更可靠匹配;~15 RPM/key 超限漏命中
- 上报:chat=usage.prompt_tokens_details.cached_tokens;responses=usage.input_tokens_details.cached_tokens;流式 include_usage 才出现、中断丢失
- 5.6+ implicit breakpoint 语义(官方原文);显式 prompt_cache_breakpoint(mode explicit/implicit);4 写/50 读上限
- 确定性:官方不承诺;预热期 cached=0 常态
- 缺陷:①2025-01 计费事故:API 报 90%+ 命中但账单几乎无 cached input,成本 1.7× 应收,员工确认 overcharge(community.openai.com/t/dashboard-usage-vs-prompt-response-usage-not-matching/1078218)②图片 token 不入 usage ③gpt-5-nano/mini 长期不命中 ④Azure Responses API 0% 命中+参数接受但无效;Azure 不支持 cache options/breakpoint ⑤流式中断丢 usage ⑥schema 字段改名 cached 1024→0(帖 967577)⑦5.6 implicit 变化尾部重写全 prompt

## Agent 研究B3 — Anthropic usage 上报语义

来源:platform.claude.com/llms-full.txt(官方全文导出,已下载 /tmp/anthropic-llms-full.txt)+ Wayback 2024-10 存档 + GitHub issues

- 三字段公式(官方):total_input = cache_read + cache_creation + input_tokens;input_tokens 只算最后一个断点之后
- cache_creation 对象含 TTL 分层(ephemeral_5m/1h_input_tokens);ITPM 只计 input+creation
- 流式:cache 字段在 message_start;message_delta.usage 累计值(langchainjs #10249 double-count)
- 计费:5m 写 1.25×、1h 写 2×、读 0.1×
- 静默 miss:低长度/20-block/并发首请求 → 重算全 prompt + 新 creation;cache_read==0 && creation>0 = 疑似 miss
- 缺陷:litellm #9812 双重计费;**TTL 默认 1h→5m 静默回归(claude-code #46829,11.9 万次调用日志,成本 +15-53%)**;自动缓存 server tool 结果强制 5m 断点;Batch best-effort 30-98%
- 官方 Cache diagnostics beta(header cache-diagnosis-2026-04-07):用上次 response id 对比请求差异定位 miss

## Agent 研究C — D2b Gemini/DeepSeek/Bedrock/Azure

### Gemini(https://ai.google.dev/gemini-api/docs/caching + Vertex blog)
- implicit 默认开启(2.5+);门槛 2048/4096 按模型;usage.total_cached_tokens / cachedContentTokenCount(显式);流式 usageMetadata 流末尾 chunk 不稳定
- implicit ≤24h 清除(Vertex);Gemini API TTL 未公开 [UNVERIFIED]
- 缺陷:G15 命中率 40-60% 不稳定(python-genai #1880);G16 gemini-3-flash-preview 账目异常(官方:preview 缓存池显著更小,discuss #113959)
- explicit CachedContent:默认 60min TTL,model 必填不可变

### DeepSeek(https://api-docs.deepseek.com/guides/kv_cache + news0802)
- 磁盘缓存全量默认;64-token 存储单元整单元匹配;best-effort
- **不变量:prompt_tokens == hit + miss(官方)**;流式 include_usage 才报
- 计费 1/10;V4 价差约 120×(社区 gist [UNVERIFIED]);模型版本升级失效(社区);user_id 隔离

### Bedrock(https://docs.aws.amazon.com/bedrock/latest/userguide/prompt-caching.html)
- 可选;checkpoint ≥1024/4096 按模型;最多 4 个;TTL 5m/1h;顺序 tools→system→messages
- **不变量:total input = inputTokens + cacheReadInputTokens + cacheWriteInputTokens;启用缓存后 inputTokens 只含非缓存 token**
- API 字段:cacheReadInputTokenCount/cacheWriteInputTokenCount(flat camelCase,braintrust-sdk-rust #52)
- Nova 自动缓存(只降延迟);ConverseStream usage 事件位置待查

### Azure(https://learn.microsoft.com/en-us/azure/foundry/openai/how-to/prompt-caching)
- 默认开启不可关;≥1024 且前 1024 完全一致(单字符差异→0);之后 128 粒度
- prompt_cache_key(gpt-5.6+);不支持 prompt_cache_options/breakpoint;保留 in_memory(5-10min)/24h
- 社区:key 非确定性需预热(50 连发 ~80%;gpt-5 系列加 key ~95%)

## Agent 研究D — D6 aimux 代码库现状(全部经验证官代码抽查复核 ✓)

### 关键事实(含文件:行号)
- LanguageModel trait 两方法,Box<dyn>/Arc<dyn> 惯例,对象安全(aimux-core/src/language_model.rs:38-41)
- LanguageModelPrompt = Vec<LanguageModelPromptMessage>;system 是 role=System 消息;tools 在 CallOptions.tools(options.rs:68)
- GenerateResult 含 request_body/response_headers(result.rs:96-113);StreamResult 含 request_body/response_headers(result.rs:116-123)
- **stream_text 用户面丢弃 StreamResult.request_body(generate.rs:255-257);FFI aimux_stream_text 同样(lib.rs:468-493)** — 流式审计硬缺口
- Usage.raw 全库 None(死字段);Anthropic 生产路径丢 cache 字段(anthropic/stream.rs:180-192,454-469 只填 total;usage.rs:75-132 完整实现仅测试用)
- OpenAI 系正确映射 cached_tokens→cache_read + cache_write + saturating_sub no_cache(openai/model.rs:76-127;openai/responses/convert.rs:1004-1042);xai responses cache_read>input 兜底(xai/responses/convert.rs:47-59)
- StreamPart 定义在 aimux-core(stream_part.rs:17-166);Finish 带 usage;Raw{raw_value} 已定义无人发射
- OpenAI 流式 usage 覆盖式更新(model.rs:494-497);Anthropic 拼接式(message_start+message_delta);断流 Usage::default()
- 请求体构建在 provider 层:OpenAI(openai/convert.rs:960)/Anthropic(convert.rs:1116)/Google(convert.rs:834);request_body = 最终发出 JSON;workspace preserve_order 开启 → 字节级稳定
- OpenAICompatProfile + 宏(openai_compat.rs:29-98)覆盖 251 家 thin wrapper(验证官复核);全部复用 execute_generate/execute_stream
- 无中间件先例;tracing 0.1 依赖零使用;retry 内部不可见(http.rs:249-310)
- FFI:aimux_generate_text(lib.rs:390-413)整体序列化;aimux_stream_text(lib.rs:432-494)每 part 推送
- 性能:单请求 0.1ms 级、2000 请求零内存增长;流式 generator 借用陷阱先例(openai/model.rs:391-395)

### 推荐
- 主推荐:LanguageModel 外 wrapper(装饰器),不改 trait
- 配套:①stream_text 补 request_body 透传 ②FFI 流式透传 ③Anthropic 生产路径补 cache 字段 ④可选 Usage.raw 填充
- 审计以 Finish.usage 为准;容忍缺失;重试语义需定义

## 验证官 — Round 1 门控结论

- 判定:GAPS。代码声明 6/6 属实;外部核验 7 项全部属实(含 OpenAI 2025-01 计费事故、Anthropic TTL 回归、Cache diagnostics、4 写/50 读、15 RPM、implicit breakpoint)
- 问题:P1 证据卫生(报告未落盘、8+ 精确数字无标记);P2 内部矛盾(128-token 增量 D1 vs D2a);P3 盲点(request_body 覆盖率未盘点 251 compat、掺水方向单边化、无来源分级);P4 肤浅条目;P5 D3/D4/D5 零覆盖、provider 6/12
- 遗留:litellm #9812 未独立确认;"员工确认 overcharge""11.9 万次日志""15-53%"三个数字需下轮补源
