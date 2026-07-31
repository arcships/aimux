> **注：aimux-tools 和 aimux-macros 已于 2026-07-31 删除。文中相关整改建议已过时，仅保留历史记录。**

﻿# aimux 架构与规范层整改方案

> ⚠️ **此文档为 2026-07-27 的历史快照。** 部分整改项已完成，provider 数已从 23 增至 221。
> 基于 `QUALITY_REVIEW.md` 的反馈，对其中每一项问题进行独立深入调研后，给出的根因分析与架构/规范层解决方案。
> 调研日期 2026-07-27，基于当前工作区状态（HEAD `f52b2dc`）。

---

## 〇、调研结论总览（先修正事实）

调研发现评审报告整体方向准确，但有几处关键事实需修正——这些修正会直接影响整改优先级。

| 评审指控 | 调研核实 | 结论 |
|---------|---------|------|
| ⛔ 现在编译不过（E0061，convert.rs 9→4 参数） | 当前 HEAD `f52b2dc`（领先评审依据的 `fc297b6` 三个提交），`cargo check --workspace` 通过，`cargo test --workspace` 全部 0 failed。convert.rs:267 函数定义实为 9 参数，与调用处匹配 | **指控对当前状态不成立**。但"无 CI 导致半完成重构曾入库"的制度性问题成立 |
| 四个 HTTP helper 全零调用 | `with_user_agent_suffix` 被 Azure 实际使用（azure/model.rs:326）；`post_json_to_api`/`handle_fetch_error`/`combine_headers` 三个确为零调用 | **3/4 成立**。死代码根因不是"忘接"，而是 helper 能力低于 provider 真实需求 |
| AbortSignal 约 175 行 | 实际 234 行（util.rs:502-735） | 行数低估，缺陷与零调用结论成立 |

| CallOptions "14 字段全 None" | 实际 15 字段：1 个 prompt（必需）+ 1 个 tool_choice（非 Option，有 Default）+ 13 个 Option | 痛点属实，措辞略不精确 |
| 8 个文件超 500 行、2 个超千行 | 实际 **12 个**超 500 行、**3 个**超千行（漏报 google/convert.rs 1034 行）；anthropic/convert.rs 实 1493 行非 1310 | **比评审更严重** |
| 7 处 #[allow] | 实际 **13 处**（src 9 + test 4） | 少计，问题更普遍 |
| HANDOFF 债务 #4 仍记待办 | HANDOFF.md:262 已标 `~~删除线~~ ✅ 已解决` | **评审误判**，#4 文档已正确更新 |
| HANDOFF #9（DeepSeek 薄封装）仍记待办 | 代码已有独立 DeepSeekModel（661 行）+ reasoning_content 解析，文档未更新 | 指控成立 |

**核心矛盾（评审总结，调研印证）**：项目在"翻译 TS 实现深度"上投入极大且做得好，但在"Rust 侧的接线与去重"上投入不足——建了很多架子，没通电就转向了下个 TS 功能的翻译。

---

## 一、工程纪律层：本地自动化门槛与提交规范（一切的前提）

### 1.1 现状

- 纯本地项目，无云端 CI。无 `rustfmt.toml`/`clippy.toml`/`rust-toolchain`，无任何 git hook。
- `cargo clippy --workspace --all-targets` 产生 **321 行 warning**，无人拦截。
- 13 处 `#[allow(...)]` 散布未清理（9 src + 4 test）。
- git 历史仅 11 提交，cache_control 重构一度堆在工作区未提交（后被提交，但期间状态不可编译）——正是"无门槛喊停"的典型后果。

### 1.2 解决方案

纯本地项目没有云端 CI 兜底，更要把门槛做在 git hook 和工具链配置上——让"半完成重构"在 `git commit`/`git push` 时被本地拦截。

**第一步：工具链配置（固定基线）**

根目录新增：
- `rust-toolchain.toml`：锁定 stable channel + components，避免工具链漂移导致"我这能编译你那不能"。
  ```toml
  [toolchain]
  channel = "stable"
  components = ["rustfmt", "clippy"]
  ```
- `rustfmt.toml`：固定格式（`edition = "2021"` 等）。
- `clippy.toml`：`msrv = "1.75"` 等基线。

