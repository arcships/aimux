# RFC-0006：Provider 开发规范

> **状态**：DRAFT（待评审）  
> **日期**：2026-07-28  
> **范围**：`aimux-providers` 中新增或重做 provider 适配  
> **关联**：[Provider 统计与提取结果](../provider-inventory/README.md)、[厂商适配层改进](0002-provider-improvements.md)、[录播测试方案](0003-test-cassette.md)、[协议转换与适配层设计](0005-protocol-conversion.md)

## 1. 定位与边界

本文档规定开发一个 provider 所需的最小流程、实现契约和验收条件。目标是确保本次声称支持的能力有协议依据、实现正确且可确定性验证。

核心原则：

1. 以厂商协议事实选择实现方式，不按 provider 名称或清单标签推断。
2. 优先复用已有共享层，但只复用共享代码实际支持的行为。
3. 只调查、实现和测试本次交付范围；不要求预先普查该厂商的全部能力。
4. 用户显式传入的选项必须被映射、产生 warning 或返回 error，不得静默丢弃。
5. 必需测试不访问公网、不读取真实凭据。

本文档不定义 provider 开发优先级、inventory 提取或统计流程，也不要求在 provider 任务中增加 CI、生成器、探测器或仓库级重构。确需修改公共契约或共享基础设施时，应作为独立、可验证的前置改动。

## 2. 每个 Provider 的主干流程

```text
确定交付范围 → 核验协议 → 选择实现路径 → 实现 → 针对性测试 → 导出与检查 → 记录实现事实
```

### 2.1 确定交付范围与最小证据

开始实现前，只需确认与本次交付直接相关的信息：

- canonical ID、必要 aliases，以及是否已有同厂商或聚合入口；
- 本次实现的能力，例如 language、embedding、speech 或 image；
- 官方 API 文档，或者官方 SDK/OpenAPI 中对应协议的位置；
- 鉴权、base URL、endpoint 公式和所需环境变量；
- 至少一个可用于测试配置的模型 ID；
- 本次能力的请求、响应和错误结构；
- 本次声称支持的可选行为，例如 streaming、tools、reasoning 或异步任务。

不属于本次交付的能力无需调查或填写“未知”。以后扩展能力时重新核验对应协议。

[`provider-inventory/providers.json`](../provider-inventory/providers.json) 用于发现候选、canonical ID、aliases 和来源线索，不能作为协议实现依据。证据冲突时按以下顺序裁决：

1. 厂商官方 API 文档、SDK 或 OpenAPI；
2. `reference/` 中成熟且可追溯的当前实现；
3. inventory 中多个独立来源一致的记录；
4. 单一第三方来源或自动推断。

没有足够证据确认本次请求和响应契约时，不进入实现。

### 2.2 选择实现路径

| 路径 | 使用条件 | 主要工作 |
|---|---|---|
| OpenAI 兼容薄封装 | 本次使用的鉴权、URL、请求、响应和流式行为均可由 OpenAI 共享层正确表达 | 配置 base URL、名称、凭据、profile 和模型工厂 |
| OpenAI 共享层扩展 | 主体兼容，但存在明确、有限且适合复用的差异 | 先扩展共享行为及回归测试，再增加薄封装 |
| 原生协议 | 鉴权、path、消息、响应、流状态机或多步骤调用存在结构性差异 | 实现厂商类型、转换、HTTP 调用和必要状态机 |
| 模态专用实现 | 只接入 embedding、reranking、speech、transcription、image 或 video 等能力 | 直接实现对应模型 trait 和工厂 |

选择规则：

- 使用能够完整保留本次协议语义的最薄实现。
- “OpenAI-compatible”、`/v1` URL 或 inventory 标签不能单独证明薄封装可用。
- profile 字段或公共方法存在，不代表请求和响应代码已经消费该能力。
- gateway、cloud platform 和 local runtime 是业务类别，不是实现路径，仍按上表判断。
- 纯模态 provider 不为统一外观伪造 language model。

### 2.3 实现契约

#### Config 与鉴权

