# Decision API

`decide` 对同一个文本或 JSON state 回答多道 typed questions，结果按问题 ID
索引。当前实现 Jev 托管 System One；[RFC-0037](../../rfc/0037-decision-api.md)
记录 contract 和后续范围。

## Rust

```rust,no_run
use aimux_core::prelude::*;
use aimux_providers::{JevConfig, JevProvider};
use serde_json::json;

# async fn example() -> Result<(), AiMuxError> {
let provider = JevProvider::new(JevConfig::from_env()?);
let model = provider.decision_model("jev-1.13")?;
let request = DecisionCallOptions::new(json!({"message": "Please refund the duplicate charge"}), vec![
    DecisionQuestion::Boolean {
        id: "refund".into(), instructions: "Is a refund requested?".into(),
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
import { jevDecision, decide } from '@arcships/aimux'

const model = await jevDecision(process.env.JEV_API_KEY!, 'jev-1.13')
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
console.log(result.answers)
```

factory 的第三个 `endpoint` 参数可覆盖完整 POST URL。
第四个 `probabilitySource` 参数接受 `native`、`logit_scoring` 或
`model_estimate`，缺省为 `native`。对自建服务，应按其实际实现声明来源：

```ts
const local = await jevDecision('local-key', 'local-model',
  'http://127.0.0.1:8000/v1/systemone', 'logit_scoring')
```

`decide` 的第三个参数接受 AbortSignal。timeout 复用已有毫秒配置。

## Python

```python
import os
from aimux import jev_decision, decide

model = jev_decision(os.environ['JEV_API_KEY'], 'jev-1.13')
result = decide(model, {'message': 'Please refund the duplicate charge'}, [
    {'id': 'refund', 'type': 'boolean', 'instructions': 'Is a refund requested?'}
], max_retries=0)
print(result['answers']['refund']['probability_true'])
```

自建服务可传第四个参数或 `probability_source='logit_scoring'` / `'model_estimate'`；
缺省为 `'native'`，结果保留调用方声明的来源。

## C ABI

`aimux_jev_decision_new(key, model_id, endpoint_or_NULL, &handle)` 创建模型，
`aimux_decide(handle, opts_json, &out_json)` 返回 JSON，
`aimux_decide_with_abort` 可传入现有 abort handle。返回错误遵循
[错误模型](../error-model.md)。用 aimux_drop_handle 释放模型，
aimux_free_string 释放结果。

自建服务可使用 `aimux_jev_decision_new_with_probability_source(key, model_id,
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
有分布的 Score 必须与概率加权的平均位置一致；校验允许概率及分数各自保留
两位小数带来的误差，拒绝与分布明显矛盾的分数。原始数字不会被改写。
probability_source 表示来源，不保证校准；confidence 保留 provider 定义。
raw response、model_version 和原始 usage 可用于审计。

Jev 最多 20 题，Choice 2–24 项，Score 2–10 级。具体文字长度与 ID 限制见
[官方 contract](https://jev-ai.org/docs/decisions/)。不支持 decision_model 的
provider 返回 UnsupportedFunctionality。当前 state 不定义图像或其他媒体 part。
