# Round 2 探索报告存档(2026-08-01)

> D2c 完整报告见 round-2-d2c.md(agent 自行落盘)。此处为 E1(D3)/E2(D4)/E3(D5) 核心内容。

## 研究E1 — D3 usage 上报可信度

### 服务端自身案例
- **OpenAI 2025-01 计费事故(已核验)**:API 响应报 90%+ cached_tokens,但 Usage Dashboard/usage API 的 input_cached_tokens=0,gpt-4o 输入按 1.7× 计费;员工 VeitB 确认 credits 扣除与 cache hit 不符,1/7 修复。→ **usage 字段与计费口径可系统性不一致**。来源:community.openai.com/t/1078218(全文已读)
- **Anthropic TTL 静默回归(已核验)**:claude-code #46829 用 119,866 次调用 JSONL(ephemeral_5m/1h 分层字段)证明 1h→5m 静默回归,写成本 +20-32%,quota 激增;closed as not planned,团队承认调查中
- Anthropic 计费口径漂移:旧文档 input_tokens 含 cache_creation,现文档三字段互斥
- Gemini billing 帖子(snippet 级,[UNVERIFIED])

### 网关层改写
- litellm #9812(属实已修):cache_creation_input_tokens 双重计费,成本 1.7×;PR #9838 修复
- **Langfuse #12306**:pydantic-ai 按 OTel 把三字段求和为 input_tokens,Langfuse 再 +cache → 2×,命中率显示 50% 实为 99%(口径叠加 bug)
- **OpenRouter 响应级缓存**:完全相同请求命中时 usage 全零、不产生计费,靠 X-OpenRouter-Cache-Status: HIT 区分 → 客户端把 0-usage 当真实 token 数会系统性漏报
- new-api #6144:上游 usage 正确但自记账用残缺副本,流式缓存命中不计费;中转站"命中仍按全价收用户"是掺水动机模型

### 流式失真
- OpenAI:interrupt/cancel 可能收不到含总 usage 的末 chunk;output 无法用 delta 精确计数
- Anthropic:usage 在 message_delta 尾事件,中断即丢

