# aimux 配置（native-rust profile）

给 `ts-rust-port` 和 `rust-native-rewrite` 两个 skill 用。仓库里的位置：`.agents/native-rust/PROFILE.md`。
“上游”“映射”和预言机里的上游录制、node 跑上游，只有 `ts-rust-port` 用；其余各节两个 skill 都用。

setup 于 2026-10-12，2026-10-13 按 AI SDK 迁移栈（[1/8]–[8/8]，#249–#256）和 Rust 化系列（#257–#260）更新；文中行号对应 #260 的提交 ef99ed47。各层合入 master 后要重新核对行号。

同目录：`gate.sh`（全套检查），`probe/`（跨构建差分的现成实现）。

## 上游

- vercel/ai monorepo。实现对照的版本以 `fixtures/aisdk/VERSIONS.json` 为准：`ai` 7.0.127、`@ai-sdk/provider` 4.0.21、`@ai-sdk/provider-utils` 5.0.53、`@ai-sdk/openai` 4.0.83、`@ai-sdk/anthropic` 4.0.71、`@ai-sdk/google` 4.0.87、`@ai-sdk/openai-compatible` 3.0.62。
- 设计文档用的是更早的一组版本（`ai` 7.0.122、openai 4.0.80 等，见 `docs/aisdk-architecture-alignment.md` 头部），文中的 file:line 对应那组。两者冲突时以 VERSIONS.json 为准。
- 不在 VERSIONS.json 里的包（xai、mistral、groq、bedrock、vertex 等）：用设计文档头部列的版本；那里也没有的先问。
- 锁定版本源码：`https://raw.githubusercontent.com/vercel/ai/<URL 编码的 包名@版本>/packages/<目录>/src/<文件>`。已验证返回 200 的例子：
  - `…/vercel/ai/%40ai-sdk%2Fprovider-utils%405.0.53/packages/provider-utils/src/index.ts`
  - `…/vercel/ai/ai%407.0.127/packages/ai/src/util/fix-json.ts`
- 包目录：`@ai-sdk/x` → `packages/x`，`ai` → `packages/ai`。上游测试是同目录的 `*.test.ts`。

## 映射

| 上游 | aimux |
|---|---|
| `ai`（`packages/ai/src`） | `aimux-core/src`：`generate.rs`、`parse_tool_call.rs`、`json_repair.rs`（← `util/fix-json.ts` + `parse-partial-json.ts`）等 |
| `@ai-sdk/provider` | `aimux-core/src` 里的 V4 类型。设计文档 §2 的目标是独立 crate `aimux-provider` |
| `@ai-sdk/provider-utils` | `aimux-provider-utils/src`，文件名 kebab → snake |
| `@ai-sdk/<vendor>` | `aimux-providers/src/<vendor 的 snake 写法>/`；例外：`google-vertex` → `vertex/`，`amazon-bedrock` → `bedrock/`。文件不一一对应，按模块头注释里的 `packages/<pkg>/src/<file>.ts` 找，或用上游函数名 `rg` |
| 无上游 | `aimux-ffi`、`bindings/`、`tools/*`、`recording.rs`/`replay.rs`、`decision_*`、`moa.rs`：只能新旧差分 |

查找：`git grep -nE 'packages/[a-z-]+/src/[A-Za-z0-9/_-]+\.ts|Port of' -- '*.rs'`

provider 数量（`docs/api/providers.md`，生成文件）：共 326 个。其中 281 个是 `aimux-providers/src/provider_registry.json` 里的 OpenAI-compatible preset，按名字从运行时 descriptor 表创建（D12），没有逐个厂商的 Rust 代码，原生化它们就是改 descriptor 表那一处；另外 45 个有自己的工厂。

## 阶段

- 已原生化：
  - `aimux-provider-utils/src/streaming_tool_call_tracker.rs`（迁移栈 [2/8] #250）；
  - `aimux-core/src/json_repair.rs`（#257）；
  - `aimux-core/src/parse_tool_call.rs` 的 repair 钩子改成 `RepairToolCall` trait（#258）；
  - provider 私有配置改成纯数据（#259）；
  - 各厂商语言模型的 `do_stream` 改成 `Transformer` 结构体（#260，`aimux-provider-utils/src/transform_stream.rs`）。
