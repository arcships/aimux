# P2 异步正确性巡检报告

**项目**: aimux  
**日期**: 2026-08-06  
**审计范围**: Send/Sync 正确性、并发安全、async 模式规范性  
**判定结论**: 🟢 整体通过（无高危缺陷；有若干中低风险注意事项）

---

## 概述

aimux 是基于 tokio 的重度异步项目（325 LLM provider + SSE/NDJSON 流）。本次审计覆盖 6 个维度，逐项审查源码。总体代码质量较高：

- 所有核心 trait 均声明 `Send + Sync`
- 无 `std::thread::sleep` 阻塞 runtime 的情况
- 流式原语使用 `pin_project_lite!` 手写 `Stream` impl，无自引用风险
- AbortSignal 使用 `CancellationToken`（Notify-based），跨线程安全且无需 polling
- 共享 HTTP Client 使用 `OnceLock`，初始化无竞态
- FFI 层的 re-entrancy 死锁风险有文档明确警告

以下逐项详细展开。

---

## 1. Box<dyn LanguageModel> — trait Send/Sync 与 object safety

🟢 **通过**

### 发现

**`LanguageModel` trait** (`aimux-core/src/language_model.rs:25`):
```rust
#[async_trait]
pub trait LanguageModel: Send + Sync {
    // ...
    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError>;
    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError>;
}
```

- ✅ 显式声明 `Send + Sync` 作为 supertrait。
- ✅ 使用 `#[async_trait]` 宏，当 trait 有 `Send + Sync` bound 时，宏生成的 `Box<dyn Future>` 自动加上 `Send` bound，确保跨 await 点的 Future 是 `Send` 的。
- ✅ Object safety 满足：所有方法接收 `&self`（非 `Self: Sized`），无泛型方法。可以作为 `dyn LanguageModel` 使用。

**`Provider` trait** (`aimux-core/src/provider.rs:9`):
```rust
pub trait Provider: Send + Sync {
    fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError>;
}
```

- ✅ 同样声明 `Send + Sync`。
- ✅ 返回 `Box<dyn LanguageModel>`，由于 `LanguageModel: Send + Sync`，trait object 自动为 `Send + Sync`。

**所有 8 个模型 trait** 均检查通过：
| Trait | 文件 | Send + Sync |
|-------|------|------------|
| `LanguageModel` | `aimux-core/src/language_model.rs:25` | ✅ |
| `EmbeddingModel` | `aimux-core/src/embedding_model.rs` | ✅ |
| `Files` | `aimux-core/src/files_model.rs` | ✅ |
| `VideoModel` | `aimux-core/src/video_model.rs` | ✅ |
| `ImageModel` | `aimux-core/src/image_model.rs` | ✅ |
| `SpeechModel` | `aimux-core/src/speech_model.rs` | ✅ |
| `RerankingModel` | `aimux-core/src/reranking_model.rs` | ✅ |
| `SearchModel` | `aimux-core/src/search_model.rs` | ✅ |

**FFI 层** (`aimux-ffi/src/lib.rs:69-79`) 使用 `Arc<dyn LanguageModel>` 存储句柄：
```rust
enum ModelHandle {
    Language(Arc<dyn LanguageModel>),
    // ...
}
```
- ✅ `Arc<dyn LanguageModel>: Send + Sync`，可安全在线程间共享。

### 风险

无高危缺陷。唯一注意事项：
- `#[async_trait]` 默认情况下，如果 trait 不声明 `Send`，生成的 Future 就是不 `Send` 的。项目中所有 trait 都已正确声明，但未来新增 model trait 时必须保持此约定。

---

## 2. async stream 生命周期 — aimux-stream

🟢 **通过**

### 发现

**aimux-stream 使用 `pin_project_lite!` + 手写 `Stream` impl，而非 `async-stream` 宏。**

#### SSE Stream (`aimux-stream/src/sse.rs:36-53`)
```rust
pin_project! {
    pub struct SseStream<S, E> {
        #[pin]
        inner: S,
        buffer: Vec<u8>,
        max_event_size: usize,
        done: bool,
        _err: std::marker::PhantomData<E>,
    }
}
```
- ✅ 使用 `pin_project_lite!` 宏，被 `#[pin]` 标记的字段 `inner` 在 `Pin<&mut Self>` 下安全投影。
- ✅ 无 async 块/闭包自引用：`poll_next` 是手动实现（`aimux-stream/src/sse.rs:86-151`），逐个 poll 内层 stream，不涉及生成器状态机跨 `await` 自引用。
- ✅ `buffer: Vec<u8>` 是 heap-allocated，即使 struct 被 move，buffer 指针仍有效。
- ✅ `poll_next` 中 `self.as_mut().get_mut()` 获取 `&mut Self`，然后 `Pin::new(&mut this.inner)` 重新 pin projection——正确模式。

