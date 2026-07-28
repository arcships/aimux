# 第 2 批调研记录（14 个 provider）

本批 14 个 provider 均为 L3 专用模态（search / rerank / image / audio / 文档解析 / 媒体生成）。
inventory 元数据（tier/protocol/openai_compatible）仅作线索，以下协议事实均依据官方 API 文档/官方 OpenAPI/官方 SDK 核验。
核验日期统一为 2026-07-28。

---

### dataforseo — Dataforseo

- **canonical ID**：dataforseo
- **aliases**：（无）
- **provider_kind**：model_vendor
- **inventory 分层**：tier=L3 / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：search

#### 1. 官方协议证据

- **文档 URL**：https://docs.dataforseo.com/v3/ 、https://docs.dataforseo.com/v3/auth/
- **核验来源**：官方 API 文档
- **证据强度**：中（官方确认 base URL、Basic 鉴权、REST 传输；search 能力对应 SERP API 系列，未逐一核验单端点请求体）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://api.dataforseo.com/
- **鉴权**：方式=HTTP Basic（login:password，Base64 编码置入 `Authorization` 头）/ 环境变量=无统一约定（账号 login + 自动生成 password）/ 是否必需=是
- **endpoint 公式**：各能力为独立端点；search 能力对应 SERP API 系列，如 `POST /v3/serp/google/organic/live/advanced`（具体端点按搜索引擎/任务类型选择）
- **协议类型**：专用模态（原生 REST，SEO/SERP 数据）
- **请求结构要点**：POST JSON body，含 query、location_name/language_code 等参数（按端点而异）；Basic 鉴权
- **响应结构要点**：JSON，含 `tasks[]`，每 task 含 `result[]` 与 `organic_results[]`（标题/url/description/position 等）；厂商专属结构
- **流式**：无
- **错误结构**：厂商专属（status_code/status_message + 内层错误码）
- **特有行为**：按 task 计费；端点按 SEO 数据类型划分（SERP / Backlinks / On-Page / DataForSEO Labs 等）

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：模态专用
- **依据**：非聊天/嵌入协议，Basic 鉴权 + 厂商专属 SERP JSON 结构，与 OpenAI 共享层无共性
- **可复用模型 ID 样例**：dataforseo/search（litellm 抽象，对应 SERP API）
- **是否需扩展共享层**：否

#### 4. 风险与限制

- search 能力实为 SERP 数据 API，需指定搜索引擎/位置/语言；端点众多，需明确映射哪一个
- Basic 鉴权（非 Bearer），与多数 provider 不同
- 计费按 task，需注意成本

#### 5. 优先级建议

- **优先级**：P2
- **理由**：证据中等；SEO/SERP 数据为小众模态，协议清晰但端点复杂，可后续按需实现

---

### duckduckgo — Duckduckgo

- **canonical ID**：duckduckgo
- **aliases**：（无）
- **provider_kind**：model_vendor
- **inventory 分层**：tier=L3 / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：search

#### 1. 官方协议证据

- **文档 URL**：https://duckduckgo.com/api
- **核验来源**：官方 API 文档（Instant Answer API）
- **证据强度**：弱（官方仅提供 Instant Answer API，返回即时答案/摘要，非完整网页搜索结果；inventory 的 search 能力（完整搜索）无官方协议）
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://api.duckduckgo.com
- **鉴权**：方式=无（公开免鉴权）/ 环境变量=无 / 是否必需=否
- **endpoint 公式**：`GET /?q=<query>&format=json`（Instant Answer API）
- **协议类型**：原生（专用：即时答案）
- **请求结构要点**：GET 查询参数 q、format=json、no_html/redirect 等可选
- **响应结构要点**：JSON，含 `AbstractText`/`AbstractURL`/`Heading`/`RelatedTopics[]`/`Results[]`/`Type` 等；仅即时答案与相关主题，非完整搜索结果列表
- **流式**：无
- **错误结构**：厂商专属（空结果或字段缺失）
- **特有行为**：仅返回即时答案（摘要/消歧/相关主题），不提供完整网页搜索结果；完整搜索结果依赖第三方非官方抓取库（如 duckduckgo-search）

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：模态专用（待定）
- **依据**：官方 Instant Answer API 用途有限，与 search 能力（完整网页搜索）预期不符；完整搜索无官方协议
- **可复用模型 ID 样例**：duckduckgo/search（inventory 抽象，无对应官方端点）
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 无官方完整网页搜索 API；第三方抓取库随时可能失效/被限流
- Instant Answer API 结果稀疏，不适合作为通用 search provider

