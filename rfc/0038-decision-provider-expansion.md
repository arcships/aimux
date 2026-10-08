# RFC-0038：官方 Decision Provider 扩展

状态：已实现，PR #226 验证中。资料核对日期：2026-10-08。
前置：[RFC-0037](0037-decision-api.md)，TypeSafe Jev 已通过 PR #221 合入。

## 目标与边界

在现有 `decide`、`DecisionModel`、统一录制与回放上接入官方决策接口及真实候选
评分。只接服务商或运行时的官方实现，不增加 structured-output 模拟、生成式
fallback 或第三方转售 provider。接口可用性仍取决于部署版本及模型。

本次补齐原清单中全部可实施项目：OpenAI 完整 choice/score 与图片、Ollama、
llama.cpp、LocalAI、Laya、Cloudflare、vLLM diffusion 示例、SGLang 原生决策和
候选评分、vLLM generative scoring、统一回放与所有宿主语言的 JSON contract。
vLLM 核心 typed API 的两个未合并 PR 单独跟踪，不伪装成已发布能力。

## 身份与入口

Provider 表示服务实现和连接配置；model ID 是服务接受的权重部署名或别名。
沿用 `openai`、`ollama`、`llamacpp`、`localai`、`vllm`、`sglang`、`cloudflare`
（兼容既有 `cloudflare_workers_ai` ID），作者 HTTP 服务使用 `laya`。
Ollama 上的 Clef 仍属于 `ollama`，llama.cpp 上的 Laya 仍属于 `llamacpp`。

全部通过既有 `Provider::decision_model` / ProviderHandle 工厂创建。复用连接、
认证、headers、重试配置；不新增每个 provider 的 C ABI 或语言构造器。
Rust 原有五个本地运行时 wrapper 同样支持 `decision_model`。

| Provider | 默认 base URL / 配置 | 实际 POST |
|---|---|---|
| openai | `https://api.openai.com/v1`，OPENAI_API_KEY | `{base}/decisions` |
| ollama | `http://127.0.0.1:11434/v1`，OLLAMA_BASE_URL | `{base}/systemone` |
| llamacpp | `http://127.0.0.1:8080/v1`，LLAMACPP_BASE_URL | `{base}/systemone` |
| localai | `http://127.0.0.1:8080/v1`，LOCALAI_BASE_URL | `{base}/systemone` |
| laya | `http://127.0.0.1:8000/v1`，LAYA_BASE_URL | `{base}/systemone` |
| sglang | `http://127.0.0.1:30000/v1`，SGLANG_BASE_URL | `{base}/systemone`、`{base}/decisions`、`{base}/score` |
| vllm | `http://127.0.0.1:8000/v1`，VLLM_BASE_URL | 示例服务 `{base}/systemone`；评分移除末尾 `/v1` 后加 `/generative_scoring` |
| cloudflare | 显式提供 `https://api.cloudflare.com/client/v4/accounts/{account}/ai/v1`，CLOUDFLARE_API_KEY | 移除末尾 `/v1` 后加 `/run/@cf/cloudflare/{clef 或 clef-flash}` |

本地 factory 无 key 可用；显式 key 或对应 `<PROVIDER>_API_KEY` 可用于有认证的
部署。base URL 不当作 key。连接参数与录制中的 key 来源一致，回放不存明文凭据。
Cloudflare model 接受 `clef`、`clef-flash` 或 `@cf/cloudflare/` 前缀，body 中使用短名。

协议由 `provider_options.<provider>.protocol` 显式选择；默认 `systemone`。
只有 SGLang 可选 `decisions` / `score`，vLLM 可选 `generative_scoring`。
协议不会改变 provider 身份。未知协议、未知本 namespace 参数、生成式
`body_overrides` 均拒绝，不静默忽略。其他 provider namespace 不参与请求。

工厂和 `capabilities()` 不探测网络，不猜模型能力，不改目录分类。模型、版本、
projector、endpoint 是否可用由服务返回实际错误。vLLM 示例必须将 base URL 指向
独立 structured server；普通 `vllm serve` 不会因创建句柄而获得 `/v1/systemone`。

## Canonical contract

- `state` 是文本或 JSON 证据；`images` 是独立的内嵌图片列表，不从 state 猜媒体。
- `DecisionImage` 为 `{data: FileBytes, media_type, detail?}`，复用已有
  `FileBytes` 的 `{"Base64":"..."}` / `{"Binary":[...]}` 表达。
- Choice 的 `label` 始终是答案及概率 map 的稳定字符串键。新增可选 `value`，
  类型为 string / boolean；缺省使用 label。原生 boolean `true` 与字符串
  `"true"` 可同时出现，只需不同 label。响应保留 selected label 和显式 value。
