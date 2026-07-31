# RFC 0005: 改名为 aimux

> **状态**：已定方案，待执行
> **决策**：错误类型 `AiMuxError` · 仓库名 `aimux` · 品牌定位「受 Vercel AI SDK 启发」· 脚本批量执行 + 验证

## 一、背景与动机

当前项目名为 `aimux` / `aimux`。改名动机有二：

1. **品牌身份**：`aimux` 连写形式容易让人联想为 Vercel AI SDK 的标识符。虽然严格来说 Vercel 的官方名是 "AI SDK"、npm 包是 `ai`、仓库是 `vercel/ai`，`aimux` 并非其注册标识符，但概念关联过强，不利于建立独立品牌。
2. **Rust 命名惯例**：Rust crate 惯例是 kebab-case 分词（`ai-sdk-core`），当前 `aimux-core` 把 ai 与 sdk 粘在一起，不规范。

## 二、命名选型理由

经多轮筛选（见会话记录），最终选定 **`aimux`**：

- **架构语义最准**：mux（multiplexer，多路复用器）= 一个输入接口背后切换多路信号源，与本项目的「统一 `LanguageModel` trait 扇出多家 provider」架构字面对应。
- **短**：5 字母，`aimux::` 前缀干净。
- **含 AI 关键词**：一眼看出领域。
- **crates.io 可用**：主名 `aimux` 及子 crate 命名空间 `aimux-core`/`aimux-providers` 等全部可用（已逐一核验）。
- **赛道稀缺**：直白派 `llm-mux`/`modelmux`/`llm-bridge`/`llmkit`/`aikit`/`llm-rs`/`modelforge` 均已被同类竞品占用，`aimux` 是少有的既短又准又可用的幸存者。

被排除的候选及原因：
- `duplex`（双工）：语义强调双向通信 2-way，与 1→N 扇出架构不符；且 bare `duplex` 已被占用（sunfishcode 的 IO trait crate，6.2 万下载）。
- `aireq`/`aifetch`/`airun`/`modelrun`（动作动词）：只点单一动作，漏掉「多 provider 统一 + 文本/对象/工具/流式全家桶」核心卖点。
- `oxide-ai`/`prism-ai`（隐喻派）：可用但不含 AI 关键词。

## 三、命名映射总表

| 层面 | 现状 | 目标 |
|------|------|------|
| 仓库名 / 根目录 | `aimux` | `aimux` |
| crate 名（6 个） | `aimux-core` 等 | `aimux-core` / `aimux-stream` / `aimux-providers` / `aimux-provider-utils` |
| 目录名（6 个） | `aimux-core/` 等 | `aimux-core/` 等 |
| 代码标识符（下划线） | `aimux_core` / `aimux_providers` / `aimux_stream` / `aimux_provider_utils` | `aimux_core` / `aimux_providers` / `aimux_stream` / `aimux_provider_utils` |
| 核心错误类型 | `AiMuxError`（遍布 25+ 文件） | `AiMuxError` |
| 测试环境变量 | `AISDK_TEST_LOAD_API_KEY_VAR` | `AIMUX_TEST_LOAD_API_KEY_VAR` |
| 品牌定位 | 「对标 Vercel AI SDK / 高性能替代方案」 | 「受 Vercel AI SDK 启发的 Rust 实现」 |

## 四、影响范围分层

### 第 1 层：目录改名（6 个）

```
aimux-core/          → aimux-core/
aimux-stream/        → aimux-stream/
aimux-providers/     → aimux-providers/
aimux-provider-utils/→ aimux-provider-utils/
```

用 `git mv` 保留历史。

### 第 2 层：Cargo.toml（7 个文件）

- **根 `Cargo.toml`**：
  - `members` 列表 6 条
  - `workspace.dependencies` 里 4 条 `aimux-*` 引用
  - `repository` URL（`github.com/yourusername/aimux`）
  - `description`
- **6 个子 crate 的 `Cargo.toml`**：
  - `name = "aimux-xxx"` → `aimux-xxx`
  - `description` 字符串
  - `dependencies` / `dev-dependencies` 里的 `aimux-*` 引用

### 第 3 层：Rust 源码（100+ 文件）

- `use aimux_core::` → `use aimux_core::`（含全路径形式 `aimux_core::error::AiMuxError`）
- `aimux_providers::` / `aimux_tools::` / `aimux_macros::` 同理
- `AiMuxError` → `AiMuxError`（核心改动，25+ 文件，含 enum 定义、impl、所有 use 和引用）
> **注：aimux-tools 和 aimux-macros 已于 2026-07-31 删除**，以下工具相关内容已过时，仅保留历史记录。

- `aimux-macros/src/lib.rs` 的 `quote!` 宏里**生成代码**引用 `aimux_tools::ToolFn` 和 `aimux_core::error::AiMuxError`——必须同步改，否则用户用 `#[tool]` 宏展开后编译失败
- 各 lib.rs 顶部 `//! # aimux-core` 等 crate 文档注释

### 第 4 层：文档（10+ 文件）

- `README.md`：标题、目录树、代码示例、对照表、品牌定位语句
- `HANDOFF.md` / `QUALITY_REVIEW.md` / `REMEDIATION.md` / `TEST_AUDIT.md` / `TRACKING.md`：文件路径引用、`AiMuxError` 提及
- `docs/01~14` + `docs/README.md`：少数提及
- `rfc/0001-multilang-bindings.md` / `rfc/0004-provider-inventory.md`：相对路径链接 `../aimux-core` 等（目录改名后链接失效，必须同步改）

### 第 5 层：脚本

- `scripts/convert_cassettes.py:13`：`OUT_DIR = Path("aimux-providers/tests/cassettes")` → `aimux-providers/tests/cassettes`

