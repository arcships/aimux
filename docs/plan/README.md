# Docs Plan — 交付计划总索引

> 工作流：docs-sprint（develop/verify/merge 循环，任务见 `tasks/`，评审见 `reviews/`，设计来源见 `analysis/`）

## 当前计划

### RFC-0017 阶段 2：退役 RequestBodyOverride + 完全用户定义（in progress）

设计：[RFC-0017 §2.3/§2.5/§3](../../rfc/0017-provider-config-dx.md)
分析：[analysis/stage2-reasoning-map.md](analysis/stage2-reasoning-map.md)
数据来源：[model-config-research/](../../internal/model-config-research/)（调研）

**核心原则（v3 定稿）**：
- aimux 保留：`reasoning` 透传 + `body_overrides`（用户定义一切）+ `max_tokens_key`（修 bug）+ warning（不静默）
- aimux 退役：`RequestBodyOverride` 枚举（含 DeepSeek）、`apply_deepseek_override`、effort 归一化
- 知识放文档（用户手册），机制放代码；零内置厂商映射

| 任务 | 内容 | 依赖 | 状态 |
|---|---|---|---|
| [stage2-001](tasks/stage2-001.md) | 退役 + max_tokens_key + warning（机制层） | — | pending |
| [stage2-002](tasks/stage2-002.md) | 测试套件（退役回归 + 矩阵 + 防死代码对照） | 001 | pending |
| [stage2-003](tasks/stage2-003.md) | 用户手册 + 全量回归 + 文档收尾 | 002 | pending |

依赖图：

```text
stage2-001 ──► stage2-002 ──► stage2-003
```

## 约定

- 任务边界 = 最小可独立验证交付物；`path` 重叠的任务串行执行
- 评审 blocking 定义：与设计文档 contract 不一致，或对接路径残留 stub/mock/fake
- 完成后：任务标 done，非阻塞发现归档 [backlog.md](backlog.md)
