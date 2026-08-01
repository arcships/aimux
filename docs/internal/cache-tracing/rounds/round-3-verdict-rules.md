# Round 3 设计:缓存命中审计判定规则表(2026-08-01)

> 设计 agent(第 3 轮)。将 D3 硬不变量 1-6、D4 分段期望模型、D5 指纹管线落成可编码规则。输入来源:10-working-document.md + rounds/ 存档。判定类:W=掺水 / OK=合理 / B=边界可疑 / U=不可判定 / M=低命中预警(非掺水)。

## 0. 符号与输入(客户端可得)

- `b_i` = 规范化 `request_body` 字节(D6:preserve_order,确定性);`tok(b)` = token 数(可选 tiktoken;缺失用字节,且 token ≤ bytes 恒成立)
- `LCP_i` = max_j LCP(b_i, b_j),j ∈ 视界内历史(同 provider+model+route_key,TTL 窗口内);`LCPb` = 字节版 LCP
- `claimed` = cached_tokens / cache_read_input_tokens / prompt_cache_hit_tokens(按 provider);`prompt` = prompt_tokens / input_tokens;`write` = cache_write / cache_creation_input_tokens;`no_cache` = prompt−claimed−write
- 附加:`first`(该 provider+model+key 首次且窗口内无先例)、`t0`、headers(X-OpenRouter-Cache-Status)、session_id、model、route_key、cache_control 档位、`usage_present`、retry 标记(同逻辑调用内 b 哈希重复)

**期望命中区间**:`U = min(prompt, quantize_down(token_upper(LCP_i), gran))`,单位统一为 token:
- `LCP_i` = 块粒度共享**下界**的字节数;真实共享 ∈ [LCP_i, LCP_i + block_size);**U 按块上界算**:`LCP_blocks_upper = (j+1)·B`(j = 共享块数,B = 块大小;与 TraceStore 设计 §5 (j+2)B 口径收敛,取 (j+1)B 为中间态,±1 块容差由 τ 承担;定案 F3)
- `token_upper` = 有 tokenizer 时对匹配块序列做 token 计数(token 级 LCP_tok);无 tokenizer 时用字节代理 `min(LCPb, bytes/4)`(方向:token ≤ bytes 恒成立,代理仅支撑上界;定案 F2)
- gran( token 单位):OpenAI<5.6=128、**Azure(含 5.6+)=128(Azure 未采用字节精确,见矩阵)**、DeepSeek/Mistral=64、vLLM=16、Anthropic=断点前段+20-block 回看(定性)、OpenAI 5.6+ 原生=无量化
- `L = 0` 默认;提升条件:存在 j 使 b_j 前缀 ⊆ b_i、claimed_j>0、间隔<TTL、无失效事件(model/tools/图片 detail/schema 未变)→ L=min(claimed_j, U)(仅漏报侧诊断用)
- 判定:`claimed∈[0,U]→OK`;`∈(U, U×(1+τ)]→B`;`>U×(1+τ)→W`;另加绝对上限 `claimed > prompt → W(高)`,任何视界下都不可能
- **无 tokenizer 字节代理模式下 W 封顶为 B(中)**(R-5.4;接入 tiktoken 后升 W 高)——避免 /4 近似在中文语料(≈3B/token)误报

**判定类 ↔ VerdictKind 映射(与数据模型契约)**:W→`SuspectOverclaim`;B→`SuspectOverclaim`(confidence=Medium,notes 注明"边界可疑");OK→`Trusted`;U→`Unknown`;M→`SuspectUnderclaim`(confidence=Low,notes 注明"低命中预警",聚合按 A4 独立计数,不与掺水混淆)。

**执行顺序(短路)**:①usage 缺失→U(R-5.1)→②响应缓存头→跳过(R-4.1)→③字段完整性→网关剥除→U(R-4.2)→④视界无关不变量 R-1.3/1.4/1.5/1.6/1.7→⑤视界依赖 R-1.1/1.2/1.8(按 strict/shared 模式)→⑥分段+白名单修正 R-2/R-3/例外表→⑦输出 verdict+区间→⑧聚合。