- 必需凭据缺失或为空时快速失败；无鉴权服务不伪造用户凭据语义。
- base URL、版本段和 endpoint 拼接规则必须明确，避免重复或遗漏 `/`。
- 只有请求代码会读取的配置项才对外暴露。
- 默认禁止调用级或配置级 header 覆盖鉴权、签名和其他协议必需 header；只有公共 API 或厂商协议明确允许时才可覆盖，并测试冲突行为。
- 凭据不得出现在日志、错误、`Debug`、测试快照或 cassette 中。

#### Provider 与模型工厂

- 只提供厂商实际支持且本次实现的模型工厂。
- 支持 language model 时实现 [`Provider`](../aimux-core/src/provider.rs)；纯模态实现直接提供对应模型工厂。
- 同一厂商的多种能力共享 Config、鉴权和 HTTP client，不复制安全敏感逻辑。
- 公共 provider 名、模型 provider 名和 model ID 必须稳定；存在非显然差异时增加断言。

#### 请求、响应与错误

- 对本次模型可接收的显式选项执行“映射、warning、error”三选一。
- `provider_options` 只读取本 provider 的稳定命名空间，并验证字段类型和取值。
- 请求转换尽量保持为无网络副作用的纯函数。
- 响应保留公共结果类型可以表达的 text、reasoning、tool calls、usage、finish reason 和 provider metadata。
- 未知但合法的枚举值安全降级并尽可能保留 raw 值；未知响应不得 panic。
- 厂商错误结构与共享结构不同时，增加专属解析，不依赖错误的默认映射。
- 不得在 provider 内伪造 core 尚未定义的公共字段或行为。

#### 代码组织与导出

- 无协议差异的薄封装优先使用单文件。
- 原生实现只在复杂度需要时拆分 `types`、`convert`、`model` 或模态文件；不创建空占位模块。
- 在 [`aimux-providers/src/lib.rs`](../aimux-providers/src/lib.rs) 导出调用者需要的 Config、Provider 和模型类型，不导出 wire types 或解析状态。

### 2.4 最小测试要求

所有必需测试使用本地 fixture、wiremock 或已脱敏 cassette，不访问真实服务，不读取真实凭据。测试范围跟随本次实现和差异，不为未实现能力建立占位测试。

| 改动类型 | 必测内容 |
|---|---|
| 所有 provider | URL/path、最小请求和最小响应；存在鉴权、必填配置、特有错误或非显然身份差异时，再测试对应行为 |
| 无差异薄封装 | 用共享 smoke test 验证 URL、凭据和模块身份；profile 有差异时再测试其行为 |
| profile 或共享层扩展 | 新差异的直接行为测试，以及默认 OpenAI 路径不回归 |
| 原生协议 | 本次涉及的纯转换、厂商错误和特有协议行为；流式仅在支持时测试状态顺序和中途错误 |
| 自定义 headers | 暴露该能力时，测试普通 header 合并和必需 header 冲突行为 |
| 不支持的 options | 本次公共 options 存在上游不支持的字段时，测试显式输入产生预期 warning 或 error |
| 模态实现 | 测试第 3.4 节中本次模态对应的输入、输出和限制，以及一个失败路径 |

共享 HTTP 或错误层已经覆盖的通用状态码行为，不要求每个 provider 重复测试。provider 只测试自身结构或映射差异。薄封装可接入 [`openai_compatible_test.rs`](../aimux-providers/tests/openai_compatible_test.rs) 的共享测试，但不重复证明未改变的共享实现细节。

### 2.5 完成检查

至少运行与改动直接相关的检查：

```bash
cargo fmt --check
cargo test -p aimux-providers --test <provider_test_target>
cargo clippy -p aimux-providers --lib -- -D warnings
```

修改 OpenAI 共享层、公共工具或 core 契约时，再运行受影响 crate 的完整测试；单纯薄封装不因流程要求承担无关的全仓验证。

实现完成后，记录对应 canonical ID 的实现状态和本次新增的协议证据。

## 3. 条件规则

本节只在本次实现命中对应能力时执行，不是每个 provider 的固定检查表。

### 3.1 OpenAI 共享层扩展

只有差异可以表达为明确的 profile 字段、封闭 enum 或通用转换规则时，才扩展共享层。

扩展必须满足：

