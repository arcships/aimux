---
name: rust-native-rewrite
description: Rewrite TS-shaped Rust into native Rust within one repo, keeping behaviour and wire format, proven by an old-vs-new differential (in-process for one module, cross-build for a cross-cutting refactor); also review such code or a rewrite diff and write a graded report.
---

# 把 TS 味的 Rust 改成原生 Rust

单仓库，不需要上游源码；行为由新旧差分证明。要和上游本身对比，用 `ts-rust-port`。
仓库事实在 `.agents/native-rust/PROFILE.md`（下称配置）；本文只写流程、纪律和写法对照。

用法：给一个文件或模块；或 `pattern <模式>`（跨模块，如“回调改成 trait”）；或 `review [ref]`（只审查，按下面的报告格式写，不改代码）。

## 纪律

1. 只改结构，不改行为；看到的 bug 记下来，另开 PR。
2. 一个 PR 一种模式或一个模块。配置“项目决定”里的写法不改。
3. 动手前在基线上跑一遍配置的检查，记下每个测试二进制的通过数；改完逐个比，多出来的只能是新测试，改过的已有测试逐个说明。
4. 差分报告写分母和覆盖面：“比较 N 个，M 处差异；没比的是 X，因为 Y”。失败一次修一个，到 0 为止；0 之后做变异检查。
5. 先测稳定性：同一构建跑两次，会变的值（计数器、UUID、时间、签名）先屏蔽。
6. 行数不是判定标准。为了“原生”多出来的样板（字段写两遍、套一层通用框架）不算改进；为写明契约、消除歧义加的声明（具名 struct、enum、类型标注）可以加，PR 里说明。
7. 去掉闭包时不能颠倒依赖：共享层不为了一个闭合 enum 去 import 具体模块；跨模块扩展点用共享层的 trait，具体模块用纯数据 struct 实现。
8. 写法对照里标“有条件”的行，只在条件成立时改，不按语法出现次数批量替换。
9. 跑完看 `git status`；临时拷进去的探针和生成器改写的文件不进提交。不自动推送。
10. 扫描数量、路径和待办写在 issue 和 PR 里；稳定下来的决定才写进配置。

## 流程

1. **选目标**：对目标跑下面的痕迹扫描，剔除“项目决定”和“不能动的面”挡住的；剩下为空就报告并停下。目标没有测试先补测试，单独提交。
2. **契约**（commit body 或 PR 描述）：改哪些、不能变什么、允许的差异、怎么测、是否 breaking。只有要 breaking、要动“不能动的面”或违背“项目决定”时先等用户回复。
3. **差分**：
   - 一个模块、纯函数或状态机：进程内差分。改动期间把旧实现复制进测试，随机生成器默认 2 万、可放大到 10^6；到 0 后，把有代表性的输入和旧实现的结果录成 fixture 入库，删掉旧实现副本。
   - 跨很多文件（配置、流、请求构造）：跨构建差分。新旧两个 worktree 各用自己的 target；同一个探针测试文件放两边，在最底层传输（`fetch`）注入一个按录制回答、记录请求的传输，加一个死代理防联网；每个操作一行 JSONL，脚本逐 key 比较。探针不入库，PR 描述写位置和命令。
4. **实现**：先把新 API 的调用写成 doc test，再按写法对照改。公开名字不变，除非契约允许 breaking。
5. **审查**（子 agent，有“本次引入”的问题就回第 4 步）：
   - BEHAVIOUR：输出、错误、边界情况和旧实现一致。
   - WIRE：“不能动的面”没变。
   - DECISION：没违背“项目决定”和工作约束；没顺手修 bug。
   - LAYERING：没有新增 `shared → 具体模块` 之类的反向依赖。
   - SIZE：行数没有无故增加；没有为编译而加的 clone/unwrap。
   - ORACLE：差分可复现，数字带分母；变异检查做了。
   - SECURITY：Debug 不打印密钥；读环境变量不 panic。
   - UNSAFE：每个 unsafe 说明为什么不能用 safe 写法；块前 `// SAFETY:` 写指针来源、有效期、别名、所有权和线程要求为什么成立；`unsafe fn` 的 `# Safety` 写调用方要满足的条件；safe 函数不能把不安全前提留给调用方。说明要对应代码，不用模板。
   - LOCK：锁 poison 按配置的项目决定处理；持锁时不调用户闭包，不跨 `.await` 持锁，不从公开方法返回 guard。