#### 5. 优先级建议

- **优先级**：搁置
- **理由**：官方无完整搜索协议；Instant Answer API 不匹配 search 能力，依赖第三方非官方方案，不宜纳入

---

### exa_ai — EXA AI

- **canonical ID**：exa_ai
- **aliases**：（无）
- **provider_kind**：model_vendor
- **inventory 分层**：tier=L3 / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：search

#### 1. 官方协议证据

- **文档 URL**：https://exa.ai/docs/reference/search 、https://exa.ai/docs/reference/search-api-guide
- **核验来源**：官方 API 文档
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://api.exa.ai
- **鉴权**：方式=`x-api-key` 头（亦支持 `Authorization: Bearer`）/ 环境变量=EXA_API_KEY（社区惯例，官方未强制命名）/ 是否必需=是
- **endpoint 公式**：`POST /search`（另有 `/contents`、`/findSimilar`、`/answer`、`/research` 等）
- **协议类型**：专用模态（原生，AI 网页搜索）
- **请求结构要点**：JSON body `{query, numResults, contents:{highlights,text,summary}, includeDomains/excludeDomains, startPublishedDate, type(neural/keyword/auto)}` 等
- **响应结构要点**：`{results:[{title,url,author,publishedDate,text,highlights,summary,...}], requestId, resolvedSearchType, costDollars}`
- **流式**：无（基础 /search；/research 等端点可能支持流式，需另核）
- **错误结构**：厂商专属（JSON error）
- **特有行为**：neural/keyword/auto 搜索类型；可同时抽取内容（text/highlights/summary）；按 costDollars 计费

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：模态专用
- **依据**：原生搜索协议，请求/响应结构与 OpenAI 无共性
- **可复用模型 ID 样例**：exa_ai/search
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 基础搜索非流式；内容抽取字段较多
- 计费按搜索类型与内容抽取

#### 5. 优先级建议

- **优先级**：P1
- **理由**：主流 AI 搜索 API，协议清晰、文档完备，易于实现原生 search provider

---

### firecrawl — Firecrawl

- **canonical ID**：firecrawl
- **aliases**：（无）
- **provider_kind**：model_vendor
- **inventory 分层**：tier=L3 / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：search

#### 1. 官方协议证据

- **文档 URL**：https://docs.firecrawl.dev/api-reference/endpoint/search 、https://docs.firecrawl.dev/api-reference/introduction
- **核验来源**：官方 API 文档
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://api.firecrawl.dev（v2: `/v2/search`；v1: `/v1/search`）
- **鉴权**：方式=`Authorization: Bearer <API_KEY>` / 环境变量=FIRECRAWL_API_KEY（社区惯例）/ 是否必需=是
- **endpoint 公式**：`POST /v2/search`（搜索+抓取合一）；另有 `/v2/scrape`、`/v2/crawl`
- **协议类型**：专用模态（原生，网页搜索+抓取）
- **请求结构要点**：JSON body `{query, limit, sources:["web"|"news"|"images"], includeDomains/excludeDomains, country, tbs, scrapeOptions, highlights}`
- **响应结构要点**：`{success, data:{web:[...],images:[...],news:[...]}, warning, id, creditsUsed}`；每条含 title/url/markdown/html/metadata 等
- **流式**：无（搜索同步；crawl 支持异步/webhook）
- **错误结构**：厂商专属（`{success:false, error, code?}`）
- **特有行为**：搜索与抓取合一，可直接返回页面 markdown/html；按 creditsUsed 计费；v1/v2 并存

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：模态专用
- **依据**：原生搜索+抓取协议，与 OpenAI 无共性
- **可复用模型 ID 样例**：firecrawl/search
- **是否需扩展共享层**：否

#### 4. 风险与限制

- v1/v2 双版本字段不同，需明确目标版本
- 搜索结果含抓取内容，体积较大

#### 5. 优先级建议

- **优先级**：P1
- **理由**：文档完备、协议清晰，搜索+抓取一体，常用 AI 检索后端

---

### google_pse — Google PSE