- OpenAI Score 可使用字符串，或 `{label, description?}`；Rust 提供
  `DecisionScoreLevel` 转为 `DecisionDescription`。其他 SystemOne 服务继续
  原样传递结构化 guidance，不能误把其 JSON 对象当作 OpenAI rubric。
- Refusal、Skipped、Abstention 是独立答案 variant；不折算为 false、零分或空分布。
- `supports_images`、`max_images`、`supports_typed_choices` 具有 serde 默认值，
  旧请求与录制保持可读。图像能力表示适配器支持该输入，实际模型可能拒绝。
- 原生概率和 confidence 保留原值。未承诺十进制舍入的服务使用空 rounding；
  校验容忍两倍 f32 epsilon（score 按级数缩放）的浮点噪声，不归一化或重算原生值。
  TypeSafe 继续使用其两位小数规则。

## OpenAI `/v1/decisions`

同步 JSON POST、Bearer 认证，复用 organization/project、headers、取消和重试。
官方示例模型为 `gpt-6-luna`；model ID 不设静态允许名单。[1][2]

| 原生字段 / 行为 | 映射 |
|---|---|
| input string | state 文本；对象/数组序列化为 JSON 文本 |
| input user content | 有 images 时先放 `input_image` data URLs，再放 `input_text` |
| questions[].name | canonical id；按 name 关联，允许响应乱序 |
| predicate.instructions / probability | Boolean instructions / P(true) |
| choice.choices[].value/description | option.value（缺省 label）/ 文本 description，2–255 项 |
| choice.choice / probabilities[] | 按原生类型匹配回 label；不合并 bool 与同名字符串 |
| score.levels[] | 字符串转 label，或保留独立 label/description；至少两级 |
| score.probabilities[].value/label | 零起点索引与请求 label 均须匹配 |
| answers[].type=refusal | 逐题 Refusal，保留其他正常答案 |
| safety_identifier | `openai.safety_identifier`，最多 128 字符 |
| usage/details | 可选统计；保留 raw，可读计数映射到 Usage；不因合计不一致丢弃答案 |

最多 128 张内嵌图片；detail 为 low/high/auto/original。图片格式和模型解码能力
由服务校验。对象/数组 instructions、Boolean true/false criteria 不在 OpenAI
契约中，HTTP 前返回 Unsupported。重复/缺失/额外题目、重复原生值、错误题型和
不完整分布拒绝，并保留 HTTP 错误上下文。模型别名与响应 model 不要求相同。
公开 API 没有承诺内部网络、一次 forward pass、校准训练或十进制舍入机制。

## SystemOne 家族

共享 codec 只负责 `state + questions map → answers map`：Boolean 为 `noul`，
Choice criteria 为 label→description/null，Score criteria 为有序 levels。
答案分别读取 `noul`、`choice/probabilities/confidence`、
`score/legend/probabilities/confidence`。Score legend 必须与请求逐级匹配。
usage 使用 input_tokens/output_tokens，缺失计数保持未知。raw 与所有扩展字段保留。

| 实现 | 机制、限制与差异 |
|---|---|
| Ollama ≥0.35 [4] | 模型需有 decision 能力；最多 64 题。通用模型候选上限通常 26，Clef 可到 255，交服务按模型判断。confidence 使用分布熵。images 是原始 base64 字符串，仅支持图片的模型可用。`keep_alive` 透传。 |
| llama.cpp [5] | 需要 decision GGUF metadata；题数、choice 上限依模型，Score 2–10。图片使用 data URL，需要模型/projector 支持。校准温度取模型元数据；不宣称所有模型 confidence 都已校准。 |
| LocalAI [6] | 配置 `decisions` usecase；不把 NER 转换路径自动宣称为专用决策模型。最多 64 题，候选上限依 backend。confidence 可为 margin/entropy 等后端定义。`keep_alive` 接受但上游忽略；latency_ms 与后端统计保留。 |
| Laya 作者服务 [7] | 最多 64 题、Choice 100、Score 32、总候选 512、state 50,000 字符、body 2 MiB。无图片。透传 max_len/head_max_len/task/lang/lang_guess/min_confidence；routing、action head、截断统计保留 raw。 |
| Cloudflare Clef [3] | 最多 64 题、Choice 2–255、Score 2–10；题目 ID 为最多 100 个 ASCII 字母/数字/`_.-`。REST envelope success/result/errors/messages；解出 result 并保留整个 envelope。confidence 来自原生分布。 |
| vLLM diffusion 示例 [8] | 最多 64 题、候选 2–26；读取固定答案槽的真实候选概率，支持重复采样/去噪。Score levels 限文本，因为上游会 stringify structured levels。其部署和核心 server 不同。 |
| SGLang SystemOne [13] | 当前 main 已合入，旧“只有 score”调研过时；待稳定 release 包含这些提交前使用 nightly。由服务完成编码或专用 decision checkpoint 评分，confidence 为上游定义的概率函数，`x_label_mass` 保留 raw。 |

