# Working Document: LLM Request Trace & Cache-Hit Audit

> 汇总各轮探索 agent 发现。按维度组织,不断更新。所有条目可追溯到来源 agent。

---

## D1 缓存机制原理(研究A, 已完成)

### 物理机制
- KV cache = prefill 阶段 attention 层产生的 K/V 张量;prefix caching 复用相同前缀的 K/V,跳过重复 prefill(OpenAI 官方:"Key/value tensors are the intermediate representation... not the prompts themselves";vLLM/SGLang 同)
- 缓存的是 **token 化后前缀的 K/V**,不是文本 → 客户端对比必须对齐 provider 渲染后的 prompt 顺序与格式

### 粒度
| Provider/引擎 | 粒度 | 细节 |
|---|---|---|
| vLLM | 16-token block 哈希 | 只缓存完整 block;extra hashes: LoRA ID、多模态 hash、cache_salt(多租户隔离) |
| OpenAI(2024-10 公告) | 128-token 增量,最低 1024 | 前 1024 一个字符不同 ⇒ 0 命中;**GPT-5.6+ 已改为精确字节级序列查找,128 对齐失效**(研究B1, 帖 1386887);GPT-5.5- 为 1024-2048 可变 |
| SGLang | token 级 radix tree | page_size=1 真正 token 级;16 时按 16-token 对齐 |
| Anthropic | content-block 断点 | 最多 4 断点;累积前缀哈希;读取回看窗口 20 block;最低长度按模型 512-4096 |
| DeepSeek | 64-token 存储单元 | 整单元完全匹配才命中;不足 64 不缓存 |
| Gemini | 未知(未公开) | 最低门槛 2048/4096 按模型 |

### 命中条件
- vLLM:相同 token 前缀 + 相同 extra hashes;temperature 等采样参数不进哈希
- OpenAI:exact prefix match + 路由同机(路由 hash 用前 ~256 tokens);可缓存内容 = messages + 图片(detail 一致) + tools 数组 + structured outputs schema(注入 system 前缀)
- Anthropic 失效表:tool 定义改 → 全部失效;tool_choice 改 → tools+system 失效;图片增删 → tools+system;thinking/effort → 模型相关;web search toggle → tools 段失效
- DeepSeek:完整匹配缓存前缀单元,前缀必须从 token 0 开始

### TTL/淘汰
- OpenAI:旧模型 in-memory 5-10 分钟不活跃淘汰(高峰外最长 1h);extended retention 24h(GPT-5.x/4.1);GPT-5.6+ prompt_cache_options.ttl=30m(最短保留)
- Anthropic:5m 默认,1h 可选(写价 2×);命中刷新 TTL 免费
- Gemini/Vertex:implicit ≤24h 清除(依负载);Gemini API 侧 TTL 未公开
- DeepSeek:磁盘缓存,闲置数小时-数天清除;构建需数秒
- vLLM:free queue + LRU;引用中 block 不可淘汰

### agent loop 场景(关键)
- Anthropic automatic caching:断点随对话前移,但 20-block lookback——每轮新增 block 数 ≥20 则断链 miss
- OpenAI 老模型:追加式对话天然命中(静态前缀+动态结尾)
- **⚠️ GPT-5.6+ implicit breakpoint:默认在最新 user/tool 消息处断点,不回退到断点前最长匹配前缀 → agent loop(后缀每次变化)即使共享数千 token 前缀,cached_tokens 也可能为 0。审计器最易误报"掺水"的场景**
- DeepSeek:第二轮 A+C 不命中 A+B 的单元,但公共前缀 A 会被落盘,第三轮命中——"看似共享长前缀却 0 命中"是合法行为
- vLLM/SGLang:纯 token 前缀,追加式多轮自动命中

### TTFT 第二信号
- 命中跳过 prefill → TTFT 下降(定性确认);但 OpenAI 官方:输入 token 减半延迟只改善 1-5%,TTFT 噪声大(路由/队列/负载)→ 只能同 provider+model+会话内相对比较
- vLLM 暴露 vllm:prefix_cache_hits / prefix_cache_queries 指标(自托管端到端验证用)