- **canonical ID**：google_pse
- **aliases**：（无）
- **provider_kind**：model_vendor
- **inventory 分层**：tier=L3 / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：search

#### 1. 官方协议证据

- **文档 URL**：https://developers.google.com/custom-search/v1/introduction 、https://developers.google.com/custom-search/v1/reference/rest/v1/cse/list
- **核验来源**：官方 API 文档
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://www.googleapis.com/customsearch/v1
- **鉴权**：方式=API Key（query 参数 `key=`）+ 可选 OAuth；另需 `cx`（可编程搜索引擎 ID）/ 环境变量=GOOGLE_API_KEY + GOOGLE_CSE_ID（社区惯例）/ 是否必需=是
- **endpoint 公式**：`GET /customsearch/v1?key=<KEY>&cx=<CX>&q=<QUERY>`（cse.list）
- **协议类型**：专用模态（原生，基于 OpenSearch 1.1 的 Google REST）
- **请求结构要点**：GET 查询参数 q、cx、key、num、start、lr、cr、safe、dateRestrict 等
- **响应结构要点**：OpenSearch 结构 `{queries, searchInformation, items:[{title,link,snippet,displayLink,pagemap,...}], context, kind}`
- **流式**：无
- **错误结构**：Google 标准 `error{code,message,errors[]}`
- **特有行为**：受限于可编程搜索引擎配置（cx）；每日免费配额约 100 次；结果范围由 cx 配置的站点决定

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：模态专用
- **依据**：Google 原生 REST + API Key 鉴权 + OpenSearch 结构，与 OpenAI 无共性
- **可复用模型 ID 样例**：google_pse/search
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 需用户自建 PSE 并提供 cx；免费配额低
- 结果范围受 cx 配置约束

#### 5. 优先级建议

- **优先级**：P2
- **理由**：协议清晰但需 cx 配置、配额受限，按需实现

---

### jina_ai — Jina AI

- **canonical ID**：jina_ai
- **aliases**：（无）
- **provider_kind**：model_vendor
- **inventory 分层**：tier=L3 / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：rerank（Jina 另含 embedding/reader，本次仅核验 rerank）

#### 1. 官方协议证据

- **文档 URL**：https://jina.ai/reranker/ （含请求示例）、https://docs.jina.ai
- **核验来源**：官方 API 文档
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://api.jina.ai
- **鉴权**：方式=`Authorization: Bearer <API_KEY>` / 环境变量=JINAI_API_KEY（社区惯例）/ 是否必需=是
- **endpoint 公式**：`POST /v1/rerank`（rerank）；另 `/v1/embeddings`（OpenAI 兼容）
- **协议类型**：专用模态（原生 rerank 协议，Cohere/Jina 式）
- **请求结构要点**：JSON body `{model, query, documents:[...], top_n, return_documents}`
- **响应结构要点**：`{model, results:[{index, relevance_score, document?}], usage:{total_tokens,...}}`
- **流式**：无
- **错误结构**：厂商专属（JSON error）
- **特有行为**：rerank 模型如 jina-reranker-v2-base-multilingual、jina-reranker-v3；注意 Jina embedding 端点为 OpenAI 兼容，但 rerank 为独立原生协议

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：模态专用
- **依据**：rerank 为独立原生协议（query+documents→relevance_score），非 OpenAI chat/embeddings 结构
- **可复用模型 ID 样例**：jina-reranker-v2-base-multilingual、jina-reranker-v3
- **是否需扩展共享层**：否（rerank 为独立模态，建议单独 rerank 抽象）

#### 4. 风险与限制

- rerank 与 embedding 协议不同，勿混用 OpenAI 兼容路径
- model 字段需对齐 Jina 模型名

#### 5. 优先级建议

- **优先级**：P1
- **理由**：rerank 是常用检索模态，协议标准、文档完备

---

### linkup — Linkup

- **canonical ID**：linkup
- **aliases**：（无）
- **provider_kind**：model_vendor
- **inventory 分层**：tier=L3 / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：search

#### 1. 官方协议证据

