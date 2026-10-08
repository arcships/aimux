# Decision API

`decide` 对同一个文本或 JSON state 回答多道 typed questions，结果按问题 ID
索引。支持 TypeSafe、OpenAI、Cloudflare 和官方本地运行时的原生决策接口。
[RFC-0037](../../rfc/0037-decision-api.md) 记录基础 contract，
[RFC-0038](../../rfc/0038-decision-provider-expansion.md) 记录各接口的 wire、机制、限制与上游版本。

## OpenAI 原生 Decisions

复用现有 provider 连接配置，显式创建 decision model。模型是否可用由服务验证；
创建句柄和查询能力均不发网络请求。

```ts
import { createProvider, decide } from '@arcships/aimux'

const provider = await createProvider('openai', process.env.OPENAI_API_KEY)
const model = await provider.decisionModel('gpt-6-luna')
const result = await decide(model, {
  state: 'Please refund the duplicate charge',
  questions: [{ id: 'refund', type: 'boolean', instructions: 'Is a refund requested?' }],
  provider_options: { openai: { safety_identifier: 'app-user-123' } },
})
console.log(result.answers.refund) // boolean 概率或 { type: 'refusal' }
```

Python 对应 `create_provider('openai', key).decision_model('gpt-6-luna')`，随后
调用下文的 `decide`。Rust 使用 `OpenAIProvider::new(OpenAIConfig::new(key))`
和 `Provider::decision_model`。服务地址为 `{base_url}/decisions`，默认使用
`https://api.openai.com/v1`；继承 provider 的认证、headers、organization/project 和重试配置。

文本 state 原样发送；JSON 对象／数组序列化为文本。Boolean 映射 predicate。
Choice 支持 2–255 项，`label` 为规范化结果的 map key，可选 `value` 是原生 string
或 boolean；缺省使用 label。Score 至少两级，接受文本或 `{label, description?}`。
逐题拒答为 `{"type":"refusal"}`，不会丢弃其他题目的答案。

```ts
const result = await decide(model, {
  state: 'Rate the picture',
  images: [{ data: { Base64: imageBase64 }, media_type: 'image/png', detail: 'low' }],
  questions: [
    { id: 'choice', type: 'choice', instructions: 'Pick a value', options: [
      { label: 'boolean_true', value: true },
      { label: 'text_true', value: 'true' },
    ] },
    { id: 'quality', type: 'score', instructions: 'Rate quality', levels: [
      { label: 'low', description: 'Missing key details' },
      { label: 'high', description: 'All details visible' },
    ] },
  ],
})
// Choice: selected='boolean_true', value=true；两个原生值使用不同概率 map key。
```

