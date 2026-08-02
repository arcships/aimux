# Model Request Config 调研（OpenAI 兼容厂商全网清单）

> **状态**：调研完成（2026-08-01，250/250 家，差距清单见 [_global_table.md](_global_table.md)）
> **目的**：按 [RFC-0017 阶段 3](../../rfc/0017-provider-config-dx.md) 统计 aimux registry 中全部 OpenAI 兼容厂商的 request 强相关特殊配置，逐项与 aimux 现状对比，产出差距清单，作为 `OpenAICompatProfile` 扩展（reasoningMap 及其他能力字段）的唯一依据。
> **原则**：每条目必须有**例子或证明**（A/B/C 级证据），禁止只有转述；D 级（无出处）不允许，存疑条目单独归档。

## 1. 范围

只收集与 request 构造强相关的配置（9 类）：

| 类别 | 例子 |
|------|------|
| 参数命名差异 | `max_tokens` vs `max_completion_tokens`、`stop` vs `stop_sequences` |
| 能力支持差异 | top_k / tools / tool_choice / response_format / logprobs / parallel_tool_calls / seed / json_mode |
| 思考机制(by-model) | 开关字段/取值/档位映射/是否可关/换代历史（Kimi 三套、Qwen `/no_think`） |
| 流式与 usage 差异 | `stream_options.include_usage`、usage 字段位置（Groq `x_groq`）、SSE 事件格式 |
| 消息/内容格式 | `reasoning_content` 别名、tool_result 别名、多模态输入格式 |
| 特殊请求体字段 | cache 类、`safety_identifier`、`store`、`metadata`、`prediction`、`service_tier` |
| headers / 认证 | `OpenAI-Organization`/`OpenAI-Project`、X-API-Key vs Authorization、SigV4/OAuth/Basic |
| URL / 端点 | 默认 base_url、路径前缀 |
| 模型 ID 约定 | 别名、前缀映射 |

无特殊配置的厂商也要有记录（"无差异"条目），避免调研盲区。

## 2. 数据源分层

| 类别 | 优先数据源 |
|------|-----------|
| 稳定类（命名/usage/tool 格式/base_url/headers） | `reference/` 完整 clone 项目（rig、pydantic-ai、async-openai 等）优先——已实践验证 |
| 换代类（thinking 机制） | 官方文档唯一权威（厂商官网 + 阿里百炼/腾讯云聚合） |
| 盲区/小厂商 | 在线 litellm/one-api/new-api 最新仓库 + 官方文档 |
| 交叉验证 | GitHub issue、社区（gateway 代码 vs 文档对照） |

## 3. 证据分级

| 等级 | 形式 | 说明 |
|------|------|------|
| A | 仓库内可运行的测试/cassette | `aimux-providers/tests/cassettes/...`、wiremock 测试名，最硬 |
| B | reference 项目代码 `file:line` | 可查证 |
| C | 官方文档原文引用（URL + 引用片段） | 可点击验证 |
| D | 仅转述、无出处 | **不允许**，条目标记 ⚠️ 存疑，不参与内置/对比 |

- 思考机制/参数命名/特殊字段类条目**必须附请求体示例**，否则视为无证明
- 验证状态：仅文档引用 = 🔲 未验证；有 A 级测试 = ✅ 已验证

## 4. 与 aimux 现状对比

对照：`OpenAICompatProfile` 5 字段（supports_top_k/supports_tools/supports_response_format/stream_usage_key/request_body_override，见 [openai/mod.rs](../../aimux-providers/src/openai/mod.rs)）、convert.rs 白名单、`deep_merge_json`、`bodyOverrides`。

| 对比结论 | 含义 | 后续动作 |
|----------|------|---------|
| ✅ 已覆盖 | aimux 已有相同机制 | 补测试即可 |
| 🔶 部分覆盖 | 字段名/取值不一致 | 记差距清单，评估 convert 调整 |
| ❌ 未覆盖 | aimux 无此机制 | 记差距清单，评估 profile 新字段或 bodyOverrides 兜底 |
| ⚠️ 不一致 | aimux 实现与调研结论冲突 | 优先处理（可能是 bug） |

基线（2026-08-01 提取自 [openai_compat_registry.rs](../../aimux-providers/src/openai_compat_registry.rs)）：250 个声明，仅 `deepseek`（`deepseek()` profile）与 `groq`（`groq()` profile）有特殊 profile，其余 248 家均为 `full()`。

## 5. 分批清单

| 批次 | 文件 | 数量 | 状态 |
|---|---|---|---|
| 01 | [batch-01.md](batch-01.md) | 42 | ✅ 已调研（2026-08-01） |
| 02 | [batch-02.md](batch-02.md) | 42 | ✅ 已调研（2026-08-01） |
| 03 | [batch-03.md](batch-03.md) | 42 | ✅ 已调研（2026-08-01） |
| 04 | [batch-04.md](batch-04.md) | 42 | ✅ 已调研（2026-08-01） |
| 05 | [batch-05.md](batch-05.md) | 42 | ✅ 已调研（2026-08-01） |
| 06 | [batch-06.md](batch-06.md) | 40 | ✅ 已调研（2026-08-01） |

## 6. 输出物

- 本目录 batch-XX.md — 全量条目（厂商/差异/例子/证据/来源/aimux 现状）
- `_research-input.tsv` — 250 家 registry 现状基线（name/display/base_url/env_var/profile）
- `_global_table.md` — 全局汇总（差异类别分布 + 差距清单）
- 差距清单 → `OpenAICompatProfile` 扩展需求（新字段如 `max_tokens_key`、`usage_key` 等，由调研结果驱动）

## 7. 中间产物

- `_research-input.tsv`：250 家 registry 基线（由 `openai_compat_registry.rs` 提取）
- `_template.md`：条目模板
- `_global_table.md`：全局汇总表