- 其余已移植的模块：忠实移植阶段。

## 设计文档

`docs/aisdk-architecture-alignment.md`（栈上，约 155KB），相当于 React 的架构说明。审查必读：

- 第一部分“设计决策记录” D1–D31；
- §0.4 四项跨文档决定，§0.5 生成链；
- §5 “Rust 适配与明确的产品差异”（哪些不像 Rust 的写法是定下来的）；
- §9.2 构建检查、§9.3 行为验收矩阵；
- 第二部分：消息协议四层和 `wire-and-options` 盘点（约 1000 行，只在改动碰到消息类型或 wire 时读）。

附录 `docs/aisdk-architecture-impact-map.md`：201 条核验过的发现；§1.1 统计 53 条 bug，§5 分级列出（合并后 59 行）。

## 不能动的面

| 面 | 位置 | 守护 |
|---|---|---|
| 公开 JSON：camelCase 字段、kebab-case `type` 标签、base64 字节、缺失属性省略、JSON null 保留（栈 #225；设计文档 §0.5） | 各 crate 的 serde 类型 | 绑定测试、`cargo test --workspace` |
| ts-rs 生成的 d.ts | `#[derive(TS)]` | `python3 scripts/gen_ts_types.py --check`。设计文档 §0.5 定了 ts-rs 和 `scripts/gen_*` 要退役，所以原生化不新增 `#[derive(TS)]` |
| C ABI | `aimux-ffi/src`、`aimux-ffi/aimux-ffi.h`、`aimux-error.h`；Go/Java/Swift/Kotlin/Flutter/C 绑定依赖它；句柄是单调递增 `u64` + 有类型枚举（D19） | `aimux-ffi/tests`、`scripts/local-ci.sh` |
| Node 绑定 | `bindings/node` | 构建后 `git diff --exit-code -- index.js index.d.ts` |
| 录制和回放 | `aimux-core/src/recording.rs`、`replay.rs`；`aimux-providers/tests/cassettes/` | 回放测试 |
| 错误 JSON | 目前用 AI SDK 的 `name` 标签（如 `AI_APICallError`）；RFC-0039 §6.1（PR #209，未合入）要另一种信封，未定 | 原生化不改错误 JSON |
| provider 文档 | `docs/api/providers.md` | `python3 scripts/gen_providers_doc.py --check` |

## 预言机

1. **上游录制**：`fixtures/aisdk/<pkg>/*.json`，由 `scripts/aisdk-fixtures` 生成（`npm ci && node record.mjs`，模拟传输，不联网）；`aimux-providers/tests/{anthropic,google,openai,openai_compatible}_factory_test.rs` 回放，比较 URL、headers、body、结果、流事件和错误分类。只覆盖 4 个包。缺口：`streamText` 的 `result.parts` 录了，但没有测试拿 core 的流事件和它比；碰到 core 流时补上。
2. **搬来的上游单元测试**：例如 `aimux-core/tests/util_test.rs` 里 fix-json 47 个、parse-partial-json 4 个用例（共 51）。
3. **node 跑上游**：Node 22 `--experimental-strip-types`。harness 读 `corpus.jsonl`、写 `results.jsonl`；json_repair 那次的 runner（`run_upstream.mjs`）没有入库；上游的 `parse-partial-json` 还依赖 provider-utils 的 `secure-json-parse`，要一起下载。
4. **进程内新旧差分**：`tests/<模块>_diff_test.rs` 放 `mod legacy`，xorshift 生成器，默认 2 万，`<模块>_DIFF_N` 放大到 100 万（例：#257 的 `json_repair_diff_test`）。
6. **跨构建差分**（#259、#260 用过）：`probe/zz_diff_probe.rs` 放进新旧两个 worktree 的 `aimux-providers/tests/`，`probe/run.sh <worktree> <target-dir> <out.jsonl> [filter]` 运行，`python3 -I probe/diff.py old.jsonl new.jsonl` 比较。
   - 用记录请求的 `fetch` 回放 cassette，覆盖每个公开语言模型入口，再加 282 个 registry preset。
   - 流式按四种方式跑：整段、带原始块、7 字节切块、切一半后传输出错。
   - 已知不稳定：`openai/responses/responses_convert.rs` 的 `source-{n}` 和 `xai/responses/mod.rs` 的 `SOURCE_ID_COUNTER` 是进程级计数器，比较前要归一化。
   - 跑不到：没有语言模型入口的 cassette（files、embeddings、realtime 等），以及 search、speech、transcription、image、video 这几类 provider（没有 cassette）。
   - 全量构建后一次约 2–3 分钟；先用 filter 跑一个厂商。
