# RFC-0012：源码精简方案

> **状态**：DRAFT（待评审）
> **日期**：2026-07-31
> **范围**：`aimux-providers`、`aimux-ffi`，以及 `aimux-providers/src/lib.rs` 的生成机制
> **关联**：[Rust 架构审核报告](../docs/rust-architecture-audit-2026-07-31.md)、[厂商适配层改进](0002-provider-improvements.md)、[Provider 开发规范](0006-provider-development.md)

## 1. 目标

消除架构层面的源码冗余，收敛后续功能膨胀斜率。**不以产物体积为目标**——LTO 已消除未引用代码，产物体积由实际链入的协议引擎决定，与源码行数无关。

核心约束：

1. **统一支持原则不变**——不拆 crate、不引入 feature gate、不减 provider 数量。
2. **测试不动**——现有 125 个测试文件（74,014 行）不修改、不删除、不合并。所有验收以 `cargo test --workspace --no-fail-fast` 全部通过为前提。
3. **公共 API 不变**——对外导出的 `XxxConfig`、`XxxProvider` 类型名和构造方法保持不变，下游代码零感知。

## 2. 当前状态

| 指标 | 数值 |
|---|---:|
| 产品源码 | 68,362 行 / 433 文件 |
| 薄封装 wrapper | 21,469 行 / 293 文件 |
| `lib.rs` 注册语句 | 650 条（325 `pub mod` + 325 `pub use`）/ 737 行 |
| FFI 单文件 | 893 行 / 1 文件 |
| Responses 变体重复 | ~7,400 行 / 7 文件 |
| Anthropic AWS 重复 | ~650 行 / 1 文件 |

## 3. 精简项

### 3.1 薄封装 manifest + macro 生成

**问题**：293 个文件结构同构，真正差异只有 3 个常量（`DEFAULT_BASE_URL`、`ENV_VAR`、`PROVIDER_NAME`）和 profile 选择。保守归一化后 248 个文件、16,965 行落入 11 组结构重复。

**方案**：

1. 在 `aimux-providers/src/openai_compat.rs` 新建一个 declarative macro：

```rust
macro_rules! declare_openai_compat_provider {
    ($name:ident, $display:literal, $base_url:literal, $env_var:literal, $profile:expr) => {
        pub struct ${concat($name, Config)}(OpenAIConfig);

        impl ${concat($name, Config)} {
            pub fn new(api_key: impl Into<String>) -> Self {
                Self(
                    OpenAIConfig::new(api_key)
                        .with_base_url($base_url)
                        .with_provider(stringify!($name))
                        .with_profile($profile),
                )
            }

            pub fn from_env() -> Result<Self, AiMuxError> {
                let key = load_api_key(None, $env_var, $display)?;
                Ok(Self::new(key))
            }

            pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
                self.0 = self.0.with_base_url(url);
                self
            }
        }

        pub struct ${concat($name, Provider)}(OpenAIProvider);

        impl ${concat($name, Provider)} {
            pub fn new(config: ${concat($name, Config)}) -> Self {
                Self(OpenAIProvider::new(config.0))
            }

            pub fn model(&self, model_id: &str) -> OpenAIModel {
                self.0.model(model_id)
            }
        }

        impl Provider for ${concat($name, Provider)} {
            fn name(&self) -> &str { stringify!($name) }
            fn language_model(&self, model_id: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
                Ok(Box::new(self.model(model_id)))
            }
        }
    };
}
```

2. 在 `aimux-providers/src/openai_compat_registry.rs` 用一个声明表调用 macro：

```rust
declare_openai_compat_provider!(ai21, "AI21 Labs", "https://api.ai21.ai/v1", "AI21_API_KEY", OpenAICompatProfile::full());
declare_openai_compat_provider!(groq, "Groq", "https://api.groq.com/openai/v1", "GROQ_API_KEY", OpenAICompatProfile::groq());
declare_openai_compat_provider!(deepseek, "DeepSeek", "https://api.deepseek.com/v1", "DEEPSEEK_API_KEY", OpenAICompatProfile::deepseek());
// ... 293 行，每行一个 provider
```

3. `lib.rs` 改为：

```rust
mod openai_compat_registry;
pub use openai_compat_registry::*;  // 一行替代 518 行 pub use
```

**保持公共 API**：`Ai21Config`、`Ai21Provider`、`GroqConfig`、`GroqProvider` 等类型名和构造方法不变。

**预期结果**：

| | 优化前 | 优化后 |
|---|---:|---:|
| 文件数 | 293 | 4（macro + registry + 2 个保留文件） |
| 行数 | 21,469 | ~1,330（macro ~50 + registry ~1,250 + 2 个保留文件 ~30） |
| 净减 | | **-20,139 行 / -289 文件** |