Laya `abstention:"abstained"` 映射 Abstention；passed / unevaluated 和门限保留 raw，
不猜测 action head。vLLM `null` 答案仅在 diagnostics.skipped 明确记录时映射
Skipped；其他 null 仍视为坏响应。支持 vLLM 题目扩展
`questions.<id>.depends_on/ask_if/alone`，以及顶层
instructions/samples/auto_max/auto_threshold/steps/think/chunk_rows/chunk_prompt/
sequential/seed。具体数值约束由上游实施，未知字段不转发。
上游还有 `ask` 子集选择参数，但核对版本的 SystemOne wrapper 会索引已排除题目的
答案而触发 KeyError；本适配器不开放该参数。需要选择题目时直接构造 questions
子集并包含所需依赖，条件跳题仍使用 ask_if。

图片的已知限制：Cloudflare 最多 4 张 PNG/JPEG/WebP、单图 4 MiB、合计 8 MiB、
body 13 MiB；LocalAI 最多 8 张 PNG/JPEG、合计 8 MiB decoded、12 MiB data URLs、
body 16 MiB（纯文本 64 KiB）。适配器检查数量、MIME、base64 与字节数；像素尺寸
解码、模型 admission 留给上游：Cloudflare 单图 16 MP，LocalAI 每边 4096、合计
16 MP。不引入本地图像解码依赖、远程 URL 抓取或多余媒体框架。

## SGLang 原生 `/v1/decisions`

PR #41208 已合入 SystemOne route，#42183 又补专用 checkpoint；不能仅依据早期
关闭的 #40992 判断没有支持。[13] `protocol:"decisions"` 使用另一套官方 schema：

- input=原生 state；questions 为数组。Boolean→yes_no 的 question/yes/no，
  Choice→options[].name/description（2–26），Score→levels（2–10）。
- images 使用 `{url:dataURL,detail?}`；SystemOne 和 decisions 都可传
  chat_template_kwargs。只有 decisions 可传 temperature、prompt_format_version、
  return_prompt_token_ids，SystemOne 不接受后三者。
- response answers 仍按 ID；yes_no probabilities 为 yes/no，Choice 为 option name，
  Score 为字符串索引。保留 server 的 choice/score 并验证分布。
- 当前服务端 prompt format=1，关闭 thinking、逐题 prefill、验证答案位置的候选是
  单 token，再对候选 logits 按 temperature 做 softmax。来源记 logit_scoring。
- label_mass 是候选在未缩放全词表的概率质量，**不是 confidence**。canonical
  confidence 留空；label_mass、prompt/label token IDs、实际 model、usage 保留 raw。

## 直接候选评分

SGLang `protocol:"score"` 和 vLLM `protocol:"generative_scoring"` 面向已编码请求，
不在 aimux 再维护未经验证的 prompt encoder。普通业务调用优先使用上述原生
question API，其服务器拥有模型编码。直接评分要求：

```json
{
  "sglang": {
    "protocol": "score",
    "model_revision": "weights-commit",
    "tokenizer_revision": "tokenizer-commit",
    "prompt_format_version": "application-encoder-v1",
    "temperature": 1,
    "encoded": {
      "urgent": {"prompt_token_ids": [101, 202], "label_token_ids": [303, 404]}
    }
  }
}
```

这里的 token ID 仅示范结构，不能复制为其他模型的编码。调用方负责使用一致的
模型/tokenizer、编码 state/题目；这三项 revision 是调用方提供的来源记录，
不是客户端对服务器版本的认证。encoded ID 必须与 question ID 完全一致；每个
prompt 非空，每个候选是一个独立 token ID，不接受 token 序列。顺序固定为
Boolean true/false、Choice options 顺序、Score levels 顺序。此路径不接受图片；
state/questions 在此用于绑定结果与录制，实际发送的是完整 prompt_token_ids。

SGLang `/v1/score` 一次批量发送 query=[]、items=各 prompt IDs、每项的
label_token_ids、apply_softmax=true、temperature、return_token_logprobs=true。
scores 的行与候选数必须匹配；未缩放全词表 logprobs 原样保留。[11][13]

vLLM `/generative_scoring` 每题每候选发一次真实请求：query=[]、一个 token-ID
item、相同候选集轮换首项、apply_softmax=true、add_special_tokens=false。
上游只返回首项 score，因此 N 个候选需要 N 次 prefill；整批成本为候选数之和。
没有 temperature 参数，只允许 1。每次 model 必须一致；归一化分布不成立时
报错，不重新缩放拼凑。所有 exchange/headers/raw 保留，usage 对成功 attempt 内实际调用求和；
缺少计数则相应总数保持未知。`response.body.exchanges` 明确表示聚合，绝不伪装
为一条上游响应。[12]

