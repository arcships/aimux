---
name: ts-rust-port
description: Port a TypeScript module to Rust, fix one place where a Rust port disagrees with its pinned TS upstream, or check a Rust module against that upstream, using upstream tests, recorded fixtures and running upstream TS.
---

# TS 上游和它的 Rust 移植

两个仓库：锁定版本的 TS 上游和 Rust 仓库，目标是 Rust 的行为和上游一致。把移植来的代码改成原生写法是另一件事，用 `rust-native-rewrite`。
仓库事实在 `.agents/native-rust/PROFILE.md`（下称配置）。

| 用法 | 做什么 |
|---|---|
| 上游文件，或还没移植的模块 | 移植 |
| `fix <一处不一致>` | 把一处行为改成和上游一样（最常见） |
| `parity <Rust 模块>` | 和上游对比，只报告 |
| `review [ref]` | 只审查 |

## 纪律

1. 移植时结构跟上游走（文件、类型、函数一一对应），不改进、不修 bug；已经移植过的代码只走 `fix` 或 `parity`。
2. 说“上游是这样”要给锁定版本的文件和行号。
3. 上游测试逐个用例搬，只改 harness，不改场景。
4. 报告写分母：“比较 N 个，M 处差异”。失败一次修一个，到 0 为止。
5. 上游源码下载到新的空目录，当数据用：路径作参数传给 node，不在那个目录里执行东西。
6. 配置“项目决定”里和上游刻意不同的地方，不改回上游。
7. 不自动推送。

## fix

1. 找到锁定版本的上游文件和行号；在“项目决定”里的就不改，报告并停下。
2. 先写一个在当前代码上失败的测试：优先回放上游录制的 fixture，其次搬上游用例，再其次用 node 跑上游拿期望值。
3. 改到通过，只改这一处；顺带看到的其他差异记下来。
4. 跑配置的检查，基线和改后逐个测试二进制比。
5. 提交 `fix(<crate>): <行为> matches <包>@<版本>`，body 写上游出处、改前改后、新测试名。同一层的几条可以同一个 PR，每条一个提交。

## 移植

1. 用配置的映射把目标解析成“Rust 文件 + 上游文件 + 版本”；查清碰到哪些“不能动的面”和“项目决定”。
2. 契约：上游文件和版本、不能变的、允许的差异、怎么测、是否 breaking。只有要 breaking 或违背配置时先等用户回复。
3. 实现（子 agent）：按上游结构写，只在所有权模型要求时偏离；上游测试逐个搬；循环到 0 失败；再做一次对比上游。常见失败：缺 match 分支、迭代顺序、null 和缺失混同、数字类型、UTF-16 下标、JSON 键顺序、错误文本。
4. 审查（子 agent，循环）：对每个改动的文件读完对应的上游文件，查 FIDELITY（数据流、边界、错误分类和上游一致）、WIRE、DECISION、ORACLE（用例搬全、引用带版本行号）、ERROR（错误名和消息保留，不新增 panic）、SCOPE。报告按 `rust-native-rewrite` 的审查报告格式写，规则出处写上游文件和行号。
5. 提交 `feat(<crate>): port <upstream file>`，body 写搬了多少用例、对比上游的 N/M。

## 对比上游

按能用的顺序：上游录制的 fixture 回放；node 跑锁定版本上游、读同一份 JSONL 语料逐条比；搬来的上游测试（最弱）。
语料覆盖空输入、截断、乱序、重复、深嵌套、错误路径、非 ASCII 和辅助平面字符，以及配置“陷阱”里的每一项。每类差异给一个最小复现，标明是已知的还是新发现的。runner 不入库，commit body 写位置和命令。

## 评测

`evals/` 里的用例给一份锁定版本的上游文件、一个带一处不一致的 Rust 移植和逐条打分标准（`graders/`），格式是 `claude plugin eval` 读的那种。改了本文件就跑一遍：

```sh
cd .agents/skills/ts-rust-port
claude plugin eval . --allow-tools Write Edit Bash --scaffold
```

这是真实的模型运行，会计费；`--runs 1` 只跑一轮。

