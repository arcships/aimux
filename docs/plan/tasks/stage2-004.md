---
id: stage2-004
scope: aimux-providers/src/openai_compat_registry.rs
status: pending
depends-on: []
---

# stage2-004: registry base_url P0 修复（7 处确定性错误，backlog B5）

## objective

修复 `openai_compat_registry.rs` 中 7 处**确定性** base_url 错误（调研发现 + 已逐行验证，用户请求必然失败）。每处修复附证据来源（调研 batch 文件 + 证据等级）。

## 修复清单（证据来自 docs/internal/model-config-research/）

| 行 | 厂商 | 现状（错误） | 修正为 | 证据 |
|---|---|---|---|---|
| 1533 | opencode | `"opencode_zen.rs"`（非法 URL 字面量） | `https://api.opencode.zen/v1` | batch-05（B 级：reference/opencode providers.mdx） |
| 759 | firepass | `"https://api.fireworks.ai/inference/v1（OpenAI"`（乱码尾缀） | `https://api.fireworks.ai/inference/v1` | batch-02（与 fireworks 同端点，乱码截断） |
| 777 | freemodel | `"client.chat.completions.create"`（SDK 代码片段） | `https://api.freemodel.dev/v1` ⚠️ **推断值**（batch-03 未确认，修复后标注待实测） | batch-03（C 级官网） |
| 939 | iflowcn | `"https://apis.iflow.cn/v1（chat"`（乱码尾缀） | `https://apis.iflow.cn/v1` | batch-03（C 级：platform.iflow.cn） |
| 840 | gmi | `"https://api.gmi-serving.com/v1（与"`（乱码尾缀） | `https://api.gmi-serving.com/v1` | batch-03（C 级：docs.gmicloud.ai） |
| 2127 | volc_engine | `https://ark.cn-beijing.volces.com`（缺路径） | `https://ark.cn-beijing.volces.com/api/v3` | batch-06（B 级：simple-one-api/uni-api） |
| 2253 | zhipu_v4 | `https://open.bigmodel.cn`（缺路径） | `https://open.bigmodel.cn/api/paas/v4` | batch-06（C 级：docs.bigmodel.cn） |

## 约束

1. 只改 `base_url` 字符串字面量，**不动** display/env_var/profile/其他任何字段
2. 公共类型 `XxxConfig`/`XxxProvider` 签名不变（RFC-0012）
3. freemodel 是推断值：在条目行内或模块注释标注 `⚠️ freemodel base_url 为推断值，待实测确认`（batch-03）
4. 存疑 ~20 家（novita/nous_research/longcat/moonshotai_cn 等）**本次不修**（无确定性证据，留待实测）

## context

- 数据：[docs/internal/model-config-research/_global_table.md](../internal/model-config-research/_global_table.md) P0 节 + batch-02/03/05/06 对应条目
- 现有：[openai_compat_registry.rs](D:\code\aimux\aimux-providers\src\openai_compat_registry.rs)

## path

- `aimux-providers/src/openai_compat_registry.rs`（仅 7 处 base_url 字面量）

## verification

1. `cargo check -p aimux-providers` 通过
2. `cargo test -p aimux-providers --tests` 全绿（无测试断言旧 base_url；conformance 用 mock server 不受影响）
3. git diff 确认**仅** 7 处字符串变化（+freemodel 标注注释）
4. 防死代码对照：每处修复对照本任务证据表（评审核对）