**第二步：git hook 本地门槛（核心）**

引入 `cargo-husky`（dev-dependency，`cargo test` 时自动安装 hook，无需手动 `git config`），或手写 `.git/hooks/` 脚本。推荐两层 hook：

- **pre-commit（轻量、快）**：只跑 `cargo fmt --all -- --check` + `cargo check --workspace`。秒级，保证"每次提交都可编译"——这是 cache_control 事故的直接防线。
  ```sh
  #!/bin/sh
  cargo fmt --all -- --check || exit 1
  cargo check --workspace || exit 1
  ```
- **pre-push（重量、全）**：跑 `cargo clippy --workspace --all-targets -- -D warnings` + `cargo test --workspace`。只在 push 时触发，避免每次提交都等全量测试。

> 用 `cargo-husky` 时在 `Cargo.toml` 配 `[dev-dependencies] cargo-husky = { version = "1", default-features = false, features = ["precommit-hook", "run-cargo-test"] }`，并按需扩展生成的 hook 脚本加入 clippy。或直接手写 `.git/hooks/` 脚本更可控（不引入依赖）。

**第三步：纪律规则（写入 CONTRIBUTING.md 或 README）**
1. **小步可编译提交**：每次 `git commit` 前 hook 自动跑 `cargo check`，挂了就不让提交。大重构拆成"每步可编译"的提交序列，禁止"改了一半就停手"。
2. **clippy 零 warning**：pre-push hook 用 `-D warnings`，新增 warning 直接挡 push。存量 321 warning 分批清零（不一次性 `allow` 掩盖）。
3. **禁止用脚本改 Rust**：删除 `add_provider_options.py`。需要批量改 Rust 时走 `sed`/`rustfmt`/语义化工具或小步手改 + 编译验证（见第三节 CallOptions 修复，会消掉该脚本存在的原因）。

### 1.3 存量 clippy 清理策略

321 warning 不可一次清完，且在 hook 启用前必须先降到 0，否则 pre-push 会一直挡自己。策略：
1. **先清零再开 hook**：分批 `cargo fix` + 手改，把 warning 降到 0 后再启用 `-D warnings` 的 pre-push。
2. 按类别分批：
   - `unused_imports`/`unused_mut`（如 openai/convert.rs:760、:181）：直接 `cargo fix --allow-dirty`。
   - `dead_code`（9 处 #[allow]）：逐个核实——真死代码删除，预留功能的加注释说明。
   - `clippy::too_many_arguments`（anthropic/convert.rs:266）：改参数结构体，不靠 `allow` 压制。

---

## 二、架构层：四项核心整改

### 2.1 工具子系统重设计（最严重的架构断裂）

#### 现状

三层抽象**完全断开**，不是"没闭环"：

```
声明侧（发给模型）          执行侧（运行时）            宏（本应桥接）
FunctionTool/ToolSet   ✗   ToolFn/ToolExecutor    ✗   #[tool]（生成代码无法编译）
CallOptions.tools           generate_text 未接入         description 被丢弃
                                                        无 schema 生成
```





#### 根因

把 TS 的"声明对象 + 执行函数 + 宏"三件套照搬，但 TS 里三者通过运行时对象天然关联；Rust 需要类型层桥接，而当前 trait 设计（`&Value -> Value` 无类型边界）和宏实现（不 inspect 签名、不生成 schema）都没提供这个桥接。

#### 解决方案：统一工具 trait + 重写宏

**Step 1：扩展 `ToolFn` trait，让单个对象同时提供声明与执行**

```rust
// aimux-tools/src/tool_executor.rs
#[async_trait::async_trait]
pub trait ToolFn: Send + Sync {
    fn name(&self) -> &str;
    /// 产出发给模型的声明（schema + description）
    fn definition(&self) -> FunctionTool;
    /// 运行时执行
    async fn execute(&self, args: &serde_json::Value)
        -> Result<serde_json::Value, AiMuxError>;
}
```

这从根上消除"声明侧与执行侧两套注册表并行手写"的问题。

**Step 2：让 `ToolSet` 持有 `ToolFn` 对象，按需导出声明**

