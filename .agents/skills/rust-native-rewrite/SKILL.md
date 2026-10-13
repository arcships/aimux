---
name: rust-native-rewrite
description: Rewrite TS-shaped Rust into native Rust within one repo, keeping behaviour and wire format, proven by an old-vs-new differential test (in-process for one module, cross-build for a cross-cutting refactor).
---

# 单仓库：把 TS 味的 Rust 改成原生 Rust

只在一个 Rust 仓库里工作，不需要上游源码。行为由“新旧实现差分”证明。
仓库有 TS 上游、要和上游本身对比时，用 `ts-rust-port` 的“对比上游”模式。

骨架来自 React Compiler 的移植流程（`compiler-port` + `port-pass` + `compiler-review`）。仓库事实写在配置文件里，本文只写流程、纪律和语法对照。

## 用法

| 参数 | 做什么 |
|---|---|
| `setup` | 只做第 0 步 |
| Rust 文件或模块路径 | 第 0 步（已有配置就跳过）到第 6 步 |
| 候选编号，如 `#3` | 取配置“候选”一节的那一项 |
| `pattern`，加一个模式（如“凭证改成 Secret”） | 跨模块的架构对齐，见“模式模式”一节 |
| `rederive`，加一个旧提交 | 把旧分支上做过的原生化重新做到已经变了的代码上，见“重新推导”一节 |
| `review`，可带 git ref 或范围 | 只做第 5 步；不带参数时审查未提交的改动 |

## 纪律

1. 只改结构，不改行为；发现的 bug 写进契约的“已知差异”，另开 PR。
2. 动手前写契约；改动不能超出契约。
3. 每个 PR 只做一种模式或一个模块。
4. 报告差分结果时写分母：“比较 N 个输入，M 处差异”。
5. 差分失败时一次只修一个，单独跑通再跑全量；不到 0 不停。
6. 0 差异后做一次变异检查，证明 harness 能发现改动。
7. 配置里“项目决定”列出的写法不改，哪怕看起来不像 Rust。
8. 跑完测试看 `git status`，生成器改写的文件不混进提交。
9. 不自动推送；按配置的推送规则处理。
10. 配置里的陷阱先标“待验证”；在真实代码里碰到、有位置和测试证据后，才写成事实。
11. 动手前在基线上跑一遍配置的全部检查，记下每个测试二进制的通过数；改完逐个比较，多出来的只能是新加的测试。改了哪些已有测试单独列出，一个没改最好。
12. 比较前先测稳定性：同一构建跑两次，两次之间就会变的值先找出来屏蔽，屏蔽规则写进报告。
13. 报告覆盖面：比了什么、没比什么、为什么没比，和分母写在一起。
14. 大规模差分先拿一个子集跑通、计时，再定全量规模和时限。

## 第 0 步：setup

找 `.agents/native-rust/PROFILE.md`。存在就读；不存在就按下表查清楚写成这个文件。
每条附依据（路径、命令输出、文档章节），查不到写“未知”。只写模型不看仓库就不可能知道的东西。

| 节 | 要查清的事 |
|---|---|
| 设计文档 | 规定目标架构、已定写法的文档和必读章节；没有就写“无”，列出 AGENTS.md、README 里起同样作用的段落 |
| 不能动的面 | 公开 JSON、生成的类型、C ABI、语言绑定、录制格式、断言过的错误文本；每类写位置和守护它的检查命令 |
| 预言机 | 仓库里已有的、能证明行为不变的测试（fixture 回放、cassette、golden）；新旧差分 harness 放哪、怎么放大规模；仓库对测试的限制 |
| 检查 | CI 实际跑的命令（读 CI 配置和 justfile/Makefile）；哪些会改写文件 |
| 平台 | CI 覆盖的目标（操作系统、大端、wasm 等）和因此不能动的实现分支 |
| 项目决定 | 已经拍板、看起来不像 Rust 但不能改的写法，各附出处 |
| 已知 bug | 已登记、不在重写里顺手修的 bug |
| 陷阱 | 这个仓库里真正会咬人的语义差异，写到字段和文件；没实例的标“待验证” |
| 工作约束 | PR 规则、基于哪个分支、能不能 breaking 和旧 API 怎么处理、推送方式、环境限制 |
| 范例模块 | 仓库里写得最原生的模块 |
| 参照库 | 作为写法标准的 Rust 库，锁 commit；每个模式写到文件和行号 |
| 候选 | 用“TS 痕迹扫描”挑出的 10 个左右：位置、痕迹、原生写法、影响 wire 吗、和项目决定冲突吗；误报也记下 |

