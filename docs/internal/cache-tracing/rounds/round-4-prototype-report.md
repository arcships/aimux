# Round 4:缓存审计核心算法可运行原型(2026-08-01)

> 原型 agent(第 4 轮)。目标:把 round-3 判定规则表 §0/§1、TraceStore 设计、Fingerprint 结构
> 落成**可运行 Rust 原型**,用 8 个合成场景证明设计可落地。
> 位置:`docs/internal/cache-tracing/prototype/`(独立 cargo 工程,零外部依赖,脱离根 workspace)。
> 代码行数:约 700(含测试)。

## 1. 实现组件

| 组件 | 实现 | 对应设计 |
|---|---|---|
| BlockChainFingerprint | `h_i = H128(key_i, block_i)`,`key_i = mix(scope_salt ^ low64(h_{i-1}))`;块大小可配(默认 4096,测试 512);128-bit = 两个 64-bit 键控哈希混合(任务允许的替代 xxh3) | D5 / round-3-design §5 |
| TraceStore(单线程) | 环形缓冲 + (slot,gen) 懒失效 + per-scope 计数/上限 + 反向索引 (scope, 块哈希低64)→(slot,gen,block);命中后 128-bit 全链验证;墓碑率>25% 重建;lookup 逐块前移,失配/过期即停(单调性);TTL idle:候选取最晚 t_send,`now−t_send ≤ TTL_idle`;明文不落盘 | round-3-design §1/§4/§5 |
| 判定引擎 | 纯函数表驱动:`U = min(prompt, quantize_down(min(LCPb, prompt_bytes)/4, gran))`(尾块损耗默认 0);`τ = max(5%·U, 1 gran)`;claimed≤U→OK、∈(U,U+τ]→B、>U+τ→W、>prompt→W;规则:R-1.1/1.1abs/1.2/1.3/1.6b/1.7/1.8/2.1/2.2/2.3/3.3/5.1,映射 W→SuspectOverclaim(High)、B→SuspectOverclaim(Medium)、OK→Trusted、M→SuspectUnderclaim(Low)、U→Unknown | round-3-verdict-rules §0/§1 |
| 场景 runner | 查→判→插(两阶段语义,排除自身);合成 body 每 token 恰 4 字节,字节代理与真实 token 数精确一致 | round-3-design §2 |

## 2. 场景结果(12/12 测试全绿,`cargo test`)

| # | 场景 | 构造 | 期望 | 实际 | 触发规则 |
|---|---|---|---|---|---|
| 1 | append-only agent loop(每轮 +1 tool 消息,前缀稳定) | 前缀 3200 token,claimed=前缀 | OK | ✅ Trusted×4 | — |
| 2 | 同上但虚报 claimed=前缀×1.1(3520) | U=3200,τ=160,U+τ=3360 | W | ✅ Overclaim(High) | R-1.1 |
| 3 | 首请求 claimed=2048>0 | 无历史 | W | ✅ Overclaim(High) | R-1.2 |
| 4 | gpt-5.6+ implicit 断点:大 LCP 2944 token、claimed=0、4 轮 | 白名单 R-2.3 | 不误报 | ✅ Trusted×4,无 R-2.2 | R-2.3 |
| 5a | DeepSeek hit+miss≠prompt(640+300≠1400) | 视界无关等式 | W | ✅ Overclaim(High) | R-1.3 |
| 5b | DeepSeek 等式成立(640+760=1400) | — | OK | ✅ Trusted | — |
| 6 | 间隔>TLL_idle 仍 claimed>0 | 距最近记录超 TTL | W | ✅ Overclaim(High),U=0 abstain | R-1.8 |
| 6b | 间隔 1s(<<TTL)claimed=1280 | — | OK | ✅ Trusted | — |
| 7 | prompt=300<1024 门槛,claimed=128>0 | — | W | ✅ Overclaim(High) | R-3.3 |
| 8a | 并行/跨 session,claimed=system 段 512 | 段位期望 U=512 | OK | ✅ Trusted | R-2.1 |
| 8b | 同左 claimed=700>512+τ | 超可解释段位 | W | ✅ Overclaim(High) | R-1.1+R-2.1 注记 |
| 8c | 同 session append-only,claimed=768 | 同 session 可认整段 U=2560 | OK | ✅ Trusted | — |
| 附 M | 非 5.6+ 3 轮前缀稳定 claimed=0 | R-2.2 | M | ✅ SuspectUnderclaim(Low) | R-2.2 |
| 附 U | usage 缺失(流式中断) | R-5.1 | U | ✅ Unknown | R-5.1 |

