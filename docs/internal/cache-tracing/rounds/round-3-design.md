# Round 3 设计:TraceStore — TTL 窗口、内存上限与作用域策略(2026-08-01)

> 设计 agent 第 3 轮。输入:D5 块哈希链算法、D3 时序不变量(硬不变量 1/6)、D6 aimux 现状。范围:追踪存储设计,不含判定规则(后者由设计文档草案承接)。

## 1. 定位与不变量

TraceStore = **有界的内存历史窗口**(默认 per-process),支撑审计硬不变量 1(命中域 ⊆ 客户端历史前缀)与不变量 6(时序:共享前缀间隔 > TTL 不可能命中)。默认路径零额外重依赖(std sync + HashMap + 纯 Rust 哈希 leaf 依赖);共享/持久化后端走 trait,可插拔。

**关键语义修正**:provider 缓存是**空闲淘汰(LRU + idle TTL)**而非绝对 TTL——前缀只要被持续使用就常驻(Anthropic 命中刷新免费;OpenAI in-memory "不活跃 5-10 分钟清除")。因此客户端时序判定用 **idle 语义**:

> 前缀 P 可作为上界 ⟺ now − last_touch(P) ≤ TTL_idle(scope),其中 last_touch(P) = 发送过 P 的最晚历史请求时间(当前请求**查完再插入**,天然排除自身,避免"刚发过必 alive"退化)。

单调性:P 过期 ⟹ 任意更长后缀 P' ⊇ P 也过期(含 P 的请求集合 ⊇ 含 P' 的集合)。故 lookup 逐块前移时,首块失配或过期即停。过期前缀 → 该段 abstain(UNKNOWN),**绝不判违规**。

## 2. TraceRecord(每请求一条)

| 字段 | 大小 | 说明 |
|---|---|---|
| slot / gen | 4+4 B | 环缓冲槽位 + 世代计数(懒失效) |
| scope: ScopeId | 8 B | 指向 scope 表 |
| chain: [BlockHash; 64] | ≤64×16 B | 128-bit keyed xxh3 块哈希链,h_i = H(key, h_{i-1}, block_i) |
| block_count / len_bytes | 2+4 B | 块数、字节数 |
| t_send_wall / t_send_mono | 8+8 B | 墙钟(导出/跨进程)+ 单调钟(进程内精确间隔) |
| usage(完成回填) | ~32 B | cache_read / cache_write / no_cache 快照 |
| flags | 1 B | truncated / canonicalized / dedup |

**明文请求体绝不留存**,只存链 + 计数。**两阶段记录(修正 wrapper 时序)**:阶段 1 调用开始时插入占位记录(scope + t_send,无链)——非流式在 do_generate 返回前拿不到 request_body,发送时无法算指纹;阶段 2 结果返回后计算块哈希链、判定(占位记录不在 lookup 范围内,天然排除自身)、回填链与 usage。流式路径在 do_stream 返回时即可拿到 request_body → 链回填提前到流启动时,usage 在 Finish 回填。**t_send = 调用开始时间(单调钟)**;流式中断也不丢历史。

## 3. TTL 参数表(TTL_idle = 审计保守上限)

| provider 族 | TTL_idle(默认) | 依据(工作文档) |
|---|---|---|
| OpenAI 旧模型(≤5.6) | 60 min | in-memory 5-10min 不活跃 + off-peak 最长 1h;命中刷新 |
| OpenAI 5.6+ | 显式 ttl(30m,最短保留)优先;模型支持 extended → 24h;否则 60 min | prompt_cache_options.ttl / extended retention(B1) |
| Anthropic | 显式 cache_control=1h 用 1h;无显式取 1h(2026-03 默认 TTL 静默回归 1h→5m,取上界防误报) | B3 / claude-code #46829 |
| Gemini implicit | 24 h | Vertex 明确 ≤24h;API 侧未知取上界 |
| DeepSeek | 配置化,默认 48 h | 磁盘缓存,闲置数小时-数天清除 |
| 自托管 vLLM/SGLang | ∞(无墙钟 TTL,LRU) | free queue + LRU;仅受环缓冲与负载约束 |

