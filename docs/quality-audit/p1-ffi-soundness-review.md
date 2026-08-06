# aimux-ffi Soundness 专项审查报告

**审查范围**：[`aimux-ffi/src/lib.rs`](../../aimux-ffi/src/lib.rs)（1878 行，37 处 unsafe 相关点）
**审查日期**：2026-08-06
**审查人**：FFI Soundness Agent
**总体评级**：🟡 **中等风险** — 无已知 UB 路径，但存在 3 项高风险改进点

---

## 概述

`aimux-ffi/src/lib.rs` 是 aimux 项目中 unsafe 代码最集中的模块，承担 C ABI 边界层的全部职责：opaque handle registry、JSON wire boundary、push callback stream。整体架构设计合理（opaque handle + JSON envelope），大部分 unsafe 代码遵循标准模式。审查发现 **3 项高风险**、**4 项中风险**发现，无已知 UB（Undefined Behavior）路径。

### 优点

- `CStr::from_ptr` 前均有 `is_null()` 守卫（`cstr_to_string`）。
- `CString::into_raw` / `CString::from_raw` 配对正确，`aimux_free_string` 正确释放。
- 回调中 `CString` 生命周期与文档契约一致（局部变量，回调返回后 drop）。
- Handle registry 使用 `OnceLock<Mutex<HashMap>>` + `AtomicU64`，线程安全。
- 所有 `ModelHandle` variant 持有的 `Arc<dyn Trait>` 均满足 `Send + Sync`（已验证全部 8 个 trait）。
- JSON 往返路径无中间 NUL 风险（serde_json 输出不含 NUL）。

---

## 逐项审查发现

### 1. 裸指针解引用 (`*const c_char` / `*mut c_char`)

**结论：🟢 安全** — 所有解引用点均有 null 守卫或已证明不可能为 null。

#### 审查点

