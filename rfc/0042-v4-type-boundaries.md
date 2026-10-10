# RFC-0042: V4 类型边界、工具参数原文与绑定生成

> **Status**: Draft
> **Date**: 2026-10-03
> **Scope**: ROADMAP 的 #185、L2 类型/metadata 补齐、旧 L2 选型节点及 D8 类型镜像；补充 PR #200 的实现与验收边界，不实现代码
> **Baseline**: master `72a37b5058ecd620d75dfe66bee34ef51f89294a`；目标设计参考未合并的 [PR #200](https://github.com/arcships/aimux/pull/200)，固定修订 `d8c3d15a9b84eaa176b64fe9c5f84d678498634d`
> **Related**: [#185](https://github.com/arcships/aimux/issues/185)、[#166](https://github.com/arcships/aimux/issues/166)、[PR #204](https://github.com/arcships/aimux/pull/204)、[RFC-0016](0016-align-with-aisdk.md)、[RFC-0035](0035-host-side-tool-call-repair.md)、[#192](https://github.com/arcships/aimux/pull/192)、[#95](https://github.com/arcships/aimux/issues/95)

## 1. 决策状态与优先关系

本文区分“master 已有行为”“PR #200 内已记录的目标决策”与“本文新增待批准设计”。PR #200 尚未合并，不能据此声称其 crate、wire 或生成器已经存在。

PR #200 第一部分及 §0.5 优先于其第二部分的未消解建议：

- 类型分为用户输入、V4 provider 输入、V4 provider 输出、core 输出四层
- V4 类型归 `aimux-provider`，用户 `ModelMessage` 归 `aimux-provider-utils`，core 输出归 `aimux`；不新建第三套 `aimux-protocol` 类型源
- JSON 字段 camelCase，内部 `type` 标签 kebab-case；字节以 base64 适配；缺失与 null 区分
- descriptor / manifest / codegen 是唯一生成链；第二部分允许继续 ts-rs 的建议不采纳
- 单步 core、不执行宿主工具、不引入宿主审批运行时；`steps` 在成功的单次生成中为一项。完整协议类型不等于实现相应运行能力
- 全依赖闭包一次性 breaking 切换，不保留旧 API、旧 wire codec 或转发兼容层
- 保留 RFC-0035 的宿主侧后处理修复；不引入跨 ABI 回调或暂停生成会话

旧 ROADMAP 的“独立 L2 数据模型选型”“先删 tracker 再换 ToolInput”“D8 serde→schema”“旧 ABI 共存至少一个 minor”与该目标存在冲突。本文以 **superseded-pending-#200** 标记前述旧设计方向：“pending”指 #200 尚未合并、关联文档与代码尚未同步，不是重开其已经记录的产品决定。按 #200 目标同步 ROADMAP/#185/#166，并另行批准本文新增冲突处置；不能仅凭本文独立 Draft 提前删除旧 ABI 或更换全部 wire。

### 1.1 本文新增、需要接受的决定

| 编号 | 提案 | 接受的后果 |
|---|---|---|
| T1 | 不在公共 wire 引入 `ToolInput::Raw/Parsed`；用不同层 DTO 的 `String` 与 JSON value 区分 | #185 的具体实现方案改写，保留消除混层的目标 |
| T2 | 删除 `aimux-stream` 公共 tracker 和第二套 `ToolCallStreamPart`；在 provider-utils 保留单一协议内部累积器 | 接受 #204 的归属方向，不接受“整套累积算法纯删除即完成”；见 §4 |
| T3 | 无法唯一归属的工具增量不静默丢弃，终止该模型流并报告 `InvalidResponseData` | 与 #204 `861f26e` 的 ambiguity-drop 行为不同，需维护者明确接受及差分测试 |
| T4 | 非法调用保留原文的结构化错误证据贯穿所有 wire/repair 路径 | 不通过 stringify(parsed input) 恢复原文 |
| T5 | D8 以 PR #200 的统一 manifest 为入口，并为自定义 serde 提供显式 codec 描述 | 禁止各语言另维护字段表、独立 schema 或第二条 TS 生成链 |

T2/T3 是待决选择，不能在 #185 或 #204 上直接写成已达成共识。若维护者选择彻底删除通用 tracker，则须在具体协议解析器实现同一状态与验收矩阵；不得同时留下两种实现或降低回归门槛。若要求精确保留上游 ambiguity-drop，应在本表登记偏离 T3，并至少把被忽略原事件纳入可检查诊断；在确定前阻塞实现合入。

## 2. 现状与真实缺口

基线源码显示：

| 面 | 已有 | 本次要解决 |
|---|---|---|
| 非流式 provider 输出 | `GenerateContent::ToolCall.input: String` | 保持原文语义，迁至 V4 层 |
| 解析入口 | `RawToolCall.input: String` | 统一 generate/stream 共用入口 |
| 流式事件 | `StreamPart::ToolCall.input: Value`，注释规定 provider 返回 Value::String、core 返回解析值 | 去掉同一类型携带两种语义；`generate.rs` 不再用 `raw_tool_input(Value)` 猜测 |
| 消息/结果 | `ContentPart`、`ToolCall` 等与 provider 数据复用 | 各层 DTO 分离，显式转换 |
| tracker | `aimux-stream/src/streaming_tool_call_tracker.rs` 公共导出；基线没有 core/provider 生产调用 | 决定删除与迁移的边界，避免保留无使用者的第二套事件协议 |
| 宿主修复 | #192、RFC-0035 已落地；`raw_tool_call_text` 从错误中找回原文 | 在类型/wire 切换时不退化 |
| metadata | `ProviderMetadata = Value`；Raw 变体已有，接线不完整 | 受约束的 namespace 对象及完整投影路径 |
| 生成 | `scripts/gen_ts_types.py` 基于 ts-rs，仅覆盖其现有 TS 产物 | 由 manifest 统一产出所有绑定 DTO/codec，并覆盖删除/多余文件 |

证据位置：`aimux-core/src/{result,stream_part,tool,content,parse_tool_call,generate,types}.rs`、`aimux-stream/src/lib.rs`、`scripts/gen_ts_types.py`、`contract-tests/fixtures/tool-call-repair.json`。以上是源码审阅，不代表运行了测试。

#204 `861f26eed8b1703fb7f78bbe7e83e0da7f1b87af` 已提出把 tracker 移到 provider-utils、输出当前 core StreamPart 并接 OpenAI。它不是 master 已有状态，也不是最终 V4 类型：本文采纳其“累积器在使用者附近”的思路后仍必须替换输出类型、处理歧义策略并接受全链测试。

## 3. 四层类型与工具参数

### 3.1 所有权

| 层 | 类型/职责 | 明确禁止 |
|---|---|---|
| 用户输入 | `ModelMessage`、用户 ToolChoice、用户文件输入；standardize/prepare/convert | 原样作为 V4 prompt 发给模型 |
| V4 输入 | `LanguageModelV4Prompt`、V4 CallOptions；tool-call input 是已解析 JSON | 塞入 core retry/session/录制配置；混用用户 ToolChoice |
| V4 输出 | `LanguageModelV4Content` / `LanguageModelV4StreamPart`；tool-call input 是 String | provider 解析/修复参数；携带 core invalid/error/toolMetadata |
| core 输出 | `ContentPart` / `TextStreamPart` / `StepResult`；tool input 是 JSON | 把字符串值自动识别成“尚未解析”；用 provider event 代替 core event |

转换只有明确方向：用户输入 → V4 prompt；provider V4 输出 → core parse/schema/repair → core content；core content → responseMessages → 用户输入。最后一路是对话语义转换，不保证重建原 provider 响应字节。

`LanguageModelV4StreamPart` 的 text/reasoning delta 字段是 `delta`；core `TextStreamPart` 对应字段是 `text`。tool-input-delta 两层都用 `delta`。不得仅重命名 enum 后复用全部 payload。

### 3.2 为什么不使用 untagged ToolInput

这两个值语义不同，但天真的 `#[serde(untagged)] Raw(String) / Parsed(Value)` 无法唯一解码：

- 原文为 JSON 字符串字面量 `"Tokyo"`，在 provider DTO 中 `input` 是含引号的文本
- 已解析 JSON 字符串 `Tokyo`，在 core DTO 中 `input` 是普通 JSON string

并且原文 `123` 是 String，解析后才是 number；原文 `null` 与解析后的 null 不等于字段缺失。根据 JSON 值类型猜测阶段不能解决这些问题。

本文不用额外 `status` tag 改造 SDK 的工具 wire。端点/事件根 schema 已确定层级，反序列化必须指定完整根类型；不能暴露 `decode_any_tool_call`。Rust 内部如需 `RawArguments(String)` newtype 防止误用，可以使用；它不是公共联合，也不改变 V4 String wire。

规范性测试向量：

| provider `input` 的文本内容 | core 解析值 | 约束 |
|---|---|---|
| `"Tokyo"` | string `Tokyo` | 再经过绑定往返仍是 string，不二次 JSON.parse |
| `"{\"x\":1}"` | string `{"x":1}` | 内容像对象也不得变对象 |
| `123` / `true` / `null` | number / bool / null | 支持 JSON primitive；schema 是否允许另行判断 |
| `{"a":1,"a":2}` | 解析器最终对象语义 | repair 错误必须保留含重复 key 的原文 |
| ` 1e2 ` | 数字值 | 不宣称解析后保留数字拼写和空白 |
| 空文本、`{"a":` | 非法调用及错误 | 不补成 `{}`，不自动 partial-JSON repair |

每行分别测试 provider DTO 和 core DTO 的 Rust→binding→Rust 往返。比较 provider 参数文本必须逐字节相等；core JSON 值按规范 JSON 语义比较。不得通过统一 canonicalize 把原文测试变成弱测试。

### 3.3 解析与非法调用

core 的一个入口执行工具查找、严格 JSON 解析、原有安全校验、schema 验证与至多一次 repair。generate 和 stream 共用它。内部 `ParseOutcome::Valid/Invalid` 可保证错误随非法结果存在；外部静态/动态联合按 #200 的 dynamic/invalid 字段编码，不新增非 SDK 状态标签。

- 合法字符串参数是合法的 parsed JSON；object-only 是具体 schema 的约束，不是类型系统默认约束
- 非法参数仍作为调用数据返回，不能因参数格式不对使整次生成失败
- provider-executed 的结果映射不执行宿主工具；`isError` 映射 core tool-error
- deferred provider result 找不到本轮 input 时省略 input，不能填 null 冒充“已知为 null”
- preliminary 结果可流式投递，但不进入最终持久内容；最终 content 按 #200 规则构建

## 4. 流式累积器与终结

### 4.1 边界

`aimux-stream` 只做通用字节/行/SSE/NDJSON 解码，不认识工具、工具 schema 或 V4 内容。OpenAI-chat 风格工具增量累积器归 provider-utils，供真正使用该 wire 的模型实现复用；不能把所有协议强行转成 OpenAI index。Anthropic block id、Responses item id 等由各协议按原生终结信号转换成 V4 事件。

累积器输出唯一的 `LanguageModelV4StreamPart`，不保留 `ToolCallStreamPart` 镜像。内部状态是每个请求的临时状态，不是跨 ABI operation session，不对绑定公开控制句柄。

### 4.2 状态与关联

每条调用内部记录：稳定内部序号、原 wire id 集、index 证据、name、输出 id、原文 buffer、metadata、是否发过 start/end/final、协议完成证据。使用 sparse map，不能用外部 index 直接扩展 Vec。

关联规则优先采用 #204 的 id/index/name 多证据解析，并以固定 fixtures 约束；不能回退到“index 唯一键”：

1. 已知 id 与 index/name 一致，路由到唯一调用
2. index 可复用；新 id 加明确开始证据可以建立新调用，不能覆盖旧 buffer
3. 缺 id 的 continuation 只有候选唯一才附加；全部证据缺失时仅在唯一未结束调用存在时附加
4. 同 id 但不同 index/name 的明确开始可能是另一调用，分配唯一输出 id，同时保留原 wire 证据
5. 有冲突或多个等价候选，执行 T3：返回 `InvalidResponseData`，不得把参数串接给任意一条工具
6. 缺 wire id 且具备合法开始证据时生成请求内唯一、非空输出 id；不得用空 id，生成冲突也要消解
7. 新调用缺 name、非法 index 或违反协议 type 约束，报协议错误；已存在调用的省略字段不自动清空已有值

“开始证据”不得仅为“buffer 已可解析成 JSON”：`1` 可以继续为 `12`，完整对象后也可能出现后续片段。沿用 #204 的结构化开始判定时必须证明它只用于关联，不用于提前完成；JSON 字符串和数字原语不能被排除出合法工具输入。

### 4.3 生命周期

| 输入/状态 | 输出与动作 |
|---|---|
| 首条可关联调用 | 发一次 tool-input-start，保存顺序 |
| 参数片段 | 原样按到达顺序拼接并发 delta；缺失不等于空文本，显式空片段按协议 fixture 处理 |
| 正常协议完成 | 对未终结调用依首次出现顺序发 end、tool-call；原文不重序列化 |
| 第二次 flush/重复结束 | 不再发 end/tool-call；幂等 |
| 已结束调用又收到增量 | 明确新调用证据才建新状态，否则协议错误 |
| 取消、不可恢复传输错误、消费者 drop | 停止读取、释放所有累积 buffer，不为未完成调用补发成功 ToolCall/Finish |
| EOF 缺少协议成功终结 | 按协议判为截断失败；若某协议规范明确允许 EOF 成功，适配器必须以独立 fixture 证明 |

Core 已交付的参数 delta 是 provider 原文，修复后不回写历史 delta。成功流的 core 生命周期只创建一项 step；取消/失败不伪造成功 finish-step。ABI 终结、取消竞态与 backpressure 由 ops/绑定契约负责；这里要求转换器在终结后 fused，不能继续调用 repair 或产生第二个 terminal。

实现必须提供每调用参数字节上限、并行调用数上限、总 buffer 上限的显式配置与固定默认值，在实施 PR 中附预算论证；不能随 provider index 放大内存。超过上限返回结构化错误并关闭流，禁止截断后当作有效调用。默认阈值属于实现验收待定项，不以本文虚构压测数据决定。

## 5. 保留 host-side repair 契约

#192 是已有能力，不是新回调功能。V4 原文与错误证据经过 camelCase 新 wire 后仍须满足 RFC-0035：

1. repair context 来自非法调用原件与同一次 prompt/options；无 tools 时 context 为 null，apply 操作拒绝
2. `InvalidToolInput.tool_input`、`NoSuchTool.tool_input` 和递归 `ToolCallRepair.original_error` 的新 wire 对应字段保留原文；不从已解析值反推引号、重复 key、数字拼写或空白
3. `repaired` / `unchanged` / `failed` 仍共享 Rust 闭包路径的分支规则；修复结果重新解析校验一次，再失败包装 ToolCallRepair
4. 修复 raw payload 不携带 core `invalid/error`；新增 unknown reply 字段仍拒绝
5. apply-to-result 按唯一 id 同时更新 content 的事实存储及派生 toolCalls/responseMessages；换 id 时检查冲突，禁止只更新便捷字段
6. 目标 id 缺失、重复、目标本来有效、修复后 id 冲突均返回 InvalidArgument；更新是全有或全无
7. generateObject 的嵌套 raw 路径、聚合 stream 结果和非流式结果均覆盖；不得因为 StepResult 改造丢掉其中一条路径
8. 原生流先交付原文 delta，宿主修复后交付/消费最终调用；OpenAI 流转换不倒改增量。非流式 OpenAI 外观仍是原生结果→修复→转换

合法参数本身不必把原始文本额外塞进 SDK core ToolCall；字节级原文由 V4 输出及受配置控制的底层录制保留。非法调用必须自带足够的错误原文供已有 repair helper 工作。无法恢复原文的新非法结构应被拒绝为无效 repair 输入，不以 stringify 猜出一份看似原文。

RFC-0035 已记录的 Swift/JVM repair 线程/取消限制仍是基线；本 RFC 不凭类型重构宣称其解决。ops/绑定工作应统一错误投递渠道并逐语言测试这些限制，不能意外从纯函数变成持锁回调或跨 ABI 新会话。

## 6. L2 投影、raw 与 metadata 不丢失

### 6.1 不混淆三种真相

- 传输字节与 framing 的真相在录制的 leaf transport；JSON Value 不保留重复 key、数字拼写或原始空白
- 原生协议的结构化事实由协议解码与 V4 适配保存；V4 不能表示的 provider 特性必须有明确 metadata/raw 保留路径
- core content 是便捷投影，不替代传输录制，不宣称与所有 provider wire 无损同构

`includeRawChunks` 为 opt-in；每个已解码原事件按到达顺序输出对应 raw 事件，不能只发聚合后的“差不多相同”对象。raw 原事件可以与一个或多个规范事件并存，必须固定相对顺序并用 fixture 验证。为避免消费者猜测，本提案固定为先 raw、再该事件产生的规范事件；不额外复制一份 raw 到 metadata。不能借 Raw 静默跳过本来应识别的 tool-call/usage/error。

当 raw 未开启时，已支持特性中会影响后续轮次的 signature、encrypted reasoning、item identity 等仍必须保存在 metadata，不能以 opt-in 开关为借口丢失。完全未知事件：开启 raw 时保留 raw；关闭时发可检查的 unsupported/projection 诊断，不伪造一个空 text。字节级需要仍走录制能力。

### 6.2 metadata 规范

按照 #200 将 `ProviderMetadata` / `ProviderOptions` 约束为 namespace→JSON object；内层允许嵌套 null/array/primitive。顶层 null 或数组不是合法 metadata。字段缺失和显式 null 由具体 Optional 声明决定，不把所有字段统一 default 成空对象。

- namespace 由具体包 descriptor/转换器声明，不从 model/provider 字符串猜测
- thoughtSignature、reasoning signature 等重复顶层字段迁到其声明的 namespace
- core → responseMessages 是无损的 part 级转存：按照 #200 的 `to_response_messages` 表，把每个保留 part 的完整 providerMetadata 映射到该 part 的 providerOptions，不按当前 provider/已知 namespace 做白名单过滤；未知 namespace、嵌套字段及 null 值一并保留。空 reasoning 只要有 metadata 仍保留，file/reasoning-file 回放也保留 part metadata
- responseMessages → V4 prompt → 厂商请求是另一边界：具体 provider 编码器按该包的 providerOptions 规则提取、转换其认识的 namespace/字段，不把所有 namespace 无条件铺进厂商 HTTP body。此出站提取不能反过来删改 core 已保存的 responseMessages，也不能被用来给前一步加 namespace 过滤
- 同 block 多次 metadata 按固定 SDK 基线对应 merge 规则合并；实现提取共用函数与上游差分向量，不能每绑定 deep-merge 一遍
- 未知 namespace/key 在合法 JSON 对象内原样保留，不允许生成器按已知字段白名单丢弃
- usage.raw、finishReason.raw、response metadata 与 tool metadata 分别归属，不把全部内容铺平塞进一个 map
- 流聚合按 `(类别, block id)` 维护首次出现顺序，text/reasoning id 同名也不冲突；不能只保留最后一个 block 的 metadata

验收采用“字段去向表”：每个协议 fixture 的特性字段标记为规范字段、metadata、raw-only 或明确拒绝，并提供下一轮重编码断言。JSON 等价不等于 HTTP 字节等价；二者分开报告。L2 的旧独立模型调研不再阻塞这些缺陷修复；如果未来另换数据模型，需要新的 RFC 与版本决定。

## 7. D8：单一生成来源

### 7.1 链路与边界

```text
Rust 规范类型 + 明确 Wire DTO/codec 描述 + 包 descriptor/preset 输入
  → aimux-codegen / aimux-manifest
  → manifest.json（含 types、schema、wire 描述、来源版本）
  → Rust preset、绑定 DTO/codec、操作描述引用、文档、contract vectors
```

Rust 类型是协议结构维护源；descriptor 是包能力维护源；manifest 是汇总生成物。JSON Schema 是 manifest 的派生投影，不允许与 Rust 双向人工编辑。普通 derive 无法描述的 Optional、受约束联合、日期、base64、错误、JSON number 必须注册 codec 描述；未注册即生成失败，不得退化为 any。

ops 文档只引用这些 DTO 的稳定 type id/hash，拥有 op/envelope/帧/握手契约；本文拥有层间类型与序列化语义。provider/registry 文档拥有 descriptor 的包能力与工厂规则。不能三处各写一套工具/错误字段。D8 不添加新的宿主 callback 注入能力。

### 7.2 生成内容

覆盖 Node/TypeScript、Python、Go、Java、Kotlin、Swift、Dart/Flutter 及 C 边界，共八种接口；Kotlin 若复用 Java artifact，生成其声明的语言外观/引用而非再复制一套状态机。C 使用 JSON 边界和生成的类型描述/常量，不把递归 JSON union 强行变成未经设计的 C struct ABI。

必须覆盖全部 user/provider/core 类型、operation I/O、错误、recording DTO、工具、各模态；不是仅 provider settings。手写只保留加载、资源释放、线程与流桥接、语言 facade；不能手写字段映射或解析工具 input。

统一 wire 规则：

- optional/nullable 的缺失、null、false、0、空 string/array/object 独立编码
- discriminator 驱动结构联合；dynamic 约束等 SDK 已用字段不能另添标签
- opaque JSON 保持递归值；Go RawMessage 仅用于这类字段，不替代已知事件 union
- arbitrary JSON number 不允许被绑定统一转成浮点后静默舍入；codec 为无法无损表示的值保留数值 token/无损 JSON 容器，便捷 native 投影必须显式校验
- 顶层未知 variant 在当前协商类型版本下返回明确 decode 错误，不变成空内容；合法 metadata 内未知字段必须保留
- 请求配置未知字段按 #200 拒绝，特别是不能接受后 `serde(skip)` 的伪回调；将来新增字段必须同步更新 schema/manifest 和版本能力
- 标准 DTO 根类型已确定 raw/core 阶段，不尝试二次解析 string input

### 7.3 版本、确定性与门禁

manifest 记录固定 SDK 基线（#200 使用 `ai@7.0.122`、`@ai-sdk/provider@4.0.19`）、生成器版本、输入 hash、schema/type revision。不得依赖在线 latest。manifest hash 是产物一致性标识，不把每次 hash 变化当成新握手协议版本。

D8 `--check` 在临时目录重建全文件集合，缺失、过期、多余文件都失败；生成失败不改工作树。不在 build.rs 偷生成第二份。原 `gen_ts_types.py`、ts-rs 导出及其他被替代脚本在同次原子切换中退役，不能“暂时双轨”留下不同 wire。

类型版本兼容性由 ops 版本/能力协商契约承载，未知不兼容版本调用前失败。本文不另引入旧 ROADMAP 未落地的独立 `data_format_version` 作为第二个未定义握手；将来若独立版本化须明确与 ops 的映射。

## 8. 实施闭包与验收

以下是集成分支工作包，不是允许分批发布破损 wire 的顺序：

1. 锁定 T1–T5、#185/#204 处置和 SDK/源码基线；同步状态图
2. 建四层 DTO、codec 描述、转换接口；编译期禁止 provider 依赖 core 输出
3. provider 累积器/各协议转 V4 输出；同时落实 raw/metadata 去向表
4. core parse/repair/responseMessages/流聚合迁移；单事实存储派生结果
5. manifest/codegen 产出全部绑定 DTO/codec；ops 引用同源类型
6. 删除旧混层 StreamPart、公开 tracker 镜像、旧生成链；更新错误/repair 文档与迁移说明
7. 全 workspace、全部绑定和真实协议 fixture 验收后原子合并

### 8.1 必须提交的证据

| 组 | 验收，不代表已通过 |
|---|---|
| 四层覆盖 | 四角色合法 part；#200 的 V4 9 content/21 stream、core 11 content/26 stream，完整 variant 集合与禁止集合 |
| 参数 | §3 每个字符串/primitive/非法文本，raw 不变、parsed 不二次解析；schema 拒绝与 parse 拒绝分别测试 |
| 分片 | 每个 UTF-8/JSON 边界切分、空 fragment、name/id 省略、交错调用、重复 index/id、歧义、flush 幂等、EOF/重复 finish |
| 取消 | start 前、半个参数、end 前、repair 等待中、消费者 drop；无成功伪造、无后续 poll、无遗留累积状态 |
| 修复 | RFC-0035 全 fixtures、新 wire、递归 originalError、修改 id、冲突回滚、嵌套 generateObject、next-turn transcript |
| metadata | thought signature、encrypted reasoning、多个 namespace、深层 null、交错 block、usage.raw、unknown 字段；完整 metadata→part providerOptions 往返及 provider 出站提取分别断言 |
| binding | Rust→八接口 codec→Rust 规范 JSON 往返，完整 root schema；大整数、缺失/null 与字符串参数 |
| 生成 | 全文件集合 --check、两次生成确定性、无旧生成器、无手写协议副本 |
| 行为差分 | 固定本地 SDK 基线的 prompt/responseMessages/metadata merge/tracker 对照；T3 等主动差异列入允许差异表 |
| 资源 | sparse index、超限、总 buffer、取消释放和流吞吐/RSS，复用性能门禁报告 |

不得把 #204 提交说明里的测试数字当作本文或新架构的测试结果。验收报告逐项给命令、版本、结果与 artifact；缺绑定运行环境则仍为阻塞，不算“类型生成成功所以行为一致”。

## 9. 合并门槛与非目标

文档可独立作为 Draft 评审；实现前需要明确接受 T1–T5，尤其 T2/T3 的 #185/#204 冲突，补齐预算默认值，并与 ops/provider RFC 交叉校验 type ownership。#200 已记录的单步、原子切换、无兼容层及生成链决定不在这里重新请求批准；尚未合并意味着当前实现仍是旧基线，实施必须满足其完整闭包验收后才进入 master。

不新增 agent loop、宿主工具执行、审批运行时、自定义 C callback、旧 wire 自动迁移、新的数据模型竞赛或 provider 能力承诺。类型可表示某种事件，不等于所有模型可产出它；必须以各协议 adapter/fixture 和能力声明为证据。
