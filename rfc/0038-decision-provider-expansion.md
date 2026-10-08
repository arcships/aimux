# RFC-0038：官方 Decision Provider 扩展

状态：实施中。资料核对日期：2026-10-08。
前置：[RFC-0037](0037-decision-api.md)，TypeSafe Jev 已通过 PR #221 合入。

## 目标与边界

在现有 `decide`、`DecisionModel`、统一录制与回放之上接入其他官方决策服务。
支持专用决策模型和真实候选评分；不增加 structured-output 模拟或生成式 fallback。
只适配服务商或开源运行时的官方接口，不为第三方转售网站增加 provider。
“官方”描述接口归属，不表示任意部署版本、任意模型都有相同能力。

## 官方接口清单

以下是上游可用性，不能当作 aimux 已实现清单；本库进度见实施计划。

| 服务／运行时 | 接口与部署 | 实现依据与约束 |
|---|---|---|
| TypeSafe | `/v1/systemone`，Jev | 已适配并实测；继续保留原有 `jev` provider ID |
| OpenAI | `/v1/decisions`，`gpt-6-luna` | 已公开 beta，包含 predicate、choice、score 与逐题 refusal；旧调研的 limited-preview 结论已过时 [1][2] |
| Cloudflare Workers AI | `/client/v4/accounts/{account}/ai/run/@cf/cloudflare/clef`，以及 `clef-flash` | 原生 SystemOne 数据结构，另有 Cloudflare 路由、认证和响应封装 [3] |
| Ollama | `/v1/systemone`，如 Nimble、Tev1 | 0.35 起提供官方接口；具体模型需声明 decision 能力 [4] |
| llama.cpp | `/v1/systemone`，如 Laya、OpenJev | 上游 server 已实现；题目限制、多模态与校准参数依模型而变 [5] |
| LocalAI | `/v1/systemone` | 按部署的 `decisions` usecase 接入；不要将 NER 转换路径无差别视为专用 decision model [6] |
| Laya 作者提供的服务 | `laya-serve` 的 `/v1/systemone` | 作者仓库自带 HTTP server；独立托管网站不属于此目标 [7] |
| vLLM 官方示例 | DiffusionGemma structured server 的 `/v1/systemone` | 可适配明确配置的示例服务；这不是每个 `vllm serve` 实例默认提供的接口 [8] |
| vLLM 核心接口 | `/v1/systemone`、`/v1/decisions` | PR #59299、#60465 尚未合并，跟踪上游，不声明已发布支持 [9][10] |
| SGLang | `/v1/score` 候选 token 评分 | PR #40826 已合并；候选限定单 token，仍需问题编码和答案映射 [11] |

vLLM 另有已公开的 `/generative_scoring`：当前文档返回第一候选 token 的分数，
不能据此承诺一次调用返回全部 Choice/Score 分布。其能力独立评估 [12]。

## Provider、model 与协议

### 身份

- **Provider** 是服务实现及连接配置。沿用 `openai`、`vllm`、`sglang`、
  `ollama`、`llamacpp`、`localai`。实例持有地址、认证、服务特有配置。
- **Model ID** 是该服务接受的部署名称或别名。自托管实例使用自己的 served
  model name，不把引擎名称或 `jev-latest` 强加给其他模型。
- **DecisionModel** 是 `(provider 实例, model_id, 决策配置)` 的任务句柄。
  继续由 `Provider::decision_model(model_id)` 创建。同一模型可以同时具有
  `LanguageModel` 和 `DecisionModel` 句柄。
- **协议** 是适配器实现细节和显式部署配置，例如 SystemOne、OpenAI Decisions、
  SGLang candidate scoring。协议兼容不改变 provider 身份。

例：Ollama 部署的 Clef 使用 provider `ollama`；Cloudflare 托管的 Clef 使用
Cloudflare provider；llama.cpp 部署的 Laya 使用 `llamacpp`。
直接调用作者的 `laya-serve` 才使用 Laya 服务适配配置。

### 能力

能力由运行时版本、模型和启用接口共同决定。现有 `DecisionCapabilities`
继续表达题型、候选数、分布、来源和舍入规则，后续添加输入模态及原生字段支持。
新字段保持 serde 默认兼容，使旧录制仍可读取。

