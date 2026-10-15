# RFC-0040：Provider 构造、凭证边界与原生传输契约

> **状态：Draft，待评审；本文不表示实现完成或维护者已接受。**
>
> **范围：** ROADMAP 的 B1–B9、#174 / #175、L0 native passthrough；区分架构切换必需工作与新增公开能力。
>
> **核对基线：** master `72a37b5`；目标架构参考尚未合并的 [PR #200](https://github.com/arcships/aimux/pull/200) head `d8c3d15`。#200 的 §0.4 已记录四项决定，不能说没有记录；PR 仍开放且文档一致性 review 尚待处理，不能据此宣称已在 master 生效。
>
> **前置决策门：** #200 的编号、状态、与已合并 RFC-0036 / ROADMAP 的替代关系需在合并前对齐。本文以其全链路 provider→model 结构作为目标，不恢复旧的 `from_resolved`、构造器 shim 或长期新旧 ABI 共存方案。
>
> **Related：** [#166](https://github.com/arcships/aimux/issues/166)、[#174](https://github.com/arcships/aimux/issues/174)、[#175](https://github.com/arcships/aimux/issues/175)、[RFC-0017](0017-provider-config-dx.md)、[RFC-0018](0018-codex-subscription.md)、[RFC-0020](0020-external-provider-config.md)、[RFC-0036（现行定位）](0036-positioning-and-layered-architecture.md)。registry 维护流程由 RFC-0033（`0033-registry-maintenance.md`）承担，ops 传输由 RFC-0039（`0039-ops-bindings-contract.md`）承担，消息类型由 RFC-0042（`0042-v4-type-boundaries.md`）承担；体积性能引用 RFC-0038（`0038-size-performance-budgets.md`），回放治理引用 RFC-0041（`0041-replay-governance.md`）。这些独立 Draft PR 的文件名在合并前仅作引用，不伪造已存在的相对链接。

## 1. 问题与边界

当前 `aimux-providers/src/provider.rs` 的内建 `RegistryEntry` 只有 name/display/base_url/env_var/profile；外部条目带 protocol 但仅支持 openai_compat。这不是通用协议工厂已经落地的证据。#166 的 B 轨把协议实现去重、配置式选择与能力保真混在一起；#174 的统一 `apply_auth(headers, spec, credential)` 又不足以签名最终 URL/body；#175 的全局解析序、store 与自动刷新会改变现有包的求值时机和 RFC-0018 的宿主持久化责任。

本 RFC 将其拆成三个交付面：

1. **G1 架构切换：** provider 工厂私有 settings、显式注册条目、包内 headers/fetch 鉴权组合、准确的能力与来源信息，以及行为回归。
2. **G2 public L0：** 对已配置 provider 的原生 HTTP 调用入口。这是新增对外能力，不搭便车进入 #200 的“只清理/对齐/修 bug”范围；本文给出可实现契约，独立评审、独立启用。
3. **G3 可选凭证协调：** 宿主侧 store / refresh 协调规则；内建包既有 ADC 等能力按包实现保留。通用自动刷新 API 属新增能力，不能因写出接口就算本期支持。

不做 OAuth 交互登录、设备码/PKCE UI、账号池、跨账号失败转移、多租户网关、宿主工具执行、网络代理服务。跨语言宿主回调遵循 #200 §0.4 Q4，Node/Python 也不作为例外提前开放。

## 2. 设计归属与唯一真相

| 数据/职责 | 所属层 | 不能承担的职责 |
|---|---|---|
| V4 Provider / Model traits | aimux-provider | 不含 registry、凭证 store、重试或公开 config |
| Fetch、Resolvable、单次 HTTP、传输诊断、中性 registration/descriptor 类型 | aimux-provider-utils | 不依赖 aimux 的 ProviderRegistry；不重跑 model operation |
| create_xxx、私有 settings、协议编解码、签名、包内鉴权 | aimux-providers | 不从 registry alias 猜厂商/协议，不输出完整 settings 快照 |
| registry/model resolution、操作预算与取消 | aimux | 不加载厂商工厂，不扫描任意进程 env |
| registry namespace、允许的凭证来源与 endpoint、store | 宿主组装层 | 不经外部请求隐式授予任意 env 读取能力 |
| 录制与 replay_fetch | aimux-devtools | 不以 descriptor 重建秘密，不让 auth 识别 replay 后暗中跳过 |

### 2.1 Descriptor、注册对象与协议是三种东西

- **Package descriptor** 是无凭证静态描述：package ID、settings schema、credential 字段标记、支持的模型方法/模态、能力方法、认证模式、模板参数声明、离线鉴权 recipe。它服务生成、校验和能力发现，不是“所有 provider 都变成一个协议解释器”。
- **ProviderRegistration** 持有 `provider + extensions + metadata` 的运行时对象。registry key 是宿主别名；同一 package 可以注册多个隔离实例。命名方法、discovery、files、tools 必须通过 extensions 保留，不能擦除为标准 Provider 后丢失。
- **Protocol implementation** 是包内代码。compat preset 可复用内部实现；原生 OpenAI、xAI、Mistral 等不能只因 wire 相似而强行降成一个带厂商分支的通用模型。

descriptor 中的 protocol/family 标识只描述实际适配实现和测试分组，不用于动态下载代码、不决定 model.provider、不充当可回放的配置。审计族来自显式元数据，不从 URL 或 provider 字符串子串推断。

### 2.2 构造与解析契约

`default_providers()` 与 `providers_from_config(document)` 返回 registration 集合；后者是纯组装，不修改进程全局 overlay。`create_provider_registry(entries, options)` 在 aimux 层完成。未知 package、未知设置、重复 key、缺失模板参数或不支持的方法必须返回具体配置错误，不能回退到 OpenAI/Gateway。

结构化模型引用包含 registry namespace/key、model ID 和可选 method；字符串便利入口只在第一个 `:` 分割，其余 model ID 原样保留。直接传 model 时 reference 可为空，不猜来源。wrap 保留 registration 扩展与 sidecar；改变模型含义时显式更新/清除来源。

工厂、取模型、每次请求的求值时机按 #200 第一部分 §3.3 的逐包表执行，不强行统一：OpenAI key 在请求 headers 求值；compat 的显式设置在工厂固定；preset 的声明式 key 可懒加载。默认实例获取不 panic、不提前读取可能失败的配置；显式工厂维持包定义的错误时机。`Resolvable::Future` 只求一次，`AsyncFn` 每次求值，两者测试必须可区分。

模板只允许 descriptor 声明的参数，分别定义来源、类型和校验。region/project/location 是有类型参数，不从占位符名称猜 env。Vertex global/eu/us/其他 location 的 host 选择、Bedrock region 和本地服务 base URL env 均由显式规则覆盖；完成展开后才检查未解析占位符。URL host 参数不能含斜杠、用户信息、端口注入或 query；路径参数按 segment 编码。

## 3. 鉴权与秘密边界（G1）

### 3.1 不采用跨包万能 apply_auth

保留可复用 header 操作与签名原语；最终组合由包负责：

- API key / bearer 在包的 headers resolver 注入，header 名、前缀、额外版本头遵循该包契约。
- SigV4 在 fetch 装饰器签名最终 method/URL/headers/body bytes；multipart 先编码，再签名。不能只给 HeaderMap 就声称完成签名。
- Azure token、Vertex express 等按包 fetch 组合；ADC 走包明确的异步凭证能力。不得 shell out 获取凭证。
- `auth=none` 不解析 key/env/store，不注入占位 key。它表示“不自动加认证”，不是静默删除调用方明确传入的 Authorization；若 endpoint policy 禁止认证头，应显式拒绝。

header 名大小写不敏感，以覆盖插入避免重复认证头，删除项遵循 `None` 语义。原生 model 路径的 provider/call/auth 优先序逐包测试，不能为了统一擅自改变 SDK 行为；G2 的凭证头防覆盖规则见 §5。

`Some("")` 是显式值，不能按 None 落回 env/store。是否空串有效由包/认证模式决定：load_api_key 按 #200 原样返回空串；不能把 #166 的“空 key 等同无鉴权”移植到所有包。缺失 key 与缺失一般设置分别映射 LoadApiKey / LoadSetting；错误只允许携带字段名或 env 名，不带值。

### 3.2 来源、目的地与隔离

配置文件仅在 descriptor 标记为 credential 的字段接受 `{ "env": "NAME" }`，不保留 `env:NAME` 字符串前缀兼容读取。解析由受信宿主进行，不让不可信 Web 请求自行指定任意 env；宿主需提供 env allowlist 或预绑定凭证句柄。

凭证作用域至少绑定 `(宿主 namespace, registration 实例, 认证模式, 允许的 origin/audience)`。只用 provider 字符串作 store key 不足以隔离两个同名实例。更换 base URL 必须新建 provider，不继承旧连接的 key、Authorization、token resolver 或默认 env 选择；确需复用时由宿主显式重新绑定到新目的地。

禁用携密认证请求的自动跨 origin redirect；默认不跟随 redirect。批准的同 origin 重定向也必须重新做目标校验与签名，不能复用旧签名。媒体下载/外部 URL 不自动继承 provider headers。TLS/代理属于 transport 实例，不能由单次 model 参数偷偷改写。

secret 值不可进入 descriptor、Debug、错误、trace、metrics label、settings snapshot 或录制；URL query 中的 token/签名也要按字段脱敏。日志只能记录 credential source 类别、namespace 的非敏感标识、generation 和结果码。调试不得 dump 原始 headers。录制 body 仍可能包含用户敏感数据，必须遵守录制单独的启用与保留策略，不能用“认证已脱敏”宣称整个文件无敏感信息。

## 4. 刷新所有权与并发（G3，明确后置）

### 4.1 当前/目标切换保留的行为

RFC-0018 的 `codex_refresh` 保持无状态一次调用：宿主提供 refresh token、持久化新 token 并重建 provider；401 映射 TokenExpired，包不自动刷新/重试。原始 refresh token 不自动重试，避免单次轮换 token 已消费而结果丢失时重复消费。Vertex 默认 ADC 与显式 token 路径分开；显式 token 无 refresher 时返回过期错误，不伪造刷新成功。

因此 #175 原方案的“统一 explicit→env→store + 默认 store + configure op + 自动刷新一次”不能作为 G1 的完成条件。既有原生包求值规则优先，通用 store 不得偷偷插进每个包的 env fallback。

### 4.2 可选宿主协调器的完整契约

若维护者批准 G3，实现一个显式注入、默认关闭的 credential coordinator；下列是提案，不是现有 API：

- `CredentialKey` = namespace + registration instance + mode + audience；`CredentialSnapshot` = opaque secret + generation + 可选 expires_at。store 用 `get(key)` 与 `compare_exchange(key, expected_generation, new_snapshot)`；禁止盲目 set 覆盖更新。
- coordinator 从宿主明确选择的 source 读取；只有“配置文档明确使用协调器”的路径才采用 explicit→声明 env→该 key 的 store。显式空值、无效值是错误而非 fallback 条件；auth none 不调用协调器。
- 每个 key/generation 最多一个 refresh flight。等待者共享结果；进入 flight 后二次读 store，若 generation 已变化则复用新快照。互斥锁只保护 flight/版本状态，不能跨网络 await 持有。
- 第一个取消的调用不得取消其他等待者的刷新。每个 waiter 的 cancel 仅退出该 waiter；refresh flight 由 coordinator 的独立有界 timeout 与 shutdown token 管理。shutdown 等待或取消 flight 并产生一个终态，不无限存活。
- refresh 结果须校验 audience、类型与有效期，然后 CAS 提交。CAS 失败时丢弃旧结果，重新读取新版本；不能把较老 token 写回。宿主负责持久化；库默认只内存，不落盘。
- 两个进程共享会轮换的 refresh token 时，内存 single-flight 不足以保护它。外部 store 必须提供跨进程互斥/租约与原子更新；未提供则不允许共享自动刷新，返回明确配置错误。
- 同一 operation 对同一 credential generation 最多触发一次“过期后刷新”；后续仍失败则 TokenExpired，禁止 refresh→retry 无限循环。刷新不能恢复已经向调用方输出的流，也不能重提交可能计费的非幂等请求。
- 刷新失败保留结构化原因。明确被服务端拒绝的 generation 标记失效，不再次提交；暂时性网络错误不清空其他 generation。为失败 flight 设置短暂、可配置退避，防止每个新 waiter 立即重试；无效凭证的重试须等宿主替换 generation。

只有可以确认请求未执行或契约支持幂等重放时，调用运行时才在总预算内执行刷新后的一个新 attempt。401 并非所有 provider 都能证明“请求没被执行”；此判断须包声明并由 fixture 覆盖。G3 不能额外创造独立 retry budget。

## 5. Public L0 native passthrough（G2）

### 5.1 公开形态与权限

建议由 registration 的 native HTTP extension 创建 `NativeEndpoint`，不增加 V4 model trait 方法。接口概念为：

```rust
struct NativeRequest {
    method: Method,
    relative_path_and_query: String,
    headers: Headers,
    body: Bytes,
    retry: NativeRetryPolicy,
}
// 传输类型复用 provider-utils；abort/timeout/observer 来自 operation context。
async fn native_request(endpoint: NativeEndpoint, request: NativeRequest,
                        context: OperationContext) -> Result<NativeResponse, NativeError>;
// NativeResponse: status, headers, effective_url, body: Stream<Result<Bytes, NativeError>>
```

`NativeEndpoint` 绑定已配置的 base URL、认证作用域和 transport；不是全局任意 URL 的携密 HTTP client。输入只能是相对 endpoint path/query；拒绝 scheme、authority、user-info、fragment、协议相对 URL、路径穿越及能改变 origin 的编码形式。构造后再比较 scheme/host/effective port；私有内网服务可由受信宿主配置，不向不可信请求开放任意目的地选择。

base URL path prefix 的行为必须唯一：尾斜杠规范化后追加相对路径，不把 `/v1/` 意外重置为 origin 根。拒绝 `..`、反斜线以及解码后同义路径穿越；query 原样保留合法编码，不 JSON 重排、不重新排序。API 文档须给 `/v1` + `responses?x=1&x=2` 等 golden 样例。

认证头集合由 endpoint 声明并由认证层拥有；G2 的调用 headers 不得覆盖这些头，冲突直接返回 InvalidHeader。无鉴权 endpoint 允许宿主明确传普通 headers；不因此提供 credential env 读取。协议 body 转换器不运行，但必要认证/签名仍运行。G2 不接收 providerOptions、不自动加 model、stream 或 store 字段。

### 5.2 “原样”的可验证定义

保证请求 body 字节与 response body 拼接字节在 observed leaf 边界相同，不 parse/re-encode JSON、不解释 SSE/NDJSON、不将非 2xx 转成模型错误。HTTP status、重复 header 值、body 均返回；传输错误、非法请求、取消、超时用 NativeError 表达。

不保证 TCP/TLS 分片、HTTP header 大小写与排序、chunked framing 或每次读到的 chunk 边界一致。默认 L0 leaf 禁用透明 body 解压；若某注入 leaf 转换了 body，必须声明它的语义，不能宣传物理线上保真。Auth/signing 增加的头与允许的 URL 编码校验是显式契约，不算暗中模型转换。

单次 `Fetch` 只执行一次 exchange；G2 调用运行时默认 `retry=never`。可选 safe retry 仅在 response head 尚未交给调用方时执行，限网络建立失败或显式列出的状态、同一 body bytes 且总预算未耗尽；不得默认重试 POST。幂等键必须由调用方提供且 endpoint 声明支持。响应头一旦返回，body 中途失败直接失败，不能拼接下一次请求的流。

### 5.3 取消、流所有权与录制

操作 context 在 credential 求值前建立，abort/timeout 覆盖设置求值、等凭证、发送、等待响应和消费 body；返回 response head 不结束 operation。流持有 operation/transport guard，直到 EOF、失败、取消或 Drop。drop body 关闭/取消底层读取；不能为“补录制”后台继续消费完整响应。

优先规则：若开始前已取消，不求值凭证、不联网；运行中以第一个已提交终态为准。每个 exchange 恰好一次 `ExchangeFinished(Completed | Aborted | Failed)`；取消与 EOF 竞争不双终结。取消后不得再向消费者排入新字节，已交付字节不撤回。错误是一个终态，之后读取 EOF，不人工注入协议 Finish/SSE 事件。

流拉取提供背压，不以无界队列缓存整个响应；内部最大缓存以 bytes 配置且有非零有限上限。慢 observer 不可使无限内存增长：录制写入失败/超限须按 devtools 策略显式报告 incomplete，不能静默声称完整 cassette。

复用 #200 的统一 observed leaf：auth/signing → observed leaf → 默认/注入 fetch。每个 attempt/exchange 独立 ID，L0 与 model 路径只记录一次；记录的是 leaf 边界，不能重复 helper 和 leaf 数据。离线回放显式注入固定凭证 recipe，禁止真实 env/ADC/刷新网络；不能让 auth 通过识别 replay 类型自动禁用自身。完整 replay 匹配与存储 schema 由治理 RFC 承担。

G2 的绑定只传 Rust 内建能力句柄；FFI/stdio 二进制分帧、终止及句柄释放引用 RFC-0039，不在本 RFC 发明另一套回调 ABI。任一绑定尚不能表达 bytes/abort 时，不得宣称该绑定已支持 public L0。

## 6. B1–B9 的承接矩阵

下表“替代”均指 **superseded-pending-#200**；#200 未合并前不改写 master 的历史事实，也不提前关闭旧 issue。

| 节点 | 保留的用户结果 | 目标实现与删除门 |
|---|---|---|
| B1 | 正确选包/方法/协议，来源完整 | registration + descriptor + 包工厂替代通用 from_resolved；不加 ProviderRecord 配置重建。constructor、默认实例、命名方法、config 和各 binding 调用点一起切换 |
| B2 | 本地服务无假 key；URL env 与地区参数不丢 | preset 生成明确工厂，auth none、base URL env、模板规则就位后删 wrapper；不要保留 PLACEHOLDER_API_KEY |
| B3 | 数据适配可 serde，避免隐式厂商判断 | compat/preset 独立可序列化 schema；共享协议内禁止 name/URL 子串分支。包原生实现允许自身厂商名，不采用误伤所有包的文本 grep |
| B4 | Mistral content arrays、tool_choice any、model_length 等不丢 | 依 #200 保留原生包的 SDK 行为；只在实际等价的内部 helper 去重。原“全塞 OpenAI profile 后删除”的方案不执行；测试重新定向后才删旧实现 |
| B5 | xAI 2xx 内错误、citations、usage、reasoning 元数据有归属 | 遵 #200 的 xAI Responses LM 目标，不重建已撤销的 chat 外观。旧 chat 能力变化要写明，并为 Responses 和公共 helper 做独立回归，不以行数证明等价 |
| B6 | Responses 各包 headers/namespace/工具/媒体不丢 | 共享真实等价 reducer/transport；保留 HF itemId、response.created、mcp_call、媒体 sniffing，以及 xAI reasoning_text.delta/response.done/provider-tool adapter。通用 body_overrides 被 #200 撤销；Codex store:false 是包内规则，不能借恢复 body_overrides 实现 |
| B7 | Vertex regional host、googleVertex namespace、错误终结正确 | 共享适合的底层 core，失败响应处理按包注入；不得把 Vertex 切成 Google express。对“error 后是否 Finish”的差异用目标 V4 终结契约明示更新，而非不经说明改 cassette |
| B8 | header、AWS credential、2xx JSON error 去重 | 只提炼语义相同的原语；image multipart 不强加 JSON Content-Type；错误保留 provider 上下文且不泄密 |
| B9 | 单模态配置壳缩减且所有模态可达 | 机会性工作，无完成日期；先满足 auth none/base URL env、descriptor 支持相应模态；SearXNG 仅 URL 场景单测。未证明等价的壳继续保留 |

#166 的修正均有去向：顺序依赖由原子切换闭包承接；responses body override 缺口由包内目标行为承接；xAI/Mistral 特性通过原生包测试承接；地区 host 与 serde profile 在 §2；FFI/Node/Python/Web/CLI 直接调用点纳入 §7；行数收益不作验收，不复述未经本实现测量的估算。

## 7. 迁移与实施边界

### 7.1 文档先行

先解决 #200 状态/编号冲突，维护一张“现行条款→新决策→证据”表。本 RFC amend RFC-0017 的通用配置/override、RFC-0020 的全局 overlay；保留 RFC-0018 OAuth 与持久化归属。#174 需要修订成包内鉴权组合与共享原语；#175 分成 G1 的现有行为保真和 G3 的可选新能力，未经批准不得打勾。

RFC-0032 若恢复，应以本 RFC 和 #200 为目标修订，而不是重新落地已准备替代的单一 protocol registry 架构。RFC-0033 仍维护 registry 数据来源、探测与更新流程，不承担运行时 token 生命周期。

### 7.2 原子架构切换（G1）

可按目录分工与分提交评审，但最终在同一集成闭包完成 provider-utils、包工厂、aimux registry、devtools、FFI、全部绑定、CLI/Web 的构造调用点替换后才进入 master。不保留兼容 shim、旧配置读取、旧 cassette schema 读取或 deprecated 别名；历史资料留 git 历史，不静默迁移用户凭证。

迁移说明是“用户必须重新构造/配置”的操作指南，不是库内数据迁移承诺。说明新 package/settings、显式 registry/method、错误时机、移除的 bodyOverrides/旧 chat 入口、重新录制条件。不得自动导入旧 token 文件，导出配置不得包含 resolved secrets。

回退单位为完整发布/完整集成 commit，不在现场混装旧 bindings 与新 core。需用版本/manifest 检查明确拒绝错配，而不是给出偶然可用的承诺。G2/G3 不阻塞 G1；各自获批后另做功能实现与对应绑定覆盖。

## 8. 可执行验收矩阵

验收记录必须包含基线/目标 SHA、fixture、预期、结果及失败原因。以下全部是待实现的门，不是本 RFC 已运行测试的声明。

| ID | 输入与故障 | 必须观察到的结果 | 门 |
|---|---|---|---|
| C01 | default vs explicit factory；missing/empty key；Future vs AsyncFn 计数器 | 错误发生阶段符合逐包表；空串不 fallback；求值次数准确 | G1 |
| C02 | 同 package 两个 registry 实例；model ID 含冒号；显式 chat/responses | reference 与 namespace 不串；方法不丢；无 Gateway fallback | G1 |
| C03 | wrap、registry child、直接 model | 扩展/discovery 可达；直接 model 不猜身份 | G1 |
| C04 | global/eu/us/一般 location；缺失/非法 region；本地 URL env | host golden 一致；非法参数早失败；无未展开占位符 | G1 |
| A01 | Bearer、api-key、x-api-key、x-goog-api-key、none | 最终头恰当，无重复头/占位 key；none 无秘密读取 | G1 |
| A02 | SigV4 JSON/multipart/query；URL/body 在签名前变化 | 对最终字节验签；变更后的旧签名不得发送 | G1 |
| A03 | base URL 覆盖、跨 origin redirect、外部下载、任意 env 请求 | 旧凭证不传播；拒绝越权来源/目的地；没有秘密日志 | G1 |
| A04 | offline replay 启动时无真实 key/ADC | 固定 recipe 足够跑真实转换器；无网络凭证请求 | G1 |
| B01 | 按 §6 每包/每模态 streaming 与 non-streaming fixture | 特殊字段、namespace、错误与终结行为有独立断言；差异逐项批准 | G1 |
| B02 | FFI/Node/Python/Web/CLI 原直接构造调用点清单 | 无旧构造类型/旧 config_snapshot 重建引用；全 workspace/绑定测试通过 | G1 |
| N01 | 任意二进制 body、非 JSON 400、SSE 字节、重复 query/header | body hash 相同，非 2xx 仍返回 raw response；不解析 SSE | G2 |
| N02 | absolute URL、//host、../、编码穿越、credential header 覆盖 | 发送前失败；网络记录为零 | G2 |
| N03 | 预取消、等凭证取消、header 前取消、body 中止、Drop、EOF 竞争 | 无额外网络/输出；唯一终态；资源释放；无合成协议 Finish | G2 |
| N04 | slow consumer、无限流、recording sink 失败 | 有界背压；取消可达；incomplete 可见，无伪完整录制 | G2 |
| N05 | POST 连接故障；GET 安全重试；已返回 head 后 body 失败 | POST 默认不重放；共享预算；流不拼接 | G2 |
| R01 | 多个并发请求同时遇同 generation 过期 | 一次 refresh，CAS 一次成功；waiter 返回同代结果 | G3 |
| R02 | 首 waiter 取消、全部取消、shutdown、refresh timeout | 不杀其他 waiter；refresh 有界；无锁跨 await | G3 |
| R03 | 刷新期间宿主替换 token；多进程无锁 store | 新代不被覆盖；不安全共享配置拒绝 | G3 |
| R04 | refresh 结果丢失/401 连续失败/流已输出 | 不盲重试轮换 token；最多一次刷新；不重放已输出流 | G3 |

安全性测试使用 sentinel secrets，扫描错误/录制/trace 中的原始值和 URL 编码形式。回归不以删 cassette、减少测试或统一修改 snapshot 获得通过；原始素材保留，目标 schema 的重录与预期差异说明由治理契约执行。性能/体积门引用专门门禁 RFC，本 RFC 不作未测量的吞吐、行数或产物大小承诺。

## 9. 备选方案与待评审点

| 备选 | 优点 | 不选为目标的原因 |
|---|---|---|
| 继续 protocol enum + from_resolved + 构造器 shim | 与原 #166 逐 PR 计划一致 | 与 #200 原子闭包、私有 settings、原生包边界冲突；在 #200 决策失败时重新评估，不同时实现两套 |
| 全部鉴权经 apply_auth(HeaderMap) | 看起来只有一个入口 | 无最终 URL/body，签名不完整；包的 headers/fetch 求值不等价 |
| 全局 CredentialStore + 无条件自动刷新 | 调用方配置简单 | 破坏包错误时机、隔离与 token 所有权；非幂等操作可重复计费 |
| public L0 接任意绝对 URL 并复用 key | 表面灵活 | endpoint 改写成为凭证外传能力；采用受限 NativeEndpoint |
| 所有 HTTP 非 2xx 抛统一模型错误 | 上层接口统一 | 丢失 raw endpoint 的业务响应与字节保真；L0 返回 status/body |

评审需显式决定：G2 是否接受作为 #200 之后的新能力；G3 是否需要库内协调器或仅保留宿主指导；NativeEndpoint 的 endpoint policy 配置是否足够覆盖目标私有服务。无答复不视为批准，不以此建立自动刷新/跨语言回调的发布承诺。

## 10. 核对来源

- [当前 master provider.rs](https://github.com/arcships/aimux/blob/72a37b5058ecd620d75dfe66bee34ef51f89294a/aimux-providers/src/provider.rs)：内建 registry 和现有 profile。
- [#200 目标文本（固定 head）](https://github.com/arcships/aimux/blob/d8c3d15a9b84eaa176b64fe9c5f84d678498634d/rfc/0036-aisdk-architecture-alignment.md)：§0.4、第一部分 §3.2–3.4、§4.3–4.6、§6–9；链接指向未合并提案，不是当前规范。
- [#166 B 轨](https://github.com/arcships/aimux/issues/166)：B1–B9 与七项修正；[#174](https://github.com/arcships/aimux/issues/174)、[#175](https://github.com/arcships/aimux/issues/175)：旧 auth 分层提案。
- 本文为设计产物，仅核对文档与源代码，未执行实现、构建、协议测试或线上请求。
