# RFC-0033: 代码收敛总体规划(providers / FFI / bindings / 录制 / 错误 / 测试 / 文档)

> **Status**: DRAFT
> **Date**: 2026-09-03
> **Scope**: 全仓。`aimux-core`、`aimux-provider-utils`、`aimux-providers`、`aimux-ffi`、`bindings/*`、`tools/*`、`docs/`、`rfc/`
> **Related**: [RFC-0032](0032-provider-protocol-registry.md) 协议驱动注册表(本规划的 providers 骨架)、[RFC-0023](0023-runtime-request-recording.md) 录制回放、[RFC-0001](0001-multilang-bindings.md) 多语言绑定、[RFC-0003](0003-test-cassette.md) cassette、[RFC-0012](0012-source-dedup.md) 上一轮源码精简、[RFC-0017](0017-provider-config-dx.md) 注册表化
> **跟踪**: [#166](https://github.com/arcships/aimux/issues/166) 删减路线图(逐 PR 勾选)、[#167](https://github.com/arcships/aimux/issues/167) 录制回放完善
> **参考项目**: pi-ai(协议优先的 provider 组织)、Apache OpenDAL(跨语言错误模型)、LiteLLM(单模态 provider 的 transform 模式)
> **实施方式**: 本 RFC 是规划文档。实现按 §12 的顺序逐 PR 进行,每个 PR 只做一条;每条都写明了前置、门禁和会丢失的行为,实现者不需要再回到本文以外的上下文。

---

## 1. Motivation

aimux 今天约 25 万行源码(不含 cassette 与文档)。上一轮 RFC-0012 做的是同类代码去重,这一轮针对的是结构性的**乘法**:

- **模态 × provider**:9 个模态 trait,72 个模型实现(17 个 `LanguageModel`、55 个单模态),251 行注册表之外还有 33 个只改 base_url 和 env 名的 wrapper 文件。
- **provider × 测试**:`aimux-providers/tests` 80,243 行,占该 crate 六成。112 个文件是 AI SDK 每个 provider 自己的 TS 测试的逐个翻译(groq、deepseek、cerebras 各自把 openai chat 的测试再跑一遍),只有 6 个文件回放 cassette。
- **功能 × FFI × 语言**:109 个 C 导出,每个功能(生成、录制、trace、session、router、MoA、OpenAI 输出格式)都各自穿过 C ABI,再在 7 种语言里各写一份构造函数、错误解码、类型镜像和流泵。类型镜像 18,944 行,约占 Rust 加 binding 源码的 10%。
- **三种记录格式**:RFC-0003 cassette、RFC-0023 Recording、RFC-0015 TraceRecord 各有自己的结构和匹配器。

pi-ai 的经验是:协议实现是代码,协议选择是数据;测试按协议 × 能力组织,不按 provider。aimux 已经有一半——251 行注册表就是 pi-ai 的 `providers/`——缺的是 `protocol` 这一列,以及把它贯穿到测试、录制、FFI 和 binding 的设计。

**本 RFC 不砍任何功能。** 每个子系统(trace、session、recording、router、MoA、OpenAI 输出格式、模型目录、search 模态、单模态 provider)全部保留,改的是实现方式和暴露方式。

## 2. 原则

1. **cassette 不删。** 2,799 个录制文件是所有 PR 的字节级回放门禁,越往后越依赖它。
2. **功能不砍,改实现。** 子系统一个不拿;每个子系统从"各自穿 FFI"改为"走统一的 op",从"各自一种记录格式"改为"一种"。
3. **文档整理不删。** 按受众重排目录,历史归档,链接一次修完。
4. **一个方向一个 PR。** 基于 `master`,与 PR #164 无交集;先 providers 后 bindings;最高置信度的删减先落地。
5. **每条都核实过。** 行数重新量过,依赖 grep 过,会丢的行为写在条目旁边。标注**改法**的条目是方向成立但原提案漏了依赖或行为,按改法做,不按原提案。
6. **测试代码与测试数据分开算。** 删测试代码要在 §8.5 的测试策略定下来之后才做。

## 3. 现状测量(master @ 5e78b54f)

### 3.1 按 crate

| crate / 目录 | 行数 | 备注 |
|---|---|---|
| aimux-core | 19,884 | 其中 generate + 9 个模态 trait + 类型约 7,000;recording 2,530 + replay 2,019 + trace 2,642 + openai_output 1,639 + moa 775 + session 715 + router 537 + json_repair 433 + catalogue 310 |
| aimux-provider-utils | 7,132 | http.rs 2,420 是主体;其余是 TS 一文件一函数的搬运 |
| aimux-providers | 134,760 | src 约 54,500;tests 80,243(127 个文件);cassette 2,799 个文件另计 |
| aimux-ffi | 6,472 | 单文件 lib.rs 4,123 行,109 个导出,手写 aimux-ffi.h,无 cbindgen 无漂移门禁 |
| bindings/node | 15,460 | napi-rs crate,完全绕过 aimux-ffi |
| bindings/java | 14,555 | JNA,91 个声明,100 个 Builder 内部类 |
| bindings/flutter | 8,526 | 91 个 dart:ffi 查找 |
| bindings/go | 8,288 | cgo |
| bindings/swift | 6,760 | |
| bindings/kotlin | 6,744 | 第二套完整 JVM binding,与 Java 无 gradle 依赖 |
| bindings/python | 6,724 | pyo3 crate |
| tools/aimux-web | 8,712 | rust 2,554 + web 2,665 + 3,111 行类型副本 |
| tools/aimux-cli | 919 | |
| tools/aimux-replay | 165 | |

### 3.2 providers 源码

| 模块 | 行数 |
|---|---|
| openai/ | 7,606 |
| anthropic/ | 5,590 |
| google/ | 4,534 |
| xai/ | 3,600(chat + responses 两份) |
| bedrock/ | 3,315 |
| vertex/ | 3,052(google 与 anthropic 的副本) |
| cohere/ | 1,758 |
| mistral/ | 1,519 |
| huggingface/ | 1,263(responses 副本) |
| azure/ | 1,040 |
| open_responses.rs | 1,394 |
| 32 个单文件单模态 provider | 约 15,000 |
| provider.rs / provider_name.rs / catalogue.rs / replay.rs | 1,090 / 1,053 / 596 / 458 |

模型实现数:LanguageModel 17、ImageModel 11、SearchModel 11、TranscriptionModel 9、EmbeddingModel 7、VideoModel 7、SpeechModel 6、RerankingModel 4。

### 3.3 providers 测试

| 测试组 | 行数 | 说明 |
|---|---|---|
| OpenAI 兼容 fork(groq、deepseek、xai、mistral、azure、openrouter、cerebras、fireworks、deepinfra、moonshotai、alibaba、baseten、perplexity、thin_wrapper 等 16 个文件) | 15,370 | 与 openai chat 测试同构 |
| responses 家族(openai、open、HF、azure、xai、codex) | 10,024 | 五份副本各测一遍 |
| 原生协议(openai、anthropic、google、bedrock、cohere、vertex) | 28,886 | 这些就是协议测试;vertex 与 google / anthropic 重复约 2,000 |
| 单模态(image、speech、transcription、video、embedding、rerank、search、files) | 17,072 | |
| 横切(conformance、cassette_*、e2e、data_loss、provider_error、replay、list_models 等) | 9,082 | 只有这里回放 cassette |

mock 方式:112 个文件用 wiremock 手写响应体;6 个文件用 `tests/common/replay.rs`(401 行)挂 cassette;cassette 目录 32 个。

### 3.4 FFI 与 bindings 的错误面

AiMuxError 13 个变体 + ApiCallError 6 个字段。C 侧 16 个 `aimux_error_*` 取字段函数(766 行);各语言的错误层:Flutter 705、Node 551、Java 457、Go 359、Kotlin 350、Python 311,合计约 2,700 行。对照 OpenDAL 四种 binding 的错误层合计 596 行(见 §8.1)。

## 4. 轨道 A · 机械整理

五个 PR,互不依赖,不碰 provider 逻辑。

| PR | 做什么 | 行数 | 核实 | 注意 |
|---|---|---|---|---|
| A1 | 删 16 个一次性脚本(`fix_profiles{,2..5}.rs`、`check_missing.py`、`list_wrong_providers.py`、`revert_wrong_inventory.py`、`update_inventory_status.py`、`migrate_openai_compatible_*.py`、`gen_provider_registry.py`、`gen_responses_convert.py`、`generate_all_providers.py`、`update_lib_rs.py`、`gen_vertex_maas_providers.py`);删 `scripts/fix_tool` workspace 成员;删根 Cargo.toml 无人用的 schemars、proc-macro2、syn、quote。**保留** `generate_thin_wrapper_cassettes.py`(conformance_test 引用为 cassette 来源,B2 后删)、两个 cassette 转换器、两个 LiteLLM 调研脚本、`responses_similarity_audit.py`(B6 后删) | −2,117 | 已核实,#168 | 逐脚本查过引用、输入和可运行性;三个生成器的输出无 generated 标记且已被手改,重跑会覆盖 |
| A2 | 删生成的 `ProviderName` 枚举及其语言副本,注册表按字符串;aimux-web 加 4 行 `provider_names()` 迭代器 | −3,483 | 改法 | 生成脚本输出表漏了第 9 份 `tools/aimux-web/web/src/types/ProviderName.ts`(262 行),一并删;`ci.yml` contract-tests job 第一步 `gen_provider_names.py --check` 连着删;Node 自动补全改由 `gen_ts_types.py` 输出字符串字面量联合(约 260 行),不设门禁 |
| A3 | `provider-inventory/` 4 个文件移到 `archive/`;RFC-0004 标历史 | 42,599 行归档 | 已核实 | 没有代码读它 |
| A4 | 删 `tools/aimux-web/web/src/types/` 140 个 ts-rs 类型手工副本(138 个与 `bindings/node/src/types` 逐字节相同,`HttpRecord.ts`、`JsonValue.ts` 已漂移);tsconfig paths 指向生成集,或把目录加进 `gen_ts_types.py` 的同步目标 | −3,018 | 改法 | 7 个 `Wire*.ts` 是 web 专有,保留;16 处引用全是 `import type` |
| A5 | 删 `bindings/flutter/example/` 的 macOS、Windows、Linux 三个平台脚手架 | −2,988 | 改法 | iOS、Android 必须留:CI `flutter-example-build` 在 iOS 模拟器构建后用 `nm -g` 找 `aimux_openai_new`,是 xcframework 强制链接(#25 / #26)的回归门禁 |

核实后**不做**:按模态加 cargo feature(一行不删,只加 CI 轴,`search` 那条会让默认 feature 的 `bindings/node` workspace 编译失败);删 `catalogue.rs` 的模型目录(产品功能,四种 binding 都暴露)。

### 4.1 provider 数据管线(四层与分级)

A1 删掉的是"生成 provider 文件"的一次性脚本。它们的产物(薄封装 `.rs`)是 phase 4 之前的形态,注册表落地后加 provider 只改 `provider_registry.json` 一行(#22 InferenceHub 即如此)。但注册表本身没有维护机制,251 这个数字也不诚实:2026-09-04 的测量,只有 19 行有真实录音 cassette,97 行在 models.dev 有第二来源,81 行除 7 月那次 LiteLLM / mastra 扫描外没有任何来源;138 行只出现在 `openai_compatible_test.rs` 的表驱动 wiremock 测试里,共用同一份假响应。与 models.dev 比对,97 个共有名字里 27 个 `base_url` 不一致,116 个 models.dev 厂商 aimux 没有。

参照 pi-ai(`packages/ai`,39 个 provider):协议实现在 `src/api/`;厂商是 `src/providers/<id>.ts` 十来行手写工厂;模型知识由 `scripts/generate-models.ts` 从 models.dev 生成到 `data/<id>.json` + `<id>.models.ts`,生成物首行 "auto-generated / do not edit",`.manifest.json` 记 schema 版本与 hash,`check-model-data.ts` 在 `build` 第一步校验;README 有七步 "Adding a New Provider" 清单。**pi 从不生成 provider,它生成的是 model 数据。** pi 窄而深,aimux 宽而浅;宽是 aimux 的卖点,不砍,但要让宽度诚实。

#### 四层

按"谁维护、变更走什么流程"划分,不按文件位置:

| 层 | 内容 | 维护者 | 变更流程 | 门禁 |
|---|---|---|---|---|
| L0 协议实现 | `aimux-providers/src/<protocol>/`,12 个目录 | 人,手写 Rust | PR 审查 | cassette conformance |
| L1 厂商身份 | `provider_registry.json`,一厂商一行:`name` / `display` / `tier` / `protocol`(B1)/ `base_url` / `auth`(#174)/ `profile` / `params` / `note` / `status` | 人,手写数据 | PR 审查;永不重生成 | `provider.rs` 的 Rust 测试校验结构;每周 diff 报告、每月探活(非阻塞) |
| L2 模型知识 | `aimux-providers/data/models/<provider>.json` + `.manifest.json`,源 models.dev(主)、anya2a(辅),合并手写 `models.overrides.json` | 机器,`scripts/gen_models.py` | 每周 Action 重生成,有 diff 开 PR,人审合并 | `gen_models.py --check`(hash 比对,不联网),PR 阻塞 |
| L3 派生物 | `ProviderName` ×8、`docs/api/providers.md`、ts-rs 类型 | 机器,三个生成器 | 跟随 L1 自动重生成 | 各自 `--check`,PR 阻塞 |

依赖单向:L1 的 `protocol` 引用 L0;L2 按 `name` 挂在 L1 上;L3 从 L1 派生。L0 / L1 是人维护的事实源,L2 / L3 是机器维护的派生物。由此得到统一答案:

- 只有 L2、L3 可以生成。A1 删的三个生成器错在想生成 L1。
- 所有 L2、L3 产物必须带 do-not-edit 标记并有 CI `--check`;不满足的生成器不准进 `scripts/`。L1 是源,用测试校验结构,不用 `--check`。
- PR CI 与构建永不联网;联网只在 Action 的定时任务里(L1 diff、L2 刷新、探活)。
- RFC-0032 的 `models[]` 覆盖层归 L1(某模型的请求行为:协议、profile);L2 是知识(context、价格、能力)。两者按 model id 关联,不合并,RFC-0027 "runtime 与 spec 不合并"的决定不变。
- auth(§8.6)描述在 L1 的 `auth` 字段,执行在 provider-utils 的 `apply_auth` / `resolve_credential`。
- 运行时是四层的消费者:`provider(name)` 读 L1 找 L0;`catalogue()` 读 L2(静态,`include_str!` 或按需加载,另开 PR 接线),`get_model_specs(url)` 保留为显式刷新,对应 pi 的 `refresh()`。
- 与 pi 的两处差别:L1 用数据不用代码(251 家对 39 家);L2 提交进仓库、定时 PR 刷新,而不是构建时联网(crate 发布与 7 种 binding 的 CI 都要离线可构建)。

#### 分级

L1 每行一个 `tier`,回答"凭什么在名单里":

| tier | 定义 | 2026-09-04 | 升级路径 |
|---|---|---|---|
| `verified` | 有真实录音 cassette,探活通过 | 19 | — |
| `listed` | 至少一个上游(models.dev / anya2a / LiteLLM)确认身份,探活通过,无真实录音 | 124 | 录一份 cassette |
| `unverified` | 只有 7 月扫描一个来源 | 108 | 每周 diff 报告找到上游背书 |
| `status: unreachable` | 探活连续两月失败,附加标记,不删行 | 0 | 人审后决定删除 |

初始定级用 `catalogue.rs::normalize_provider_name` 把 models.dev / anya2a 的连字符 id 折到注册表的 snake_case 名字上(`alibaba-cn` + `alibaba`、四个 `stepfun*` 合并),否则 models.dev 几乎匹配不上。L2 首次生成:137 个厂商、4,872 个模型、2.7 MB,`data/` 从发布的 crate 里 `exclude`,直到 Rust 侧有消费者。

文档表 `verified` 在前、`unverified` 默认折叠;`ProviderName` 枚举包含全部四级(不破坏 API)。aimux 有意与上游不同的行加 `note` 说明,diff 报告跳过带 `note` 的行。

#### 脚本纪律与保留清单

- 生成器:`gen_provider_names.py`、`gen_ts_types.py`(已合规)、`gen_providers_doc.py`(#172 补 `--check`)、`gen_models.py`(新)。
- 维护:`sync_registry.py --report`(吸收 `extract_litellm_bases.py`、`scan_litellm_urls.py`)、`probe_registry.py`,由 `.github/workflows/registry-maintenance.yml` 定时驱动。
- 过渡期保留:`generate_thin_wrapper_cassettes.py`(cassette 来源,随 B2 删)、`responses_similarity_audit.py`(随 B6 删)、两个 cassette 转换器(长期,测试数据来源)。
- 流程:`docs/contributing/adding-a-provider.md`,三种情况各列要碰哪几层、跑哪几个命令。

跟踪:管线 PR(本节全部)、一次性清账 #171、`gen_providers_doc.py --check` #172、adding-a-provider 文档 #173、原 #170 扩为 L1 diff + L2 刷新 + 探活;A1 本身 #168 / PR #169。

## 5. 轨道 B · providers 协议化

骨架是 RFC-0032:17 个 `LanguageModel` 归成 6 到 7 个协议臂,注册表行加 `protocol` / `auth` / `params` / `models[]`,33 个 wrapper 退役,responses 家族合并。

### 5.1 写回 RFC-0032 的修正

1. **第 5 步依赖第 1 步,不是并行。** 今天 `provider_handle()` 只返回 chat completions 的 `OpenAIProvider`,`Provider::name()` 硬编码 `"openai"`,responses 的 `provider_options` 键也硬编码 `"openai"`。open_responses / HF / xai 的 responses 合并全踩在这三处上。
2. **responses 请求构造器不应用 `body_overrides`。** 只有 chat 构造器应用(`openai/convert.rs:1500`)。codex 的 `store: false` 靠 flag 传进去要先补这个口。
3. **xai chat 和 mistral chat 不是纯 fork。** xai:HTTP 200 里的扁平错误体 `{"error":"msg"}`、缺 usage 时的零值、非包含式缓存 token。mistral:content 数组、`tool_choice: any`、`model_length` 结束原因。这些是 profile 要新增的钩子;openai 的 `ChatCompletionResponse` 字段没有 `serde(default)`,直接换路径会解析失败。合并后可删 500 到 1,700 行,取决于 `convert.rs` 是否折进去,不是固定 650。
4. **`vertex_ai_*` 合成一行会丢 eu / us 区域主机推导。** 占位符只支持 `{project}` 一类静态替换;`{location}` 要按 global / eu / us / 其他推导三种主机形态。`bedrock_mantle` 同理丢 `AWS_REGION`。
5. **profile 要能从 JSON 反序列化。** `OpenAICompatProfile.stream_usage_key: Option<&'static str>` 装不进注册表行;先换成 owned 字符串的 serde 结构(pi-ai 的 compat 形状)。
6. **删 `mistral/model.rs`、`xai/model.rs` 会让 workspace 编译失败。** `aimux-ffi/src/lib.rs`(1190–1242)、`bindings/node/src/lib.rs`(1127、1165)、`bindings/python/src/lib.rs`(497、518)、`tools/aimux-web/src/model_builder.rs`、`tools/aimux-cli/src/probe/provider.rs` 直接调 `MistralProvider::model()` / `XAIProvider::model()`。C2 的转调 shim 和这五处改 `provider()` 必须排在 B4 / B5 之前。
7. **注册表行不是 7 行。** 加了 `protocol`、嵌套 `auth{kind,env}`、`params{}` 后一行约 11 行,77 个新条目按 850 行算;responses 家族 4,557 行与目标实现只共享 95 到 190 行,参数化重写按 1,000 行算。
8. **`ProviderRecord` 应等于解析后的注册表行 + protocol。** 这样 RFC-0023 的 `rebuild_provider` 退化为 `provider(name, key)`,27 个 `config_snapshot` 覆盖和 OpenAI 兼容白名单一起消失(见 §8.3)。这条要写进 RFC-0032 的改动面。

### 5.2 PR 序列

| PR | 做什么 | 删 / 建 | 核实 | 门禁 / 会丢什么 |
|---|---|---|---|---|
| B1 | `Protocol` 枚举、`AuthKind`、`params`、每协议一个 `XxxConfig::from_resolved`;`provider("anthropic", …)` 可用;`ProviderRecord` 加 `protocol`。纯新增 | 0 / +350 | RFC-0032 §3 | cassette 全绿。同 PR 把 `Provider::name()` 改成返回配置里的名字(1 行),否则 B2 后 openrouter 报 `"openai"` |
| B2 | `auth.kind = none / bearer_env`,`OpenAIConfig.api_key` 允许为空且不发 Authorization(单独 commit 加测试);33 个 wrapper 变注册表行,删文件、`delegate_list_models!`、`thin_wrapper_config_test.rs` | −4,300 / +850 | 改法 | 丢 `OLLAMA_BASE_URL` 一类"env 里是 URL"的约定,加 `base_url_env` 找回;vertex `{location}` 推导先做;`params` 展开要在 `base_url_has_placeholder()` 拒绝 `{` 之前;`replay.rs` 白名单只删 20 个名字,保留 `"openrouter"`;44 个 pub 类型消失,Rust API 破坏 |
| B3 | compat flag 改 serde 结构;CI 加 grep:`openai/ anthropic/ google/ bedrock/ cohere/` 里不许出现 provider 名字字面量(今天 `openai/` 已有 14 个 `"groq"`) | 0 / +200 | 两位评审要求 | pi-ai 没这条,漂成 25 个 flag + 31 个 `provider ===` 分支 + 19 处 baseUrl 嗅探 |
| B4 | mistral chat 走 `openai::model` 的 `execute_*`,4 个 profile 钩子;删 `mistral/{model,convert,types}.rs` | −530 至 −1,160 / +150 | 改法 | 前置 C2 与五处调用点。`execute_*` 内部调 openai 的 `build_request_body`,mistral 构造器会成死代码:要么把 content 数组和 `tool_choice: any` 折进 openai 做 flag,要么只删 `model.rs`。76 个测试的 reasoning id 从 `rc-{nanos}` 变 `reasoning-0`;86 个 cassette 字节比对 |
| B5 | xai chat 同上:`response_handlers`、citations → `Source`、`search_parameters`、200 里的错误体、可选字段 `serde(default)` | −490 至 −1,700 / +250 | 改法 | 前置同 B4。`xai/responses/convert.rs` 从 `xai::convert` 引了 `remove_additional_properties_false`、`supports_reasoning_effort`(21 行),先搬进 `openai/`。86 个测试固定的 `text-{chunk_id}` / `xai-source-N` id 和零值 usage 全变;62 个 cassette 字节比对 |
| B6 | responses 家族。先补三个前置(`build_headers` 忽略 `config.headers`、`provider_options_name()` 读配置、构造器应用 `body_overrides`),再按序:azure 壳(−203)→ `open_responses.rs` 变注册表行(−1,396)→ HF(−1,264)→ codex 订阅循环(−259)→ xai responses 只删流循环、保留 provider-tool 适配(−400) | −3,500 / +1,000 | 改法 | HF 不是 open_responses 的子集:`{"huggingface":{"itemId"}}` 元数据、`response.created` → `ResponseMetadata`、`mcp_call` 条目、base64 媒体类型嗅探要进共享实现,否则丢 22 项行为;xai 的 `reasoning_text.delta`、`response.done` 在共享 reducer 加两臂。360 个测试重定向。直接删 `open_responses.rs` 被驳回:会丢 LM Studio 的 Responses 端点 |
| B7 | vertex:`anthropic_model.rs` 走 `anthropic_*_core`(anthropic_aws 已如此),core 加 `failed_response_handler` 参数;`vertex/model.rs` 走 google 的 `execute_*` 缝 | −870 / +250 | 改法 | google core 8 处硬编码 `"google"` 元数据命名空间,vertex 用 `"googleVertex"`,要参数化;流内 `{type:error}` 后 vertex 今天不发 `Finish`,core 会发,是行为变化 |
| B8 | 6 份相同的 `build_header_list`(`openai/image.rs` 那份故意不带 Content-Type,保留)、AWS 凭据加载去重、3 份"2xx 里是 JSON 错误"守卫上收到 provider-utils | −170 / 0 | 改法 | 错误文案里的 provider 名字作为参数 |
| B9 | 32 个单模态 provider 改 §8.2 的执行器模式 | 见 §8.2 | 改法 | 依赖 B2 的 `auth: none` 与 `base_url_env`(searxng 没有 key 只有 `SEARXNG_URL`,今天 `resolve_key` 对空 env_var 直接报错) |

## 6. 轨道 C · FFI 收口

三份设计里评审选了"从 C ABI 往里设计":一个 `spec_json` 构造任意模型,一个 `call(op)` 和一个 `stream(op)` 承载所有操作,错误是一段 JSON。今天 109 个导出(分支 115),目标 9 个。旧导出与新导出并存一个次版本,让 7 个 binding 的迁移不必挤进同一个发布。

```c
typedef void (*aimux_event_cb)(const char* event_json, void* ctx); /* NULL = 流结束 */

char*    aimux_model_new(const char* spec_json, uint64_t* out_handle);
char*    aimux_session_new(uint64_t model, uint64_t abort, const char* opts_json, uint64_t* out);
char*    aimux_call(uint64_t handle, const char* op, const char* request_json, char** out_json);
char*    aimux_stream(uint64_t handle, uint64_t abort, const char* op, const char* request_json,
                      aimux_event_cb on_event, void* ctx);
char*    aimux_session_push(uint64_t session, const uint8_t* data, size_t len);
uint64_t aimux_abort_new(void);
void     aimux_abort(uint64_t abort);
void     aimux_drop(uint64_t handle);
void     aimux_free_string(char* s);

/* spec_json: {"provider":"anthropic","model":"claude-sonnet-4-5","kind":"language",
               "api_key"?,"base_url"?,"headers"?,"protocol"?,"params"?:{region,project,location,…}}
   复合模型是虚拟 provider: {"provider":"router","models":[h1,h2],"config":{…}}
   错误 JSON 见 §8.1 */
```

op 是字符串,按 handle 类型分组:语言模型 `generate_text` / `stream_text` / `generate_object` / `generate_text_as_openai` / `stream_text_as_openai`;单模态 `generate_image` / `generate_speech` / `transcribe` / `embed` / `rerank` / `search` / `generate_video` / `upload_file`;handle 0 上的 `configure` 承接日志、录制、trace、session、代理、外部 provider 注册、模型目录;`session_*` 承接 RFC-0028 的实时转写。op 表要有穷举测试(每个 op × 每种 handle 类型),并在 `aimux.h` 和各 binding 里导出为常量。

| PR | 做什么 | 删 / 建 | 核实 | 注意 |
|---|---|---|---|---|
| C1 | 新增 9 个导出和 `dispatch.rs`(op → aimux-core 调用,JSON 进出),与旧导出并存 | 0 / +2,200 | 设计已核实 | `lib.rs` 重写按 1,150 行算(导出之间 858 行辅助函数仍需要),`dispatch.rs` 300,头文件加测试 720 |
| C2 | 40 个按 provider 的构造函数改为对 `model(spec)` 的一行转调(符号不变) | −682 / +120 | 改法 | 保留 api_key 为 NULL 时的确定性错误(Java / Kotlin / Swift 测试拿它当"真实 C ABI 错误"样本);bedrock 的 key / secret / region、vertex 的 project / location 走 `params`。同 PR 重写 `exports_smoke_test.rs`(1,102 行,引用这 40 个符号 29 次);`aimux-ffi.h` 手写,40 条声明手动删 |
| C3 | 16 个 `aimux_error_*` 取字段函数换成 `aimux_error_json`,保留 code / message 给 C 调用者 | −766 / +40 | 改法 | `retry_ms` 是 `retry_after_hint()` 现算的(含 HTTP 日期解析),不在 serde JSON 里,信封要加派生字段 `retry_after_ms`;分支上还有 6 个 Retry 取字段函数,信封要带 Retry 变体 |
| C4 | 删旧导出、旧头文件、2,073 行旧 FFI 测试;trace / session / recording / mock / router / moa 家族不删,改成 op | −7,000 / 0 | 改法 | 直接删 `aimux_trace_*`、`aimux_session_*` 被驳回:Go 和 Flutter 文档把它们当五种纯 C ABI 语言的唯一入口。转 op 后 C ABI 仍可达。`next_part` 的 `out_state` 保持原样,不套 JSON 信封(忙轮询路径) |

核实过的两点:一次性转写和文件上传改成 JSON 里的 base64 不多一次拷贝,这两个导出今天就是 base64 的 C 字符串(`lib.rs:2126`、`:2381`);流式路径的裸字节由 `aimux_session_push` 保留。复合模型的子 handle 以整数放在 spec 里,构造时要像 `aimux_router_new` 一样 Arc-clone 子模型,加"构造后 drop 子 handle 仍可生成"的测试。

## 7. 轨道 D · bindings

收口到 9 个符号之后,每种 binding 只剩四个对象:`Model.new(spec)`、`model.call(op, request)`、`model.stream(op, request, abort?)`、`AimuxError`。类型镜像保留为可选的类型化外层,改为生成(D8)。

| 语言 | 外观层今天 | 之后 | 路径 | 核实 |
|---|---|---|---|---|
| Go | 2,965 | 450 | cgo 不变;删 44 个 `New*` 和逐字段错误解码 | 已量 |
| Java | 3,450 | 600 | JNA 声明 91 → 9;一个 `Model` 类。100 个 Builder 内部类(1,594 行)可删,前提是私有全参构造器改 public 或升 Java 17 用 record;九个模态类的 handle 样板抽一个 `NativeHandle` 基类再省 192 行;工厂里 40 处 `requireNonNull` 是文档承诺的 NPE 契约,保留 | Builder 改法 |
| Kotlin | 4,744 | 300 + 350 | 原提案"整体改为依赖 aimux-java"被驳回:`Errors.kt` 的 sealed `AimuxException` 是文档承诺的穷举 `when` 契约,Java 8 没有 sealed。改法:JNA 接口、Model 外观、多模态外观(约 1,700 行)改依赖 Java 工件;Kotlin 保留 sealed 异常层(350 行)作为映射层,加协程 / Flow 糖;CI 加 kotlin → java 工件依赖边。§8.1 落地后这 350 行也可以缩成对 `kind` 的 when | 改法 |
| Swift | 1,832 | 400 | 删 31 个静态工厂和 13 个 `fromC` getter;`AsyncThrowingStream` 包 `aimux_stream` | 已量 |
| Flutter | 2,795 | 630 | 91 个 dart:ffi 查找减到 9。过渡期先用 ffigen 从头文件生成声明(−458 行,RFC-0001 §227 已预授权,91 个符号头文件里都有原型)。两处手写:`openAimuxLibrary` 加载器(iOS 的 `DynamicLibrary.process()` 路径)和 `dropHandlePtr`(把 `aimux_drop_handle` 重解释为 `NativeFinalizer`) | 改法 |
| Node | 3,514 | 1,200 | 保留 napi-rs(ctypes 式调用会卡事件循环),改为 rlib 依赖 `aimux_ffi::dispatch`,与 C 路径共用 op、spec、错误 JSON。`aimux-ffi` crate-type 已含 rlib;但 node 今天完全绕过 aimux-ffi,是新依赖边,napi 的 tokio 桥与 FFI 全局 runtime 的重入守卫是否冲突未量过。三个 Rust 粘合 crate 共用的"绑定核心"核实后只有 1,500 行(prompt / options 解析、spec、dispatch);Node 的结构化错误类要拿 `Env` 才能构造 JS 异常,`AimuxResult` 和流泵留在 napi 侧 | 改法 |
| Python | 2,611 | 400 | 设计提议整个 pyo3 crate 换 stdlib ctypes。两位评审要求先做原型:流式要 helper 线程 + queue,ctypes 回调里的异常会被吞,wheel 要按平台捆绑 `libaimux_ffi`。原型不过退回"pyo3 薄层调 dispatch" | 先原型 |

PR:D1 Kotlin(只依赖 Java 工件,不依赖新 ABI,可最早做)→ C1 落地后 Go、Java、Swift、Flutter、Node 各一个 PR(D2–D6)→ Python 原型通过后(D7)→ D8 类型镜像生成。每个 binding PR 只跑自己那条 CI job。

**D8 类型镜像改生成。** aimux-core 的 serde 类型只被机械导出一次(ts-rs → `bindings/node/src/types`,CI 用 `--check` 守着),然后在 Java、Flutter、Swift、Kotlin、Go、Python 六种语言里各手写一份镜像,加 aimux-web 的副本共 18,944 行。每个文件头都写着它抄自哪份 ts 输出;PR #164 的错误类型改动要碰 8 种语言,根源在这。不能先删后补:Go 的 8 个多模态入口只收类型化的 `*XxxCallOptions`,Python 的 `wrapper.py`(1,221 行)要么整个走要么整个留。做法是从同一份 `#[derive(TS)]` 再导出 JSON Schema,按语言生成,把 `gen_ts_types.py --check` 扩展到六种输出。行数不减,手工维护归零。

## 8. 设计层面

### 8.1 错误模型跨语言扁平化(参照 OpenDAL)

**问题。** `AiMuxError` 是 13 个带内容的变体,`ApiCall` 里再套 6 字段的 `ApiCallError`。这个形状穿过 FFI 时 C 侧要 16 个取字段函数,每种语言再把 13 个变体各建一个子类来接,合计约 2,700 行,而且每次 core 加一个变体要改 8 种语言。

**OpenDAL 的做法(已核对源码)。** Rust 侧 `Error { kind: ErrorKind, message, context }`,`ErrorKind` 13 个标签不带内容。C 侧 `opendal_error { code, message }` 两个字段(126 行);Python 13 个只有 message 的异常类加一张 match 表(128 行);Java 一个 `OpenDALException` 加 `Code` 枚举(77 + 159 行);Go `Error { code, message }`(106 行);Node 直接 `napi::Error::from_reason(字符串)`。四种 binding 合计 596 行。

**设计。** Rust 侧的 enum 不动。跨 FFI 边界只传一个 JSON 结构:

```json
{
  "kind": "api_call",            // 13 个标签之一,与 AiMuxError 变体一一对应
  "message": "429 Too Many Requests from groq",
  "status": 429,                 // 可选,以下均可选
  "provider_code": "rate_limit_exceeded",
  "request_id": "req_…",
  "response_body": "{…}",        // 截断到 4 KiB
  "retry_after_ms": 2000,        // retry_after_hint() 派生
  "retryable": true,
  "data": { "model_id": "…", "provider_id": "…", "available": [] }   // NoSuchModel 等变体的附加字段
}
```

每种语言一个异常类加一个 `kind` 字段(Go 是 `Code()`),Kotlin 想保留 sealed 层级就按 `kind` 映射,一张表。C3 的 `aimux_error_json` 直接返回这段 JSON;C 调用者保留 `code`(kind 的整数)和 `message` 两个取字段函数。

**具体场景。** 用户通过 groq 调 `generate_text`,groq 返回 429 并带 `retry-after: 2`:

1. `openai_chat` 协议实现把响应交给 `ResponseHandler`,产出 `AiMuxError::ApiCall(ApiCallError { status_code: 429, provider_code: Some("rate_limit_exceeded"), retry_after_ms: Some(2000), … })`。Rust 用户 `match` 变体,不变。
2. Core 的重试按 `retry_after_ms` 等 2 秒重试;超过次数后错误穿出 `generate_text`。
3. FFI 的 `aimux_call` 返回非空 `char*`,内容是上面那段 JSON。
4. Python:`except AimuxError as e: if e.kind == "api_call" and e.retry_after_ms: …`。Go:`var ae *aimux.Error; errors.As(err, &ae); ae.Code() == aimux.CodeApiCall`。Kotlin:`when (e) { is AimuxException.ApiCall -> e.retryAfterMs }`,子类由 `kind` 表构造。
5. 录制里 `OutcomeRecord` 存同一段 JSON,回放时原样重现。

**代价。** 公开 API 破坏(binding 的错误类构造方式变);穷举匹配从类型变成对 `kind` 的匹配,编译器不再替你查全。golden test 固定的 `AiMuxError` serde JSON 作为 `data` 字段原样保留,不破坏。

### 8.2 单模态 provider 的执行器模式(参照 LiteLLM)

**问题。** 32 个单文件单模态 provider 约 15,000 行,28 个有自己的 `build_headers`,22 个有自己的 URL 拼接,4 个(fal、black_forest_labs、klingai、runwayml)各写一遍提交加轮询;fal / luma / BFL / revai / gladia 的 submit + poll 在同一个 `do_generate` 里,Core 重试会重新提交计费任务(video 已修的同一个 bug)。

**LiteLLM 的做法(已核对源码)。** 每个模态一个基类(`BaseImageGenerationConfig` 110 行),provider 只实现五个纯函数:`validate_environment`(headers)、`get_complete_url`、`transform_request`、`transform_response`、`get_error_class`;HTTP 收发、重试、轮询在共享执行器里。Recraft 的图像生成整个是 153 行。

**设计。** provider-utils 加两个执行器:

```rust
/// 一次 POST,JSON 或 multipart,走统一的 ResponseHandler。
async fn call_json<Req, Resp>(cfg: &HttpConfig, url, headers, body: Req) -> Result<Resp, AiMuxError>;
/// 提交 + 轮询:submit 只发一次,重试只作用于 poll;done 判定和超时由调用方给闭包。
async fn submit_and_poll<Job, Resp>(cfg, submit: Request, poll: impl Fn(&Job) -> Request, done: impl Fn(&Value) -> Poll<Resp>, backoff) -> Result<Resp, AiMuxError>;
```

每个单模态 provider 缩成 `XxxConfig { api_key, base_url }`(B2 之后可由注册表行表达,含 `auth: none` 与 `base_url_env`)加两个转换函数。预期每家从 300 到 800 行缩到 100 到 200 行,共约 −8,000;功能一个不少,`submit_and_poll` 顺手修掉重复提交计费任务的 bug。AI SDK 自己是逐家手写的,不作参照。

### 8.3 录制回放收敛(#167)

RFC-0023 标为 IMPLEMENTED,但在四处停在 MVP 边界:mock 回放只解析 OpenAI chat.completions wire(决策 R8,其他协议 `Unsupported`,单模态无法回放);`rebuild_provider` 只覆盖 OpenAI 兼容族(白名单)且 27 个 provider 各自覆盖 `config_snapshot`;三种记录格式并存(cassette、Recording、TraceRecord),两套匹配器;P5 绑定透传只做了 C ABI 和 Node。

设计(详见 #167):**回放在传输层,不在模型层**——把录制的 HTTP exchange 挂到 mock transport,跑真实的协议代码,任何协议、任何模态、流式与否都能字节级回放,`MockReplayModel` 的 OpenAI 专用解析(约 1,500 行)消失;**`ProviderRecord` = 解析后的注册表行 + protocol**,rebuild 退化为 `provider(name, key)`;**`Recording.http[]` 用 cassette 的 exchange schema**,测试与运行时共用一个匹配器,aimux-web 录下来的可以直接入库当 cassette;Router / MoA 记录实际选中的子模型;加 `PassthroughOnMiss`;录制 / mock / session 走 C1 的 op,7 种语言同一面。

### 8.4 子系统不砍,改暴露方式

| 子系统 | 核心行数 | 今天怎么暴露 | 之后 |
|---|---|---|---|
| 缓存命中探测 trace(RFC-0015 / 25 / 29) | 2,642 + cli 919 + web 8,330 | FFI 7 个导出,各 binding 手写 | `configure` op 的 `trace.*` 子命令;aimux-web / cli 直接调 core,不经 FFI |
| session 归组(RFC-0024) | 715 | FFI 3 个导出 | `configure` op |
| recording / mock(RFC-0023) | 4,549 | FFI 4 个导出 | `configure` op;实现按 §8.3 |
| router / MoA(RFC-0021 / 22) | 1,440 | `aimux_router_new` / `aimux_moa_new` | spec_json 的虚拟 provider |
| OpenAI 输出格式(RFC-0026) | 1,639 | 3 个导出 | `generate_text_as_openai` / `stream_text_as_openai` 两个 op |
| 模型目录(RFC-0027) | 906 | 4 种 binding 各自暴露 | `list_models` op + `catalogue` op |

每一块的功能面不变;变的是它不再各自穿一遍 FFI 和 7 种语言。

### 8.6 auth 层(今天不存在)

注册表 251 行全是 `env_var`,语义只有"读环境变量当 API key 发 Bearer"。手写侧只有零件:`codex.rs` 的无状态 `codex_refresh`(RFC-0018 §3.2 把登录、持久化、刷新编排都留给集成方);`bedrock/sigv4.rs` 静态凭据签名,被 bedrock / anthropic_aws / aws_polly / bedrock_mantle 各自调用;`vertex/mod.rs` 读 `GOOGLE_VERTEX_ACCESS_TOKEN` 或 Express key,无 ADC、token 过期即死;azure(`api-key`)、google(`x-goog-api-key`)、lmnt / exa / parallel / you.com / tinyfish(`x-api-key`)各自拼 header。12 处 header 代码,没有共享函数。

pi-ai 的 `src/auth/`:`types.ts` 定义 `ApiKeyAuth` / `OAuthAuth` / `CredentialStore` / `AuthEvent`;`resolve.ts` 定解析顺序(显式 key → env → 凭据库);`oauth/` 8 个厂商的 device-code / PKCE 流程;`credential-store.ts` 内存实现供集成方替换。

分三级,前两级做(#174、#175),第三级记 open question:

1. **数据描述(并入 B1 / B2,RFC-0032 §3.3 已有草案)。** `env_var` 升级为 `"auth": {"kind", "env", "header"?}`,kind ∈ `api_key`(`header` 缺省 Bearer,可选 `x-api-key` / `api-key` / `x-goog-api-key`)、`none`(本地服务,退役 33 处 `PLACEHOLDER_API_KEY`)、`sigv4`(`params` 带 region)、`bearer_token`(外部提供 access token,vertex)。provider-utils 加 `apply_auth(headers, &AuthSpec, &Credential)`,替掉 12 处手写 header。净删。
2. **运行时解析与刷新(独立 PR)。** `Credential` 枚举(ApiKey / BearerToken{expires_at} / Aws{..});`resolve_credential(spec, explicit, env, store)` 固定顺序;`CredentialStore` trait + 内存默认实现;`codex_refresh` 泛化为 `TokenRefresher` 钩子,任何 `bearer_token` provider 遇 `TokenExpired` 走同一条刷新重试路径,vertex 可挂。对应 pi 的 `resolve.ts` + `credential-store.ts`,不含登录。
3. **登录流程(不做,open question)。** pi-ai 把 device-code / PKCE 放进库;aimux RFC-0018 选了相反立场。aimux 跨 7 种 binding,交互式登录要每种语言暴露事件流,成本远高于单语言。维持 RFC-0018:第 2 级做完,集成方登录后把 token 放进 `CredentialStore` 即可。

### 8.5 测试策略(待定,先研究再动)

本节只记录数据与选项,**不在本轮删任何测试代码**。决定前要看 pi-ai、AI SDK 以及其他成熟 SDK 的做法。

数据:§3.3。112 个 wiremock 手写文件里,fork 组 15,370 行与 openai chat 测试同构,responses 家族 10,024 行是五份副本各测一遍。

已知的三种做法:

| 做法 | 谁在用 | 特点 |
|---|---|---|
| 逐 provider 手写 mock(翻译自 AI SDK 每个 package 的测试) | aimux 今天、AI SDK | 覆盖细,但 fork 与协议同构的部分完全重复;协议改一处要改 N 份 |
| 协议 × 能力矩阵,真实 key 打 live,env 门控 | pi-ai(30 套) | 无 mock,反映真实行为;不可离线,CI 要 key,漂移由测试发现 |
| cassette 回放 + 表驱动 conformance | aimux 的 6 个横切文件 | 离线、字节级、确定;cassette 要有刷新手段 |

协议化之后自然的形状:一个协议一套行为测试(openai chat 已有 6,400 行);一张 `(provider, protocol, cassette 目录)` 表驱动 conformance,251 行注册表每行一条;profile flag 的差异各一个测试;可选加 pi-ai 式 live 矩阵,同一套测试带 `--record` 就刷新 cassette。若采用,fork 组可删约 13,000、responses 组约 7,500、vertex 重复约 2,000,合计 −22,000 到 −25,000 行测试代码。**是否采用、采用多少,等研究结论。**

## 9. 轨道 E · PR #164 内部

合并前在分支上做,一个 PR,现有 CI 覆盖:

- `docs/ai-sdk-request-pipeline.md` 1,037 行只留 §1–§3 和 §14 约 115 行,搬到 `rfc/0031`。§4–§10 复述 rustdoc,§11–§13 是已完成的迁移清单。−920
- 八个模态的 `generate_*` 包装和 `generate_text` / `stream_text` 重复同一段前奏(timeout、`prepare_retries`、`RecordingContext`、session、span),抽一个 `run_operation`。−150
- abort 处理写了六遍(retry 循环、`retry::delay`、`timeout::run_until`、`sleep_or_abort`、`send_one_request`、两个 body reader)。所有入口都在 `timeout::run_until` 里,drop future 即取消;留一个。−120
- `StreamTextResult::text()` 等于 `consume().map(|a| a.text)`。−45
- `PreparedRetries` 是 `RetryConfig` 换单位;泛型 `retry_with_exponential_backoff` 生产只有一个调用方。−70
- provider-utils 照搬 TS 的一文件一函数(`get_from_api.rs` 25 行、`handle_fetch_error.rs` 21 行、`retry.rs` 3 行 re-export 等)并进 `http.rs`。−90,−7 文件
- 两个相同的 body 读循环合一。−50

## 10. 扫描中发现的缺陷与遗漏(修复清单)

与删减无关但要修的,按严重度:

1. **重试重复提交计费任务。** #164 已把 assemblyai / gladia 改成逐阶段重试,并核实 fal / luma / black_forest_labs / revai / replicate 等的计费后阶段已各自重试;但每个 provider 手写一套脚手架,§8.2 的 `submit_and_poll` 仍是收口。
2. **`DEFAULT_MAX_DOWNLOAD_SIZE = 2 GiB` 读进 `Vec`**。#164 已给 JSON 成功体单独的 64 MiB 默认上限并允许按请求覆盖;二进制下载仍是 2 GiB。
3. **responses 家族 `build_headers` 忽略 `config.headers`**;responses 请求构造器不应用 `body_overrides`(只有 chat 应用,`openai/convert.rs:1500`)。B6 前置。
4. **`Provider::name()` 硬编码 `"openai"`**,responses 的 `provider_options` 键也硬编码;注册表 provider(openrouter 等)全部自报 `"openai"`,录制的 `ProviderRecord.provider` 因此不可信,`rebuild_provider` 只能靠 base_url 反推厂商。B1 修。
5. **`openai/` 目录里有 14 个 `"groq"` 字面量**,协议实现里嵌了 provider 分支。B3 的 CI grep 防止再长。
6. **`is_retryable()` 与 `retry.rs` 的内联谓词写了两遍**,已经可以不一致。
7. **`aimux-ffi.h` 手写,无 cbindgen,无漂移门禁**;109 条声明与 `lib.rs` 之间没有任何检查。C1 起用 cbindgen 或加 `--check`。
8. **aimux-web 的 `HttpRecord.ts`、`JsonValue.ts` 已与 node 的生成版漂移**(A4)。
9. **`gen_provider_registry.py` 从未可运行**(读的文件在同一 commit 被删),`provider.rs` 的注释仍指向它(A1)。
10. **`vertex_ai_*` 的 eu / us 主机推导和 `bedrock_mantle` 的 `AWS_REGION`** 是注册表占位符表达不了的,协议化时要先补 `{location}` 推导。
11. **Router / MoA 的录制只有逻辑模型快照**,实际选中的子模型不在记录里,路由调用无法对同一子模型回放(#167)。
12. **`config_snapshot` 覆盖不全**(27 处覆盖,其余走最小默认),RFC-0023 风险表已承认;§5.1 第 8 条后不再需要覆盖。
13. **RFC 编号撞号**:0005 × 2、0027 × 2。不改编号,索引里标清楚(§11)。
14. **`quality-audit/round4/` 里 119k 行的 clippy 日志和 lcov** 是构建产物被提交进仓库。
15. **Node 的 napi tokio 桥与 FFI 全局 runtime 的重入守卫是否冲突未量过**,D6 前先量。
16. **`generate_object` 的 `fix_json` 路径**只有 `json_repair.rs` 一个消费者,与 AI SDK 的 `parse_partial_json` 流式路径没有接上(流式 object 不修复)。记为 RFC-0016 的能力缺口,不在本轮。
17. **`generate_image` 不校验 `n`、不按 `max_images_per_call()` 分批**;`max_images_per_call` / `max_embeddings_per_call` 是必填 trait 方法但零读者。#164 只给 video 补了分批,image 应照搬同一段逻辑。
18. **`VideoOperationStart.response` 七个 video provider 都写、无人读**。要么暴露到 `VideoResult`,要么删字段。
19. **跨语言 fixture 只覆盖 `TimeoutConfiguration`**(`contract-tests/fixtures/wire-format.json`),所以 Kotlin / Go 的 `VideoCallOptions` 缺六个字段、Java `fps` 类型错、Node/Python 三个模态传 options 即报错,CI 都抓不到。D 轨道的类型镜像生成(§7)前,至少给每个 `*CallOptions` 加一条 fixture。
20. **`HttpRequest` 二十余处字面量构造**,每加一个字段(如 #164 的 `max_json_response_bytes`)都要改所有调用点;已 `derive(Default)`,收成 `..Default::default()`。

## 11. 文档整理(不删)

按受众分三层,`git mv` 搬家,链接一次 grep 修完。

- **`docs/` 只留用户要读的**:`README.md` 索引;`guides/` 收 session-affinity、provider-config、codex-subscription、error-model;`api/` 只留 `reference.md` 和生成的 `providers.md`,9 篇按语言的指南各自搬进 `bindings/<lang>/README.md`;`api/gaps.md` 转成 issue 后删;`PERF-RESULTS.md`、`aimux-vs-aisdk-node.md`、`bindings/node/bench` 归 `docs/benchmarks/`。
- **`rfc/` 补索引**:新建 `rfc/README.md`,一张表:编号、标题、状态(已实现 / 已被取代 / 草稿);README 里的 RFC 列表移过来;`docs/ai-sdk-request-pipeline.md` 搬到 `rfc/0031-*.md`;撞号不改编号,索引里标清楚。
- **`archive/` 收历史资料**:`docs/internal/`(105 文件)、`docs/quality-audit/`(34)、`docs/plan/`(17)、`provider-inventory/` 整体移到根目录 `archive/`,一个 README 说明"不再维护,git 历史可查";`docs/internal/cache-tracing/prototype` 只留 README;round4 的 clippy 日志与 lcov 例外删除。
- **语言**:写进 CONTRIBUTING.md——面向用户的 `docs/` 与 `bindings/*/README` 用英文,`rfc/` 保持中文。本轮只搬家不翻译。

## 12. 实施顺序与依赖

```
A1 A2 A3 A4 A5 ──────────────────────────────────────── 随时,互不依赖
                                                          
B1 (protocol enum, from_resolved, ProviderRecord.protocol)
 ├─ B2 (auth kinds, 33 wrapper → rows)  ──┬─ B3 (serde flags + CI grep)
 │                                        ├─ B9 (单模态执行器, §8.2)
 │                                        └─ #167-1 (rebuild_provider = provider(name))
 ├─ C2 (40 ctor → model(spec) 转调) ──────┬─ B4 (mistral)
 │                                        └─ B5 (xai chat, 先搬 21 行辅助函数)
 ├─ B6 (三个前置 → azure → open_responses → HF → codex → xai responses)
 └─ B7 (vertex 走 core)  B8 (零碎)

C1 (9 个导出 + dispatch) ── C3 (error_json, §8.1) ── D2..D6 (Go Java Swift Flutter Node)
                                                   ── D7 (Python, 先原型)
                                                   ── #167-2..6 (录制 / mock / session 走 op)
                                                   ── C4 (删旧导出, 所有 binding 迁完之后)
D1 (Kotlin 依赖 Java 工件) ── 随时
D8 (类型镜像生成) ── D 轨最后
E1 ── PR #164 合并前
文档整理 ── 随时,建议在 A 轨之后一次做完
§8.5 测试 ── 研究结论之后
```

每个 PR 的验收:cassette 全绿;涉及的 binding 只跑自己那条 CI job;删导出的 PR 附 `nm -g` 对比;删测试的 PR(若有)附覆盖率对比。

## 13. 总账

| 轨道 | 删 | 建 | PR 数 |
|---|---|---|---|
| A 机械整理 | −11,100 | +10 | 5 |
| B providers 协议化(含 §8.2 单模态) | −18,000 至 −20,000 | +3,000 | 9 |
| C FFI | −7,000 | +2,200 | 4 |
| D bindings(含 §8.1 错误扁平化) | −24,000 | +4,500 | 8 |
| E PR #164 内部 | −1,500 | 0 | 1 |
| #167 录制回放 | −2,500 | +800 | 6 |
| §8.5 测试(待定) | 0 至 −25,000 | +800 | 待定 |
| 合计(不含测试) | −64,000 至 −66,000 | +10,500 | 33 |

另有 42,599 行库存数据归档。C 和 D 的"删"里相当部分是重写,残留量计在"建"列。

## 14. Risks

- **Rust 公开 API 破坏**(B2 的 44 个 pub 类型、§8.1 的 binding 错误类构造方式)。用一个次版本并存,CHANGELOG 列出迁移表。
- **测试 id 与零值 usage 的变化**(B4、B5 共 162 个测试)。这是实现细节泄漏到断言里,改断言不改行为;cassette 字节比对是真正的门禁。
- **vertex 流内错误后是否发 `Finish`**(B7)是行为变化,要在 CHANGELOG 里写。
- **Node 依赖 aimux-ffi 是新依赖边**,runtime 重入未量。D6 前先写一个最小复现。
- **Python ctypes 原型可能不过**,退路是 pyo3 薄层。
- **协议化后 provider 差异全靠 flag**,pi-ai 漂成 25 个 flag + 31 个 provider 分支。B3 的 CI grep 是唯一的防线,必须先于 B4 落地。

## 15. Open Questions

1. §8.5 测试策略:看完 pi-ai / AI SDK / 其他 SDK 后定。
2. Kotlin 是否在 §8.1 之后放弃 sealed 层级,只留一个异常类加 `kind`。
3. Java 8 与 Java 17 record:Builder 删除的前提。
4. `DEFAULT_MAX_DOWNLOAD_SIZE` 的默认值。
5. `archive/` 是否需要保留在发布的 crate 里(建议 `exclude`)。

## 16. 与既有 RFC 的关系

| RFC | 本规划对它的影响 |
|---|---|
| 0004 provider inventory | 标历史,数据归档(A3) |
| 0012 source dedup | 已被本 RFC 取代 |
| 0017 / 0020 注册表化、外部 provider | 注册表 schema 由 RFC-0032 扩展;外部覆盖层照旧 |
| 0023 录制回放 | 状态改回 PARTIAL,§3.6 按 #167 更新 |
| 0032 协议驱动注册表 | 写回 §5.1 的 8 条修正 |
| 0001 多语言绑定 | §227 的 ffigen 预授权生效;新增 9 符号 ABI 章节(C1) |
| 0003 cassette | exchange schema 成为 Recording 的 HTTP 层(#167) |
| 0026 OpenAI 输出格式 | 暴露方式改为两个 op,格式不变 |
| 0028 转写流式 | `aimux_session_push` 保留裸字节路径 |