**方向性:TTL_idle 宁可高估**。低估会把合法命中判为"不可能"→ 掺水误报;高估只少判(安全方向)。参数优先级:请求显式 TTL 参数 > provider 族默认 > 用户 per-scope 覆盖。

**时钟**:进程内判定全部用单调钟(now − t_send_mono,免疫 NTP);墙钟仅用于记录导出与共享后端跨进程比较(±skew 容差默认 5 min;回拨时 duration 钳 0 → 判 alive,安全方向)。

## 4. 内存上限与淘汰

- 结构:`Vec<TraceRecord>` 固定容量**环形缓冲**(全局 cap)+ per-scope cap。
- 默认 max_records = 2048、max_records_per_scope = 512;设 0 关闭跨请求审计(仅单请求自洽检查,零增长)。
- 淘汰 = **FIFO 环(硬上界)+ 惰性 TTL 过期 + per-scope 最老淘汰**。LRU 默认不做(intrusive 结构换来的收益小,idle-TTL 已覆盖绝大多数"该淘汰"场景;留 feature flag)。
- 过期回收**惰性**(lookup 时跳过),无后台线程、零额外依赖。
- 反向索引驱逐:被淘汰记录的 chain 不即时删,靠 (slot, gen) 世代校验跳过;墓碑率 >25% 时整索引重建(均摊 O(总条目) ≈ 插入成本 ×4)。

## 5. 反向索引(块哈希 → 候选)

- 结构:per-scope `HashMap<u64, Vec<BlockRef(slot, block_idx)>>`;**索引键 64-bit**,命中后用记录内 128-bit 链验证(64-bit 碰撞 → 验证失败 → 拒绝,安全方向)。
- 哈希宽度一致性(修 G7):进程内 TraceStore 记录持有完整 **128-bit 链**;导出/序列化的 `TraceRecord.block_hashes` 为 64-bit 键(JSON/TS 无法精确表达 u128)。**判定不依赖导出数据**——导出仅作证据与离线分析,进程内判定始终用 128-bit 链。
- 增量维护:插入时对每块哈希追加 BlockRef;淘汰/重建见 §4。
- Lookup:逐块查 h_i → 128-bit 验证 → 候选集合取最晚 t_send 做 alive 检查 → 失配/过期即停(单调性)。
- 跨请求 LCP 为**块粒度**;块内字节无法跨请求 memcmp(无明文)→ D5 的 memcmp 精确化仅能在当前请求自身内部做。对审计无损:claimed_cached > 块粒度共享**下界**即可证伪(下界确定成立);期望上限按 (j+2)·B 输出(±1 块容差,符合 D5)。

## 6. 并发模型

- `TraceStore` 以 `Arc<TraceStore>` 共享;std 类型天然 Send+Sync。
- **两阶段 + 分片锁**:
  1. 无锁阶段:canonicalize + 块哈希链计算(O(L),~10-20µs @200KB)——不占锁;
  2. 短临界区:按 scope 哈希取分片(默认 16 片 `std::sync::RwLock`)。read-lock 查索引 + 验证 + 算 LCP(快照;之后候选被淘汰只弱化结论,不破坏正确性);write-lock 插入记录 + 更新索引(µs 级)。
- 同 scope 并发请求在 ~2-5µs 临界区串行;不同 scope(不同租户/模型)完全并行。1000 req/s 同 scope 锁占用率 ≈ 5%,不进热路径。(升级路径:parking_lot / sharded lock-free,默认不引入。)

## 7. 作用域 key(ScopeKey)

```
ScopeKey = H(normalize(base_url) ‖ fp(api_key) ‖ model ‖ session_pin?)
```

- **base_url** 规范化:scheme+host+port,去尾斜杠。网关 vs 直连 = 不同 scope。
- **api_key 指纹**:sha256(api_key) 前 8 字节,不存明文 key;不同用户 key → 不同 scope → **多租户天然隔离**(vLLM cache_salt 同理,客户端按 key 分)。
- **model**:精确字符串;alias 切换 = 新 scope(0 命中合法,不误报)。
- **session_pin**(有则加入):OpenRouter session_id / prompt_cache_key(conversation hash 钉上游部署)、OpenAI prompt_cache_key、xAI x-grok-conv-id。**不同 pin 可能被钉到不同上游部署 → 缓存域不相交,必须分开**;无 pin 时按默认 scope(命中"可能但不保证",只弱化上界,安全方向)。
- 每 scope 持有:TTL_idle、HMAC 盐(哈希键 = 进程随机 master key ⊕ scope 盐)→ **跨租户哈希不可比**(隐私 + 防低熵字典攻击)。