**不适用此方案的 provider**：实测只有 2 个薄封装有额外方法——`openrouter.rs` 和 `huggingface.rs`（都有 `responses_model`）。这两个保留独立文件，不纳入 macro 生成。其余 281 个纯 `model()` 薄封装全部由 macro 生成。

### 3.2 `lib.rs` 根注册自动生成

**问题**：737 行中有 650 条机械 `pub mod` + `pub use`，每个 provider 要改两处。

**方案**：

- 薄封装部分：`pub use openai_compat_registry::*;` 一行替代 518 行。
- 原生协议和模态专用 provider：保留手写 `pub mod` + `pub use`，因为数量有限（~30 个）且各自有独立的导出类型。
- 新增兼容厂商时只改 `openai_compat_registry.rs` 一处。

**预期结果**：

| | 优化前 | 优化后 |
|---|---:|---:|
| `lib.rs` 行数 | 737 | ~80 |
| 新增 provider 改动点 | 2 处（mod + use） | 0（registry 内一行） |

### 3.3 FFI 重复模式提取

**问题**：[`aimux-ffi/src/lib.rs`](../aimux-ffi/src/lib.rs) 893 行，有 20 处重复的 `cstr_to_string` 双参数解包和 10 处重复的 `block_on → serialize → CString` 模式。

**方案**：

1. 提取通用 helper：

```rust
/// 从两个 C 字符串构造 (key, model_id)，失败返回 None。
///
/// # Safety
///
/// 调用者必须确保 `a` 和 `b` 要么是 null，要么指向有效的以 NUL 结尾的 C 字符串。
/// 函数内部通过 `CStr::from_ptr` 安全地读取字符串，但指针有效性由调用者保证。
unsafe fn parse_two_args(a: *const c_char, b: *const c_char) -> Option<(String, String)> {
    match (cstr_to_string(a), cstr_to_string(b)) {
        (Some(k), Some(m)) => Some((k, m)),
        _ => None,
    }
}

/// 执行一个 async 操作并返回 JSON 字符串（caller 必须 free）。
fn run_and_serialize<F, T>(model_msg: &str, f: F) -> *mut c_char
where
    F: std::future::Future<Output = Result<T, AiMuxError>>,
    T: serde::Serialize,
{
    let result = runtime().block_on(f);
    match result {
        Ok(r) => serde_json::to_string(&r)
            .map(into_cstring_raw)
            .unwrap_or_else(|e| error_json_raw(format!("serialize: {e}"))),
        Err(e) => error_json_raw(format!("{}: {e}", model_msg)),
    }
}

/// 解析 base_url 参数，空字符串视为未设置。
fn parse_base_url(base_url: *const c_char) -> Option<String> {
    cstr_to_string(base_url).filter(|url| !url.is_empty())
}
```

2. 每个 `extern "C"` 函数缩减为 2-4 行调用。

**不改变 ABI**：`#[unsafe(no_mangle)] pub extern "C" fn` 签名完全不变。

**预期结果**：

| | 优化前 | 优化后 |
|---|---:|---:|
| `aimux-ffi/src/lib.rs` 行数 | 893 | ~450 |
| 净减 | | **-443 行** |

### 3.4 Anthropic AWS 合并

**问题**：[`aimux-providers/src/anthropic_aws/model.rs`](../aimux-providers/src/anthropic_aws/model.rs)（650 行）的流式循环与 [`aimux-providers/src/anthropic/model.rs`](../aimux-providers/src/anthropic/model.rs) 几乎逐字重复。唯一差异是 SigV4 鉴权和 `HttpBody::Bytes` 发送方式。

**方案**：

1. 在 `anthropic/model.rs` 提取一个 `anthropic_stream_reducer` 函数，接收一个 `Fn(&[u8], &str, &str) -> Vec<(String, String)>` 做 header 构建（标准路径返回 Bearer header，AWS 路径返回 SigV4 签名 header）。
2. `anthropic_aws/model.rs` 调用同一个 reducer，只覆盖 `build_headers` 和 body 编码。

**预期结果**：

| | 优化前 | 优化后 |
|---|---:|---:|
| `anthropic_aws/model.rs` | 650 行 | ~200 行 |
| 净减 | | **-450 行** |

### 3.5 Responses API 变体合并

**问题**：7 个文件各自实现 Responses API 转换，结构高度相似但有厂商差异：

| 文件 | 行数 |
|---|---:|
| `open_responses.rs` | 1,290 |
| `huggingface/responses.rs` | 1,196 |
| `azure/responses.rs` | 1,106 |
| `openai/responses/mod.rs` | 969 |
| `openai/responses/convert.rs` | 1,088 |
| `xai/responses/mod.rs` | 954 |
| `xai/responses/convert.rs` | 819 |
| **合计** | **7,422** |

