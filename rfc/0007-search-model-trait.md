# RFC-0007：Search Model Trait 设计

> **状态**：ACCEPTED
> **日期**：2026-07-28
> **范围**：`aimux-core` 新增 `SearchModel` trait 及配套类型
> **关联**：[Provider 开发规范](0006-provider-development.md)、[Provider 调研报告](../docs/provider-research/README.md)

## 1. 动机

调研报告显示 11 个 search provider（tavily/serper/exa_ai/firecrawl/linkup/parallel_ai/searxng/google_pse/tinyfish/you_com/dataforseo）因 aimux-core 无 search trait 而搁置。这些 provider 的协议证据均为强/中，是最大的单类阻塞项。

Vercel AI SDK 参考实现中不存在 search model trait（仅有 language/embedding/rerank/speech/transcription/image/video/files/realtime 九种）。因此本 RFC 需要从零设计 search trait 的接口契约。

## 2. 设计目标

1. 覆盖 11 个 search provider的协议共性，不因个别 provider 的特有字段污染核心接口。
2. 与现有 trait（RerankingModel、EmbeddingModel）风格一致：provider-facing `do_*` 方法 + `SearchCallOptions` + `SearchResult`。
3. 不修改 `Provider` trait 的现有契约（`name` + `language_model`），search 能力通过 provider 上的额外方法暴露。
4. 不引入用户 API 层（`generate_text` 级别的 `search` free function），仅定义 provider-facing trait。用户 API 可后续独立提案。

## 3. trait 设计

### 3.1 SearchModel trait

```rust
/// The unified search model trait (provider-facing).
///
/// Aligned with the pattern of `RerankingModel` / `EmbeddingModel`:
/// providers implement `do_search`; users never call it directly.
#[async_trait]
pub trait SearchModel: Send + Sync {
    /// Specification version (always `"v4"`).
    fn specification_version(&self) -> &'static str {
        "v4"
    }

    /// Provider name, e.g. `"tavily"`.
    fn provider(&self) -> &str;

    /// Provider-specific model ID (some providers use fixed IDs like
    /// `"tavily-search"`; others accept endpoint-specific names).
    fn model_id(&self) -> &str;

    /// Execute a search query and return results.
    async fn do_search(
        &self,
        options: &SearchCallOptions,
    ) -> Result<SearchResult, AiMuxError>;
}
```

### 3.2 SearchCallOptions

```rust
/// Options passed to [`SearchModel::do_search`].
#[derive(Debug, Clone)]
pub struct SearchCallOptions {
    /// The search query string.
    pub query: String,

    /// Maximum number of results to return.
    pub max_results: Option<u32>,

    /// Whether to include raw page content in results.
    /// Provider support varies; providers that cannot honor this should
    /// issue a warning rather than erroring.
    pub include_raw_content: Option<bool>,

    /// Optional time range filter (e.g. `"day"`, `"week"`, `"month"`, `"year"`).
    /// Provider support varies.
    pub time_range: Option<String>,

    /// Optional list of domains to include in results.
    pub include_domains: Option<Vec<String>>,

    /// Optional list of domains to exclude from results.
    pub exclude_domains: Option<Vec<String>>,

    /// Abort signal for cancelling the operation.
    pub abort_signal: Option<AbortSignal>,

    /// Additional provider-specific options, keyed by provider name.
    pub provider_options: Option<SharedProviderOptions>,

    /// Additional HTTP headers to send with the request.
    pub headers: Option<SharedHeaders>,
}

impl SearchCallOptions {
    /// Create options with a query and all else unset.
    pub fn new(query: impl Into<String>) -> Self { ... }
}
```

### 3.3 SearchResult

```rust
/// A single search result item.
#[derive(Debug, Clone)]
pub struct SearchResultItem {
    /// The title of the result (e.g. page title).
    pub title: Option<String>,

    /// The URL of the result.
    pub url: Option<String>,

    /// A snippet/summary of the result content.
    pub content: Option<String>,

    /// Raw page content (when `include_raw_content` is requested and
    /// supported by the provider).
    pub raw_content: Option<String>,

    /// A relevance score (0.0–1.0) if the provider returns one.
    pub score: Option<f64>,

    /// Provider-specific metadata for this result.
    pub provider_metadata: Option<SharedProviderMetadata>,
}

/// The result of [`SearchModel::do_search`].
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// Ordered list of search results.
    pub results: Vec<SearchResultItem>,

    /// An optional direct answer / summary (some providers return
    /// an AI-generated answer alongside results).
    pub answer: Option<String>,

    /// Additional provider-specific metadata.
    pub provider_metadata: Option<SharedProviderMetadata>,

    /// Warnings for the call.
    pub warnings: Vec<Warning>,

    /// Optional response information for debugging.
    pub response: Option<SearchResponse>,
}

/// Optional response information for a search call.
#[derive(Debug, Clone, Default)]
pub struct SearchResponse {
    /// Response headers.
    pub headers: Option<SharedHeaders>,
    /// The response body (opaque JSON).
    pub body: Option<serde_json::Value>,
}
```

## 4. Provider 集成方式

### 4.1 不修改 Provider trait

`Provider` trait 保持不变（`name` + `language_model`）。纯 search provider 不适合通过 `language_model` 返回值——与 image/video/speech/rerank provider 的模式一致，search 能力通过 provider 上的额外方法暴露：

```rust
// 在 provider 实现中（如 TavilyProvider）
impl TavilyProvider {
    pub fn search_model(&self, model_id: &str) -> TavilySearchModel { ... }
}

// Provider trait impl 仍需实现（language_model 返回 Unsupported error）
impl Provider for TavilyProvider {
    fn name(&self) -> &str { "tavily" }
    fn language_model(&self, _: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Err(AiMuxError::Unsupported("tavily does not support language models".into()))
    }
}
```