## 8. 共享存储 trait(可选扩展,默认路径零依赖)

```rust
// 默认路径:不注册任何实现 → 不链接任何持久化代码。
// 共享后端(Redis/SQLite/文件)只收哈希链,绝不收明文。
#[async_trait]
pub trait TraceStorage: Send + Sync {
    /// 原子语义由后端保证(查询 + 追加);返回进程内看不到的跨进程候选。
    async fn query_and_append(
        &self,
        scope: &ScopeKey,
        rec: TraceRecordRef,
    ) -> Result<Vec<PrefixCandidate>, TraceStoreError>;

    /// 离线审计导出(可选)。
    async fn export_window(
        &self,
        scope: &ScopeKey,
        since: SystemTime,
    ) -> Result<Vec<TraceRecord>, TraceStoreError>;
}
```

内存后端走同步快路径;共享后端为异步镜像:query 得跨进程候选 → 判定标 UNKNOWN 仅作参考上界 → append。跨进程比较需共享 master HMAC 键(部署配置),记录含 process_id 与墙钟。

## 9. 内存 / CPU 预算(200KB body,B = 4KiB → 50 块)

| 项 | 量 |
|---|---|
| 记录 | 50×16B 链 + ~80B 元数据 ≈ 0.9 KB |
| 索引 | 50 条 ×(8B 键 + 8B ref + std HashMap 摊余 ~32B)≈ 2.4 KB |
| 合计 / 请求 | ≈ 3.3 KB |
| 默认 2048 记录 | ≈ 6-7 MB 上限(**有界**,满足"2000 请求零增长"诉求) |
| 链计算(xxh3-128) | 10-20 µs |
| 索引 lookup + 验证 | 50 × ~100 ns ≈ 5 µs |
| 全链(规范+哈希+审计) | ≈ 100-300 µs @200KB;相对 3-10s LLM 请求可忽略;建议放请求完成后的异步管道 |

## 10. 边界情况

1. **首请求 / 无历史**:LCP=0;cache_read>0 → 同进程内中置信可疑;无历史(可能他进程/其他客户端预热)→ UNKNOWN,不判掺水。
2. **跨进程共享后端**:仅参考上界,判定仍 UNKNOWN;时钟漂移 ±5min 容差。
3. **空闲超 TTL 的前缀**:整段 abstain(§1 单调性)。
4. **淘汰竞态**:lookup 快照后候选被淘汰 → 已算下界仍有效(只弱化)。
5. **重试去重**:相同 chain 短窗内(默认 60s)合并为一条记录;usage 取**最后一次成功响应**(与规则 R-5.2 一致,重试后命中合法);verdict 基于最终 usage。
6. **超大请求**:超 max_len(默认 512KB)只存前 K 块 + truncated 标志,上界降级。
7. **哈希碰撞**:64-bit 索引碰撞 → 128-bit 验证拒绝;128-bit 链碰撞 ~2⁻¹²⁸,忽略。
8. **墙钟回拨**:duration 钳 0 → 判 alive(安全)。
9. **OpenRouter response-cache**(usage 全零 + X-OpenRouter-Cache-Status):独立判定路径,不算 prefix hit。
10. **流式 usage 缺失**:仅记录 chain,verdict 容忍。
11. **进程重启**:内存历史丢失 → 重启后首请求 UNKNOWN(预热期合法)。
12. **同 key 并发预热竞态**:首个写其余读,服务端不保证;客户端 LCP 上界仍成立(都发过该前缀),不误报。

## 11. 剩余不确定性

- OpenAI 5.6+ `prompt_cache_options.ttl` "最短保留"的确切上界语义 [UNVERIFIED] → TTL_idle 取值有 ± 风险(方向:取大)。
- 共享存储 `query_and_append` 原子性依赖后端事务/锁语义,具体后端未定。
- block 粒度 ±1 容差与 OpenAI 128-token 量化、DeepSeek 64-token 单元的叠加上界,需验证官复核。