#### NDJSON Stream (`aimux-stream/src/ndjson.rs:26-42`)
结构与 SSE 相同，同样通过审查。

#### TimeoutBodyStream (`aimux-provider-utils/src/http.rs:501-513`)
```rust
struct TimeoutBodyStream {
    inner: BoxStream<'static, Result<Bytes, AiMuxError>>,
    // ...
    sleep: Option<Pin<Box<tokio::time::Sleep>>>,
    abort_wait: Option<Pin<Box<dyn Future<Output = ()> + Send>>>,
    done: bool,
}
```
- ✅ `sleep` 和 `abort_wait` 在 heap 上 `Box::pin`，Move 安全。
- ✅ `sleep` 的 deadline 重置逻辑（`:622-629`）通过 `Box::pin` 重新创建，旧 `Sleep` 被 drop 时自动从 timer wheel 注销。
- ✅ `abort_wait: Pin<Box<dyn Future<Output = ()> + Send>>` 显式标注 `Send`。

#### StreamingToolCallTracker (`aimux-stream/src/streaming_tool_call_tracker.rs`)
纯同步数据结构（`Vec` + `String`），无 async 代码。不涉及 stream 生命周期问题。

### 风险

无高危缺陷。`BoxStream<'static>` 的 `'static` 生命周期意味着 provider 不能借用局部数据传入流——这实际上是正确的设计约束。

---

## 3. tokio runtime in FFI — block_on 与重入死锁

🟡 **中风险 — 文档已警告，但无运行时防护**

### 发现

**Runtime 初始化** (`aimux-ffi/src/lib.rs:153-159`):
```rust
fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Runtime::new()
            .expect("aimux-ffi: failed to build tokio runtime")
    })
}
```

- ✅ `Runtime::new()` 创建 **multi-threaded** runtime（默认），支持多线程并发。
- ✅ `OnceLock<Runtime>` 保证全局单例初始化，线程安全。
- ✅ Multi-threaded runtime 的 `block_on` 可以从任意线程调用，每次调用创建独立 context。

**`block_on` 使用模式** (`aimux-ffi/src/lib.rs:304, 1122, 1218`):
```rust
fn run_and_serialize<F, T>(_model_msg: &str, f: F) -> *mut c_char
where
    F: std::future::Future<Output = Result<T, AiMuxError>>,
    T: serde::Serialize,
{
    let result = runtime().block_on(f);  // line 304
    // ...
}
```

**重入死锁风险** (`aimux-ffi/src/lib.rs:20-24`):
```rust
//! Callbacks execute on the same thread/call-stack that
//! invoked the FFI function, so they must not re-enter the FFI layer (doing so
//! would deadlock the runtime).
```

- ✅ 文档明确告知回调不能重入 FFI。
- ⚠️ 但是**无运行时检测**：如果 Swift/Kotlin 调用方在 `on_part` 回调中再次调用 FFI 函数，会静默死锁。`block_on` 在已有 runtime context 的线程上再次调用会 panic（当前线程已在 runtime 中），但这里的 `block_on` 是在 C 调用线程上执行的——该线程**不在** runtime worker 线程池中，所以不会触发 tokio 的 "cannot block the current thread from within a runtime" panic。真正的问题在于：如果 `block_on` 内部的任务试图提交工作到 runtime，而 runtime 的所有 worker 线程都在忙，可能导致死锁。

**并发安全**：
- ✅ Multi-threaded runtime：多个 C 线程同时调用 FFI 函数 → 每个 `block_on` 独立等待 → runtime 内部线程池调度。
- ✅ Registry 用 `Mutex<HashMap<u64, ModelHandle>>` 保护，线程安全。

### 风险评估

| 场景 | 结果 |
|------|------|
| 多 C 线程并发调 FFI 函数 | ✅ 安全（multi-threaded runtime） |
| 回调中重入 FFI | ❌ 死锁（已文档警告，无法运行时阻止） |
| block_on 内部 spawn 大量任务 | ✅ runtime 线程池处理 |
| block_on 后持有锁跨 await | ⚠️ 取决于 provider 实现（见第 6 节） |

### 建议

- 考虑在回调入口设置 `thread_local!` 标志位，检测重入并立即返回错误（如 `AiMuxError::Aborted`），而非死锁。
- 或者改用 `Runtime::spawn_blocking` 模式：在 C 线程上不 `block_on`，而是通过 channel 把任务提交到 runtime 的专用 worker，然后用 `oneshot` 接收结果。

---

## 4. shared_client 连接池 — 全局 OnceLock<Arc<reqwest::Client>>

🟢 **通过**

### 发现