- **文档 URL**：https://docs.linkup.so/pages/documentation/endpoints/search/reference 、https://docs.linkup.so/pages/documentation/platform/authentication
- **核验来源**：官方 API 文档
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://api.linkup.so
- **鉴权**：方式=`Authorization: Bearer <token>`（亦支持 x402 付费协议）/ 环境变量=LINKUP_API_KEY（社区惯例）/ 是否必需=是
- **endpoint 公式**：`POST /v1/search`（另 `/v1/fetch`）
- **协议类型**：专用模态（原生，AI 网页搜索）
- **请求结构要点**：JSON body `{q, depth:"deep"|"standard"|"fast", outputType:"sourcedAnswer"|"searchResults"|"structured", includeDomains/excludeDomains, dataSchema?}`
- **响应结构要点**：outputType=sourcedAnswer 时 `{answer, sources:[{name,url,content,...}]}`；searchResults 时 `{results:[{name,url,content,favicon,type}]}`
- **流式**：无
- **错误结构**：厂商专属（JSON error）
- **特有行为**：depth 决定搜索深度（fast/standard/deep）；outputType 决定返回答案或纯结果

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：模态专用
- **依据**：原生搜索协议，与 OpenAI 无共性
- **可复用模型 ID 样例**：linkup/search、linkup/search-deep
- **是否需扩展共享层**：否

#### 4. 风险与限制

- outputType/depth 组合影响响应结构，需按类型解析
- deep 模式延迟较高

#### 5. 优先级建议

- **优先级**：P2
- **理由**：协议清晰，与其他 search provider 同类；按需实现

---

### parallel_ai — Parallel AI

- **canonical ID**：parallel_ai
- **aliases**：（无）
- **provider_kind**：model_vendor
- **inventory 分层**：tier=L3 / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：search

#### 1. 官方协议证据

- **文档 URL**：https://docs.parallel.ai/search/search-quickstart 、https://docs.parallel.ai/getting-started/overview
- **核验来源**：官方 API 文档
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://api.parallel.ai
- **鉴权**：方式=`x-api-key` 头（PARALLEL_API_KEY）/ 环境变量=PARALLEL_API_KEY（官方文档确认）/ 是否必需=是
- **endpoint 公式**：`POST /v1/search`（另 entity search、extract 等）
- **协议类型**：专用模态（原生，AI 网页搜索）
- **请求结构要点**：JSON body `{objective, search_queries:[...], mode?:"advanced"|"turbo"}`
- **响应结构要点**：`{search_id, results:[{url,title,publish_date,excerpts:[...]}], warnings, usage:[{name,...}]}`
- **流式**：无
- **错误结构**：厂商专属（JSON error）
- **特有行为**：以 objective + search_queries 驱动；返回 LLM 优化 excerpts；advanced（高质量）/turbo（低延迟）模式

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：模态专用
- **依据**：原生搜索协议，与 OpenAI 无共性
- **可复用模型 ID 样例**：parallel_ai/search、parallel_ai/search-pro
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 请求字段为 objective/search_queries，与其他 search provider 的 q/query 不同
- 较新 API，字段可能演进

#### 5. 优先级建议

- **优先级**：P2
- **理由**：协议清晰；新兴 search API，按需实现

---

### recraft — Recraft

- **canonical ID**：recraft
- **aliases**：（无）
- **provider_kind**：model_vendor
- **inventory 分层**：tier=L3 / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：image_generation

#### 1. 官方协议证据

- **文档 URL**：https://www.recraft.ai/docs/api-reference/endpoints 、https://www.recraft.ai/docs/api-reference/getting-started
- **核验来源**：官方 API 文档
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://external.api.recraft.ai/v1
- **鉴权**：方式=`Authorization: Bearer <RECRAFT_API_TOKEN>` / 环境变量=RECRAFT_API_TOKEN（官方文档确认）/ 是否必需=是
- **endpoint 公式**：`POST /images/generations`（另 `/images/generations/raster`、`/images/generations/vector`、`/images/edit`、`/remove_background`、`/colors`、`/styles` 等）
- **协议类型**：OpenAI 兼容（Images API 形态，含扩展字段）
- **请求结构要点**：JSON/multipart body `{prompt, model, n, size(WxH), style, style_id, negative_prompt, random_seed, response_format:"url"|"b64_json", text_layout?}`；图像输入支持 multipart 或 `image_url`（data URL）
- **响应结构要点**：`{data:[{url 或 b64_json}]}`（OpenAI Images 式）
- **流式**：无
- **错误结构**：与 OpenAI 共享结构基本一致（厂商可能扩展）
- **特有行为**：模型 recraftv4_1/recraftv4/recraftv3/recraftv2（含 `_vector` 变体）；style/style_id、raster/vector 端点变体为 Recraft 扩展；可生成矢量图

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：共享层扩展（OpenAI Images 薄封装）
- **依据**：主体遵循 OpenAI `/v1/images/generations` 请求/响应结构（prompt/n/size/response_format/data[].url），可直接复用 OpenAI Images 共享层；需透传 style/style_id/negative_prompt 等 Recraft 专有字段
- **可复用模型 ID 样例**：recraftv3、recraftv2（另 recraftv4_1、recraftv4）
- **是否需扩展共享层**：是（在 Images 请求中透传 style/style_id/negative_prompt 等 Recraft 专有可选字段；并支持 raster/vector 端点变体）