“项目决定”和“不能动的面”请用户确认，确认前先按查到的继续。

## 模式模式（跨模块）

对象是一个写法模式（如“凭证改成脱敏的 newtype”“回调改成 trait”），不是一个文件。第 2 到 6 步照常，以下几处不同：

1. 先对照设计文档和“项目决定”。被拍板挡住的模式不写代码，产出一份修改建议（现状、参照库的写法和行号、要改的决定、代价），交给用户转给维护者。
2. 列出这个模式在仓库里的全部出现位置，契约写清楚这次改哪些、为什么留下其余的。
3. 依据写“参照库”里的文件和行号；参照库之间不一致时都列出来，不挑一个当标准。
4. 预言机：纯函数用进程内差分；provider 行为用已有的 fixture、cassette 回放，再加跨构建差分；公开 API 变了，按“工作约束”的 breaking 规则，连同 FFI 和各语言绑定在同一个 PR 里改完。
5. 一个 PR 只做一个模式。

## 差分怎么搭

| 场景 | 做法 |
|---|---|
| 一个模块、纯函数或内部状态机 | 进程内差分：旧实现原样复制进 `tests/<模块>_diff_test.rs` 的 `mod legacy`，随机生成器，默认 2 万个，环境变量放大到 10^6。harness 入库 |
| 跨很多文件的结构重构（配置、流、请求构造），旧实现无法单独复制 | 跨构建差分，见下 |

跨构建差分：

1. 新旧两个提交各开一个 worktree，各用自己的 `CARGO_TARGET_DIR`。
2. 写一个探针测试文件，两边放同一份：从仓库已有的录制（cassette、fixture）推出调用参数，对每个能讲那种 wire 的公开入口都跑一遍；流式的再按"整段、带原始块、按小块切、切一半后传输出错"几种方式跑。
3. 注入点放在最底层的传输（`fetch`），用一个会记录请求、按录制回答的传输，被改的配置、URL、headers 都在路径上；再设一个死代理，漏出去的请求直接失败，不会联网。
4. 每个操作输出一行 JSONL，key 写成“录制 | 入口 | 操作”，记请求和结果。
5. 同一进程内每个操作跑两次，两次之间变的值（计数器、UUID、签名、当前时间）换成占位符；先拿同一构建的两次全量结果比，确认屏蔽后为 0。
6. 新旧结果用脚本逐 key 比较，报告“比较 N 个请求 / N 条流 / N 个结果，M 处差异”，每类差异给一个例子；再列没覆盖的录制和原因。
7. 变异检查：在新构建里故意改坏一处，确认只在预期的字段上报差异，改回。
8. 探针和脚本不入库时，在 PR 描述写明位置和命令。配置的“预言机”一节给出现成的实现。

## 重新推导（rederive）

对象是一个旧提交：它在旧的代码上做过原生化，现在代码变了，要把同样的结构改动做到新代码上。

1. 把旧提交逐块分成四类：结构、改行为、改公开 API、新功能。只有“结构”一类搬过来；其余每块写一行没搬的原因。
2. 旧提交删掉或收窄的东西，如果配置“项目决定”要求保留，就保留。
3. 契约照第 3 步写，“已知差异”里写明新旧两版在哪里本来就不同。
4. 验证用跨构建差分，基线是新代码改动前的提交，不是旧提交。
5. PR 描述单列“没搬的部分”。