## 1. 硬不变量组(违反→W,除注明)

| ID | 检查逻辑 | 判定/置信 | 例外 |
|---|---|---|---|
| R-1.1 前缀包含域 | strict 模式(自托管/单客户端,视界=本进程全历史):claimed>LCP+τ→W;**shared 模式(共享 API key,默认)**:有本地来源且超限→B(中);无本地来源→U(他进程可能写过更长前缀,见 R-5.3) | W(高)/B(中)/U | 5.6+ implicit 断点段位修正(R-2.3);字节上界模式:claimed>LCPb→B(中) |
| R-1.2 首请求零命中 | first && claimed>0 | W(高);shared 模式→B(中) | 跨进程预热过;显式 prompt_cache_key 预建 |
| R-1.3 DeepSeek 等式 | prompt==hit+miss,纯内部一致性,视界无关 | 不等→W(高) | 字段被网关剥→U;±1 token 容差 |
| R-1.4 Anthropic 三字段 | input==read+creation(5m+1h)+input_tokens;首请求 read==0;read≤历史未过期 write;TTL 档位字段⊇请求 cache_control(请求侧档位来自 TraceRecord.request_cache_hints,R-5.5;取不到时该项降级跳过) | 任一违反→W(高) | server tool 自动 5m 断点合法;流式 message_delta 累计未去重→input 虚高→B(中,口径 bug 非掺水);Batch best-effort 30-98% |
| R-1.5 Bedrock 等式 | total==input+read+write;quota_burndown=input+write+output×factor,read 不计 | 违反→W(高) | ConverseStream 流式 usage 位未确认→U |
| R-1.6 OpenAI 门槛/量化(5.6-) | prompt<1024→claimed 必 0;claimed%128==0 或 0;claimed≤prompt | <1024 有命中→W(高);非 128 倍数→B(中) | 5.6+ 不量化;Azure 前 1024 单字符差异⇒0 合法;5.6+ implicit 0 合法 |
| R-1.7 OpenAI 5.6+ 写读等式 | claimed+write+no_cache==prompt;no_cache≥0 | 不等→W(高);write 缺失→B(中) | Azure 不报 write→U;responses API 字段名不同 |
| R-1.8 TTL 时序 | claimed>0 时,须存在 j:LCP_tok(b_i,b_j)≥claimed 且 now−t0_j ≤ TTL(见 TraceStore round-3-design.md §3 TTL 表;无 tokenizer 时用字节代理:LCPb/4 ≥ claimed 的弱检查,方向安全仅漏报);无 j 或超 TTL | W(高);shared 模式无本地 j→U | 跨进程(R-5.3);Gemini TTL 未公开→保守 24h;TTL 静默回归→按客户端 cache_control 档位(请求侧 hints,R-1.4)而非默认值 |

## 2. 分段期望模型组(诊断为主,不单独判 W)

| ID | 检查逻辑 | 判定/置信 | 例外 |
|---|---|---|---|
| R-2.1 段位匹配 | claimed 落位段 ≤ 公共段位:system/tools 段可跨 session 命中;conversation 段仅同 session append-only 链 | 超可解释段位→B(中) | 跨 session 字节相同 conversation(罕见)不可区分 |
| R-2.2 低命中预警 | 同 session 连续 ≥3 轮、前缀长度稳定(相邻字节 LCP 波动 <5%,system/tools 段未变)、claimed 恒 0 且 LCP>1024 | M(前缀破坏预警:动态 system/历史压缩/tools 变化),不判掺水;映射 SuspectUnderclaim(Low) | 5.6+ implicit 断点(R-2.3);DeepSeek 64 单元 A+C 二轮 0 合法;预热期(R-3.1) |
| R-2.3 5.6+ implicit 白名单 | 5.6+ && claimed==0 && 大 LCP:implicit breakpoint 默认在最新 user/tool 消息,后缀变化⇒0 合法;显式 breakpoint 存在时改用断点前段期望 | 不判异常;显式模式下超出断点前段→B(中) | 5.6+ 是审计最易误报场景 |