### 4.2 不引入用户 API

本 RFC 不定义 `search()` free function（类似 `generate_text()`）。原因：
- 现有 `generate_text` / `stream_text` 围绕 `LanguageModel` 设计，search 的输入/输出模型不同。
- 用户 API 设计需考虑与 `generate_text` 的组合（如 "搜索后生成"），属于更高层抽象。
- 先稳定 provider-facing trait，用户 API 可后续独立提案。

## 5. 各 provider 映射

| Provider | endpoint | query 字段 | results 字段 | 特有行为 |
|---|---|---|---|---|
| tavily | POST /search | query | results[].{title,url,content,score} | answer 字段 |
| serper | POST /search | q | organic[].{title,link,snippet} | 无 score |
| exa_ai | POST /search | query | results[].{title,url,text} | 无 score |
| firecrawl | POST /v2/search | query | data[].{url,markdown} | 搜索+抓取合一 |
| linkup | POST /v1/search | query | results[].{url,title,content} | depth/outputType |
| parallel_ai | POST /v1/search | objective | results[] | objective 模式 |
| searxng | GET /search | q | results[].{title,url,content} | 自托管、无鉴权 |
| google_pse | GET /v1 | q | items[].{title,link,snippet} | 需 cx 参数 |
| tinyfish | POST /search | query | results[] | x-api-key |
| you_com | POST /search | query | hits[].{url,title,description} | ydc-index.io |
| dataforseo | POST /v3/serp/... | keyword | tasks[].result.organic[] | Basic 鉴权 |

**共性**：所有 provider 都有 query → results[] 的基本结构。title/url/content/snippet 是通用字段。score、answer、raw_content 是可选字段。

## 6. 与现有 trait 的关系

- **与 RerankingModel 的相似性**：都是 query → ordered results 的模式。但 reranking 的输入是已有文档列表（rerank），search 的输入是 query（从互联网/索引检索）。
- **与 LanguageModel 的关系**：某些 search provider（如 Perplexity）将搜索集成在 language model 中（通过 provider-executed tool `web_search`）。本 trait 不替代这种模式，而是为纯 search API 提供独立接口。
- **GenerateContent::Source**：现有 `GenerateContent::Source` 变体已用于 language model 的搜索引用结果。`SearchResultItem` 与 `Source` 结构相似但用途不同——`Source` 是 language model 输出中的引用，`SearchResultItem` 是 search API 的直接输出。

## 7. 不做的事

1. 不修改 `Provider` trait。
2. 不引入用户 API（`search()` free function）。
3. 不定义流式搜索——所有 11 个 provider 均为同步请求/响应。
4. 不定义 search + generate 组合抽象。
5. 不为个别 provider 的特有字段（如 serper 的 `peopleAlsoAsk`、google_pse 的 `cx`）在核心 trait 中增加字段——这些走 `provider_options` 透传。
6. 不在核心 trait 中定义 `model_id` 路由逻辑——endpoint 路径参数（如 dataforseo 的 `/v3/serp/google/organic/live/advanced`）由各 provider 在 `do_search` 内部自行处理。
7. 不在 `SearchResultItem` 中定义 `published_date` 字段——多数 provider 不支持，tavily/serper 等可通过 `provider_metadata` 透传。

## 8. 变更范围

| 位置 | 变更 |
|---|---|
| `aimux-core/src/search_model.rs` | 新增：SearchModel trait + SearchCallOptions + SearchResult + SearchResultItem + SearchResponse |
| `aimux-core/src/lib.rs` | 新增 `pub mod search_model;` + re-export |
| `aimux-core/src/prelude.rs` | 新增 search 相关 re-export |
| `aimux-providers/` | 后续逐个实现 11 个 search provider |

## 9. 风险

1. **Vercel AI SDK 无参考**：search trait 无上游参考实现，接口设计需自行验证。缓解：先稳定 provider-facing trait，不急于定义用户 API。
2. **provider 差异大**：11 个 provider 的字段差异较大（如 dataforseo 用 Basic 鉴权、searxng 无鉴权、tavily 有 answer 字段）。缓解：核心 trait 只覆盖共性，特有字段走 `provider_options`。
3. **与 language model 搜索的组合**：用户可能期望 search → generate 的组合流程。缓解：本 RFC 不处理组合，留待后续提案。

## 10. 开放问题（已关闭）

1. ~~`SearchResultItem` 是否需要 `published_date` 字段？~~ **不加。** 多数 provider 不支持，tavily/serper 等可通过 `provider_metadata` 透传。与 RFC-0006 §2.3"只有请求代码会读取的配置项才对外暴露"原则一致。
2. ~~`time_range` 应为 `String` 还是 `enum`？~~ **保持 `String`。** 各 provider 的枚举值不统一（tavily 用 day/week/month/year，serper 用中文式描述），强行 enum 会丢失语义或导致 mapping 地狱。String + provider_options 是最务实的。
3. ~~是否需要 `SearchModel::max_results_per_call()` 方法？~~ **不需要。** search 不像 embedding 有批量调用的硬限制——`max_results` 已经在 `SearchCallOptions` 中，provider 自行截断即可。

## 11. 实现顺序

1. 本 RFC 评审通过后，先在 aimux-core 新增 trait 和类型。
2. 逐个实现 11 个 search provider（按调研优先级：tavily/serper/exa_ai 先行）。
3. 每个实现按 RFC-0006 流程：核验协议 → 实现 → wiremock 测试。
