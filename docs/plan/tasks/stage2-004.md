---
id: stage2-004
scope: bindings/node + workspace
status: pending
depends-on: [stage2-002, stage2-003]
---

# stage2-004: 集成/E2E + 全量回归 + 文档收尾

## objective

打通 Node 侧端到端链路并全量回归（analysis §6 I3 真实路径）：

1. **Node E2E**（`bindings/node/__test__/` 追加）：
   - `openai(key, model, { reasoningMap: { off: {...} } })` → `generateText(..., { reasoning: 'none' })` → mock server 捕获请求体 → 断言 off 字段注入
   - `maxTokensKey` 透出 → 请求体 key 断言
   - reasoningMap 未配置 + `reasoning: 'none'` → 结果带 warning（非静默）
2. **全量回归**：`cargo test --workspace --no-fail-fast` 全绿 + Node 测试全绿
3. **文档收尾**：RFC-0017 阶段 2 验收清单勾选；README 计划状态更新；stage2-002 防死代码对照表复核通过

## context

- 设计：[docs/plan/analysis/stage2-reasoning-map.md](../analysis/stage2-reasoning-map.md) §6 I3、§7.1
- RFC：[rfc/0017-provider-config-dx.md](../../../rfc/0017-provider-config-dx.md) 阶段 2
- 现有：[bindings/node/__test__/](D:\code\aimux\bindings/node\__test__)

## path

- `bindings/node/__test__/`（E2E 追加）
- `docs/plan/README.md`、`rfc/0017-provider-config-dx.md`（状态行）

## verification

1. Node E2E 3 场景全绿
2. `cargo test --workspace --no-fail-fast` 全绿
3. RFC-0017 阶段 2 验收清单全部勾选