```rust
pub struct ToolSet {
    tools: Vec<Box<dyn ToolFn>>,  // 单一来源
}
impl ToolSet {
    pub fn definitions(&self) -> Vec<FunctionTool> {
        self.tools.iter().map(|t| t.definition()).collect()
    }
    pub async fn execute(&self, name: &str, args: &Value) -> Result<Value, AiMuxError> { ... }
}
```

消除 `ToolSet`（声明）与 `ToolExecutor`（执行）的双轨。

**Step 3：接通执行回路**

`generate_text`/`stream_text` 接收 `ToolSet`，收到模型 tool call 时自动派发执行（这是 agent loop 的基础）。

**Step 4：重写 `#[tool]` 宏**

用 `syn::parse::Parse` 解析属性（支持逗号安全的 `name = "..."` / `description = "..."`），遍历 `sig.inputs` 生成匿名 `#[derive(Deserialize)]` 参数结构体，同时产出 `definition()`（含 schema）与 `execute()`（`serde_json::from_value` 反序列化后按位传参）。理想生成代码草图见调研报告。

**Step 5：加测试**

为 `aimux-macros` 引入 `trybuild`（编译测试）+ `macrotest`（展开快照），把 doctest 从 `ignore` 改为可编译。

#### 业界对比

`reference/anthropic-sdk-rust` 用"声明对象 + 执行 trait + registry 配对注册"模式；`reference/edgequake-llm` 用纯数据 `ToolDefinition` + 泛型 `parse_arguments::<T>()`。Rust 生态通行做法是用 `schemars` 的 `JsonSchema` derive 让参数结构体自带 schema，或泛型 `Tool<Args: DeserializeOwned + JsonSchema, Output: Serialize>` 在类型层闭环。本项目已引入 `schemars`（Cargo.toml:50）却未用于工具——应启用。

---

### 2.2 HTTP 执行层统一（消灭重复 + 死代码）

#### 现状

- 3 个 helper 是死代码（`post_json_to_api`/`handle_fetch_error`/`combine_headers`），1 个在用（`with_user_agent_suffix`）。
- 死代码根因：helper 能力低于 provider 真实需求——
  - `post_json_to_api` 用 `req.header(k,v)` 直接塞 `&String`，非法 header 名会 panic；而所有 provider 内联版都做了 `HeaderName::try_from().ok()` 防御性跳过。**接通现版本 = 健壮性回退**。
  - 无法发送预签名原始字节（Bedrock SigV4 需要）。
  - `handle_fetch_error` 依赖 JS 运行时错误字符串（"fetch failed"/Bun code），与 reqwest 错误模型不适配。
- provider 侧重复：12 处头部构建样板、28 处错误映射、9 个 provider 各自重复 `.send().await` + 状态码检查。

#### 解决方案：升级 helper 到"够用"再采纳，而非直接接通现版本

**删除**：
- `handle_fetch_error`——TS 直译，与 reqwest 错误模型不适配，重试决策已由 `retry` 模块负责。
- `combine_headers`——极简且零调用，不重构就直接删。

**升级 `post_json_to_api` 后采纳**（二选一，推荐升级）：

```rust
// aimux-provider-utils/src/http.rs
pub async fn post_json(
    client: &reqwest::Client,
    url: &str,
    headers: Option<&HeaderMap>,        // 调用方构建好的、已做 try_from 校验的
    body: &serde_json::Value,
) -> Result<reqwest::Response, AiMuxError> {
    let mut req = client.post(url)
        .header(CONTENT_TYPE, "application/json");
    if let Some(h) = headers { req = req.headers(h.clone()); }
    req.json(body).send().await
        .map_err(|e| classify_reqwest_error(e))  // reqwest 原生错误分类
}

/// 支持原始字节（Bedrock SigV4 用）
pub async fn post_raw(
    client: &reqwest::Client,
    url: &str,
    headers: Option<&HeaderMap>,
    body: Vec<u8>,
) -> Result<reqwest::Response, AiMuxError> { ... }

/// reqwest 错误 → AiMuxError（替代不适配的 handle_fetch_error）
fn classify_reqwest_error(e: reqwest::Error) -> AiMuxError {
    if e.is_timeout() || e.is_connect() { AiMuxError::RateLimited(...) }  // 可重试
    else { AiMuxError::Http(e.to_string()) }
}
```