## 3. 生命周期/白名单组

| ID | 检查逻辑 | 判定/置信 | 例外 |
|---|---|---|---|
| R-3.1 预热期 | 每 (provider,model,key) 前 N=10 次或 128s 内 claimed=0 全合法;统计排除 | 不判 | 无 |
| R-3.2 模型版本升级 | model 串变化→匹配空间仅同 model,新 model 首请求 claimed>0→W(同 R-1.1 model 键控) | W(高) | 权重升级但 model 名不变(DeepSeek)→0 合法、不可检测 |
| R-3.3 低门槛 | prompt<门槛(OpenAI 1024/Gemini 2048-4096/Anthropic 512-4096/DeepSeek 64/Mistral 64/vLLM 16)→期望 0;claimed>0 | W(高) | 显式 CachedContent(Gemini)不适用 |

## 4. 网关/第二类缓存组

| ID | 检查逻辑 | 判定/置信 | 例外 |
|---|---|---|---|
| R-4.1 响应缓存 | headers 含 X-OpenRouter-Cache-Status:HIT / X-OpenRouter-Cache:true(LiteLLM 响应缓存同理)→usage 归零合法,标记 response_cache 不审计 | 不判 | 与 provider prompt cache 严格区分,否则系统性漏报 |
| R-4.2 字段剥除 | 已知 C 级形态(one-api/simple-one-api 非流式/Portkey 流式/ferro 扁平/vLLM V1/Ollama/llama.cpp/TRT-LLM)→字段缺失/扁平化 | U+网关改写标记(中,聚合用) | A 级透传(OpenRouter/LiteLLM 部分路径/Cohere/Mistral/xAI/vLLM V0/SGLang enable-cache-report)正常审计 |
| R-4.3 网关自填 | 网关自记账(new-api 式,命中仍全价)→与外部账单对不上 | B(中)→聚合升级 | 需外部账单输入,本地仅标记 |

## 5. 数据完整性组

| ID | 检查逻辑 | 判定/置信 | 例外 |
|---|---|---|---|
| R-5.1 usage 缺失 | Finish.usage 空/默认(流式中断)→本请求 U;统计缺失率 | U | 无 |
| R-5.2 retry 合并 | 同逻辑调用内 b 哈希重复(重试,http.rs 内部不可见):取最后一次 usage;二次命中合法(前缀相同+间隔短) | 不判,标记 retry | retry 间隔>TTL 且 claimed 1→0 为正常 eviction |
| R-5.3 多进程视界 | claimed>0 但本地无来源→U(无法排除他进程/他客户端);claimed/本地 LCP>10→B(中);可配共享历史(Redis)扩视界 | U/B(中) | 单进程隔离部署(自托管 vLLM 单实例)→strict 模式可用 |
| R-5.4 无 tokenizer | 字节上界 tok≤bytes;claimed>LCPb→B(中);接入 tiktoken 后升 W(高) | B(中)→W(高) | 服务端吃原始字节:NFC 仅客户端自洽,audit 基准=去噪原始字节,语义级才用规范化 Prompt |
| R-5.5 请求侧 cache_control | Anthropic 5m/1h、OpenAI 5.6+ ttl 等请求档位来自 wrapper 从 CallOptions 捕获的 `request_cache_hints`(best-effort);缺省→TTL 时序/档位检查按 provider 默认值 | 缺省不判 W | 取不到档位时 R-1.4/R-1.8 的档位相关项降级跳过,其余检查照常 |

## 6. 聚合规则(单请求→会话→provider/model)

- A1 会话级:统计 {W,B,U,OK,M};W≥1 且可判定率≥70%→会话标记"含掺水"。
- A2 漂移检测:每 15min 对 (provider,model) 算 `Δ=mean(claimed)−mean(U)`;Δ>5%·mean(U) 或 掺水率 w>10%(样本≥20)→警报。系统性高报比单条更可靠。
- A3 多信号升级:W 或 B + TTFT 无随命中率改善(低置信旁证)+ 账单漂移(外部,2025-01 式)→升级"高置信掺水"报告。
- A4 漏报侧:M 计数(低命中预警)与掺水分开统计,避免方向混淆。
- A5 护栏:可判定率<70%→会话结论降级"证据不足";缺失率>30% 同理。