5. **cassette 回放**：`aimux-providers/tests/cassettes/`，不联网。

## 检查（`.github/workflows/ci.yml`、`CONTRIBUTING.md`）

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo doc --workspace --no-deps
cargo test --workspace                    # 会改写 bindings/node/src/types/*.ts，跑完 git checkout -- bindings/node/src/types
python3 scripts/gen_ts_types.py --check
python3 scripts/gen_providers_doc.py --check
bash scripts/check_provider_boundaries.sh
```

碰到 napi 或 FFI 时加跑对应绑定（`scripts/local-ci.sh` 各节）。

`gate.sh <worktree> <target-dir> <log>` 一次跑完上面全部，并按测试二进制汇总通过数，基线和改后各跑一次对比。
- 它用 `-D warnings -A clippy::must_use_candidate` 跑 clippy：较新的本地 clippy（1.97）会在 `aimux-provider-utils/src/fetch.rs` 的 `bytes_stream` 和 `bedrock/event_stream.rs` 上报这条，CI 的工具链不报，是已有的代码。
- CI（`.github/workflows/ci.yml`）对每个 PR 运行。

## 项目决定（看起来不像 Rust，但已经定了）

设计文档 §5 和决策记录：

- **wire 形态先统一，再谈原生化**：设计文档 §0.1 登记过“wire 三套并存”（内部标签、外部标签、camelCase 混 snake_case）。已定的目标是 AI SDK 的 JSON 形态：camelCase 字段、kebab-case `type` 标签、base64 字节、缺失属性省略、JSON null 保留（§0.5、第二部分 target），栈 #225 已按这个改了公开 JSON。Rust 字段用 snake_case，靠 `#[serde(rename_all = "camelCase")]` 等属性对上 wire。原生化碰到还没统一的 wire 形态时，不在同一个 PR 里顺手改：wire 变化单独成 PR，并同步所有绑定。
- **可选方法**用 `Option<Result<…>>` 或可选 future，区分“没有这个能力”和“执行失败”（§5、D22）。不改成 `Err(Unsupported)`。
- **可调用 provider**：每个包的 provider 有 `call(model_id) -> Arc<dyn LanguageModel>`，和 `Provider::language_model` 返回同一个模型（§5）。
- **设置**：`Resolvable<T>` 只用在包的 settings 真正暴露它的地方；OpenAI、Anthropic、Google 的 key 是 `Option<String>`，headers 是固定 map（§5）。设置在工厂、取模型、请求三个阶段中哪一段求值，逐包跟上游（D10）；重写构造函数时不能改变求值时机。
- **默认 provider 访问**不可失败，失败延到真正使用时；显式工厂返回 `Result`（§5）。
- **preset**：从 registry JSON 的运行时 descriptor 表按名字创建，不为每个厂商生成 `create_<name>`（D12）。
- **厂商判断**：不用 provider 字符串（包括子串）判断厂商或审计族（D8）；共享协议包不比较 provider 名字（`check_provider_boundaries.sh` 第 4 条）。
- **providerOptions**：规则逐模型实现，键在每个包的 `options.rs` 只写一次（D9、边界检查第 6–8、10 条）。
- **retry** 只在调用层；provider 和 model 不带默认 retry，也不从 `CallOptions` 读重试、超时、会话 id（D14、边界检查第 1、9 条）。
- **`LanguageModel` 的 `do_generate`/`do_stream`** 保持 `async_trait`，registry 和 FFI 需要 `dyn`。
- **core 单步**：不引入 agent loop、不执行宿主工具（D25、§0.4 Q1）。
- **工具参数**：非对象的工具参数编码成 `"{}"`，空参数变成 `{}`，跟 `@ai-sdk/openai` 的 `serializeToolCallArguments`（4.0.49 起）。项目决定跟上游。
- **重定向**：跨域跟随，但跨域后只保留 User-Agent，SigV4 签名也去掉；回到原域后不恢复（2026-10-11 决定；实现在 `b46f16d9`）。
- **流在没有结束标记时到 EOF**，仍发出 tool-call 和 Finish（跟上游；RFC-0042 §4.3（PR #212，未合入）想要报截断，只记录不改）。
- 保留 approval 类型（`ToolApprovalResponsePart` 等）和 `StreamingToolCallTracker`（和上游 provider-utils 一致；rig 的 `Out::fragment` 做法不同，见 rust-native-rewrite 的 rig 对照）。
- 保留 openai-compatible 的 `transformRequestBody`、`metadataExtractor`、`supportsMultiPartToolContent`，以及 `specification_version`；原生化不能顺手删掉（E1 旧版删过，#259 没搬那部分）。
- 范围：设计文档只做结构清理、行为对齐和已发现缺陷的修复，不新增对外能力（§0.4 范围原则）。

