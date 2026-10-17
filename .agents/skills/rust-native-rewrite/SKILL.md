---
name: rust-native-rewrite
description: Rewrite TS-shaped Rust into native Rust within one repo, keeping behaviour and wire format, proven by an old-vs-new differential (in-process for one module, cross-build for a cross-cutting refactor).
---

# 把 TS 味的 Rust 改成原生 Rust

单仓库，不需要上游源码；行为由新旧差分证明。要和上游本身对比，用 `ts-rust-port`。
仓库事实在 `.agents/native-rust/PROFILE.md`（下称配置）；本文只写流程、纪律和写法对照。

用法：给一个文件或模块；或 `pattern <模式>`（跨模块，如“回调改成 trait”）；或 `review [ref]`（只审查）。

## 纪律

1. 只改结构，不改行为；看到的 bug 记下来，另开 PR。
2. 一个 PR 一种模式或一个模块。配置“项目决定”里的写法不改。
3. 动手前在基线上跑一遍配置的检查，记下每个测试二进制的通过数；改完逐个比，多出来的只能是新测试，改过的已有测试逐个说明。
4. 差分报告写分母和覆盖面：“比较 N 个，M 处差异；没比的是 X，因为 Y”。失败一次修一个，到 0 为止；0 之后做变异检查。
5. 先测稳定性：同一构建跑两次，会变的值（计数器、UUID、时间、签名）先屏蔽。
6. 改完要比原来短或一样长。为了“原生”多出来的样板（字段写两遍、套一层通用框架）不算改进。
7. 去掉闭包时不能颠倒依赖：共享层不为了一个闭合 enum 去 import 具体模块；跨模块扩展点用共享层的 trait，具体模块用纯数据 struct 实现。
8. 跑完看 `git status`；临时拷进去的探针和生成器改写的文件不进提交。不自动推送。

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
6. **提交和汇报**：`refactor(<crate>)[!]: …`，body 写契约、差分 N/M、变异检查。PR 描述最后放一张 Verification 表：基线和改后测试数、测试改动、差分、变异检查、各项检查。

## 写法对照

| TS 形状 | 原生写法 |
|---|---|
| 一个 `kind`/`role`/`state` 字段，其余字段随它变 | enum，每个变体带自己的数据 |
| 几个 `bool` 组成的状态；`finished` 标志 | enum 变体；终结操作取 `self` |
| `-1`、空字符串哨兵；`isize` 下标；`Vec<char>` 扫描 | `Option`、`usize`、`char_indices()` |
| 和字符串字面量比较的 `kind: String` | enum + serde rename |
| `extra: Value`、`.get("x")` | 有类型字段；开放部分用 `#[serde(flatten)]` |
| `Option<Vec<_>>`，缺失和空无区别 | `Vec` + `Default` |
| `expect("never empty")` 维持的不变量 | 让非法状态构造不出来 |
| options 里的 `Box<dyn Fn>` 钩子 | trait + 给闭包的 blanket impl，`new(closure)` 照常能用 |
| 包内有限几种配置闭包（`url()`、`headers()`） | 包内纯数据 enum，请求时求值 |
| 共享层给各具体模块开的扩展点 | 共享层的 trait，具体模块的纯数据 struct 实现 |
| `new TransformStream({ start, transform, flush })`、靠一堆局部变量维持的流状态 | 实现 `Transformer` 的具名 struct；只读上下文和 `#[derive(Default)]` 的累积状态分开；同步驱动入口方便无 I/O 单测 |
| 外面套一个通用的 tracker/accumulator 对象 | 状态直接作为使用者 struct 的字段，逻辑是它的私有方法 |
| 把 `Transformer` 接到流上的 `async_stream!` + 每帧新 `Vec` | 直接实现 `futures::Stream` 的 struct，复用一个 `VecDeque` |
| 每次调用返回新 `Vec` | 写进调用方给的 `&mut Vec` |
| 小集合配 `HashMap` 索引 | `Vec` 线性扫描 |
| `createX(settings)`，settings 全 `Option` | `X::new(必填)` + consuming setter |
| `Result<_, ()>`、`Other(String)` 错误 | `thiserror` enum，`#[from]` 保留 source |
| 只有一个实现的 trait | struct |
| 插入顺序有意义却用 `HashMap` | `IndexMap` |
| 凭据类型 `derive(Debug)` | 手写脱敏 `Debug` |

## 痕迹扫描

```sh
P=path/to/module.rs   # 只扫目标
rg -n '\w+: bool\b|let mut \w+ = (true|false);|isize|-1\b|Vec<char>' "$P"
rg -n '\.expect\(|\.unwrap\(|unreachable!|panic!' "$P"
rg -n 'Option<(Vec|HashMap)<|(type|kind|role)\w*: (String|&str)|== "\w+"' "$P"
rg -n 'pub \w+: (serde_json::)?Value|as_object\(\)|\.get\("|#\[serde\(untagged\)\]|Other\(String\)' "$P"
rg -n '(Box|Arc)<dyn Fn|pub fn create_\w+\(|pub stream:|AbortSignal|: Option<f64>' "$P"
rg -n 'use crate::(vertex|bedrock|google|anthropic|openai|aws_polly)' aimux-providers/src/shared   # 反向依赖
```

rig（`0xPlaygrounds/rig`）里可对照的：按角色的消息 enum（`completion/message.rs`）、请求 `new` + setter + `#[non_exhaustive]`（`completion/request.rs`）、纯数据 provider 配置和 `Secret`（`providers/openai/wire.rs`）、流的 `finish(self)`（`streaming/mod.rs`）、钩子 trait（`agent/hook.rs`）。
