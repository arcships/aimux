# P3 依赖版本跟踪报告

**调研日期：** 2026-08-06  
**范围：** 根 [Cargo.toml](../../Cargo.toml) 中列出的关键 workspace 依赖；锁定版本来自 [Cargo.lock](../../Cargo.lock)。

## 概述

截至调研日，项目的锁文件已经跟上大多数依赖的最新兼容版本：Tokio 1.53.1、serde_json 1.0.151、thiserror 2.0.19、ts-rs 12.0.1 等均为 crates.io 当前稳定版本。需要优先评估的跨大版本升级是 `reqwest 0.12 -> 0.13`、`schemars 0.8 -> 1.2.2`、`rand 0.8 -> 0.9`（或当前 crates.io 标记的 0.10.2 稳定轨道）。`serial_test` 也已经发布 4.x，但不是运行时/安全关键路径。

版本结论以 crates.io API（`https://crates.io/api/v1/crates/<name>`）和对应 crate 页面为准；搜索结果/页面可能随时间变化，升级前应重新运行 `cargo update`、`cargo tree -d` 与测试。

## 依赖版本对照表

| 依赖名 | 声明版本 | Cargo.lock 锁定版本 | 最新版本（2026-08-06） | 是否需升级 | 备注 |
|---|---:|---:|---:|---|---|
| tokio | 1 | 1.53.1 | 1.53.1 | 否 | 同一 1.x 轨道；当前 LTS 信息见 crates.io 页面。 |
| futures | 0.3 | 0.3.33 | 0.3.33 | 否 | 最新 0.3 patch。 |
| async-trait | 0.1 | 0.1.91 | 0.1.91 | 否 | 最新 0.1 patch。 |
| pin-project-lite | 0.2 | 0.2.17 | 0.2.17 | 否 | 最新 0.2 patch。 |
| serde | 1 | 1.0.229 | 1.0.229 | 否 | 最新 1.0 patch。 |
| serde_json | 1 | 1.0.151 | 1.0.151 | 否 | 最新 1.0 patch；保留 `preserve_order` 特性。 |
| reqwest | 0.12 | 0.12.28 | 0.13.4 | **是（评估）** | 0.13 已发布；存在 API/特性和 MSRV/传输默认值变化风险。 |
| thiserror | 2 | 2.0.19 | 2.0.19 | 否 | 当前 2.x 最新；无需因版本落后升级。 |
| anyhow | 1 | 1.0.104 | 1.0.104 | 否 | 最新 1.x patch。 |
| tokio-stream | 0.1 | 0.1.19 | 0.1.19 | 否 | 最新 0.1 patch。 |
| bytes | 1 | 1.12.1 | 1.12.1 | 否 | 最新 1.x patch。 |
| async-stream | 0.3 | 0.3.6 | 0.3.6 | 否 | 最新 0.3 patch。 |
| url | 2 | 2.5.8 | 2.5.8 | 否 | 最新 2.x patch。 |
| tracing | 0.1 | 0.1.44 | 0.1.44 | 否 | 最新 0.1 patch。 |
| tracing-subscriber | 0.3 | 0.3.23 | 0.3.23 | 否 | 最新 0.3 patch。 |
| schemars | 0.8 | 未出现在 Cargo.lock | 1.2.2 | **是（评估）** | 当前直接锁定条目缺失；1.x 相对 0.8 是 breaking-change 级别，需先核对实际使用/API。 |
| ts-rs | 12 | 12.0.1 | 12.0.1 | 否 | 类型生成关键依赖，已是 12.x 最新。 |
| rand | 0.8 | 0.8.7（另有传递依赖 0.10.2） | 0.9.5（稳定）/ 0.10.2（crates.io API 的 max_stable_version） | **是（评估）** | 0.9 有 breaking changes；当前直接依赖仍为 0.8。 |
| serial_test | 3 | 3.5.0 | 4.0.1 | 可选 | 测试工具；4.x 需检查宏/API和 MSRV。 |
| httpdate | 1 | 1.0.3 | 1.0.3 | 否 | 最新 1.x patch。 |

> `Cargo.lock` 中 `schemars` 没有直接匹配条目，建议确认它是否只在未纳入当前 lock 的配置/成员中声明，或是否已被移除。`rand 0.10.2` 是 API 的 `max_stable_version` 值，而 API 同时把 0.9.5 标为 `newest_version`/当前 0.9 release track；因此升级前必须确认 0.10 的兼容性和项目所需特性，不能仅按一个字段机械升级。

## 重点依赖详细分析

### reqwest

- 当前锁定 `0.12.28`，声明 `0.12`；crates.io 当前最新 `0.13.4`：<https://crates.io/crates/reqwest>，API：<https://crates.io/api/v1/crates/reqwest>。
- `0.13` 是实际大版本升级，不应仅依赖 semver 自动解决。项目显式使用 `default-features = false`、`json`、`stream`、`rustls-tls`，升级时必须检查 feature 名称、rustls 后端及 `hyper`/TLS 依赖树；0.13 页面显示默认 TLS/feature 组合发生过调整，不能假定行为完全不变。
- 安全：本次检索未发现 reqwest crate 自身当前 RustSec 公告；但不要启用 `danger_accept_invalid_certs`，并继续保持 rustls TLS、证书校验和 URL/重定向策略。应在升级后用 `cargo audit`/RustSec 数据库复核传递依赖（尤其 TLS、hyper、URL 解析）。搜索到的“接受无效证书”风险是错误配置风险，不是本项目已证实的 reqwest CVE：<https://www.sourcery.ai/vulnerabilities/rust-lang-security-reqwest-accept-invalid>。
- 建议：**中优先级评估升级**。先建分支跑编译、集成 HTTP 测试、SSE/stream 测试和 MSRV（项目为 Rust 1.85）；若无收益或上游兼容性成本，可继续锁定 0.12 的最新 patch。

