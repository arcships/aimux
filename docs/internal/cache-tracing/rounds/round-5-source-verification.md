# Round 5: 缓存机制源码级验证(vLLM / SGLang / LMCache)

> 2026-08-05,三框架源码浅克隆 HEAD 逐项核实,证据等级 A(源码)。
> 目的:为 RFC-0015 判定逻辑(块哈希链 LCP)提供 A 级机制证据,修正矩阵中
> 此前仅 C 级(官方 issue)支撑的描述。
> 方法:每框架 `git clone --depth 1`,符号定位关键路径,结论仅采源码,
> 不使用二手博客。验证后已清理克隆目录。

---

## 1. 共同机制确认(跨三框架一致)

| 机制 | vLLM | SGLang | LMCache | 与 aimux 设计 |
|---|---|---|---|---|
| **命中 = 从头最长公共前缀** | ✅ `find_longest_cache_hit` 从头逐块,首个缺失即停(`v1/core/single_type_kv_cache_manager.py:728`) | ✅ RadixCache `match_prefix`("Find the longest cached prefix",`mem_cache/radix_cache.py:352`) | ✅ 滚动前缀哈希("each chunk's hash depends on all previous chunks",`v1/multiprocess/token_hasher.py:199`) | 一致——LCP 判定是普遍语义 |
| **父链哈希** | ✅ `hash_block_tokens(parent_block_hash, curr_block_token_ids, extra_keys)`(`v1/core/kv_cache_utils.py:576`) | —(前缀树,非哈希链) | ✅ `hash_func((prefix_hash, tokens, None))`(`token_hasher.py:183`) | 一致——块哈希父链有源码先例 |
| **块/页粒度,尾部不足不命中** | ✅ 默认 16 token(`config/cache.py:43`);只哈希完整块(`kv_cache_utils.py:701`) | ✅ page_size 默认 1(非 MUSA),>1 时对齐截断(`radix_cache.py:161`) | ✅ chunk 256 token,完整 chunk 才参与(`token_hasher.py:206`) | 一致——"部分块不算命中"的块粒度语义成立 |
| **上报字段** | `prompt_tokens_details.cached_tokens`(`entrypoints/openai/chat_completion/serving.py:88`) | 同(`entrypoints/openai/protocol.py:177`) | 不回填 vLLM usage | 一致——统一映射字段正确 |
| **salt/额外键参与哈希** | ✅ LoRA name + `cache_salt`(`kv_cache_utils.py:497,554`) | ✅ `extra_key` 不同不共享节点(`radix_cache.py:197`) | — | 一致——scope 盐设计有依据 |

**结论:三个开源 serving 框架的缓存机制与 RFC-0015 的判定模型(父链块哈希 + 从头逐块 LCP + 块粒度下界)在源码级一致。**

---

## 2. 偏差与矩阵修正

### vLLM(证据:`v1/core/kv_cache_utils.py`、`v1/core/single_type_kv_cache_manager.py`、`entrypoints/openai/chat_completion/serving.py`)

| 原矩阵/假设 | 源码事实 | 修正 |
|---|---|---|
| "V1 恒 null(#44961)" | V1 内部**计算并传递** `num_cached_tokens`(`v1/core/kv_cache_manager.py:731`);OpenAI 层默认 `enable_prompt_tokens_details=False` 所以**默认不返回**(`cli_args.py:132`) | 矩阵 vLLM 行改为:"V1 需 `--enable-prompt-tokens-details` 才返回 cached_tokens;默认缺失 ≠ 恒 null" |
| 块粒度恒 16 | `DEFAULT_BLOCK_SIZE=16` 可配置;`prefix_match_unit` 可更细 | 注明"默认 16,可配" |
| 仅完整块命中 | fine-grained `prefix_match_unit` 支持物理块内哈希边界命中(`single_type_kv_cache_manager.py:741`) | 对判定**无影响**:aimux 按块下界保守,只会漏判不会误报;RFC §10 注明 |

### SGLang(证据:`mem_cache/radix_cache.py`、`entrypoints/openai/usage_processor.py`、`disaggregation/decode.py`)

