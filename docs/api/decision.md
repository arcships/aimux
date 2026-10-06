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

## C ABI

`aimux_jev_decision_new(key, model_id, endpoint_or_NULL, &handle)` 创建模型，
`aimux_decide(handle, opts_json, &out_json)` 返回 JSON，
`aimux_decide_with_abort` 可传入现有 abort handle。返回错误遵循
[错误模型](../error-model.md)。用 aimux_drop_handle 释放模型，
aimux_free_string 释放结果。

## 结果语义

Boolean probability_true 表示 P(true)，调用方设置业务阈值。Choice selected
是 label，Score expected_value 是零起点的浮点位置，可能为小数。
Choice/Score probabilities 可为空，Jev adapter 会返回完整分布。
probability_source 表示来源，不保证校准；confidence 保留 provider 定义。
raw response、model_version 和原始 usage 可用于审计。

Jev 最多 20 题，Choice 2–24 项，Score 2–10 级。具体文字长度与 ID 限制见
[官方 contract](https://jev-ai.org/docs/decisions/)。不支持 decision_model 的
provider 返回 UnsupportedFunctionality。当前 state 不定义图像或其他媒体 part。
