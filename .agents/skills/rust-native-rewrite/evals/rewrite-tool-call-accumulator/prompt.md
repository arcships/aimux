---
tags: [rust-native-rewrite]
max_turns: 40
timeout_seconds: 1500
allowed_tools: [Read, Glob, Grep, Skill, Write, Edit, Bash]
---

src/lib.rs 是从 TS 移植来的工具调用累积器，写法还是 TS 的样子。请用 rust-native-rewrite 把它改成原生 Rust。

要求：JSON 输出和行为不能变；公开 API 可以 breaking（这个 crate 没有别的调用方），测试文件里只能改调用方式，不能改场景和期望值。仓库事实在 .agents/native-rust/PROFILE.md；这里是从 aimux 抽出来的独立 crate，配置里的检查只跑 cargo fmt、cargo clippy --all-targets -- -D warnings、cargo test；cargo 用 ./.eval/cargo（评测环境的 HOME 是临时的，它指向本机的工具链，离线构建）。改完提交，把报告写到 REPORT.md，报告里写上各项检查的命令和结果。