**连接池定义** (`aimux-provider-utils/src/http.rs:95-126`):
```rust
static SHARED: OnceLock<Client> = OnceLock::new();
static SHARED_STREAMING: OnceLock<Client> = OnceLock::new();

pub fn shared_client() -> &'static Client {
    SHARED.get_or_init(|| build_client(PoolConfig::default(), TimeoutConfig::default()))
}

pub fn shared_streaming_client() -> &'static Client {
    SHARED_STREAMING.get_or_init(|| build_client(PoolConfig::default(), TimeoutConfig::streaming()))
}
```

- ✅ `OnceLock::get_or_init` 保证单一初始化（std 库保证 happens-before），无竞态条件。
- ✅ 非流式 Client 带 30s 整体超时 (`TimeoutConfig::default()` `response_timeout_ms: 30_000`)，流式 Client 禁用整体超时 (`response_timeout_ms: 0`)——正确分流。
- ✅ 连接池参数合理：`max_idle_per_host: 10`, `idle_timeout_secs: 30`, TCP keepalive 20s。
- ✅ 返回 `&'static Client` 共享引用——不 clone、不持有所有权。

**跨 provider 共享**：
- ✅ `reqwest::Client` 内部使用 `Arc` 包裹连接池，跨 provider 共享是安全且推荐的。
- ✅ provider 不持有 Client 引用（每次调 `shared_client()` 获取 `&'static` 引用即用即弃）。
- ✅ 请求重建时（`build_request_builder` at `:868-895`），从纯数据 `HttpRequest` 重新构造 `RequestBuilder`，不依赖 Client 状态。
- ✅ 测试验证两个 shared_client 调用返回同一指针 (`http.rs:968-972`)。

### 风险

无高危缺陷。

⚠️ 小注意事项：所有 325 provider 共享同一连接池，某个 provider 的慢/死连接可能占用池中槽位。但默认 `max_idle_per_host: 10` 是按 host 维度的，而非全局池——所以一个 provider host 的问题不会影响其他 host。**无需修复**。

---

## 5. AbortSignal 跨线程 — 原子标志 vs channel

🟢 **通过**

### 发现

**AbortSignal 实现** (`aimux-core/src/shared.rs:83-129`):
```rust
pub struct AbortSignal {
    token: CancellationToken,  // from tokio_util::sync::CancellationToken
}
```

- ✅ 后端是 `tokio_util::sync::CancellationToken`（内部是 `tokio::sync::Notify` + AtomicBool）。
- ✅ `is_aborted()` — 同步、lock-free 检测（`AtomicBool` 读取）。
- ✅ `cancelled()` — 返回 `Future`，abort 时立即通过 Notify 唤醒（event-driven，无需 polling）。
- ✅ `Clone` 实现正确：clone 的 `AbortSignal` 共享同一个 `CancellationToken`，任意一个 clone 上调用 `abort()` 所有 clone 都可见。
- ✅ 类型标注 `Send + Sync`，文档明确("This type is `Send + Sync` and cheap to clone")。

**跨线程传递路径**：
1. FFI 层：`aimux_create_abort_signal()` → `intern_handle(ModelHandle::Abort(signal))` → 存入 `Mutex<HashMap>` 注册表
2. FFI 层：`aimux_generate_text()` → `get_abort_signal(handle)` → `CallOptions.abort_signal`
3. provider-utils HTTP 层：`HttpRequest.abort_signal: Option<AbortSignal>` → `send_request()` 中 `tokio::select!` 监听
4. TimeoutBodyStream：`abort_signal` 字段 + `abort_wait` 懒创建

**HTTP 层的 abort 集成**（`aimux-provider-utils/src/http.rs`）：
- `send_request` (`:842-852`)：`tokio::select! { biased; _ = signal.cancelled() => ..., result = response => ... }`
- `send` body read (`:251-258`)：同样 `tokio::select!`
- `send_with_retry_raw` backoff (`:802-811`)：`tokio::select!` 中断 retry backoff
- `TimeoutBodyStream::poll_next` (`:578-584`, `:588-592`, `:633-639`)：fast path `is_aborted()` + event-driven `abort_wait`

全部使用 `biased` 模式：abort 优先级高于正常完成，确保取消信号不被忽略。

### 风险

无高危缺陷。

---

## 6. retry 逻辑的 async — sleep 的正确性

🟢 **通过**

### 发现

**`retry_with_exponential_backoff`** (`aimux-provider-utils/src/retry.rs:26-54`):
```rust
tokio::time::sleep(delay).await;  // line 47
```

**`retry_with_exponential_backoff_respecting_retry_headers`** (`retry.rs:86-121`):
```rust
tokio::time::sleep(Duration::from_millis(delay_ms.max(0) as u64)).await;  // line 112
```