配套集中化 status → `parse_provider_error`，让 provider 只写"差异"（URL/body/header/响应解析），HTTP 执行样板不再重复。

**保留**：`with_user_agent_suffix`（Azure 在用）。

---

### 2.3 取消语义：删旧建新

#### 现状

- `AbortSignal`/`AbortController`（util.rs:502-735，234 行）全仓 src 零调用。
- 两个缺陷：`timeout` 丢弃 `tokio::spawn` 的 `JoinHandle` 导致任务泄漏；`Mutex<Vec<Box<dyn Fn>>>` 监听器有死锁/poison 风险。
- `CallOptions`/`GenerateTextOptions`/`generate_text`/`stream_text` 全无 abort 字段。

#### 解决方案

**立即删除** 234 行 + 配套测试（util_test.rs:505-768 + TestError + Duration 导入）。删除安全：符号不在 prelude，工作区 0.1.0 pre-1.0，无内部消费者。可选移除 aimux-core 的 tokio 依赖（删除后 src 内零 tokio 调用）。

**未来需要取消语义时**，引入 `tokio_util::sync::CancellationToken`（无锁、原生 `select!` 友好），并**真正接入管线**：
- `GenerateTextOptions`/`CallOptions` 增 `abort: Option<CancellationToken>`。
- `generate_text`/`stream_text` 用 `tokio::select!` 在 provider future 与 `token.cancelled()` 间竞争。
- 超时用 `tokio::time::timeout`，不再 `spawn` 不可取消的 sleep。
- provider 层用 `token.cancelled()` 包裹 `RequestBuilder::send`。

---

### 2.4 错误处理与数据完整性

#### (a) Bedrock event_stream CRC 校验（P1，生产路径缺陷）

**现状**：编码侧正确写 CRC，解码侧 `decode_messages`（event_stream.rs:128-227）从不校验——prelude_crc 和 msg_crc 静默跳过。该函数在生产路径被调用（bedrock/model.rs:254 解析真实 HTTP 响应）。后果：损坏帧被当有效消息解析，单点损坏扩散为整流解同步（`offset += total_length` 跳到错误位置）。往返测试因输入合法而掩盖。

**解决方案**：
1. 在 `decode_messages` 中，**信任 total_length/headers_length 之前**先校验 prelude_crc；帧尾校验 msg_crc。失败时 `break`（或改签名为 `Result<Vec<EventStreamMessage>, DecodeError>`）。
2. 补"损坏帧被拒"测试（翻转 payload 一位、翻转 length 一位），防回归。
3. 修正函数文档（event_stream.rs:127 "Malformed frames are skipped" → 明确含 CRC 失败）。

#### (b) Google convert 注释-行为矛盾（P1，语义 bug）

**现状**：google/convert.rs:64-71 注释称"match TS SDK's hard error"，代码却无任何语句、直接继续 push——中段 system 消息被误塞进 `systemInstruction`，语义错误。函数返回 `GooglePrompt` 非 `Result`，无上报能力。TS 上游确硬抛 `UnsupportedFunctionalityError`。

**解决方案（按对齐 TS 程度排序）**：
1. **推荐**：签名改 `Result<GooglePrompt, UnsupportedFunctionalityError>`，`!system_messages_allowed` 时返回错误，`?` 传播。最对齐 TS，属破坏性改动但"正确"。
2. 务实折中：加 `debug_assert!(false, ...)` + 保持容错 + 改正注释。
3. 次优：`continue` 跳过 + 改正注释。

无论选哪个，都**不应维持现状**（把中段 system 塞进 systemInstruction 是语义 bug），并补"system 在 user 之后"的测试。

---

## 三、规范层：API 与组织一致性

### 3.1 CallOptions 规范化（消掉 Python 脚本的需求）

**现状**：`CallOptions`（options.rs:32）仅 `Debug, Clone`，15 字段（prompt 必需 + tool_choice 有 Default + 13 个 Option）。每个测试文件手写 `default_options(prompt)` 辅助函数罗列全 None。`add_provider_options.py` 正是为批量补字段而生。

**解决方案（推荐方案 a）**：拆 `CallOptionsData` + `CallOptions`