## 已知 bug（原生化时不顺手修，新代码也不能再犯）

- 凭据类型的 `Debug` 打印明文：`aimux-provider-utils/src/sigv4_fetch.rs:28` 的 `AwsCredentials`、`aimux-providers/src/codex.rs:515` 的 `CodexTokens`（master 上由 #248 修了，迁移栈上还在）。
- `aimux-providers/src/openai/mod.rs:277`：`openai()` 单例里的 `expect`，`OPENAI_BASE_URL` 不合法时 panic，违反“默认访问不可失败”。
- `aimux-providers/src/openai/mod.rs:264`：user-agent 写死 `4.0.80`，锁定版本是 4.0.83。
- `ContentPart::Reasoning` 顶层的 `signature` 字段：`response_messages.rs:202` 写入，`language_model_message.rs:360-363` 转 V4 时被丢弃。
- core 流的 text-delta/reasoning-delta 用字段名 `delta`，上游和 RFC-0042 §3.1（PR #212，未合入）用 `text`。
- `aimux-web` 的 `env:VAR` 能读服务进程的任意环境变量（设计文档 §0.6）。
- 影响面地图 §5 分级列出的 bug（§1.1 统计 53 条）。

## 陷阱

通用的 JS/Rust 差异（字符串排序、正则断言等）在 aimux 的 SDK 行为里没有契约，不列。下面几类和 aimux 相关；“已见”有代码位置，“待验证”要等真实案例。