## 7. 参数建议值

| 参数 | 建议值 | 依据 |
|---|---|---|
| 上界容差 τ | 5% 或 1 gran 块(取大) | D1 容差窗口 |
| gran | 128(OpenAI<5.6)**/128(Azure 全系含 5.6+,Azure 未采用字节精确,矩阵实证)**/64(DeepSeek,Mistral)/16(vLLM)/无(OpenAI 原生 5.6+) | D1/D2b/矩阵 |
| Anthropic 断链 | 每轮新增≥20 block→U=断点前段 | D1 20-block lookback |
| TTL | OpenAI 60min(off-peak 1h,与 TraceStore §3 统一,不取 10min 低估)/24h(extended)/30m(5.6+ ttl 显式);Anthropic 5m/1h;Gemini 24h 保守;DeepSeek 48h 保守(与 TraceStore 默认统一,取高估安全方向);vLLM 无硬上限(护栏 24h) | D1/D2a/D2b/round-3-design §3 |
| 预热期 | N=10 次或 128s | 社区实测 |
| 漂移窗口 | 15min(警报)/1h(报告) | 首版默认,可调 |
| 掺水警报 | w>10% 且样本≥20 | 首版默认,可调 |
| UNKNOWN 护栏 | 可判定率<70% 降级 | 首版默认,可调 |
| M 预警门槛 | 连续 ≥3 轮、相邻 LCP 波动<5%、LCP>1024、claimed=0 | D4(与 R-2.2 同源,见 R-2.2) |

## 8. 边界情况清单

1. 5.6+ implicit breakpoint——最易误报,规则 R-2.3 必查
2. DeepSeek A+C 二轮 0 命中(单元粒度)
3. 跨进程/跨 API key 共享缓存(shared 模式降级 U)
4. OpenRouter 响应缓存 usage 归零(R-4.1)
5. 预热期与并发首请求
6. 流式中断 usage 缺失
7. retry 不可见(R-5.2)
8. 模型版本升级:名变(硬判)与名不变(不可检测)
9. 低于最低门槛
10. 网关字段剥除/扁平化(R-4.2)
11. Azure:参数接受但无效、0 命中、不报 write
12. gpt-5-nano/mini 长期不命中(0 合法)
13. Anthropic server tool 自动 5m 断点(1h 请求现 5m 写,合法)
14. Anthropic 流式 message_delta 累计 double-count(B 非 W)
15. Gemini preview 账目异常(低命中合法;账目异常标记待外部核对)
16. Batch API best-effort(30-98%)
17. 图片 token 不计 usage→图片多时命中率高估(该段不可审计,标 U)
18. response_format/tools/图片 detail 变化→前缀分叉,0 合法
19. vLLM cache_salt/LoRA 隔离(不同 salt 0 合法)
20. OpenRouter 首请求不粘路由(预热期 0 合法)
21. prompt_cache_key 15 RPM/key 限流漏命中
22. Anthropic TTL 1h→5m 静默回归(时序规则按客户端 cache_control 档位)
23. 字节 vs NFC:audit 基准用去噪原始字节,NFC 只用于语义诊断
24. 无来源命中:strict=W/shared=U(模式是最高层开关,先于一切规则)

## 9. 编码备注

- 规则引擎:纯函数表驱动(规则数组,含 guard+check+verdict),输入为每请求 TraceRecord + 历史索引;strict/shared 为全局配置
- 历史索引用 D5 块哈希链(R-1.1 查询 O(δ+B));retry 标记用 b 哈希计数
- 白名单(例外)先于规则判定生效,防止 5.6+/DeepSeek/预热误报
- 输出 per-request verdict + [L,U] + 命中的规则 ID,供聚合与告警