## 第 1 步：校验目标

1. 目标存在，并且有能覆盖它的测试；没有就先补测试，单独提交。
2. 对目标跑“TS 痕迹扫描”，逐项对照“项目决定”和“不能动的面”，剔除不该改的。
3. 剩下的为空，就报告“没有要改的”并停下。

## 第 2 步：读上下文

配置及其指定的设计文档章节；目标模块、测试、调用方（`rg`）；碰到的“不能动的面”对应的代码和测试；范例模块。

## 第 3 步：计划和契约

契约（放 PR 描述或 commit body）：对齐单位；不能变的东西；允许的差异；已知差异；测量方式和规模；是否 breaking 和旧 API 怎么处理。
计划：每一项“改前 → 改后”，以及按“项目决定”保留的项。
发给用户后直接进第 4 步；只有要 breaking、要改“不能动的面”或违背“项目决定”时先等回复。

## 第 4 步：实现（子 agent）

```
原生化重写 <Rust 文件>。已附：配置；设计文档相关章节；目标模块、测试、调用方；范例模块；契约和计划；
rust-native-rewrite skill 的“语法对照”。

1. 先把新 API 的调用写成 doc test。
2. 建差分 harness（按“差分怎么搭”选）：进程内的，旧实现原样复制进 tests/<模块>_diff_test.rs 的 mod legacy，
   生成器覆盖空输入、截断、乱序、重复、深嵌套、错误路径、非 ASCII 和辅助平面字符，默认 2 万个，环境变量放大到 10^6；
   跨构建的，先测稳定性，再拿子集计时。
3. 按语法对照重写。公开名字保持不变，除非契约允许 breaking。
4. 循环到 0 差异：有失败只挑一个，单独跑，修根因，再跑全量。
5. 变异检查：故意改坏一个分支，确认 harness 报差异，改回。
6. 跑配置里的检查命令，看 git status。

不推送，不改契约外的文件，不修契约外的 bug。报告：改了哪些文件；基线和改后每个测试二进制的数；改了哪些已有测试；
“比较 N 个输入，M 处差异”和没覆盖的部分；变异检查结果；每项结构变化；新发现的已知差异。
```

## 第 5 步：审查（子 agent，循环）

取 diff：无参数 `git diff HEAD`（为空看 `--cached`）；单个 ref `git diff <ref>^!`；范围直接用。启动审查子 agent，有“本次引入”的问题就交回第 4 步修，再审，直到没有。

```
审查一个原生化重写。先读配置和设计文档相关章节。只看改动的行和紧邻上下文。

- BEHAVIOUR：和旧实现行为一致（输出、错误、边界情况）。
- WIRE：“不能动的面”没变。
- DECISION：没违背“项目决定”和“工作约束”（breaking、旧 API 处理、提交格式、CHANGELOG）；没顺手修“已知 bug”。
- ORACLE：差分 harness 和变异检查可复现（入库，或在描述里写明位置和命令）；数字带分母和覆盖面；改过的已有测试逐个说明理由。
- NATIVE：对改动文件跑“TS 痕迹扫描”，列出还剩的；为编译通过而加的 clone/unwrap 也算。
- ERROR：生产代码没有新的 unwrap/expect/panic；错误类型符合配置。
- SECURITY：Debug 不打印密钥；读环境变量的构造不 panic；配置列的安全底线守住。
- SCOPE：一种模式或一个模块；没有契约外的文件。

不算问题：所有权模型要求的偏离；配置没规定的风格偏好；补注释的建议。

输出：
N. [类别] path:line — 问题
   Expected: …（附出处）
   Found: …
   来源: 本次引入 | 旧实现已有
没有问题写 "No issues found."
```

## 第 6 步：检查、提交、汇报