| 陷阱 | 状态 | 内容 |
|---|---|---|
| 数字类型 | 已见 | core 的 token 用量是 `u32`（`aimux-core/src/types.rs` 的 `InputTokenUsage`/`OutputTokenUsage`，`embedding_model.rs:78` 的 `EmbeddingUsage`），上游是 JS number。几个 provider 的 DTO 先按 `f64` 解析再 `as u32`（如 `openai/embedding.rs:131,100`、`cohere/embedding.rs:182`），超出范围会静默饱和、小数被丢掉；`aimux-providers/src` 里 `as u32` 共 58 处。另外 `f64` 字段原样写回 JSON 时，`12.0` 和上游的 `12` 不同（如 `xai/video.rs:640,808` 的 `costInUsdTicks`）。原生化碰到用量、价格、max_tokens 类字段时，用整数类型解析，并对写回的 JSON 加 golden 测试 |
| null 和缺失 | 待验证 | 上游的 `??`/`?.` 区分 `null` 和缺字段；Rust 侧 `Option` + `skip_serializing_if` 会把两者合并。重点看 `rawFinishReason`（`generate.rs:253,284`，`Option<String>`；`finish_reason` 本身不是 Option）和 provider 响应里可为 `null` 的字段。设计文档第二部分要求 wire DTO 保留 JSON null 语义 |
| SSE 分块边界 | 已有测试 | 网络可以在任意字节切开一行 `data:`，上游 `eventsource-parser` 的重组行为就是规格。aimux 用 `sse-stream` crate 加一层适配（`aimux-stream/src/sse.rs`），头注释列了刻意保留的两处差异（非法 UTF-8 报错结束流；传输错误后结束流）。`aimux-stream/tests/sse_test.rs` 有不均匀分块和多字节字符被切开的用例。重写任何缓冲或分帧逻辑（包括 Bedrock event-stream、WS）时，用“同一字节流的所有切法”做差分 |
| headers | 已见 | `aimux-provider-utils/src/headers.rs`：层叠顺序是 settings → 单次调用 → 鉴权，后层覆盖前层（不追加）；`None` 表示删除（上游的 `undefined`）；名字统一小写，只差大小写的重名按原名字节序处理，避免依赖 HashMap 顺序。上游的 `headers()` 每次请求求值（设计文档开头“AI SDK 的做法”），鉴权写在 headers 闭包或 fetch 装饰器里（D11）；设计文档 §9.3 “Headers”一行要求：大小写、覆盖、None 删除、无重复凭证、逐 attempt 更新 token。重写时不能把每次请求求值改成构造时求值一次 |
| Promise 立即执行，future 不 poll 不执行 | 已见一处设计，影响待验证 | `aimux-provider-utils/src/resolvable.rs`：`Resolvable::Future` 缓存结果，相当于 promise；`Fn`/`AsyncFn` 每次 resolve 都重新计算。但 JS 的 promise 在创建时就开始跑，Rust 的 future 要到第一次 resolve 才开始，所以“工厂时取 token”这类行为的时机不同。另外上游里不 await 的 promise（发出去不管）照样会执行，移植成没 await 的 future 就什么都没做；碰到时改成 spawn 或删掉，并写进契约 |
| UTF-16 长度 | 已见一例，其余待验证 | JS 的 `.length` 和下标按 UTF-16 码元。`json_repair` 和上游对比时，辅助平面字符造成 63/1,000,000 处差异（已知差异）。`openai/options.rs:110,113` 已经按 `encode_utf16().count()` 校验长度，是对的写法。碰到截断、长度校验、token 估算时，用 emoji 和中文跑一次上游 |

## 同一概念的多份类型

设计文档 §0.1 登记过“ToolCall ×5、ToolResult ×4、wire 三套并存”（第二部分 WIRE-1）。栈顶还在的独立 tool-call 类型有 4 个：`aimux-core/src/tool.rs:124` 的 `RawToolCall`、`:143` 的 `ToolCall`、`content.rs:151` 的内联变体、`language_model_message.rs:102` 的 `ToolCallPart`；`stream_part.rs:112,268` 和 `result.rs:34` 是承载前两者的变体。原生化时不要再加一份；合并它们按第二部分的四层协议做，单独成 PR。

## 工作约束

- 维护者要求每个 PR 只做一个清楚的方向；原生化一次一个模块或一种模式。
- 分支：迁移栈合入 master 前，原生化 PR 叠在栈顶之上，每个 PR 的 base 是上一个；合入后基于 master。标题 `[n/m] refactor: …`，分支名和标题一致，开 PR 前定好（改 head 分支名会关闭 PR）。
- breaking：栈内不留 deprecated 别名和转发层，直接替换，并在同一 PR 改完所有调用方和绑定（设计文档“立场”、D31、§5“不采用”）。发布后的规则看 RFC-0039，未定。
- 推送：按维护者和贡献者自己的推送方式；不改写别人已推送的分支。PR 先开 Draft，CI 全绿再转 Ready。
- 每个 worktree 用自己的 `CARGO_TARGET_DIR`，否则会复用别的分支的旧产物；全量构建一个 target 约十几 GB，用完删掉。
- 规范、工具、厂商 crate 的生产代码限制 `unwrap`/`expect`/`panic`（设计文档 §9.2）。
- 提交：`refactor(<crate>)[!]: …`，契约放 commit body，API 变化写进 CHANGELOG。