`list_models()` 只返回服务实际给出的发现数据。模型出现在 `/models` 中，
或静态目录称其为 decision model，都不能证明当前部署开放了 decision endpoint。
当前无需新增目录分类或能力注册中心；执行能力以具体任务句柄为准。
不将静态目录信息自动合并进请求配置。

vLLM 决策 endpoint 可能与生成 endpoint 使用不同端口，接入该部署时允许显式指定。
工厂不发网络探测，也不要求服务证明版本。adapter 尚未实现时返回 Unsupported；
已选择 adapter 后由服务正常返回模型／endpoint 错误，不退回 JSON 生成。

## 协议实现

### 共享 SystemOne 编解码

从 Jev adapter 提取请求映射、wire 类型及答案转换，保留官方 Jev 外部入口。
共享模块不固定 provider 名、认证、候选上限、舍入精度或 confidence 含义。
TypeSafe 的 255 choices、10 score levels、两位小数和 Choice argmax 约束
由 Jev adapter 声明；其他服务按各自契约配置和校验。

HTTP 传输继续使用 `HttpRequest` 与既有 JSON/error handler。共享 wire
不要求把不同服务的 endpoint、错误处理或 provider metadata 合并成同一个 provider。

### OpenAI Decisions

在现有 `OpenAIProvider` 上实现独立决策句柄，调用 `{base_url}/decisions`。
复用 OpenAI key、organization、project、headers 和重试配置。
只有明确的 OpenAI provider 使用此工厂，不向所有 OpenAI-compatible wrapper
自动授予决策能力。模型名显式传入，由服务验证其可用性。

第一阶段采用当前 canonical contract 可无损表达的文本子集：

- state 字符串直接传入 `input`；对象／数组序列化为 JSON 文本，不解释为媒体。
- Boolean → predicate；Choice 的字符串 label → choices.value；Score 的文本
  levels → levels.label。题目 ID → name，保持题目顺序。
- OpenAI 只接受文本描述；对象／数组 guidance 和 Boolean true/false criteria
  在 HTTP 前报 Unsupported，不静默丢弃或改写。
- Choice 的原生 boolean value、Score 独立 label/description 以及媒体输入，
  留待 canonical 类型扩展后暴露；不能把 boolean 与同名字符串合并。
- Choice 2–255 项；Score 至少两级，上限只在官方资料明确时设置。
- 保留逐题 refusal：增加 `DecisionAnswer::Refusal`，序列化为
  `{"type":"refusal"}`。拒答是该题的结果，其他正常答案仍返回。
- 响应按 name 关联；不再额外要求数组位置相同。拒绝重复／缺失／额外答案、错误类型、重复概率项、
  Score 索引或 label 不符。原生概率、confidence 和 usage/raw 均保留。
- 不继承 Jev 的两位小数规则。未见 OpenAI 承诺十进制舍入，先仅容忍既有
  浮点误差；线上证据若表明不同规则，应记录证据并调整该 provider。
- 本阶段仅接受明确列出的 provider options，例如 `openai.safety_identifier`；
  不解释其他 provider 的 namespace。拒绝本 namespace 中无定义参数和生成接口的
  body overrides，避免默默丢弃用户要求。服务响应中的新增字段保留在 raw 中。
- usage 是统计信息，不参与答案有效性判断。缺失、未知或不一致的计数保留 raw；
  无法计算的规范化计数留空，不因 `total_tokens` 对不上或 details 缺失丢弃答案。

### OpenAI wire 契约与映射完整性

接口为同步 JSON POST，Bearer 认证；本阶段不提供 stream 选项。使用共享的
HTTP 错误解析、Retry-After、重试、超时和取消机制，不新增决策专用重试器。
模型公开名称目前为 `gpt-6-luna`，它是示例值而非客户端允许名单。