```rust
#[derive(Debug, Clone, Default)]
pub struct CallOptionsData {
    pub max_output_tokens: Option<u32>,
    // ... 13 个 Option + tool_choice（已有 Default）
    pub tool_choice: ToolChoice,
}

#[derive(Debug, Clone)]
pub struct CallOptions {
    pub prompt: LanguageModelPrompt,
    #[serde(flatten)]
    pub data: CallOptionsData,
}
```

测试可 `CallOptions { prompt, ..Default::default() }`，与 `GenerateTextOptions` 形态对齐。落地前 grep provider 里 `opts.<field>` 的访问点，评估是否需 `Deref` 或直接展开字段。

**收益**：直接消掉 `add_provider_options.py` 存在的需求；新增 Option 字段时所有测试自动获得默认值。

### 3.2 依赖与 feature flag 管理

**现状**：
- `aimux-providers` 无 `[features]` 段，23 个 provider 全量编译。只想要 OpenAI 的用户也拉入 `sha2`/`hmac`/`chrono`（仅 Bedrock/AWS sigv4 用）。
- `base64`/`sha2`/`hmac`/`hex`/`chrono`/`wiremock` 直接写死版本号，未纳入 `[workspace.dependencies]`（该段存在但未覆盖这些）。

**解决方案**：
1. `aimux-providers/Cargo.toml` 加 `[features]`，每 provider 一个 feature，`default = ["openai"]`：
   ```toml
   [features]
   default = ["openai"]
   openai = []
   bedrock = ["dep:sha2", "dep:hmac", "dep:hex", "dep:chrono"]
   anthropic-aws = ["dep:sha2", "dep:hmac"]
   ```
2. `lib.rs` 的 `pub mod` 用 `#[cfg(feature = "...")]` 门控。
3. `base64`/`sha2`/`hmac`/`hex`/`chrono`/`wiremock` 提升到根 `[workspace.dependencies]` 统一版本。

### 3.3 文件组织规范

**现状**：12 个文件超 500 行、3 个超千行（anthropic/convert.rs 1493、openai/convert.rs 1068、google/convert.rs 1034）。util.rs 735 行堆四个不相关功能。openai/model.rs 的 `execute_stream` 单函数 260 行内联状态机。

**解决方案**：
- **util.rs 拆分**：`json_repair.rs`（fix_json + parse_partial_json，强相关）、`math.rs`（cosine_similarity）、删除 abort 段。
- **convert.rs 拆分**：按职责分"请求构建 / 响应解析 / 工具准备 / usage 转换"子模块。
- **execute_stream 重构**：提取 `fn process_event(...) -> Vec<StreamPart>` 纯函数 + 状态结构体，便于单测。
- **规范**：CONTRIBUTING.md 约定"src 文件超 500 行需有拆分理由；单函数超 100 行考虑提取"。

### 3.4 流式 buffer 性能

**现状**：ndjson.rs:59-60 和 sse.rs:131-133 每解析一行/事件都 `buffer[..pos].to_string()` + `buffer[pos+1..].to_string()` 整体拷贝剩余 buffer，O(n²)。README 称"零拷贝 SSE"与实现不符。

**解决方案**：
- 改 `String::drain(..pos)` 原地移除已消费前缀，剩余部分不重新分配。
- 或改游标索引（`usize` 偏移）+ 定期 `drain` 压缩。
- README 措辞改为"基于 tokio-stream 的流式 SSE/NDJSON 解码"，去掉"零拷贝"；或改用 `BytesMut` + `split_to` 真正兑现零拷贝。

### 3.5 文档与代码一致性

**现状**：HANDOFF.md #9（DeepSeek 薄封装）代码已修复但文档未更新；README 勾选"#[tool] 基础实现已完成"但宏无法编译。

**解决方案**：
- 通读 HANDOFF.md，所有已完工项统一打 ✅（#9 需补）。
- README 的功能矩阵与代码实际状态对齐——`#[tool]` 未完成就标 WIP，不要勾选。
- **规范**：CONTRIBUTING.md 约定"功能完工的判定标准是 CI 通过 + 集成测试覆盖，而非代码存在"；文档变更与代码变更同提交。

---

## 四、执行优先级与路线图

按"依赖关系"与"风险收益比"排序：