## 五、明确不动项（边界）

| 项 | 原因 |
|----|------|
| `reference/` 整个目录 | 第三方参考项目（TokenHub / traceloop-hub / uni-api / unia / rig cassettes），非本项目代码 |
| `docs/07-kernel-infrastructure.md` 里的 `AISDKError` | 描述 Vercel AI SDK 原生类 `@ai-sdk/provider.AISDKError`，是上游事实，非本项目符号 |
| `aimux-providers/tests/fal_transcription_test.rs` 里的 `"Versal AISDK"` | mock 的 provider 返回文本（虚构服务名），与项目改名无关 |
| `Cargo.lock` | 目录/crate 改名后 `cargo check` 自动重生成 |
| `.githooks/`（pre-commit / pre-push） | 不含 aimux 字样 |
| `rust-toolchain.toml` / `rustfmt.toml` / `.gitattributes` / `.gitignore` | 与命名无关 |

## 六、关键风险点

1. **宏生成代码**（`aimux-macros/src/lib.rs` 的 `quote!`）：替换必须在 `aimux_tools` 和 `aimux_core::error::AiMuxError` 两处同步，否则 `#[tool]` 宏用户编译失败。当前 README 标注 macros 子系统「待重写未闭环」，但改名仍要保持引用一致。
2. **相对路径链接**：`rfc/` 里的 `../aimux-core` 链接，目录改名后失效。
3. **脚本批量替换的误伤**：必须**排除 `reference/`**，否则会把 Vercel 原生 `AISDKError` 描述、mock 数据一起改掉。
4. **大小写敏感**：`aimux`（小写）/ `AiMux`（驼峰）/ `AISDK`（全大写）三种形态要分别处理，不能一个正则通吃。具体替换规则：
   - `aimux` → `aimux`
   - `AiMux` → `AiMux`
   - `AISDK` → `AIMUX`
   - `aimux` → `aimux`（仓库名去掉 -rs，注意此替换要在 `aimux`→`aimux` 之前或单独处理，避免重复替换）

## 七、执行步骤（脚本批量方案）

### 步骤 1：目录改名

```powershell
git mv aimux-core aimux-core
git mv aimux-stream aimux-stream
git mv aimux-tools aimux-tools
git mv aimux-macros aimux-macros
git mv aimux-providers aimux-providers
git mv aimux-provider-utils aimux-provider-utils
```

### 步骤 2：批量替换文本（PowerShell 脚本）

脚本需满足：
- 作用范围：根目录下的 `Cargo.toml`、`*.rs`、`*.md`、`scripts/*.py`
- **排除 `reference/` 目录**
- **排除 `target/` 目录**
- 大小写敏感，按 `aimux`→`aimux`、`AiMux`→`AiMux`、`AISDK`→`AIMUX` 三组替换
- `aimux`（仓库名）需单独处理为 `aimux`，注意顺序：先替换 `aimux`→`aimux`，再替换 `aimux`→`aimux`，避免 `aimux` 被拆成 `aimux-rs`

伪代码：
```powershell
$root = "C:\Users\eric8\Desktop\code\aimux"  # 注意：根目录本身暂不改名，由用户后续 git remote/rename 处理
$files = Get-ChildItem -Recurse -File $root -Include *.rs,*.toml,*.md,*.py |
  Where-Object { $_.FullName -notmatch '\\reference\\' -and $_.FullName -notmatch '\\target\\' -and $_.FullName -notmatch '\\.git\\' }

foreach ($f in $files) {
    $content = Get-Content $f.FullName -Raw -Encoding UTF8
    # 仓库名先处理（注意顺序）
    $content = $content -replace 'aimux', 'aimux'
    # 三种大小写形态
    $content = $content -replace 'aimux', 'aimux'
    $content = $content -replace 'AiMux', 'AiMux'
    $content = $content -replace 'AISDK', 'AIMUX'
    Set-Content $f.FullName -Value $content -NoNewline -Encoding UTF8
}
```

### 步骤 3：手动检查边界

- 确认 `reference/` 目录未被触碰
- 确认 `docs/07` 的 `AISDKError`（Vercel 原生类描述）未被误改 —— **此条需在脚本后手动 grep 确认**，因为 `AISDK`→`AIMUX` 会误伤。处理：脚本执行后，手动把 `docs/07` 中描述 Vercel 原生类的 `AIMUXError` 改回 `AISDKError`。
- 确认 `fal_transcription_test.rs` 的 mock 文本 `"Versal AISDK"` 未被误改 —— 同上，`AISDK`→`AIMUX` 会误伤，需手动改回 `"Versal AISDK"`。

### 步骤 4：验证

```powershell
cargo check --workspace --all-targets
cargo test --workspace
```

`.githooks/pre-commit` 会自动跑 `cargo fmt --all --check` + `cargo check --workspace` 兜底。

### 步骤 5：根目录 / 仓库改名（用户自行处理）

本 RFC 范围内的脚本执行时，根目录名仍为 `aimux`。仓库改名（`git remote set-url`、GitHub repo rename、本地目录改名）由用户在脚本验证通过后自行执行，不在自动化范围内。

## 八、预计工作量

- 脚本执行 + 手动边界检查：5-10 分钟
- `cargo check` 首次编译验证：2-5 分钟
- `cargo test`：视测试套件规模，5-15 分钟
- 总计：约 15-30 分钟

## 九、回滚

若验证失败：
- `git status` 查看改动
- `git restore --staged .` + `git checkout .` 撤销（目录改名用 `git mv` 撤回）
- 或直接 `git reset --hard HEAD`（注意会丢失未提交的其他改动）