#### 4. 风险与限制

- base URL 为 `external.api.recraft.ai/v1`（非标准 `api.` 域）
- 矢量/光栅变体端点与 style 枚举需对齐模型
- 图像编辑/去除背景等端点为 Recraft 专有，超出 OpenAI Images 范围

#### 5. 优先级建议

- **优先级**：P1
- **理由**：OpenAI Images 兼容，复用共享层成本低；图像生成常用模态

---

### reducto — Reducto

- **canonical ID**：reducto
- **aliases**：（无）
- **provider_kind**：model_vendor
- **inventory 分层**：tier=L3 / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：other（文档解析/parse）

#### 1. 官方协议证据

- **文档 URL**：https://docs.reducto.ai/parse/overview 、https://docs.reducto.ai/quickstart
- **核验来源**：官方 API 文档
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://platform.reducto.ai
- **鉴权**：方式=`Authorization: Bearer <REDUCTO_API_KEY>` / 环境变量=REDUCTO_API_KEY（官方文档确认）/ 是否必需=是
- **endpoint 公式**：`POST /parse`（同步）、`POST /parse_async`（异步）；另 `POST /upload`
- **协议类型**：专用模态（原生，文档解析）
- **请求结构要点**：JSON body `{input: file_id|public_url|presigned_url|jobid://..., retrieval:{chunking:{chunk_mode}}}`；文件先经 `/upload` 取 file_id
- **响应结构要点**：`{job_id, duration, result:{type:"full"|"url", chunks:[{content, embed, blocks:[{type,bbox,confidence}]}]}, usage:{num_pages,credits}, studio_link}`
- **流式**：无（异步 /parse_async 支持 webhook）
- **错误结构**：厂商专属（JSON error）
- **特有行为**：OCR+版面检测+分块；input 支持 reducto://、public URL、presigned URL、jobid://；同步/异步双端点

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：模态专用
- **依据**：文档解析原生协议，与 OpenAI 无共性
- **可复用模型 ID 样例**：reducto/parse-legacy、reducto/parse-v3
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 大文件需异步 + 上传/presigned；多步流程（upload→parse→轮询）
- 结果 type 为 url 时需额外下载

#### 5. 优先级建议

- **优先级**：P2
- **理由**：文档解析为小众模态；协议清晰但多步异步，按需实现

---

### runwayml — Runwayml

- **canonical ID**：runwayml
- **aliases**：（无）
- **provider_kind**：model_vendor
- **inventory 分层**：tier=L3 / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：audio_speech、image_generation、video_generation

#### 1. 官方协议证据