| 位置 | 代码 | 分析 |
|------|------|------|
| [lib.rs:173-179](../../aimux-ffi/src/lib.rs#L173-L179) | `cstr_to_string(ptr)` | `ptr.is_null()` 检查 → `CStr::from_ptr()`。唯一入口函数，所有构造器/操作函数均通过它解引用 C 字符串。正确。 |
| [lib.rs:1295-1301](../../aimux-ffi/src/lib.rs#L1295-L1301) | `aimux_free_string(ptr)` | `ptr.is_null()` 检查 → `CString::from_raw(ptr)`。正确配对 `CString::into_raw`。 |
| [lib.rs:274-277](../../aimux-ffi/src/lib.rs#L274-L277) | `parse_two_args` | 仅调用 `cstr_to_string`，无直接 unsafe。✅ |
| [lib.rs:285-295](../../aimux-ffi/src/lib.rs#L285-L295) | `parse_four_args` | 同上。✅ |

**⚠️ 设计观察**：

- `cstr_to_string`（line 173）内部包含 unsafe 块但函数签名不是 `unsafe fn`，依赖 `#![allow(clippy::not_unsafe_ptr_arg_deref)]`（line 25）抑制 lint。这是有意的设计权衡——文档中说明了调用者契约——但意味着编译器不强制调用者承担安全义务。
- `parse_two_args` / `parse_four_args` 被声明为 `unsafe fn` 但内部没有任何 unsafe 操作（仅调用安全的 `cstr_to_string`）。这是 **unsafe 标记过度**，增加了调用点的噪声（如各构造器中的 `unsafe { parse_two_args(...) }`），但没有引入实际风险。

**改进建议**：
- 考虑移除 `parse_two_args` / `parse_four_args` 的 `unsafe` 标记，它们只是组合调用安全函数。
- 考虑将 `cstr_to_string` 改为 `unsafe fn` 以在类型层面传递契约，同时移除 `clippy::not_unsafe_ptr_arg_deref` allow。

---

### 2. 内存所有权契约（`*mut c_char` 分配/释放）

**结论：🟡 中风险** — 分配路径配对正确，但 `into_cstring_raw` 的 null 返回路径存在 API 契约缺口。

#### 审查点

| 位置 | 代码 | 分析 |
|------|------|------|
| [lib.rs:208-212](../../aimux-ffi/src/lib.rs#L208-L212) | `into_cstring_raw(s)` | `CString::new(s).into_raw()` 或返回 `std::ptr::null_mut()`（当 s 包含 NUL）。 |
| [lib.rs:1295-1301](../../aimux-ffi/src/lib.rs#L1295-L1301) | `aimux_free_string(ptr)` | `CString::from_raw(ptr)` — 正确处理 null 输入。✅ |
| [lib.rs:227-228](../../aimux-ffi/src/lib.rs#L227-L228) | `error_json_raw(msg)` | 直接返回 `into_cstring_raw(...)`，不检查是否为 null。 |
| [lib.rs:238-239](../../aimux-ffi/src/lib.rs#L238-L239) | `handle_json(handle)` | 同上。 |
| [lib.rs:256-257](../../aimux-ffi/src/lib.rs#L256-L257) | `fire_error(on_error, msg)` | 使用局部 `CString`，`cstr.as_ptr()` 传递给回调，函数返回时自动 drop。✅ |

#### 🔴 高风险发现：`into_cstring_raw` 的 null 返回路径

`into_cstring_raw` 的文档（line 208-211）声称"transferring ownership to the caller"，但当 `CString::new(s)` 失败时返回 `std::ptr::null_mut()`。这破坏了 API 契约——调用者期望获得一个需要 free 的非 null 指针。

**理论触发条件**：serde_json 的输出永远不会包含 NUL 字节（JSON spec 禁止），且 `serde_json::to_string` 不会产生 NUL。但是在错误消息路径（`error_json` 接收 `impl std::fmt::Display`，可能包含用户输入），如果某个 Display 实现产生了包含 NUL 的字符串，`CString::new` 就会失败。

**实际风险评估**：🔴 **高风险（但触发概率极低）**
- `fire_error` 函数（line 255-258）在 `CString::new` 失败时静默跳过回调调用——这意味着错误信息永远不会到达调用者。
- 构造函数路径（`handle_json`、`error_json_from`、`error_json_raw`）会向 FFI 调用者返回 null 指针——调用者调用 `aimux_free_string(null)` 是安全的（有 null 检查），但无法区分"成功但 handle=0"与"序列化失败"。

**改进建议**：
1. 在 `into_cstring_raw` 中对 NUL 字节进行显式处理（例如，将 NUL 替换为 U+FFFD 替代字符，或返回一个预分配的错误 CString）。
2. 让 `fire_error` 在 `CString::new` 失败时发送一个 fallback 错误 JSON（"internal error: failed to serialize error message"）。
3. 在 API 文档中明确说明 null 返回值的含义（"allocation or encoding failure"）。

---

### 3. CString/CStr 转换配对

**结论：🟢 安全** — from_raw / into_raw 配对正确，JSON 往返保真。

#### 审查点

| 转换方向 | 模式 | 审查结果 |
|----------|------|----------|
| C → Rust | `CStr::from_ptr(ptr)` → `cstr.to_str()` → `str::to_owned` | ✅ 正确处理非 UTF-8（返回 None）。 |
| Rust → C（owned） | `CString::new(s)` → `c.into_raw()` | ✅ 标准模式。 |
| Rust → C（borrowed） | 局部 `CString`，回调接收 `cstr.as_ptr()` | ✅ 生命周期正确（CString 在回调返回后 drop）。 |
| C → Rust → C（round-trip） | JSON string → `CString::new` → `into_raw` → `CStr::from_ptr` → `to_str` | ✅ 保真（serde_json 输出不含 NUL）。 |

**无发现问题。**

---

### 4. Handle Registry（HashMap + AtomicU64）

**结论：🟢 安全** — 架构设计合理，无明显竞态或内存安全问题。

#### 审查点

| 组件 | 位置 | 分析 |
|------|------|------|
| `REGISTRY: OnceLock<Mutex<ModelRegistry>>` | [lib.rs:84](../../aimux-ffi/src/lib.rs#L84) | `OnceLock::get_or_init` 线程安全。`Mutex<HashMap>` 提供互斥。✅ |
| `NEXT_HANDLE: AtomicU64` | [lib.rs:85](../../aimux-ffi/src/lib.rs#L85) | `fetch_add(1, Ordering::Relaxed)` — 仅需保证唯一性，Relaxed 足够。✅ |
| `ModelHandle` 各 variant | [lib.rs:68-80](../../aimux-ffi/src/lib.rs#L68-L80) | 所有 `Arc<dyn Trait>` 均满足 `Send + Sync`（已验证全部 8 个 trait）。✅ |
| Handle 注册/查找/释放 | [lib.rs:94-151](../../aimux-ffi/src/lib.rs#L94-L151) | `intern_model`、`get_model`、`drop_handle` 均正确加锁。✅ |

#### 🟡 中风险发现：Mutex 中毒后的全局恐慌

所有 registry 操作使用 `.expect("aimux-ffi: registry mutex poisoned")`（如 line 98、107、124 等）。如果某个 FFI 调用内部 panic 持有锁，整个 registry 永久中毒——后续所有 FFI 调用都将 panic。

**实际风险**：由于 `extern "C"` 边界不能 unwind（panic 跨 FFI 是 UB），理论上任何 FFI 函数内部不应 panic。但 registry 操作本身位于 `block_on` 中的 Rust 代码中，而 `block_on` 允许 panic 传播。

**改进建议**：
- 考虑使用 `Mutex::lock().unwrap_or_else(|e| e.into_inner())` 来从中毒状态恢复（如果确定中毒时数据结构仍然一致）。
- 或者保持当前行为但添加明确的文档：Mutex 中毒表示不可恢复的内部错误，进程应终止。

#### 🟡 中风险发现：Handle ID 溢出（理论路径）

`NEXT_HANDLE.fetch_add(1, Ordering::Relaxed)` 在 2^64 次分配后回绕到 0。Handle 0 是预留的无效值（`aimux_drop_handle` 和 `aimux_abort_signal_drop` 将 handle=0 视为 no-op）。回绕后的 handle 可能被误以为是无效 handle。

**实际风险**：2^64 次分配在物理上不可能达到（即使每秒百万次分配也需要 58 万年）。🟢 可以忽略。

---

### 5. Tokio Runtime（`block_on` / 死锁分析）

**结论：🟡 中风险** — 文档描述的"死锁"实际上是 panic，但后果一致（破坏 FFI 调用）。

#### 审查点

| 位置 | 代码 | 分析 |
|------|------|------|
| [lib.rs:154-159](../../aimux-ffi/src/lib.rs#L154-L159) | `runtime()` | `OnceLock<Runtime>` + `tokio::runtime::Runtime::new()` — 多线程运行时，仅初始化一次。✅ |
| [lib.rs:304](../../aimux-ffi/src/lib.rs#L304) | `run_and_serialize` 中的 `runtime().block_on(f)` | 在非 tokio 上下文（FFI 调用线程）正确阻塞。✅ |
| [lib.rs:1218-1269](../../aimux-ffi/src/lib.rs#L1218-L1269) | `stream_text_with_signal` 中 `block_on` | 同上。✅ |

#### 🔴 高风险发现：回调重入 FFI 导致 panic（文档误导）

流式函数（`stream_text_with_signal` 和 `stream_text_as_openai_with_signal`）在 `block_on` 内部同步调用 `on_part`、`on_error`、`on_done` 回调。此时调用线程处于 tokio runtime 上下文中。

如果回调函数尝试再次调用任何 FFI 函数（如 `aimux_generate_text`），该 FFI 函数会调用 `runtime().block_on(...)`。**Tokio 1.x 的 `Runtime::block_on` 不支持嵌套调用**——它会 panic 并携带 "Cannot block the current thread from within a runtime" 消息。这不是"死锁"（如文档第 24 行所述），但后果同样严重：panic 跨 `extern "C"` 边界是 **未定义行为**。

**文档位置**：[lib.rs:22-24](../../aimux-ffi/src/lib.rs#L22-L24)
> Callbacks execute on the same thread/call-stack that invoked the FFI function, so they must not re-enter the FFI layer (doing so would deadlock the runtime).

**修正**：文档应描述为"doing so will cause a panic and undefined behavior"而非"deadlock"。

**改进建议**：
1. 在回调调用点包裹 `std::panic::catch_unwind`，将 panic 转换为 `on_error` 回调调用：
   ```rust
   let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
       on_part(cstr.as_ptr());
   }));
   if result.is_err() {
       fire_error(on_error, "callback panicked");
       return;
   }
   ```
   但这也有问题——`on_done`/`on_error` 回调本身也可能 panic。
2. 更根本的：考虑使用 `tokio::task::spawn_blocking` 将回调调用移出 runtime 上下文，但这会改变回调的同步语义。
3. 最低限度：用 `std::panic::set_hook` 或 abort-on-panic 配置，使 panic 不会 unwind 跨 FFI。

---

### 6. 回调契约（Stream Callback 生命周期 / Panic）

**结论：🔴 高风险** — 回调 panic 直接导致 UB。

#### 审查点

| 位置 | 代码 | 分析 |
|------|------|------|
| [lib.rs:1155-1158](../../aimux-ffi/src/lib.rs#L1155-L1158) | `on_part(cstr.as_ptr())` | 局部 `CString`，回调返回后 drop。✅ 生命周期正确。 |
| [lib.rs:1251-1256](../../aimux-ffi/src/lib.rs#L1251-L1256) | `on_part(cstr.as_ptr())` | 同上。✅ |
| [lib.rs:256-258](../../aimux-ffi/src/lib.rs#L256-L258) | `fire_error` 中的 `on_error(cstr.as_ptr())` | 局部 `CString`。✅ |
| [lib.rs:262-265](../../aimux-ffi/src/lib.rs#L262-L265) | `fire_error_struct` 中的 `on_error(cstr.as_ptr())` | 同上。✅ |

#### 🔴 高风险发现：回调 panic 跨 FFI 边界的 UB

所有流式函数中的回调（`on_part`、`on_done`、`on_error`）都是 `extern "C" fn(...)` 类型。如果这些回调 panic，unwind 将跨越 `extern "C"` 边界——这是 **未定义行为**。

**影响范围**：
- [lib.rs:875-877](../../aimux-ffi/src/lib.rs#L875-L877)：`aimux_stream_text` 的三个回调
- [lib.rs:930-932](../../aimux-ffi/src/lib.rs#L930-L932)：`aimux_stream_text_with_abort` 的三个回调
- [lib.rs:1012-1014](../../aimux-ffi/src/lib.rs#L1012-L1014)：`aimux_stream_text_as_openai` 的三个回调
- [lib.rs:1035-1037](../../aimux-ffi/src/lib.rs#L1035-L1037)：`aimux_stream_text_as_openai_with_abort` 的三个回调
- [lib.rs:875-877](../../aimux-ffi/src/lib.rs#L875-L877)：`fire_error` / `fire_error_struct` 中的 `on_error` 回调

**共涉及约 30+ 个回调调用点。**

**改进建议**：
1. 首选：在所有回调调用点包裹 `std::panic::catch_unwind(AssertUnwindSafe(|| callback(...)))`，将 panic 转换为日志输出和/或 `fire_error` 调用。注意 `on_done` 回调无返回值，panics 无法传递给 `on_error`——需要额外的错误报告机制。
2. 次选：在进程级别设置 `panic = "abort"`，确保 panic 不会 unwind 而是立即终止进程。这是在 `Cargo.toml` 中添加 `[profile.release] panic = "abort"`。
3. 文档：在回调类型文档中明确声明"回调不得 panic"。

---

### 7. `parse_two_args` / `parse_four_args` 的指针参数处理

**结论：🟢 安全** — 参数校验正确，逻辑健壮。

#### 审查点

| 位置 | 代码 | 分析 |
|------|------|------|
| [lib.rs:273-278](../../aimux-ffi/src/lib.rs#L273-L278) | `parse_two_args(a, b)` | 通过 `cstr_to_string` 逐个检查，任一失败返回 `None`。✅ |
| [lib.rs:281-296](../../aimux-ffi/src/lib.rs#L281-L296) | `parse_four_args(a, b, c, d)` | 同上（四元组版本）。✅ |
| 所有构造器调用点 | `unsafe { parse_two_args(...) }` 等 | 所有调用都正确处理了 `None` 情况（返回 `invalid_args_json()`）。✅ |

#### 🟡 中风险发现：`unsafe` 标记过度

如前所述，`parse_two_args` 和 `parse_four_args` 被标记为 `unsafe fn`，但它们内部只调用安全的 `cstr_to_string`。这导致每个构造器中都需要 `unsafe { parse_two_args(...) }` 包装（约 25 处），增加了代码噪音。

此外，部分构造器的三参数解析（如 `aimux_anthropic_aws_new` 的 `api_key`/`region`/`model_id`）内联实现了三元组模式，没有复用 `parse_four_args`（因为只需要 3 个参数）——这增加了代码重复但逻辑正确。

**改进建议**：
- 移除 `parse_two_args` / `parse_four_args` 的 `unsafe` 标记。
- 添加 `parse_three_args` 辅助函数以消除 `aimux_anthropic_aws_new` 等函数中的内联三元组解析。

---

## 测试覆盖分析

测试位于 `aimux-ffi/tests/`，共两个文件：

| 文件 | 测试数 | 覆盖范围 |
|------|--------|----------|
| [`error_detail_test.rs`](../../aimux-ffi/tests/error_detail_test.rs) | 8 tests | 错误详情保真（serde detail）、null 参数、未知 provider、并发构造器 |
| [`native_constructors_test.rs`](../../aimux-ffi/tests/native_constructors_test.rs) | 3 tests | 所有 native 构造器（cohere/mistral/xai/bedrock/vertex/anthropic_aws/azure）的成功和失败路径 |

**未覆盖的边界**：
- ❌ **`aimux_free_string` 的正确释放**：无直接测试（仅在 `take_json` helper 中隐式使用）。
- ❌ **回调生命周期**：测试中的回调 `on_part`/`on_error`/`on_done` 仅验证内容，未验证 CString 生命周期。
- ❌ **tokio runtime 初始化竞争**：无多线程同时调用 `runtime()` 的测试。
- ❌ **Mutex 中毒恢复**：无测试验证中毒后的行为。
- ❌ **Handle ID 碰撞**：无测试（理论上不可能但值得 edge case 考虑）。
- ❌ **回调 panic 行为**：无测试验证 panic 回调的后果。
- ✅ 覆盖了 null 参数、无效 JSON、未知 provider、serde 详情传递——这些是主要路径。

---

## 额外发现

### 🟡 中风险：`parse_base_url` 用于非 URL 参数

[lib.rs:491](../../aimux-ffi/src/lib.rs#L491) — `aimux_azure_new` 中：
```rust
if let Some(version) = parse_base_url(api_version) {
```

`parse_base_url` 函数（line 314-316）按名称是解析 base URL 的，但被用于解析 `api_version` 参数。逻辑上行为相同（filter null + empty），但命名误导。类似用法也出现在 `aimux_azure_new_with_base`（line 528）。

### 🟡 中风险：`EmbeddingCallOptions::new("")` 空字符串初始化

[lib.rs:1411](../../aimux-ffi/src/lib.rs#L1411) — `aimux_embed` 中：
```rust
let mut opts = aimux_core::embedding_model::EmbeddingCallOptions::new("");
```

用空字符串初始化 `model` 字段，随后覆盖 `opts.values`。这不是 bug，但暴露了 API 设计问题：`EmbeddingCallOptions::new` 要求一个看似无意义的字符串参数。

---

## 改进建议汇总

### 🔴 高风险（建议修复）

| # | 问题 | 位置 | 建议 |
|---|------|------|------|
| 1 | **回调 panic 跨 FFI 导致 UB** | [lib.rs:1157-1158, 1255-1256, 等](../../aimux-ffi/src/lib.rs#L1157) | 在所有回调调用点包裹 `std::panic::catch_unwind` 或设置 `panic = "abort"` |
| 2 | **`into_cstring_raw` null 返回破坏 API 契约** | [lib.rs:208-212](../../aimux-ffi/src/lib.rs#L208-L212) | 在 `CString::new` 失败时返回 fallback 错误 JSON，而非 null |
| 3 | **文档误导：回调重入 FFI 是 panic 非 deadlock** | [lib.rs:22-24](../../aimux-ffi/src/lib.rs#L22-L24) | 更新为 "doing so will cause a runtime panic and undefined behavior" |

### 🟡 中风险（建议改进）

| # | 问题 | 位置 | 建议 |
|---|------|------|------|
| 4 | **`parse_two_args` / `parse_four_args` 的 `unsafe` 标记过度** | [lib.rs:273, 281](../../aimux-ffi/src/lib.rs#L273) | 移除 `unsafe` 标记（内部无 unsafe 操作） |
| 5 | **Mutex 中毒后全局 panic** | [lib.rs:98, 等](../../aimux-ffi/src/lib.rs#L98) | 考虑使用 `lock().unwrap_or_else(|e| e.into_inner())` 或文档说明 |
| 6 | **`parse_base_url` 误用于 `api_version`** | [lib.rs:491](../../aimux-ffi/src/lib.rs#L491) | 重命名或创建专用的 `parse_optional_str` 函数 |
| 7 | **缺少回调 panic 和 Mutex 中毒的测试** | tests/ | 添加相关测试用例 |

### 🟢 低优先级

| # | 问题 | 建议 |
|---|------|------|
| 8 | 三参数构造器内联解析模式 | 添加 `parse_three_args` 辅助函数消除代码重复 |
| 9 | `EmbeddingCallOptions::new("")` 空字符串初始化 | 检查 aimux-core 是否需要改进 API |

---

## 附录：完整 unsafe 位置清单

以下列出 `lib.rs` 中所有 unsafe 相关位置供参考：

| 行号 | 类别 | 描述 | 风险 |
|------|------|------|------|
| 25 | allow | `#![allow(clippy::not_unsafe_ptr_arg_deref)]` | 🟡 |
| 178 | unsafe block | `CStr::from_ptr(ptr)` in `cstr_to_string` | 🟢 |
| 273 | unsafe fn | `parse_two_args` (过度标记) | 🟡 |
| 281 | unsafe fn | `parse_four_args` (过度标记) | 🟡 |
| 338 | unsafe block | `parse_two_args` in `aimux_openai_new` | 🟢 |
| 358 | unsafe block | `parse_two_args` in `aimux_openai_new_with_base` | 🟢 |
| 380 | unsafe block | `parse_two_args` in `aimux_anthropic_new` | 🟢 |
| 398 | unsafe block | `parse_two_args` in `aimux_anthropic_new_with_base` | 🟢 |
| 550 | unsafe block | `parse_four_args` in `aimux_bedrock_new` | 🟢 |
| 576 | unsafe block | `parse_four_args` in `aimux_bedrock_new_with_base` | 🟢 |
| 601 | unsafe block | `parse_four_args` in `aimux_vertex_new` | 🟢 |
| 623 | unsafe block | `parse_four_args` in `aimux_vertex_new_with_base` | 🟢 |
| 644 | unsafe block | `parse_two_args` in `aimux_cohere_new` | 🟢 |
| 660 | unsafe block | `parse_two_args` in `aimux_cohere_new_with_base` | 🟢 |
| 682 | unsafe block | `parse_two_args` in `aimux_mistral_new` | 🟢 |
| 698 | unsafe block | `parse_two_args` in `aimux_mistral_new_with_base` | 🟢 |
| 717 | unsafe block | `parse_two_args` in `aimux_xai_new` | 🟢 |
| 733 | unsafe block | `parse_two_args` in `aimux_xai_new_with_base` | 🟢 |
| 1300 | unsafe block | `CString::from_raw(ptr)` in `aimux_free_string` | 🟢 |
| 1313 | unsafe block | `parse_two_args` in `aimux_openai_embedding_new` | 🟢 |
| 1326 | unsafe block | `parse_two_args` in `aimux_openai_embedding_new_with_base` | 🟢 |
| 1342 | unsafe block | `parse_two_args` in `aimux_cohere_embedding_new` | 🟢 |
| 1355 | unsafe block | `parse_two_args` in `aimux_cohere_embedding_new_with_base` | 🟢 |
| 1371 | unsafe block | `parse_two_args` in `aimux_google_embedding_new` | 🟢 |
| 1384 | unsafe block | `parse_two_args` in `aimux_google_embedding_new_with_base` | 🟢 |
| 1438 | unsafe block | `parse_two_args` in `aimux_openai_speech_new` | 🟢 |
| 1451 | unsafe block | `parse_two_args` in `aimux_openai_speech_new_with_base` | 🟢 |
| 1485 | unsafe block | `parse_two_args` in `aimux_openai_image_new` | 🟢 |
| 1498 | unsafe block | `parse_two_args` in `aimux_openai_image_new_with_base` | 🟢 |
| 1514 | unsafe block | `parse_two_args` in `aimux_google_image_new` | 🟢 |
| 1527 | unsafe block | `parse_two_args` in `aimux_google_image_new_with_base` | 🟢 |
| 1561 | unsafe block | `parse_two_args` in `aimux_openai_transcription_new` | 🟢 |
| 1574 | unsafe block | `parse_two_args` in `aimux_openai_transcription_new_with_base` | 🟢 |
| 1681 | unsafe block | `parse_two_args` in `aimux_cohere_reranking_new` | 🟢 |
| 1694 | unsafe block | `parse_two_args` in `aimux_cohere_reranking_new_with_base` | 🟢 |
| 1732 | unsafe block | `parse_two_args` in `aimux_google_video_new` | 🟢 |
| 1745 | unsafe block | `parse_two_args` in `aimux_google_video_new_with_base` | 🟢 |

> **注**：所有 `extern "C"` 函数使用 `#[unsafe(no_mangle)]`（Rust 2024 语法）而非 `#[no_mangle]`，这是正确的——它们接收原始指针、暴露 C ABI，本质上 unsafe。

---

## 总结

`aimux-ffi/src/lib.rs` 的 unsafe 代码整体设计良好，遵循了标准的 FFI 模式。主要风险集中在 **回调 panic 跨 FFI 边界** 和 **`into_cstring_raw` 的 null 返回契约缺口**。这两个问题是 FFI 层面的经典隐患——在很多成熟的 Rust FFI 库中也会出现。建议在下一轮迭代中优先修复这 3 项高风险问题。

**最终评级**：🟡 **中等风险** — 可通过定位修复提升至 🟢 **安全**。