| 原生字段／行为 | canonical 映射或处理 |
|---|---|
| `model` | 请求使用 model_id；响应保留实际 model，不要求与别名相等 |
| `input: string` | state 文本；JSON state 做确定的 JSON 序列化 |
| `input: user messages`，文本／内嵌图片 | 官方支持；本阶段暂不暴露，不从 state 数组猜测 |
| `questions[].name` | 始终发送 canonical 非空 ID；因而响应 name 缺失／null 是无效匹配 |
| `predicate.instructions`、响应 `probability` | Boolean instructions、P(true)，调用方自行定阈值 |
| `choice.choices[].value/description` | 字符串 label／可选文本 description；原生 boolean value 待扩展 |
| `score.levels[].label/description` | 首批文本 level → label；独立 description 待扩展 |
| `choice.probabilities[]` | 检查重复 value 后转换为 label map；不重算 confidence |
| `score.probabilities[].value/label/probability` | 按零起点 index 排列，核对 label；保留期望分数 |
| `answers[].type=refusal` | 逐题 Refusal，无概率；不丢弃其他题目 |
| `safety_identifier` | `provider_options.openai.safety_identifier`，最多 128 字符 |
| `usage` 与 details | 保留 raw 和可读取计数；未承诺信息保持未知 |

机制边界：官方定义有限候选上的决策概率与 Score 期望值；公开 API 文档未披露
内部网络结构、是否恰好一次 forward pass、校准训练方法或十进制舍入规则。
设计不把这些未知机制当作事实。HTTP 200 中的拒答与 HTTP 错误分开处理。

### vLLM 与 SGLang 候选评分

接 typed endpoint 时由服务器完成题目编码；接 scoring primitive 时，adapter
必须明确模型编码，包含 tokenizer/revision、prompt encoder 版本、
候选标签与 token ID 映射、归一化／temperature 规则。

SGLang 的能力先限定 single-token 候选。多个问题可能需要多次真实 HTTP
调用，均归入同一 decision call 和对应 retry attempt，不伪造一个批量响应。
Boolean 的 P(true)、Choice 的候选分布、Score 的零起点期望值分别校验。
未经验证的多 token 候选直接拒绝。返回来源为 `logit_scoring`；该标记不等于
概率已针对用户数据校准。

先为首个已验证模型实现普通的编码函数和必要配置，不预建可插拔 strategy/profile
框架、模型别名注册表或任意 prompt DSL。出现第二种实际实现差异时再提取接口。

官方示例中的 SemIf 编码可作为考察对象，但它不是 SGLang 的通用题目 schema；
不能因为 `/v1/score` 存在就替任意权重选择一套 prompt 并宣称已支持 decide。

## 多模态与扩展结果

后续增加独立媒体输入字段，复用现有媒体类型中适合决策的部分；不把任意 JSON
state 猜测成消息或图片。OpenAI 的 data URL 和最多 128 张图，以及 Cloudflare、
Ollama、llama.cpp 的不同媒体约束，分别由 adapter 校验。

拒答先作为独立 variant 实现。条件跳题、abstention、模型特有 action head
需以后续公开契约分别设计，不能折算成 Boolean false 或零分。

## 录制、回放与宿主语言

继续使用 RFC-0023 的同一 JSONL/ring recorder，operation=decision。
保存原始 HTTP、规范化结果、请求 model ID 与响应实际 model、协议和能力快照。
scoring profile 的版本及映射进入快照；每次 HTTP exchange 如实记录。
凭据由既有边界脱敏，live replay 重新注入 key，不能重发 `[REDACTED]`。

每个 adapter 都应具备：离线 contract fixture、真实 HTTP 模拟测试、统一
record→mock replay、provider 重建→HTTP replay。手写 fixture 明确标为 synthetic；
只有实际 API 调用产生的录制才标为 live。没有 key 时不得把离线验证说成实测。

各语言继续共用 DecisionCallOptions/DecisionResult。在已有 ProviderHandle 上增加
`decisionModel(model_id)`／`decision_model(model_id)`，复用地址、key、headers 等配置。
C ABI 增加 `aimux_provider_decision_model`。不再逐厂商增加一组语言构造器。
已有 Jev 快捷工厂保持兼容。Refusal 同步到 TypeScript 类型，其余 JSON 宿主原样保留。

## 自审结论与剩余契约工作

2026-10-08 自审删去两种多余校验：按题目 ID 关联后再校验数组位置、用 token
统计自洽性决定答案成败。保留身份／类型／完整分布等业务语义校验。
共享 SystemOne 仅抽取实际 wire 代码；Jev argmax 规则留在 Jev，不增加通用策略开关。
复用 ProviderHandle，暂不增加目录类型、运行时探测或评分插件体系。

官方清单不代表所有服务的 wire 设计已经完整。OpenAI 首批映射已列明；其他 adapter
开始实现前还需补充各自的版本、完整样本、认证／封装、题型限制和以下差异：