1. 请求构造或响应解析实际读取新增配置；
2. 默认 OpenAI 行为保持不变并有回归测试；
3. 不按 provider 名称在共享状态机中散落条件分支；
4. 若差异只服务一个复杂厂商且显著增加共享层复杂度，改用原生实现。

### 3.2 流式输出

只有声明支持 streaming 时，才需要核验和测试：

- 使用上游实际传输协议，例如 SSE、NDJSON、WebSocket 或 event stream；
- 公共事件满足 start、delta、end 和 finish 的顺序契约；
- text、reasoning 和 tool input 使用稳定 ID；
- 工具参数分片在形成合法 JSON 后再产生最终 tool call；
- 上游提供最终 usage、finish reason 或结束元数据时，在公共结果中正确保留；
- 上游错误、格式错误或连接中断不 panic，也不制造成功 `Finish`。

非 SSE 协议使用对应解析器或独立状态机，不为复用接口而改变协议语义。

### 3.3 Tools、reasoning 与结构化输出

只有声明支持对应能力时，才核验其请求和响应：

- tools：定义、choice、调用参数、结果和流式分片；
- reasoning：请求选项、内容字段和 usage；
- structured output：schema、模式限制和不支持行为。

### 3.4 非语言模型

只执行本次模态对应的规则：

| 能力 | 必须保持的契约 |
|---|---|
| Embedding | 输出与输入顺序一致；明确批量和维度限制 |
| Reranking | 保留原始 document index、score 和 `top_n` 语义 |
| Speech / Transcription | 正确处理媒体类型、二进制或 URL 结果和支持的格式 |
| Image / Video | 正确处理数量、尺寸或比例、输入文件和结果形式 |

超过上游限制时应报错或按公共 trait 明确允许的规则分批，不得静默截断。

### 3.5 异步任务 API

只有调用链包含提交和轮询时，才定义并测试：

- 提交结果与任务 ID；
- 轮询间隔和处理中状态；
- 成功、失败和超时终止条件；
- 公共接口确实支持时的取消行为。

不得无限轮询，也不得把处理中状态当作成功结果。

### 3.6 Cassette

确定性测试能够精确覆盖请求和解析时，cassette 不是必需门禁。以下情况可增加已脱敏 cassette：

- 原生或复杂流协议缺少稳定 fixture；
- 真实响应与官方文档存在已知差异；
- 实现依赖厂商特有字段或事件组合。

无差异薄封装不需要仅为流程完整而录制 cassette。录制、脱敏和来源要求见 [RFC-0003](0003-test-cassette.md)。

### 3.7 公共契约变更

如果正确接入依赖新增 core 字段、options、trait 方法或共享基础能力，应先单独评审该公共契约，并用回归测试证明已有 provider 不受影响。provider 实现可以依赖该改动，但不得在本地类型中制造不一致的替代接口。

## 4. 变更记录要求

简单薄封装只需在 issue 或 PR 中提供官方协议证据、共享层复用依据、已验证差异和对应测试，无需填写独立设计表。

只有共享层扩展、原生协议、自定义流状态机、签名鉴权、异步任务或 core 变更需要独立设计记录。记录应覆盖协议证据、关键转换、错误边界、改动范围和已知限制，格式不限。

## 5. Definition of Done

- [ ] 本次能力范围、provider 身份和官方协议证据明确。
- [ ] 实现路径符合第 2.2 节，且没有依赖未生效的共享能力。
- [ ] Config、鉴权、请求、响应和错误满足第 2.3 节。
- [ ] 命中的条件能力满足第 3 节；未命中的条件没有被强制实现或测试。
- [ ] 第 2.4 节要求的确定性测试通过，测试不使用公网或真实凭据。
- [ ] 公共导出完整，相关格式化、测试和 lint 检查已执行。
- [ ] canonical ID 的实现事实和新增协议证据已记录。
- [ ] 任何共享层或 core 改动都已独立验证默认路径不回归。

## 6. 实现入口

- 公共模型契约：[`aimux-core/src/`](../aimux-core/src/)
- OpenAI 共享层：[`aimux-providers/src/openai/`](../aimux-providers/src/openai/)
- Provider 测试：[`aimux-providers/tests/`](../aimux-providers/tests/)
