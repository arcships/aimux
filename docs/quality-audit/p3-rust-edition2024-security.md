# P3：Rust Edition 2024 / 1.85+ 安全调研报告

> 调研截止：2026-08-06。范围：Rust 1.85.0（2025-02-20）至 2026-08-06；结论均附来源 URL。项目仓库状态检查发现本任务开始前已有用户修改：`bindings/node/package-lock.json`、`rustfmt.toml`，本报告未触碰。

## 概述

- Rust 2024 在 Rust 1.85.0 稳定，是可选 edition，不是对 2021 crate 的全局强制升级。[Rust 1.85/2024 发布说明](https://blog.rust-lang.org/2025/02/20/Rust-1.85.0/)
- aimux 已在根 [Cargo.toml](../../Cargo.toml#L12-L18) 声明 `edition = "2024"`、`rust-version = "1.85"`；`aimux-provider-utils` 继承该设置。
- [logging.rs](../../aimux-provider-utils/src/logging.rs#L191-L196) 中 `unsafe { std::env::set_var(...) }` 是 Edition 2024 下正确的语法。它不是“为了绕过安全检查”而加的 unsafe：调用方仍必须证明没有其他线程并发访问进程环境。
- 截止日期在关键依赖中明确发现两项需处理的 RustSec 信息：`tokio` RUSTSEC-2025-0023（已被锁定版本 1.53.1 修复）和 `anyhow` RUSTSEC-2026-0190（受影响 `<1.0.103`；本仓库未在 Cargo.lock 中发现 anyhow，workspace 声明是宽版本 `1`）。建议执行 `cargo update`/`cargo audit` 验证解析后的实际图。
- Rust 工具链方面，发现 2026 年两项 Cargo 安全公告（CVE-2026-5222、CVE-2026-5223），均在 Rust 1.96 修复；对 crates.io 使用者影响有限/无影响，但第三方 registry 用户应立即升级。Rust 1.85 不应作为安全工具链基线，建议 CI/发布机使用当前稳定版（截至本报告日期至少 1.96）。

## Edition 2024 breaking changes 清单

以下清单来自官方 [Rust 1.85 发布说明](https://blog.rust-lang.org/2025/02/20/Rust-1.85.0/) 和 [Edition Guide](https://doc.rust-lang.org/edition-guide/rust-2024/index.html)。影响标记是对 aimux 的定向判断。

| 变化 | 安全/迁移含义 | 对 aimux |
|---|---|---|
| RPIT `impl Trait` 生命周期捕获规则 | 无 `use<..>` 时，2024 隐式捕获所有作用域内泛型参数（含生命周期），可能改变返回 opaque type 的可用生命周期边界；必要时用 `use<...>` 保持旧语义。[官方章节](https://doc.rust-lang.org/edition-guide/rust-2024/rpit-lifetime-capture.html) | **潜在影响**：检查公开 API、async fn、trait RPITIT；编译器的 `impl_trait_overcaptures`/`cargo fix --edition` 可辅助发现。不是内存安全漏洞 |
| `if let` 临时值作用域 | 改变临时值 drop 时机，可能影响锁、guard、借用和析构副作用。[官方发布说明](https://blog.rust-lang.org/2025/02/20/Rust-1.85.0/) | **潜在影响**：针对锁/资源 guard 的分支运行测试 |
| block 尾表达式临时值作用域 | 改变 tail expression 临时值生命周期/drop 顺序。[官方发布说明](https://blog.rust-lang.org/2025/02/20/Rust-1.85.0/) | **潜在影响**：审查返回表达式涉及 guard/临时引用的位置 |
| match ergonomics 保留限制 | 2024 禁止部分易混淆的模式组合，为未来改进保留语法空间。[官方章节](https://doc.rust-lang.org/edition-guide/rust-2024/match-ergonomics.html) | **低**：仅在相关模式出现时改写 |
| unsafe extern blocks | `extern` block 必须写 `unsafe extern`。[官方章节](https://doc.rust-lang.org/edition-guide/rust-2024/unsafe-extern.html) | **低/条件性**：主要检查 FFI crate；aimux-ffi 若声明 extern 需审计 ABI/签名 |
| unsafe attributes | `no_mangle`、`export_name`、`link_section` 必须写 `#[unsafe(...)]`，并人工证明符号/链接安全。[官方章节](https://doc.rust-lang.org/edition-guide/rust-2024/unsafe-attributes.html) | **中/条件性**：FFI 导出符号需检查；这不是简单机械替换 |
| `unsafe_op_in_unsafe_fn` 默认 warning | unsafe fn 内的每个 unsafe 操作需显式 `unsafe {}`，强化局部安全审计。[官方章节](https://doc.rust-lang.org/edition-guide/rust-2024/unsafe-op-in-unsafe-fn.html) | **中**：建议 CI 不要压制该 lint |
| 禁止引用 `static mut` | 对 `static mut` 生成 deny-by-default 错误；建议原子类型/Mutex/OnceLock。[官方章节](https://doc.rust-lang.org/edition-guide/rust-2024/static-mut-references.html) | **低/条件性**：搜索后未作为本报告任务修改；logging 使用 `Once` |
| never type fallback | 改变 `!` fallback，并将流入 unsafe 的相关 lint 设为 deny。[官方章节](https://doc.rust-lang.org/edition-guide/rust-2024/never-type-fallback.html) | **低**：编译检查即可发现 |
| 宏 fragment specifier | `expr` 现在也匹配 `const`/`_`；缺少 fragment specifier 变硬错误。[官方章节](https://doc.rust-lang.org/edition-guide/rust-2024/macro-fragment-specifiers.html) | **低/条件性**：proc-macro/宏 crate 编译检查 |
| `gen` 关键字、保留语法 | `gen` 被预留；`#"..."#`、`##` 等语法被预留，可能要求 raw identifier/重命名。[官方章节](https://doc.rust-lang.org/edition-guide/rust-2024/gen-keyword.html) | **低**：若有同名标识符需迁移 |
| Prelude 变化 | `Future`、`IntoFuture` 加入 prelude，可能造成名称冲突/解析变化。[官方章节](https://doc.rust-lang.org/edition-guide/rust-2024/prelude.html) | **低**：编译器会暴露冲突 |
| Box slice `IntoIterator` | 为 `Box<[T]>` 增加 `IntoIterator`，可能改变方法解析。[官方章节](https://doc.rust-lang.org/edition-guide/rust-2024/intoiterator-box-slice.html) | **低** |
| 新增 unsafe 标准库函数 | `env::set_var`、`env::remove_var`、Unix `before_exec` 仅在 2024 变 unsafe。[官方章节](https://doc.rust-lang.org/edition-guide/rust-2024/newly-unsafe-functions.html) | **明确影响**：`logging.rs:194` 已使用 unsafe block；需保留明确 SAFETY 说明并确保测试进程无并发环境访问 |
| Cargo rust-version aware resolver | Cargo resolver 考虑依赖的 `rust-version`，可能选择不同版本/暴露 MSRV 冲突。[官方章节](https://doc.rust-lang.org/edition-guide/rust-2024/cargo-resolver.html) | **中**：锁文件和 MSRV CI 要求可重复验证 |
| Cargo inherited default-features 行为 | workspace 继承依赖时，`default-features = false` 的无效用法会被拒绝。[官方章节](https://doc.rust-lang.org/edition-guide/rust-2024/cargo-inherited-default-features.html) | **低**：当前 workspace 配置需 cargo check 验证 |
| rustdoc/rustfmt 行为 | doctest 合并、nested `include!` 相对路径和格式化/排序规则变化。[官方发布说明](https://blog.rust-lang.org/2025/02/20/Rust-1.85.0/) | **低**：只影响文档/格式，不属于运行时安全漏洞 |

### `set_var` 的具体确认

官方说明指出，多线程程序在部分平台上修改进程环境可能不 sound，因此 2024 将 `set_var`/`remove_var` 标为 unsafe；调用必须发生在其他线程可能运行之前，或由调用方满足平台安全前提。[新 unsafe 函数章节](https://doc.rust-lang.org/edition-guide/rust-2024/newly-unsafe-functions.html)。aimux 的测试注释声称通过 `serial_test` 使环境访问互斥，但 `serial_test` 只约束标注的测试；不能自动证明整个进程没有其他线程。建议把 SAFETY 注释具体化，并避免在库运行期间全局修改环境；优先在启动阶段由应用读取配置。

## Rust 安全补丁时间线（1.85 → 2026-08）

| 日期/版本 | 公告与修复 | 对本项目 |
|---|---|---|
| 2025-02-20 / 1.85.0 | Rust 2024 稳定；发布说明列出上述 unsafe/捕获规则变化。[官方发布](https://blog.rust-lang.org/2025/02/20/Rust-1.85.0/) | **影响**：项目 MSRV/edition 基线正是该版本 |
| 2025-10-01 / 1.89.0 | CVE-2025-11233：1.87–1.88 的 tier-3 `x86_64-pc-cygwin` Path 分隔符处理错误，可能导致路径遍历；1.89 修复。[官方安全公告](https://groups.google.com/g/rustlang-security-announcements/c/oT9zCvLLYkw) | **通常不影响**：项目开发环境为 Linux；除非手工构建/发布 Cygwin target。MinGW 明确不受影响 |
| 2026-05-25 / 1.96.0 | CVE-2026-5222：Cargo 第三方 sparse registry URL 规范化可能泄露 registry credential；1.96 修复。[官方公告](https://blog.rust-lang.org/2026/05/25/cve-2026-5222/) | **条件影响/建议立即修复**：crates.io 不受影响；使用私有/第三方 sparse registry 的 CI 或开发者必须升级到 1.96+ |
| 2026-05-25 / 1.96.0 | CVE-2026-5223：Cargo 提取第三方 registry crate tarball 中 symlink，可覆盖同 registry 其他 crate cache；1.96 修复。crates.io 禁止 symlink，故 crates.io 用户不受影响。[官方公告](https://blog.rust-lang.org/2026/05/25/cve-2026-5223/) | **条件影响/建议立即修复**：第三方 registry 场景升级 1.96+；仅 crates.io 场景风险低 |
| 2026-08-06 | rustup 组件历史显示当前 stable 组件可用至 2026-08-06。[rustup 组件历史](https://rust-lang.github.io/rustup-components-history/) | **建议**：CI 使用锁定的当前 stable/至少 1.96 做安全构建，同时保留 1.85 MSRV job |

调研未找到 1.85 之后针对 `std::env` 或一般 Rust 内存模型的新增 CVE；这不等于不存在未公开/未检索到的问题。CVE-2024-24576/43402 是更早的 Windows batch 参数问题，已在 1.77.2/1.81 修复，不影响 Rust 1.85+；来源：[CVE-2024-24576](https://blog.rust-lang.org/2024/04/09/cve-2024-24576/)、[CVE-2024-43402](https://blog.rust-lang.org/2024/09/04/cve-2024-43402/)。

## RUSTSEC 公告（按关键依赖分组）

以下以 RustSec package/advisory 页面为准，并结合仓库锁文件：`tokio 1.53.1`、`reqwest 0.12.28`、`serde 1.0.229`、`thiserror 2.0.19`、`async-trait 0.1.91`；workspace 声明中 `anyhow = "1"`，但当前 `Cargo.lock` 未找到 anyhow package 条目。

### tokio

- **RUSTSEC-2025-0023**：broadcast channel 并行调用 `Clone` 却只要求 `Send`，当值为 `Send` 但非 `Sync` 时可触发 unsoundness；修复版本包含 `>=1.44.2`。[公告](https://rustsec.org/advisories/RUSTSEC-2025-0023.html)
- 当前锁定 `tokio 1.53.1`，满足修复范围，**不受该公告影响**。仍建议在升级/重解析后跑 `cargo audit`，且不要降级到旧版本。
- RustSec package 页还列出历史 RUSTSEC-2023-0005、2023-0001、2021-0124、2021-0072；当前 1.53.1 已远高于这些修复线。[tokio advisories](https://rustsec.org/packages/tokio.html)

### reqwest

未检索到针对 `reqwest` crate 本身的 RustSec advisory package 记录；搜索结果中的 surf/pingora/quick-xml 是其他 crate，不能归因给 reqwest。[RustSec advisory database](https://rustsec.org/advisories/)、[RustSec database](https://github.com/rustsec/advisory-db)

**建议**：仍需审计 reqwest 的传递依赖（尤其 TLS/backend、URL/HTTP 解析库），不能把“reqwest 无直接公告”当作整棵依赖树无风险。

### serde / serde_json

未发现 `serde` 本体针对本报告时间窗的新 RustSec 公告。搜索到的 `serde-json-wasm`（RUSTSEC-2024-0012）和 `rmp-serde`（RUSTSEC-2022-0092）不是 aimux 直接使用的 `serde`/`serde_json`；`vmm-sys-util`（RUSTSEC-2024-0002）也不是 serde 本体。[RustSec advisory database](https://rustsec.org/advisories/)

**建议**：对不可信 JSON 保持输入大小/深度限制，审计锁文件中的具体 JSON 解析依赖；当前 `serde 1.0.229` 需以 `cargo audit` 的实际 advisory DB 结果为最终判断。

### anyhow

- **RUSTSEC-2026-0190**：`Error::context` 后再调用 `Error::downcast_mut` 会违反借用规则并导致 UB；所有 `<1.0.103` 受影响，`>=1.0.103` 修复。[公告](https://rustsec.org/advisories/RUSTSEC-2026-0190.html)
- 当前锁文件没有 anyhow 条目，无法证明 workspace 声明实际解析版本；若任何成员启用该依赖，建议显式解析到 `>=1.0.103` 并运行 `cargo update -p anyhow`/`cargo audit`。
- **风险判断：条件性但高优先级**。只有使用受影响版本且同时走到该 API 模式才触发，但这是 UB，不能忽略。

### thiserror

未发现 `thiserror` 本体公告；RustSec 搜索结果中的 `failure` 是另一 crate。[RustSec advisory database](https://rustsec.org/advisories/)

### async-trait

未发现 `async-trait` 本体公告；搜索结果中的 `async-coap`、`libp2p-deflate` 等是其他 crate。[RustSec advisory database](https://rustsec.org/advisories/)

> RustSec 的“未发现”结论是基于截至 2026-08-06 的公开 advisory database 页面/搜索结果；依赖树的最终事实应以项目锁文件运行 `cargo audit` 为准。不要仅凭 crate 名称搜索排除传递依赖。

## 对本项目的具体建议

1. **不建议仅因 Edition 2024 升级 `rust-version`**：1.85 是 2024 的最低合理基线，保持 `rust-version = "1.85"` 可服务 MSRV 用户；但安全构建环境不要只安装 1.85。
2. **建议工具链双轨**：保留 Rust 1.85 MSRV CI；增加当前 stable（至少 1.96，实际使用截至 2026-08-06 的 stable）安全/发布 CI。若使用私有 registry，1.96+ 是立即要求，尤其针对 CVE-2026-5222/5223。
3. **立即核实 anyhow**：workspace 声明 `anyhow = "1"`，但当前锁文件未出现，先运行 `cargo tree -i anyhow` 和 `cargo audit`；若被解析，确保 `anyhow >=1.0.103`。不要依赖宽版本约束代替锁文件审计。
4. **tokio 当前锁定版本无需因 RUSTSEC-2025-0023 紧急升级**：1.53.1 已在修复范围；升级时保留锁文件并重跑审计。
5. **审查 `set_var` 的真实并发前提**：保留 unsafe block，但把 SAFETY 说明写成可验证的事实；库 API 不应在 Tokio runtime/HTTP 请求期间修改全局环境。测试中的 `serial_test` 不是对所有线程的全局证明。
6. **执行迁移/安全检查**：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`（若项目约定允许）、`cargo test --workspace`、`cargo audit`；必要时在临时分支运行 `cargo fix --edition`，人工检查 RPIT 捕获、临时 drop 顺序、unsafe attributes/extern/FFI。
7. **审计 FFI 边界**：特别检查 `aimux-ffi`/bindings 的 `unsafe extern`、`#[unsafe(no_mangle)]`、导出 ABI 和指针生命周期；Edition 2024 的语法迁移不能替代 ABI 安全审计。

## 剩余不确定性

- 本次 Web 研究无法替代在当前网络/本地 advisory DB 上执行 `cargo audit`；RustSec 页面部分 crate package 页为 404 或搜索索引不完整。
- Cargo.lock 没有 `anyhow` 条目，但 workspace 中有声明；可能是未被当前成员使用，也可能由未检查的 manifest/feature 路径引入，需命令验证。
- “截至 2026-08-06 无更多 Rust compiler/std CVE”是公开搜索结果范围内的判断，不是 Rust 官方“无漏洞”证明；应持续订阅 Rust security announcements。
