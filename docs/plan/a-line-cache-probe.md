# A 线:缓存探测链 实施追踪(RFC-0015 / 0024 / 0025)

> 本文件是 A 线(缓存探测链)的实施追踪入口,关联 issue:
> - [#49](https://github.com/arcships/aimux/issues/49) — RFC-0024 调用会话聚合(session_id 归组)
> - [#36](https://github.com/arcships/aimux/issues/36) — RFC-0015 缓存命中探测(core 探测基础设施,三层拆分①)
> - [#52](https://github.com/arcships/aimux/issues/52) — RFC-0025 aimux-cli 缓存探测 client(独立产物)

## 依赖链

```
#49 (RFC-0024 session_id 归组)
  └─> #36 (RFC-0015 core 探测:TraceRecord.session_id 取自 CallOptions.session_id)
        └─> #52 (RFC-0025 CLI:消费 RFC-0015 查询接口与 jsonl)
```

- #49 独立、无 trait 改动、无破坏性变更,先行落地
- #36 复用 #49 的 session_id 字段做链级聚合;原型在 `docs/internal/cache-tracing/prototype/`(12/12 测试绿),迁移进 core
- #52 依赖 #36 的查询接口与 `export_jsonl`,最后实施

## 协同

- 与录制(#48 / RFC-0023)共享 ring-store/sink 抽象,先实施方预留合并口子
- 与回放(#48)配合做缓存可复现性验证

## 状态

| issue | 状态 | 备注 |
|---|---|---|
| #49 | ✅ 完成 | 2026-08-05 落地(P1/P2/P5);gpt-sol 独立审核 APPROVE-WITH-NITS;P3/P4 待依赖 RFC-0023/0015 |
| #36 | ✅ 完成 | 2026-08-05 落地(探测本身进 core);gpt-sol 三轮审核 APPROVE;P3 待依赖 RFC-0023 |
| #52 | ✅ 完成 | 2026-08-05 落地(offline/session/provider);glm-5.2 三轮审核 APPROVE |
