# RFC-0007: Search Model Trait Design

> **Status**: ACCEPTED
> **Date**: 2026-07-28
> **Scope**: `aimux-core` adds `SearchModel` trait and accompanying types
> **Related**: [Provider Development Spec](0006-provider-development.md), [Provider Research Report](../docs/provider-research/README.md)

## 1. Motivation

The research report shows that 11 search providers (tavily/serper/exa_ai/firecrawl/linkup/parallel_ai/searxng/google_pse/tinyfish/you_com/dataforseo) are stalled because aimux-core has no search trait. The protocol evidence for these providers is all strong/medium, making this the largest single-category blocker.

No search model trait exists in the Vercel AI SDK reference implementation (only nine kinds: language/embedding/rerank/speech/transcription/image/video/files/realtime). Therefore, this RFC needs to design the search trait's interface contract from scratch.

## 2. Design Goals

1. Cover the protocol commonalities of the 11 search providers, without polluting the core interface with individual providers' unique fields.
2. Be consistent in style with existing traits (RerankingModel, EmbeddingModel): provider-facing `do_*` method + `SearchCallOptions` + `SearchResult`.
3. Do not modify the existing contract of the `Provider` trait (`name` + `language_model`); search capability is exposed via an additional method on the provider.
4. Do not introduce a user API layer (a `search` free function at the `generate_text` level); only define the provider-facing trait. A user API can be proposed separately later.

## 3. Trait Design

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

## 4. Provider Integration Approach

### 4.1 Do Not Modify the Provider Trait

The `Provider` trait remains unchanged (`name` + `language_model`). A pure search provider is not suited to being returned via `language_model` — consistent with the pattern for image/video/speech/rerank providers, search capability is exposed via an additional method on the provider:

```rust
// In the provider implementation (e.g. TavilyProvider)
impl TavilyProvider {
    pub fn search_model(&self, model_id: &str) -> TavilySearchModel { ... }
}

// Provider trait impl still required (language_model returns Unsupported error)
impl Provider for TavilyProvider {
    fn name(&self) -> &str { "tavily" }
    fn language_model(&self, _: &str) -> Result<Box<dyn LanguageModel>, AiMuxError> {
        Err(AiMuxError::Unsupported("tavily does not support language models".into()))
    }
}
```

### 4.2 Do Not Introduce a User API

This RFC does not define a `search()` free function (similar to `generate_text()`). Reasons:
- The existing `generate_text` / `stream_text` are designed around `LanguageModel`; search has a different input/output model.
- User API design needs to consider composition with `generate_text` (e.g. "search then generate"), which is a higher-level abstraction.
- Stabilize the provider-facing trait first; a user API can be proposed separately later.

## 5. Provider Mapping

| Provider | endpoint | query field | results field | Unique behavior |
|---|---|---|---|---|
| tavily | POST /search | query | results[].{title,url,content,score} | answer field |
| serper | POST /search | q | organic[].{title,link,snippet} | no score |
| exa_ai | POST /search | query | results[].{title,url,text} | no score |
| firecrawl | POST /v2/search | query | data[].{url,markdown} | search + scrape combined |
| linkup | POST /v1/search | query | results[].{url,title,content} | depth/outputType |
| parallel_ai | POST /v1/search | objective | results[] | objective mode |
| searxng | GET /search | q | results[].{title,url,content} | self-hosted, no auth |
| google_pse | GET /v1 | q | items[].{title,link,snippet} | requires cx parameter |
| tinyfish | POST /search | query | results[] | x-api-key |
| you_com | POST /search | query | hits[].{url,title,description} | ydc-index.io |
| dataforseo | POST /v3/serp/... | keyword | tasks[].result.organic[] | Basic auth |

**Commonality**: All providers have the basic query → results[] structure. title/url/content/snippet are common fields. score, answer, raw_content are optional fields.

## 6. Relationship with Existing Traits

- **Similarity to RerankingModel**: Both follow the query → ordered results pattern. But reranking's input is an existing list of documents (rerank), while search's input is a query (retrieved from the internet/index).
- **Relationship with LanguageModel**: Some search providers (e.g. Perplexity) integrate search within the language model (via the provider-executed tool `web_search`). This trait does not replace that pattern; instead, it provides an independent interface for pure search APIs.
- **GenerateContent::Source**: The existing `GenerateContent::Source` variant is already used for search citation results in language models. `SearchResultItem` is structurally similar to `Source` but serves a different purpose — `Source` is a citation within language model output, while `SearchResultItem` is the direct output of a search API.

## 7. Non-Goals

1. Do not modify the `Provider` trait.
2. Do not introduce a user API (`search()` free function).
3. Do not define streaming search — all 11 providers are synchronous request/response.
4. Do not define a search + generate composition abstraction.
5. Do not add fields to the core trait for individual providers' unique fields (e.g. serper's `peopleAlsoAsk`, google_pse's `cx`) — these are passed through via `provider_options`.
6. Do not define `model_id` routing logic in the core trait — endpoint path parameters (e.g. dataforseo's `/v3/serp/google/organic/live/advanced`) are handled by each provider internally within `do_search`.
7. Do not define a `published_date` field in `SearchResultItem` — most providers do not support it; tavily/serper etc. can pass it through via `provider_metadata`.

## 8. Scope of Changes

| Location | Change |
|---|---|
| `aimux-core/src/search_model.rs` | New: SearchModel trait + SearchCallOptions + SearchResult + SearchResultItem + SearchResponse |
| `aimux-core/src/lib.rs` | New `pub mod search_model;` + re-export |
| `aimux-core/src/prelude.rs` | New search-related re-exports |
| `aimux-providers/` | Implement the 11 search providers one by one later |

## 9. Risks

1. **No Vercel AI SDK reference**: The search trait has no upstream reference implementation; the interface design needs self-validation. Mitigation: stabilize the provider-facing trait first, do not rush to define a user API.
2. **Large provider differences**: The 11 providers differ significantly in fields (e.g. dataforseo uses Basic auth, searxng has no auth, tavily has an answer field). Mitigation: the core trait only covers commonalities; unique fields go through `provider_options`.
3. **Composition with language model search**: Users may expect a search → generate composition flow. Mitigation: this RFC does not handle composition; left for a later proposal.

## 10. Open Questions (closed)

1. ~~Does `SearchResultItem` need a `published_date` field?~~ **No.** Most providers do not support it; tavily/serper etc. can pass it through via `provider_metadata`. This is consistent with the RFC-0006 §2.3 principle "only config items that request code will read are exposed externally".
2. ~~Should `time_range` be `String` or `enum`?~~ **Keep `String`.** The enum values are inconsistent across providers (tavily uses day/week/month/year, serper uses Chinese-style descriptions); forcing an enum would lose semantics or cause mapping hell. String + provider_options is the most pragmatic.
3. ~~Is a `SearchModel::max_results_per_call()` method needed?~~ **No.** Unlike embedding, search has no hard batch-call limit — `max_results` is already in `SearchCallOptions`, and the provider can truncate on its own.

## 11. Implementation Order

1. After this RFC is approved, first add the trait and types in aimux-core.
2. Implement the 11 search providers one by one (in research priority order: tavily/serper/exa_ai first).
3. Each implementation follows the RFC-0006 process: verify protocol → implement → wiremock test.
