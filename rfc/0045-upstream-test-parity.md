# RFC-0045：上游测试迁移策略

> **Status**: Draft
>
> **Date**: 2026-10-19
>
> **Baseline**: master `5743a40a`。上游测试取 `fixtures/aisdk/VERSIONS.json` 锁定的 18 个包，每个包从 vercel/ai 的 git tag `<包名>@<版本>` 取（发布包不带测试）：`ai` 7.0.127、`@ai-sdk/provider-utils` 5.0.55、`@ai-sdk/openai` 4.0.83、`@ai-sdk/anthropic` 4.0.71 等。对齐文档按别的版本对齐的模块例外，见第 3.1 节。
>
> **Tracks**: [#278](https://github.com/arcships/aimux/issues/278)。未实现功能归 [#275](https://github.com/arcships/aimux/issues/275)、[#276](https://github.com/arcships/aimux/issues/276)。
>
> **Related**: [对齐文档](../docs/aisdk-architecture-alignment.md) §0.4 Q1（单步 core、不执行宿主工具）、§5（已登记差异）、§9.3（行为验收矩阵）；[RFC-0036](0036-positioning-and-layered-architecture.md) §9.3 协议差分；[RFC-0046](0046-provider-protocol-abstraction.md) §4.4（`layout.toml`）、§5（测试位置）；[RFC-0044](0044-crate-layering-and-dependency-inversion.md) §3.2、§3.3（aimux 的目录与测试归属）；`.agents/skills/ts-rust-port`。

## 1. 决策摘要

1. **测试跟着代码走。** 只迁 aimux 已经实现的模块对应的上游测试文件。没实现的功能，它的测试不迁；将来实现那个功能的 PR 连同测试一起带进来。
2. **选了文件再选用例。** 选中的文件里，属于固定排除类别的用例不迁：单步 core 不做的多步、工具执行与审批；还没做的遥测和生命周期回调；TS 专属的实现细节；已登记的差异；本模块没实现的子功能。前两类由脚本按 API 名自动判定，后三类每条手写一行理由。
3. **按行为选证据形式。** 跨边界的行为（请求、响应、流事件、错误分类）优先用差分 fixture，一个场景覆盖多条上游用例；纯转换函数（prompt 转换、工具准备、工具参数解析）逐条搬上游用例。
4. **位置按 RFC-0046 §5**，文件状态记在 RFC-0046 §4.4 的 `layout.toml`。每个搬来的测试带一行上游出处标记，标记写完整的 `describe` 链，每层 `describe` 对应一个 Rust `mod`。
5. **不设全局清零目标，也不设只升不降的限制**，CI 不比较前后的数量。完成标准是"选中的每个文件迁过一遍"：文件里每个用例要么有对应测试，要么被自动规则排除，要么带理由手写排除。

## 2. 现状

统计范围：`VERSIONS.json` 的 18 个包，每个包在自己的 tag 下 `packages/<目录>/` 里的全部 `*.test.ts`、`*.test.tsx`，不含 `*.test-d.ts` 类型测试。一个用例是一次 `it(…)` 或 `test(…)` 调用，包括 `.skip`、`.only`、`.todo` 和 `.each`，`.each` 的表只算一次。

统计脚本是 `scripts/aisdk-tests/inventory.mjs`，在 vercel/ai 的 blobless clone 上运行：

```sh
git clone --filter=blob:none https://github.com/vercel/ai ../vercel-ai
node scripts/aisdk-tests/inventory.mjs --repo ../vercel-ai --json target/aisdk-tests.json
```

它按包打印文件数和用例数，`--json` 写出每个用例的行号和 `describe` 链。2026-10-19 的结果是 445 个文件、8810 个用例，其中 `ai` 165 个文件、3451 个用例。

> 初稿写的 409 个文件、7784 个用例，取的是 `ai@7.0.127` 这一个提交。那个提交里的厂商包比锁定版本新（google-vertex 是 5.0.101，锁定 5.0.98；amazon-bedrock 是 5.0.105，锁定 5.0.100），而且没算 `ai/` 的 ui、ui-message-stream、agent、realtime 等目录，所以和按锁定 tag 统计的数对不上。下表按锁定 tag 重算。

逐个判断 445 个文件对应的模块 aimux 是否实现。"已有对应"按用例名匹配，规则同 #278：

| 模块状态 | 文件 | 用例 | 已有对应 |
|---|---:|---:|---:|
| 已实现 | 208 | 4643 | 1327 |
| 部分实现（文件里有些子功能没做） | 52 | 1692 | 104 |
| 没实现 | 40 | 652 | 23 |
| 不适用 | 145 | 1823 | 21 |

"不适用"包括已登记不做的 Batch、Realtime、翻译、v2/v3 适配器、宿主工具执行（`ai/agent`），TS 专属模块和测试 helper，以及 `ai/` 的 ui 和 ui-message-stream（24 个文件、505 个用例）。aimux 没有 UI 层，这一条在 `layout.toml` 的 `not-applicable` 条目上写理由。

所以真正该迁的是前两行：260 个文件、6335 个用例，其中 1431 个已有对应。再按第 3.2 节去掉排除类别，**要补的大约 3000 到 3500 个**。这个数是估计；第 3.2 节的规则落到每个文件后才有准数。

缺口最大的是 core。`generate-text.test.ts` 和 `stream-text.test.ts` 合计 746 个用例，按用例名对上 26 个。用第 3.2 节的自动规则：

- 310 个是 single-step；
- 87 个是 telemetry；
- 剩下 349 个。其中还有一些多步场景规则没抓到，例如 `programmatic tool calling › 5 steps: …`，要手写排除并带理由。

所以 core 要补的在 250 到 350 个之间，内容是文本、推理、来源、文件、工具调用、错误、中止、结构化输出和工具选择检查。

## 3. 迁移策略

### 3.1 选文件

每个上游测试文件对应一个上游源文件（`foo.test.ts` 测 `foo.ts`）。RFC-0046 §4.4 的 `layout.toml` 逐个列上游源文件，`status` 字段已经说明它在 aimux 里的去向（provider-utils 和 aimux 照同样格式各有一张）。选文件直接按 `status`：

| `status` | 测试怎么办 |
|---|---|
| `mapped`、`elsewhere` | 迁；条目必须有 `tests` 表（第 3.2 节） |
| `missing` | 不迁；条目上已有 issue，功能实现的 PR 把状态改成 `mapped` 并迁测试 |
| `not-applicable` | 不迁；条目上写登记位置或理由 |

所以本 RFC 只在 `layout.toml` 的条目上加两个字段，不另建台账：

- `tests`：手写的排除，格式见第 3.2 节；
- `version`：只在这个模块按另一个版本对齐时写。

**版本。** 条目默认用 `VERSIONS.json` 里该包的版本，测试从这个版本的 tag 取。对齐文档按更高版本对齐的模块，条目写 `version`，源文件和测试都从那个 tag 取。现在只有一处：`decide` 按 `ai@7.0.128 src/decide/` 对齐（对齐文档 §3.1），而锁定的 `ai@7.0.127` 只有 `src/evaluate/`。处理如下：

- `decide.ts` 的条目写 `version = "7.0.128"`，迁 `decide.test.ts` 的 20 个用例；
- `deprecated-aliases.test.ts` 测 deprecated 的 evaluation 别名，对齐文档 §3.1 写明不移植（D31），是 `not-applicable`；
- 7.0.127 的 `src/evaluate/` 条目也是 `not-applicable`，理由相同。

`check.mjs` 要求 `version` 不低于锁定版本，锁定版本升到它以后删掉这个字段。

### 3.2 选用例

选中文件里的用例，属于下列类别之一的不迁。类别是固定的，新增类别要改本 RFC：

| 类别 | 依据 | 判定 |
|---|---|---|
| `single-step` | 对齐文档 Q1、D25 | 自动 |
| `telemetry` | 对齐文档 §9.1 已知差距，等可观测性 RFC | 自动 |
| `ts-only` | 没有 Rust 对应物：zod 转换细节、`Object.prototype` 防护、JS stream 管道、类型推断 | 手写，带理由 |
| `registered` | 对齐文档 §5 的登记差异，例如跨域重定向去头、header 大小写合并 | 手写，带理由和登记位置 |
| `sub-feature` | 本模块没做的子功能，例如 `generateImage` 按上限拆多次调用 | 手写，带理由和 issue |

**自动判定。** 只用于 `ai` 包的文件。`check.mjs` 看用例的 `describe` 链和用例体，出现下列 API 名就归入对应类别，不写进 `layout.toml`：

| 类别 | API 名 |
|---|---|
| `single-step` | `stopWhen`、`stepCountIs`、`isStepCount`、`hasToolCall`、`prepareStep`、`needsApproval`、`tool-approval-request` 和 `tool-approval-response` part、带 `execute` 的工具 |
| `telemetry` | `telemetry`、`experimental_telemetry`、名为 `on…Start`、`on…End`、`on…Finish` 的回调 |

规则如下：

- 名单写在 `check.mjs` 里，改名单要同时改上表。
- `onError`、`onChunk`、`onAbort` 不在名单里：它们测的是错误和中止行为本身，按 aimux 的错误和中止事件迁。
- 有出处标记的用例不算自动排除。
- 规则没抓到、但确实属于这两类的用例，和后三类一样手写，带理由。

**手写排除。** `tests` 一律是表。`mapped`、`elsewhere` 的条目必须有这张表；空表表示文件已选中、没有手写排除。每条手写排除是一个 `{ case, reason }`：`case` 写完整 `describe` 链，同第 4 节的标记；`registered` 另写 `ref`（对齐文档 §5 那一行的名字），`sub-feature` 另写 `issue`。

```toml
[[openai.files]]
upstream = "src/responses/openai-responses-language-model.ts"
rust = "openai/responses/responses_language_model.rs"
status = "mapped"

[openai.files.tests]
registered = [
  { case = "OpenAIResponsesLanguageModel › …", ref = "headers 合并大小写不敏感", reason = "…" },
]
sub-feature = [
  { case = "OpenAIResponsesLanguageModel › …", issue = 275, reason = "…" },
]
```

`sub-feature` 的功能将来实现时，用例从排除里移出并补上测试。

### 3.3 证据形式

- **差分 fixture 优先**：请求 URL、headers、body，响应解析，流事件顺序，错误分类。上游的语言模型测试文件里大量用例只是请求体的变体，一个 fixture 场景可以同时覆盖几条，在 case 上用 `upstream: [...]` 列出覆盖了哪些。
- **逐条搬**：纯函数和转换逻辑，比如 prompt 转换、`prepare-tools`、`parse-tool-call`、`detect-media-type`、JSON 修复。这类用例小而多，适合批量交给 haiku。
- 两者都可以时选差分：它比较的是上游真实行为，不是我们对上游测试的转述。

## 4. 位置与标记

位置按 RFC-0046 §5（provider-utils 和 aimux 按 RFC-0044 §3.2 的目录照同样规则），按上游用例测的是什么来分：

- 测内部函数的（prompt 转换、`prepare-tools`、用量换算、finish reason 映射等）放 `foo/tests.rs`，源文件 `foo.rs` 里写一行 `#[cfg(test)] mod tests;`。这是 Rust 的标准写法，不用 `#[path]`。
- 走公开 API 的（provider → 模型 → mock fetch，例如上游语言模型测试里的 `doGenerate`、`doStream`，以及录制回放和 e2e）放 `tests/it/<vendor>/`，文件名对应上游测试文件，全部合成一个测试程序 `tests/it/main.rs`。
- 同一个上游测试文件两类都有时，按用例分开放，两处的标记都指向同一个上游文件。

只用 `it()` 的描述做函数名分不开用例。`@ai-sdk/openai@4.0.83` 的 `openai-responses-language-model.test.ts` 里有 5 个用例都叫 `should send request body with include and tool`（第 3713、3795、4035、4087、4190 行），只有中间那层 `describe` 不同；同一个 Rust 文件里同名函数也编译不过。所以：

1. 每层 `describe` 对应一个 `mod`，从最外层开始；`mod` 名和函数名是 `describe`、`it()` 描述的 snake_case。
2. 每个测试上方写一行标记，内容是包名、测试文件和完整的 `describe` 链。
3. 锁定版本里有 7 处用例的完整链相同（例如 `google-language-model.test.ts › doGenerate › should pass tools and toolChoice` 出现两次）。这种按文件里出现的顺序，标记末尾加 `#2`、`#3`，函数名加 `_2`、`_3`。

```rust
// aimux-providers/tests/it/openai/responses_language_model.rs（走公开 API）
mod openai_responses_language_model {
    mod do_generate {
        mod code_interpreter_tool {
            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › code interpreter tool › should send request body with include and tool
            #[tokio::test]
            async fn should_send_request_body_with_include_and_tool() { … }
        }

        mod image_generation_tool {
            /// upstream: @ai-sdk/openai src/responses/openai-responses-language-model.test.ts › OpenAIResponsesLanguageModel › doGenerate › image generation tool › should send request body with include and tool
            #[tokio::test]
            async fn should_send_request_body_with_include_and_tool() { … }
        }
    }
}
```

标记不写版本，版本按第 3.1 节取。差分 fixture 覆盖的用例写在 `cases/<包>.mjs` 的 `upstream` 字段里，格式同标记。现有约 85 个文件的自由格式出处注释，在 RFC-0046 搬移测试时一并改成标记。

## 5. 移植方法

1. 守 `ts-rust-port` 的纪律：逐条搬，只换 harness，不改场景。
2. harness 对应：上游 mock fetch 对应本地 HTTP 桩，上游 `MockLanguageModelV4` 对应 `ScriptedModel`。共用的 mock fetch 和回放 helper 按 RFC-0046 §5 放在 `src/test_support/`，用 `test-support` feature 打开，`foo/tests.rs` 和 `tests/it/` 用同一份。
3. 快照：`toMatchInlineSnapshot` 写成内联 `serde_json::json!`；`__snapshots__/` 文件快照放在测试所在目录的 `snapshots/`。不引入快照库。
4. 发现行为和上游不一致：先写失败的测试，再按 `ts-rust-port` 的 `fix` 流程在同一 PR 里修，单独一个提交；属于刻意差异的，登记到对齐文档 §5，用例记进 `registered`。不允许 `#[ignore]`。
5. PR 描述写"这个文件 N 个用例：搬 a、差分 b、自动排除 c、手写排除 d（按类别）"。

## 6. 差分 fixture 扩展

录制脚本现成（`scripts/aisdk-fixtures/record.mjs`），只补 case：

1. **厂商包**：`VERSIONS.json` 里除 `ai`、`@ai-sdk/provider`、`@ai-sdk/provider-utils` 以外的 15 个是厂商包。已有 fixture 的是 openai、openai-compatible、anthropic、google；其余 11 个都要补：amazon-bedrock、anthropic-aws、azure、cohere、deepseek、google-vertex、groq、huggingface、mistral、open-responses、xai。每家覆盖非流式文本、流式文本、工具调用（流与非流）、结构化输出、错误分类和各家特有的 provider 工具。
2. **core**：新增 `cases/ai-core.mjs`，用上游 `MockLanguageModelV4` 按脚本返回，录 `streamText` 的完整事件流和 `generateText` 的结果；Rust 用同一份脚本驱动 `ScriptedModel` 比对。只录单步场景。

## 7. CI 校验

一个 job，跑 `scripts/aisdk-tests/check.mjs`。它用 `inventory.mjs` 列出的用例做底，做三件事：

1. 每个标记都指向条目版本（第 3.1 节）里真实存在的用例，同一个用例只被标记一次。上游改名、删除用例时要同步。条目的 `version` 不低于锁定版本。
2. `layout.toml` 的 `tests` 只用第 3.2 节的类别。每条手写排除都有 `reason`，`registered` 有 `ref`，`sub-feature` 有 `issue`，`case` 指向真实存在的用例。
3. 把每个选中文件的进度打印到 job summary：已迁、自动排除、手写排除、未处理，自动排除的用例单独列出，评审能看到规则排除了什么。这一项只报告，不判失败。

测试数量前后相等是 RFC-0046 搬移 PR 的检查，不属于本 RFC。

升级上游版本时，`check.mjs diff` 列出选中文件里新增的用例，由升级 PR 决定迁还是排除。

## 8. 分批

| 批次 | 内容 |
|---|---|
| 0 | `layout.toml`（随 RFC-0046 建立）的 `tests` 和 `version` 字段填好；现有测试改标记（随 RFC-0046 测试搬移）；`check.mjs` 上线 |
| 1 | core：`generate-text`、`stream-text` 的 250 到 350 个要补的用例，加 core 差分 fixture |
| 2 | 纯转换函数：prompt、`prepare-tools`、`parse-tool-call`、provider-utils 小文件 |
| 3 | 主力厂商语言模型：Anthropic、OpenAI（Responses、chat）、Google、Bedrock，差分优先 |
| 4 | 其余厂商包的差分 fixture 和剩余用例 |

每批拆成多个独立 PR，不堆叠。

## 9. 完成标准

- 445 个上游测试文件对应的源文件在 `layout.toml` 里都有 `status`，`mapped` 和 `elsewhere` 的都有 `tests`；
- 每个选中文件里没有"未处理"的用例：每个用例有标记，或被自动规则排除，或有带理由的手写排除；
- 差分 fixture 覆盖第 6 节列的 15 个厂商包和 core 的单步场景；
- 对齐文档 §9.3 行为验收矩阵每一格至少有一个带标记的测试或 fixture。

## 10. 开放问题

1. **`telemetry` 类别何时取消。** 可观测性 RFC（RFC-0044 拆出的后续）落地后，这一类用例要迁。到时是改本 RFC 还是由那个 RFC 负责，倾向后者。
2. **差分覆盖与逐条搬是否二选一。** 本稿允许一个用例只由差分 fixture 覆盖，不再搬单测。
3. **上游快照里未注入的随机值。** 比较时归一化，不改 aimux 的 id 生成。
