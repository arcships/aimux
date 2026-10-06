# RFC-0037: 稳定的 typed Decision API

> **Status**: 第一阶段已实现；后续 adapter 和绑定仍待开发
> **Date**: 2026-10-06
> **Scope**: Core、Jev System One、Rust / C ABI / Node / Python
> **Research**: [调查报告](../docs/research/decision-api/report.zh-CN.md)、[接口材料](../docs/research/decision-api/interface-materials.zh-CN.md)

## Contract

公开操作为 `decide`，不使用 experimental 前缀。遵循现有架构：
`decide(model, DecisionCallOptions)` 负责请求与答案校验、operation retry、
timeout 和 abort；`DecisionModel::do_decide` 执行一次 provider attempt。
`Provider::decision_model(model_id)` 默认返回 UnsupportedFunctionality。

支持范围限于原生决策接口和候选评分能力。普通生成 API 的 structured output
不作为 decision adapter，也不提供生成答案或概率的模拟 fallback。
AI SDK 的此类适配仅作为调研背景，不属于 aimux 开发范围。

state 是文本或 JSON Value；它不隐式解释媒体对象。questions 是带 ID 的有序数组，
支持 Boolean、Choice、Score。Choice options 是 label / optional description
数组；Score levels 是从低到高的 label 数组。第一阶段不引入 Boolean 的 true/false
说明或 Score 的额外 description，避免在没有确定 wire 字段时静默丢弃信息。

answers 使用 question ID map。Boolean probability_true 始终表示 P(true)，
不隐式应用 0.5 阈值。Choice selected 必须是候选 label；可选 probabilities
以 label 为键。Score expected_value 是零起点的浮点位置；可选 probabilities
以 levels 的顺序排列。confidence 保留 provider 数值，不跨服务解释。

Capability 声明题型支持、数量限制、完整分布与 probability_source。
当前来源包括 native、logit_scoring、model_estimate；这些名称不是校准保证。
返回模型版本、usage/raw、latency、provider metadata 和 HTTP response/raw body。

请求重复 ID/label、空 instructions/criteria、本地能力限制会在 HTTP 前失败。
缺题、多题、题型/label/levels 不符、非有限或超范围概率、错误分布键会使整次调用失败。
分布允许每项两位小数舍入的累积误差，保留原始数值，不自动归一化。
Score 验证合法范围、等级与分布期望的一致性。概率与分数分别舍入到两位小数
时，每项允许 0.005 的误差；采用中心化残差 `sum((i - score) * p[i])`，
容差为 `0.005 + 0.005 * sum(abs(i - score))`，另加浮点运算余量。这允许
舍入后概率之和略偏离 1，但拒绝分数和分布表达相反判断。
后续 adapter 应按其精度声明扩展校验，当前两位小数规则仍属于首批实现。

## 第一阶段 Jev adapter

默认 POST `https://jev-ai.org/api/v1/systemone/`，Bearer key；环境变量
`JEV_API_KEY`。Boolean 映射为 noul；Choice 数组转换成 criteria 对象，缺省
description 使用 label；Score levels 转换成 criteria 数组。响应 legend 与
probabilities 的索引必须连续且匹配请求等级，保留分数的小数部分。

Jev 托管限制：1–20 题，Choice 2–24 项，Score 2–10 级，ID 为不超过 64
个字符的 ASCII 字母数字/下划线/横线且以字母数字开头。instructions 最多
1000 字符，Choice label 64 字符 / description 400 字符，Score label 400 字符。
这些限制来自 [Jev 文档](https://jev-ai.org/docs/question-types/)，不能使用
TypeSafe 的 Choice 1–255 限制代替。

HTTP 错误使用既有 ApiCallError，保留 code、raw body、headers 与请求上下文。
Jev 409 幂等冲突不重试，499 可以重试，429 daily spend limit 不自动重试。
其他 retry 行为复用现有 operation retry，支持 Retry-After。调用方通过 headers
传 Idempotency-Key；重试复用同一 headers。无自动 key 生成。

JevConfig.endpoint 可指定完整 URL，适用于显式部署的 System One wrapper。
使用 wrapper 时调用方应按其实现设置 probability_source。第一阶段仍使用
Jev 的保守限制；不宣称默认 vLLM/SGLang serve 提供这个路由，不支持 wrapper
特有的 samples、steps、依赖问题或多模态扩展。

## 调用入口

- Rust：JevProvider::decision_model + core decide。
- Node：jevDecision + typed decide(model, options, signal?)；factory 的可选第四个 probabilitySource 参数声明来源；raw model.decide 接受 JSON。
- Python：jev_decision + decide(model, state, questions, **options)；factory 的可选 probability_source 参数声明来源；native 调用释放 GIL。
- C：aimux_jev_decision_new、aimux_jev_decision_new_with_probability_source、aimux_decide、aimux_decide_with_abort；采用现有 handle/error/string 生命周期，原 factory 签名不变。

各语言包装及示例见 [Decision API](../docs/api/decision.md)。

## 验证与后续

Jev 使用官方示例的离线 contract fixture，明确标注不是实测录制。
测试覆盖三种 wire 映射、浮点 score、raw metadata、非法请求、坏响应与
provider retry 分类；core 覆盖 optional distributions、abort、timeout；C 覆盖
handle 类型、销毁、JSON 错误和 abort。

验证：Rust 工作区 3848 passed / 57 ignored；Node 的 decision、wrapper、
error 三组测试共 26 passed；Python decision 测试 6 passed。工作区 fmt、
Clippy（warnings as errors）、TypeScript 编译与类型生成一致性检查通过。
跨语言测试使用实际构建的 native 模块与本地 HTTP server，未调用线上 Jev。
Node 测试同时发现并修复共享 AbortBridge 对已取消 signal 的处理，覆盖
调用前取消与在途取消。

Provider 精度规则与 Node/Python/C 的 capability 查询仍属后续完善范围。

第一优先级是完成 TypeSafe / Jev 的原生 System One 接入。当前实现仅覆盖
`jev-ai.org` 文档 contract，尚不能宣称完成 TypeSafe 托管接口：其 AI SDK
provider 使用 `https://api.typesafe.ai/v1/systemone`、`TYPESAFE_AI_API_KEY`
和 `jev-latest`，字段能力与限制也需逐项核对。优先复用现有 System One
编解码，补齐 endpoint、认证配置、模型、能力限制及 contract 测试；不将
TypeSafe 另列为候选 provider，也不预设需要另写一套 adapter。

完成首要接入后，再扩展 wrapper 专用能力、SGLang scoring adapter，
以及 Go/Java/Kotlin/Swift/Flutter 宿主包装。Provider 精度规则和 capability
查询按首要接入的实际需要推进。OpenAI 自有 Decisions endpoint 待取得
公开 preview wire schema 后实现；不以 Responses structured output 代替。