跑配置的全部检查，看 `git status`。提交 `refactor(<crate>)[!]: rewrite the <module> in idiomatic Rust`，body 写契约、“比较 N 个输入，M 处差异”、变异检查结果、每项结构变化一行。API 变化写进 CHANGELOG。按配置的推送规则处理。

PR 描述用仓库的模板。Notes for reviewers 写：位置（栈里的上一层）、方向（只改结构）、没动的面、没搬的部分（rederive）。最后一张 Verification 表，审查者只看这一张：

| Check | Result |
|---|---|
| 基线测试 | N 个测试二进制：通过 / 失败 / 忽略 |
| 改后测试 | 同上，并写明多出来的是哪几个新测试 |
| 上游 fixture 回放 | 每个回放测试的通过数，和基线一样 |
| 测试改动 | 无；或逐个列出和理由 |
| 新旧差分 | 比较 N 个，M 处差异；没覆盖的部分 |
| 变异检查 | 改了哪一行，报了几处，是否只在预期字段 |
| 门禁 | 配置里的每个检查；本地工具链和 CI 不同造成的提示写明 |
汇报：改了什么、按项目决定保留了什么、差分规模和差异数、审查结论。仓库有上游时，提醒可以再跑一次 `ts-rust-port` 的“对比上游”。

## 语法对照：TS 形状 → 原生 Rust

只列“改成什么”。每项先对照配置的“项目决定”。

| TS 形状在 Rust 里的样子 | 原生写法 |
|---|---|
| struct 里一个 `kind`/`role`/`state` 字段，其他字段随它变 | enum，每个变体带自己的数据 |
| 几个 `bool` 组合成的状态；`finished` 标志 | enum 变体；终结操作取 `self`（`finish(self)`） |
| `-1`、空字符串等哨兵；`isize` 下标 | `Option`、`usize` |
| `kind: String`、和字符串字面量比较 | enum + serde rename |
| `#[serde(untagged)]` 表示的输入联合 | 内部只存一种形状，其余走 `From`/`impl Into`；形状属于 wire 契约时保留 |
| `extra: Value`、`as_object()`/`.get("x")` | 有类型的字段；真正开放的部分用 `#[serde(flatten)]` 的具名字段 |
| `Option<Vec<_>>`，缺失和空没有区别 | `Vec` + `Default` |
| `Option<f64>` 表示计数、下标、token | 整数类型；会写回 JSON 的加 golden 测试 |
| 扁平状态栈加跨状态共享的计数器 | `Vec<Frame>`，`Frame` 是带数据的 enum；起止状态用控制流 |
| `Vec<char>` 加下标扫描 | `char_indices()` 和字节偏移 |
| `expect("never empty")` 维持的不变量 | 用类型让非法状态构造不出来 |
| options 里的 `Box<dyn Fn>` 钩子 | trait，加一个给闭包的 blanket impl，原来的 `new(closure)` 照常能用，不算 breaking；只给测试用的钩子删掉 |
| 每次调用返回新 `Vec` | 写进调用方给的 `&mut Vec` |
| 小集合配 `HashMap` 索引 | `Vec` 线性扫描 |
| 自持数据的输入 | 借用（`Delta<'a>`） |
| `createX(settings)`，settings 全是 `Option` | `X::new(必填)` + consuming setter；`create_x` 留作薄包装 |
| 全局单例里 `expect` | 失败延到使用时，或返回 `Result` |
| 函数接收某 struct 的几个字段再加 bool 开关 | 那个 struct 的方法；bool 改 enum |
| `Result<_, ()>`、`Other(String)`、`String` 错误 | `thiserror` enum，结构化字段，`#[from]` 保留 source |
| 公开的 `pub stream:` 字段 | 类型自己实现 `Stream`，加 `finish(self)` |
| JS 式 `AbortSignal` | drop 取消；`AbortSignal` 只留在 FFI 层 |
| 只有一个实现的 trait | struct |
| 插入顺序有意义的 map/set 用了 `HashMap` | `IndexMap`/`IndexSet` |
| 公开类型没有 `#[non_exhaustive]` | 加上（先给被字面量构造的类型加构造器） |
| 凭据类型 `derive(Debug)` | 手写脱敏的 `Debug` |