6. **提交和汇报**：`refactor(<crate>)[!]: …`，body 写契约、差分 N/M、变异检查。PR 描述最后放一张 Verification 表：基线和改后测试数、测试改动、差分、变异检查、各项检查。

## 审查报告格式

`review` 和第 5 步都按这个写：

```markdown
# 审查：<范围>（<ref>）

<一段：改了什么、结论、最要紧的一条；跑了哪些检查>

## 必须改
### 1. <标题>（`file.rs:42`）
**规则：** <写法对照的哪一行 / 配置的哪条项目决定 / clippy lint 名 / 上游文件和行号>
**问题：** <错在哪、后果>
**改法：** <只写要改的几行代码>

## 应该改
## 可以考虑
## 已经做得好的
- <两到四条，具体到写法>
```

必须改：行为或 wire 变了、违背项目决定、unsound、可达的 panic。应该改：能用但违反有代价的规则。可以考虑：命名、风格。每条都要位置和规则出处；同类问题合成一条列出位置；看起来是有意为之的写法放“可以考虑”，写清取舍。

## 写法对照

| TS 形状 | 原生写法 |
|---|---|
| 一个 `kind`/`role`/`state` 字段，其余字段随它变 | enum，每个变体带自己的数据 |
| 几个 `bool` 组成的状态；`finished` 标志 | enum 变体；终结操作取 `self` |
| `-1`、空字符串哨兵；`isize` 下标；`Vec<char>` 扫描 | `Option`、`usize`、`char_indices()` |
| 和字符串字面量比较的 `kind: String` | enum + serde rename |
| 参数 `&String`、`&Vec<T>`、`&Box<T>`、`&Option<T>` | `&str`、`&[T]`、`&T`、`Option<&T>`；小的 `Copy` 值直接传值；要改写调用方状态的 `&mut Option<T>` 不算；改公开签名时 PR 写明源码兼容影响 |
| 一组开关作位置参数一起传（上游多半是对象参数） | 照上游对象参数做具名参数 struct；能力类开关并进已有的能力 struct |
| 同一个派生事实（`is_reasoning_model` 之类）沿转换调用链逐个函数往下传 | 转换开始时建一个上下文 struct（或转换器 struct 持有这些字段），函数取 `&Ctx` |
| 调用处单个 `true`/`false` 字面量；同类型的三元组返回值 | 两值 enum、两个方法，或私有具名 struct；公开签名一律不用 `bool` 参数，setter 也一样（1.0 前允许 breaking，同一 PR 改完调用方和绑定）；只有调用点几行内可见的私有函数、语义明确的局部 tuple 不改 |
| `getXxx()` 式的 getter | 去掉 `get_`，`fn len(&self)`；上游叫 `getXxx` 也去掉 |
| `#[allow(lint)]` | `#[expect(lint, reason = "...")]`，放在最小的项上；生成代码和宏输出可以用 `allow` |
| `extra: Value`、`.get("x")` | 有类型字段；开放部分用 `#[serde(flatten)]` |
| `Option<Vec<_>>`，缺失和空无区别 | `Vec` + `Default` |
| `expect("never empty")` 维持的不变量 | 让非法状态构造不出来 |
| options 里的 `Box<dyn Fn>` 钩子 | trait + 给闭包的 blanket impl，`new(closure)` 照常能用 |
| 包内有限几种配置闭包（`url()`、`headers()`） | 包内纯数据 enum，请求时求值 |
| 共享层给各具体模块开的扩展点 | 共享层的 trait，具体模块的纯数据 struct 实现 |
| `new TransformStream({ start, transform, flush })`、靠一堆局部变量维持的流状态 | 实现 `Transformer` 的具名 struct；只读上下文和 `#[derive(Default)]` 的累积状态分开；同步驱动入口方便无 I/O 单测 |
| 外面套一个通用的 tracker/accumulator 对象 | 状态直接作为使用者 struct 的字段，逻辑是它的私有方法 |
| 把 `Transformer` 接到流上的 `async_stream!` + 每帧新 `Vec` | 直接实现 `futures::Stream` 的 struct，复用一个 `VecDeque` |
| 每次调用返回新 `Vec` | 有条件：热路径且有实测时，写进调用方给的 `&mut Vec` |
| 小集合配 `HashMap` 索引 | 有条件：集合确实小且顺序无关时，`Vec` 线性扫描 |
| 为过借用检查加的 `clone`、`Arc` | 有条件：先试 `let` 延长临时值、`mem::take`/`Option::take`、拆 struct；数据小且不可变时有意 clone 可以留 |
| 手写循环 | 有条件：管道短、自上而下读得通时用迭代器；循环体做几件事或要提前返回时保留 `for`；`for` 头里的长链先绑到具名变量 |
| `createX(settings)`，settings 全 `Option` | `X::new(必填)` + consuming setter |
| `Result<_, ()>`、`Other(String)` 错误 | `thiserror` enum，`#[from]` 保留 source |
| 只用来组织代码、只有一个实现、没有泛型或 `dyn` 用途的 trait | struct 的固有方法；用于泛型、`dyn`、扩展 trait、跨层依赖反转的保留 |
| 插入顺序有意义却用 `HashMap` | `IndexMap` |
| 凭据类型 `derive(Debug)` | 手写脱敏 `Debug` |
| 手写的 `Default`、`Debug`、`Clone` 等改成 derive | 有条件：先查 derive 后默认值是否一样、是否需要脱敏、泛型上生成的 trait bound 是否等价；bound 写在 impl 上，不写在 struct 定义上 |
| 混合层级的 `use`、业务代码的 glob 导入 | 同一路径一条 `use`；类型和 trait 直接用，跨模块自由函数带模块前缀（`mem::take`）；glob 只留测试里的 `use super::*`、函数内的 `use Enum::*`，napi/PyO3 等框架 prelude 可以 |
| 要跨线程的公开类型没有检查 | 编译期断言 `Send`（`const _: () = { const fn f<T: Send>() {} f::<X>(); };`） |