最多 128 张内嵌图，detail 支持 low/high/auto/original。也可使用
`data: { Binary: [137, 80, ...] }`，与已有 FileBytes contract 一致。
对象/数组 instructions、Boolean criteria 和生成接口 body overrides 返回 Unsupported。
`openai` options 仅接受可选 safety_identifier（最多 128 字符）。
原生概率/confidence 保留原值，不套用 Jev 两位小数规则；usage 缺失或合计不一致
不影响有效答案。本地 fixture 基于[官方 reference](https://developers.openai.com/api/reference/resources/decisions/methods/create)，并非新增服务的线上录制。

## 本地运行时和 Cloudflare

```ts
const runtime = await createProvider('sglang', undefined, {
  baseUrl: 'http://127.0.0.1:30000/v1',
})
const model = await runtime.decisionModel('my-served-model')
const result = await decide(model, {
  state: { ticket: 'Checkout failed' },
  questions: [{ id: 'urgent', type: 'boolean', instructions: 'Needs action today?' }],
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

每个 options namespace 只接受下列参数；上游继续校验参数值及模型适用性：

| Namespace / protocol | 参数 |
|---|---|
| ollama、localai | keep_alive（LocalAI 接受但忽略） |
| laya | max_len、head_max_len、task、lang、lang_guess、min_confidence |
| vllm / systemone | instructions、samples、auto_max、auto_threshold、steps、think、chunk_rows、chunk_prompt、sequential、seed；questions.<id>.depends_on/ask_if/alone |
| sglang / systemone | chat_template_kwargs |
| sglang / decisions | temperature、prompt_format_version、return_prompt_token_ids、chat_template_kwargs |
| llamacpp、cloudflare | 只使用公共请求字段 |

vLLM 上游 SystemOne wrapper 的 ask 子集参数存在答案索引问题，本库不开放；
通过公共 questions 选择题目及其依赖，条件执行使用 ask_if。

Laya 低置信度弃答返回 `{type:'abstention'}`，门限、原始分布和 action head 在 raw 中。
vLLM 条件跳题返回 `{type:'skipped'}`，diagnostics 保留跳题原因。
SGLang decisions 的 label_mass 不等于 confidence；canonical confidence 留空。

各支持图片的 runtime 使用同一 images 输入，由 adapter 转成本地协议。Cloudflare
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
  questions: [{ id: 'urgent', type: 'boolean', instructions: 'Needs action today?' }],
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
须全部对应。顺序为 Boolean true/false、Choice options、Score levels。此路径发送
编码后的 token IDs，state/questions 用于结果映射与录制；不接图片，不生成答案文本。

vLLM 将 namespace 改为 vllm、protocol 改为 generative_scoring；temperature 只可为 1。
其上游每次只返回第一候选的概率，因此每题 N 个候选实际执行 N 次 prefill。
每次请求及用量均记录；结果 usage 汇总成功 attempt，整次 retry 会重做评分。返回完整分布并验证总和，不做
事后归一化；Boolean 取 P(true)，Choice 取 argmax，Score 计算期望值，confidence
保持未知。来源均为 logit_scoring，不等于已针对业务数据校准。

## Rust

```rust,no_run
use aimux_core::prelude::*;
use aimux_providers::{JevConfig, JevProvider};
use serde_json::json;

# async fn example() -> Result<(), AiMuxError> {
let provider = JevProvider::new(JevConfig::from_env()?);
let model = provider.decision_model("jev-latest")?;
let request = DecisionCallOptions::new(json!({"message": "Please refund the duplicate charge"}), vec![
    DecisionQuestion::Boolean {
        id: "refund".into(), instructions: "Is a refund requested?".into(), criteria: None,
    },
]);
let result = decide(model.as_ref(), request).await?;
if let DecisionAnswer::Boolean { probability_true } = result.answers["refund"] {
    println!("P(true) = {probability_true}");
}
# Ok(())
# }
```

## Node.js

```ts
import { jevDecision, decide, decisionCapabilities } from '@arcships/aimux'

const model = await jevDecision(process.env.TYPESAFE_API_KEY!, 'jev-latest')
const result = await decide(model, {
  state: { message: 'Please refund the duplicate charge' },
  questions: [
    { id: 'refund', type: 'boolean', instructions: 'Is a refund requested?' },
    { id: 'queue', type: 'choice', instructions: 'Which team handles this?',
      options: [{ label: 'billing', description: 'Invoices and refunds' }, { label: 'support' }] },
    { id: 'urgency', type: 'score', instructions: 'Rate urgency from low to high',
      levels: ['low', 'normal', 'high'] },
  ],
  max_retries: 0,
})
console.log(result.answers, result.rounding)
console.log(decisionCapabilities(model))
```

默认请求 TypeSafe 官方 `https://api.typesafe.ai/v1/systemone`。
factory 的第三个 `endpoint` 参数可覆盖完整 POST URL，用于显式配置的代理
或本地 contract 测试。第四个 `probabilitySource` 参数接受 `native`、
`logit_scoring` 或 `model_estimate`，官方接入缺省为 `native`。

`decide` 的第三个参数接受 AbortSignal。timeout 复用已有毫秒配置。

## Python

```python
import os
from aimux import jev_decision, decide, decision_capabilities

model = jev_decision(os.environ['TYPESAFE_API_KEY'], 'jev-latest')
result = decide(model, {'message': 'Please refund the duplicate charge'}, [
    {'id': 'refund', 'type': 'boolean', 'instructions': 'Is a refund requested?'}
], max_retries=0)
print(result['answers']['refund']['probability_true'])
print(decision_capabilities(model))
```

可选第四个参数 `probability_source` 缺省为 `'native'`，结果保留声明的来源。

## Jev 原生题目描述字段

instructions、Choice description 和 Score levels 的每一级均支持字符串、对象、
数组。这里描述 Jev 能力，OpenAI 的 instructions/choice description 接受文本，Score 支持独立 label/description。Boolean 可提供 `criteria.true` / `criteria.false` 描述。对象/数组以原生
JSON 发送，Score 返回的 levels 保留结构化描述。下面的 questions 可用于所有绑定：

```json
[
  {"id":"urgent","type":"boolean",
   "instructions":{"question":"Is immediate action needed?"},
   "criteria":{"true":{"includes":["blocked payments"]},"false":["routine"]}},
  {"id":"team","type":"choice","instructions":["Route the ticket"],
   "options":[{"label":"billing","description":{"includes":["payments"]}},
              {"label":"technical"}]},
  {"id":"severity","type":"score","instructions":"Rate severity",
   "levels":[{"label":"minor"},["blocking","No workaround"]]}
]
```

Rust 使用 `DecisionDescription::Text/Object/Array`，字符串可以 `.into()`。
Boolean criteria 使用 `DecisionBooleanCriteria`。顶层数字、布尔值或 null 不作为
题目描述；Core 校验空 instructions、重复选项/等级和缺少等级等错误。

## Go / Java / Kotlin / Swift / Flutter

五种绑定复用相同请求/结果 JSON contract，图片、typed values、Score 描述均保持原生类型。通过已有的 provider 工厂创建
`openai` 句柄后，调用 Go `provider.DecisionModel("gpt-6-luna")`，或
Java/Kotlin/Swift/Flutter `provider.decisionModel("gpt-6-luna")`。
模型独立持有连接配置，关闭 provider 后仍可使用；模型需按下表释放。
下表保留 Jev 快捷工厂，默认访问 TypeSafe 官方服务。

| 语言 | 创建模型 | 调用 | 能力查询 | 释放 |
|---|---|---|---|---|
| Go | `NewJevDecision(key, "jev-latest")` | `model.Decide(options)` / `DecideContext(ctx, options)` | `model.Capabilities()` | `defer model.Close()` |
| Java | `DecisionModel.jev(key, "jev-latest")` | `model.decide(optionsJson)` | `model.capabilities()` | try-with-resources |
| Kotlin | `DecisionModel.jev(key, "jev-latest")` | `model.decide(optionsJson)` | `model.capabilities()` | `use {}` |
| Swift | `DecisionModel.jev(apiKey: key, modelId: "jev-latest")` | `model.decide(options: optionsJson)` | `model.capabilities()` | `close()` 或 ARC |
| Flutter | `DecisionModel.jev(key, 'jev-latest')` | `model.decide(optionsMap)` / `decideJson(optionsJson)` | `model.capabilities()` | `model.close()` |

Go 的 Decide 接受可 JSON 序列化的对象，Context 取消会停止 native HTTP 请求。
Java/Kotlin/Swift 的调用及能力查询返回 JSON 字符串；Flutter 返回 Map 并提供
原始 JSON 调用。Java/Kotlin/Swift/Flutter 可传原有 C ABI abort handle；0 表示
无显式取消。C ABI 调用同步阻塞，Flutter 可将 HTTP 调用放入 worker isolate。

## C ABI

`aimux_provider_handle_new("openai", key, config_json, &provider_handle)` 和
`aimux_provider_decision_model(provider_handle, model_id, &handle)` 创建 OpenAI
决策模型。不支持决策的 provider 返回 UnsupportedFunctionality。

`aimux_jev_decision_new(key, model_id, endpoint_or_NULL, &handle)` 创建模型，
`aimux_decide(handle, opts_json, &out_json)` 返回 JSON，
`aimux_decide_with_abort` 可传入现有 abort handle。返回错误遵循
[错误模型](../error-model.md)。用 aimux_drop_handle 释放模型，
aimux_free_string 释放结果。`aimux_decision_capabilities(handle, &out_json)`
返回能力 JSON，无网络请求，并遵守相同的字符串生命周期。

显式配置概率来源时可使用 `aimux_jev_decision_new_with_probability_source(key, model_id,
endpoint, probability_source, &handle)`，source 接受上述三个值，NULL 使用
`native`。原有 `aimux_jev_decision_new` 签名保持兼容。未知 source 在模型创建时
返回 InvalidArgument。

## 结果语义

Decision adapter 的支持范围限于原生决策接口和候选评分能力，不提供
structured-output 模拟或生成式 fallback。`model_estimate` 仅用于如实记录
外部服务的概率来源，不表示 aimux 实现了相应模拟能力。

Boolean probability_true 表示 P(true)，调用方设置业务阈值。Choice selected
是 label，Score expected_value 是零起点的浮点位置，可能为小数。
Choice/Score probabilities 的公共类型允许为空；已实现的官方 adapter 对正常答案返回完整分布。
`type: "refusal"` 表示该题拒答，不含概率，不应作为 false 或零分处理。
Jev 的 selected 必须是最大概率选项，允许并列最高，由 adapter 按官方协议校验。
有分布的 Score 必须与概率加权的平均位置一致。provider 在 capabilities 和
结果的 `rounding` 中分别声明 `probability_decimals` 和 `score_decimals`。TypeSafe
声明两者均为 2；其他 provider 若不声明，默认仅容忍浮点运算误差。Core 按
各自精度校验，结果的精度必须匹配 provider 声明，原始数字不会被改写。
probability_source 表示来源，不保证校准；confidence 保留 provider 定义。
raw response、实际执行的 model 和原始 usage 可用于审计。未提供的独立 model_version 保持未知；LocalAI 等服务的 latency_ms 按原生值保留。
TypeSafe 的 usage 对象必须存在；input_tokens/output_tokens 可缺省或为 null，
未报告的数量保留为未知，不影响有效答案。

TypeSafe 官方 Choice 支持 1–255 项，Score 支持 2–10 级；state 支持字符串、
对象或数组。不施加未公布的题目数量、ID 字符或文字长度上限。依据见
[官方 contract](https://docs.typesafe.ai/api)。不支持 decision_model 的
provider 返回 UnsupportedFunctionality。state 不定义媒体 part；图片放在独立 images 字段中。
能力查询在所有绑定均可用，包含题型支持、数量限制、完整分布、图片/typed-choice 支持、概率来源与
舍入精度。查询不会发起 HTTP 请求。

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
`input.options` 保留 state/questions/images 以及协议、编码 options；provider 快照含 endpoint 和 capabilities；
exchanges 记录各重试 attempt 的 HTTP 请求、响应和时延；
`outcome.decision_result` 保存规范化结果。凭据沿用统一脱敏规则。
旧语言录制缺少 operation 时仍按 language model 解析，schema 2 保持兼容。

CLI 离线回放不需要 key：

```sh
aimux-replay ./recordings/recordings.jsonl --mock
aimux-replay ./recordings/recordings.jsonl --dry-run
```

Rust 可用 `aimux_core::replay::MockDecisionReplayModel::from_jsonl` 加载录制，
再通过 `decide` 或 `replay_decision_with_model` 离线回放。匹配 provider/model、
state/questions/images、headers 和 provider_options；timeout/retry 控制不参与匹配。
输入未命中或记录不完整时返回错误。请求回放可通过
`aimux_providers::rebuild_decision_provider` 重建所有上述官方 provider，再调用
`replay_decision_with_model` 发出真实请求；CLI 不带 `--mock` 时也走这条路径。

实测录制在 `aimux-providers/tests/fixtures/jev_systemone_live.jsonl`。
Provider 回归测试通过共享 HTTP replay helper 重放实际 wire 交换；另有
规范化结果回放测试。两者均离线执行。
OpenAI 使用 `aimux-providers/tests/fixtures/openai_decisions.json` 的 synthetic
契约样本，运行时使用 runtime_decisions.json；图片/typed values 共用
contract-tests/fixtures/decision-openai-full.json。均覆盖本地 HTTP、录制→mock replay、
重建 provider→HTTP replay，多次评分保留所有真实 exchange。
