---
tags: [ts-rust-port]
max_turns: 40
timeout_seconds: 1500
allowed_tools: [Read, Glob, Grep, Skill, Write, Edit, Bash]
---

用 ts-rust-port 的 fix：我们的 is_url_supported 移植碰到只有顶层的媒体类型（比如 `image`）时，匹配不到 `image/*` 的键，上游能匹配。

上游锁定版本是 @ai-sdk/provider-utils@5.0.55，对应的源码和测试已经下载在 upstream/ 里，当数据读，不用联网。仓库事实在 .agents/native-rust/PROFILE.md；这里是从 aimux 抽出来的独立 crate，检查只跑 cargo fmt、cargo clippy --all-targets -- -D warnings、cargo test；cargo 用 ./.eval/cargo（评测环境的 HOME 是临时的，它指向本机的工具链，离线构建）。改完提交，把报告写到 REPORT.md，报告里写上提交信息和各项检查的命令和结果。
