# Batch 06 — Model Request Config 调研

> 状态: ✅ 已完成（2026-08-01 调研）· 厂商数: 40
> 模板见 [_template.md](_template.md) · 方法论见 [README.md](README.md)
> 证据硬性要求: 每条目必须有例子或证明(A/B/C 级),禁止编造来源。

## 厂商清单

| # | id | display | base_url | env_var | profile |
|---|---|---|---|---|---|
| 1 | tencent | Tencent (混元/Hunyuan) | https://api.hunyuan.cloud.tencent.com/v1 | TENCENT_API_KEY | OpenAICompatProfile::full() |
| 2 | tencent_coding_plan | Tencent Coding Plan (China) | https://api.lkeap.cloud.tencent.com/coding/v3 | TENCENT_CODING_PLAN_API_KEY | OpenAICompatProfile::full() |
| 3 | tencent_token_plan | Tencent Token Plan | https://api.lkeap.cloud.tencent.com/plan/v3 | TENCENT_TOKEN_PLAN_API_KEY | OpenAICompatProfile::full() |
| 4 | tencent_token_plan_enterprise_auto | 腾讯云 Token Plan / Token Plan 企业版轻享套餐 | https://tokenhub.tencentmaas.com/plan/v3 | TENCENT_TOKEN_PLAN_ENTERPRISE_API_KEY | OpenAICompatProfile::full() |
| 5 | tencent_token_plan_enterprise_pro | 腾讯云 Token Plan / Token Plan 企业版专业套餐 | https://tokenhub.tencentmaas.com/plan/v3 | TENCENT_TOKEN_PLAN_ENTERPRISE_API_KEY | OpenAICompatProfile::full() |
| 6 | tencent_token_plan_general_personal | 腾讯云 Token Plan / 通用 Token Plan（个人版） | https://api.lkeap.cloud.tencent.com/plan/v3 | TENCENT_TOKEN_PLAN_API_KEY | OpenAICompatProfile::full() |
| 7 | tencent_token_plan_hy_personal | 腾讯云 Token Plan / Hy Token Plan（个人版） | https://api.lkeap.cloud.tencent.com/plan/v3 | TENCENT_TOKEN_PLAN_API_KEY | OpenAICompatProfile::full() |
| 8 | tencent_tokenhub | Tencent TokenHub | https://tokenhub.tencentmaas.com/v1 | TENCENT_TOKENHUB_API_KEY | OpenAICompatProfile::full() |
| 9 | tensormesh | Tensormesh | https://serverless.tensormesh.ai | YOUR_API_KEY | OpenAICompatProfile::full() |
| 10 | the_grid_ai | The Grid AI | https://thegrid.ai/docs | THEGRIDAI_API_KEY | OpenAICompatProfile::full() |
| 11 | thinkingmachines | Thinking Machines | https://tinker.thinkingmachines.dev/services/tinker-prod/oai/api/v1 | TINKER_API_KEY | OpenAICompatProfile::full() |
| 12 | tinfoil | Tinfoil | https://inference.tinfoil.sh/v1 | TINFOIL_API_KEY | OpenAICompatProfile::full() |
| 13 | togetherai | Together AI | https://api.together.xyz/v1 | TOGETHER_API_KEY | OpenAICompatProfile::full() |
| 14 | tokenflux | Tokenflux | https://tokenflux.ai/v1 | TOKENFLUX_API_KEY | OpenAICompatProfile::full() |
| 15 | tokenpony | TokenPony | https://api.tokenpony.com/v1 | TOKENPONY_API_KEY | OpenAICompatProfile::full() |
| 16 | trustedrouter | TrustedRouter | https://api.trustedrouter.com/v1 | TRUSTEDROUTER_API_KEY | OpenAICompatProfile::full() |
| 17 | tundra | Tundra | https://api.tundra.ai/v1 | TUNDRA_API_KEY | OpenAICompatProfile::full() |
| 18 | umans_ai | Umans AI | https://api.code.umans.ai/v1 | UMANS_AI_API_KEY | OpenAICompatProfile::full() |
| 19 | unorouter | UnoRouter | https://unorouter.com/en | UNOROUTER_API_KEY | OpenAICompatProfile::full() |
| 20 | upstage | Upstage | https://api.upstage.ai/v1 | UPSTAGE_API_KEY | OpenAICompatProfile::full() |
| 21 | v0 | v0 (Vercel) | https://api.v0.dev/v1 | V0_API_KEY | OpenAICompatProfile::full() |
| 22 | venice | Venice | https://api.venice.ai/api/v1 | VENICE_API_KEY | OpenAICompatProfile::full() |
| 23 | vercel | Vercel | https://api.v0.dev/v1 | VERCEL_API_KEY | OpenAICompatProfile::full() |
| 24 | vivgrid | Vivgrid | https://api.vivgrid.com/v1 | VIVGRID_API_KEY | OpenAICompatProfile::full() |
| 25 | volc_engine | VolcEngine | https://ark.cn-beijing.volces.com | ARK_API_KEY | OpenAICompatProfile::full() |
| 26 | vultr | Vultr | https://api.vultrinference.com/v1 | VULTR_API_KEY | OpenAICompatProfile::full() |
| 27 | wafer | Wafer | https://api.wafer.ai/v1 | WAFER_API_KEY | OpenAICompatProfile::full() |
| 28 | wandb | Weights & Biases | https://api.inference.wandb.ai/v1 | WANDB_API_KEY | OpenAICompatProfile::full() |
| 29 | xiaomi_token_plan_ams | Xiaomi Token Plan (Europe) | https://token-plan-ams.xiaomimimo.com/v1 | MIMO_API_KEY | OpenAICompatProfile::full() |
| 30 | xiaomi_token_plan_cn | Xiaomi Token Plan (China) | https://token-plan-cn.xiaomimimo.com/v1 | MIMO_API_KEY | OpenAICompatProfile::full() |
| 31 | xiaomi_token_plan_sgp | Xiaomi Token Plan (Singapore) | https://token-plan-sgp.xiaomimimo.com/v1 | MIMO_API_KEY | OpenAICompatProfile::full() |
| 32 | xiaomimimo | Xiaomi MiMo | https://mimo.xiaomi.com/v1 | XIAOMI_API_KEY | OpenAICompatProfile::full() |
| 33 | xpersona | Xpersona | /v1 | XPERSONA_API_KEY | OpenAICompatProfile::full() |
| 34 | xunfei | Xunfei | https://spark-api-open.xf-yun.com/v1 | XUNFEI_API_PASSWORD | OpenAICompatProfile::full() |
| 35 | zai | Zai | https://api.z.ai/api/paas/v4 | ZAI_API_KEY | OpenAICompatProfile::full() |
| 36 | zai_coding_plan | Z.AI Coding Plan | https://api.z.ai/api/anthropic | ZHIPU_API_KEY | OpenAICompatProfile::full() |
| 37 | zeldoc | Zeldoc | https://api.zeldoc.ai/v1 | ZELDOC_API_KEY | OpenAICompatProfile::full() |
| 38 | zenmux | ZenMux | https://zenmux.ai/api/v1 | ZENMUX_API_KEY | OpenAICompatProfile::full() |
| 39 | zhipu_v4 | ZhipuV4 | https://open.bigmodel.cn | ZHIPU_API_KEY | OpenAICompatProfile::full() |
| 40 | zhipuai_coding_plan | Zhipu AI Coding Plan | https://docs.bigmodel.cn/cn/coding-plan/quick-start | ZHIPU_API_KEY | OpenAICompatProfile::full() |

---

## 条目

### tencent — Tencent (混元/Hunyuan)