## 范例模块

`aimux-provider-utils/src/streaming_tool_call_tracker.rs`：先忠实移植（栈上 `6035201d`，45 个 tracker 和 9 个 argument-state 上游用例），再原生化（`b0fd923a`），和忠实移植版差分约 110 万个随机 delta，0 处差异。改法：`has_finished` + `flush` → `finish(mut self, out)`；`Structured { complete: bool, stack }` → `Open { stack }` 和 `Complete` 两个变体；自持数据加 `extra: Value` 的 delta → 借用、拍平的 `StreamingToolCallDelta<'a>`；删掉两个 `Box<dyn Fn>` 钩子和测试专用 builder；`HashMap<String, HashSet<usize>>` 索引 → `Vec` 线性扫描。

## 参照库

- rig `https://github.com/0xPlaygrounds/rig` @ 9e7cb60（2026-10-08）。这一版已经没有 `CompletionModel`、`ProviderClient`、`PromptHook`、`OneOrMany`；核心是 `Model<Wire, Transport>`（crates/rig-core/src/driver.rs:53）。
- async-openai `https://github.com/64bit/async-openai` @ c2deb88（2026-10-09）。
- 逐项对照（已对齐的、可以改的 A1-A8、被设计文档挡住的）见同目录的 `rig-alignment.md`。A1（`AwsCredentials`、`CodexTokens` 换成脱敏 newtype）同时修已知 bug，优先。

## 候选（2026-10-12 在 rfc-0036 栈顶扫描，行号没按 ef99ed47 更新，用前重新扫）

| # | 位置 | 痕迹 | 原生写法 | wire | 和项目决定 |
|---|---|---|---|---|---|
| 1 | `aimux-core/src/message.rs:36-42` | `ModelMessage { role, content }` 互相独立 | 按 role 区分的 enum，`#[serde(tag = "role")]` | 不影响 | 符合第二部分四层协议 |
| 2 | `aimux-core/src/generate.rs:87-166` | `GenerateTextOptions` 全是 `Option`，再逐字段复制 | 请求类型 `new(prompt)` + setter | 不影响 | 无冲突 |
| 3 | `generate.rs:102,117,123` | `top_k: Option<f64>`；`Option<集合>` | `u32`；空集合 | `top_k` 不再收小数，需确认 | 无冲突 |
| 4 | `generate.rs:156,165` | options 里的 `AbortSignal`、repair 钩子 | drop 取消；repair 改成 trait（**repair 已在 #258 完成**） | 不影响 | 先查 §9.3“流”一行的取消要求 |
| 5 | `aimux-core/src/provider.rs:86-121` | `Option<Result<Arc<dyn …>>>` | — | — | **冲突**：§5、D22 定了这个写法，保留 |
| 6 | `aimux-core/src/error.rs:283-289,301` | `model_type: String` | `ModelKind` enum，序列化成同样的字符串 | 不影响 | 无冲突 |
| 7 | `error.rs:184,232,312` | `JsonParse(String)` 等字符串错误 | 结构化变体 + `#[from]` | 影响 | 等 RFC-0039 错误信封 |
| 8 | `aimux-providers/src/openai/mod.rs:184-277,369` | `create_openai` 全 `Option`；`call()` | — | — | **部分冲突**：`call()` 和 settings 形状是 §5 定的；只剩 `expect`（已知 bug，单独修） |
| 9 | `aimux-providers/src/openai/options.rs:9-13` | `JsonObject` + 手写校验器模拟 zod | 有类型的选项 struct | golden 测试守住就不影响 | 符合 D9；键仍只在 `options.rs` 写一次 |
| 10 | `generate.rs:305-312` | `pub stream: Pin<Box<…>>` | 类型自己实现 `Stream` + `finish(self)` | 不影响 | 先查 §9.3“流”一行 |
| 11 | `aimux-core/src/message.rs:26-33` | `serde(untagged)` 的 `string \| Part[]` | 内部只存 `Vec<ContentPart>`，字符串走 `From` | 序列化可能变 | 先查第二部分的用户输入层 |