### 阶段一：止血（1-2 天，无破坏性）✅ 已完成
1. **补本地门槛**：`rust-toolchain.toml` + `rustfmt.toml` + git hook（pre-commit 跑 fmt+check，pre-push 跑 clippy+test）。这是一切的前提。
2. **删 AbortSignal**（234 行 + 测试），可选移除 aimux-core 的 tokio 依赖。
3. **删 `handle_fetch_error`/`combine_headers`** 死代码。
4. **修 event_stream CRC 校验** + 补损坏帧测试。
5. **修 google convert**（至少方向 2/3 + 改注释 + 补测试）。
6. **更新 HANDOFF.md #9**、修正 README `#[tool]` 状态。

### 阶段二：规范化（局部破坏性）— 部分完成
7. ✅ **CallOptions 构造器**：未采用原计划的 `CallOptionsData` 拆分（会破坏 100+ 处字段访问），改用 `CallOptions::new(prompt)` 构造器方案，效果等价——16 个测试 helper 简化为一行，加字段只改构造器一处。同时给 `LanguageModelPromptMessage` 加 Default（70 处 `..Default::default()`）、给 ContentPart 9 个变体加构造器（~50 处）、给 FunctionTool 加 `::new` + builder（15 处）。`add_provider_options.py` 已删除，根因消除。
8. ✅ **存量 clippy 清零**：329 warning → 0（`cargo clippy --fix` 自动修 133 处 + 手工修剩余）。pre-push hook 已开 `-D warnings`。
9. ✅ **util.rs 拆分**：拆成 `json_repair.rs`（fix_json + parse_partial_json）+ `math.rs`（cosine_similarity），util.rs 降为 re-export 转发入口。convert.rs 大文件拆分**未做**——3 个超千行文件（anthropic/openai/google convert.rs）按职责拆分工程量大、收益主要是可读性，当前不紧迫，留待按需推进。
10. ❌ **feature flag + 依赖统一**：**不做**。这是纯本地项目，无外部消费者需要裁剪依赖；sha2/hmac/hex/chrono 合计编译时间对 `cargo check` 的 13 秒几乎无影响。feature flag 的维护成本（23 个 mod 门控 + `--all-features` 测试）与收益不成比例。若未来发布到 crates.io 再重新评估。
11. ❌ **流式 buffer 改 `drain`**：**不做**。SSE/NDJSON 的剩余 buffer 在实践中很小（每 chunk 通常是单行/单事件），O(n²) 的 n 是几十到几百字节，对秒级 LLM API 延迟完全可忽略。代码改动虽小但收益微乎其微。README "零拷贝" 措辞待修正（改为"基于 tokio-stream 的流式 SSE/NDJSON 解码"）。

### 阶段三：架构重设计（1-2 周，破坏性）
12. **工具子系统重设计**：扩展 `ToolFn` trait（加 `definition()`）→ 统一 `ToolSet`/`ToolExecutor` → 接通 `generate_text` 执行回路 → 重写 `#[tool]` 宏 → 加 trybuild 测试。
13. **HTTP 执行层统一**：升级 `post_json_to_api`（加 header 校验 + raw body + reqwest 错误分类），迁移 9 个 provider 消灭重复。

### 阶段四：取消语义（可选，按需）
14. 引入 `tokio_util::sync::CancellationToken`，接入 `CallOptions`/`generate_text`/provider 层。

---

## 五、核心原则

1. **接通优先于新建**：已建的架子（工具宏、HTTP helper、取消信号）通电，比再翻译新 TS 功能紧迫。
1. **本地门槛是制度保障**：纯本地项目无云端 CI，更要把 `cargo check`/clippy 做进 git hook——无门槛是"半完成重构入库"的制度性根因，必须最先补。
3. **Rust 侧去重**：翻译 TS 时不要把 TS 的运行时动态关联照搬——Rust 需要类型层桥接（工具 trait 加 `definition()`、HTTP helper 升级到够用再采纳）。
4. **测试覆盖真实缺陷路径**：CRC 往返测试因输入合法而掩盖缺陷；宏 doctest 标 `ignore` 掩盖无法编译。测试要覆盖"损坏/非法/边界"路径。
5. **文档与代码同提交**：功能完工标准是 CI + 测试，而非代码存在；文档变更随代码变更落地。
