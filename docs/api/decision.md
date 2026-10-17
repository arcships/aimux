# Decision API

`decide` 对同一个文本或 JSON state 回答多道 typed questions，结果按问题 ID
索引。模型契约是 AI SDK 的 `DecisionModelV4`（`@ai-sdk/provider@4.0.22
src/decision-model/v4/`），操作对应 `ai@7.0.128 src/decide/decide.ts`。
支持 TypeSafe、OpenAI、Cloudflare 和官方本地运行时的原生决策接口，以及
OpenAI/Google 语言模型的结构化输出适配。
[RFC-0037](../../rfc/0037-decision-api.md) 记录基础 contract，
[RFC-0038](../../rfc/0038-decision-provider-expansion.md) 记录各接口的 wire、机制、限制与上游版本。

## 请求与结果

`DecideOptions` 是 V4 call options 加操作设置：

| 字段 | 说明 |
|---|---|
| `state` | 字符串、JSON 对象或 JSON 数组；数组仍是一个 state |
| `questions` | 以问题 ID 为键的对象，插入顺序即问题顺序 |
| `max_retries` | 瞬时失败的重试次数，缺省 2 |
| `timeout` | 操作截止时间（aimux 扩展），沿用已有毫秒配置 |
| `headers` | 额外请求头 |
| `provider_options` | provider 命名空间下的扩展参数，见各 provider 一节 |

三种题型：

```json
{
  "urgent": {"type": "boolean", "instructions": "Needs action today?",
             "criteria": {"true": "blocked payments", "false": null}},
  "team": {"type": "choice", "instructions": "Which team?",
           "criteria": {"billing": "Invoices and refunds", "support": null}},
  "severity": {"type": "score", "instructions": "Rate severity",
               "criteria": ["minor", "major", "critical"]}
}
```

Choice 的 criteria 是选项名到描述的非空对象（`null` 表示无描述），Score 的
criteria 是至少两级的有序数组，Boolean 的 criteria 可省略，只能描述 `true` /
`false`。instructions 与各项描述可以是字符串、对象或数组；provider 不支持的形态返回
UnsupportedFunctionality。

`decide` 依次执行：校验输入（`validate-decision.ts:50-125`）→ 在任何 I/O 前拒绝
模型不支持的题型 → 按 `max_retries` 重试 `do_decide` → 重试结束后校验答案 →
补全 usage 与 response。结果 `DecideResult`：

| 字段 | 说明 |
|---|---|
| `answers` | 每题恰好一个答案，按问题顺序 |
| `usage` | `{input_tokens, output_tokens, total_tokens}`；未报告的为 null，两者都已知时才有合计 |
| `warnings` | provider 警告 |
| `rounding` | provider 声明的 `probability_decimals` / `score_decimals` |
| `provider_metadata` | 以 provider 名为键的扩展信息 |
| `response` | `timestamp`（缺省为调用结束时）、`model_id`（缺省为模型 ID）、`headers`、`body` |

答案形态：Boolean `{type:'boolean', probability}` 为 P(true)；Choice
`{type:'choice', choice, probabilities?}`，choice 是选项名；Score
`{type:'score', score, probabilities?}`，score 为零起点的小数位置，分布以
`"0"`、`"1"`… 为键。决策没有部分结果：某题拒答或被跳过时整个调用失败。

## OpenAI 原生 Decisions

复用现有 provider 连接配置，显式创建 decision model。模型是否可用由服务验证；
创建句柄和描述模型均不发网络请求。

```ts
import { createProvider, decide } from '@arcships/aimux'

const provider = await createProvider('openai', process.env.OPENAI_API_KEY)
const model = await provider.decisionModel('gpt-6-luna')
const result = await decide(model, {
  state: 'Please refund the duplicate charge',
  questions: { refund: { type: 'boolean', instructions: 'Is a refund requested?' } },
  provider_options: { openai: { safety_identifier: 'app-user-123' } },
})
console.log(result.answers.refund) // { type: 'boolean', probability: 0.93 }
```

