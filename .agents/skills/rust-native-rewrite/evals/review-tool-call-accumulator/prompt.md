---
tags: [rust-native-rewrite]
max_turns: 30
timeout_seconds: 900
allowed_tools: [Read, Glob, Grep, Skill, Write, Edit, Bash]
---

用 rust-native-rewrite 的 review 帮我审查一下 src/lib.rs。这是从 TS 移植来的工具调用累积器，后面要改成原生 Rust。只审查，不改代码，结果写到 REVIEW.md。仓库事实在 .agents/native-rust/PROFILE.md；这里是从 aimux 抽出来的独立 crate，配置里的检查只跑 cargo fmt、cargo clippy --all-targets -- -D warnings、cargo test，PROFILE 里的 aimux 路径在这里不存在；cargo 用 ./.eval/cargo（评测环境的 HOME 是临时的，它指向本机的工具链，离线构建）。
