# aimux 配置（native-rust profile）

`ts-rust-port` 和 `rust-native-rewrite` 共用。只写不看仓库就不知道、又不会很快过期的事：不写行号、提交号和待办清单，那些放 issue 和 PR。

## 上游和映射

- vercel/ai monorepo；锁定版本以 `fixtures/aisdk/VERSIONS.json` 为准，不在里面的包用 `docs/aisdk-architecture-alignment.md` 头部的版本，再没有就先问。
- 源码：`https://raw.githubusercontent.com/vercel/ai/<URL 编码的 包名@版本>/packages/<目录>/src/<文件>`；测试是同目录的 `*.test.ts`。

| 上游 | aimux |
|---|---|
| `ai` | `aimux-core/src`（如 `json_repair.rs` ← `util/fix-json.ts` + `parse-partial-json.ts`） |
| `@ai-sdk/provider` | `aimux-core/src` 的 V4 类型 |
| `@ai-sdk/provider-utils` | `aimux-provider-utils/src`，kebab → snake |
| `@ai-sdk/<vendor>` | `aimux-providers/src/<vendor>/`；`google-vertex` → `vertex/`，`amazon-bedrock` → `bedrock/`；按模块头注释的 `packages/<pkg>/src/<file>.ts` 找 |
| 无上游 | `aimux-ffi`、`bindings/`、`recording.rs`/`replay.rs`、`decision_*`：只能新旧差分 |

OpenAI-compatible preset 来自 `provider_registry.json` 的运行时 descriptor 表（D12），没有逐厂商代码。

## 设计文档

`docs/aisdk-architecture-alignment.md`：决策记录 D1–D31，§0.4–0.5，§5（定下来的不像 Rust 的写法），§9.2–9.3（构建检查、行为验收）。碰到消息类型或 wire 再读第二部分。

## 不能动的面

| 面 | 守护 |
|---|---|
| 公开 JSON：camelCase、kebab-case `type`、base64 字节、缺失省略、null 保留 | 绑定测试、`cargo test --workspace` |
| 各绑定的 DTO 和 web 控制台的 wire 类型（`cargo run -p aimux-codegen` 从 Rust 类型生成，不手写、不用 ts-rs） | `cargo run -p aimux-codegen -- --check`；`python3 scripts/check_dto_vectors.py <语言>` |
| C ABI（`aimux-ffi`，各语言绑定依赖） | `aimux-ffi/tests`、`scripts/local-ci.sh` |
| Node 绑定 | 构建后 `git diff --exit-code -- bindings/node/index.js bindings/node/index.d.ts` |
| 录制格式、cassette 回放 | 回放测试 |
| 错误 JSON（AI SDK 的 `name` 标签） | 原生化不改 |
| `docs/api/providers.md` | `python3 scripts/gen_providers_doc.py --check` |

## 预言机

1. 上游录制：`fixtures/aisdk/<pkg>/*.json`，`aimux-providers/tests/*_factory_test.rs` 回放（URL、headers、body、结果、流事件、错误分类）。
2. 搬来的上游单元测试，例如 `aimux-core/tests/util_test.rs` 的 fix-json 用例。
3. node 跑锁定版本上游（Node 22 `--experimental-strip-types`），读写 JSONL。
4. cassette 回放：`aimux-providers/tests/cassettes/`，不联网。
5. 新旧差分只在改动期间存在；入库的是它记下的结果（例：`aimux-core/tests/fixtures/json_repair_cases.jsonl`），不是旧实现。跨构建差分的探针不入库，PR 描述写位置和命令。进程级计数器（`source-{n}` 之类）比较前归一化。

## 检查（CI 对每个 PR 跑）

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo test --workspace
cargo run -p aimux-codegen -- --check   # 生成物漂移；改了公开类型先不带 --check 重新生成
python3 scripts/gen_providers_doc.py --check
bash scripts/check_provider_boundaries.sh
```

碰到 napi、FFI 时加跑 `scripts/local-ci.sh` 对应的节。每个 worktree 用自己的 `CARGO_TARGET_DIR`。

lint 统一写在根 `Cargo.toml` 的 `[workspace.lints]`，成员 crate 用 `[lints] workspace = true`，CI 用 `-D warnings`。新开一个 lint 的 PR 同时清掉它报的全部位置，不用全局 `allow` 绕过；逐处抑制用带 `reason` 的 `#[expect]`。哪些 lint 分哪批开、各报多少处，记在风格整改的 issue 里。