| 接口 | 已确认机制 | 仍须落实的契约细节 |
|---|---|---|
| SystemOne 家族 | state + question map → answer map；Boolean 概率、Choice 分布、Score 期望 | 各实现限制、confidence 定义、usage 缺省行为，不继承 Jev 常量 |
| vLLM diffusion 示例 | 固定答案槽、去噪后读取候选分布，可多次读取 | samples/steps/think、条件跳题、diagnostics、单 token 标签、图像封装 |
| SGLang score | 候选 token 评分、温度缩放，可返回未缩放词表 logprobs | 逐项请求字段、归一化轴、批量维度、模型编码和版本固定 |
| vLLM generative scoring | 同一 prompt 下候选 token 概率，返回首项评分 | 能否取得完整分布及请求成本；先不承诺三种题型 |

这些缺项作为对应阶段的进入条件，不用猜测值补齐，也不阻塞已明确契约的首批实现。

## 实施计划与验收

任务完成以代码、契约测试和文档为准，不能把上游存在接口标为本库完成。

| 阶段 | 任务 | 当前状态 | 验收条件 |
|---|---|---|---|
| 0 | TypeSafe 官方 Jev | 已完成 | PR #221、统一 live 录制与回放 |
| 1 | 本 RFC、共享 SystemOne codec | 已实现，待 PR 合入 | Jev 全部已有 fixture／录制回归保持通过 |
| 1 | OpenAI 文本 decision 与逐题 refusal | 已实现，待 PR 合入 | 三题型、坏响应、错误／重试、本地 HTTP 录制回放、各宿主工厂 |
| 2 | Ollama、llama.cpp | 待开发 | 官方版本／模型约束、各自 profile 与统一回放 |
| 2 | Cloudflare、LocalAI、Laya 官方服务 | 待开发 | 路由／认证／响应封装、原生语义、各自回放 |
| 2 | vLLM DiffusionGemma 官方示例 | 待开发 | 显式部署 endpoint、真实 slot 分布与 diagnostics |
| 3 | SGLang 候选评分 | 待开发 | 固定模型编码 profile、单 token 校验、概率映射和录制 |
| 3 | vLLM generative scoring | 待评估 | 明确可实现题型，不能把首项分数当完整分布 |
| 4 | 显式图像输入及细化能力字段 | 待开发 | 各服务媒体限制、跨语言序列化、回放 |
| 4 | OpenAI typed choice／完整 Score 描述 | 待开发 | 可逆的 canonical 表达、旧请求与录制兼容 |
| 跟踪 | vLLM 核心两个 typed endpoint | 等待上游合并 | 固定已发布版本后接入，替换／扩展原有配置 |

第一阶段 PR 完成不代表本表整体完成。后续 PR 按此表推进并更新状态。

第一阶段使用官方 reference 构造的 `openai_decisions.json` synthetic fixture，
覆盖拒答、乱序答案、未知响应字段、可选 usage、HTTP 错误／重试和统一回放。
没有执行 OpenAI 线上调用，不将这些样本称为 live 录制。

## 官方来源

1. [OpenAI Decisions guide](https://developers.openai.com/api/docs/guides/decisions)
2. [OpenAI Decisions reference](https://developers.openai.com/api/reference/resources/decisions/methods/create)
3. [Cloudflare Clef](https://developers.cloudflare.com/workers-ai/models/clef/)
4. [Ollama 官方公告](https://ollama.com/blog/ollama-now-supports-jev-style-decision-models)
5. [llama.cpp server reference](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md#typesafe-compatible-api-endpoints)
6. [LocalAI Decisions](https://localai.io/docs/features/decisions/)
7. [Laya 作者仓库 HTTP server](https://github.com/NandhaKishorM/laya/blob/main/laya/serve.py)
8. [vLLM structured diffusion](https://docs.vllm.ai/en/latest/examples/features/structured_diffusion/)
9. [vLLM #59299](https://github.com/vllm-project/vllm/pull/59299)
10. [vLLM #60465](https://github.com/vllm-project/vllm/pull/60465)
11. [SGLang #40826](https://github.com/sgl-project/sglang/pull/40826)
12. [vLLM generative scoring](https://docs.vllm.ai/en/latest/serving/online_serving/generative_scoring/)