Python 对应 `create_provider('openai', key).decision_model('gpt-6-luna')`，随后
调用下文的 `decide`。Rust 使用 `create_openai(settings)` 返回的 provider 的
`decision(model_id)` 或 `Provider::decision_model`。服务地址为 `{base_url}/decisions`，默认使用
`https://api.openai.com/v1`；继承 provider 的认证、headers、organization/project 和传输，
重试次数由调用的 `max_retries` 控制。Registry 的 `openai:<model>` 解析到这个原生模型。

文本 state 原样发送；JSON 对象／数组序列化为文本。Boolean 映射 predicate。
Choice 支持 2–255 项，Score 至少两级，每级接受文本或 `{label, description?}`。
服务拒答某题时返回错误 `OpenAI declined to answer question "<id>"`。

`provider_options.openai` 接受：

| 参数 | 说明 |
|---|---|
| `images` | 最多 128 张内嵌图 `{data: <base64>, media_type, detail?}`，detail 支持 low/high/auto/original，位于 state 之前 |
| `choice_values` | `{<question id>: {<option>: true \| false \| "text"}}`，选项的原生值；缺省使用选项名 |
| `safety_identifier` | 最多 128 字符 |

```ts
const result = await decide(model, {
  state: 'Rate the picture',
  questions: {
    choice: { type: 'choice', instructions: 'Pick a value', criteria: { boolean_true: null, text_true: null } },
    quality: { type: 'score', instructions: 'Rate quality', criteria: [
      { label: 'low', description: 'Missing key details' },
      { label: 'high', description: 'All details visible' },
    ] },
  },
  provider_options: { openai: {
    images: [{ data: imageBase64, media_type: 'image/png', detail: 'low' }],
    choice_values: { choice: { boolean_true: true, text_true: 'true' } },
  } },
})
// answers.choice.choice = 'boolean_true'；
// provider_metadata.openai.answers.choice.value = true
```