- **registry 现状**：profile=`full()` · base_url=`https://api.hunyuan.cloud.tencent.com/v1` · env=`TENCENT_API_KEY`
- **变体**：tencent_coding_plan / tencent_token_plan / tencent_tokenhub（同厂商不同入口，各自成条目）

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异（vision 走标准 `image_url`，支持 base64） | - | C | https://cloud.tencent.com/document/product/1729/111007 |
| 思考机制 | TokenHub 侧 hy3/hy3-preview 支持深度思考：请求体 `thinking` 对象 + `reasoning_effort`（默认 `low`；带 tools 时 low 自动映射 high）；响应含 `reasoning_content`。独立混元 API（api.hunyuan.cloud.tencent.com）文档未见 thinking 字段 | `{"model":"hy3","messages":[...],"thinking":{"type":"enabled"}}`；响应 `"message":{"content":"...","reasoning_content":"..."}` | C | https://cloud.tencent.com/document/product/1823/132252 |
| 流式/usage | 无差异（标准 `stream_options:{"include_usage":true}`，usage 含 `prompt_tokens_details.cached_tokens` / `completion_tokens_details.reasoning_tokens`） | - | C | https://cloud.tencent.com/document/product/1823/132252 |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 混元独立 API 支持自定义参数 `enable_enhancement`（增强） | `{"model":"hunyuan-turbos-latest","messages":[...],"enable_enhancement":true}` | C | https://cloud.tencent.com/document/product/1729/111007 |
| headers/认证 | 无差异（`Authorization: Bearer $HUNYUAN_API_KEY`） | - | C | https://cloud.tencent.com/document/product/1729/111007 |
| URL/端点 | 无差异（base_url 与官方一致） | - | C | https://cloud.tencent.com/document/product/1729/111007 |
| 模型 ID | 版本代号：`hunyuan-turbos-latest`、`hunyuan-vision`；TokenHub 侧 `hy3` / `hy3-preview` | - | C | https://cloud.tencent.com/document/product/1823/132252 |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖
- **aimux 代码位置**：[convert.rs](aimux-providers/src/openai/convert.rs#L1485)（apply_deepseek_override 仅按 provider_options["deepseek"] 注入 thinking）
- **差距说明**：`thinking` 对象（hy3 深度思考）与 `enable_enhancement` 无内置支持；reasoning_content 响应已支持（[model.rs](aimux-providers/src/openai/model.rs#L558)）。
- **建议动作**：thinking 字段可先用 bodyOverrides 兜底；若 hy3 成为主模型，评估通用 `thinking` override（GLM 系同款，见 zai/zhipu_v4）。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证(仅文档引用)
- **存疑标记**：无

### tencent_coding_plan — Tencent Coding Plan (China)

- **registry 现状**：profile=`full()` · base_url=`https://api.lkeap.cloud.tencent.com/coding/v3` · env=`TENCENT_CODING_PLAN_API_KEY`
- **变体**：无（与 tencent_token_plan 并列，Coding Plan 偏编程场景）

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 套餐内模型（hy3 系）沿用 TokenHub 深度思考机制（`thinking.type.enabled`），未见独立文档，标 ⚠️ | 同 tencent 条目 `{"thinking":{"type":"enabled"}}` | C(⚠️) | https://cloud.tencent.com/document/product/1823/130092 |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer + Coding Plan 专用 API Key） | - | C | https://cloud.tencent.com/document/product/1823/130092 |
| URL/端点 | 无差异（`https://api.lkeap.cloud.tencent.com/coding/v3` 与官方一致；另提供 Anthropic 兼容端点 `/coding/anthropic`） | - | C | https://cloud.tencent.com/document/product/1823/130092 |
| 模型 ID | 套餐配额制模型（hy3 等），未见独立代号 | - | C | https://cloud.tencent.com/document/product/1823/130092 |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖（除 thinking 字段，同 tencent）
- **aimux 代码位置**：[openai_compat_registry.rs](aimux-providers/src/openai_compat_registry.rs#L1915)
- **差距说明**：无独立差距；thinking 覆盖见 tencent 条目。
- **建议动作**：无需动作（bodyOverrides 兜底 thinking）。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证(仅文档引用)
- **存疑标记**：thinking 机制 ⚠️（推断自 TokenHub hy3 文档，套餐内模型清单未单独确认）

### tencent_token_plan — Tencent Token Plan

- **registry 现状**：profile=`full()` · base_url=`https://api.lkeap.cloud.tencent.com/plan/v3` · env=`TENCENT_TOKEN_PLAN_API_KEY`
- **变体**：tencent_token_plan_general_personal / tencent_token_plan_hy_personal（个人版，同 base_url）；tencent_token_plan_enterprise_auto / tencent_token_plan_enterprise_pro（企业版，base_url=`https://tokenhub.tencentmaas.com/plan/v3`，env=`TENCENT_TOKEN_PLAN_ENTERPRISE_API_KEY`）

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 套餐内模型沿用 TokenHub 深度思考（`thinking.type.enabled` / `enable_thinking` 按模型族），未单列文档 ⚠️ | 同 tencent 条目 | C(⚠️) | https://cloud.tencent.com/developer/article/2675771 |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer + Token Plan 专用 API Key） | - | C | https://intl.cloud.tencent.com/document/product/1300/81317 |
| URL/端点 | 无差异（`/plan/v3` 与官方一致；另支持 Anthropic 兼容接口） | - | C | https://cloud.tencent.com/developer/article/2675771 |
| 模型 ID | 套餐内多模型（qwen/hy3/deepseek 等），按网关路由 | - | C | https://cloud.tencent.com/developer/article/2675771 |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖（除 thinking 字段，同 tencent）
- **aimux 代码位置**：[openai_compat_registry.rs](aimux-providers/src/openai_compat_registry.rs#L1924)
- **差距说明**：企业版两个条目仅套餐差异，request 层无差别，可合并。
- **建议动作**：无需动作；思考类字段 bodyOverrides 兜底。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证(仅文档引用)
- **存疑标记**：思考机制 ⚠️（推断）

### tencent_token_plan_enterprise_auto — Tencent Token Plan 企业版轻享套餐

- **registry 现状**：profile=`full()` · base_url=`https://tokenhub.tencentmaas.com/plan/v3` · env=`TENCENT_TOKEN_PLAN_ENTERPRISE_API_KEY`
- **变体**：无（**并入主条目 tencent_token_plan**——同套餐体系，仅 base_url 指向 tokenhub.tencentmaas.com 子路径 `/plan/v3`，request 构造差异与主条目一致）

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（并入主条目） | - | C | https://cloud.tencent.com/developer/article/2675771 |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 同主条目（TokenHub 深度思考 `thinking.type.enabled` / `enable_thinking` by-model，⚠️ 推断） | - | C(⚠️) | 见 tencent_tokenhub |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer + 企业版专用 Key） | - | C | https://cloud.tencent.com/document/product/1823/130092 |
| URL/端点 | 无差异（`/plan/v3` 子路径为套餐入口） | - | C | https://cloud.tencent.com/developer/article/2675771 |
| 模型 ID | 同主条目 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：[openai_compat_registry.rs](aimux-providers/src/openai_compat_registry.rs#L1933)
- **差距说明**：与 tencent_token_plan 仅套餐/额度差异，request 层相同。
- **建议动作**：无需动作（可与主条目合并声明）。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证(仅文档引用)
- **存疑标记**：思考机制 ⚠️（推断）

### tencent_token_plan_enterprise_pro — Tencent Token Plan 企业版专业套餐

- **registry 现状**：profile=`full()` · base_url=`https://tokenhub.tencentmaas.com/plan/v3` · env=`TENCENT_TOKEN_PLAN_ENTERPRISE_API_KEY`
- **变体**：无（**并入主条目 tencent_token_plan**，与 enterprise_auto 仅套餐档位差异）

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（并入主条目） | - | - | - |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 同主条目（⚠️ 推断） | - | C(⚠️) | 见 tencent_tokenhub |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer + 企业版专用 Key） | - | C | https://cloud.tencent.com/document/product/1823/130092 |
| URL/端点 | 无差异 | - | C | https://cloud.tencent.com/developer/article/2675771 |
| 模型 ID | 同主条目 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：[openai_compat_registry.rs](aimux-providers/src/openai_compat_registry.rs#L1942)
- **差距说明**：无独立差距。
- **建议动作**：无需动作。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证(仅文档引用)
- **存疑标记**：思考机制 ⚠️（推断）

### tencent_token_plan_general_personal — Tencent Token Plan 通用（个人版）

- **registry 现状**：profile=`full()` · base_url=`https://api.lkeap.cloud.tencent.com/plan/v3` · env=`TENCENT_TOKEN_PLAN_API_KEY`
- **变体**：无（**并入主条目 tencent_token_plan**，同 base_url 与 env）

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（并入主条目） | - | - | - |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 同主条目（⚠️ 推断） | - | C(⚠️) | 见 tencent_tokenhub |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer + 个人版 Key） | - | C | https://cloud.tencent.com/developer/article/2675771 |
| URL/端点 | 无差异 | - | C | https://cloud.tencent.com/developer/article/2675771 |
| 模型 ID | 同主条目 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：[openai_compat_registry.rs](aimux-providers/src/openai_compat_registry.rs#L1951)
- **差距说明**：与 tencent_token_plan 完全同构。
- **建议动作**：无需动作。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证(仅文档引用)
- **存疑标记**：思考机制 ⚠️（推断）

### tencent_token_plan_hy_personal — Tencent Token Plan Hy（个人版）

- **registry 现状**：profile=`full()` · base_url=`https://api.lkeap.cloud.tencent.com/plan/v3` · env=`TENCENT_TOKEN_PLAN_API_KEY`
- **变体**：无（**并入主条目 tencent_token_plan**；"Hy" 指套餐内主推混元 hy3 模型）

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（并入主条目） | - | - | - |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 套餐主打 hy3，深度思考 `thinking.type.enabled` 直接适用（⚠️ 推断） | `{"model":"hy3","messages":[...],"thinking":{"type":"enabled"}}` | C(⚠️) | https://cloud.tencent.com/document/product/1823/132252 |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer + 个人版 Key） | - | C | https://cloud.tencent.com/developer/article/2675771 |
| URL/端点 | 无差异 | - | C | https://cloud.tencent.com/developer/article/2675771 |
| 模型 ID | 同主条目（hy3 系） | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖（thinking 同 tencent）
- **aimux 代码位置**：[openai_compat_registry.rs](aimux-providers/src/openai_compat_registry.rs#L1960)
- **差距说明**：无独立差距。
- **建议动作**：无需动作（bodyOverrides 兜底 thinking）。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证(仅文档引用)
- **存疑标记**：思考机制 ⚠️（推断）

### tencent_tokenhub — Tencent TokenHub

- **registry 现状**：profile=`full()` · base_url=`https://tokenhub.tencentmaas.com/v1` · env=`TENCENT_TOKENHUB_API_KEY`
- **变体**：无（网关本体；token_plan_enterprise_* 走 `/plan/v3` 子路径）

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | **by-model 两套思考开关**：Qwen3.5 系用顶层布尔 `enable_thinking`（默认开启）+ `preserve_thinking` + 消息内 `/think`、`/no_think` prompt 开关；hy3 系用 `thinking` 对象 | `{"model":"qwen3.5-plus","messages":[...],"enable_thinking":false}`（可关）；`{"model":"hy3","messages":[...],"thinking":{"type":"enabled"}}` | C | https://cloud.tencent.com/document/product/1823/132247 ；https://cloud.tencent.com/document/product/1823/132252 |
| 流式/usage | 无差异（标准 include_usage） | - | C | https://cloud.tencent.com/document/product/1823/132252 |
| 消息格式 | 推理过程统一 `reasoning_content` 字段 | - | C | https://cloud.tencent.com/document/product/1823/132247 |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer + TokenHub API Key） | - | C | https://cloud.tencent.com/document/product/1823/132247 |
| URL/端点 | 无差异（`/v1` 与官方一致；hy3 同时兼容 Responses / Anthropic 协议） | - | C | https://cloud.tencent.com/document/product/1823/132252 |
| 模型 ID | 网关聚合多模型族：`qwen3.5-plus`、`hy3`、`hy3-preview` 等 | - | C | https://cloud.tencent.com/document/product/1823/132247 |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖
- **aimux 代码位置**：[openai_compat_registry.rs](aimux-providers/src/openai_compat_registry.rs#L1969)
- **差距说明**：`enable_thinking`/`thinking` 均需 bodyOverrides；`/think`、`/no_think` 是消息内容层面的协议，aimux 无内置（需用户拼进 prompt）。
- **建议动作**：bodyOverrides 兜底；若做 reasoningMap，TokenHub 是"一网关双机制"的典型样例，需要 by-model 映射。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证(仅文档引用)
- **存疑标记**：无

### tensormesh — Tensormesh

- **registry 现状**：profile=`full()` · base_url=`https://serverless.tensormesh.ai` · env=`YOUR_API_KEY`
- **变体**：无

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | tools 支持但**每个 tool 必须带非空 description**，否则 400 | `{"tools":[{"type":"function","function":{"name":"get_weather","description":"...","parameters":{...}}}]}` | C | https://docs.litellm.ai/docs/providers/tensormesh |
| 思考机制 | 思考模式走 **vLLM chat-template 控制**：`chat_template_kwargs` 传 `thinking` + `reasoning_effort`；输出在 `reasoning_content` | `extra_body={"chat_template_kwargs":{"thinking":true,"reasoning_effort":"high"}}` | C | https://docs.litellm.ai/docs/providers/tensormesh |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer）；litellm 侧 env 名为 `TENSORMESH_INFERENCE_API_KEY`，registry 的 `YOUR_API_KEY` 是占位符 | - | C | https://docs.litellm.ai/docs/providers/tensormesh |
| URL/端点 | **registry 缺 `/v1` 后缀**：litellm/官方默认 `https://serverless.tensormesh.ai/v1` | `POST https://serverless.tensormesh.ai/v1/chat/completions` | C | https://docs.litellm.ai/docs/providers/tensormesh |
| 模型 ID | catalog 式前缀模型：`openai/gpt-oss-120b`、`MiniMaxAI/MiniMax-M2.5`、`deepseek-ai/DeepSeek-V4-Flash` | - | C | https://docs.litellm.ai/docs/providers/tensormesh |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖
- **aimux 代码位置**：[openai_compat_registry.rs](aimux-providers/src/openai_compat_registry.rs#L1978)
- **差距说明**：base_url 少 `/v1`（[model.rs](aimux-providers/src/openai/model.rs#L73) 拼接后变 `.../chat/completions`，非 `.../v1/chat/completions`）；`chat_template_kwargs` 思考开关需 bodyOverrides。
- **建议动作**：registry base_url 补 `/v1`；思考用 bodyOverrides。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证(仅文档引用)
- **存疑标记**：无

### the_grid_ai — The Grid AI

- **registry 现状**：profile=`full()` · base_url=`https://thegrid.ai/docs` · env=`THEGRIDAI_API_KEY`
- **变体**：无

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（`max_tokens` 已废弃，建议 `max_completion_tokens`） | - | C | https://thegrid.ai/docs/api-reference/consumption-api |
| 能力支持 | 无差异（tools/tool_choice/response_format/logprobs/seed 全支持） | - | C | https://thegrid.ai/docs/api-reference/consumption-api |
| 思考机制 | `reasoning_effort`（none/minimal/low/medium/high/xhigh）标准透传 | `{"model":"text-prime","messages":[...],"reasoning_effort":"high"}` | C | https://thegrid.ai/docs/api-reference/consumption-api |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | **OpenRouter 风格路由字段**：`provider` 对象、`route:"fallback"`、`safety_identifier`、`prompt_cache_key`、`prompt_cache_retention`、`service_tier`（auto/default/flex/scale/priority）、`web_search_options`、`verbosity` | `{"model":"text-prime","provider":{"allow_fallbacks":true},"route":"fallback","safety_identifier":"..."}` | C | https://thegrid.ai/docs/api-reference/consumption-api |
| headers/认证 | OpenAI 面 `Authorization: Bearer`；Anthropic 面（`https://messages-beta.api.thegrid.ai/v1`）用 `x-api-key` | - | C | https://thegrid.ai/docs/api-reference/consumption-api |
| URL/端点 | **registry base_url 是 docs 页面**；真实 API 为 `https://api.thegrid.ai/v1` | `POST https://api.thegrid.ai/v1/chat/completions` | C | https://thegrid.ai/docs/api-reference/consumption-api |
| 模型 ID | 模型=instrument 代号：`text-prime`、`code-max`、`claude-opus-latest`（非 org 前缀） | - | C | https://thegrid.ai/docs/api-reference/consumption-api |

#### 2. aimux 现状对比

- **对比结论**：⚠️ 不一致
- **aimux 代码位置**：[openai_compat_registry.rs](aimux-providers/src/openai_compat_registry.rs#L1987)
- **差距说明**：base_url 错误（docs 页不是 API 端点）；provider/route/safety_identifier 等路由字段无内置（bodyOverrides 可兜底）。
- **建议动作**：registry base_url 改为 `https://api.thegrid.ai/v1`；特殊字段文档化走 bodyOverrides。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证(仅文档引用)
- **存疑标记**：无

### thinkingmachines — Thinking Machines (Tinker)

- **registry 现状**：profile=`full()` · base_url=`https://tinker.thinkingmachines.dev/services/tinker-prod/oai/api/v1` · env=`TINKER_API_KEY`
- **变体**：无

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 训练/推理 API，思考行为取决于权重路径（sampler weight），无独立开关字段证据 | - | C(⚠️) | https://tinker-docs.thinkingmachines.ai/tinker/compatible-apis/openai/ |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer，`{env:TINKER_API_KEY}` 官方示例） | - | C | https://tinker-docs.thinkingmachines.ai/tutorials/deployment/opencode/ |
| URL/端点 | 无差异（registry 与官方 OAI 端点完全一致） | `POST https://tinker.thinkingmachines.dev/services/tinker-prod/oai/api/v1/chat/completions` | C | https://tinker-docs.thinkingmachines.ai/tinker/compatible-apis/openai/ |
| 模型 ID | **模型名 = Tinker 采样器权重路径**：`tinker://0034d8c9...`（非普通模型代号） | `{"model":"tinker://0034d8c9...","messages":[...]}` | C | https://tinker-docs.thinkingmachines.ai/tinker/compatible-apis/openai/ |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：[openai_compat_registry.rs](aimux-providers/src/openai_compat_registry.rs#L1996)
- **差距说明**：无（模型 ID 特殊但 aimux 直接透传 model 字段，无需转换）。
- **建议动作**：补文档说明模型名须为 `tinker://` 权重路径；无需代码改动。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证(仅文档引用)
- **存疑标记**：思考机制无直接证据（本厂商为训练 API，chat 面用于权重推理）

### tinfoil — Tinfoil

- **registry 现状**：profile=`full()` · base_url=`https://inference.tinfoil.sh/v1` · env=`TINFOIL_API_KEY`
- **变体**：无

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异（tool calling、structured outputs、多模态 base64、文档处理均 OpenAI 兼容） | - | C | https://docs.tinfoil.sh/introduction |
| 思考机制 | 无差异（无独立开关证据） | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | SDK 额外做远端 attestation 验证（传输层，非请求体字段），请求体无特殊字段 | - | C | https://docs.tinfoil.sh/introduction |
| headers/认证 | 无差异（Bearer；SDK 同时校验 enclave 证明） | `curl -X POST https://inference.tinfoil.sh/v1/chat/completions -H "Content-Type..."` | C | https://tinfoil.sh/inference |
| URL/端点 | 无差异 | - | C | https://tinfoil.sh/inference |
| 模型 ID | 未确认模型清单 ⚠️（docs 未列出 model ID 表） | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：[openai_compat_registry.rs](aimux-providers/src/openai_compat_registry.rs#L2005)
- **差距说明**：无 request 层差距（attestation 属传输层，不在 aimux 范围）。
- **建议动作**：无需动作；模型 ID 待补。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证(仅文档引用)
- **存疑标记**：模型 ID 清单证据不足

### togetherai — Together AI

- **registry 现状**：profile=`full()` · base_url=`https://api.together.xyz/v1` · env=`TOGETHER_API_KEY`
- **变体**：无

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（`max_tokens` 标准用法；支持 `stop`） | - | C | https://docs.together.ai/docs/inference/chat/parameters |
| 能力支持 | 无差异（temperature/top_p/**top_k**/seed/repetition_penalty；`response_format` json_schema 结构化输出；tools/logprobs 均支持） | `{"model":"meta-llama/Llama-3.3-70B-Instruct-Turbo","messages":[...],"max_tokens":100,"stop":["\n\n"],"seed":42,"top_k":40}` | C | https://docs.together.ai/docs/inference/chat/parameters |
| 思考机制 | 推理模型（如 DeepSeek-R1）无独立开关字段证据；输出 reasoning_content 标准 | - | - | - |
| 流式/usage | 无差异（标准 SSE） | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer） | - | - | - |
| URL/端点 | 无差异 | - | C | https://docs.together.ai/docs/inference/chat/overview |
| 模型 ID | **org 前缀模型名**：`meta-llama/Llama-3.3-70B-Instruct-Turbo`、`deepseek-ai/DeepSeek-R1` 等（直接透传即可） | - | C | https://docs.together.ai/docs/inference/chat/parameters |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：[openai_compat_registry.rs](aimux-providers/src/openai_compat_registry.rs#L2014)；top_k 白名单 [convert.rs](aimux-providers/src/openai/convert.rs#L1111)
- **差距说明**：无（full() 已覆盖 top_k/tools/response_format/usage）。
- **建议动作**：补一条结构化输出 + top_k 的 cassette 测试即可。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证(仅文档引用)
- **存疑标记**：无

### tokenflux — TokenFlux

- **registry 现状**：profile=`full()` · base_url=`https://tokenflux.ai/v1` · env=`TOKENFLUX_API_KEY`
- **变体**：无

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异（Models/Chat Completions/Messages/Embeddings/Images API 齐备） | - | C | https://tokenflux.ai/docs/introduction |
| 思考机制 | 无差异（未见独立开关文档） | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 管理面用 `X-Api-Key`（projects 接口）；模型推理面 OpenAI 兼容 Bearer ⚠️ 推理面 header 细节未逐条确认 | - | C(⚠️) | https://tokenflux.ai/ |
| URL/端点 | **双 base URL**：推荐 `https://tokenflux.ai/openai/v1`（drop-in），`https://tokenflux.ai/v1` 亦可；registry 用后者 | `POST https://tokenflux.ai/openai/v1/chat/completions` | C | https://tokenflux.ai/docs/api-reference/embeddings |
| 模型 ID | 未逐一确认 ⚠️ | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：[openai_compat_registry.rs](aimux-providers/src/openai_compat_registry.rs#L2024)
- **差距说明**：无 request 构造差距。
- **建议动作**：无需动作。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证(仅文档引用)
- **存疑标记**：推理面认证 header 细节、模型 ID 清单证据不足

### tokenpony — TokenPony（小马算力）

- **registry 现状**：profile=`full()` · base_url=`https://api.tokenpony.com/v1` · env=`TOKENPONY_API_KEY`
- **变体**：无

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异（DeepSeek/GLM/MiniMax 等聚合，兼容 OpenAI 规范） | - | C | https://www.tokenpony.cn/ |
| 思考机制 | 透传上游（deepseek 系 reasoning），无独立开关文档 ⚠️ | - | C(⚠️) | https://www.tokenpony.cn/ |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer 聚合网关） | - | C | https://www.tokenpony.cn/ |
| URL/端点 | **registry 域名存疑**：官方为 `www.tokenpony.cn` / `api.tokenpony.cn/v1`（chats 项目同）；registry 写 `api.tokenpony.com` ⚠️ | `https://api.tokenpony.cn/v1/chat/completions` | B | [chats/doc/zh-CN/release-notes/1.8.1.md](reference/chats/doc/zh-CN/release-notes/1.8.1.md#L151) ；https://www.tokenpony.cn/ |
| 模型 ID | 上游模型直通（`deepseek-v3.*` 等，chats 1.9.1 提及） | - | B | [chats/doc/zh-CN/release-notes/1.9.1.md](reference/chats/doc/zh-CN/release-notes/1.9.1.md#L126) |

#### 2. aimux 现状对比

- **对比结论**：⚠️ 不一致（base_url 域名待核）
- **aimux 代码位置**：[openai_compat_registry.rs](aimux-providers/src/openai_compat_registry.rs#L2032)
- **差距说明**：`.com` vs `.cn` 域名差异需实测确认哪个可解析/可服务；若 `.cn` 为真则 registry 需修正。
- **建议动作**：实测两个域名连通性后修正 base_url。

#### 3. 证据与验证

- **证据等级**：B+C
- **验证状态**：🔲 未验证(仅文档引用)
- **存疑标记**：base_url 域名 ⚠️

### trustedrouter — TrustedRouter

- **registry 现状**：profile=`full()` · base_url=`https://api.trustedrouter.com/v1` · env=`TRUSTEDROUTER_API_KEY`
- **变体**：无

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 无差异（透传上游） | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | **OpenRouter 风格 `provider` 路由对象**：`min_privacy`（"zdr"/"confidential"，fail-closed 硬约束）、`data_collection:"deny"`；x402 自动充值 | `{"model":"your/model","provider":{"min_privacy":"zdr"}}` | C | https://trustedrouter.com/docs |
| headers/认证 | 无差异（Bearer，`TRUSTEDROUTER_API_KEY` 与 registry env 一致） | - | C | https://trustedrouter.com/docs |
| URL/端点 | 无差异 | - | C | https://trustedrouter.com/docs |
| 模型 ID | **preset 路由模型**：`trustedrouter/auto`（failover）、`trustedrouter/eu`、`trustedrouter/socrates`、Synth presets（`trustedrouter/iris-2.0` 等） | - | C | https://trustedrouter.com/docs |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖
- **aimux 代码位置**：[openai_compat_registry.rs](aimux-providers/src/openai_compat_registry.rs#L2041)
- **差距说明**：`provider` 路由对象需 bodyOverrides；其余标准。
- **建议动作**：bodyOverrides 兜底；模型 ID 约定写进厂商文档。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证(仅文档引用)
- **存疑标记**：无

### tundra — Tundra

- **registry 现状**：profile=`full()` · base_url=`https://api.tundra.ai/v1` · env=`TUNDRA_API_KEY`
- **变体**：无

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（证据不足） | - | - | - |
| 能力支持 | 无差异（证据不足） | - | - | - |
| 思考机制 | 无差异（证据不足） | - | - | - |
| 流式/usage | 无差异（证据不足） | - | - | - |
| 消息格式 | 无差异（证据不足） | - | - | - |
| 特殊字段 | 无差异（证据不足） | - | - | - |
| headers/认证 | 无差异（证据不足） | - | - | - |
| URL/端点 | 无法确认 ⚠️（公开检索未命中 api.tundra.ai 有效文档/服务） | - | - | - |
| 模型 ID | 无法确认 ⚠️ | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：❓ 无法判断（证据不足）
- **aimux 代码位置**：[openai_compat_registry.rs](aimux-providers/src/openai_compat_registry.rs#L2050)
- **差距说明**：未找到该厂商任何可验证的 API 文档/集成代码。
- **建议动作**：保留 full() 默认假设，标记"证据不足"，后续有实测再补。

#### 3. 证据与验证

- **证据等级**：无
- **验证状态**：🔲 未验证
- **存疑标记**：⚠️ 全条目存疑（无任何公开证据）

### umans_ai — Umans AI

- **registry 现状**：profile=`full()` · base_url=`https://api.code.umans.ai/v1` · env=`UMANS_AI_API_KEY`
- **变体**：无

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | **Coding Plan 走 Anthropic Messages 协议，thinking 用 Anthropic budget 模式**：`{"thinking":{"mode":"budget","efforts":["minimal","low","medium","high","xhigh"]}}`；OpenAI 面（`/v1`）无独立文档 ⚠️ | 见左（Anthropic 面） | C(⚠️) | https://github.com/can1357/oh-my-pi/issues/3286 |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | Anthropic 面 `x-api-key`；OpenAI 面假设 Bearer ⚠️ | - | C(⚠️) | https://github.com/can1357/oh-my-pi/issues/3192 |
| URL/端点 | 官方公开 base `https://api.code.umans.ai`（HF inference-providers 收录为 OpenAI 兼容）；registry 的 `/v1` 子路径未经官方文档确认 ⚠️ | `https://api.code.umans.ai/v1/chat/completions`（推断） | C(⚠️) | https://huggingface.co/spaces/huggingface/HuggingDiscussions/discussions/49 |
| 模型 ID | `umans-glm-5.2` 等 `umans-*` 前缀模型 | - | C | https://github.com/can1357/oh-my-pi/issues/3192 |

#### 2. aimux 现状对比

- **对比结论**：⚠️ 不一致（协议面存疑）
- **aimux 代码位置**：[openai_compat_registry.rs](aimux-providers/src/openai_compat_registry.rs#L2059)
- **差距说明**：官方主推 Coding Plan（Anthropic 协议）；OpenAI 兼容面是否存在 `/v1` 需实测。若仅 Anthropic，aimux OpenAI 请求会失败。
- **建议动作**：实测 `https://api.code.umans.ai/v1/chat/completions` 连通性；确认前标注存疑。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证(仅文档引用)
- **存疑标记**：OpenAI 面/认证 header ⚠️

### unorouter — UnoRouter

- **registry 现状**：profile=`full()` · base_url=`https://unorouter.com/en` · env=`UNOROUTER_API_KEY`
- **变体**：无

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异（200+ 模型 OpenAI 兼容网关） | - | C | https://sourceforge.net/software/product/UnoRouter/ |
| 思考机制 | 无差异（透传上游） | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer） | - | C | https://unorouter.com/en |
| URL/端点 | **registry base_url 是官网路径**；API 端点为 `https://api.unorouter.com/v1` | `curl -X POST https://api.unorouter.com/v1/chat/completions` | C | https://unorouter.com/en/docs/integrations/sillytavern |
| 模型 ID | 平台模型 slug（未逐一确认） | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：⚠️ 不一致（base_url 错误）
- **aimux 代码位置**：[openai_compat_registry.rs](aimux-providers/src/openai_compat_registry.rs#L2068)
- **差距说明**：`unorouter.com/en` 是营销页，请求必然 404；应改为 `https://api.unorouter.com/v1`。
- **建议动作**：修正 registry base_url。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证(仅文档引用)
- **存疑标记**：无

### upstage — Upstage

- **registry 现状**：profile=`full()` · base_url=`https://api.upstage.ai/v1` · env=`UPSTAGE_API_KEY`
- **变体**：无

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异（官方 OpenAI SDK 直连示例：stream/messages 标准） | - | C | https://console.upstage.ai/api-keys |
| 思考机制 | 无差异（Solar 推理为模型内建，无独立开关字段证据） | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异（Document AI 是独立产品线，不走 chat 请求体） | - | - | - |
| 特殊字段 | 旧版 RAG 曾传 `document` 参数，当前版本未见官方文档，⚠️ 不采信 | - | - | - |
| headers/认证 | 无差异（Bearer） | - | C | https://console.upstage.ai/api-keys |
| URL/端点 | 无差异 | - | C | https://console.upstage.ai/api-keys |
| 模型 ID | `solar-pro-3`、`solar-mini`、`solar-pro` 等 `solar-*` 代号（非 org 前缀） | - | C | https://openrouter.ai/upstage/solar-pro-3 |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：[openai_compat_registry.rs](aimux-providers/src/openai_compat_registry.rs#L2077)
- **差距说明**：无。
- **建议动作**：无需动作。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证(仅文档引用)
- **存疑标记**：无（旧版 `document` 参数未采信）

### v0 — v0 (Vercel)

- **registry 现状**：profile=`full()` · base_url=`https://api.v0.dev/v1` · env=`V0_API_KEY`
- **变体**：vercel（Vercel AI Gateway，见 vercel 条目）

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异（function/tool calls、多模态 base64 图片、低延迟流式均支持；限流 200 消息/天、上下文 128K） | `POST https://api.v0.dev/v1/chat/completions` + Bearer | C | https://apidog.com/blog/vercel-v0-1-0-md-api/ |
| 思考机制 | 无差异（v0 模型无公开 thinking 开关） | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异（image base64） | - | C | https://apidog.com/blog/vercel-v0-1-0-md-api/ |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer + v0.dev API key） | - | C | https://apidog.com/blog/vercel-v0-1-0-md-api/ |
| URL/端点 | 无差异（registry 与官方一致） | - | C | https://v0.app/docs/api/platform/overview |
| 模型 ID | v0 专有模型：`v0-1.0-md`、`v0-1.0`、`v0-1.1` 等 | - | C | https://apidog.com/blog/vercel-v0-1-0-md-api/ |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：[openai_compat_registry.rs](aimux-providers/src/openai_compat_registry.rs#L2086)
- **差距说明**：无。
- **建议动作**：无需动作。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证(仅文档引用)
- **存疑标记**：无

### venice — Venice

- **registry 现状**：profile=`full()` · base_url=`https://api.venice.ai/api/v1` · env=`VENICE_API_KEY`
- **变体**：无

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | `max_tokens` 已废弃（建议 `max_completion_tokens`）；`user` 字段被丢弃仅兼容 | - | C | https://docs.venice.ai/api-reference/endpoint/chat/completions |
| 能力支持 | 富参数：`top_k`、`logprobs`/`top_logprobs`、`min_p`、`min_temp`/`max_temp`（动态温控）、`repetition_penalty`、`stop_token_ids`、`seed`、tools/response_format json_schema | `{"model":"zai-org-glm-5-1","top_k":40,"min_p":0.05,"repetition_penalty":1.2,"stop_token_ids":[151643]}` | C | https://docs.venice.ai/api-reference/endpoint/chat/completions |
| 思考机制 | 双通道：`reasoning_effort`（none/minimal/low/medium/high/xhigh/max）+ `reasoning` 对象（`effort`/`summary`）；`venice_parameters.disable_thinking` / `strip_thinking_response` 控制开关与剥离；响应 `reasoning_content` | `{"reasoning":{"effort":"medium","summary":"auto"},"venice_parameters":{"disable_thinking":false,"strip_thinking_response":false}}` | C | https://docs.venice.ai/api-reference/endpoint/chat/completions |
| 流式/usage | 无差异（标准 `stream_options.include_usage`） | - | C | https://docs.venice.ai/api-reference/endpoint/chat/completions |
| 消息格式 | 无差异（text/image/audio/video/file 多模态） | - | C | https://docs.venice.ai/api-reference/endpoint/chat/completions |
| 特殊字段 | **`venice_parameters` 对象**：`enable_web_search`、`enable_web_citations`、`enable_web_scraping`、`include_venice_system_prompt`、`character_slug`、`enable_e2ee`、`include_search_results_in_stream`、`return_search_results_as_documents`；另有 `prompt_cache_key`、`prompt_cache_retention`（"default"/"extended"/"24h"）、`verbosity`/`text.verbosity`；`store` 字段被接受但忽略 | `{"venice_parameters":{"enable_web_search":"auto","include_venice_system_prompt":true,"character_slug":"venice","enable_e2ee":true}}` | C | https://docs.venice.ai/api-reference/endpoint/chat/completions |
| headers/认证 | Bearer 或 `SIGN-IN-WITH-X`（x402 钱包认证，`X-Sign-In-With-X` 为迁移期别名）；402 响应=余额不足 | - | C | https://docs.venice.ai/api-reference/endpoint/chat/completions |
| URL/端点 | 无差异（`/api/v1` 与官方一致） | - | C | https://docs.venice.ai/api-reference/endpoint/chat/completions |
| 模型 ID | 模型 ID 或 **feature suffix**（如追加后缀启用 web search/角色人设），可替代 venice_parameters | - | C | https://docs.venice.ai/api-reference/endpoint/chat/model_feature_suffix |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖
- **aimux 代码位置**：[openai_compat_registry.rs](aimux-providers/src/openai_compat_registry.rs#L2095)
- **差距说明**：`venice_parameters`/`prompt_cache_key`/`min_p`/`min_temp`/`max_temp`/`stop_token_ids` 等均需 bodyOverrides（convert.rs 白名单无这些键）；`reasoning` 对象需 bodyOverrides；`max_completion_tokens` 已有（[convert.rs](aimux-providers/src/openai/convert.rs#L1119)）。
- **建议动作**：venice_parameters 是封闭厂商专属字段，建议文档化走 bodyOverrides；无需 profile 扩展（除非要内置 web search）。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证(仅文档引用)
- **存疑标记**：无

### vercel — Vercel (AI Gateway)

- **registry 现状**：profile=`full()` · base_url=`https://api.v0.dev/v1` · env=`VERCEL_API_KEY`
- **变体**：v0（同 host 不同产品线，见 v0 条目）

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异（chat/completions、embeddings、reasoning 控制、file attachments、tool calling、structured outputs） | - | C | https://vercel.com/docs/ai-gateway/sdks-and-apis/openai-chat-completions |
| 思考机制 | 有独立 Reasoning 文档（控制思考量），字段与 OpenAI 标准 reasoning_effort 一致 | - | C | https://vercel.com/docs/ai-gateway/sdks-and-apis/openai-chat-completions/reasoning |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异（支持图片与 PDF 附件） | - | C | https://vercel.com/docs/ai-gateway/sdks-and-apis/openai-chat-completions |
| 特殊字段 | 网关扩展：`provider` 选项、模型 fallback、BYOK、prompt caching（advanced 文档） | - | C | https://vercel.com/docs/ai-gateway/sdks-and-apis/openai-chat-completions/advanced |
| headers/认证 | Bearer（AI Gateway API key 或 **OIDC token**） | - | C | https://vercel.com/docs/ai-gateway/sdks-and-apis/openai-chat-completions |
| URL/端点 | **registry base_url 存疑**：官方 AI Gateway 为 `https://ai-gateway.vercel.sh/v1`（registry 复用 v0 的 `api.v0.dev/v1`）⚠️ | `POST https://ai-gateway.vercel.sh/v1/chat/completions` | C | https://vercel.com/docs/ai-gateway/sdks-and-apis/openai-chat-completions |
| 模型 ID | **provider 前缀**：`anthropic/claude-opus-5`、`openai/gpt-5.6-sol` | - | C | https://vercel.com/docs/ai-gateway/sdks-and-apis/openai-chat-completions |

#### 2. aimux 现状对比

- **对比结论**：⚠️ 不一致（base_url 存疑）
- **aimux 代码位置**：[openai_compat_registry.rs](aimux-providers/src/openai_compat_registry.rs#L2104)
- **差距说明**：registry 的 vercel 与 v0 共用 `api.v0.dev/v1`，但 Vercel AI Gateway 官方端点是 `ai-gateway.vercel.sh/v1`，两者模型清单/配额体系不同；需确认 registry 意图（也许指向 v0 的 Vercel 通道）。
- **建议动作**：与仓库维护者确认意图后修正 base_url；若走 AI Gateway，`ai-gateway.vercel.sh/v1` 为标准 OpenAI 兼容，full() 适用。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证(仅文档引用)
- **存疑标记**：base_url ⚠️

### vivgrid — Vivgrid

- **registry 现状**：profile=`full()` · base_url=`https://api.vivgrid.com/v1` · env=`VIVGRID_API_KEY`
- **变体**：无

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异（OpenAI `/chat/completions` 端点） | - | C | https://vivgrid.com/docs/quick-start |
| 思考机制 | 无差异（透传上游） | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer，`VIVGRID_API_KEY` 与 registry 一致） | - | C | https://mastra.ai/models/providers/vivgrid |
| URL/端点 | 无差异 | - | C | https://vivgrid.com/blog/decoupling-prompts-tools-models-from-agent-client |
| 模型 ID | **org 前缀**：`vivgrid/deepseek-v3.2`、`vivgrid/gemini-3.1-pro-preview`、`vivgrid/gpt-5.6-sol` 等 | - | C | https://mastra.ai/models/providers/vivgrid |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：[openai_compat_registry.rs](aimux-providers/src/openai_compat_registry.rs#L2113)
- **差距说明**：无。
- **建议动作**：无需动作。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证(仅文档引用)
- **存疑标记**：无

### volc_engine — VolcEngine (火山方舟)

- **registry 现状**：profile=`full()` · base_url=`https://ark.cn-beijing.volces.com` · env=`ARK_API_KEY`
- **变体**：无

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异（OpenAI 兼容 /api/v3；支持工具调用） | - | C | https://www.volcengine.com/docs/82379/1298459 |
| 思考机制 | 推理模型（豆包/DeepSeek 系）无统一开关字段证据（按模型走 enable_thinking 或 thinking 需实测）⚠️ | - | - | - |
| 流式/usage | 无差异（标准） | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | **官方推荐 IAM（AccessKey/SecretKey）签名鉴权**；OpenAI 兼容面也可 API Key Bearer（ARK_API_KEY） | - | B/C | https://www.volcengine.com/docs/82379/1263279 ；[simple-one-api 接入指南](reference/simple-one-api/docs/火山方舟大模型接入指南.md#L7) |
| URL/端点 | **registry 缺 `/api/v3` 路径**：官方 OpenAI 兼容 base 为 `https://ark.cn-beijing.volces.com/api/v3` | `POST https://ark.cn-beijing.volces.com/api/v3/chat/completions` | B | [simple-one-api/docs/火山方舟大模型接入指南.md](reference/simple-one-api/docs/火山方舟大模型接入指南.md#L24) ；[uni-api/README.md](reference/uni-api/README.md#L233) |
| 模型 ID | **推理接入点 ID**：`ep-xxxxxxxxxx-yyyy`（endpoint 格式）或 `doubao-*` 模型名 | `{"model":"ep-20240612090709-hzjz5","messages":[...]}` | B | [simple-one-api/docs/火山方舟大模型接入指南.md](reference/simple-one-api/docs/火山方舟大模型接入指南.md#L18) |

#### 2. aimux 现状对比

- **对比结论**：⚠️ 不一致（base_url 缺 `/api/v3`）
- **aimux 代码位置**：[openai_compat_registry.rs](aimux-providers/src/openai_compat_registry.rs#L2122)；端点拼接 [model.rs](aimux-providers/src/openai/model.rs#L73)
- **差距说明**：当前拼接结果为 `https://ark.cn-beijing.volces.com/chat/completions`，缺少 `/api/v3` 路径段，必 404；Bearer 认证可用（免 IAM 签名，官方兼容面接受 API Key）。
- **建议动作**：registry base_url 改为 `https://ark.cn-beijing.volces.com/api/v3`（高优先）。

#### 3. 证据与验证

- **证据等级**：B（本地 reference 配置）+C
- **验证状态**：🔲 未验证(仅文档引用)
- **存疑标记**：无（base_url 结论强）

### vultr — Vultr (Inference)

- **registry 现状**：profile=`full()` · base_url=`https://api.vultrinference.com/v1` · env=`VULTR_API_KEY`
- **变体**：无

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（`max_tokens` 默认 4096；`stop`、`seed` 标准） | - | C | https://api.vultrinference.com/ |
| 能力支持 | 无差异（logprobs/top_logprobs/tools/tool_choice 标准） | - | C | https://api.vultrinference.com/ |
| 思考机制 | 无差异（raw 端点透传上游 `reasoning_content`；**normalize 模式可改写为 `reasoning`**，见下） | - | C | https://api.vultrinference.com/ |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | raw 端点可能返回非标准字段（content=None + tool_calls 等），normalize 模式修正 | - | C | https://api.vultrinference.com/ |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer *API Key*） | - | C | https://api.vultrinference.com/ |
| URL/端点 | 无差异 | - | C | https://api.vultrinference.com/ |
| 模型 ID | **org 前缀 + `-normalize` 后缀约定**：`deepseek-ai/DeepSeek-V4-Pro` / `deepseek-ai/DeepSeek-V4-Pro-normalize`（normalizer 修正 `reasoning_content`→`reasoning`、工具调用 ID 等非标准响应） | `{"model":"deepseek-ai/DeepSeek-V4-Pro-normalize","messages":[...]}` | C | https://api.vultrinference.com/ |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖
- **aimux 代码位置**：[openai_compat_registry.rs](aimux-providers/src/openai_compat_registry.rs#L2131)
- **差距说明**：aimux 已兼容 `reasoning_content`（[model.rs](aimux-providers/src/openai/model.rs#L558) 优先读它），与 raw 端点兼容良好；`-normalize` 只是用户可选模型名后缀，无需代码改动；但需在文档提示 deepseek 系模型加 `-normalize` 以获得标准响应。
- **建议动作**：补文档说明，无需 profile 改动。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证(仅文档引用)
- **存疑标记**：无

### wafer — Wafer

- **registry 现状**：profile=`full()` · base_url=`https://api.wafer.ai/v1` · env=`WAFER_API_KEY`
- **变体**：无

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异（OpenAI 兼容 `/chat/completions`） | - | C | https://www.truefoundry.com/docs/ai-gateway/wafer |
| 思考机制 | 无差异（透传上游；模型表带 Reasoning 列） | - | C | https://mastra.ai/models/providers/wafer.ai |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer，`WAFER_API_KEY` 与 registry 一致） | - | C | https://mastra.ai/models/providers/wafer.ai |
| URL/端点 | **registry 域名存疑**：TrueFoundry/Mastra 官方 base 为 `https://pass.wafer.ai/v1`（Wafer Pass 产品）；registry 写 `api.wafer.ai` ⚠️ | `POST https://pass.wafer.ai/v1/chat/completions` | C(⚠️) | https://www.truefoundry.com/docs/ai-gateway/wafer ；https://mastra.ai/models/providers/wafer.ai |
| 模型 ID | **org 前缀**：`wafer.ai/GLM-5.1`、`wafer.ai/Kimi-K2.6`、`wafer.ai/MiniMax-M3` | - | C | https://mastra.ai/models/providers/wafer.ai |

#### 2. aimux 现状对比

- **对比结论**：⚠️ 不一致（base_url 域名待核）
- **aimux 代码位置**：[openai_compat_registry.rs](aimux-providers/src/openai_compat_registry.rs#L2140)
- **差距说明**：`api.wafer.ai` vs 官方 `pass.wafer.ai`；需实测 api.wafer.ai 是否解析/转发。
- **建议动作**：实测后修正 registry base_url。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证(仅文档引用)
- **存疑标记**：base_url 域名 ⚠️

### wandb — Weights & Biases (Serverless Inference)

- **registry 现状**：profile=`full()` · base_url=`https://api.inference.wandb.ai/v1` · env=`WANDB_API_KEY`
- **变体**：无

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异（标准 chat/completions，模型清单由 W&B 托管） | - | C | https://docs.wandb.ai/inference/api-reference/chat-completions |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer）；**可选 `OpenAI-Project: [YOUR-TEAM]/[YOUR-PROJECT]` 头**做 usage 追踪 | `curl .../chat/completions -H "Authorization: Bearer [YOUR-API-KEY]" -H "OpenAI-Project: [YOUR-TEAM]/[YOUR-PROJECT]"` | C | https://docs.wandb.ai/inference/api-reference/chat-completions |
| URL/端点 | 无差异 | - | C | https://docs.wandb.ai/inference/api-reference/chat-completions |
| 模型 ID | **org 前缀**：`meta-llama/Llama-3.1-8B-Instruct` 等托管模型 | - | C | https://docs.wandb.ai/inference/api-reference/chat-completions |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：[openai_compat_registry.rs](aimux-providers/src/openai_compat_registry.rs#L2149)；`OpenAI-Project` 头已有 [mod.rs](aimux-providers/src/openai/mod.rs#L101)（`with_project`）
- **差距说明**：`OpenAI-Project` 头需要用户显式 `with_project("team/project")` 配置，默认不发送——符合可选语义。
- **建议动作**：厂商文档提示设置 project 头以启用 usage 追踪。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证(仅文档引用)
- **存疑标记**：无

### xiaomi_token_plan_ams — Xiaomi Token Plan（欧洲区）

- **registry 现状**：profile=`full()` · base_url=`https://token-plan-ams.xiaomimimo.com/v1` · env=`MIMO_API_KEY`
- **变体**：无（**并入主条目 xiaomi_token_plan_cn**——同一 Token Plan 产品，区域镜像域名 `token-plan-ams.xiaomimimo.com`）

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（并入主条目，`max_completion_tokens`） | - | C | https://mimo.mi.com/docs/en-US/quick-start/summary/first-api-call |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 无差异（同主条目，无独立开关文档） | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | **`api-key` 头**（同主条目，Key 格式 `tp-xxxxx`） | - | C | https://mimo.mi.com/docs/en-US/quick-start/summary/first-api-call |
| URL/端点 | 无差异（区域子域） | - | C | https://mimo.mi.com/docs/en-US/quick-start/summary/first-api-call |
| 模型 ID | 同主条目（`mimo-v2.5-pro` 等） | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖（同主条目：`api-key` 头需 with_headers）
- **aimux 代码位置**：[openai_compat_registry.rs](aimux-providers/src/openai_compat_registry.rs#L2158)
- **差距说明**：无独立差距。
- **建议动作**：与主条目一并处理认证头问题。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证(仅文档引用)
- **存疑标记**：区域域名是否真实开通未实测

### xiaomi_token_plan_sgp — Xiaomi Token Plan（新加坡区）

- **registry 现状**：profile=`full()` · base_url=`https://token-plan-sgp.xiaomimimo.com/v1` · env=`MIMO_API_KEY`
- **变体**：无（**并入主条目 xiaomi_token_plan_cn**，区域镜像 `token-plan-sgp.xiaomimimo.com`）

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（并入主条目） | - | C | https://mimo.mi.com/docs/en-US/quick-start/summary/first-api-call |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | **`api-key` 头**（同主条目） | - | C | https://mimo.mi.com/docs/en-US/quick-start/summary/first-api-call |
| URL/端点 | 无差异（区域子域） | - | C | https://mimo.mi.com/docs/en-US/quick-start/summary/first-api-call |
| 模型 ID | 同主条目 | - | - | - |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖（同主条目）
- **aimux 代码位置**：[openai_compat_registry.rs](aimux-providers/src/openai_compat_registry.rs#L2176)
- **差距说明**：无独立差距。
- **建议动作**：与主条目一并处理认证头问题。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证(仅文档引用)
- **存疑标记**：区域域名是否真实开通未实测

### xiaomi_token_plan_cn — Xiaomi Token Plan（中国区）

- **registry 现状**：profile=`full()` · base_url=`https://token-plan-cn.xiaomimimo.com/v1` · env=`MIMO_API_KEY`
- **变体**：xiaomi_token_plan_ams（`token-plan-ams.xiaomimimo.com/v1`）/ xiaomi_token_plan_sgp（`token-plan-sgp.xiaomimimo.com/v1`），区域镜像同协议

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（`max_completion_tokens` 官方示例） | - | C | https://mimo.mi.com/docs/en-US/quick-start/summary/first-api-call |
| 能力支持 | 无差异（temperature/top_p/frequency_penalty/presence_penalty/stop/stream 标准） | - | C | https://mimo.mi.com/docs/en-US/quick-start/summary/first-api-call |
| 思考机制 | MiMo 推理模型无独立开关字段证据（reasoning 模型按权重）⚠️ | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | **认证头是 `api-key`（非 `Authorization: Bearer`）**：官方 curl 用 `--header "api-key: $MIMO_API_KEY"`；Key 格式 `tp-xxxxx`（Token Plan） | `curl ... 'https://token-plan-cn.xiaomimimo.com/v1/chat/completions' --header "api-key: $MIMO_API_KEY"` | C | https://mimo.mi.com/docs/en-US/quick-start/summary/first-api-call |
| URL/端点 | 无差异（cn 区与官方一致；另提供 Anthropic 面 `/anthropic`） | - | C | https://mimo.mi.com/docs/en-US/quick-start/summary/first-api-call |
| 模型 ID | `mimo-v2.5-pro`、`mimo-v2.5` 等 `mimo-*` 代号 | - | C | https://mimo.mi.com/docs/en-US/quick-start/summary/first-api-call |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖
- **aimux 代码位置**：[openai_compat_registry.rs](aimux-providers/src/openai_compat_registry.rs#L2158)
- **差距说明**：aimux 默认发送 `Authorization: Bearer`；若 MiMo 拒绝 Bearer 只认 `api-key`，则请求 401——需用户 `with_headers` 注入 `api-key` 头（[mod.rs](aimux-providers/src/openai/mod.rs#L155) 支持自定义 headers，可兜底但非零配置）。
- **建议动作**：实测 `api-key` vs `Authorization` 兼容性；若不兼容，考虑在 profile 增加 auth 头选项或在厂商文档注明 with_headers 用法。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证(仅文档引用)
- **存疑标记**：是否同时接受 Bearer 未确认（官方只示范 `api-key`）

### xiaomimimo — Xiaomi MiMo（按量计费）

- **registry 现状**：profile=`full()` · base_url=`https://mimo.xiaomi.com/v1` · env=`XIAOMI_API_KEY`
- **变体**：无（与 xiaomi_token_plan_* 是同一平台两种计费方式）

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（`max_completion_tokens`） | - | C | https://mimo.mi.com/docs/en-US/quick-start/summary/first-api-call |
| 能力支持 | 无差异 | - | C | https://mimo.mi.com/docs/en-US/quick-start/summary/first-api-call |
| 思考机制 | 无差异（无独立开关文档） | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | **认证头 `api-key`**（同 Token Plan）；Key 格式 `sk-xxxxx` | - | C | https://mimo.mi.com/docs/en-US/quick-start/summary/first-api-call |
| URL/端点 | **registry 域名存疑**：官方按量计费 base 为 `https://api.xiaomimimo.com/v1`（registry 写 `mimo.xiaomi.com/v1`，后者是官网站）⚠️ | `POST https://api.xiaomimimo.com/v1/chat/completions` | C(⚠️) | https://mimo.mi.com/docs/en-US/quick-start/summary/first-api-call |
| 模型 ID | `mimo-v2.5-pro` 等（同 Token Plan） | - | C | https://mimo.mi.com/docs/en-US/quick-start/summary/first-api-call |

#### 2. aimux 现状对比

- **对比结论**：⚠️ 不一致（base_url + 认证头）
- **aimux 代码位置**：[openai_compat_registry.rs](aimux-providers/src/openai_compat_registry.rs#L2185)
- **差距说明**：base_url 需按官方改为 `api.xiaomimimo.com/v1`（或实测 mimo.xiaomi.com 是否通 API）；认证头 `api-key` 需 with_headers。
- **建议动作**：修正 base_url；认证头问题与 xiaomi_token_plan_cn 一并处理。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证(仅文档引用)
- **存疑标记**：base_url 域名 ⚠️

### xpersona — Xpersona

- **registry 现状**：profile=`full()` · base_url=`/v1` · env=`XPERSONA_API_KEY`
- **变体**：无

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异（OpenAI 兼容 `/chat/completions`） | - | C | https://mastra.ai/models/providers/xpersona |
| 思考机制 | 无差异 | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer） | - | C | https://www.xpersona.co/ |
| URL/端点 | **registry base_url 只有 `/v1` 相对路径（缺 host）**：官方为 `https://www.xpersona.co/v1`（Mastra 配置） | `POST https://www.xpersona.co/v1/chat/completions` | C(⚠️) | https://mastra.ai/models/providers/xpersona |
| 模型 ID | **org 前缀**：`xpersona/claude-fable-5` 等（聚合 GPT/Claude/Gemini） | - | C | https://mastra.ai/models/providers/xpersona |

#### 2. aimux 现状对比

- **对比结论**：⚠️ 不一致（base_url 缺 host）
- **aimux 代码位置**：[openai_compat_registry.rs](aimux-providers/src/openai_compat_registry.rs#L2194)
- **差距说明**：`/v1` 相对路径无法发起请求，需补全 host。
- **建议动作**：registry base_url 改为 `https://www.xpersona.co/v1`（或官方 API 域名，以实测为准）。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证(仅文档引用)
- **存疑标记**：官方 API 域名（www vs 裸域）⚠️

### xunfei — Xunfei（讯飞星火）

- **registry 现状**：profile=`full()` · base_url=`https://spark-api-open.xf-yun.com/v1` · env=`XUNFEI_API_PASSWORD`
- **变体**：无

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（`max_tokens`、`user`、`presence_penalty`、`frequency_penalty` 标准；temperature 建议 1.2） | - | C | https://www.xfyun.cn/doc/spark/HTTP%E8%B0%83%E7%94%A8%E6%96%87%E6%A1%A3.html |
| 能力支持 | **支持 `top_k`**；`response_format:{"type":"json_object"}`；tools 支持 function 与 **`web_search` 类型内置工具** | `{"tools":[{"type":"function","function":{...}},{"type":"web_search","web_search":{"enable":true,"show_ref_label":true,"search_mode":"deep"}}]}` | C | https://www.xfyun.cn/doc/spark/HTTP%E8%B0%83%E7%94%A8%E6%96%87%E6%A1%A3.html |
| 思考机制 | Ultra 已升级 X1.5 快思考；X1/X2 深度思考模型存在（另册 X1 HTTP 文档），OpenAI 兼容面无统一 thinking 开关证据 ⚠️ | - | C(⚠️) | https://www.xfyun.cn/doc/spark/X1http.html |
| 流式/usage | 无差异（stream 标准 SSE） | - | C | https://www.xfyun.cn/doc/spark/HTTP%E8%B0%83%E7%94%A8%E6%96%87%E6%A1%A3.html |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | **`suppress_plugin` 数组**：抑制内置插件（如 knowledge 搜索插件） | `{"suppress_plugin":["knowledge"]}` | C | https://www.xfyun.cn/doc/spark/HTTP%E8%B0%83%E7%94%A8%E6%96%87%E6%A1%A3.html |
| headers/认证 | 无差异（`Authorization: Bearer <APIPassword>`，env 名与 registry 的 XUNFEI_API_PASSWORD 语义一致） | `--header 'Authorization: Bearer 123456'` | C | https://www.xfyun.cn/doc/spark/HTTP%E8%B0%83%E7%94%A8%E6%96%87%E6%A1%A3.html |
| URL/端点 | 无差异（`/v1/chat/completions` 官方一致） | - | C | https://www.xfyun.cn/doc/spark/HTTP%E8%B0%83%E7%94%A8%E6%96%87%E6%A1%A3.html |
| 模型 ID | **版本码约定**：`4.0Ultra` / `generalv3.5`(Max) / `max-32k` / `generalv3`(Pro) / `pro-128k` / `lite`（非 OpenAI 式命名） | `{"model":"generalv3.5","messages":[...]}` | C | https://www.xfyun.cn/doc/spark/HTTP%E8%B0%83%E7%94%A8%E6%96%87%E6%A1%A3.html |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖
- **aimux 代码位置**：[openai_compat_registry.rs](aimux-providers/src/openai_compat_registry.rs#L2203)
- **差距说明**：`suppress_plugin` 不在 convert.rs 白名单（需 bodyOverrides）；`web_search` 类型 tool 不在 aimux `Tool` 枚举（[convert.rs](aimux-providers/src/openai/convert.rs#L1374) 只处理 Function 类型，Provider 类型被丢弃并 warn）——星火内置搜索工具无法透传；top_k/response_format/tools(function) 均 ✅。
- **建议动作**：`web_search` 工具透传可评估（低优先）；suppress_plugin 文档化走 bodyOverrides。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证(仅文档引用)
- **存疑标记**：X1 思考开关字段未确认

### zai — Z.AI (GLM)

- **registry 现状**：profile=`full()` · base_url=`https://api.z.ai/api/paas/v4` · env=`ZAI_API_KEY`
- **变体**：zai_coding_plan（订阅套餐，见下条）

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（官方 GLM-5 示例用 `max_tokens` + `temperature`） | - | C | https://docs.z.ai/guides/llm/glm-5 |
| 能力支持 | 无差异（Function Call、Structured Output、streaming、上下文缓存） | - | C | https://docs.z.ai/guides/llm/glm-5 |
| 思考机制 | **`thinking` 对象开关**：`{"type":"enabled"}`（GLM-5/4.6V 官方示例）；GLM-5 提供多种 thinking mode 档位；响应含 reasoning 输出 | `{"model":"glm-5","messages":[...],"thinking":{"type":"enabled"},"max_tokens":4096,"temperature":1.0}` | C | https://docs.z.ai/guides/llm/glm-5 ；https://docs.bigmodel.cn/cn/guide/models/vlm/glm-4.6v |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异（多模态走标准 image_url） | - | C | https://docs.bigmodel.cn/cn/guide/models/vlm/glm-4.6v |
| 特殊字段 | 无差异（上下文缓存为服务端行为） | - | - | - |
| headers/认证 | 无差异（Bearer） | - | C | https://docs.z.ai/guides/llm/glm-5 |
| URL/端点 | 无差异（`/api/paas/v4` 与官方一致） | - | C | https://docs.z.ai/guides/llm/glm-5 |
| 模型 ID | `glm-5`、`glm-5.2`、`glm-4.6v` 等 `glm-*` 系列 | - | C | https://docs.z.ai/guides/llm/glm-5 |

#### 2. aimux 现状对比

- **对比结论**：🔶 部分覆盖
- **aimux 代码位置**：[openai_compat_registry.rs](aimux-providers/src/openai_compat_registry.rs#L2212)
- **差距说明**：`thinking` 对象需 bodyOverrides（现有 DeepSeek override 的 thinking 注入绑定 provider_options["deepseek"]，不通用）；其余 ✅。
- **建议动作**：若 GLM 成为主流，建议在 RequestBodyOverride 增加 GLM 变体（thinking 透传 + reasoning_effort 可选）或 profile 增加 `thinking_field` 通用支持。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证(仅文档引用)
- **存疑标记**：无

### zai_coding_plan — Z.AI Coding Plan

- **registry 现状**：profile=`full()` · base_url=`https://api.z.ai/api/anthropic` · env=`ZHIPU_API_KEY`
- **变体**：无

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | **协议级差异**：该端点是 Anthropic Messages 协议，无 `messages.role=system` 顶层数组，改用顶层 `system` + `max_tokens` 必填 | `{"model":"glm-5.2[1m]","max_tokens":... ,"system":"...","messages":[...]}` | C | https://docs.z.ai/devpack/tool/claude |
| 能力支持 | Anthropic 面（tool_use/tool_result、thinking budget 模式） | - | C | https://docs.z.ai/devpack/tool/claude |
| 思考机制 | Claude Code 集成：`ANTHROPIC_AUTH_TOKEN`=ZHIPU_API_KEY，模型 `glm-4.7` / `glm-5.2[1m]`；thinking 走 Anthropic budget 模式 | `"env":{"ANTHROPIC_AUTH_TOKEN":"your_zai_api_key","ANTHROPIC_BASE_URL":"https://api.z.ai/api/anthropic","ANTHROPIC_DEFAULT_OPUS_MODEL":"glm-5.2[1m]"}` | C | https://docs.z.ai/devpack/tool/claude |
| 流式/usage | Anthropic SSE 格式（event: message_start/content_block_delta 等） | - | C | https://docs.z.ai/devpack/tool/claude |
| 消息格式 | Anthropic 消息格式（顶层 system，tool_result 角色） | - | C | https://docs.z.ai/devpack/tool/claude |
| 特殊字段 | Anthropic 专属（`thinking` budget、`metadata` 等） | - | C | https://docs.z.ai/devpack/tool/claude |
| headers/认证 | `x-api-key` + `anthropic-version` 头（非 OpenAI Bearer） | - | C | https://docs.z.ai/devpack/tool/claude |
| URL/端点 | registry base_url 与官方一致（Anthropic 面）；**注意 OpenAI 面是 `https://api.z.ai/api/paas/v4`，非此端点** | `POST https://api.z.ai/api/anthropic/v1/messages` | C | https://docs.z.ai/devpack/tool/claude |
| 模型 ID | `glm-4.7`、`glm-5.2[1m]`（带 `[1m]` 长上下文后缀） | - | C | https://docs.z.ai/devpack/tool/claude |

#### 2. aimux 现状对比

- **对比结论**：⚠️ 不一致（协议不匹配）
- **aimux 代码位置**：[openai_compat_registry.rs](aimux-providers/src/openai_compat_registry.rs#L2221)
- **差距说明**：registry 以 OpenAICompatProfile::full() + Anthropic 端点声明，aimux 的 OpenAI 请求体（顶层 messages + system 作为消息）发往 `/api/anthropic` 会与 Anthropic Messages 协议冲突。要么改用 OpenAI 面 base_url（`https://api.z.ai/api/paas/v4`），要么用 anthropic 协议实现。
- **建议动作**：与维护者确认意图：若走 OpenAI 协议，base_url 改 `https://api.z.ai/api/paas/v4`（高优先）；若保留 Anthropic 端点，需单独的 Anthropic 适配（超出 OpenAICompat 范围）。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证(仅文档引用)
- **存疑标记**：无（协议差异证据充分）

### zeldoc — Zeldoc

- **registry 现状**：profile=`full()` · base_url=`https://api.zeldoc.ai/v1` · env=`ZELDOC_API_KEY`
- **变体**：无

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异（@ai-sdk/openai-compatible，单模型） | - | C | https://models.dev/providers/zeldoc |
| 思考机制 | 无差异（无证据） | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（`ZELDOC_API_KEY` 与 registry 一致） | - | C | https://models.dev/providers/zeldoc |
| URL/端点 | 无差异 | - | C | https://models.dev/providers/zeldoc |
| 模型 ID | 单模型（models.dev 表未展示 ID，证据不足）⚠️ | - | C(⚠️) | https://models.dev/providers/zeldoc |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：[openai_compat_registry.rs](aimux-providers/src/openai_compat_registry.rs#L2230)
- **差距说明**：无。
- **建议动作**：无需动作。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证(仅文档引用)
- **存疑标记**：模型 ID 细节证据不足

### zenmux — ZenMux

- **registry 现状**：profile=`full()` · base_url=`https://zenmux.ai/api/v1` · env=`ZENMUX_API_KEY`
- **变体**：无

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异（OpenAI Chat Completions 全兼容） | - | C | https://zenmux.ai/docs/guide/quickstart.html |
| 思考机制 | 无差异（透传上游） | - | - | - |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer） | - | C | https://zenmux.ai/docs/guide/quickstart.html |
| URL/端点 | 无差异（同时提供 Responses `/api/v1`、Anthropic `/api/anthropic`、Gemini `/api/vertex-ai` 面，协议互通） | - | C | https://zenmux.ai/docs/guide/quickstart.html |
| 模型 ID | 平台模型 slug（如 `google/gemini-3.1-pro-preview` 格式，按 Models 页） | - | C | https://zenmux.ai/docs/guide/quickstart.html |

#### 2. aimux 现状对比

- **对比结论**：✅ 已覆盖
- **aimux 代码位置**：[openai_compat_registry.rs](aimux-providers/src/openai_compat_registry.rs#L2239)
- **差距说明**：无。
- **建议动作**：无需动作。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证(仅文档引用)
- **存疑标记**：无

### zhipu_v4 — ZhipuV4 (GLM 开放平台)

- **registry 现状**：profile=`full()` · base_url=`https://open.bigmodel.cn` · env=`ZHIPU_API_KEY`
- **变体**：无（zhipuai_coding_plan 为订阅套餐，见下条）

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异 | - | - | - |
| 能力支持 | 无差异（Function Calling、流式、视觉多模态 image_url） | - | C | https://docs.bigmodel.cn/cn/guide/models/vlm/glm-4.6v |
| 思考机制 | **`thinking` 对象开关**（同 z.ai）：`{"type":"enabled"}`；深度思考可关 | `{"model":"glm-4.6v","messages":[{"role":"user","content":[{"type":"image_url",...},{"type":"text","text":"..."}]}],"thinking":{"type":"enabled"}}` | C | https://docs.bigmodel.cn/cn/guide/models/vlm/glm-4.6v |
| 流式/usage | 无差异（stream 标准；深度思考输出经 reasoning 通道） | - | C | https://docs.bigmodel.cn/cn/guide/models/vlm/glm-4.6v |
| 消息格式 | 无差异（多模态标准 image_url；支持视频/文件） | - | C | https://docs.bigmodel.cn/cn/guide/models/vlm/glm-4.6v |
| 特殊字段 | 无差异（上下文缓存服务端处理） | - | - | - |
| headers/认证 | 无差异（Bearer） | - | C | https://docs.bigmodel.cn/cn/guide/models/vlm/glm-4.6v |
| URL/端点 | **registry 缺路径**：官方 OpenAI 兼容 base 为 `https://open.bigmodel.cn/api/paas/v4`（registry 只写 host） | `POST https://open.bigmodel.cn/api/paas/v4/chat/completions` | C | https://docs.bigmodel.cn/cn/guide/models/vlm/glm-4.6v |
| 模型 ID | `glm-4.6v`、`glm-4.6`、`glm-4.6v-flash` 等 `glm-*` | - | C | https://docs.bigmodel.cn/cn/guide/models/vlm/glm-4.6v |

#### 2. aimux 现状对比

- **对比结论**：⚠️ 不一致（base_url 缺路径）
- **aimux 代码位置**：[openai_compat_registry.rs](aimux-providers/src/openai_compat_registry.rs#L2248)
- **差距说明**：当前拼接 `https://open.bigmodel.cn/chat/completions` 缺 `/api/paas/v4`，必 404（高优先修正）；`thinking` 字段需 bodyOverrides。
- **建议动作**：registry base_url 改为 `https://open.bigmodel.cn/api/paas/v4`；thinking 覆盖同 zai 条目。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证(仅文档引用)
- **存疑标记**：无

### zhipuai_coding_plan — Zhipu AI Coding Plan

- **registry 现状**：profile=`full()` · base_url=`https://docs.bigmodel.cn/cn/coding-plan/quick-start` · env=`ZHIPU_API_KEY`
- **变体**：无

#### 1. request 差异发现

| 类别 | 差异 | 例子(请求/响应体片段) | 证据等级 | 来源 |
|------|------|----------------------|---------|------|
| 参数命名 | 无差异（OpenAI 面标准；Anthropic 面用顶层 system/max_tokens） | - | C | https://docs.bigmodel.cn/cn/coding-plan/quick-start |
| 能力支持 | 无差异 | - | - | - |
| 思考机制 | 套餐内 GLM 模型沿用 `thinking` 对象（同 zhipu_v4），官方未在 Coding Plan 页单列 ⚠️ | `{"model":"glm-4.6","messages":[...],"thinking":{"type":"enabled"}}`（推断） | C(⚠️) | https://docs.bigmodel.cn/cn/coding-plan/quick-start |
| 流式/usage | 无差异 | - | - | - |
| 消息格式 | 无差异 | - | - | - |
| 特殊字段 | 无差异 | - | - | - |
| headers/认证 | 无差异（Bearer；**团队版套餐 Key 与开放平台其他 Key 不通用**） | - | C | https://docs.bigmodel.cn/cn/coding-plan/quick-start |
| URL/端点 | **registry base_url 是 docs 页面**：真实 OpenAI 面为 `https://open.bigmodel.cn/api/coding/paas/v4`，Anthropic 面 `https://open.bigmodel.cn/api/anthropic` | `POST https://open.bigmodel.cn/api/coding/paas/v4/chat/completions` | C | https://docs.bigmodel.cn/cn/coding-plan/quick-start |
| 模型 ID | `glm-4.6`、`glm-4.7`、`glm-5.x` 等套餐内模型 | - | C | https://docs.bigmodel.cn/cn/coding-plan/quick-start |

#### 2. aimux 现状对比

- **对比结论**：⚠️ 不一致（base_url 错误）
- **aimux 代码位置**：[openai_compat_registry.rs](aimux-providers/src/openai_compat_registry.rs#L2257)
- **差距说明**：registry 的 base_url 是文档页面 URL，非 API 端点；应改为 `https://open.bigmodel.cn/api/coding/paas/v4`（Coding Plan 专用 OpenAI 面，非普通 /api/paas/v4）。
- **建议动作**：修正 base_url（高优先）；思考覆盖同 zhipu_v4。

#### 3. 证据与验证

- **证据等级**：C
- **验证状态**：🔲 未验证(仅文档引用)
- **存疑标记**：思考机制推断 ⚠️