### 业界方案
- Helicone/Langfuse/Braintrust 均为被动采集,无人做缓存命中真伪校验;专门产品未找到
- 最接近审计实践:社区对账(usage vs 账单)、Claude Code 社区用 ephemeral 分层字段做 TTL 时序审计(#46829 方法)

### 硬不变量(按置信度排序)
1. **命中域 ⊆ 之前发过的前缀:claimed_cached > 客户端 token 级 LCP → 不可能(最高价值)**
2. DeepSeek: hit + miss == prompt_tokens(官方硬等式)
3. Anthropic: 三字段相加=总输入;首请求 cache_read==0;cache_read ≤ 历史写过未过期总量;TTL 档位字段匹配请求 ttl 参数
4. Bedrock: quota = input + cacheWrite + output×burndown,cacheRead 不计 quota
5. OpenAI(5.6 前): <1024 → cached==0;>1024 时 128 步长量化;5.6+ cache_write_tokens
6. 时序:共享前缀间隔 > TTL 后不可能命中

### 掺水判定置信度
- **高置信(硬判)**:违反不变量 1/2/3/5——首请求有命中、命中>LCP、hit+miss≠prompt、<1024 有命中、超 TTL 仍命中
- **中置信**:usage 与账单/配额端到端对不上(2025-01 式漂移);流式非流式 usage 不一致;网关未见 provider 响应自行填 usage
- **低置信/辅助**:TTFT 无随命中率改善;命中率>95% 但成本不降
- 取证建议:客户端逐请求记录 {原始 prompt 哈希、token 数、usage 字段、TTFT、到达时间} 离线跑全部不变量

## 研究E2 — D4 agent loop 识别与前缀连续性

### 真实案例
- **TradingAgents #750(0% 命中)**:16-22 次调用全 miss,多付 30-40%。5 条根因:①零缓存层 ②prompt.partial() 把 current_date/instrument_context/tool_names 注入 system(从第一个 token 分叉)③create_msg_delete() 清空消息历史换单条 "Continue" ④debate agent f-string 嵌动态报告 ⑤structured output 用裸 f-string。建议:动态值移到 user message,静态 system prompt 保留
- **openclaw #49700(实测)**:143 次连续调用,cacheWrite 138/143(96%),cacheRead 仅 15/143(10.5%);buildAgentSystemPrompt() 把静态+动态拼成 60-120KB 单块 system,动态节(日期时间/runtime/workspace/heartbeat)每请求变 → 修复=拆 static(cache_control)+dynamic 两块,命中率预期 10%→90%+
- **Claude Code 官方文档**:三层结构 system prompt(工具集变化才变)→project context(会话开始变)→conversation(每轮变);**缓存键 = 内容前缀 + model + effort level**(模型/effort 切换整会话 0 命中);/compact、fast mode、MCP 连接变化都全量重算
- LangChain Deep Agents:真实轨迹缓存降本 49-80%;cache bust 主因=加载新 skill/tool 修改 prompt 靠前部分;Manus 引语:"KV cache hit rate 是生产 agent 最重要的单一指标"
- Towards AI《Agent Loop Caching》:agent loop 每轮重发全量历史,5 次工具调用任务 71% token 冗余;**assistant 消息不适合做缓存锚点**(流式输出/max_tokens 截断/stop/extended thinking 使其 token 组成不稳定)

### 破坏前缀连续性的模式
- system 动态注入(TradingAgents/openclaw/时间戳)
- 消息顺序/历史不稳定(/compact、清历史、压缩)
- tools 定义动态变化(Anthropic 官方:改 tool 定义全缓存失效)
- 多模型混用(/model、fallback = 缓存清空)
- 并行请求并发:共享 base prompt 时"首个写、其余读"竞态(机制推断,无官方并发语义)

### 连续调用识别 + 业界方案
- 可判定特征:①历史 append-only,后次是前次超集 ②同 (provider, model) 连续 ③tool_use→tool_result 成对回填 ④时间窗约束(TTL)
- **OTel GenAI semconv**:gen_ai.conversation.id(Conditionally Required);gen_ai.operation.name=chat;invoke_agent/execute_tool;v1.40+ 有 gen_ai.usage.cache_read.input_tokens / cache_creation.input_tokens(OpenAI 约定)
- Langfuse sessionId(纯应用层分组);Helicone Helicone-Session-Id/-Path/-Name(Path=按功能分层)
- **provider 缓存不认 session,只认 token 前缀 + model + effort**

### 关键审计洞察(分段期望模型)
- 命中只依赖前缀精确匹配,与 session 无关:跨 session 共享 system prompt 命中是预期行为
- 审计匹配空间应放宽到"任意更早请求的相同前缀"(跨 session),而非限定同 session
- 同 session append-only 连续但命中率≈0 → 前缀被动态内容破坏(查 system/tools/参数)
- 服务端报命中、客户端却找不到任何含该长前缀的历史请求 → 可疑(掺水信号)
- **分段期望:system/tools 段可跨 session 命中;conversation 段仅同 session append-only 链上可命中——两端分别核对 cache_read 上报**

## 研究E3 — D5 字符串对比与指纹算法

### Canonicalization
- 对比必须落在"服务端实际收到的字节"而非语义对象
- JSON 规范形:serde_json::Value(preserve_order)+ to_string 重序列化
- Unicode NFC(仅客户端自洽;服务端 BPE 吃原始字节不归一化)
- 排除字段:随机 request_id/timestamp/nonce/stream;保留 messages/system/tools
- **双级对比:语义级 LCP 用规范化 LanguageModelPrompt(UX/诊断);审计基准用规范化 request_body(准绳)**

### LCP 算法
- 朴素逐字节 O(L)/轮 → 会话 O(N²);必须增量
- **推荐:vLLM 式块哈希链**:规范字节流按块(4KB 或 128 token)切分,h_i = H(key, h_{i-1}, block_i);新请求从头比块哈希,首个失配块回退块内 memcmp;均摊 O(δ+B);内存 O(L/B)(200KB ≈ 50 块 ≈ 400B)
- Rabin-Karp 前缀哈希数组 O(1) 判定但非前缀更新整段失效;字典树仅跨请求统计才值得

### 字符级 vs token 级
- **byte-level BPE 编码单射:字节相同 ⟺ token 序列相同 → 字节 LCP 是服务端匹配的必要条件,客户端字节 LCP 可直接证伪多报**
- 无 tokenizer 降级:字节 LCP 是上限;tokens≈bytes/4(英文)粗估
- OpenAI 上报:≥1024 才缓存、128 步长 → claimed cached 超过 token 级 LCP 或与粒度不符即判假
- Rust 有纯 Rust tiktoken crate(cl100k/o200k/llama3/deepseek_v3/qwen2 等 11 编码);vocab 几 MB 一次性载入;每轮只编码增量 O(δ)

### 指纹与存储
- 每请求存:128-bit xxh3 指纹 + 块哈希链 + 字节/token 计数;块哈希→请求 id HashMap 跨请求复用索引(vLLM cache table 式)
- **脱敏:绝不落盘明文;会话密钥哈希(HMAC-SHA256/SipHash,每会话随机 key)防低熵字典攻击**
- 零分配:smallvec/arena;128-bit 碰撞率 ~2⁻¹²⁸

### 推荐总管线
规范化 request_body(preserve_order + NFC + 剔除噪声)→ 块哈希链增量 LCP(xxh3-128)→ 边界块 memcmp 精确化 →(可选)tiktoken token 级扩展 → 与上报 cached_tokens 对账。时间均摊 O(δ+B)/轮,内存 O(L/B)/请求,零堆分配

### 不确定性
- Anthropic cache_read 精确度 [UNVERIFIED];5.6B/token [UNVERIFIED];simdutf NFC [UNVERIFIED];块哈希链+客户端对账无现成库(原创组合,基于 vLLM 已验证设计)