### 对审计的启示(研究A 建议)
1. 期望命中上限公式:`expected_cached_tokens ≤ max_over_window(prefix_match_len(serialize(req_i), serialize(req_j)))` − 粒度取整损失(128-token/16-block/20-block) − TTL 窗口外无效
2. 合法 0 命中 ≠ 掺水 4 种情况:DeepSeek 单元粒度;GPT-5.6+ implicit 断点;跨机路由;低于最低门槛
3. 判定要输出"期望命中区间"而非单点
4. **canonical serializer 需求已由 D6 化解:request_body 就是最终发给服务的 JSON(preserve_order 确定性序列化)→ 审计基准字节可直接取,无需为 20+ provider 各写 serializer,只需规范化(剔除噪声字段、NFC);语义级诊断再辅以 LanguageModelPrompt 层对比**
5. 字符级 vs token 级口径差;建议 tokenize 后对比或保守上界
6. 容差窗口而非严格相等(边界 ±1 block)

---

## D2a OpenAI 与 Anthropic(研究B1+B3, 已完成)

### OpenAI(研究B1)
- 自动缓存 ≥1024 token(5.6+ strict minimum);旧模型 1024-2048 可变;128-token 增量对 5.6 已失效(精确字节级序列查找,命中计数非 128 对齐)
- 有效期:in-memory 5-10 分钟不活跃清(off-peak 最长 1h);extended 24h(GPT-5.x/4.1, KV 卸载 GPU-local storage);5.6+ prompt_cache_options.ttl=30m
- 写缓存收费:5.6 前免费;5.6+ 1.25× uncached input(cache_write_tokens)
- cache key: messages + 图片(detail 一致) + tools + structured output schema(注入 system 前缀);temperature 等不参与([UNVERIFIED] 官方未明说);路由 = 前 ~256 token 哈希 → 同机才命中
- prompt_cache_key:5.6+ 启用"更可靠匹配"需设置;每 key ≈15 RPM 超限漏命中;高流量拆多 key
- 上报:chat = usage.prompt_tokens_details.cached_tokens / cache_write_tokens(5.6+);responses = usage.input_tokens_details.cached_tokens;流式仅 include_usage=true 时最终 chunk 出现,中断即缺失
- **5.6+ implicit breakpoint:默认在最新 user/tool 消息处断点,不回退到断点前最长匹配前缀 → agent loop(尾部变化)即使共享数千 token 前缀 cached_tokens=0 是正常行为**
- 显式断点:prompt_cache_breakpoint(mode: explicit/implicit);不支持 block 上打标记 → 400;旧模型拒绝参数
- 配额:每请求最多 4 个新写(implicit 占 1);读时考虑最近 50 个断点;多断点取最长匹配
- 确定性:官方不承诺命中;预热期 cached=0 常态(社区实测第 1-4 次 0,~128s 后第 5 次才中;50 连发热身后 ~80%;gpt-5/5-mini + key 第二发即中 ~95%)
- **已知上报缺陷**:
  1. **2025-01 计费事故:API 报 90%+ 命中率但 Usage dashboard/账单几乎无 cached input(实测成本 1.7× 应收),员工确认 overcharge** → 上报偏高有实锤历史
  2. 图片 token 不计入 usage 响应 → 图片多时命中率被高估
  3. gpt-5-nano/mini 长期不命中(官方无解)
  4. Azure Responses API 命中率 0% + 显式缓存参数"接受但无效";Azure 官方不支持 prompt_cache_options/breakpoint,5.6 不报 cache_write_tokens
  5. 流式中断丢 usage
  6. response_format schema 字段改名 → cached 1024→0(前缀对比必须含 response_format/tools/图片 detail)
  7. 5.6 implicit 模式变化尾部请求在重写全 prompt(cache_write 全量、cached=0),既没省钱又付写费

