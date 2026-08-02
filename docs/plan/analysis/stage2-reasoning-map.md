# Analysis: RFC-0017 阶段 2 — 退役 RequestBodyOverride + 完全用户定义

> 设计来源：[RFC-0017 §2.3/§2.5/§3](../../../rfc/0017-provider-config-dx.md)
> 数据来源：[model-config-research/_global_table.md](../../internal/model-config-research/_global_table.md)（调研差距清单）
> 状态：分析定稿（2026-08-01）——**v3**：否决一切内置厂商映射，完全用户定义

## 0. 设计原则（v3 定稿）

```text
aimux 保留的机制（全部已有）：
  reasoning        → 通用路径透传 reasoning_effort（7 档，厂商自决）
  body_overrides   → 用户定义一切厂商差异（provider 级 + per-call）
  max_tokens_key   → 修 aimux 自身推断 bug（内部数据，非用户概念）
  warning          → reasoning 无映射且未发 effort 时提示（不静默）

aimux 退役的机制：
  RequestBodyOverride 枚举（含 DeepSeek）
  apply_deepseek_override 函数
  reasoning_effort 归一化（xhigh→max/minimal→low）——改为直接透传
```

**理由**：映射是"知识"不是"机制"——知识会过期（DeepSeek V4 官方 effort 仅 low/high/max 三档、xhigh 按模型映射、Kimi 一年三套机制），且用户 bodyOverrides 完全可达；知识放文档（用户手册），机制放代码。

## 1. 目标与范围

### 做

| 项 | 说明 |
|---|---|
| A. 退役 | 删 `RequestBodyOverride`/`request_body_override`/`apply_deepseek_override`/归一化；`deepseek()` profile 回归 full() |
| B. max_tokens_key | 内部字段 + 分支（6 家 `"max_tokens"`，groq/heroku `"max_completion_tokens"`） |
| C. warning | `reasoning` 无映射且未发 effort → warning（不静默） |
| D. 用户手册 | 调研矩阵 → `docs/provider-config-manual.md`（每厂商 bodyOverrides 示例，含 DeepSeek V4 官方机制） |
| E. 测试 | 退役回归 + max_tokens_key + warning |

### 不做

- 任何内置厂商映射（枚举/数据均不保留）
- 任何用户可见新概念
- 档位翻译/归一化（直接透传）

## 2. 设计定稿

### 2.1 退役细节

| 删除 | 替代 |
|---|---|
| `RequestBodyOverride` 枚举（mod.rs ~53-56） | 无——用户 bodyOverrides |
| `request_body_override` profile 字段 | 无 |
| `apply_deepseek_override`（convert.rs ~1485-1552） | 无——thinking 注入由用户配置 |
| `OpenAICompatProfile::deepseek()` 特化 | 回归 `full()` |
| reasoning_effort 归一化（xhigh→max + warning、minimal→low） | 直接透传 7 档（DeepSeek V4 官方接受 xhigh 自行映射——已验证该路线） |

**保留**：`deepseek()` profile 函数本身（返回 full()，薄封装结构不变）、DeepSeek 请求体其他行为（convert 通用路径）。

### 2.2 DeepSeek V4 官方机制（用户手册核心示例,2026-08 核实）

来源:api-docs.deepseek.com/guides/thinking_mode/

| 参数 | 取值 |
|---|---|
| `thinking.type` | `"enabled"`/`"disabled"`(默认 enabled) |
| `reasoning_effort` | `"low"`/`"high"`/`"max"` 三档 |
| effort 映射 | xhigh → flash:high / pro:max(按模型不同);pro 的 low → high(2026-08 初更新) |
| 无效参数 | thinking 模式下 temperature/top_p/penalty 无效(不报错) |
| 默认 | thinking 开启,effort=high |

用户配置示例:

```ts
// 关思考
await generateText(model, p, { bodyOverrides: { thinking: { type: 'disabled' } } })
// 开思考 + xhigh(官方接受,自行映射)
await generateText(model, p, { reasoning: 'xhigh', bodyOverrides: { thinking: { type: 'enabled' } } })
```

### 2.3 max_tokens_key（保留项）

```rust
pub struct OpenAICompatProfile {
    // ... 现有字段 ...
    pub max_tokens_key: Option<&'static str>,   // "max_tokens" | "max_completion_tokens" | None
}
```