**方案**：

1. 在 `aimux-providers/src/openai/responses/` 下提取共享的 `responses_convert.rs`，包含请求体构建、流式事件解析和 usage 提取的通用逻辑。
2. 各厂商只保留差异覆盖：endpoint 拼接、model id 映射、provider-specific 字段。
3. 不强行合并到单一函数——各厂商的 responses 实现有真实协议差异，只提取共享框架。

**预期结果**：

| | 优化前 | 优化后 |
|---|---:|---:|
| Responses 变体 | ~7,400 行 / 7 文件 | ~4,000 行 / 4 文件 |
| 净减 | | **-3,400 行 / -3 文件**（估算值，实施前需先做逐行相似度审计确认） |

## 4. 不做的事

| 不做 | 原因 |
|---|---|
| 拆分 `aimux-providers` crate | 统一支持原则 |
| 引入 Cargo feature gate | 统一支持原则 |
| 修改任何测试文件 | 明确约束 |
| 合并原生协议引擎（openai/anthropic/google/bedrock 等） | 各家协议差异是真实复杂度，不是冗余 |
| 合并模态专用 provider（TTS/STT/image/video） | 各家 API 差异大，无法共享 |
| 修改公共 API 类型名或构造方法 | 下游零感知 |
| 追求产物体积优化 | 本方案不以体积为目标 |

## 5. 验收标准

### 5.1 功能验收

- [ ] `cargo test --workspace --no-fail-fast` 全部通过，0 failures。
- [ ] `tests/` 目录下测试文件数不变（125 个），测试行数不变（74,014 行）。
- [ ] 允许在 `src/` 文件内新增 inline `#[cfg(test)]` 断言（如验证 macro 生成的类型存在），不计入上述 125 个文件。
- [ ] 所有 `XxxConfig`、`XxxProvider` 类型仍可从 `aimux_providers` 导入，构造方法签名不变。
- [ ] 所有 `#[unsafe(no_mangle)] pub extern "C" fn aimux_*` 符号仍存在，签名不变。

### 5.2 规模验收

- [ ] `aimux-providers/src/` 下 `.rs` 文件数从 388 降到 ~100。
- [ ] `aimux-providers/src/lib.rs` 行数从 737 降到 ~80。
- [ ] `aimux-ffi/src/lib.rs` 行数从 893 降到 ~450。
- [ ] 符合完整薄封装骨架的文件数从 293 降到 0（全部由 macro 生成）。

### 5.3 质量验收

- [ ] `cargo check --workspace --all-targets` 0 errors。
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` 0 errors。
- [ ] `cargo fmt --all -- --check` 0 diffs。

### 5.4 收敛验收

- [ ] 新增一个 OpenAI 兼容厂商只需在 `openai_compat_registry.rs` 加 1 行，不新建文件、不改 `lib.rs`。
- [ ] `aimux-providers/src/` 下不再出现新的单文件薄封装 wrapper。

## 6. 实施顺序

```text
3.1 薄封装 manifest + macro  →  3.2 lib.rs 自动生成  →  3.3 FFI helper 提取  →  3.4 Anthropic AWS 合并  →  3.5 Responses 合并
```

每步独立可验证。3.1 和 3.2 必须连续完成（3.2 依赖 3.1 的 registry）。3.3、3.4、3.5 互相独立，可并行。

## 7. 预期结果

| 指标 | 当前 | 目标 | 净减 |
|---|---:|---:|---:|
| 产品源码行数 | 68,362 | ~43,243 | -25,119（-37%） |
| 产品源码文件数 | 433 | ~140 | -293 |
| `lib.rs` 行数 | 737 | ~80 | -657 |
| FFI 行数 | 893 | ~450 | -443 |
| 新增兼容厂商成本 | 1 文件 / ~65 行 | 1 行 manifest | -99.5% |
| 测试行数 | 74,014 | 74,014（不动） | 0 |

## 8. 风险

| 风险 | 缓解 |
|---|---|
| macro 生成的类型名与手写不一致 | 用编译测试断言每个导出类型存在 |
| `${concat}` 宏在旧 Rust 版本不可用 | 项目 MSRV 1.85，支持 `${concat}`；CI 固定 stable |
| 个别薄封装有隐藏差异（如额外方法） | macro 不适用的 provider 保留独立文件 |
| Anthropic AWS 合并后行为漂移 | cassette 回放测试覆盖流式行为 |
| Responses 合并后丢厂商差异 | 逐厂商保留差异覆盖 + 各自测试 |

## 9. 变更记录

| 日期 | 说明 |
|---|---|
| 2026-07-31 | 初版，基于架构审核报告的冗余数据制定 |