## 痕迹扫描

```sh
P=path/to/module.rs   # 只扫目标
rg -n '\w+: bool\b|let mut \w+ = (true|false);|isize|-1\b|Vec<char>' "$P"
rg -n '\.expect\(|\.unwrap\(|unreachable!|panic!' "$P"
rg -n 'Option<(Vec|HashMap)<|(type|kind|role)\w*: (String|&str)|== "\w+"' "$P"
rg -n 'pub \w+: (serde_json::)?Value|as_object\(\)|\.get\("|#\[serde\(untagged\)\]|Other\(String\)' "$P"
rg -n '(Box|Arc)<dyn Fn|pub fn create_\w+\(|pub stream:|AbortSignal|: Option<f64>' "$P"
rg -n '&(Option|Vec|String|Box)<?|#\[allow\(|use [\w:]+::\*;|\.lock\(\)\.(unwrap|expect)|into_inner\)|unsafe \{' "$P"
rg -n 'use crate::(vertex|bedrock|google|anthropic|openai|aws_polly)' aimux-providers/src/shared   # 反向依赖
```

rig（`0xPlaygrounds/rig`）里可对照的：按角色的消息 enum（`completion/message.rs`）、请求 `new` + setter + `#[non_exhaustive]`（`completion/request.rs`）、纯数据 provider 配置和 `Secret`（`providers/openai/wire.rs`）、流的 `finish(self)`（`streaming/mod.rs`）、钩子 trait（`agent/hook.rs`）。

## 评测

`evals/` 里每个用例有提示、期望结果和逐条打分标准（`graders/`），格式是 `claude plugin eval` 读的那种。改了本文件就跑一遍，带技能和不带技能各一组：

```sh
cd .agents/skills/rust-native-rewrite
claude plugin eval . --allow-tools Write Edit Bash --scaffold
```

这是真实的模型运行，会计费；`--runs 1` 只跑一轮。用例的脚手架会把配置复制进临时目录，评测的是配置加本文件。

