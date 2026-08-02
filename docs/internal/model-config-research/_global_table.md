# Model Request Config 调研 — 全局汇总与差距清单

> **状态**：调研完成（2026-08-01，250/250 家）
> **方法**：见 [README.md](README.md)；明细见各 batch 文件
> **基线**：registry 250 个声明，仅 deepseek/groq 有特殊 profile，其余 248 家全为 `full()`

## 1. 覆盖统计

| 批次 | 家数 | 有差异 | 存疑(D/弱) | 文件 |
|---|---|---|---|---|
| 01 | 42 | 13 | 11 | [batch-01.md](batch-01.md) |
| 02 | 42 | 18 | 18 | [batch-02.md](batch-02.md) |
| 03 | 42 | 11 | 7 | [batch-03.md](batch-03.md) |
| 04 | 42 | 16 | 8 | [batch-04.md](batch-04.md) |
| 05 | 42 | 19 | 12 | [batch-05.md](batch-05.md) |
| 06 | 40 | ~30 | 14 | [batch-06.md](batch-06.md) |
| **合计** | **250** | **~107** | **~70** | |

证据分布：A 级（aimux cassette：cerebras thinking wire contract、deepseek/fireworks thin wrapper、zai 等 conformance）少数；B 级（reference 代码）少量；**绝大多数为 C 级（官方文档）**——`reference/` 网关项目被剪枝为文档快照，无法提供适配代码证据。所有条目验证状态 🔲（未实测）。

## 2. 差距清单（按优先级）

### P0 修复：registry base_url 数据错误（确定性，直接改 [openai_compat_registry.rs](../../aimux-providers/src/openai_compat_registry.rs)）

| 行 | 厂商 | 现状 | 问题 |
|---|---|---|---|
| 1533 | opencode | `"opencode_zen.rs"` | 非法 URL（字面量），必失败 |
| 759 | firepass | `".../inference/v1（OpenAI"` | 乱码 + 与 fireworks 完全重复 |
| 777 | freemodel | `"client.chat.completions.create"` | 截断的 SDK 示例代码 |
| 939 | iflowcn | `".../v1（chat"` | 乱码 |
| 840 | gmi | `".../v1（与"` | 乱码 |
| 2127 | volc_engine | `https://ark.cn-beijing.volces.com` | 缺 `/api/v3` |
| 2253 | zhipu_v4 | `https://open.bigmodel.cn` | 缺 `/api/paas/v4` |

base_url 存疑（需人工/实测确认，见各 batch 存疑节）：novita、nous_research、longcat、moonshotai_cn、minimax_coding_plan、oci、krutrim、kilo、lemonade、qiniu、tokenpony、wafer、xiaomimimo、vercel、the_grid_ai、unorouter、zhipuai_coding_plan、xpersona、tensormesh、orcarouter、pinstripes、requesty、predibase、bytez。

### P1 高价值：profile 扩展需求（→ RFC-0017 阶段 2 reasoningMap / 新字段）

#### A. thinking 机制 — "thinking 对象"家族（泛化现有 DeepSeek override 即可覆盖 5+ 厂商）

| 机制 | 厂商 | aimux 现状 |
|---|---|---|
| `thinking:{type:"enabled\|disabled"}` | DeepSeek ✅ | bigmodel/zai/zhipu、bytedance/byteplus、baidu(deepseek-v4 系)、腾讯 hy3 | 🔶 |
| `thinking:{type,budget_tokens}` | Fireworks、方舟/豆包 | 🔶 需带档位 |
| `thinking:{type,keep}` | Moonshot kimi-k2.6 | 🔶 |
| `reasoning:{enabled}` | DeepInfra | ❌ |
| `chat_template_kwargs.enable_thinking` | Hetzner、Tensormesh | ❌ |
| `venice_parameters.disable_thinking` | Venice | ❌ |

#### B. thinking 机制 — enable_thinking 家族（Qwen 系）