### Anthropic(研究B3)
- **usage 三字段公式(官方):total_input = cache_read_input_tokens + cache_creation_input_tokens + input_tokens;input_tokens 只算最后一个断点之后**
- usage 新增 cache_creation 对象(ephemeral_5m/1h_input_tokens TTL 分层);ITPM 只计 input+creation,read 不计
- 流式:cache 字段在 message_start 的 message.usage;**message_delta.usage 是累计值**(langchainjs #10249 因此 double-count 2×)
- 计费:5m 写 1.25×、1h 写 2×、读 0.1×;命中刷新免费
- **静默 miss:低于最小缓存长度/20-block 回看找不到/并发首请求 → 不报错重算全 prompt,产生新 creation;cache_read==0 && cache_creation>0 = 疑似 miss 事件**
- 已知缺陷:
  1. 代理层双重计费(litellm #9812: input+cache_creation 相加再收一次,费用近翻倍)
  2. 流式 double-count(#10249)
  3. **TTL 默认值回归(2026-03 前后):默认 TTL 从 1h 静默退回 5m,写成本 +20-32%(claude-code #46829,11.9 万次调用 JSONL 实证,官方未公告)**
  4. 自动缓存下 server tool 结果自动加 5m 断点,即使全程 1h TTL,usage 仍现 ephemeral_5m 写(审计陷阱)
  5. Batch API 官方自认 best-effort,命中率 30%-98% 视流量
- **官方新工具:Cache diagnostics(beta, header cache-diagnosis-2026-04-07,仅 Claude API):用上次 response id 对比两请求差异定位 miss 原因 → 建议 aimux 对接**



---

## D2b Gemini/DeepSeek/Bedrock/Azure(研究C, 已完成)

### Gemini
- implicit caching 默认开启(2.5+);门槛:2.5 Flash/Pro 2048、3.1 Pro Preview 4096、3.5 Flash 4096
- 上报:usage.total_cached_tokens(SDK)/ cachedContentTokenCount(显式,Vertex);流式 usageMetadata 在流末尾 chunk(不稳定,preview 模型时有时无)
- 折扣:命中 10%(90% 折扣);implicit 保留 ≤24h(Vertex),Gemini API 侧 TTL 未公开 [UNVERIFIED]
- 已知缺陷:G15 命中率不稳定 40-60%(python-genai #1880);G16 gemini-3-flash-preview 缓存账目异常(官方回复:preview 模型缓存池显著更小)
- explicit CachedContent:默认 60 分钟 TTL,model 必填不可变;Interactions API 只支持 implicit

### DeepSeek
- 磁盘缓存全量默认开启;64-token 存储单元,整单元完全匹配;best-effort 不保证 100%
- **不变量:prompt_tokens == prompt_cache_hit_tokens + prompt_cache_miss_tokens(官方文档)**
- 流式:stream_options.include_usage 时 [DONE] 前一个 chunk 带 usage
- 计费:命中 0.1 元/M vs 1 元/M(1/10);V4 后价差约 120×(社区 gist,[UNVERIFIED]);模型版本升级失效(社区,[UNVERIFIED]);user_id 隔离
- 失效:模型版本升级权重更新 → 全部缓存失效(社区确认,官方未声明);user_id 参与缓存隔离
- 社区对上报准确性无公开质疑;主要讨论命中率预热与计费

### Bedrock
- 可选特性,on-demand 端点;checkpoint 最低 1024/4096 按模型,最多 4 个;TTL 5m 默认,部分 1h;顺序 tools→system→messages
- **不变量:total input = inputTokens + cacheReadInputTokens + cacheWriteInputTokens;启用缓存后 inputTokens 只代表非缓存 token**
- API 字段名:cacheReadInputTokenCount / cacheWriteInputTokenCount(flat camelCase)
- Claude 简化管理:单 checkpoint + 自动 20-block 回看;Nova 自动缓存(只降延迟,省钱要显式)
- ConverseStream 流式 usage 事件位置待查

### Azure OpenAI
- 默认开启不可关;≥1024 且前 1024 完全一致(单字符差异 ⇒ 0);之后 128-token 粒度
- prompt_cache_key(gpt-5.6+);不支持 prompt_cache_options/breakpoint;保留策略 in_memory(5-10min)/24h
- 路由:前 ~256 token 哈希
- 社区实测:prompt_cache_key 非确定性、需预热(每 2 秒 50 发约半程后 ~80% 命中;gpt-5 系列预热后 ~95%)

---

## D6 aimux 代码库现状(研究D, 已完成)

### 请求链路
- LanguageModel trait: do_generate / do_stream;Box<dyn>/Arc<dyn> 惯例;对象安全
- LanguageModelPrompt = Vec<LanguageModelPromptMessage>;无独立 system/tools 字段(system 是 role=System 的消息;tools 在 CallOptions.tools)
- 用户面 generate_text/stream_text(generate.rs:159/239)→ CallOptions → do_generate/do_stream

### 响应侧
- GenerateResult: usage + provider_metadata + **request_body: Option<Value>** + response_headers(result.rs:96-113)
- StreamResult: stream + request_body + response_headers;但 **stream_text 用户面丢弃 request_body(generate.rs:255-257),FFI 同样丢弃**——流式审计硬缺口
- Usage/TokenUsage 已有 cache_read/cache_write/no_cache/types.rs:33-62;Usage.raw 全库为 None(死字段,无人填)
- **Anthropic 生产路径丢 cache 字段**:anthropic/usage.rs:75-132 有完整映射但只有测试用;生产(stream.rs:180-192, 454-469)只填 total
- OpenAI 系正确映射 cached_tokens→cache_read(cache_write→cache_write, saturating_sub 算 no_cache);xai responses 有 cache_read>input 兜底

### 流式
- StreamPart 定义在 aimux-core(stream_part.rs:17-166);Finish 带 usage;Raw{raw_value} 已定义无人发射
- OpenAI 流式 usage 覆盖式更新;Anthropic 拼接式(message_start input + message_delta output);断流时 Usage::default()
- 流式 usage 语义不统一 → 审计以 Finish.usage 为准

### 协议转换
- 请求体构建在 provider 层:OpenAI build_request_body_with_warnings_fallible(openai/convert.rs:960)、Anthropic(convert.rs:1116)、Google(convert.rs:834)
- **request_body 就是最终发给服务的 JSON;workspace preserve_order 开启(Cargo.toml)→ serde_json 序列化确定,key 顺序稳定 → 字节级前缀对比成立**
- OpenAICompatProfile + declare_openai_compat_provider! 宏覆盖 ~100 家 thin wrapper,全部复用 execute_generate/execute_stream → 接 OpenAI 核心路径即可覆盖绝大多数 provider
- Anthropic AWS SigV4 走精确字节(BodyEncoding::Bytes)

### 中间件先例
- 无任何 wrapper/拦截器模式;tracing 0.1 依赖存在但零使用;retry 在 http.rs send_with_retry_raw 内部,重试次数对用户不可见

### 性能约束
- 单请求 0.1ms 级、2000 请求零内存增长 → trace 数据不应全局累积,应随结果一次性返回;前缀对比 O(body) 相对 3-10s LLM 请求可忽略
- 流式 generator 内加钩子注意借用(先例:profile 不能移入 generator,需提前捕获 openai/model.rs:391-395)
- wrapper 只要 Send+Sync 即无破坏

### FFI
- aimux-ffi 单文件 1031 行全 JSON:aimux_generate_text(lib.rs:390-413)整体序列化 GenerateTextResult(raw 含 request_body);aimux_stream_text(lib.rs:432-494)每 part 序列化推送
- 非流式 trace 数据量大(200KB 上下文时 JSON 输出翻倍)→ FFI 侧需考虑裁减选项

### request_body 覆盖率盘点(验证官 P3,主调查补查 2026-08-01)
- 设置 request_body 的文件(生成+流式双路径):openai/model.rs:339,746;anthropic/stream.rs:205,507(Anthropic 非流式也走 stream.rs);google/model.rs;cohere/model.rs;mistral/model.rs;bedrock/model.rs;xai/model.rs + xai/responses/mod.rs;vertex/model.rs + vertex/anthropic_model.rs;huggingface/responses.rs;open_responses.rs;openai/responses/mod.rs + responses_convert.rs;azure/responses.rs
- 251 家 compat provider:宏包装 `OpenAIProvider`(openai_compat.rs:62-72)→ 统一走 execute_generate/execute_stream → **request_body 全覆盖**
- 结论:原生 LLM 调用双路径均有 request_body,审计数据源可用;唯一缺口是**用户面 stream_text 丢弃 StreamResult.request_body**(generate.rs:255-257),FFI 同样

### 推荐钩子(研究D)
- **主推荐:LanguageModel 之外的 wrapper(装饰器)**——不改 trait、不动 172 个实现、不碰热路径
- 配套暴露面:①stream_text 补 request_body 透传 ②FFI 流式透传 request_body ③Anthropic 生产路径补 cache 字段 ④可选 Usage.raw 填充
- 审计以 Finish.usage 为准;容忍缺失;重试语义需定义

---

## 跨维度综合(临时)

### 已确认的官方不变量(审计基准)
| Provider | 不变量 | 来源 |
|---|---|---|
| DeepSeek | prompt_tokens == hit + miss | 官方文档 |
| Bedrock | input + cacheRead + cacheWrite == total input | 官方文档 |
| Azure/OpenAI | 前 1024 token 单字符差异 ⇒ cached_tokens=0;128 粒度量化 | 官方文档 |
| Gemini | 低于门槛(2048/4096)不应有 implicit 命中 | 官方文档 |

### 掺水风险点(服务端可能报多)
1. Gemini preview 模型账目异常(G16)、命中不稳定(G15)
2. Bedrock 三字段混算易虚报(B4)
3. OpenAI 不保证命中;路由/eviction 导致 miss 但计费上 cached_tokens 是服务端自报
4. 网关/代理转发改写 usage(待 D3 调查)
5. 流式 usage 丢失后客户端补算(待 D3)

### 合法低命中 ≠ 掺水
- DeepSeek 单元粒度、GPT-5.6+ implicit 断点、跨机路由、低于门槛、模型版本升级、缓存池小(preview)、预热期、跨 API key 不共享缓存、OpenRouter 非 sticky 首请求

---

## D3 usage 上报可信度(研究E1, 已完成)

### 实锤案例
- OpenAI 2025-01 计费事故:API 报 90%+ 命中但账单 input_cached_tokens=0,输入按 1.7× 计费;员工确认修复 → **usage 字段与计费口径可系统性不一致**(community.openai.com/t/1078218)
- Anthropic TTL 1h→5m 静默回归:claude-code #46829,119,866 次调用 JSONL 实证,写成本 +20-32%
- litellm #9812 双重计费(已修 #9838);Langfuse #12306 口径叠加 2×(命中率显示 50% 实为 99%);new-api #6144 自记账残缺副本
- OpenRouter 响应级缓存:命中时 usage 全零 + X-OpenRouter-Cache-Status: HIT 头
- 流式:OpenAI 中断丢末 chunk;Anthropic usage 只在 message_delta 尾

### 硬不变量(按置信度排序)
1. **命中域 ⊆ 之前发过的前缀(claimed_cached > 客户端 token 级 LCP → 不可能)**
2. DeepSeek: hit + miss == prompt_tokens
3. Anthropic: 三字段=总输入;首请求 cache_read==0;cache_read ≤ 历史写过未过期;TTL 档位匹配请求参数
4. Bedrock: quota = input + cacheWrite + output×burndown,read 不计 quota
5. OpenAI: <1024 → 0;128 步长量化(5.6-);5.6+ cache_write_tokens
6. 时序:共享前缀间隔 > TTL 不可能命中

### 掺水判定置信度
- 高置信(硬判):违反 1/2/3/5
- 中置信:usage vs 账单漂移;流式非流式不一致;网关自填 usage
- 低置信(旁证):TTFT 不随命中率改善;命中率>95% 成本不降
- 取证:逐请求记录 {prompt 哈希、token 数、usage、TTFT、到达时间} 离线跑全部不变量

---

## D4 agent loop 识别与前缀连续性(研究E2, 已完成)

### 案例
- TradingAgents #750:0% 命中,5 条根因(动态 system 注入/prompt.partial/清历史/f-string 嵌动态报告)
- openclaw #49700:143 调用 cacheRead 仅 10.5%,60-120KB 单块动态 system → 拆 static+dynamic 两块修复
- Claude Code:缓存键 = 内容前缀 + model + effort level;三层结构;model 切换整会话 0 命中
- agent loop 每轮重发全量历史(5 次工具调用任务 71% token 冗余);**assistant 消息不适合做缓存锚点**

### 破坏前缀连续性模式
system 动态注入 / 历史压缩重写 / tools 定义变化 / 模型混用 / 并行竞态(首个写其余读)

### 识别特征
历史 append-only 超集、同 (provider,model) 连续、tool_use→tool_result 回填、TTL 时间窗
- OTel GenAI: gen_ai.conversation.id、gen_ai.usage.cache_read.input_tokens(v1.40+)
- **provider 缓存不认 session,只认前缀+model+effort**

### 分段期望模型(核心设计输入)
- system/tools 段:可跨 session 命中(匹配空间放宽到任意更早请求)
- conversation 段:仅同 session append-only 链可命中
- 同 session 连续但 0 命中 → 前缀被破坏;报命中但无含该前缀的历史请求 → 掺水信号

---

## D5 字符串对比与指纹算法(研究E3, 已完成)

### Canonicalization
- 审计基准 = 规范化 request_body(preserve_order + NFC + 剔除噪声字段);语义级对比用规范化 LanguageModelPrompt(诊断 UX)
- byte-level BPE 单射:字节相同 ⟺ token 相同 → **字节 LCP 是服务端匹配的必要条件,可直接证伪多报**

### LCP 算法(推荐)
- **vLLM 式块哈希链**:4KB/128-token 块,h_i = H(key, h_{i-1}, block_i);首失配块 memcmp 精确化;均摊 O(δ+B)/轮;内存 O(L/B)
- 每请求存:128-bit xxh3 + 块哈希链 + 计数;块哈希→请求 id 索引
- 脱敏:HMAC 会话密钥哈希,绝不落盘明文
- 可选 token 级:tiktoken-rs(cl100k/o200k/deepseek_v3 等),增量编码 O(δ)

### 总管线
规范化 request_body → 块哈希链增量 LCP(xxh3-128)→ memcmp 精确化 → (可选) token 级 → 与上报对账。O(δ+B)/轮,零堆分配

### 不确定性(E3 标注,保留)
- Anthropic cache_read 上报精确度 [UNVERIFIED];1 token≈5.6B [UNVERIFIED];simdutf 是否支持 NFC [UNVERIFIED];块哈希链+客户端对账无现成库(原创组合,基于 vLLM 已验证设计)

---

## D2c 网关/自托管(研究E4, 完整报告 round-2-d2c.md)

### 可信度分级
- **A 级可直接审计**:OpenRouter 透传、Cohere v2 usage.cached_tokens、Mistral(64-token 粒度,读价 10%)、xAI/Grok、vLLM V0(--enable-prompt-tokens-details)
- **B 级有坑**:SGLang(--enable-cache-report 时可用,但有 #25972 双重计数 bug 污染 usage.cached_tokens,需按部署形态降级)、LiteLLM(流式丢字段;#9812 #27763)、Portkey(流式 OpenAI 剥 cached_tokens)、ferro(扁平 cache_read_tokens)
- **C 级报不出 → 降级**:vLLM V1(默认引擎,prompt_tokens_details 恒 null,#44961 14 个月未修)、Ollama(官方自认永远报 0)、llama.cpp、TRT-LLM、one-api、simple-one-api 非流式剥字段
- OpenRouter sticky routing(同 provider 端点;session_id 钉路由);OpenRouter/LiteLLM 响应级缓存是第二类合法缓存(usage 归零),审计需区分
- vLLM cache_salt:多租户隔离,不同 salt 不命中正常
- 自托管端到端首选 vllm:prefix_cache_hits 指标



---

## 待办
- [x] Round 1: D1 / D2a(OpenAI+Anthropic)/ D2b / D6 完成(验证官:GAPS,P1-P5)
- [x] 修复 P2: 128-token 增量矛盾已 reconcile(5.6+ 失效)
- [x] P1: agent 报告已落盘 rounds/round-1.md
- [x] Round 2: D3 / D4 / D5 / D2c 完成;报告 rounds/round-2.md + round-2-d2c.md;request_body 覆盖率盘点完成(全覆盖)
- [x] Round 2 验证官修复:TTL 双数 reconcile(20-32%)、[UNVERIFIED] 恢复、D1↔D6 连接、SGLang 降级
- [x] Round 3: 判定规则表(round-3-verdict-rules.md)/ trace 数据模型(round-3-trace-data-model.md)/ TraceStore 设计(round-3-design.md)/ provider 矩阵 19 行(round-3-provider-cache-audit-matrix.md)
- [x] Round 3 验证官(3 号):GAPS(轻-中)→ 修复 G1-G11(全部落实,验证官4 复核 11/11)
- [x] Round 4: P0 集成方案 + 绑定影响评估(round-4-p0-integration-binding.md)/ 原型 12/12 绿(round-4-prototype-report.md, prototype/ 可运行)
- [x] Round 4 验证官(4 号):**PASS**;F1-F3 定案已修(OpenAI TTL 60min、字节代理 W 封顶 B、U 块上界 (j+1)B)
- [x] 最终设计文档 rfc/0015-cache-trace-audit.md(RFC 惯例在仓库根 ./rfc/,0014 已被 logging 占用;RFC-0014 引用本子系统的 span 树挂载点)
