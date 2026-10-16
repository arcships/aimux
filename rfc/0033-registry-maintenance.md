# RFC-0033: Registry 维护报告与无凭证探测

> **Status**: Draft
>
> **Date**: 2026-10-03
>
> **Scope**: provider 元数据的上游差异报告、无凭证可达性探测、人工偏差记录与定时报告。只定义维护工具和验收，不改变 provider 构造、鉴权、协议分发或运行时 API。
>
> **Related**: [#170](https://github.com/arcships/aimux/issues/170)、[#171](https://github.com/arcships/aimux/issues/171)、[#166](https://github.com/arcships/aimux/issues/166)、[ROADMAP](../ROADMAP.md)
>
> **核验基线**: master `a8e3a996`；V4 层源码位置与行号以 [#256](https://github.com/arcships/aimux/pull/256) 的提交 `65ca6be7` 为准（#249–#261 按顺序叠放，该提交包含全部各层）。[#200](https://github.com/arcships/aimux/pull/200) 已于 `d8fb4ac5` 合入为 [docs/aisdk-architecture-alignment.md](../docs/aisdk-architecture-alignment.md)，其 §0.4 已记录四项决定，本 RFC 不重新裁定这些决定。

## 1. 问题与边界

当前的人工修正必须保留，同时需要持续发现上游端点、key 变量名和 provider 列表的变化。上游数据只在核对文件保护下更新既有 provider 的地址与 key 变量名，不做无条件覆盖；探测也不能证明某个 provider 支持聊天、工具调用或特定模型。

本 RFC 使用已被 #170 / #171 引用的 0033 编号，补齐尚未入库的规范；它不是已有草稿的原文恢复。以下区分既有要求和新提案：

| 类别 | 内容 |
|---|---|
| 既有要求 | #170：每周差异、每月探测；定时任务不阻塞合并；支持人工 `note` / `status` |
| 既有要求 | #171：逐项审查端点差异；新增或修改行仍需相应 cassette；不能因上游存在条目就直接接入 |
| 已记录的架构决定 | #200 §0.4：一次性切换、不引入多步运行时、采用本地 registry、宿主回调后置 |
| 本 RFC 提案 | 规范化比较输入、结构化报告、精确偏差匹配、探测分类、安全边界与离线验收 |
| 本 RFC 提案（修订 #170 “只报告、不重写”） | sync 工具按上游直接更新既有 provider 的地址与 key 变量名；人工修正存于核对文件、更新时不被覆盖；上游新 provider 仍只进报告（§3.2、§3.3） |

不在范围内：原 RFC-0032 节点的协议合并（目标由 RFC-0040 承接）、凭证解析、模型目录运行时下载、自动新增 provider、地址与 key 变量名之外的字段写回（display、profile、auth、params 等）、带 key 的能力探测、cassette 重录、CI 性能阈值、ops 或 L0 API。

### 1.1 与 #200 的关系

#200 第一部分 §3.3 / §6.5 规定 preset 输入、descriptor 与 manifest 的单一生成链。维护报告应读取该链的规范输入或可追溯投影，不能成为第二个源码生成器。

#200 已合并（`d8fb4ac5`），其第一部分 §3.3 / §6.5 把 preset 输入、descriptor 与 manifest 定为唯一生成链，registry 列保留为 descriptor 的数据来源；preset schema（`provider_registry.json` 的 V4 形态）随 V4 层（#249–#261）进入 master。维护工具直接以 preset schema 为输入，不存在“等 #200 合入”的迁移条件；旧 `RegistryEntry` 适配器随 V4 层删除后不再维护。维护视图是工具内部结构，不是对外 API 或旧配置兼容层。本 RFC 自身不要求删除 ABI、运行时 API、overlay 或 profile，也不启动完整架构切换；它只要求已经发生的输入变化不产生第二个数据真相。

本 RFC 不要求先实施旧的 `from_resolved` 方案。原 0032 缺失规范的行为要求由 [RFC-0040](0040-provider-auth.md) 在目标架构下承接；本稿不再补写同题旧架构 RFC。

## 2. 已核验的现状

| 证据 | 现状与影响 |
|---|---|
| V4 层的 `preset.rs`（#249–#261；位置以 #256 提交 `65ca6be7` 为准） | `provider.rs` 的 `RegistryEntry` 已整个删除；`provider_registry.json` 由 `RegistryRow` / `RegistryParam` 解析，均带 `#[serde(deny_unknown_fields)]`，以 `include_str!` 嵌入，任一行解析失败直接 panic。列表因此是严格 schema：运行时 JSON 只放运行时要用的数据，多余字段直接报错，维护字段（`note` / `status`）只能存放在核对文件（§3.3） |
| [`provider_registry.json`](../aimux-providers/src/provider_registry.json) | 282 行；字段含 `name` / `display` / `base_url` / `env_var` / `profile`，V4 形态新增 `auth`、`base_url_env`、`params[].env`（例如 bedrock_mantle 的模板参数 env 为 `BEDROCK_MANTLE_REGION` / `AWS_REGION`）；21 行 `auth:"none"` 中 6 行仍带 `env_var`（ollama、vllm、sglang、llamacpp、localai、laya），含义是“有就带上的可选 key”。仍存在 localhost、127.0.0.1、`{...}`、`${...}`、`<...>` 端点，不能直接批量发请求 |
| preset 加载校验与端点回归测试 | 名称、格式、占位符检查在 preset 加载校验中；历史端点修正的守护测试已由 [#254](https://github.com/arcships/aimux/pull/254) 恢复为 `registry_fixed_base_urls_are_correct`（同时检查 descriptor 与实际请求 URL；已实现、待合并）。本 RFC 采用与位置无关的要求：历史端点修正在 schema 变更后仍由测试固定，新增测试不能把修正退回上游值 |
| [`gen_providers_doc.py`](../scripts/gen_providers_doc.py) | 仍是 JSON 消费方；sync 写回不得破坏其生成输出，不另建一套 provider 生成链。`gen_provider_names.py` 将删除（provider 名统一为字符串），不作为消费方 |
| [`extract_litellm_bases.py`](../scripts/extract_litellm_bases.py)、[`scan_litellm_urls.py`](../scripts/scan_litellm_urls.py) | 扫描本地 `reference/litellm`，有硬编码集合和启发式提取；不是完整可靠的协议判定器 |
| [`.github/workflows/ci.yml`](../.github/workflows/ci.yml) | 当前 PR / push 检查与 live report 分开设计；仓库还没有本 RFC 的两个 registry 脚本 |

#170 的 97 个重叠、27 个端点分歧、116 个缺失 provider 是 2026-09-04 的历史观察，不是当前网络事实。没有对应上游快照和映射时，不能声称可精确复现这些数字。

## 3. 输入与差异契约

### 3.1 内部维护视图

适配器输出以下结构；字段名为本 RFC 提案：

```text
MaintenanceEntry {
  id: string,                  // preset name；未来是明确映射的 preset/package ID
  source_ref: string,          // 仓库相对路径及条目标识
  endpoint: string | null,     // 未解析模板原文；不求值
  key_env_names: string[],     // 凭证 env（env_var）名称，不读取值
  config_env_names: string[],  // 地址参数 env（base_url_env、params[].env）名称，不读取值
  note: string | null,         // 来自核对文件（§3.3），不进入 registry 列表
  status: "unreachable" | null // 来自核对文件（§3.3），不进入 registry 列表
}
```

`id` 必须唯一、非空；变量名称去重并排序；未知字段与不支持的输入 schema 应有明确诊断，不能把解析失败当作空列表。列表中的环境变量分两类：`env_var` 是 key 变量；`base_url_env` 与 `params[].env` 是地址参数（例如区域），只用于展示，不与上游 key 比较。`auth:"none"` 且没有 `env_var` 视为显式空集（没有 key），带 `env_var` 的记为可选凭证；两者都不是 unknown。动态 endpoint 无静态值时使用 null 并报告不可比较，不猜测实际地址。

- models.dev 使用显式版本适配器读取 `https://models.dev/api.json`；快照内需验证 provider ID、endpoint、env 字段形状。上游新增无关字段可忽略，但被比较字段改变类型必须报 schema 错误。
- LiteLLM 为可选本地 checkout；只读静态文件，不 import、安装或执行上游代码。记录 commit（可取得时）及实际读取内容的哈希。启发式提取结果标为 candidate，不能与 models.dev 相互覆盖。
- 同名不一定代表同一协议。默认仅做精确 ID 匹配；别名通过入库的显式映射表处理，并附依据。禁止下划线、连字符、大小写或相似度猜测。映射目标歧义必须报错。
- “missing”仅指上游有条目、维护视图无对应条目，不等于 aimux 整个仓库没有原生实现。每行显示比较范围，交由 #171 核对原生模块、协议与 cassette 后决定。

### 3.2 比较规则

输出三张主要表：上游缺失项、`base_url` 差异、key 变量名差异。另附不可比较、人工偏差和输入错误区域。

比较结果同时驱动写回（`--apply`，见 §5）：对列表中既有 provider，地址（`base_url`）与 key 变量名（`env_var`）按上游（models.dev 为主，LiteLLM 可选）更新；核对文件中记录的人工修正以核对文件为准，不被覆盖。上游新出现的 provider 不自动加入列表，仅进入报告，接入仍按 #171。display、profile、auth、params 等其他字段不由 sync 写回。

端点原文和比较值仅存于进程内存。比较值仅规范化 scheme / host 大小写与默认端口；不改 path、query、尾斜线，不添加 `/v1`，不把原生端点和兼容端点视作等价。无法解析或有模板时只在内存中比较并标明类型，不能网络展开。key 变量（`env_var`）按名称集合比较，不读取或推测变量值；`base_url_env` 与 `params[].env` 属地址参数，不与上游 key 比较。上游未提供某字段是 unknown，不等于空字符串或空集合。

**全局脱敏规则**适用于 sync、probe、源 URL、重定向、工具异常、跳过或未探测的条目，以及全部 JSON / Markdown / 日志 / artifact / issue 评论；不得靠“未发请求”豁免：

- 不依赖 `token` / `key` 等参数名猜测秘密。所有 userinfo、整个 query（含参数名与值）和 fragment 均视为可能含凭证，输出前分别替换为固定 `[REDACTED]`；报告不输出秘密的长度、哈希或编码值。此类 endpoint 标为 `invalid-sensitive-url`，不探测、不生成持久化精确例外。
- 安全解析后的表示由规范化 scheme / host / port、path 和上述固定标记构成，另给出 `redacted_components` 的固定枚举列表。无法安全解析的 URL 整体输出 `[REDACTED_URL]`，不回显 parser 原始错误。path 也发现凭证或无法安全划分组成部分时同样整体遮盖。
- 比较结果单独输出 `equal` / `different` / `unknown`。即使两侧脱敏表示相同，内存比较发现 query 不同也可报告 `different`；不得为解释差异暴露隐藏值。敏感输入不得复制到核对文件、快照 artifact 或调试转储。
- 例：`https://user:pass@example.invalid/v1?custom=secret#private` 输出 `https://[REDACTED]@example.invalid/v1?[REDACTED]#[REDACTED]`。query 改成任何其他参数名和值后仍使用同一表示；结果字段负责说明是否发生变化。

只在没有被遮盖内容时保留完整 URL 展示；任意 query 均不允许用于本 RFC 的 live probe。脱敏发生在构造报告对象之前，所有输出视图只能接收该对象，不能重新读取原值。

不同上游的观察分别列出；不投票选出“正确地址”。报告不修改 JSON、不改代码、不创建 provider，也不根据 diff 自动发探测请求。探测目标只来自受审查的本地输入和 allowlist。

### 3.3 核对文件：人工修正与状态

人工确认过、与上游不一致的字段值记录在独立的核对文件中，写明原因与来源链接。工具按上游更新列表时以核对文件为准，不覆盖其中记录的修正；CI 离线检查列表与“上游快照 + 核对文件”计算出的结果一致（§6）。核对文件即 §3.2 的写回例外来源，文件路径由实现 PR 确定并在脚本帮助中列明，不增加运行时配置职责。

- 人工修正按 `(id, upstream, field, local_value, upstream_value)` 精确匹配已确认偏差。只有精确匹配才标为 acknowledged；任何一侧数值变化，原条目失效并列入需复核项，复核数计入总待复核。核对文件之外的字段差异仍显示，不能隐藏新变化。
- `note: string` 与 `status: "unreachable"` 沿用 #170 语义，但存放在核对文件、按 id 关联进维护视图，不写入 registry 列表：列表只放运行时要用的数据，加载时严格校验，多余字段直接报错。`note` 必须为非空说明，可附官方来源链接；带 note 的行放入“人工偏差”区域，不计入三张主表的待处理总数，但仍显示所有字段差异。
- `status` 是人工维护的观察标记，不使 runtime 禁用 provider，也不免除后续探测。定时失败不自动写入 `status`；恢复成功只在报告提示人工复核。

这比 #170 “有 note 就不报告”更严格，也是对 #170 “只报告、不重写 registry”的修订（sync 直接写回地址与 key 变量名，见 §3.2）。采用后同步 #170 / #171 的验收措辞，避免把整行永久排除。

## 4. 无凭证探测契约

### 4.1 目标选择与请求

默认任务只允许探测受审查的公共 endpoint allowlist。allowlist 记录 provider ID、准确 origin、获准的基路径及来源；初次启用由实现 PR 审查具体列表，不能把上游新 URL 自动放行。端点发生变化后标为 blocked-unapproved，待人工更新 allowlist。

静态、公共且获准的 base URL 使用 `base_url.rstrip('/') + '/models'` 构造 GET，保留 `/v1` 等基路径；这只是兼容端点的可达性观察。对于模板、无法安全组合的 query/fragment 或非 models 路径，不猜测参数，报告 skipped 与原因。未来 descriptor 声明专用探测路径时，同样必须通过 allowlist 审查。

探测进程不得加载 provider 工厂、应用凭证配置、读取 API key 环境变量、ADC、keychain、云实例身份或 `.netrc`，不得复用 provider HTTP client。只发固定的无凭证请求头；禁用 cookie jar、自动认证、隐式代理环境变量和自动重试。网络配置参数通过显式 CLI 参数传递，不从用户环境推导。每个目标一次尝试；再次观察属于后续显式运行。

超时、并发、总运行预算、响应头/体积上限和最大重定向次数必须为有限正值（重定向可为零），由实现 PR 说明并固定默认值、接受合法 CLI 覆盖。此处不宣称未经测量的数值已经获准或达标。到达上限后产生 timeout / limit-exceeded / not-run，不能伪装为 provider 不可达。

### 4.2 SSRF 与数据边界

- 仅允许 HTTP(S)；拒绝 userinfo、任意 query / fragment、Unix socket、file URL 以及未知 scheme；拒绝或跳过结果也遵守 §3.2 的全局脱敏规则。拒绝 localhost、loopback、私网、link-local、云 metadata、保留/非全局地址，覆盖 IPv4、IPv6 和 IPv4-mapped IPv6。
- 在解析后、建立连接前检查全部 DNS 结果；混合公共与非公共地址同样拒绝。连接使用已校验地址并保留正确 Host / TLS SNI，不允许验证后再次 DNS 解析绕过检查。TLS 校验保持启用。
- 每次重定向都重新检查 scheme、目标 allowlist、地址和 DNS；不把跨 origin 跳转自动视作获准，不接受 HTTPS 降级。无法提供逐跳校验的 HTTP 库必须禁用重定向并报告。
- allowlist 不豁免私网限制。本 RFC 的定时任务不探测本地 Ollama 等服务；内部环境探测需要单独范围，不提供绕过上述限制的开关。
- 响应体不作为报告输入，不保存原始 body、Set-Cookie 或任意响应头。可记录 HTTP status、获准目标、耗时、受控错误分类及经过验证和脱敏的最终目标。遇到限流只记录结果，不高频重试。
- 输入和响应均是不可信数据；报告转义 Markdown/HTML 与控制字符，禁止触发 mention、嵌入任意脚本或把响应内容作为指令执行。错误日志不得回显带 userinfo、query 凭证或本机秘密的原始输入。

### 4.3 结果语义

| 观察 | 分类 | 可以得出的结论 |
|---|---|---|
| 2xx | reachable | 目标返回 HTTP 成功；不代表 API schema、模型或鉴权正确 |
| 401 / 403 | reachable-auth-or-policy | 有 HTTP 服务响应，可能是鉴权或 WAF；不证明 provider API 正常 |
| 404 / 405 | reachable-path-unsupported | 有 HTTP 响应，`/models` 路径可能不适用 |
| 429 | reachable-rate-limited | 有 HTTP 响应，本次限流 |
| 其他 4xx | reachable-client-error | 有 HTTP 响应，需解释路径或策略 |
| 5xx | server-error | 本次服务错误，需要报告；不据此永久禁用 |
| 3xx 未跟随 | redirect-unverified | 仅确认原目标响应，最终目标未验证 |
| DNS / TLS / 连接失败 / 超时 | 对应独立分类 | 本次无法完成观察；不能混为“provider 已下线” |
| 安全检查拒绝 / 模板 / 预算耗尽 | blocked / skipped / not-run | 未探测，不能计入不可达 |

安全错误与瞬时错误均保留为结构化结果，单个失败不丢掉其余目标结果。只有输入整体无法解析或输出无法写入时，报告任务无法完成。

## 5. CLI 与报告格式

以下为拟新增接口，不表示脚本已实现：

```sh
python scripts/sync_registry.py --report --source models-dev --snapshot INPUT.json --output REPORT.json
python scripts/sync_registry.py --apply --source models-dev --snapshot INPUT.json --output REPORT.json
python scripts/sync_registry.py --check --source models-dev --snapshot INPUT.json
python scripts/sync_registry.py --report --source models-dev --fetch --output REPORT.json
python scripts/sync_registry.py --report --source litellm --checkout PATH --output REPORT.json
python scripts/probe_registry.py --allowlist ALLOWLIST.json --output REPORT.json
```

`--apply` 按上游更新既有 provider 的地址与 key 变量名（核对文件条目除外），报告额外记录写回行（字段、旧值、新值，同样脱敏），供 #171 逐项审查与 cassette 配套；不自动新增 provider、不改其他字段。`--check` 离线校验列表与“上游快照 + 核对文件”计算出的地址与 key 变量名一致，供 CI 使用；不一致退出非零并输出差异。定时任务仍以 `--report` 产出差异，不自动提交或开 PR；列表更新由维护者运行 `--apply` 后提交，快照与核对文件同步入库。

`--snapshot` 和 `--fetch` 互斥，无隐式网络回退。离线输入失败就报告错误；live fetch 失败不得用旧缓存冒充本次成功。可展示旧报告，但必须标注 stale、原采集时间及本次失败原因。

报告 JSON 包含 `schema_version`、`mode`、仓库 commit、输入哈希、source URL/commit、适配器版本、比较策略、覆盖范围、结果行、分类计数和工具错误。live 另含采集时间与耗时。每行含稳定 ID、来源、字段、两侧脱敏表示、comparison 结果及 outcome；所有 URL 按 §3.2 脱敏。存在被遮盖内容时只输出脱敏输入的哈希，并标 `input_hash_kind: sanitized`，不输出原始敏感输入或其哈希；无敏感内容时标 `input_hash_kind: original`。JSON 为事实来源，Markdown 是同一对象的视图，不单独计算。

离线比较要求相同快照、仓库输入和工具版本产生字节一致的 JSON；排序按 `(upstream, id, field)`，不写当前时间、绝对路径或无序集合。live 网络时序不保证字节一致，不能拿它当 cassette。probe 的离线测试使用受控 transport 和 DNS fixture，不提供隐藏的实网 fallback。

退出码提案：`0` 表示报告完整生成（允许 diff、5xx、blocked 等观察结果），`1` 表示输入/schema/工具/输出错误导致不完整，`2` 表示 CLI 参数错误；`--check` 发现列表与计算结果不一致同样退出 `1`。部分上游获取失败时保留其他来源的报告，但整个任务退出 `1` 并标 partial；不能显示“无差异”。

## 6. CI、发布与权限

分开两种检查：

1. **静态验证与离线测试**：验证维护 schema、ID/别名唯一性、核对文件匹配、生成一致性和 fixture 行为；用入库的上游快照离线校验列表与“上游快照 + 核对文件”计算出的地址与 key 变量名一致（`--check`）。均可作为正常 PR 阻塞检查。不得发网络请求，不因外部服务状态影响合并。
2. **定时观察**：每周 sync、每月 probe，沿用 #170 的频率；具体 UTC 时间、并发组和跟踪 issue 在实现 PR 中配置。不是 required check，不能通过修改分支保护把 live 结果变成合并门禁。

“非阻塞”不等于吞掉错误：工具失败应保留失败 job 和 partial 报告，区别于发现正常差异。报告发布失败也要显式显示，不能声称已经送达。

提案选择固定 tracking issue，不采用自动编辑 registry 的 PR。该 issue 的 ID 必须先明确配置；未配置时只生成 artifact / job summary。相同输入哈希与结果不重复发评论；变化报告需带稳定 run 链接和内容指纹，发布重试先查指纹，防止重复。缺失上游与人工确认偏差分别统计。

网络采集 job 只读仓库、不给 provider secrets 或 issue 写权限；发布 job 只接收经过 schema 校验和转义的报告，不执行报告内容，用最小 issue 写权限发布。发布流程只在可信默认分支的 schedule / 明确手动运行启用，不能借 `pull_request_target` 执行 PR 代码。

## 7. 实施、迁移与依赖

本 RFC 不引入运行时 breaking change，不承诺版本、负责人或日期。

1. **规范与 fixture**：实现内部视图、输入适配器、报告 schema、别名/核对文件校验及离线测试；输入直接采用 V4 preset schema（`RegistryRow`，`#[serde(deny_unknown_fields)]`）。固定可合法入库的最小上游快照并记录来源；历史完整快照不可得时不要声称复现 27 / 116。
2. **sync 写回与一致性检查**：实现核对文件（人工修正 + `note` / `status`，按 id 索引）与 `--apply` / `--check` 模式；按上游更新既有 provider 的地址与 key 变量名，核对条目不被覆盖；CI 离线校验列表与“上游快照 + 核对文件”一致。registry 列表保持严格 schema，不新增维护字段。
3. **安全探测**：实现 allowlist、DNS/地址校验、有界读取、受控重定向与分类；通过 §8 的离线矩阵后才启用 live schedule。
4. **报告自动化**：配置跟踪 issue、发布权限、频率与去重，先人工触发核验完整报告，再开 schedule。维护工作不触发 provider API 调用或付费操作。
5. **#171 人工 triage**：依据可追溯报告逐项决定；provider 新增继续独立 PR、cassette 与协议验证，不由报告程序执行；sync 写回的行修改按 #171 配套 cassette 与验证，工具输出修改清单便于逐项审查。
6. **schema 演进**：preset schema 字段变化时同步适配器与核对文件键；ID 变化须明确迁移映射，不能静默丢失 `note` / `status`。ABI 或完整架构切换由 #200 的既有决策与实施管理，不由 RFC-0033 追加或触发。

两个旧 LiteLLM 脚本在新适配器覆盖其有效提取结果、有对照 fixture 且无消费者后才能删除；不能把启发式遗漏当作去重成果。

回退：关闭定时工作流或报告发布不影响运行时；保留已审查的 registry 修正。输入切换失败先修复新适配器，不自动退回已过时数据并报告成功。本 RFC 没有运行时配置迁移承诺。

## 8. 验收矩阵

以下全部是实施验收要求，不是本次文档已经运行的测试。

| 场景 | 必须断言 |
|---|---|
| 固定快照重复运行 | JSON 字节一致；顺序、主表与附表计数一致；输入文件哈希未变 |
| 缺失字段、错误类型、重复 ID、别名冲突 | 区分 unknown 与 invalid；不输出虚假的空差异 |
| endpoint 比较 | `/v1`、尾斜线、query、原生/兼容端点不被错误合并；模板不求值 |
| key 变量比较 | 仅 `env_var` 与上游比较；`base_url_env` / `params[].env` 不作 key 比较；`auth:"none"` 无 `env_var` 为显式空集、有 `env_var` 为可选凭证，均非 unknown；数组去重；不读取环境值；包含诱饵凭证的环境不会被访问或输出 |
| 核对文件 | 精确匹配才 acknowledged；值变化失效；核对文件之外的新差异仍显示并计入待复核 |
| sync 写回 | 既有 provider 的地址与 key 变量名按上游更新；核对条目不被覆盖；上游新 provider 只进报告、不写入列表；display/profile/auth/params 等其他字段不变 |
| 全局 URL 脱敏 | 离线 fixture 覆盖任意名称 query、userinfo、fragment、编码凭证、畸形 URL、上游源 URL、跳过目标与异常；全部 JSON/Markdown/日志/artifact/评论不出现诱饵秘密及其编码值；重复运行表示与排序一致；隐藏 query 改变仍报告 different，不泄漏原值或秘密哈希 |
| 本地与恶意目标 | localhost、私网、link-local、metadata、IPv6、mapped IPv4、混合 DNS、DNS rebinding、userinfo、凭证 query 均被拒绝或安全脱敏，未发出请求 |
| 重定向 | 每跳重新校验；跨 origin 未授权、私网跳转、降级、循环和超限均终止 |
| HTTP 分类 | §4.3 每类 fixture 都有预期；401/403 不等于能力验证；404 不等于下线 |
| 有界行为 | timeout、超大头/响应、取消、总预算耗尽都有结果；未运行不计失败，其他结果不丢失 |
| 无凭证网络 | 不加载 `.netrc`、代理环境、ADC 或 provider headers；只发送固定头，不记录 body/cookie |
| 部分上游失败 | 其余报告保留、标 partial、退出非零；旧缓存不伪装新观察 |
| 发布 | 未配置 issue 不发消息；相同指纹不重复；发布失败明确；恶意 Markdown/mention 转义 |
| CI 分离 | PR 静态检查完全离线；schedule 非 required check，失败不阻塞合并 |
| 离线一致性 | `--check` 用入库快照复算列表的地址与 key 变量名，与“上游快照 + 核对文件”一致；不一致时 CI 失败并输出差异 |
| 生成链切换 | preset schema 与未来 descriptor 链的等价元数据生成同一事实报告；不保留第二套生成链 |
| 运行时无回归 | 历史端点修正在 schema 变更后仍由测试固定；provider 构造、生成器检查及相关 cassette 继续通过 |

## 9. 替代方案与尚需确认的事项

- **无保护的自动同步覆盖 JSON**：拒绝。本 RFC 采用的写回以核对文件保护人工修正，且只更新既有 provider 的地址与 key 变量名；上游端点面向不同协议等顾虑仍由核对文件逐条记录理由，不做全字段覆盖或自动新增。
- **只靠带 key 的能力测试**：不采用。会扩大秘密与费用边界，也不能替代元数据来源比对；以后可另立治理规范。
- **有 note 就整行永久忽略**：不采用。无法发现后续新变化；采用可见附表和核对文件的精确条目。
- **把 live probe 做 PR required check**：拒绝。外部故障、WAF、DNS 和限流会使无关代码无法合并。
- **等待架构重构结束再维护数据**：无需等待。只读报告和安全边界可以独立落地，切换时只更换输入适配器。

合入前应确认的提案是：§3.2 写回范围与 §3.3 核对文件规则（含 `note` / `status` 存放位置）、§5 报告/退出码契约、固定 issue 发布方式。实现 PR 必须补齐实际上游 schema fixture、allowlist、有限资源默认值与跟踪 issue 配置；这些未指定项不影响本 RFC 的边界，也不能被实现悄悄省略。