convert.rs ~1118-1138 分支改造。内置行:

| 厂商 | 值 | 方向 |
|---|---|---|
| stepfun/siliconflow/sarvam/reka/publicai/perplexity | `"max_tokens"` | 只认 max_tokens（batch-05 C 级） |
| groq/heroku | `"max_completion_tokens"` | max_tokens 弃用/官方要求（batch-03 C 级；heroku 的 allow_ignored_params 另行评估） |

### 2.4 warning

- `reasoning` 设置 + 无任何映射 + 通用路径未发 effort → warning（"未翻译,该厂商需 bodyOverrides"）
- 通用路径已发 effort（OpenAI 有效）→ 不 warning（防误报）
- 厂商认不认 reasoning_effort 无法判断（那是知识）——"无害无效"由用户发现

### 2.5 兼容性承诺

| 层面 | 结论 |
|---|---|
| 用户 API | ✅ 零新增；`body_overrides`/`reasoning` 均已存在 |
| 破坏性 | ⚠️ DeepSeek `reasoning:'none'` 行为变化（不再自动注入 thinking:{type:"disabled"}）——0.x minor bump + 迁移说明（该功能 2026-07-28 才上线,影响面小） |
| 其他厂商 | ✅ 行为不变（它们本来就没有特化） |
| 绑定层 | ✅ 不涉及 |
| max_tokens_key | ✅ 修 bug（groq/heroku 行为变化属修复） |

## 3. 模块分解

| 模块 | 文件 | 任务 |
|---|---|---|
| M1 退役 + max_tokens_key + warning | `openai/mod.rs` + `openai/convert.rs` + `openai_compat_registry.rs`（deepseek/groq 行） | stage2-001 |
| M2 测试 | `tests/reasoning_map_test.rs`（新）+ 现有 deepseek 测试改造 | stage2-002 |
| M3 用户手册 + 收尾 | `docs/provider-config-manual.md`（新）+ 全量回归 + 文档状态 | stage2-003 |

## 4. 集成枚举

| # | 连接 | 验证 | 任务 |
|---|---|---|---|
| I1 | reasoning:'none' + 未发 effort → warning 透出 | warnings 断言 | stage2-001 |
| I2 | reasoning:'none' + 已发 effort（OpenAI）→ 无 warning | 同上（防误报） | stage2-001 |
| I3 | max_tokens_key → 请求体 key 名（推理模型分支） | wiremock 断言 | stage2-001 |
| I4 | 退役后 DeepSeek 请求体不含 thinking 注入（除非用户配置） | wiremock 断言 | stage2-002 |
| I5 | 用户 bodyOverrides 注入 thinking → 请求体含之（阶段 1 能力回归） | wiremock 断言 | stage2-002 |

## 5. 测试验收方案

- [ ] `cargo test --workspace --no-fail-fast` 全绿
- [ ] 退役回归：DeepSeek `reasoning:'none'` 请求体不再含 `thinking`；`reasoning_effort` 直传 7 档（含 xhigh）
- [ ] max_tokens_key：6 家 `"max_tokens"` + groq/heroku `"max_completion_tokens"`，推理/非推理两分支
- [ ] warning：I1/I2 两条路径
- [ ] 用户手册：覆盖调研 §3 全部思考机制厂商（每厂商配置示例 + 来源）
- [ ] 绑定层零改动（git diff 确认）

## 6. 任务与依赖

| 任务 | 内容 | 依赖 |
|---|---|---|
| stage2-001 | 退役 + max_tokens_key + warning（机制层） | — |
| stage2-002 | 测试套件（退役回归/矩阵/max_tokens_key/warning） | 001 |
| stage2-003 | 用户手册 + 全量回归 + 文档状态 | 002 |

## 7. 风险

| 风险 | 级别 | 缓解 |
|---|---|---|
| DeepSeek 退役破坏现有用户 | 中 | 0.x minor bump + 迁移说明；功能上线仅几天 |
| reasoning_effort 直传 xhigh 对不支持的厂商报错 | 低 | 厂商自决（用户哲学）；OpenAI/DeepSeek V4 均官方接受 xhigh |
| 用户手册过期 | 中 | 手册标注核实日期；知识由用户按需更新（比代码内置更容易修正） |
| groq/heroku max_tokens_key 行为变化 | 低 | 属修复（官方文档方向）；测试锁定 |