## 项目决定

- wire 跟 AI SDK 的 JSON 形态；wire 变化单独成 PR，同步所有绑定。
- 可选方法用 `Option<Result<…>>`（§5、D22）；provider 可调用 `call(model_id)`；默认 provider 访问不可失败，显式工厂返回 `Result`。
- 设置求值时机（工厂、取模型、请求）逐包跟上游（D10）；headers 每次请求求值。
- 不用 provider 名字判断厂商（D8）；providerOptions 的键在各包 `options.rs` 只写一次（D9）；retry 只在调用层（D14）；core 单步，不跑 agent loop（D25）。
- `do_generate`/`do_stream` 保持 `async_trait`（registry、FFI 要 `dyn`）。
- 分层单向：`shared/`、`openai_compatible/` 不引用具体厂商模块。跨模块扩展点由共享层定义 trait，厂商用自己的纯数据 struct 实现；包内有限几种情况用包内 enum（`check_provider_boundaries.sh` 守）。
- 流：每个厂商的流是一个实现 `Transformer` 的具名 struct；工具调用的累积是流 struct 自己的字段，没有共享的 tracker 类型（和上游 provider-utils 结构不同，行为照上游）。
- 保留 openai-compatible 的 `transformRequestBody`、`metadataExtractor`、`supportsMultiPartToolContent` 和 `specification_version`。
- 栈内 breaking 直接替换，不留 deprecated 别名，同一 PR 改完调用方和绑定（D31）。
- 范围只做结构清理、行为对齐和缺陷修复，不新增对外能力（§0.4）。
- 锁 poison 一律转成错误返回：不 `unwrap`/`expect`（release 是 abort，panic 等于宿主进程退出），也不用 `PoisonError::into_inner` 吞掉继续用。
- 移植先照上游结构忠实移植，原生写法是第二步；差分用大样本随机输入；不加默认的 `#[non_exhaustive]`；setter 取 `self`（rig 风格）；错误跟 AI SDK 错误类走（按功能拆分错误类型、错误 JSON 不变，在 #365 待讨论），`thiserror` 保留 `#[error]`、`#[source]`。通用 Rust 风格指南和这些冲突时按这里。

## 陷阱

- 数字：上游是 JS number，用量、价格、max_tokens 用整数解析；写回 JSON 的 `f64` 会把 `12` 变成 `12.0`，加 golden 测试。
- null 和缺失：上游区分，`Option` + `skip_serializing_if` 会合并；wire DTO 保留 null。
- SSE 和 event-stream 可以在任意字节切开；重写分帧时用同一字节流的所有切法差分。
- headers：settings → 单次调用 → 鉴权，后层覆盖；`None` 删除；名字小写。
- JS promise 创建就跑，Rust future 不 poll 不跑；不 await 的 promise 移植时改成 spawn 或删掉。
- UTF-16：JS 的长度和下标按码元；截断、长度校验用 emoji 和中文对一次上游。
- panic 策略：`release`、`ios-release` profile 是 `panic = "abort"`，这两种产物里 `catch_unwind` 拦不住 panic；dev、test 能拦住不代表发布产物能（发布产物改 unwind 并在 FFI 边界 `catch_unwind`，在 #365 第六节待评估）。

## 工作约束

- 每个 PR 一个方向：一个模块或一种模式。标题 `[n/m] refactor: …`，分支名和标题一致，开 PR 前定好。
- 不改写别人已推送的分支；PR 先 Draft，CI 绿了再 Ready。
- 生产代码不新增 `unwrap`/`expect`/`panic`（§9.2）；凭据类型的 `Debug` 脱敏。
- 参照库：rig（`0xPlaygrounds/rig`）、async-openai（`64bit/async-openai`）。