- **文档 URL**：https://docs.dev.runwayml.com/api/ （OpenAPI 3.1 嵌入）、https://docs.dev.runwayml.com/guides/using-the-api 、https://docs.dev.runwayml.com/guides/setup；官方 SDK `@runwayml/sdk`
- **核验来源**：官方 API 文档 + 官方 OpenAPI + 官方 SDK
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://api.dev.runwayml.com（官方 SDK 默认；可经 `RUNWAYML_BASE_URL` 覆盖）
- **鉴权**：方式=`Authorization: Bearer <RUNWAYML_API_SECRET>`；必需头 `X-Runway-Version: 2024-11-06` / 环境变量=RUNWAYML_API_SECRET（官方确认）/ 是否必需=是
- **endpoint 公式**：一律 `/v1/<能力>`：`POST /v1/image_to_video`、`/v1/text_to_video`、`/v1/video_to_video`、`/v1/text_to_image`、`/v1/text_to_speech`、`/v1/sound_effect`、`/v1/voice_isolation`、`/v1/voice_dubbing`、`/v1/speech_to_speech`、`/v1/character_performance`、`/v1/image_upscale`、`/v1/video_upscale`；任务管理 `GET /v1/tasks/{id}`
- **协议类型**：专用模态（原生，异步任务式媒体生成）
- **请求结构要点**：JSON body，按端点而异：text_to_image `{model, promptText, ratio, referenceImages?}`；image_to_video `{model, promptImage, promptText?, ratio, duration}`；text_to_video `{model, promptText, ratio, duration}`；text_to_speech `{model, text, voice:{type,presetId}}`
- **响应结构要点**：创建任务返回 `{id, status, ...}`；轮询 `GET /v1/tasks/{id}` 至 status=RUNNING/SUCCEEDED/FAILED，output 含生成媒体 URL
- **流式**：无（任务异步轮询；实时 avatar 走 realtime_sessions/WebRTC）
- **错误结构**：厂商专属（OpenAPI 定义 error 对象）
- **特有行为**：全异步任务模型（create→poll）；模型如 gen4.5/gen4_turbo/gen4_aleph/gen4_image/gen4_image_turbo、veo3 系列、eleven_multilingual_v2（TTS 走 ElevenLabs）；必需 X-Runway-Version 头

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：模态专用（原生异步任务）
- **依据**：原生任务式媒体生成协议，多端点 + 异步轮询 + 专有版本头，与 OpenAI chat/embeddings 无共性
- **可复用模型 ID 样例**：runwayml/gen3a_turbo、gen4_aleph、gen4_image、gen4_image_turbo、eleven_multilingual_v2
- **是否需扩展共享层**：否（建议独立 media-generation 任务抽象：create→poll→output）

#### 4. 风险与限制

- 全异步，需轮询/回调机制；多模态多端点
- 必需 X-Runway-Version 头，版本变更需跟踪
- TTS/音效实际经 ElevenLabs 模型

#### 5. 优先级建议

- **优先级**：P1
- **理由**：主流媒体生成 API，OpenAPI 完备；异步任务模型需独立抽象但价值高

---

### searxng — Searxng

- **canonical ID**：searxng
- **aliases**：（无）
- **provider_kind**：model_vendor
- **inventory 分层**：tier=L3 / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：search

#### 1. 官方协议证据

- **文档 URL**：https://docs.searxng.org/dev/search_api.html
- **核验来源**：官方 API 文档
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：自部署实例 URL（如 https://searx.example.org），无官方托管
- **鉴权**：方式=无（公开；实例可自配限制）/ 环境变量=无（需配置实例 URL）/ 是否必需=否
- **endpoint 公式**：`GET/POST /search`（或 `/`），参数 q + format=json
- **协议类型**：专用模态（原生，元搜索聚合）
- **请求结构要点**：GET 查询参数或 POST form：q、format=json|csv|rss、categories、language、pageno、time_range、safesearch
- **响应结构要点**：JSON `{query, results:[{url,title,content,engine,score,publishedDate,...}], number_of_results, suggestions, unresponsive_engines}`
- **流式**：无
- **错误结构**：403 Forbidden（format 未启用时）；实例专属
- **特有行为**：自托管元搜索，聚合多引擎；JSON 输出需在实例 settings.yml 启用；许多公共实例禁用 JSON

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：模态专用
- **依据**：自托管元搜索原生协议，无鉴权、依赖实例配置，与 OpenAI 无共性
- **可复用模型 ID 样例**：searxng/search
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 需用户提供自部署实例 URL；JSON 输出可能被禁用（403）
- 公共实例不稳定/限流，建议自建

#### 5. 优先级建议

- **优先级**：P2
- **理由**：协议简单清晰，但依赖自托管实例与 format 启用，按需实现

---

### serper — Serper

- **canonical ID**：serper
- **aliases**：（无）
- **provider_kind**：model_vendor
- **inventory 分层**：tier=L3 / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：search

#### 1. 官方协议证据