**HTTP 层 retry backoff** (`http.rs:802-811`):
```rust
match &request.abort_signal {
    Some(signal) => {
        tokio::select! {
            biased;
            _ = signal.cancelled() => return Err(AiMuxError::Aborted),
            _ = tokio::time::sleep(delay) => {}
        }
    }
    None => tokio::time::sleep(delay).await,
}
```

- ✅ 全部使用 `tokio::time::sleep`，不阻塞 runtime worker 线程。
- ✅ 全仓库 `std::thread::sleep` 搜索：**0 处匹配**。
- ✅ `tokio::time::sleep` 搜索：26 处使用，全部正确（retry、polling、测试）。
- ✅ HTTP 层 retry backoff 在 abort 信号存在时使用 `tokio::select!` 确保 abort 能中断等待。
- ✅ Full Jitter（`delay ∈ [0, base)` + `gen_range(0..base)`）防止 429 惊群。
- ✅ `base <= 0` 保护：`gen_range(0..0)` 会 panic，代码提前返回 0 (`retry.rs:152-154`)。

**Retry trait bound** (`retry.rs:31-34`):
```rust
where
    F: FnMut()
        -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, AiMuxError>> + Send>>,
    T: Send,
```
- ✅ 要求闭包返回的 Future 为 `Send`，确保可以在多线程 runtime 中跨 `.await` 点安全传递。
- ✅ `T: Send` 确保返回值可跨线程。

### 风险

无高危缺陷。

⚠️ 小注意事项：`retry_with_exponential_backoff` 内部有 `for attempt in 0..=max_retries` 循环，每次重试 `await` 都会让出 runtime。但如果 `max_retries` 被设为极大值（如 `u32::MAX`），且每个错误都是可重试的，可能产生长时间运行的 task。建议对 `max_retries` 添加上限校验。

---

## 补充审查: 其他并发模式审查

### 全局 Registry (`aimux-ffi/src/lib.rs:84`)
```rust
static REGISTRY: OnceLock<Mutex<ModelRegistry>> = OnceLock::new();
```
- ✅ `Mutex<HashMap<u64, ModelHandle>>` 保护 handle 注册表。
- ✅ `get_model` / `get_handle` 在锁内 clone `Arc`，锁临界区极短。
- ⚠️ `intern_model` 和 `drop_handle` 也是锁内操作——极短，但若并发调用量大（325 provider 同时初始化），可能短暂排队。当前无性能数据支撑优化需求。

### `Box<dyn Fn>` in StreamingToolCallTracker (`aimux-stream/src/streaming_tool_call_tracker.rs:143-147`)
```rust
type ExtractMetadataFn<M> = Box<dyn Fn(&StreamingToolCallDelta) -> Option<M>>;
type BuildMetadataFn<M> = Box<dyn Fn(Option<&M>) -> Option<M>>;
```
- 这两个 `Box<dyn Fn>` **不标注** `Send + Sync`。`StreamingToolCallTracker` 在 `flush()` 时 `&mut self` 调用它们，不跨线程共享——但 `Box<dyn Fn>` 不是 `Send`，如果未来将 tracker 放入 spawn 的 task，编译器会报错。当前用法正确，无风险。

### `Tokio sync` 原语使用
- `CancellationToken`（`tokio_util::sync`）— 正确的 abort 原语，Notify + AtomicBool 后端
- 未发现 `std::sync::RwLock` 或 `std::sync::Condvar` 在 async 上下文中的误用
- 未发现 `MutexGuard` 跨 `.await` 持有（Rust 编译器会阻止，但检查了所有显式 `lock().unwrap()` 后是否有 `.await` 点——无此模式，因为所有锁临界区都非常短且同步）

---

## 总结

| 维度 | 判定 | 说明 |
|------|------|------|
| 1. `Box<dyn LanguageModel>` Send/Sync | 🟢 | 所有 8 个 model trait 正确声明 Send + Sync；async_trait 正确派生 Send future |
| 2. async stream 生命周期 | 🟢 | pin_project_lite! + 手写 Stream，无自引用风险；TimeoutBodyStream 使用 Box::pin |
| 3. tokio runtime in FFI | 🟡 | 功能正确，但重入死锁无运行时防护（已有文档警告） |
| 4. shared_client 连接池 | 🟢 | OnceLock 无竞态；按 host 维度的连接池隔离正确 |
| 5. AbortSignal 跨线程 | 🟢 | CancellationToken（Notify + AtomicBool），event-driven，biased select 优先 abort |
| 6. retry async sleep | 🟢 | 全用 tokio::time::sleep；无 std::thread::sleep；Full Jitter 正确 |

**整体结论**: 🟢 通过。异步代码质量高，无明显 Send/Sync 缺失或并发竞态。唯一的 🟡 项（FFI 重入死锁）已通过文档明确警告，建议后续版本添加运行时检测。