| 机制 | 厂商 | aimux 现状 |
|---|---|---|
| `enable_thinking` + `thinking_budget` | alibaba(qwen3)、baidu、siliconflow、TokenHub | ❌ |
| `/no_think` 消息级开关 | Qwen 新版混合思考 | ❌ |
| 不可关（发 warning） | kimi-k2.7-code、qwen3-thinking-2507 | ❌ 无 unsupported 路径 |
| `reasoning_effort` 三档 low/high/max | Moonshot kimi-k3 | 🔶 与 aimux 7 档不同，需档位映射 |
| `reasoning_effort` 四档 minimal/low/medium/high | Perplexity | 🔶 |
| `reasoning_format: general/deepseek-style` | StepFun（aimux 仅对 groq 发） | 🔶 |

#### C. max_tokens 命名（新 profile 字段 `max_tokens_key`）

- **只认 `max_tokens`**：stepfun、siliconflow、sarvam、reka、publicai、perplexity → aimux 推理分支发 `max_completion_tokens`（[convert.rs:1122](../../aimux-providers/src/openai/convert.rs#L1122)）会 400/静默失败
- **只认 `max_completion_tokens`**：Heroku（还需 `allow_ignored_params`）、cerebras ✅、Groq（max_tokens 已弃用）

#### D. 响应解析

- `reasoning_content` 别名：**已覆盖** ✅（[types.rs:40](../../aimux-providers/src/openai/types.rs#L40)，RFC-0002）——batch-04 此条为误判，无需动作
- `message.reasoning`（llmgateway 等网关透传）与 `reasoning` 双键：model.rs 已覆盖 ✅

### P2 认证/headers 差异（`with_headers`/`bodyOverrides` 已可表达，文档化即可）

| 厂商 | 差异 | 现状 |
|---|---|---|
| azure_ai、xiaomi MiMo | `api-key` 头而非 Bearer | 🔶 需 with_headers |
| portkey | `x-portkey-api-key`/`x-portkey-provider` | 🔶 |
| sarvam | `api-subscription-key` | 🔶 |
| snowflake | `X-Snowflake-Authorization-Token-Type` | 🔶 |
| copilot | VSCode headers + OAuth device-flow | ❌ 需专用适配 |
| GigaChat | OAuth 预请求 + 自签名证书跳过 TLS | ❌ 需专用适配 |
| inference_net | `x-inference-provider(-api-key)` 代理头 | 🔶 |
| chatgpt | `OAI-Product-Sku: codex` 头 + 需 `responses_model()` | 🔶（conformance 已通，需文档化） |

### P3 协议不匹配 / 退役（需决策）

| 厂商 | 问题 |
|---|---|
| coze | 原生 `/v3/chat` + bot_id，非 OpenAI 格式，registry full() 与事实不符 |
| zai_coding_plan | Anthropic 协议端点却声明 OpenAI full() |
| umans_ai | Anthropic 协议（推断，⚠️） |
| github | GitHub Models 2026-07-30 整体退役 |
| kluster_ai | 团队并入 MITO，服务存疑 |

## 3. 结构性结论

1. **registry 数据质量是当前最大问题**：7 处确定性错误 + ~20 处存疑，先修数据再谈功能（P0）
2. **thinking 机制泛化是最高性价比功能扩展**：现有 DeepSeek `thinking:{type}` override 与 6+ 厂商同构（P1-A），扩展为配置数据（RFC-0017 reasoningMap）即可覆盖；Qwen 系 `enable_thinking` 是另一族（P1-B）
3. **`max_tokens_key` 是首个被数据驱动的 profile 新字段**（P1-C）：6 家只认 max_tokens，推理分支必踩
4. **认证/headers 差异大多已有兜底机制**（with_headers/bodyOverrides），无需 code，只需文档化
5. **协议不匹配 3 家 + 退役 2 家**需产品决策（移除/专用适配/降级标注）

## 4. 建议下一步

1. P0：修复 7 处 registry base_url（确定性）+ 抽查 ~20 处存疑
2. RFC-0017 阶段 2 落地：reasoningMap 配置数据 + max_tokens_key 字段
3. 对 P1-A 同构家族写 wiremock 测试（borrow cerebras thinking cassette 模式）
4. 存疑 ~70 家归档，等真 key 实测
