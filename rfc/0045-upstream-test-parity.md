# RFC-0045：上游测试迁移策略

> **Status**: Draft
>
> **Date**: 2026-10-19
>
> **Baseline**: master `5743a40a`。上游测试按 `fixtures/aisdk/VERSIONS.json` 锁定的各包版本取（`ai` 7.0.127、`@ai-sdk/provider-utils` 5.0.55、`@ai-sdk/openai` 4.0.83、`@ai-sdk/anthropic` 4.0.71 等）。
>
> **Tracks**: [#278](https://github.com/arcships/aimux/issues/278)。未实现功能归 [#275](https://github.com/arcships/aimux/issues/275)、[#276](https://github.com/arcships/aimux/issues/276)。
>
> **Related**: [对齐文档](../docs/aisdk-architecture-alignment.md) §0.4 Q1（单步 core、不执行宿主工具）、§5（已登记差异）、§9.3（行为验收矩阵）；[RFC-0036](0036-positioning-and-layered-architecture.md) §9.3 协议差分；[RFC-0046](0046-provider-layout-and-reuse.md) §3.4（`layout.toml`）、§4（测试位置）；[RFC-0044](0044-crate-layering-and-provider-layout.md) §5（provider-utils 与 aimux 的目录）；`.agents/skills/ts-rust-port`。

## 1. 决策摘要

1. **测试跟着代码走。** 只迁 aimux 已经实现的模块对应的上游测试文件。没实现的功能，它的测试不迁；将来实现那个功能的 PR 连同测试一起带进来。
2. **选了文件再选用例。** 选中的文件里，属于固定排除类别的用例不迁：单步 core 不做的多步、工具执行与审批；还没做的遥测和生命周期回调；TS 专属的实现细节；已登记的差异；本模块没实现的子功能。排除只写类别，不逐条写理由。
3. **按行为选证据形式。** 跨边界的行为（请求、响应、流事件、错误分类）优先用差分 fixture，一个场景覆盖多条上游用例；纯转换函数（prompt 转换、工具准备、工具参数解析）逐条搬上游用例。
4. **位置按 RFC-0046 §4**，文件状态记在 RFC-0046 §3.4 的 `layout.toml`，每个搬来的测试带一行上游出处标记。
5. **不设全局清零目标，也不设棘轮。** 完成标准是"选中的每个文件迁过一遍"：文件里每个用例要么有对应测试，要么属于一个排除类别。

## 2. 现状

2026-10-19 的核对：

- 17 个对齐的厂商包加 `ai/` 在范围内的子目录，共 409 个上游测试文件、7784 个用例。
- 按用例名能在 aimux 里找到对应的只有 1412 个。

逐个判断 409 个文件对应的模块 aimux 是否实现，结果如下：

| 模块状态 | 文件 | 用例 | 已有对应 |
|---|---:|---:|---:|
| 已实现 | 211 | 4502 | 1277 |
| 部分实现（文件里有些子功能没做） | 53 | 1672 | 96 |
| 没实现 | 41 | 642 | 23 |
| 不适用（Batch、Realtime、翻译、v2/v3 适配器、宿主工具执行等已登记不做的，以及 TS 专属模块） | 104 | 968 | 16 |

所以真正该迁的是前两行，6174 个用例。再按第 3.2 节去掉排除类别，估计剩 4500 到 5000 个，其中约 1400 个已有对应，**要补的大约 3000 到 3500 个**。这个数是估计；第 3.2 节的规则落到每个文件后才有准数。

缺口最大的是 core：

- `generate-text.test.ts` 和 `stream-text.test.ts` 合计 743 个用例，对上 38 个。
- 按用例名粗分，其中约 400 个测多步、工具执行与审批，单步 core 不做（Q1）。
- 约 80 个测遥测和回调，还没做。
- 剩下约 250 个是真正要补的：文本、推理、来源、文件、工具调用、错误、中止、结构化输出、工具选择检查。

## 3. 迁移策略

### 3.1 选文件

每个上游测试文件对应一个上游源文件（`foo.test.ts` 测 `foo.ts`）。RFC-0046 §3.4 的 `layout.toml` 逐个列上游源文件，`status` 字段已经说明它在 aimux 里的去向（provider-utils 和 aimux 照同样格式各有一张）。选文件直接按 `status`：

| `status` | 测试怎么办 |
|---|---|
| `mapped`、`elsewhere` | 迁；`tests` 写 `"port"`，有排除时写成第 3.2 节的表 |
| `missing` | 不迁；条目上已有 issue，功能实现的 PR 把状态改成 `mapped` 并迁测试 |
| `not-applicable` | 不迁；条目上写登记位置或理由 |

所以本 RFC 只在 `layout.toml` 上加一个 `tests` 字段，用来放第 3.2 节的排除清单，不另建台账。

### 3.2 选用例

`port` 文件里的用例，属于下列类别之一的不迁。类别是固定的，新增类别要改本 RFC：

| 类别 | 依据 | 例子 |
|---|---|---|
| `single-step` | 对齐文档 Q1、D25 | `stopWhen`、`prepareStep`、第二步、宿主工具执行、工具审批流转 |
| `telemetry` | 对齐文档 §9.1 已知差距，等可观测性 RFC | `onStart`/`onFinish`/`onStepFinish`、telemetry integration |
| `ts-only` | 没有 Rust 对应物 | zod 转换细节、`Object.prototype` 防护、JS stream 管道、类型推断 |
| `registered` | 对齐文档 §5 的登记差异 | 跨域重定向去头、header 大小写合并 |
| `sub-feature` | 本模块没做的子功能，要写 issue | `generateImage` 按上限拆多次调用 |

排除记录在 `layout.toml` 该条目的 `tests` 下，按类别列出用例名，例如 `tests = { port = true, single-step = ["…"], telemetry = ["…"] }`。`sub-feature` 的功能将来实现时，用例从排除里移出并补上测试。

### 3.3 证据形式

- **差分 fixture 优先**：请求 URL、headers、body，响应解析，流事件顺序，错误分类。上游的语言模型测试文件里大量用例只是请求体的变体，一个 fixture 场景可以同时覆盖几条，在 case 上用 `upstream: [...]` 列出覆盖了哪些。
- **逐条搬**：纯函数和转换逻辑，比如 prompt 转换、`prepare-tools`、`parse-tool-call`、`detect-media-type`、JSON 修复。这类用例小而多，适合批量交给 haiku。
- 两者都可以时选差分：它比较的是上游真实行为，不是我们对上游测试的转述。

## 4. 位置与标记

位置按 RFC-0046 §4（provider-utils 和 aimux 按 RFC-0044 §5 的目录照同样规则）：

- 单元测试紧挨源文件，`foo_test.rs` 对应 `foo.test.ts`；
- 函数名用上游 `it()` 描述的 snake_case；
- 跨厂商场景进 `tests/it/main.rs`。

snake_case 会丢掉 `describe` 链，同名用例也会撞，所以每个测试上方再写一行完整标记：

```rust
/// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › doGenerate › should send request body with include and tool
#[tokio::test]
async fn should_send_request_body_with_include_and_tool() { … }
```

标记不写版本，版本以 VERSIONS.json 为准。差分 fixture 覆盖的用例写在 `cases/<包>.mjs` 的 `upstream` 字段里。现有约 85 个文件的自由格式出处注释，在 RFC-0046 搬移测试时一并改成标记。

## 5. 移植方法

1. 守 `ts-rust-port` 的纪律：逐条搬，只换 harness，不改场景。
2. harness 对应：上游 mock fetch 对应本地 HTTP 桩，上游 `MockLanguageModelV4` 对应 `ScriptedModel`。共用 helper 放 RFC-0046 §4 的 `src/test_support/`。
3. 快照：`toMatchInlineSnapshot` 写成内联 `serde_json::json!`；`__snapshots__/` 文件快照放厂商目录的 `snapshots/`。不引入快照库。
4. 发现行为和上游不一致：先写失败的测试，再按 `ts-rust-port` 的 `fix` 流程在同一 PR 里修，单独一个提交；属于刻意差异的，登记到对齐文档 §5，用例归 `registered`。不允许 `#[ignore]`。
5. PR 描述写"这个文件选中 N 个用例：搬 a、差分 b、排除 c（按类别）"。

## 6. 差分 fixture 扩展

录制脚本现成（`scripts/aisdk-fixtures/record.mjs`），只补 case：

1. **厂商包**：给还没有 fixture 的 amazon-bedrock、google-vertex、xai、azure、groq、mistral、deepseek、cohere、open-responses、anthropic-aws 补场景。每家覆盖非流式文本、流式文本、工具调用（流与非流）、结构化输出、错误分类和各家特有的 provider 工具。
2. **core**：新增 `cases/ai-core.mjs`，用上游 `MockLanguageModelV4` 按脚本返回，录 `streamText` 的完整事件流和 `generateText` 的结果；Rust 用同一份脚本驱动 `ScriptedModel` 比对。只录单步场景。

## 7. CI 校验

一个 job，跑 `scripts/aisdk-tests/check.mjs`：

1. 每个标记都指向锁定版本里真实存在的上游用例（改名、删除要同步）；
2. `layout.toml` 的排除只用第 3.2 节的类别，`sub-feature` 带 issue；
3. 打印每个 `port` 文件的进度（已迁、已排除、未处理）到 job summary，只报告，不判失败；
4. 测试数量前后不减少，用 RFC-0046 §4 的 `cargo test -- --list` 对照。

升级上游版本时，`check.mjs diff` 列出 `port` 文件里新增的用例，由升级 PR 决定迁还是排除。

## 8. 分批

| 批次 | 内容 |
|---|---|
| 0 | `layout.toml`（随 RFC-0046 建立）的 `tests` 字段填好；现有测试改标记（随 RFC-0046 测试搬移）；`check.mjs` 上线 |
| 1 | core：`generate-text`、`stream-text` 约 250 个要补的用例，加 core 差分 fixture |
| 2 | 纯转换函数：prompt、`prepare-tools`、`parse-tool-call`、provider-utils 小文件 |
| 3 | 主力厂商语言模型：Anthropic、OpenAI（Responses、chat）、Google、Bedrock，差分优先 |
| 4 | 其余厂商包的差分 fixture 和剩余用例 |

每批拆成多个独立 PR，不堆叠。

## 9. 完成标准

- 409 个上游测试文件对应的源文件在 `layout.toml` 里都有 `status`，`mapped` 和 `elsewhere` 的都有 `tests`；
- 每个 `port` 文件里没有"未处理"的用例；
- 差分 fixture 覆盖每个对齐的厂商包和 core 的单步场景；
- 对齐文档 §9.3 行为验收矩阵每一格至少有一个带标记的测试或 fixture。

## 10. 开放问题

1. **`telemetry` 类别何时取消。** 可观测性 RFC（RFC-0044 拆出的后续）落地后，这一类用例要迁。到时是改本 RFC 还是由那个 RFC 负责，倾向后者。
2. **差分覆盖与逐条搬是否二选一。** 本稿允许一个用例只由差分 fixture 覆盖，不再搬单测。
3. **上游快照里未注入的随机值。** 比较时归一化，不改 aimux 的 id 生成。
