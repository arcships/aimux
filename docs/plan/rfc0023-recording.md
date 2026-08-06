# RFC-0023 录制与回放 — 方案对齐记录

> 本文件是 #48(RFC-0023)的**对齐追踪入口**:先对齐方案,再实施。
> 关联:issue [#48](https://github.com/arcships/aimux/issues/48)、[RFC-0023](../../rfc/0023-runtime-request-recording.md)(DRAFT)
> 状态:2026-08-06 开始对齐(P1-P3 曾实施后回滚,代码在 `backup/rfc0023-p1-p3` 分支,仅作参考)

## 对齐流程

```
1. 对齐前置依赖与范围    ← 进行中
2. 评审 RFC-0023 DRAFT    ← 待做
3. 逐项确认设计决策       ← 待做
4. 对齐 RFC-0015 sink     ← 待做
5. 锁定实施草案 → 再实施
```

## 已对齐结论

### D1. P4 请求回放与 RFC-0020 解耦(2026-08-06)

**问题**:RFC 原标注 P4 依赖 RFC-0020(外部 provider 配置),经对齐确认该依赖不成立:

- RFC-0020 只覆盖 **OpenAI 兼容协议**(其 Non-Goal #1:原生协议 provider 是代码实现,无法用配置数据描述)——即使落地也重建不了 anthropic/google/bedrock 等。
- 原依赖的实质是"按数据构造 provider 的统一入口",而非"外部配置覆盖层"——两个正交机制。

**对齐设计**:`replay_request(recording, model: Option<&dyn LanguageModel>, api_key: Option<&str>, overrides)`

- 自动重建优先:openai 兼容族 + `env:VAR`/`none` 来源可重建;
  `explicit` 来源调用方补 `api_key`。
- 手动兜底:原生协议或无构造入口 → 调用方传 `model` 实例(回放框架本身 provider 无关)。
- 已同步修订 RFC-0023 §3.6.1 与 §11 P4 行。

## 待对齐项

| # | 项 | 内容 | 状态 |
|---|---|---|---|
| A1 | 前置依赖梳理 | RFC-0015 ✅(sink 对齐待 C1)/ RFC-0014 ✅ / RFC-0003 判定为参考不依赖 / RFC-0020 已解耦 / RFC-0021/22 不阻塞 | ✅ 已对齐 |
| A2 | RFC 评审 | RFC-0023 仍 DRAFT,需评审定稿(或至少锁定实施草案) | ⏳ 待做 |
| B1 | ConfigSnapshot 形式 | RFC 原文:独立可选 trait;回滚前实施:LanguageModel 默认方法。**取舍?** | ⏳ 待对齐 |
| B2 | 落盘线程模型 | RFC:tokio 后台 task;回滚前实施:std::thread + mpsc(不引入 tokio 正式依赖)。**取舍?** | ⏳ 待对齐 |
| B3 | 门控 API | RFC:OnceLock(init 一次);回滚前实施:RwLock(支持替换/关闭/测试隔离)。**取舍?** | ⏳ 待对齐 |
| B4 | 流式 outcome | RFC:outcome 是最终摘要;回滚前实施:建流即记 success,finish/usage 留空。**是否等流结束补全?** | ⏳ 待对齐 |
| C1 | RFC-0015 sink 对齐 | RFC §6:共享 ring-store 模式,先实施方预留合并口子(RingTraceStore 已实施)。**RingRecorder 抽象怎么对齐?** | ⏳ 待对齐 |
| D2 | P5 绑定层范围 | 哪些语言先做(Node/C ABI/Python/Go/…) + PrefixMatcher | ⏳ 待对齐 |
| D3 | P6 RingRecorder | 可选,是否纳入本期 | ⏳ 待对齐 |

## 参考

- 回滚前的 P1-P3 实施代码:分支 `backup/rfc0023-p1-p3`(3 个提交,仅作方案参考,不直接复用)
