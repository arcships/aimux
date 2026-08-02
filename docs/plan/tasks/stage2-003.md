---
id: stage2-003
scope: docs + workspace
status: pending
depends-on: [stage2-002]
---

# stage2-003: 用户手册 + 全量回归 + 文档收尾

## objective

1. **用户手册**（新文件 `docs/provider-config-manual.md`）：调研成果 → 用户可读的配置指南：
   - 每厂商"关思考/字段差异"的 `bodyOverrides` 配置示例（来源标注，含核实日期）
   - DeepSeek V4 官方机制详解（thinking:{type} + reasoning_effort 三档 + xhigh 映射表，2026-08 核实）
   - GLM/Qwen/Kimi/MiniMax/方舟等（来源：调研 batch 文件）
   - `reasoning` 7 档透传说明（OpenAI/DeepSeek V4 支持 xhigh；不支持的厂商自决）
   - 迁移说明：旧版 DeepSeek `reasoning:'none'` 用户 → bodyOverrides
2. **全量回归**：`cargo test --workspace --no-fail-fast` 全绿
3. **绑定层零变化确认**：git diff 确认 bindings/、aimux-ffi/ 无改动
4. **文档收尾**：RFC-0017 阶段 2 验收勾选；docs/plan/README.md 状态更新

## context

- 设计：[docs/plan/analysis/stage2-reasoning-map.md](../analysis/stage2-reasoning-map.md) §2.2、§5、§7
- 数据：[model-config-research/](../../internal/model-config-research/)（batch-01~06 + _global_table.md）
- RFC：[rfc/0017-provider-config-dx.md](../../../rfc/0017-provider-config-dx.md) §5（用户手册参考表）

## path

- `docs/provider-config-manual.md`（新）
- `docs/plan/README.md`、`rfc/0017-provider-config-dx.md`（状态行）

## verification

1. `cargo test --workspace --no-fail-fast` 全绿
2. 手册覆盖调研 §3 全部思考机制厂商（每厂商示例 + 来源 + 日期）
3. 绑定层零改动
4. RFC-0017 阶段 2 验收清单勾选