| 原矩阵/假设 | 源码事实 | 修正 |
|---|---|---|
| 恒 token 级 | `page_size` 默认 1(非 MUSA),但 ROCm/MUSA/部分后端自动 64/128(`arg_groups/overrides.py:2274`) | 注明"默认 token 级,后端可覆盖页粒度" |
| "PD #25972 双重计数" | 风险模型存在;当前代码有防护:decode 用 `already_computed` 种子 + `max(0, pre_len - already_computed)` clamp(`disaggregation/decode.py:1881`、`decode_schedule_batch_mixin.py:65`) | 矩阵降级为:"防护存在(already_computed + clamp);需验证具体部署路径是否破坏不变量,不能断言当前版本有未修复 bug" |
| 需 `--enable-cache-report` | 确认;`return_cached_tokens_details` 是**另一个**开关(详细 breakdown,`openai/utils.py:139`) | 区分两个开关 |

### LMCache(证据:`v1/multiprocess/token_hasher.py`、`integration/vllm/vllm_v1_adapter.py`)

| 假设 | 源码事实 | 修正 |
|---|---|---|
| LMCache 命中会体现在 vLLM cached_tokens | **不会**:`vllm_cached_tokens` 与 `lmcache_cached_tokens` 是两个独立计数,retrieve 只回填 KV buffer 不回填 block-hash cache(`vllm_v1_adapter.py:64,821`) | 新增:"部署 LMCache 时,其命中**不**出现在 vLLM 的 cached_tokens 中,会漏报;可配 `enable_cache_usage_details_in_response` 返回 `num_lmcache_cached_tokens` 需另行识别" |
| 哈希碰撞防护 | 普通 prefix lookup 无二次内容校验(仅 CacheBlend V3 有 poly-hash 校验,`blend_v3.py:312`) | aimux 的"64-bit 索引 + 全 128-bit 链验证"比 LMCache 普通路径保守,保持 |

---

## 3. 对判定实现的影响

- **无需改代码**:核心假设(父链块哈希、从头逐块 LCP、块粒度下界、scope 盐、cached_tokens 统一映射)全部有源码级支撑,且偏差都在"服务端比客户端更精细"的方向——我们的块下界只会低估客户端上界,不会误报。
- **RFC-0015 §10 UNVERIFIED 项更新**:"vLLM V1 null 是否已修复"→ A 级证据:默认不返回(开关关闭),非 null bug;开启 `--enable-prompt-tokens-details` 即有数据。
- **矩阵升级**:vLLM/SGLang/LMCache 三行从 C 级(issue)升为 A 级(源码),措辞按上表修正。

## 3.5 集群/路由部署事实(2026-08-05,实测补充)

生产环境模型多为**集群部署**(LB 多节点),节点本地 KV 缓存不共享:
- 路由变化(或请求差异改变路由 hash)→ 上轮缓存失效 → **报 0 命中是常态**
- DeepSeek 磁盘缓存**全局共享**(不依赖单机 KV 状态),跨节点一致 → 实测命中最稳定
- 对判定:① 命中仍 ≤ 客户端前缀 → 无掺水误报;② R-2.2 低命中预警在
  `route_affinity_known=false`(默认)时抑制为备注——"前缀稳定但报 0"在集群里是
  路由假象,不是前缀破坏;单机/粘性路由/全局缓存部署可显式开启该诊断
- 中转/网关(OpenRouter/LiteLLM/Portkey)同理:上游集群路由 + 网关转发都是
  缓存断点;矩阵 A 级透传行仍需结合部署形态判断

## 4. 遗留不确定(非源码可答)

1. 运行时行为未验证(未跑模型实例):特定模型/混合注意力/启动参数下的实际输出。
2. vLLM fine-grained 模式的实际部署率未知(默认关闭)。
3. SGLang PD 部署路径(元数据传输/重 bootstrap)是否破坏 `already_computed` 不变量——需运维侧验证。
4. LMCache 在真实集群中的命中率与 vLLM 自身缓存的比例——影响"漏报"严重度,需实测。