断言失败信息含完整 verdict(kind/conf/violated/U/claimed/lcp/notes),可定位到规则。

## 3. 性能粗测(200KB body,49 块 @4096B)

| 项 | debug | release | 设计预算(round-3-design §9) |
|---|---|---|---|
| 链计算 | 2815 µs/op(71 MB/s) | 288 µs/op(694 MB/s) | xxh3:10-20 µs |
| lookup(49 块,128-bit 验证+TTL) | 11.1 µs | 0.8 µs | 50×~100ns≈5 µs |
| 全管线(链+lookup+判定) | 9059 µs | 1710 µs | 100-300 µs |
| 内存/记录 | — | ~880 B(49×16+元数据) | ~0.9 KB |

结论:lookup 达标;链计算慢 ~30× 是因为自研占位哈希(splitmix64 折叠)非 xxh3——
换 xxh3-128 即回到设计预算;绝对量级上相对 3-10s LLM 请求仍可忽略,建议放请求完成后的异步管道。

## 4. 诚实报告:歧义与未原型化

**设计歧义(已按任务公式实现,标注取舍)**
1. **块粒度 LCP 是共享下界**:真共享 ∈ [j·B, (j+1)·B)。设计 §5 建议期望上限 (j+2)·B(±1 块),任务公式直接用 LCPb=j·B。原型按任务公式(保守下界,可证伪方向);后果:claimed 恰超 1 块内时偏严。5.6+(无 gran)τ 只有 5%,未显式计入尾块损耗(tail_loss 默认 0)——建议生产配 tail_loss=1 块。
2. **τ 与虚报幅度耦合**:τ=max(5%, 1 gran)。带 gran 的 provider(如 128)在 U<2560 时,10% 虚报落在 B 而非 W;测试用大前缀(3200)使 5%>gran 才断言 W。规则表对"多大虚报算 W"是相对的。
3. **字节代理 bytes/4**:合成 body 每 token 恰 4 字节,测试断言精确;真实语料下 token_upper 是近似(tok≤bytes 恒成立,方向安全),生产应接 tiktoken 升 W(规则 R-5.4)。
4. **R-1.1 字节上界例外**:规则表注明无 tokenizer 时 claimed>LCPb→B(中),但任务明确要求场景 2 判 W;原型按 §0 区间公式(W)实现,该例外未启用,需验证官定夺。

**无法/未原型化(诚实清单)**
- tokenizer 精确计数(R-1.1 token 级 LCP_tok)— 需 tiktoken,原型用字节代理
- Anthropic 三字段/20-block 回看(R-1.4)、Bedrock quota(R-1.5)— 需 provider 字段管线,与算法核心无关
- 响应缓存头跳过(R-4.1)、网关字段剥除清单(R-4.2)— 输入头/形态识别,代码留契约注释
- retry 去重(R-5.2,需链 hash 计数+60s 窗)、多进程共享后端(R-5.3,shared 降级路径已写未测)、聚合 A1-A5
- 并发(round-3-design §6 分片锁)— 单线程原型,锁开销/竞态未验证

**剩余不确定性**:Azure 128 量化与 OpenAI 5.6+ 字节精确的双轨 gran 分派、Anthropic cache_read 精确度、TTL 高估方向是否过于宽松,均待真实 provider 数据回放验证。

## 5. 文件清单

- `prototype/Cargo.toml`(独立 workspace,零依赖)
- `prototype/src/hash.rs`(键控 64-bit ×2 → 128-bit)
- `prototype/src/fingerprint.rs`(BlockChainFingerprint)
- `prototype/src/store.rs`(TraceStore:环缓冲+懒失效+反向索引+TTL idle)
- `prototype/src/verdict.rs`(判定引擎 + ProviderSpec 参数表)
- `prototype/src/synth.rs`(合成场景 DSL + runner)
- `prototype/tests/scenarios.rs`(12 测试)
- `prototype/examples/bench.rs`(性能粗测)