## TS 痕迹扫描

```sh
P=path/to/target_module.rs   # 只扫目标模块；整个 crate 一起扫噪音太大
rg -n '\w+: bool\b|let mut \w+ = (true|false);' "$P"        # 靠布尔维持的状态
rg -n 'isize|-1\b|Vec<char>|\.chars\(\)\.collect' "$P"        # JS 下标、-1 哨兵、UTF-16 思维
rg -n '\.expect\(|\.unwrap\(|unreachable!|panic!' "$P"        # 运行时断言
rg -n 'Option<(Vec|HashMap|BTreeMap)<' "$P"                   # 空集合能表达？约一半误报
rg -n '(type|kind|role|model_type|method)\w*: (String|&str)' "$P"  # 本该是 enum 的字符串
rg -n '== "\w+"|as_str\(\) == ' "$P"                          # 字符串字面量比较
rg -n 'pub \w+: (serde_json::)?Value|as_object\(\)|\.get\("' "$P"  # 无类型 JSON 袋子
rg -n -- '-> Option<Result<|Result<[^,>]+, \(\)>|ok_or\(\(\)\)|Err\(\(\)\)' "$P"  # 可选方法直译、无信息的错误
rg -n -C2 'pub \w+: Option<' "$P" | rg 'state|kind|status'    # 状态字段 + 依赖它的 Option
rg -n '#\[serde\(untagged\)\]|Other\(String\)' "$P"           # 联合直译、兜底字符串错误
rg -n '(Box|Arc)<dyn Fn|pub type \w+ = (Arc|Box)<dyn' "$P"    # 从 options 搬来的钩子
rg -n 'pub fn create_\w+\(|OnceLock|pub fn call\(' "$P"       # createX、全局单例、可调用对象
rg -n 'pub stream:|AbortSignal' "$P"                          # 公开的流字段、JS 式取消
rg -n 'fn \w+\(&mut self[^)]*\) -> (Result<)?Vec<' "$P"       # 热路径每次返回新 Vec
rg -n ': Option<f64>' "$P"                                    # JS number 直译
```

## rig 里的对应写法（commit 9e7cb60）

`git clone --depth 1 https://github.com/0xPlaygrounds/rig`

| 需要 | 位置 |
|---|---|
| 按角色划分的消息 enum | `crates/rig-core/src/completion/message.rs:13-25` |
| 请求类型：`new(必填)`、consuming setter、`#[non_exhaustive]` | `crates/rig-core/src/completion/request.rs:867-921, 1105, 1130-1403` |
| `IntoFuture` builder | `crates/rig-agent/src/agent/runner.rs:795-797` |
| 纯数据配置 + `Secret` + `from_env()` | `crates/rig-core/src/providers/openai/wire.rs:526-627` |
| 同步 decoder 写进调用方的 sink | `crates/rig-core/src/wire.rs:704-724` |
| 流的 `finish(self)` | `crates/rig-core/src/streaming/mod.rs:156-163` |
| 关联类型的 provider 选项，保留 `raw` | `crates/rig-core/src/completion/provider_options.rs:43-56` |
| 统一的 `#[non_exhaustive]` 错误 | `crates/rig-core/src/error.rs:322-451` |
| 钩子用 trait | `crates/rig-agent/src/agent/hook.rs` |
| 流式回复的拼装（各家共用的写入器，按 wire 序号拼文本、推理、tool call） | `crates/rig-core/src/operation/completion.rs:1455` `Out::fragment`。和 AI SDK 的 `StreamingToolCallTracker` 做同一件事，但行为不同：参数有 32MB 上限，同一序号来了新 id 时按参数是否完整猜是不是新 call，接受不带序号的 fragment 和先发字面量 `null` 的网关。项目对齐 AI SDK 时不照搬 |