对象/数组 instructions 和 Boolean criteria 返回 Unsupported。
原生概率保留原值，不套用 Jev 两位小数规则；每题 confidence 在
`provider_metadata.openai.answers.<id>.confidence`。usage 缺失或合计不一致
不影响有效答案。本地 fixture 基于[官方 reference](https://developers.openai.com/api/reference/resources/decisions/methods/create)，并非新增服务的线上录制。

## 语言模型结构化输出

AI SDK 的 OpenAI 与 Google provider 通过语言模型的 JSON 输出回答问题
（`@ai-sdk/provider-utils@5.0.55 src/decision-language-model.ts`）。Rust 的
`DecisionLanguageModel::new(language_model, provider)` 包装任一语言模型，缺省
provider 为 `<语言模型 provider>.decision`。`OpenAIProvider::evaluation(model_id)`
使用 Responses 模型，provider 为 `openai.evaluation`；Google provider 的
`decision_model` 即为这个适配，provider 为 `google.evaluation`。语言模型调用与
决策在同一条录制中。

## 本地运行时和 Cloudflare

```ts
const runtime = await createProvider('sglang', undefined, {
  baseUrl: 'http://127.0.0.1:30000/v1',
})
const model = await runtime.decisionModel('my-served-model')
const result = await decide(model, {
  state: { ticket: 'Checkout failed' },
  questions: { urgent: { type: 'boolean', instructions: 'Needs action today?' } },
  provider_options: { sglang: { protocol: 'decisions', temperature: 1 } },
})
```

省略 protocol 使用 SystemOne。model ID 必须是部署接受的名称，不能将引擎名当成
模型名；创建句柄不探测服务器，模型/版本需要具备对应能力。

| Provider | 接口 | 模型/版本要求 |
|---|---|---|
| ollama | /v1/systemone | ≥0.35，decision-capable model |
| llamacpp | /v1/systemone | 含 decision metadata 的 GGUF；图片需 projector |
| localai | /v1/systemone | 配置 decisions usecase 和对应 backend |
| laya | /v1/systemone | 作者 laya-serve，model 为 checkpoint 名称 |
| sglang | /v1/systemone、/v1/decisions、/v1/score | 2026-10-08 main 已支持；稳定版本未包含时使用 nightly |
| vllm | 示例服务 /v1/systemone；核心 /generative_scoring | diffusion 必须显式配置独立 structured server 地址 |
| cloudflare / cloudflare_workers_ai | /ai/run/@cf/cloudflare/clef 或 clef-flash | baseUrl 配到带 account 的 /ai/v1；使用 Cloudflare token |

本地 provider 无 key 可用，可显式提供 key 或对应 `<PROVIDER>_API_KEY`；地址可从
`<PROVIDER>_BASE_URL` 或 baseUrl 配置。Ollama 默认 11434，llamacpp/LocalAI 8080，
SGLang 30000，vLLM/Laya 8000。完整地址均含 `/v1`。Cloudflare 使用显式 account
baseUrl，model 可为 `clef` / `clef-flash` 或 `@cf/cloudflare/` 前缀。

`provider_options.<provider>` 可含 `protocol`、`images`，以及下列参数；上游继续校验参数值及模型适用性：

| Namespace / protocol | 参数 |
|---|---|
| ollama、localai | keep_alive（LocalAI 接受但忽略） |
| laya | max_len、head_max_len、task、lang、lang_guess、min_confidence |
| vllm / systemone | instructions、samples、auto_max、auto_threshold、steps、think、chunk_rows、chunk_prompt、sequential、seed；questions.<id>.depends_on/ask_if/alone |
| sglang / systemone | chat_template_kwargs |
| sglang / decisions | temperature、prompt_format_version、return_prompt_token_ids、chat_template_kwargs |
| llamacpp、cloudflare | 只使用公共请求字段 |

请求前检查的数量上限：ollama、localai、laya、cloudflare 最多 64 题；Choice
至少 2 项（sglang 为 1），laya 最多 100 项、cloudflare 最多 255 项；Score
laya 最多 32 级，llamacpp、cloudflare 最多 10 级。

vLLM 上游 SystemOne wrapper 的 ask 子集参数存在答案索引问题，本库不开放；
通过公共 questions 选择题目及其依赖。条件执行（ask_if）跳过的题目使调用失败，
因为决策必须回答每一道题。

Laya 低置信度弃答时保留预测，弃答信息在
`provider_metadata.laya.answers.<id>.abstention`。
SGLang decisions 的 label_mass 不等于 confidence，不作为 confidence 报告。
SGLang SystemOne 普通生成模型返回 `x_label_mass`，来源记为 `logit_scoring`；
专用决策 checkpoint 的响应省略该字段，来源为 `native`。概率来源在
`provider_metadata.<provider>.probability_source`，SystemOne 原有 confidence 在
`provider_metadata.<provider>.answers.<id>.confidence`，LocalAI 等服务的
`latency_ms` 在 `provider_metadata.<provider>.latency_ms`，均保持原值。

各支持图片的 runtime 使用同一 images 形态（与 OpenAI 相同），由 adapter 转成本地协议。Cloudflare
最多 4 张 PNG/JPEG/WebP、单图 4 MiB、合计 8 MiB；LocalAI 最多 8 张 PNG/JPEG、
合计 8 MiB。Laya 不支持图片。Ollama/llama.cpp/vLLM/SGLang 的实际图片能力依模型。
SGLang detail 接受 low/high/auto；其他本地 SystemOne provider 不接受 detail。
像素尺寸与模型 admission 由上游判断；完整限制见 RFC-0038。

## 显式 token 候选评分

有原生 question endpoint 时优先使用它；它负责正确的模型编码。
已有完整 prompt token IDs 的调用方可选 SGLang score 或 vLLM generative_scoring：

```ts
const result = await decide(model, {
  state: 'The state encoded into the prompt',
  questions: { urgent: { type: 'boolean', instructions: 'Needs action today?' } },
  provider_options: { sglang: {
    protocol: 'score',
    model_revision: 'weights-commit',
    tokenizer_revision: 'tokenizer-commit',
    prompt_format_version: 'my-encoder-v1',
    temperature: 1,
    encoded: { urgent: { prompt_token_ids: promptIds, label_token_ids: [yesId, noId] } },
  } },
})
```

IDs 必须来自目标 tokenizer 和答案位置的实际编码。revision 字段记录调用方来源，
不验证部署。每题的完整 prompt 非空，每个候选恰好一个 token ID 且互异；题目 ID
须全部对应。顺序为 Boolean true/false、Choice criteria、Score criteria。此路径发送
编码后的 token IDs，state/questions 用于结果映射与录制；不接图片，不生成答案文本。

vLLM 将 namespace 改为 vllm、protocol 改为 generative_scoring；temperature 只可为 1。
其上游每次只返回第一候选的概率，因此每题 N 个候选实际执行 N 次 prefill。
每次请求及用量均记录；结果 usage 汇总成功 attempt，整次 retry 会重做评分。返回完整分布并验证总和，不做
事后归一化；Boolean 取 P(true)，Choice 取 argmax，Score 计算期望值。
来源均为 logit_scoring，不等于已针对业务数据校准。

## Rust

```rust,no_run
use aimux_core::prelude::*;
use aimux_providers::{JevConfig, JevProvider};

# async fn example() -> Result<(), AiMuxError> {
let provider = JevProvider::new(JevConfig::from_env()?);
let model = provider
    .decision_model("jev-latest")
    .expect("Jev provides decision models")?;
let mut questions = DecisionQuestions::new();
questions.insert(
    "refund".into(),
    DecisionQuestion::Boolean { instructions: "Is a refund requested?".into(), criteria: None },
);
let options = DecideOptions::new("Please refund the duplicate charge", questions);
let result = decide(model.as_ref(), options).await?;
if let DecisionAnswer::Boolean { probability } = result.answers["refund"] {
    println!("P(true) = {probability}");
}
# Ok(())
# }
```

`Provider::decision_model` 对不提供决策模型的 provider 返回 `None`；
`ProviderRegistry::decision_model("provider:model")` 与自定义 provider 将其报告为
NoSuchModel（`modelType = "decisionModel"`），与 `provider-registry.ts:447-461` 一致。

## Node.js

```ts
import { jevDecision, decide, describeDecisionModel } from '@arcships/aimux'

const model = await jevDecision(process.env.TYPESAFE_API_KEY!, 'jev-latest')
const result = await decide(model, {
  state: { message: 'Please refund the duplicate charge' },
  questions: {
    refund: { type: 'boolean', instructions: 'Is a refund requested?' },
    queue: { type: 'choice', instructions: 'Which team handles this?',
      criteria: { billing: 'Invoices and refunds', support: null } },
    urgency: { type: 'score', instructions: 'Rate urgency from low to high',
      criteria: ['low', 'normal', 'high'] },
  },
  max_retries: 0,
})
console.log(result.answers, result.rounding)
console.log(describeDecisionModel(model))
// { specification_version: 'v4', provider: 'jev', model_id: 'jev-latest',
//   supported_question_types: ['choice', 'score', 'boolean'] }
```

默认请求 TypeSafe 官方 `https://api.typesafe.ai/v1/systemone`。
factory 的第三个 `endpoint` 参数可覆盖完整 POST URL，用于显式配置的代理
或本地 contract 测试。第四个 `probabilitySource` 参数接受 `native`、
`logit_scoring` 或 `model_estimate`，官方接入缺省为 `native`。

`decide` 的第三个参数接受 AbortSignal。

## Python

```python
import os
from aimux import jev_decision, decide, describe_decision_model

model = jev_decision(os.environ['TYPESAFE_API_KEY'], 'jev-latest')
result = decide(model, {'message': 'Please refund the duplicate charge'}, {
    'refund': {'type': 'boolean', 'instructions': 'Is a refund requested?'},
}, max_retries=0)
print(result['answers']['refund']['probability'])
print(describe_decision_model(model))
```

`decide(model, state, questions, **options)` 的关键字参数即 `DecideOptions` 的其余字段。
`jev_decision` 可选第四个参数 `probability_source` 缺省为 `'native'`，结果在
`provider_metadata['jev']['probability_source']` 保留声明的来源。

## Jev 原生题目描述字段

instructions、Choice criteria 的描述和 Score criteria 的每一级均支持字符串、对象、
数组。这里描述 Jev 能力，OpenAI 的 instructions/choice 描述接受文本，Score 支持独立 label/description。Boolean 可提供 `criteria.true` / `criteria.false` 描述。对象/数组以原生
JSON 发送。下面的 questions 可用于所有绑定：

```json
{
  "urgent": {"type": "boolean",
             "instructions": {"question": "Is immediate action needed?"},
             "criteria": {"true": {"includes": ["blocked payments"]}, "false": ["routine"]}},
  "team": {"type": "choice", "instructions": ["Route the ticket"],
           "criteria": {"billing": {"includes": ["payments"]}, "technical": null}},
  "severity": {"type": "score", "instructions": "Rate severity",
               "criteria": [{"label": "minor"}, ["blocking", "No workaround"]]}
}
```

Rust 使用 `DecisionInput::Text/Object/Array`，字符串可以 `.into()`。
Boolean criteria 使用 `DecisionBooleanCriteria`。顶层数字、布尔值或 null 不作为
题目描述；Core 按上游规则校验空 instructions、空 criteria、Score 少于两级、
Boolean criteria 的未知键等错误。Jev 在请求前检查 Choice 最多 255 项、Score 最多 10 级，
不接受 provider options。

## Go / Java / Kotlin / Swift / Flutter

五种绑定复用相同请求/结果 JSON contract。通过已有的 provider 工厂创建
`openai` 句柄后，调用 Go `provider.DecisionModel("gpt-6-luna")`，或
Java/Kotlin/Swift/Flutter `provider.decisionModel("gpt-6-luna")`。
模型独立持有连接配置，关闭 provider 后仍可使用；模型需按下表释放。
下表保留 Jev 快捷工厂，默认访问 TypeSafe 官方服务。

| 语言 | 创建模型 | 调用 | 描述模型 | 释放 |
|---|---|---|---|---|
| Go | `NewJevDecision(key, "jev-latest")` | `model.Decide(options)` / `DecideContext(ctx, options)` | `model.Describe()` | `defer model.Close()` |
| Java | `DecisionModel.jev(key, "jev-latest")` | `model.decide(optionsJson)` | `model.describe()` | try-with-resources |
| Kotlin | `DecisionModel.jev(key, "jev-latest")` | `model.decide(optionsJson)` | `model.describe()` | `use {}` |
| Swift | `DecisionModel.jev(apiKey: key, modelId: "jev-latest")` | `model.decide(options: optionsJson)` | `model.describe()` | `close()` 或 ARC |
| Flutter | `DecisionModel.jev(key, 'jev-latest')` | `model.decide(optionsMap)` / `decideJson(optionsJson)` | `model.describe()` | `model.close()` |

Go 的 Decide 接受可 JSON 序列化的对象，Context 取消会停止 native HTTP 请求。
Java/Kotlin/Swift 的调用及描述返回 JSON 字符串；Flutter 返回 Map 并提供
原始 JSON 调用。Java/Kotlin/Swift/Flutter 可传原有 C ABI abort handle；0 表示
无显式取消。C ABI 调用同步阻塞，Flutter 可将 HTTP 调用放入 worker isolate。

## C ABI

`aimux_provider_handle_new("openai", key, config_json, &provider_handle)` 和
`aimux_provider_decision_model(provider_handle, model_id, &handle)` 创建 OpenAI
决策模型。不提供决策模型的 provider 返回 `AIMUX_E_NO_SUCH_MODEL`。

`aimux_jev_decision_new(key, model_id, endpoint_or_NULL, &handle)` 创建模型，
`aimux_decide(handle, opts_json, &out_json)` 接受 `DecideOptions` JSON、返回
`DecideResult` JSON，`aimux_decide_with_abort` 可传入现有 abort handle。返回错误遵循
[错误模型](../error-model.md)。用 aimux_drop_handle 释放模型，
aimux_free_string 释放结果。`aimux_decision_capabilities(handle, &out_json)`
返回 `{specification_version, provider, model_id, supported_question_types}`，
无网络请求，并遵守相同的字符串生命周期（导出名保持兼容）。

显式配置概率来源时可使用 `aimux_jev_decision_new_with_probability_source(key, model_id,
endpoint, probability_source, &handle)`，source 接受上述三个值，NULL 使用
`native`。原有 `aimux_jev_decision_new` 签名保持兼容。未知 source 在模型创建时
返回 InvalidArgument。

## 结果语义

Decision adapter 的支持范围限于原生决策接口、候选评分能力和上游的语言模型
结构化输出适配，不提供生成式 fallback。`model_estimate` 仅用于如实记录
外部服务的概率来源，不表示 aimux 实现了相应模拟能力。

Boolean probability 表示 P(true)，调用方设置业务阈值。Choice 的 choice
是选项名，有分布时必须是最大概率项（允许并列）。Score 是零起点的浮点位置，
有分布时必须等于概率加权的平均位置。分布若提供必须完整、和为 1。
Core 在重试结束后按 `validate-decision.ts:176-298` 校验答案：缺省容差为 1e-6；
provider 在结果的 `rounding` 中分别声明 `probability_decimals` 和
`score_decimals`（0–15）时，每个舍入值容许半个末位单位。TypeSafe 声明两者均为 2。
原始数字不会被改写。
probability_source 表示来源，不保证校准；confidence 保留 provider 定义。
raw response、实际执行的 model 和原始 usage 可用于审计。
TypeSafe 的 usage 对象必须存在；input_tokens/output_tokens 可缺省或为 null，
未报告的数量保留为未知，不影响有效答案。

TypeSafe 官方 Choice 支持 1–255 项，Score 支持 2–10 级；state 支持字符串、
对象或数组。不施加未公布的题目数量、ID 字符或文字长度上限。依据见
[官方 contract](https://docs.typesafe.ai/api)。
模型描述在所有绑定均可用，只含 V4 属性：规范版本、provider、模型 ID 与支持的题型。
不发起 HTTP 请求。数量上限等 provider 限制在调用时、HTTP 之前检查。

## 录制与回放

`decide` 复用统一录制器；各语言现有的 `init_recording` / ring recording
入口均会录制 decision。无需在调用处另存 JSON。例如 Python：

```python
from aimux import init_recording, recording_flush, recording_stop

init_recording("./recordings")
try:
    result = decide(model, state, questions)
finally:
    recording_flush()
    recording_stop()
```

标准 `recordings.jsonl` 每行记录一次逻辑调用：`input.operation = "decision"`，
`input.options` 是脱敏后的 `DecideOptions`（state/questions/headers/provider_options
及操作设置）；不保存能力快照，模型属性由回放时调用方 registry 重建的 model 提供；
provider 只记录 provider_id/provider/model_id，endpoint 与凭证来自调用方配置；
exchanges 记录各重试 attempt 的 HTTP 请求、响应和时延；
`outcome.decision_result` 保存 `decide` 返回的 `DecideResult`，`outcome.usage`
保存 `DecideUsage`。凭据沿用统一脱敏规则。
录制使用 schema 2，与 aimux 0.5 相同；语言录制缺少 operation 时仍按 language model 解析。
aimux 0.5 写的 decision 录制可以读取，但其选项与结果是旧形状、没有 provider_id，回放时报错说明。

CLI 离线回放不需要 key：

```sh
aimux-replay ./recordings/recordings.jsonl --mock
aimux-replay ./recordings/recordings.jsonl --dry-run
```

Rust 可用 `aimux_core::replay::MockDecisionReplayModel::from_jsonl` 加载录制，
再通过 `decide` 或 `replay_decision_with_model` 离线回放。匹配 provider/model、
state/questions、headers 和 provider_options；timeout/retry 控制不参与匹配。
mock 接受任意题型，返回录制的结果。
输入未命中或记录不完整时返回错误。请求回放可通过
`aimux_providers::rebuild_decision_provider` 从调用方 registry 重建上述任一官方 provider 的 decision model，再调用
`replay_decision_with_model` 发出真实请求；CLI 不带 `--mock` 时也走这条路径。

实测录制在 `aimux-providers/tests/fixtures/jev_systemone_live.jsonl`。
Provider 回归测试通过共享 HTTP replay helper 重放实际 wire 交换；另有
规范化结果回放测试。两者均离线执行。
OpenAI 使用 `aimux-providers/tests/fixtures/openai_decisions.json` 的 synthetic
契约样本，运行时使用 runtime_decisions.json；图片/typed values 共用
contract-tests/fixtures/decision-openai-full.json。均覆盖本地 HTTP、录制→mock replay、
重建 provider→HTTP replay，多次评分保留所有真实 exchange。