两条路径均返回 logit_scoring；Boolean 取第一候选，Choice 取最大概率（并列按
请求顺序），Score 算零起点期望值。confidence 留空，不把 label_mass 或最大概率
假装成上游 confidence。取消/timeout 覆盖整个 call，重试重新执行整个 attempt；
失败前的真实 HTTP 也保留，可能产生的重复评分成本如实记录。

## 录制、验证与宿主语言

继续使用统一 JSONL / RingRecorder，operation=decision。输入含 images、typed
choices、协议 options、token IDs 与 encoder provenance；provider snapshot 保存
连接/能力和默认协议，实际选定协议以 call options 为准。
SGLang/vLLM 的题数/候选上限随协议变化，静态 capabilities 不填写统一上限；
相应 native endpoint 在请求映射时校验自己的限制。probability_source 静态值描述
默认协议，实际来源以每次 result 为准。旧录制缺 images 等价于
空列表。mock 匹配包含图片及 detail；变更媒体不会误命中旧结果。

每个适配器具备：synthetic official-contract fixture、本地 HTTP 测试、
record→mock replay、重建 provider→HTTP replay。vLLM 多请求按真实顺序重放。
既有 TypeSafe live 录制继续回归；新增服务没有线上凭据/部署时，不称为 live 实测。
Rust、C、Node、Python、Go、Java、Kotlin、Swift、Flutter 共用相同 JSON；所有
绑定均加入 OpenAI 图片、原生 boolean choice 和独立 Score description 的契约测试。

## 自审与实施状态

删除了数组位置检查、用 usage 合计决定答案成败等冗余校验。共享模块仅包含实际
复用的 codec/transport/媒体编码；不新增 provider 类型层级、能力注册中心、网络
探测或评分插件体系。各服务的新响应字段进入 raw；未知事实留给上游契约和模型。

| 项目 | 状态 |
|---|---|
| TypeSafe、OpenAI 全部本 RFC 范围 | 已实现 |
| Ollama、llama.cpp、LocalAI、Laya、Cloudflare | 已实现 |
| vLLM diffusion 示例、SGLang 两种原生题目协议 | 已实现 |
| SGLang score、vLLM generative_scoring 三题型 | 已实现；显式编码，完整记录成本 |
| 图片、扩展答案、typed values、回放、宿主 JSON | 已实现 |
| vLLM 核心 #59299 / #60465 | 等待上游合并/发布，不能在本库完成上游工作 |

## 官方来源与核对版本

1. [OpenAI Decisions guide](https://developers.openai.com/api/docs/guides/decisions)
2. [OpenAI Decisions reference](https://developers.openai.com/api/reference/resources/decisions/methods/create)
3. [Cloudflare Clef](https://developers.cloudflare.com/workers-ai/models/clef/)、[REST envelope](https://developers.cloudflare.com/api/resources/ai/methods/run/)
4. [Ollama 官方公告](https://ollama.com/blog/ollama-now-supports-jev-style-decision-models)、[源码 e3cddc3](https://github.com/ollama/ollama/blob/e3cddc3e897d8414a60a46e23f5ef3a99be2eb81/decision/systemone.go)
5. [llama.cpp server reference](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md#post-v1systemone-typesafe-compatible-system-one-api)
6. [LocalAI Decisions](https://localai.io/docs/features/decisions/)
7. [Laya server 3cf26cb](https://github.com/NandhaKishorM/laya/blob/3cf26cbcb18725dbc2d127bb8bb2c4c43243ae63/laya/serve.py)、[confidence gate](https://github.com/NandhaKishorM/laya/blob/3cf26cbcb18725dbc2d127bb8bb2c4c43243ae63/laya/confidence.py)
8. [vLLM structured server ba77c4c](https://github.com/vllm-project/vllm/blob/ba77c4c13018aa19244545cdc7ae8e016d66955e/examples/features/structured_diffusion/structured_server.py)
9. [vLLM #59299（未合并）](https://github.com/vllm-project/vllm/pull/59299)
10. [vLLM #60465（未合并）](https://github.com/vllm-project/vllm/pull/60465)
11. [SGLang #40826（已合并）](https://github.com/sgl-project/sglang/pull/40826)
12. [vLLM generative scoring](https://docs.vllm.ai/en/latest/serving/online_serving/generative_scoring/)
13. [SGLang decision_models 45abea0](https://github.com/sgl-project/sglang/blob/45abea0269c4fce45ae811aabacb5bdf2aacd355/docs/docs/supported-models/decision_models.mdx)、[#41208](https://github.com/sgl-project/sglang/pull/41208)、[#42183](https://github.com/sgl-project/sglang/pull/42183)
