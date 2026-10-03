# RFC-0038：产物体积预算与性能回归门禁

> **Status**: Draft
>
> **Date**: 2026-10-03
>
> **Scope**: ROADMAP 的 0.6 S1 / S2 / P、0.7 S3、0.8 stdio P，以及 0.6–1.0 的持续发布门禁。定义产物计量、预算注册、基准测试方法、CI 判定、例外与验收；不改变默认 API、默认功能或协议。
>
> **Baseline**: `master@72a37b5058ecd620d75dfe66bee34ef51f89294a`。下文的“现行”均指该提交；拟新增文件、命令及工作流明确标为设计。
>
> **Related**: [ROADMAP](../ROADMAP.md) §2 / §6.1、[RFC-0036](0036-positioning-and-layered-architecture.md) §4.3 / §7、[RFC-0010](0010-perf-benchmark-vs-aisdk.md)、[#166](https://github.com/arcships/aimux/issues/166)、[#196](https://github.com/arcships/aimux/pull/196)。
>
> **Amends**: RFC-0010 的“基准结果如何成为内部回归门禁”，不重写其竞品比较研究。PR [#200](https://github.com/arcships/aimux/pull/200) 的 V4 全链路 / 一次性切换方向尚未合入本基线；本 RFC 不以它合入为 S1/S2/P 前提，也不将它描述为现有 API。

## 1. 决策摘要

1. **按产物建账，不按仓库总量建账。** 每个发布文件有 target、feature 集、profile、处理阶段、精确字节数与整数阈值；原生文件、压缩包、解包总量分别检查。缺少、重复、未登记产物均失败。
2. **S1 仅切换 staticlib 消费路径。** 将 `ios-release` 统一命名为 `staticlib-release`，继承 `release` 并关闭 LTO；桌面 `.a`、iOS static framework 使用它。发布 `.so/.dylib/.dll` 仍使用根 `release` 的 fat-LTO。不得将一次 Cargo 调用顺带生成的 LTO-off cdylib 发布出去。
3. **预算由“产品上限”和“已批准实测基线 +10%”同时约束。** 预算必须入库为精确整数；没有实测则处于未注册状态，不能宣称 S2 已完成。本文给出明确的初始产品上限，逐个注册时只能在该上限内收紧；超限要重新评审预算，不能伪造基线。
4. **iOS 的 64 MiB 是现行故障防线，32 MB 是待达成产品目标。** 两者不混写为已完成。Android 默认全量，可选精简构建才承诺三 ABI 合计 ≤15 MB；pub 压缩包目标 ≤25 MB。
5. **性能以同一固定硬件上的基线 / 候选成对运行作为必需检查。** 单请求、200 KB 序列化、流式吞吐、RSS 增长分别判定，禁止用一个指标改善抵消另一个退化。真实 provider、费用、互联网延迟不进入硬门禁。
6. **0.8 的 stdio 不是仅测空 ping。** 握手、已启动进程往返、真实 dispatch + 200 KB 请求、流、二进制帧、背压 / cancel 与进程内存均纳入；新功能未注册性能与体积基线不得发布。
7. **构建、计量、校验完成后才允许发布。** 发布任务只能消费已校验且 SHA-256 一致的文件，不允许 `maturin publish` / `cargo publish` 边构建边绕过计量，或在 npm 发布 hook 中重建原生文件。
8. **回归不得通过自动更新基线变绿。** 预算、基线、工具链升级走可审阅变更；安全紧急修复允许精确、短期、有上限的例外，不允许移除 TLS、认证、大小限制、取消语义或测试来换成绩。

## 2. 问题、事实与边界

### 2.1 已核实的现状

| 事实 | 证据及解释 |
|---|---|
| 根 release 使用 `lto=true`、`codegen-units=1`、`opt-level="z"`、`strip=true`、`panic="abort"` | [Cargo.toml](../Cargo.toml)；`aimux-ffi` 同时声明 `cdylib/staticlib/rlib` |
| iOS 已单独关闭 LTO并保留全局符号 | [build-ios-xcframework.sh](../scripts/build-ios-xcframework.sh) 使用 `ios-release`、`strip -Sx`，两片均执行 `67108864` 字节硬上限 |
| staticlib 138.4 MB →66.5 MB 是历史 Linux 测量 | #196 合入提交 `7ce40c7` 的记录；不是所有平台的当前值，不能当成每个平台精确基线 |
| ROADMAP 的桌面 `.a ×5` 与工作流清单有差异 | [release.yml](../.github/workflows/release.yml) 的 `go-build` 实际发布 4 个 target：Linux x64、macOS x64/arm64、Windows GNU x64；没有已配置的第 5 个。S2 以实际清单登记，新增平台必须先扩清单与预算 |
| Android 21 MB、Node/wheel 15–22 MB、pub 31.7 MB 是汇总或区间 | ROADMAP 没有逐文件字节、哈希、工具链或测量阶段；只能作为制定初始上限的证据，不能冒充 exact baseline |
| Node/Python 是独立 Cargo workspace | 各自 `Cargo.toml` 的 `[workspace]`；不继承根 profile。S1 不顺手更改它们的优化选项，S2 登记它们实际生效的 profile |
| CLI 已在发布工作流构建 | 二进制名 `aimux`，发布名 `aimux-cli-<platform>`；0.8 是加入 stdio 功能与预算细化，并非届时才首次有 CLI |
| 现有性能工作有历史结果，缺少可执行连续门禁 | [RFC-0010](0010-perf-benchmark-vs-aisdk.md)、[PERF-RESULTS](../docs/PERF-RESULTS.md)、Node/Python `bench/`；结果页日期为 2026-07-30，不能直接作为 0.6 的当前基线 |

现有 mock 以 URL 是否包含 `/stream` 选择 SSE，且一次 `res.end` 写出拼接字符串；性能门禁必须改为检查真实请求的 `stream` 字段、固定分片并检验消费结果。现有部分脚本以 `main().catch(console.error)` 结束，可能只打印错误而不非零退出；它们未改造前不能直接用作 required check。Python 历史 RSS 读不到 `/proc` 时返回 0、按整 MB 取整，不能证明“零增长”。

### 2.2 非目标

- 不因体积预算删除 provider、公开方法、ABI 符号或 cassette；默认全量功能不变
- 不把与 AI SDK 的速度倍数当作合并条件，不以竞品升级重置 aimux 自身回归基线
- 不把公共共享 runner 的一次时间测量当稳定性能结论
- 不引入代理网关、UDS/HTTP 新载体或运行时遥测上传
- 不在本 RFC 决定 V4 数据模型或重构切换策略；未来合法 API 变更由其提案授权，性能测试随后迁移

## 3. 统一计量契约

### 3.1 单位、阶段与身份

**MB = 1,000,000 bytes；MiB = 1,048,576 bytes。** 比较只使用整数 bytes，显示同时给 bytes / MiB。压缩比仅作诊断，不可替代 raw cap。

`artifact_id = package / kind / target / feature_set / build_variant`；版本号不属于稳定 ID，实际文件名、发布版本和 SHA-256 属于一次测量。wheel 再包含 Python ABI 与实际 platform tag，JAR 包含 classifier。禁止使用不受约束的 glob 代表“所有平台均已覆盖”。

每个 ID 登记：

- `source_sha`、构建 recipe 版本、Cargo/package lock SHA、工具链和 SDK 精确版本
- `profile`、Cargo feature 展开结果的有序哈希、RUSTFLAGS、panic/LTO/strip 值、目标三元组
- `raw_bytes`：发布原生文件在所有正式 strip / 平台处理后的实际长度
- `packed_bytes`：真正上传的 `.whl/.tgz/.crate/.jar` 等文件长度
- `unpacked_bytes`：包成员普通文件逻辑长度之和；保留重复成员事实，拒绝重复路径、路径穿越、越界 symlink 与不受支持的成员类型
- 原生子文件与包的包含关系；xcframework 两片独立计量，再测整个目录的文件总量
- `baseline_bytes`、`hard_cap_bytes`、`product_cap_bytes`、测量证据与批准时间

预先签名、嵌入 frontend、生成元数据等会改变交付文件的步骤，必须放在计量之前。无签名内容的归一化统计可解释变化，但最终发布文件仍需通过最终 bytes 上限。GitHub 自动生成的 source archive 不由发布器构建，不列作受控原生发布文件；自建 source 包与所有显式上传文件必须登记。

### 3.2 初次注册与 +10% 规则

为每个实际 ID 执行两次全新、同 SHA、同工具链、同 recipe 构建。设 `B` 为两次最终 bytes 的较大值。原生文件的大小差异必须为 0；打包格式若尚不能完全确定性构建，差异不得超过 `max(4096 bytes, 较小值×0.001)`，并记录不可确定字段和修复 issue；超出即注册失败。

初始硬阈值：`H = min(C, ceil(B × 1.10))`，其中 `C` 是 §4 产品上限。JSON 内保存计算完成的整数 `B/H/C`，正常 CI 只比较 `actual <= H`，不会以当前候选值重算 H。如果 `B > C`，该产物不能注册为绿色；必须优化或明确修改该产物的 C 并解释 roadmap 影响。

后续基线不会每次跟随 master 上升。成功减重可经独立预算 PR 下调 B/H；提高 H 必须审阅，不可超过 C。新 target、ABI、feature 集、crate 拆分、包新增分类均作为新 ID 注册；没有 baseline、缺工具链信息、测试未覆盖或预期文件不存在，统一失败 `BUDGET_UNREGISTERED` / `ARTIFACT_MISSING`。

**本文所有尚未测量项都显式标作拟议产品上限，不能在实现说明里写成“当前 +10% 已完成”。** 实施 PR 必须补齐真实测量 JSON，才算 S2 交付。

## 4. 每个发布产物的数字预算

以下 C 是初始产品上限。除 iOS `67,108,864` 为已生效门禁外，表中数字是拟议政策，不是本次新测量。基于历史区间的 C 不保证每个平台已经达标，缺测量的状态一律是“待注册”。同列使用同值仍需为每一平台生成独立记录，禁止合并成总量掩盖单平台增重。

### 4.1 原生库、移动库与工具

| ID / 实际 target | 最终 raw C（bytes） | 依据、后续目标 |
|---|---:|---|
| staticlib / `x86_64-unknown-linux-gnu` | 70,000,000 | ROADMAP ≤70 MB；#196 历史 Linux 66.5 MB；S1 后重测 |
| staticlib / `x86_64-apple-darwin` | 70,000,000 | ROADMAP 产品目标，平台基线待测 |
| staticlib / `aarch64-apple-darwin` | 70,000,000 | 同上 |
| staticlib / `x86_64-pc-windows-gnu` | 70,000,000 | 同上；仍使用 cgo 可链接 GNU archive |
| cdylib / `x86_64-unknown-linux-gnu` / `.so` | 24,200,000 | 拟议全量库上限，参照绑定 15–22 MB 区间的上沿 +10%；本平台待测 |
| cdylib / `aarch64-apple-darwin` / `.dylib` | 24,200,000 | 同上，必须 fat-LTO |
| cdylib / `x86_64-pc-windows-msvc` / `.dll` | 24,200,000 | 同上，必须 fat-LTO |
| iOS / `aarch64-apple-ios` / framework binary | 67,108,864 | 保留现行 64 MiB；0.7 目标 `32,000,000`，达到前不将目标作为现状 |
| iOS / `aarch64-apple-ios-sim` / framework binary | 67,108,864 | 同上；device/simulator 分开登记 |
| iOS xcframework / 全目录 | 136,314,880 | 两个 64 MiB slice 加 2 MiB 头文件/元数据预算；还受 pub 解包总量约束 |
| Android / `aarch64-linux-android` / arm64-v8a | 9,000,000 | 拟议单 ABI 上限；21 MB 总量不能推出单 ABI 实测 |
| Android / `armv7-linux-androideabi` / armeabi-v7a | 9,000,000 | 同上 |
| Android / `x86_64-linux-android` / x86_64 | 9,000,000 | 同上 |
| Android / 三 ABI raw 总量 | 23,100,000 | ROADMAP 历史 21 MB +10%；单片通过不豁免合计 |
| `aimux-cli-linux-x64`、`aimux-cli-macos-arm64`、`aimux-cli-windows-x64.exe` | **每个** 32,000,000 | 拟议单二进制产品预算，0.6 登记现有 CLI，0.8 stdio 仍在同一 C 内 |
| `aimux-replay-linux-x64`、`aimux-replay-macos-arm64`、`aimux-replay-windows-x64.exe` | **每个** 32,000,000 | 现有发布工具，不能遗漏；功能合并后有显式退役记录 |
| `aimux-web-linux-x64`、`aimux-web-macos-arm64`、`aimux-web-windows-x64.exe` | **每个** 40,000,000 | 含正式 embedded frontend；比 CLI 多 8 MB 为拟议 frontend 预算 |

上述 Android 单片 9 MB、工具与 cdylib 数字是待验证的明确设计选择。若某个平台本来就超出，不把它拆为多个“未超限”文件规避；注册 PR 必须提交证据与该行的修订建议，未获批准前发布门禁保持失败。

### 4.2 npm 与 Python

| ID | raw/native C | packed C | unpacked C |
|---|---:|---:|---:|
| `.node` / `win32-x64-msvc` | 24,200,000 | 对应平台 npm `.tgz`：24,500,000 | 25,000,000 |
| `.node` / `win32-arm64-msvc` | 24,200,000 | 24,500,000 | 25,000,000 |
| `.node` / `darwin-x64` | 24,200,000 | 24,500,000 | 25,000,000 |
| `.node` / `darwin-arm64` | 24,200,000 | 24,500,000 | 25,000,000 |
| `.node` / `linux-x64-gnu` | 24,200,000 | 24,500,000 | 25,000,000 |
| `.node` / `linux-arm64-gnu` | 24,200,000 | 24,500,000 | 25,000,000 |
| `@arcships/aimux` 主 npm 包 | 不应内嵌 `.node` | 2,000,000 | 5,000,000 |
| Python / Linux x64 / abi3-py38+ | 24,200,000 | wheel：24,500,000 | 25,000,000 |
| Python / macOS arm64 / abi3-py38+ | 24,200,000 | 24,500,000 | 25,000,000 |
| Python / Windows x64 / abi3-py38+ | 24,200,000 | 24,500,000 | 25,000,000 |

Python 的三个平台由现行三个 host 推导为**登记目标**；实际 wheel tag 必须从 wheel 元数据确认，不能从 `macos-latest` 或文件名猜。平台 tag 变化（例如 manylinux/macOS 最低版本变化）先做兼容性审查，再注册 ID，禁止构建环境漂移自动获得新预算。当前 `--no-sdist`，若增加 sdist，必须先登记具体文件与数值，不能沿用 wheel 预算。

24.2 MB 来自历史绑定区间上沿的保守包络；额外 0.3/0.8 MB 分别为压缩封装/非原生内容的拟议余量。不是声称所有 raw 与 wheel 恰好有相同实测大小。初次登记将按每一 raw、packed、unpacked 指标分别计算 H，通常明显小于 C。

### 4.3 其余交付物：不能只守原生二进制

| 交付物 | packed / file C（bytes） | unpacked / 聚合 C（bytes） | 说明 |
|---|---:|---:|---|
| `aimux-stream`、`aimux-core`、`aimux-provider-utils`、`aimux-providers`、`aimux-ffi` 的 `.crate` | **每个** 9,500,000 | **每个** 20,000,000 | 仓库 providers 注释记录 crates.io 10 MB 限制；9.5 MB 是内部余量，并非依赖外站限额永不变 |
| `aimux-java` main JAR | 2,000,000 | 5,000,000 | 包体不自动包含外部依赖体积；报告另列依赖图 |
| `aimux-kotlin` main JAR | 2,000,000 | 5,000,000 | 若 resources 确实嵌入 native，必须按 native artifact 新增精确预算，不能扩大通配阈值 |
| Java / Kotlin `sources.jar` | **每个** 2,000,000 | **每个** 5,000,000 | sources 与 main 分开登记 |
| Java / Kotlin `javadoc.jar` | **每个** 10,000,000 | **每个** 30,000,000 | 生成文档不挤占 native 预算 |
| Maven POM / module metadata | **每个** 1,000,000 | — | 每个已发布 classifier 均登记 |
| detached signature / checksum | **每个** 16,384 / 1,024 | — | 只计大小/校验，不在日志输出签名私钥 |
| FFI header、xcframework 单个 Info.plist | **每个** 1,000,000 | — | 按实际发布路径登记；不是授权新增头文件发布 |
| Flutter pub 压缩包 | 34,870,000 | 134,217,728 | 历史 31.7 MB +10% 的暂定压缩 C；解包内部 C=128 MiB；两个指标独立 |

Flutter 的 **≤25,000,000 压缩 bytes 是 S3 目标**。128 MiB 解包预算给历史 pub.dev 256 MiB 故障留余量，不声称它是 pub.dev 当前服务上限。打包时按 publisher 实际选出的文件列表计量，不能用整个 git 工作树或任意 `tar` 结果替代。正式上传包必须与预先计量的文件列表/内容哈希一致；如发布工具不可上传预构建包，必须冻结目录，校验实际归档与清单，任何 hook 导致变化立即失败。

Go、Swift 的源码分发不另复制 `.a`/`.dylib` 预算；依赖的 release artifact 单独验证。若新增独立 Swift ZIP、Android AAR、其他 npm target 或压缩下载包，进入“新产物注册”流程；不因为它们不在这张表里而豁免。

### 4.4 收紧路线

- **0.6**：全部实际发布文件在 C 内完成 B/H 注册；staticlib 各 ≤70 MB；iOS 64 MiB guard 继续工作
- **0.7 S3**：争取 iOS 每片 ≤32,000,000 bytes、pub 压缩 ≤25,000,000 bytes。不能达到时，S3 相应验收保持未完成，记录数据与明确后续，不勾选目标；版本是否延期由 release owner 评审，不能偷偷将目标改写为现状
- Android 默认全量三 ABI 的 S3 验收为 `sum(new) < sum(已注册的0.6基线)`，且每片不回归；不声称必须回到缺乏可复现证据的 13.4 MB。可选 slim 三 ABI 目标为 **15,000,000 bytes**，每片候选 C=6,000,000，合计 cap 独立执行
- **0.8**：stdio 增量留在已登记的 CLI 32 MB 产品上限内，新增性能指标完成注册；如果不能满足，必须显式预算变更
- 达到目标后下调 C/H 并永久守门；禁止因后续一次发布临时恢复旧的大上限

## 5. S1：正确分离 staticlib 与 cdylib

### 5.1 构建变化

拟议根 profile：

```toml
[profile.staticlib-release]
inherits = "release"
lto = false
```

保留根 `release` 原值，`ios-release` 在仓库脚本、CI、文档全部迁移后删除；不保留一个容易误用的静默别名。自定义 profile 属于构建接口变化，应在 release notes 给旧命令替换示例，但不改变库 API。

- `go-build` 与 CI `go-binding` 改用 `cargo build -p aimux-ffi --locked --profile staticlib-release --target <triple>`；相应 `CGO_LDFLAGS` / copy 路径切换至 `target/<triple>/staticlib-release`
- iOS 脚本两 target 同样切 profile 和目录，保留 `strip -Sx` 与每片大小 gate；不要恢复已因兼容性问题移除的 `bitcode_strip`
- `ffi-build`、Android、Java/Kotlin/Swift/Flutter 的动态链接测试与 release 保持 `--release`。桌面 DLL 的 MSVC 与 Go archive 的 GNU target 不混用
- 在单独 target 目录中构建两种 profile；上传清单只允许 `staticlib-release/*.a` 或 `release/*.{so,dylib,dll}`。即使 Cargo 因 crate-type 同时产出其他文件，非授权变体一律不复制、不上传
- Node/Python 先记录其有效优化 profile，不在 S1 添加 `lto=false`。以后选择 fat-LTO/size profile 要有独立 before/after 证据和对应性能门禁

### 5.2 防止“瘦了但不能用”

构建后对 archive 成员检查 LLVM bitcode section（ELF `.llvmbc/.llvmcmd`、Mach-O `__LLVM`）；不得仅看 `.a` 总大小推断已关 LTO。静态库保留链接所需全局符号；不要在 archive 上使用可能破坏符号索引的激进 strip。保存 exported-symbol allowlist 与基线差异，工具平台化实现而非假设 GNU `nm` 遍地可用。

用正式 archive 执行 C 链接/调用 smoke、Go vet/test（含 macOS）、iOS CocoaPods/SwiftPM 支持路径的链接验证；用正式动态库执行既有各语言 contract tests。对 roots 的 `release` 检查有效编译命令包含 fat-LTO，确认发布文件的 profile 来源，不能只检查 Cargo.toml 文本。测试允许发现原本的平台问题，不能靠删除符号或跳过平台让 S1 通过。

## 6. S3：可解释的体积审计与可选 slim

### 6.1 审计顺序

对可取得源码与锁文件的旧版本、0.5.0、已注册 0.6 基线，用**同一工具链/SDK**重建 Android 三 ABI 和一个桌面动态库。若旧版本无法用该工具链构建，分别保留其原工具链对照，明确不可归因的工具链差异；13.4→21 MB 只作为待解释问题，不能直接归因给某 crate。

依次输出：

1. raw、section（text/rodata/data/debug/bitcode）、strip 前后、压缩后差异
2. 固定版本 `cargo-bloat --crates`、函数前 50 项，`cargo tree -e features` 与重复依赖；不能把 cargo-bloat 的 `.text` 合计误当整个发布文件
3. JSON schema、TLS/WebSocket、重复 codec、泛型单态化、注册表/字符串、绑定打包重复文件的贡献与置信度；按实际证据排序
4. 每个候选优化单独 before/after size + P 结果，先构建/打包优化，再无语义内部优化，最后显式 opt-in feature

审计工具只处理本地可信构建物，不执行包里的任意脚本。使用带符号分析副本时，不发布这些副本；正式计量仍针对最终 stripped 文件。

### 6.2 feature 契约

已有 `aimux-providers` 默认 `realtime` 与 provider-utils `ws` 的可选路径是起点，不等于端到端 FFI slim 已实现。Cargo feature 是可加的，必须在依赖边明确控制 `default-features` 与转发关系，并用 `cargo tree -e features` 证明未被另一路重新启用。

新增精简构建使用单独标识 `feature_set=slim-text`，拟议包含文本、工具、所选协议；协议清单在 recipe 中精确列出。默认 crate feature、默认 release 包、默认 constructor 行为保持不变。Rust 的 optional API 暴露规则必须有文档；C ABI 的函数/错误约定保持可链接，未编译能力返回既有能力错误类别并说明缺失 feature，不可空返回或触发 panic。ops 能力查询就绪后还需反映真实 feature 集；此前在包元数据列出。

默认和 slim 各有独立 artifact_id、文件名/包标记、capabilities、测试矩阵及预算；slim 不能覆盖 full 的标准下载链接。共享支持能力必须通过相同 cassette/contract tests。默认构建必须继续覆盖全部现有协议和模态；不能以 slim 数字宣称默认体积已达标。第一版只测试并发布 recipe 中明确列出的 full/slim 组合；任意 feature 幂集不在支持承诺内，但任何受支持组合缺包、缺符号或错误能力声明均阻塞。

## 7. P：基准测试实验设计

### 7.1 固定环境与可信执行

拟新增 `bench/environment.json`，登记专用 `perf-linux-x64-01` 执行池中的**唯一具体机器**：CPU 型号/stepping、物理核心与 SMT 拓扑、microcode、RAM、内核、OS 镜像 digest、CPU governor/turbo 策略。分配至少 4 个独占物理核心和 16 GiB RAM；锁定两个物理核心运行 client/runtime、一个运行 mock、一个采样，兄弟 SMT 核不安排其他负载。硬件不存在或无法隔离时，先完成配置，不能拿公共 runner 结果填正式基线。

绑定进程与 mock 都固定 affinity；Tokio worker 数固定 2。编译使用固定通用 target，不启用宿主 `target-cpu=native` 来偷换发布特性。runner 身份/拓扑不匹配登记值，作 infrastructure failure。toolchain lock 精确记录 rustc/LLVM、Node patch、Python patch、编译器/linker、NDK/Xcode，以及依赖 lock digest；不得使用 `stable/latest/^` 作为可复现证据。选择登记时实际可用版本，不在 RFC 捏造测量时尚未存在的版本号。

编译和测量隔离；测量开始前无同机编译、swap 活动或其他作业，CPU steal 不超过 1%，背景 CPU 利用率不超过 5%。基线校准数据越界使整对运行失效；候选本身的高 CPU、内存或抖动不能作为“环境噪声”删除。

专用 runner 不把未经审查的 fork PR 代码当可信任务执行。公开 PR 先在临时沙箱构建；执行性能作业需受保护审批、无 secrets、禁外网、一次一组、执行后销毁环境，机器管理面与代码隔离。不能把网络不通变为连接真实 provider 的理由。

### 7.2 baseline 与样本

每个 PR 使用 `merge-base(candidate, origin/master)` 对应代码与 candidate 的合成合并结果，按同一已固定 recipe 构建、成对测量；同时比较版本内已批准的固定 anchor baseline，防止多个 4% 退化累计超过预算。anchor 二进制保留且 hash 固定，在同批实验中重新运行；登记的历史结果用于校准与审计，不把不同日期的单次数值直接相减。变更 benchmark fixture/harness 的 PR，先用旧 harness 测公共场景，再让同一个新 harness 同时测 base/head；不能改一侧输入或对照选择。

每一 scenario × backend：

- 10 对独立进程运行，顺序 AB/BA 按固定 seed 随机交错，mock 为每个进程重置
- 每次至少 1,000 次请求或 5 秒预热（取较长者）；单请求计时收集 10,000 个有效样本
- streaming 每次至少 100 条完整流且测量不少于 10 秒；吞吐只统计经过内容校验的 payload bytes / events
- memory 每次完成 2,000 次预热后，运行 20,000 请求，分 20 个 1,000 请求窗口；长流/取消场景另行 1,000 次生命周期测试
- 无效、超时、断流、内容/用量/事件计数不符，首先为 correctness failure，不从延迟样本中“清洗”掉

计时使用各后端的单调高精度时钟，吞吐测量覆盖首个完整有效事件到最后一个有效事件的消费窗口；TTFT 从调用开始到首个有效输出，二者不可混算。所有脚本未捕获异常、样本不足、mock 提前退出必须以非零状态退出，汇总器检查实际样本数。

每个 `scenario/backend` 必须登记正整数 `operation_timeout_ms` 和 `run_timeout_ms`；timeout 是硬失败，不是丢弃样本的理由。拟议初始值如下，注册时保存为精确整数并纳入 recipe hash：

| 场景类 | operation timeout（ms） | run timeout（ms，每次独立进程运行） |
|---|---:|---:|
| request-small / request-200k / serialize-only / tools-and-errors | 5,000 | 600,000 |
| stream-small-chunks / stream-large-chunks / stdio-stream / stdio-binary | 15,000（从调用到最后事件校验） | 600,000 |
| sustained / binding-lifecycle | 5,000（每次请求） | 600,000 |
| cancel-and-drop / stdio-backpressure-cancel | 2,000（恢复读取并发送 cancel 到确认及本次释放完成） | 600,000 |
| stdio-cold-start | 2,000（spawn 到完整握手） | 600,000 |
| stdio-roundtrip-1k / stdio-roundtrip-200k / stdio-incremental-cost / stdio-memory | 5,000（每个 op） | 600,000 |
| stdio-lifecycle | 2,000（每次 EOF/parent-exit 到子进程退出；cancel/join/flush 总计） | 600,000 |

operation timeout 是防悬挂 watchdog，不能替代 §7.4 / §8 更严格的 P50/P95 产品护栏；例如握手即使小于 2 s，P95 超过 100 ms 仍不通过。预热和正式样本均受 operation timeout，整次 run deadline 覆盖预热、样本与清理。单进程阻塞可能让语言内 timer 无法执行，外层监督进程用单调时钟监控并终止超时子进程，同时记录 timeout、已完成样本数与退出状态。

base/head/anchor **必须使用完全相同的两个 timeout 数值**，在实验开始前锁定；不得因 candidate 变慢单边放宽。缺字段、0、负值、浮点、无穷、溢出或无法实施外层 watchdog，均为 `BENCH_CONFIG_INVALID`，阻塞注册和正常门禁。新场景必须显式给数值，不能静默继承无限等待；修改 timeout 视为 benchmark recipe 变更，按 §10 审阅并按 §11 重置适用窗口。任一有效样本或 run 超时记 `BENCH_TIMEOUT` 并硬失败，不能走噪声补测把它消掉；基础设施证据明确说明运行环境失效时，另记 `INFRA_FAILURE`，保留原始记录再重跑。

输出逐运行 mean/P50/P95/P99、吞吐、事件计数、CPU 时间、RSS 时间序列；总体比较使用**每对独立运行统计量**，不把 100,000 个相关请求当 100,000 次独立实验。保存 seed、原始样本、环境、fixture hash、命令与 source SHA。

### 7.3 负载与后端矩阵

固定 synthetic 数据，UTF-8 JSON 的序列化后 request body 大小分别为 1,024 / 10,240 / 102,400 / **200,000** bytes；200 KB 指整个计入负载的序列化 body，不能用估计 token 数替代。内容使用公开固定文本/字节，不含用户对话。响应为 1,024 或 50,000 bytes，并包含固定 token usage；生成脚本检查精确长度与 hash。

| 场景 | 必测内容 | 硬门禁后端 / 阶段 |
|---|---|---|
| `request-small` | 1 KB request / 1 KB response；复用模型和连接，计时含用户侧 stringify/parse/typed wrapper | Rust、C ABI、Node public wrapper、Python public API：0.6 |
| `request-200k` | 200 KB request / 50 KB response；构造/序列化/解析分别辅助计时 | 同上，200 KB E2E 为硬门禁 |
| `serialize-only` | 固定 JSON encode/decode；校验语义与 hash，不做 HTTP | Rust、C ABI、Node、Python；不要将跨语言差值解释为某语言纯 CPU 成本 |
| `stream-small-chunks` | 10,000 个 payload event，每个 payload 32 bytes，确定性 SSE 分片；包含终结与 usage | 同上；吞吐、TTFT、结束清理 |
| `stream-large-chunks` | 1,024 个 event，每个 payload 1,024 bytes；固定 chunk 边界 | 同上；bytes/s 与 peak RSS |
| `tools-and-errors` | 5 个 tool schema、分片 tool input、结构化错误；验证解析产物 | 同上；不能用省略错误处理提升结果 |
| `sustained` | 200 KB 请求，20,000 次，C=1/10/50；不保存响应历史 | Rust/C ABI/Node/Python，RSS/延迟/吞吐各检查 |
| `cancel-and-drop` | 1,000 次建模/建流/取消/释放；检查取消响应和句柄释放 | C ABI/Node/Python；资源计数与 RSS |
| `binding-lifecycle` | 相同请求/流/释放的轻量脚本 | Go/Java/Kotlin/Swift/Flutter 在原生 CI 上做功能与内存泄漏烟测；不得宣称 Linux P 覆盖所有移动端硬件 |

OpenAI Chat-compatible 固定本地 mock 为首个硬性能 backend；B 轨新增协议后每个协议至少有一次请求与一条流的 correctness + benchmark 场景，注册完成后参与该协议的回归比较。不可把 Rust direct path 的数值作为 Node/FFI 的替身。Node raw napi 辅助诊断保留，但对外 public wrapper 是主门禁。

mock 独立进程，仅监听 `127.0.0.1`，按请求解析出的 stream 标志返回固定 JSON/SSE；流动效场景按固定 seed 分片、包含拆开 UTF-8 / SSE 边界。吞吐场景不人工 sleep；带延迟/背压场景独立列出。禁用 retry 和录制用于隔离基础路径，另外增加 retry/录制开启的独立功能/成本场景，不能为了“默认更快”改产品默认值。

B0 使用相同输入、响应消费、keepalive 策略的本地直连 client。B0 是系统与 mock 饱和校准，不以 `aimux−B0` 的单次差值充当精确 CPU 归因；不同 HTTP 栈相减可能为负。端到端值是硬门禁，细分 CPU instrumentation 为解释证据，不随发布开启。

### 7.4 数字门槛与统计判定

首次登记使用 §7.2 实测 anchor；历史 0.101 ms、0.66 ms、+23 MB 只为量级参考，机器/样本不同，不直接作为本门禁 baseline。

| 指标 | 相对已批准 anchor / 成对 base 的回归预算 | 绝对产品护栏（拟议，需登记验证） |
|---|---|---|
| 单请求 P50 | 增幅 ≤5%，且忽略小于 5 μs 的绝对差 | 1 KB：≤1.0 ms；200 KB：≤5.0 ms |
| 单请求 P95 | 增幅 ≤10%，且忽略小于 10 μs 的绝对差 | 1 KB：≤2.0 ms；200 KB：≤10.0 ms |
| serialize-only 200 KB P50 | 增幅 ≤5%，绝对差容忍 5 μs | ≤2.0 ms |
| 流式 event/s、payload bytes/s | 降幅 ≤5%，不设置可累积的绝对豁免 | 绝对下限由已注册 baseline 的 90% 写入整数；不在没有实测时发明吞吐量 |
| TTFT P95 | 增幅 ≤10%，绝对差容忍 10 μs | 无人为延迟 mock：≤5.0 ms |
| retained RSS 增量 | ≤anchor/base + `max(10%, 2 MiB)` | 每后端持续场景 ≤32 MiB |
| peak RSS | ≤anchor/base + `max(10%, 8 MiB)` | C=1 ≤256 MiB；C=10/50 ≤512 MiB（不含 mock） |
| RSS 增长斜率 | Theil–Sen 斜率及其区间，末 15 个窗口 | ≤0.5 MiB / 1,000 请求 |
| correctness / timeout / leaked handles | 0 个 | 任一个失败即阻塞 |

“忽略绝对差”仅用于避免微秒计时粒度把小值比例放大；绝对护栏始终适用。内存以 bytes/page 粒度采样，读不到为失败，不返回 0。Node 如为 retained 指标使用显式 GC，只能在每窗口固定的测量边界做，不在 latency 主样本中强制 GC；同时保留不 GC 的 peak。释放后按固定 1 秒静置点取 retained；allocator 不返还 RSS 不等于内存泄漏，因此结合 slope、句柄数与资源计数判定，不只做堆对象计数。

每对统计量做配对 bootstrap（固定 seed，10,000 次），生成退化量的 99% 置信区间。点估计越过预算且区间下界也超过预算与对应绝对差门槛，为 `PERF_REGRESSION`；区间跨越边界为 `PERF_INCONCLUSIVE`，**不视为通过**。允许一次额外完整 10 对补测，合并全部有效样本再算；不能挑更快的一组。补测后仍不确定则 required check 失败，等待调整实验隔离或人工分析，不能自动提高容忍度。

base 的各次 P50 稳健离散度 `1.4826×MAD/median` 必须 ≤3%，校准 B0 与登记正常区间偏移 ≤5%；超过为 `BENCH_ENV_UNSTABLE`。候选离散度过高必须保留为可能的真实退化。P99 首期输出但不单独判相对百分比，防止小样本尾部噪声；绝对超时与错误率为硬门禁，稳定扩样后才新增 P99 预算。

绝对产品护栏用每次 run 的相应统计量检查，持续越界不能以“base 同样慢”通过。bootstrap 用于相对回归判断，不豁免确定性错误、缺样本或绝对资源上限。

## 8. 0.8：stdio 入口性能与资源预算

本节依赖 ops 协议实现和测试接口就绪，**不阻塞 0.6 的现有路径门禁**。帧结构、版本协商、最大 frame 与 cancel 语义由 ops RFC 决定，本 RFC 只按真实协议测量，不另设一份 wire 格式。benchmark 用协议协商获得版本，不硬编码开发中导出数量。

| 场景 | 测量边界 | 拟议数字护栏 |
|---|---|---|
| `stdio-cold-start` | 父进程 spawn 到首个有效握手响应；不计下载与首次安装 | P95 ≤100 ms；相对已注册值 +10% |
| `stdio-roundtrip-1k` | 已启动/握手进程，1 KB 有效真实 op，请求写入到完整响应验证 | P50 ≤1 ms、P95 ≤2 ms；成对回归同 §7 |
| `stdio-roundtrip-200k` | 父进程序列化→写 pipe→真实 dispatch→50 KB 响应→父进程解析 | P50 ≤5 ms、P95 ≤10 ms；相对回归同 §7 |
| `stdio-incremental-cost` | 同环境同 op 的 stdio 与进程内 dispatch 成对差值 | 1 KB P50 增量 ≤0.5 ms；200 KB ≤2 ms；仅解释 IPC，总量仍需过门禁 |
| `stdio-stream` | 完整 10,000 event / 32-byte payload 与 1,024×1,024-byte 场景 | 相对注册吞吐降幅 ≤5%；TTFT P95 ≤5 ms |
| `stdio-binary` | 每帧 65,536 bytes，16 帧；内容 hash 校验，帧数/字节不变 | 吞吐下限为注册值×90%；不转 base64；相对降幅 ≤5% |
| `stdio-backpressure-cancel` | 父进程暂停读 1 秒、恢复、发送 cancel；100 条并发流 | 恢复后 cancel 确认 P95 ≤100 ms；全连接、全部适配器排队 payload 合计上限 8 MiB；超出必须受控背压/按协议报错 |
| `stdio-lifecycle` | 1,000 次 request/cancel/drop，正常 EOF 与异常 parent exit | 子进程退出 ≤2 s；0 个 orphan/残留句柄 |
| `stdio-memory` | 父 + 子进程分别及合计 RSS，排除 mock | 子进程 idle ≤64 MiB；合计 retained 增长 ≤32 MiB；总 peak ≤256 MiB（C=1）、≤512 MiB（C=100） |

有些 dispatch op 没有 1 KB 输入，不为成绩添加只用于生产暴露的空操作；选择实际轻量本地操作作为协议往返，另测带 mock HTTP 的 generate 路径。create/drop 的成本单独报告，不从 warm E2E 的内容中抹掉真实调用所需工作。

背压上限与 ops 层安全帧限额必须共同成立：全连接所有适配器共享 8 MiB 排队 payload 预算，另最多一个正在处理的、受协商帧上限约束的 frame；不是每个 stream/适配器各占 8 MiB。按“一个协商上限的在途 frame +8 MiB 有界等待 payload”公式登记，最大 frame 单独纳入 peak 测试；不能以队列 cap 改变协议允许的 frame。stdio 的 2 s 清理是 cancel/join/flush 的整体期限，不可叠加进程内 FFI 单独场景的 join 等待期限。持续输出、恶意长度、超限 frame、错误 JSON、日志挤占 stdout 均是 correctness failure。stdout 仅承载协议帧，stderr 可诊断但不能打印 payload/密钥；父进程始终并行排空 stderr，避免测到日志死锁。

stdio 初始 B/H 注册需按上表所有场景完成；任一个缺样本不允许用“ops 功能测试通过”代替。把 H 与 peer API/协议版本一同保留，未来 V4 切换也应迁移而非删除这些场景。

## 9. CI / 发布管线与失败语义

### 9.1 拟新增文件与唯一来源

```text
quality/artifacts.json             # 实际发布清单、profile、feature、路径与依赖包关系
quality/size-budgets.json          # 每个 artifact/metric 的 B/H/C、证据、目标
quality/perf-baselines/            # 环境 / scenario / backend / anchor 分版本
quality/exceptions.json           # 精确范围、原因、批准、期限与临时上限
scripts/quality/measure-artifacts.py
scripts/quality/check-budgets.py
scripts/quality/compare-perf.py
bench/environment.json
bench/fixtures/                   # synthetic fixture + hash
```

路径是本 RFC 的拟议实现，不是当前已存在。schema version 固定，JSON 拒绝未知键、重复 ID、浮点/负数字节、缺字段。产物清单驱动 build matrix 与汇总 expected-set；若暂时无法生成 YAML matrix，增加双向一致性测试确保清单与工作流矩阵一致。单一表同时服务 PR/nightly/tag，不能各维护一份阈值。

拟议开发命令：

```sh
python3 scripts/quality/measure-artifacts.py --manifest quality/artifacts.json --input staging --out size-report.json
python3 scripts/quality/check-budgets.py --report size-report.json --budgets quality/size-budgets.json
python3 scripts/quality/compare-perf.py --base base.json --head head.json --anchor approved-anchor.json
```

失败信息含 ID、期望/实际 target/profile/features、bytes/限额/差值、source SHA、baseline SHA、证据路径和本地重跑命令。不包含环境变量全文、请求 Authorization、真实录制 body 或签名 secret。

### 9.2 执行层次

| 触发 | 必需检查 | 目的 |
|---|---|---|
| 每个 PR | manifest/schema 自检；相关 release-equivalent size 构建；当前 P 成对测试；既有 correctness / cassette / contract gates | 阻止回归进入 master |
| 文档-only PR | 必需汇总检查仍运行；受版本控制的路径规则证明不触及代码/预算/recipe/fixture，报告明确 `not_applicable` | 防止 required job 被 GitHub path-filter 整体跳过而卡住或误绿 |
| master push | size/P 汇总，相关路径矩阵与 artifact provenance 检查 | 合并结果，而非仅 PR head |
| 每日 master / 活跃 release candidate | 全平台、全部包、全部性能场景，固定 anchor 比较；candidate 固定 ref 单独记录 | 检查未触及平台、打包链与累计回归，积累候选窗口 |
| tag/release candidate | 完整全平台最终构建与全场景 P；证据不早于 24 小时；验证连续绿窗口 | 发布不可只复用局部 CI |

修改 Cargo/lock/profile/基础协议/公共代码/打包清单/预算时，size matrix 全跑；只改某绑定可按 manifest 依赖闭包选 target，但 nightly 与 tag 无裁剪。required 汇总 job 使用 always 语义收集 missing/failed/cancelled，而不是任一 upstream skipped 就自动 success。定时 provider registry/reachability 探测保持报告性质，不被并入本 RFC 的必需门禁。

### 9.3 发布顺序与不可变性

现行 `rust-publish`、`python-release`、`jvm-publish` 直接发布、部分 npm hook 重建、GitHub Release 用宽泛 glob；实施 S2 必须分离：

`plan → build/package → measure → test → quality-gate → publish exact artifacts → verify published manifest`

- Python：maturin **build** 得到 wheel，量测 / 安装 smoke 后才发布同一 wheel；保留 trusted publishing
- Rust：逐 crate `cargo package`、量测并校验依赖顺序。若 `cargo publish` 必须重新打包，发布前再次校验同内容清单/hash 与 cap；不能把未经验证的新 archive 当旧结果
- npm：先生成所有 wrapper/types/platform package 并执行必要 hook，`npm pack` 产出的每个 `.tgz` 过 gate；发布预构建 tarball，禁发布时再变原生文件
- JVM：构建并量测每个 main/sources/javadoc/POM/module 及最终签名输出，gate 之后才 upload；并检查 Kotlin native resources 是否发生意外嵌入
- Flutter：使用 publisher 的精确 include/exclude 文件集冻结目录，执行 dry-run/验证包与两种大小 gate；上传之前校验未变更
- GitHub Release：按 manifest 的明确文件列表上传，拒绝 `release-assets/**` 将无关 job artifact、分析报告或调试库偶然公开

跨 registry 发布无法事务回滚。所有目标的发布前检查应先完成；之后某 registry 上传失败，标记 `partial_release`，仅重试 hash 相同的未发布对象，不重新构建、不覆盖既有版本。已发布产物与 manifest 摘要做 readback 验证；如果发现不一致，停止剩余上传并由 release owner 处理，不能删除证据后宣称发布成功。

release 手动选择子集只影响哪些 publisher 执行，不豁免该子集的依赖包、共享核心测试与基线；tag 完整 release 的清单不可用输入 flag 缩减。工作流 token 权限按 build/publish 分离，build job 无 registry secrets。

### 9.4 失败类别与恢复

- `SIZE_REGRESSION`：actual > H；指向最大的 section/dependency/package 差异，不自动 rebaseline
- `PROFILE_MISMATCH` / `FEATURE_MISMATCH`：路径或构建实际参数错误；修 recipe，不放宽 cap
- `ARTIFACT_MISSING/UNEXPECTED/DUPLICATE`：清单与实际集合不相等；直接失败，不能忽略空 glob
- `BUDGET_UNREGISTERED`：新增平台/metric 或缺精确 B/H；先注册，不走 report-only 放行
- `PERF_REGRESSION`：相对预算统计显著越界；优化或精确例外
- `PERF_INCONCLUSIVE/BENCH_ENV_UNSTABLE`：按 §7 补测；仍不稳定则阻塞，修执行环境
- `BENCH_CONFIG_INVALID`：缺少/无效/不一致的 timeout 或无法落实 watchdog；先修正固定 recipe，不得无限等待
- `BENCH_TIMEOUT`：操作或整次 run 超过登记 deadline；硬失败，保留部分样本与资源诊断，不筛掉超时样本
- `CORRECTNESS_FAILURE`：内容、错误、取消、用量、输出 schema 或资源释放错误；禁止通过性能例外豁免
- `INFRA_FAILURE`：runner 离线、下载、采样失败；只对同 SHA/相同参数重新运行，不生成绿色报告

所有失败必须留下 machine-readable JSON 和简短 summary。完整性能原始数据保留 90 天，已发布 tag 的预算/环境/摘要/hash 作为 release evidence 长期保留；体积证据不依赖容易过期的单一 CI URL。

## 10. 基线更新、例外与安全

### 10.1 更新规则

更新 baseline 或 C/H 的 PR 必须列出旧/新数值、真实产物/运行证据、增重/退化原因、默认功能是否相同、预期用户收益、追踪 issue。代码作者不能自己批准例外；由另一个维护者或 release owner 审核。S3 体积降低通常只下调基线，不能顺带放宽 P。

编译器/SDK/硬件升级不是“删除历史”。在旧/新环境分别运行同一个 anchor 与候选，提供四格对照；选定新环境的 B/H 后保存旧环境记录并从新环境重新积累连续绿窗口。无法得到旧环境时明确披露证据缺口，不把跨环境差当性能改进。

合法功能增加或 V4 one-shot API 切换：同语义 synthetic fixtures 通过适配层在两版运行。适配仅在 benchmark，不要求生产保留旧 API。完全没有等价项时，新场景先完成注册、旧场景标明退役原因与替代 coverage，人工批准；不能把全部 baseline 清空来吸收重构成本。PR #200 合并与否不影响 S1/S2 的 profile 与 bytes 契约。

### 10.2 有限例外

例外记录包含准确 artifact/scenario/backend、指标、临时整数 cap/百分比上限、适用 commit 或有限版本区间、issue、业务/安全理由、批准者、到期日、补救责任人。最长 **14 个日历日或下一次 minor release，先到者失效**；只能人工延期并再次提交证据，不允许通配 `*` 或永久 `ignore`。

安全修复导致密码库升级、额外校验或内存清零增加成本时优先保留安全属性，可走上述临时例外。例外状态显示 `pass_with_exception`，不是连续绿；不允许豁免认证/TLS、正确性、产物缺失、未注册基线或不可知实验环境。常规发布不能依赖过期或无明确上限的例外；紧急安全 patch 若必须带有效例外，由 release owner 明确记录，后续连续绿窗口重新开始。

## 11. 持续发布判据

“连续绿”的计数单元为 `(UTC 日期, 候选 SHA, 环境/预算版本)`。同一天、同候选的重跑不能增加天数；**同一候选 SHA 在不同日期的完整验证可以分别计数**。每个日期采用首次有效完整检查，同时保留该日所有失败记录；已有真实回归、inconclusive 或 pass_with_exception 不能靠当天后续碰巧成功变绿。环境故障可以同条件重试，恢复后的首次有效完整检查可代表该日，但缺测未恢复的日期为 unknown，会中断连续日期。

- **0.6 / 0.7 / 0.8 / 0.9 常规发布**：release candidate 经连续 7 个 UTC 日期的完整检查通过；master 的既有 nightly 证据只能在验证其产物、依赖、构建 recipe、性能场景和公共合同闭包与候选完全相同时复用，并记录逐项 digest 等价证明。不能仅因两次检查都来自 master 而跨代码变化拼接窗口
- **1.0**：同一拟发布候选 SHA 连续 14 个 UTC 日期的完整检查通过，与 RFC-0036 §14 的候选验证周期对齐。全部已支持产物、stdio 与既有绑定的适用场景均注册，无有效 size/P 例外；还需满足 ROADMAP 的 ABI/L0/L1 冻结与其他发布条件，本 RFC 只补充门禁条件
- 候选是已合入 master 的具体提交。master 可以继续其他工作，候选 daily job 仍检验固定 ref；发布 tag 必须指向该候选，最终 bytes 与该 tag 对应，最后的全量 gate 在同一 SHA 成功且证据 ≤24 小时
- 候选代码、公共错误/取消/重试/序列化合同、依赖、构建 recipe、预算、环境、fixture 语义、scenario 集或产物集发生变化时，受影响闭包重新验证；全量 S2/P 窗口从新候选开始。不得将受影响的旧 SHA 绿灯计入新候选。与候选无关的 master 文档变更不改变固定候选；只改检查报告排版且经 diff 证明不改输入/判定，也不重置
- 窗口内所有已执行的相关 PR/master/candidate 检查不得存在未解决真实失败。紧急安全 patch 适用 §10.2，不能借“常规回滚”绕过；回滚到旧 SHA 仍需验证当下打包/发布对象与旧证据一致

每日作业的安排行为属于项目 CI，不是向外部 provider 发起探测；不与 #170 的“scheduled registry 报告不阻塞合并”冲突。

## 12. 实施拆分、依赖与回滚

| 步骤 | 版本 / 依赖 | 可独立交付的变更 | 完成证据 |
|---|---|---|---|
| Q0 | 0.6，无 B/C/D 依赖 | 产物 inventory、schema、计量器、比对器及负例测试 | 全清单映射实际 workflow；暂无测量项仍标未注册 |
| S1 | 0.6，Q0 | staticlib profile 改名、4 桌面 target +2 iOS target、消费路径和符号/链接测试 | before/after bytes、无 bitcode、cdylib 仍 fat-LTO、测试全绿 |
| S2 | 0.6，Q0/S1 可同 PR | 每项 B/H 注册，build/publish 分离，汇总 required check | 完整 release dry-run 对全部实际文件守门 |
| P0 | 0.6，固定 runner | 修现有 bench、synthetic fixtures、成对运行、raw/typed 路径 | 校准与误差实验、故意 10% 退化被阻塞 |
| P1 | 0.6，P0 | 请求/吞吐/RSS 正式 hard gate 与固定 anchor | 全后端注册及连续绿开始 |
| S3 | 0.7，S2/P1；不以 B 轨完成为前提 | 归因报告、逐项体积优化、可选 slim 的端到端 feature 和能力测试 | 目标与实际分别列明；默认全量回归不变 |
| P-stdio | 0.8，ops dispatch/帧实现 | §8 场景与预算注册、CLI bytes 更新 | 父子资源/背压/取消等全量通过 |
| release gates | 各 minor /1.0 | 连续绿统计、证据清单和发布前校验 | 正确窗口与精确 tag/hash |

允许 Q0 的开发期 report-only 采集以创建第一份 baseline，但它不计为 S2/P 完成，也不计为连续绿；0.6 交付前关闭过渡模式。预算校验代码自身需受 branch protection required check 保护，不能让修改配置的 PR 顺便跳过验证。

回滚实现问题时，优先回滚生成/打包变更，不删除质量门禁。若 staticlib profile 切换破坏某平台链接，保留失败证据，修复或回滚该 profile 变更；原有 fat archive 如超过 C，不得无说明发布，可按限定例外处理。对于误报先诊断环境/fixture，不能永久 `continue-on-error`。门禁本身损坏时以已验证的上一版校验器运行当前产物，审阅恢复后再合并。

## 13. 验收清单

### S1 / S2

- [ ] release 清单准确覆盖 4 个桌面 staticlib、3 个桌面 cdylib、2 个 iOS slice、3 个 Android ABI、6 个 Node native/平台包、3 个实际 wheel target、9 个工具、所有 Rust/JVM/Flutter 与明确上传的附属文件
- [ ] ROADMAP 的 `.a ×5` 差异被明确修正或第 5 个 target 经正式新增；不靠猜测补齐
- [ ] 每个实际文件的 raw/packed/unpacked 适用指标都有整数 B/H/C、hash、recipe 和环境证据；无“稍后补”即可通过的路径
- [ ] H-1/H/H+1 边界测试；MB/MiB 换算；缺失/重复/未登记/损坏包/路径穿越/跨 profile 文件均稳定失败
- [ ] 恢复 staticlib fat-LTO 的负例触发 bitcode/profile/size 至少一项失败；误取 LTO-off cdylib 必然失败
- [ ] 四桌面 `.a` 各 ≤70 MB，iOS 当前 64 MiB 守门有效；32 MB 目标未达成时不标完成
- [ ] 正式交付库通过对应 C/Go/iOS 链接、各语言 contract、移动 example 测试，导出符号/默认能力不缺失
- [ ] publish 消费已校验 bytes，模拟某 registry 失败不会重建已发布产物或发布未过 gate 的剩余文件

### S3

- [ ] Android 增重有同环境 before/after、section/dependency 与 feature 证据，无法归因的部分明确标出
- [ ] 默认 full 三 ABI 合计较登记的 0.6 基线降低，单 ABI 不越 H；默认 API/功能与 cassette 覆盖不变
- [ ] 可选 slim ≤15 MB 合计且每 ABI ≤6 MB，功能列表准确、未编译能力明确报错、不能覆盖默认包
- [ ] iOS 每片 ≤32 MB、pub 压缩 ≤25 MB 的各自完成情况有真实文件证据；未达目标保持未勾选
- [ ] 达标产品 C/H 下调，后续增重的注入测试会失败

### P /0.8/发布

- [ ] 固定硬件与完整工具链登记，可离线复现同 synthetic workload；读不到 RSS 或 missing data 不会变 0/绿色
- [ ] 1 KB/200 KB、public wrapper、流式、C=1/10/50、RSS slope/cancel 场景均完成 baseline 注册
- [ ] 注入 +10% latency、−10% throughput、每 1,000 请求泄漏 1 MiB、丢 event、错误 success 返回，分别触发预期失败
- [ ] 每个 scenario/backend 有固定正整数 operation/run timeout，base/head/anchor 数值相同；0/缺失/无限值、单边修改、进程挂死与整次运行超时负例均正确失败
- [ ] 噪声注入触发环境/不确定而非自动调高 cap；一次补测合并全部样本，无挑选性通过
- [ ] base/head/anchor 三方比较阻止累计小回归；baseline 或 fixture 更新必须审阅
- [ ] 0.8 stdio 全部 §8 场景完成，错误帧/超限帧/慢 reader/父进程退出/取消后再请求均可重跑验证
- [ ] 连续 7 日 /1.0 连续 14 日门禁统计包含完整 artifact 集；例外、缺测、真实失败会打断窗口
- [ ] tag 的最终 bytes、版本、source SHA 与校验报告一致；证据不过期；正式发布前全部必需检查通过

## 14. 风险、替代方案与不采纳项

| 风险 / 备选 | 判断与措施 |
|---|---|
| 只用总包大小 | 一个 ABI 的增长可被另一文件减少掩盖，且压缩会隐藏 bitcode；采用 per-file + package 双层预算 |
| 所有 target 统一关闭 LTO | 能减 `.a`，可能放大动态库且降低性能；只切 staticlib 消费路径，动态库保持原 profile |
| 所有 benchmark 跑公共 shared runner | 成本低但硬件漂移与抢占使门禁不可靠；shared runner 做 smoke，专用隔离环境做 P |
| 固定绝对时间、不比较 baseline | 机器差异大，早期回归可能尚未撞 cap；使用绝对护栏 + 配对回归 + 长期 anchor |
| 只用百分比 | 极小值噪声放大、内存接近零无法计算；加入微秒绝对差和 MiB 增量，保留绝对护栏 |
| 每次自动重建 baseline | 累计慢涨无法被发现；baseline 受审阅且 anchor 在版本内固定 |
| feature 默认关闭 / 删 provider | 违背默认全量与 API 约束；只允许明确 opt-in slim，收益不冒充 default |
| 立即要求所有硬件/绑定完整性能矩阵 | 成本过高且缺少稳定设备；Linux 核心/主要调用边界做硬 P，其他绑定保留功能/资源 smoke，新增稳定平台再注册 |
| 体积优化伤害语义或安全 | 保留字节级 cassette、错误/取消/限额测试；任何 correctness failure 不可性能豁免 |
| CI 耗时增加 | artifact 缓存 keyed by SHA/lock/recipe/profile/target；PR 用依赖闭包，nightly/tag 全量。缓存命中仍需校验 bytes，不能缓存“通过”状态跨 SHA |
| 没有完整逐平台历史测量 | 不虚构当前值；把当前上限与待注册状态分开，0.6 发布以真实登记为阻塞条件 |

**决定结果**：本 RFC 将“体积与性能是第一基准线”变为有产物身份、有测量契约、有可重现统计、有失败原因、有发布前置条件的工程约束。缺少证据与发生回归同样不能视为完成；优化收益必须来自可验证的构建与运行改进。