- **文档 URL**：https://serper.dev （官方站点含响应示例与端点）；请求格式经官方 playground + 多源集成一致确认
- **核验来源**：官方站点 + 多源一致
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://google.serper.dev
- **鉴权**：方式=`X-API-KEY` 头 / 环境变量=SERPER_API_KEY（社区惯例）/ 是否必需=是
- **endpoint 公式**：`POST /search`（另 /news、/images、/videos、/places、/shopping、/scholar、/patents、/autocomplete）
- **协议类型**：专用模态（原生，Google SERP）
- **请求结构要点**：JSON body `{q, gl, hl, num, page, tbs, location?}`
- **响应结构要点**：`{knowledgeGraph?, organic:[{title,link,snippet,position,sitelinks?}], peopleAlsoAsk:[...], relatedSearches:[...], images?/news?...}`
- **流式**：无
- **错误结构**：厂商专属（JSON error）
- **特有行为**：返回 Google SERP 结构（knowledgeGraph/organic/peopleAlsoAsk/relatedSearches）；按 query 计费（credits）

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：模态专用
- **依据**：原生 Google SERP 协议，X-API-KEY 鉴权，与 OpenAI 无共性
- **可复用模型 ID 样例**：serper/search
- **是否需扩展共享层**：否

#### 4. 风险与限制

- 返回 Google 原始 SERP 结构，字段多；非 AI 摘要型
- 请求头为 X-API-KEY（非 Bearer）

#### 5. 优先级建议

- **优先级**：P1
- **理由**：主流 SERP API，协议简单、鉴权统一，易于实现

---

### soniox — Soniox

- **canonical ID**：soniox
- **aliases**：（无）
- **provider_kind**：model_vendor
- **inventory 分层**：tier=L3 / protocol=unknown / openai_compatible=null / confidence=0.3
- **能力**（本次调研覆盖）：audio_transcription

#### 1. 官方协议证据

- **文档 URL**：https://soniox.com/docs/api-reference （OpenAPI: https://soniox.com/docs/openapi.yaml）、https://soniox.com/docs/stt/async/async-transcription 、https://soniox.com/docs/api-reference/stt/transcriptions/create_transcription
- **核验来源**：官方 API 文档 + 官方 OpenAPI
- **证据强度**：强
- **核验日期**：2026-07-28

#### 2. 协议事实

- **base URL**：https://api.soniox.com/v1（实时 STT WebSocket：wss://stt-rt.soniox.com）
- **鉴权**：方式=`Authorization: Bearer <SONIOX_API_KEY>` / 环境变量=SONIOX_API_KEY（官方确认）/ 是否必需=是
- **endpoint 公式**：异步：`POST /v1/transcriptions`（创建）、`GET /v1/transcriptions/{id}`（轮询）、`GET /v1/transcriptions/{id}/transcript`（取结果）、`POST /v1/files`（上传）；`GET /v1/models`
- **协议类型**：专用模态（原生，STT 异步 + WebSocket 实时）
- **请求结构要点**：`POST /v1/transcriptions` JSON `{model:"stt-async-v5", audio_url|file_id, language_hints?, enable_speaker_diarization?, enable_language_identification?, translation?, context?, webhook_url?}`
- **响应结构要点**：201 `{id, status:"queued"|"processing"|"completed"|"error", created_at, model, audio_url/file_id, filename, audio_duration_ms, error_type?}`；transcript 返回 tokens/segments（含 speaker/language）
- **流式**：实时走 WebSocket（wss://stt-rt.soniox.com）；异步为轮询/webhook
- **错误结构**：厂商专属 ApiError `{status_code, error_type, message, validation_errors[], request_id, more_info}`
- **特有行为**：异步任务模型（create→poll/webhook）；支持说话人分离、语言识别、翻译；模型 stt-async-v4/v5

#### 3. 实现路径建议（对应 RFC-0006 §2.2）

- **建议路径**：模态专用
- **依据**：STT 原生协议（异步任务 + WebSocket 实时），与 OpenAI 无共性
- **可复用模型 ID 样例**：soniox/stt-async-v4、soniox/stt-async-v5
- **是否需扩展共享层**：否（建议独立 STT 抽象：异步任务 + 实时流）

#### 4. 风险与限制

- 异步需轮询/webhook；实时需 WebSocket 客户端
- 区域端点（US/EU/JP）需按 data residency 选择

#### 5. 优先级建议

- **优先级**：P2
- **理由**：STT 为独立模态，OpenAPI 完备；按需实现（异步优先，实时后续）
