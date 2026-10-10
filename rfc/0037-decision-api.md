# RFC-0037: 稳定的 typed Decision API

> **Status**: TypeSafe 官方字段、全语言绑定、能力查询与精度规则已实现
> **Date**: 2026-10-06
> **Scope**: Core、TypeSafe 官方 Jev、Rust / C ABI / Node / Python / Go / Java / Kotlin / Swift / Flutter
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
数组；instructions、description 和 Score levels 每项均为字符串、对象或数组。
Boolean criteria 可包含 true/false 描述；结构化字段原样发送并保留在 Score
答案的 levels 中，不转换成字符串。

answers 使用 question ID map。Boolean probability_true 始终表示 P(true)，
不隐式应用 0.5 阈值。Choice selected 必须是候选 label；可选 probabilities
以 label 为键。Score expected_value 是零起点的浮点位置；可选 probabilities
以 levels 的顺序排列。confidence 保留 provider 数值，不跨服务解释。

Capability 声明题型支持、数量限制、完整分布与 probability_source。
当前来源包括 native、logit_scoring、model_estimate；这些名称不是校准保证。
返回模型版本、usage/raw、latency、provider metadata 和 HTTP response/raw body。

请求重复 ID/label、空 instructions/criteria、本地能力限制会在 HTTP 前失败。
缺题、多题、题型/label/levels 不符、非有限或超范围概率、错误分布键会使整次调用失败。
舍入精度由 provider 的 `DecisionCapabilities.rounding` 声明，并写入
`DecisionResult.rounding`；结果精度必须匹配声明，不能通过响应自行放宽。
`probability_decimals` 与 `score_decimals` 独立配置，支持 0–15 位，None 表示
仅容忍浮点误差。TypeSafe Jev 声明两者为 2；不归一化或改写原始数字。

概率项误差上界 `ep = 0.5 * 10^(-probability_decimals)`，Score 误差上界
`es = 0.5 * 10^(-score_decimals)`；无舍入声明时各自为 0。分布和容差为
`ep * n`，Score 中心化残差 `sum((i - score) * p[i])` 的容差为
`es + ep * sum(abs(i - score))`，均另加浮点运算余量。精确、高精度和
两位小数 provider 分别采用其声明，不再使用统一两位小数规则。

## 第一阶段 Jev adapter

默认 POST `https://api.typesafe.ai/v1/systemone`，Bearer key；环境变量
`TYPESAFE_API_KEY`，与 TypeSafe 官方 SDK 一致。模型 ID 显式传入，示例使用
官方推荐的 `jev-latest`。Jev 是 TypeSafe 的模型，只适配官方服务。

Boolean 映射为 noul；Choice 数组转换成 criteria 对象，省略 description
时使用官方允许的 null；Score levels 转换成有序 criteria 数组。响应 legend
与 probabilities 的索引必须连续且匹配请求等级，保留分数的小数部分。