### thiserror

- 当前 `2.0.19`，声明 `2`，也是 crates.io 最新：<https://crates.io/crates/thiserror>。
- 没有可执行的升级项。2.x 已含相对 1.x 的 API/宏行为变化，但项目已经在 2.x，不建议回退或做无意义改动。
- 继续使用 derive 生成错误类型，并在 `cargo audit` 中关注其 proc-macro 传递依赖即可。

### ts-rs

- 当前锁定 `12.0.1`，声明 `12`，也是最新 release track：<https://crates.io/crates/ts-rs>，API：<https://crates.io/api/v1/crates/ts-rs>。
- 没有版本升级建议。由于它直接决定 `.d.ts` 生成，任何未来 major/minor 升级都应把生成文件纳入 golden diff，重点验证 `serde-json-impl`、`no-serde-warnings`、可选类型、枚举和字段命名。
- 本次检索未发现 ts-rs 当前 RustSec 公告；类型生成安全主要取决于构建脚本和生成物审查，勿把不可信输入交给构建期代码。

### schemars

- 声明为 `0.8`，但当前锁文件没有 `schemars` 条目；crates.io 当前最新为 `1.2.2`：<https://crates.io/crates/schemars>，API：<https://crates.io/api/v1/crates/schemars>。
- 1.x 相对 0.8 属于高 breaking-change 风险，JSON Schema 结构、derive 属性和与 serde 的兼容层都应做快照比较。不能仅修改版本号；应先确认 workspace 成员是否实际使用该依赖，再阅读 1.x migration notes 并逐个修复编译错误。
- 建议：**中优先级、单独 PR**；若当前没有 lock 条目，优先修正依赖清单/锁定状态，而不是盲目升级。

### rand

- 直接锁定 `0.8.7`；crates.io API 当前报告 `0.9.5` 为最新 0.9 release track，同时 `max_stable_version` 为 `0.10.2`：<https://crates.io/crates/rand>，版本页：<https://crates.io/crates/rand/versions>。
- 0.9 是 breaking upgrade：RNG 获取、`gen`/相关方法命名和 `rand_core` 生态可能需要调整；0.10 更应视为独立升级项目。项目只用于 retry 的 Full Jitter backoff，风险面较小，但应验证抖动分布、边界和可重复测试。
- 建议：**低到中优先级**，先评估 0.9，不建议仅为“最新”直接跳 0.10；升级后确认传递依赖中同时存在多个 rand major 的必要性。

## 已知问题与安全检查

- 本次 crates.io/RustSec 定向检索没有确认上述直接依赖存在当前未修复安全公告。搜索结果中的 `astral-tokio-tar`/CVE-2025-62518 是独立 tar crate，不是 Tokio runtime：<https://rustsec.org/advisories/>、<https://nvd.nist.gov/vuln/detail/cve-2025-62518>，不能据此判定 Tokio 有漏洞。
- serde_json 定向搜索未发现 `serde_json` 本体的当前公告；`serde-json-wasm` 的深度嵌套 JSON 栈耗尽是不同 crate：<https://nvd.nist.gov/vuln/detail/CVE-2024-58264>。对不可信 JSON 仍应限制输入大小/嵌套深度，必要时使用 `unbounded_depth` 以外的默认保护策略，不把该特性作为安全修复。
- 推荐在实际升级 PR 中运行：`cargo audit`、`cargo deny check advisories`（若项目已配置）、`cargo tree -d`、完整测试及网络集成测试。当前报告未执行这些命令，因此不能声称依赖树已通过安全审计。

## 升级建议（按优先级）

1. **P1：保持 patch 更新并进行安全审计。** 当前 lock 已基本最新；在 CI 固定执行 `cargo audit`，审查 TLS/HTTP 传递依赖，尤其是 reqwest 0.12 树。
2. **P2：评估 reqwest 0.13.4（中风险 breaking change）。** 单独 PR；验证 rustls feature、请求/响应 API、SSE 流、超时、重定向、代理和 Rust 1.85 MSRV。
3. **P2：确认 schemars 依赖状态并规划 0.8 -> 1.2.2（高 breaking change）。** 先确认为何未锁定，再用 schema snapshot 和消费者兼容性测试护航。
4. **P3：评估 rand 0.8 -> 0.9.5（中 breaking change）。** 仅用于 jitter 时收益有限；先确认 API 和随机延迟行为，暂不直跳 0.10.2。
5. **P3：serial_test 3 -> 4（低业务收益、潜在 breaking change）。** 只在测试依赖维护窗口升级。
6. **无需专项升级：** tokio、serde、serde_json、futures、async-trait、pin-project-lite、thiserror、anyhow、tokio-stream、bytes、async-stream、url、tracing、tracing-subscriber、httpdate、ts-rs；它们的锁定版本已等于 crates.io 当前最新兼容版本。

## 结论

项目当前没有明显的“落后 patch”问题，且 `ts-rs 12.0.1`、`thiserror 2.0.19`、Tokio 1.53.1、serde_json 1.0.151 均无需升级。真正需要产品/兼容性决策的是 reqwest 0.13、schemars 1.x 和 rand 0.9/0.10；其中 reqwest 的 HTTP/TLS 影响最大，schemars 的 schema 输出兼容性风险最高。建议先补充可重复的 RustSec 审计与 lock 状态确认，再分别以小 PR、快照和完整测试推进，而不是一次性批量改版本。
