# Decision API

`decide` 对同一个文本或 JSON state 回答多道 typed questions，结果按问题 ID
索引。当前接入 TypeSafe 官方 Jev System One；[RFC-0037](../../rfc/0037-decision-api.md)
记录 contract 和后续范围。

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

## 官方结构化题目字段

instructions、Choice description 和 Score levels 的每一级均支持字符串、对象、
数组。Boolean 可提供 `criteria.true` / `criteria.false` 描述。对象/数组以原生
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

五种绑定复用相同请求/结果 JSON contract，默认均访问 TypeSafe 官方服务。
模型支持能力查询、可选 endpoint 和概率来源配置，以及原有错误传输。

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
Choice/Score probabilities 可为空，Jev adapter 会返回完整分布。
Jev 的 selected 必须是最大概率选项，允许并列最高，由 adapter 按官方协议校验。
有分布的 Score 必须与概率加权的平均位置一致。provider 在 capabilities 和
结果的 `rounding` 中分别声明 `probability_decimals` 和 `score_decimals`。TypeSafe
声明两者均为 2；其他 provider 若不声明，默认仅容忍浮点运算误差。Core 按
各自精度校验，结果的精度必须匹配 provider 声明，原始数字不会被改写。
probability_source 表示来源，不保证校准；confidence 保留 provider 定义。
raw response、实际执行的 model 和原始 usage 可用于审计。官方响应不含独立
model_version 或 latency_ms，这两个可选字段留空。
TypeSafe 的 usage 对象必须存在；input_tokens/output_tokens 可缺省或为 null，
未报告的数量保留为未知，不影响有效答案。

TypeSafe 官方 Choice 支持 1–255 项，Score 支持 2–10 级；state 支持字符串、
对象或数组。不施加未公布的题目数量、ID 字符或文字长度上限。依据见
[官方 contract](https://docs.typesafe.ai/api)。不支持 decision_model 的
provider 返回 UnsupportedFunctionality。当前 state 不定义图像或其他媒体 part。
能力查询在所有绑定均可用，包含题型支持、数量限制、完整分布、概率来源与
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
`input.options` 保留 state/questions；provider 快照含 endpoint 和 capabilities；
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
state/questions、headers 和 provider_options；timeout/retry 控制不参与匹配。
输入未命中或记录不完整时返回错误。请求回放可通过
`aimux_providers::rebuild_decision_provider` 重建官方 Jev，再调用
`replay_decision_with_model` 发出真实请求；CLI 不带 `--mock` 时也走这条路径。

实测录制在 `aimux-providers/tests/fixtures/jev_systemone_live.jsonl`。
Provider 回归测试通过共享 HTTP replay helper 重放实际 wire 交换；另有
规范化结果回放测试。两者均离线执行。