官方 Choice 支持 1–255 项，Score 支持 2–10 级。state 为字符串、对象或数组；
不施加未在官方 contract 中公布的题目数量、ID 格式或文字长度上限。
依据：[API reference](https://docs.typesafe.ai/api)、
[SDK 常量](https://docs.typesafe.ai/sdk/python/api/constants)。

官方结构化 instructions、Choice 描述、Score 等级和 Noul true/false criteria
均已暴露。Score legend 使用同一描述类型，保留对象/数组。

响应 `model` 保留实际模型版本，usage 保留输入/输出 token 和 raw 数据。
官方未定义独立的 model_version 和 latency_ms 字段，这两个可选结果字段留空。
Choice/Score 的 confidence、分布及 usage 按官方必需字段校验。
usage 对象内的 input_tokens/output_tokens 可缺省或为 null，映射为未知数量。
Jev Choice 按官方定义选取最大概率项，允许并列最高；该规则在 adapter 校验。
依据：[官方 SDK 响应定义](https://docs.typesafe.ai/sdk/python/api/types/responses)。

HTTP 错误使用既有 ApiCallError，保留 code、raw body、headers 与请求上下文。
官方 429 限流和 529 过载复用 Core 的临时错误重试与 Retry-After 处理。
不包含第三方服务的计费、幂等冲突或取消状态专用规则。

JevConfig.endpoint 可覆盖完整 URL，用于显式配置的代理或本地 contract 测试；
不提供其他服务的专用 adapter 或生成式 fallback。

## 调用入口

- Rust：JevProvider::decision_model + core decide。
- Node：jevDecision + typed decide(model, options, signal?)；factory 的可选第四个 probabilitySource 参数声明来源；raw model.decide 接受 JSON。
- Python：jev_decision + decide(model, state, questions, **options)；factory 的可选 probability_source 参数声明来源；native 调用释放 GIL。
- C：aimux_jev_decision_new、aimux_jev_decision_new_with_probability_source、aimux_decide、aimux_decide_with_abort、aimux_decision_capabilities；采用现有 handle/error/string 生命周期。
- Go：NewJevDecision + Decide / DecideContext / Capabilities；支持 context 取消。
- Java / Kotlin / Swift / Flutter：DecisionModel.jev + decide + capabilities，复用同一 C ABI。
- Node/Python 提供 decisionCapabilities / decision_capabilities，raw model.capabilities 返回 JSON。所有能力查询均无 HTTP。

各语言包装及示例见 [Decision API](../docs/api/decision.md)。

## 验证与后续

Jev 使用官方示例的离线 contract fixture，明确标注不是实测录制。
测试覆盖三种 wire 映射、浮点 score、raw metadata、非法请求、坏响应与
provider retry 分类；core 覆盖 optional distributions、abort、timeout；C 覆盖
handle 类型、销毁、JSON 错误和 abort。

验证：Rust 工作区 3871 passed / 57 ignored；Node 的 decision、wrapper、
error 三组测试共 28 passed；Python decision 测试 12 passed。
Usage/Choice 修复另通过 Jev 与录制回放测试共 23 个。工作区 fmt、
Clippy（warnings as errors）、TypeScript 编译与类型生成一致性检查通过。
跨语言测试使用实际构建的 native 模块与本地 HTTP server，未调用线上 Jev。
Node 测试同时发现并修复共享 AbortBridge 对已取消 signal 的处理，覆盖
调用前取消与在途取消。

共享 `decision-native.json` fixture 覆盖各宿主语言的结构化字段、能力查询和
原始等级保留；并验证 handle 关闭、无 HTTP 能力查询及 Go context 取消。

第一优先级为 TypeSafe 官方 Jev 接入。本 PR 直接校正现有 System One
adapter，不另增第三方 provider。当前接入以官方文档 fixture 和本地 HTTP
server 验证；另于 2026-10-06 使用 Python binding 对官方 endpoint 完成
两次线上调用（普通题目、原生对象/数组描述），均返回 `jev-1.13.0`。
三种题型、原生等级保留、usage、概率分布与 Score 精度校验均通过。
随后使用统一 recorder 重新录制这两种请求，实际 wire 交换和规范化结果
保存在 `aimux-providers/tests/fixtures/jev_systemone_live.jsonl`，不含凭据。
回归测试通过共享 HTTP replay helper 和 Core decision mock 分别回放；
CI 无需线上 key。

`decide` 复用统一 JSONL/ring recorder，输入标记为 decision（旧记录默认
language model），state/questions 存入 input.options，能力声明存入 input.decisionCapabilities，provider 仅保留身份，
规范化结果存入 outcome.decisionResult。每次 retry 分配独立 attempt，
录制器快照跨 HTTP、取消和收尾保持一致。覆盖成功、错误、超时、取消、
脱敏、无重复占位记录、严格匹配和 recorder 替换。
`aimux-replay --mock` 支持标准 JSONL 的离线 decision 回放；不带 --mock
时通过调用方 registry 配置重建官方 Jev 执行请求回放。

官方字段覆盖、Provider 精度规则、全语言 capability 查询和宿主包装均已实现。
其他官方决策接口
另按其正式 contract 接入；OpenAI Decisions 待取得公开 preview wire schema，
不以 Responses structured output 代替。
