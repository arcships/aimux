# Provider 调研报告（未实现 provider 协议核验）

> **状态**：调研完成，待确认开工顺序
> **日期**：2026-07-28
> **目的**：按 [RFC-0006 §2.1](../../rfc/0006-provider-development.md) 对 inventory 中 `implemented_in_aimux=false` 的 209 个 provider 逐个核验官方协议证据，形成实现路径建议与优先级排序，作为后续开工依据。
> **原则**：inventory 元数据仅作候选线索，**不作为实现依据**；以厂商官方 API 文档/SDK/OpenAPI 为准。证据不足者标"无"并建议搁置，不臆造。

## 1. 方法论

1. **数据来源**：[provider-inventory/providers.json](../../provider-inventory/providers.json) 中 209 个未实现 provider，基础线索已导出至 `_research-input.json`。
2. **分批**：按"分层把握度（L2→L3→unknown）+ 能力复杂度（纯 chat 优先）+ 字母序"排序后分 15 批，每批 13–14 个。批次输入见 `batches/batch-XX.json`，调研记录见 `batch-XX.md`。
3. **每 provider 核验项**（见 `_template.md`）：官方协议证据、协议事实（base URL/鉴权/endpoint/协议类型/请求响应/流式/错误）、实现路径建议、风险与限制、优先级建议。
4. **证据裁决顺序**（RFC-0006 §2.1）：官方文档/SDK > `reference/` 成熟实现 > 多来源一致 > 单一第三方。
5. 调研由 15 个并行 agent 完成，每个 agent 负责一批，各自用 WebFetch/WebSearch 核验官方文档。

## 2. 优先级定义

| 优先级 | 含义 | 判定 |
|---|---|---|
| P0 | 立即可实现 | 证据强 + 薄封装/模态专用 + 有可用模型 ID |
| P1 | 近期实现 | 证据中/强 + 路径明确（薄封装或共享层扩展） |
| P2 | 后续实现 | 证据中/强 + 原生协议或需共享层扩展/模态基建 |
| 搁置 | 暂不实现 | 证据无/弱 / 已有别名覆盖 / 需 core 契约变更 / 已弃用 |

## 3. 全局统计

### 证据强度分布

| 证据强度 | 数量 | 占比 |
|---|---|---|
| 强 | 153 | 73% |
| 中 | 41 | 20% |
| 弱 | 6 | 3% |
| 无 | 9 | 4% |

### 优先级分布

| 优先级 | 数量 | 占比 |
|---|---|---|
| P0 | 6 | 3% |
| P1 | 95 | 45% |
| P2 | 70 | 33% |
| 搁置 | 38 | 18% |

### 实现路径分布

| 实现路径 | 数量 |
|---|---|
| 薄封装 | 133 |
| 模态专用 | 27 |
| 原生 | 20 |
| 共享层扩展/原生 | 10 |
| 待定 | 18 |
| 共享层扩展 | 1 |

## 4. 关键发现

### 4.1 可直接开工的规模

**171 个 provider 有协议证据**（强 153 + 中 41 - 18 待定中部分），其中 **P0+P1 共 101 个**是近期可实现的主力。搁置 38 个（证据不足 15 个、已有别名覆盖 6 个、已弃用 4 个、需 core 契约变更 3 个、非真实 provider 6 个、定位不符 4 个）。

### 4.2 三类结构性阻塞（需先做 core 契约变更）

1. **search trait 缺失**：aimux-core 仅有 language/embedding/rerank/speech/transcription/image/video/files 模态，无 search trait。导致 tavily/serper/exa_ai/firecrawl/linkup/parallel_ai/searxng/google_pse/tinyfish/you_com/dataforseo 等 **11 个 search provider** 被迫搁置或降级。这是**最大的一类阻塞**。
2. **AWS SigV4 签名**：aws/bedrock_mantle/sagemaker 系列、aws_polly 均依赖 AWS SigV4 签名，需共享签名能力。
3. **Vertex AI partner 扩展**：10 个 vertex_ai_*_models 需扩展现有 `vertex` provider 的 publisher 路由 + rawPredict。

### 4.3 薄封装是绝对主流

133 个 provider 判定为 OpenAI 兼容薄封装，占比 64%。这批只需配置 base URL/名称/凭据/profile，复用 OpenAI 共享层，实现成本极低。**首批开工应集中在这批**。

### 4.4 inventory 纠错汇总

调研中发现 inventory 多处错误，已逐条记入各 batch 文件，主要包括：
- **能力标注错误**：suno_api（实为音乐）、vidu/sora/doubao_video/jimeng/kling（实为视频/图像）、jina/jina_ai（实为 embedding/rerank）
- **base_url 错误**：inferx、jiekou、minimax_cn、merge_gateway、moark、baidu_v2、thinkingmachines、tensormesh、synthetic 等
- **环境变量名不一致**：llmgateway、anyapi、gmicloud、hetzner、stepfun 等
- **重复条目**：alibaba_cn/ali→alibaba、ollama_chat→ollama、oobabooga→oobabooba、opencode→opencode_zen、gmi→gmicloud、umans_ai_coding_plan→umans_ai、xiaomi_token_plan 三区域副本
- **非真实 provider**：advanced_custom（new-api 通道类型）、aiproxy/aiproxy_library（已下线）、custom_provider（占位）、midjourney/midjourney_plus（中转误标）、nova（Bedrock 已覆盖）
- **已弃用**：palm/pa_lm/vertex_ai_text_models/anthropic_text（PaLM/legacy 系列已下线）

### 4.5 合并实现建议

多个套餐/区域变体应合并为单一 provider + 配置差异，而非重复接入：
- minimax_cn / minimax_cn_coding_plan / minimax_coding_plan
- alibaba_coding_plan / alibaba_coding_plan_cn
- alibaba_token_plan / alibaba_token_plan_cn
- tencent 六个 token_plan 变体
- xiaomi_token_plan_ams / cn / sgp
- stepfun_step_plan / stepfun_ai_step_plan
- umans_ai / umans_ai_coding_plan

## 5. 分批清单

| 批次 | 文件 | 数量 |
|---|---|---|
| 01 | [batch-01.md](batch-01.md) | 14 |
| 02 | [batch-02.md](batch-02.md) | 14 |
| 03 | [batch-03.md](batch-03.md) | 14 |
| 04 | [batch-04.md](batch-04.md) | 14 |
| 05 | [batch-05.md](batch-05.md) | 14 |
| 06 | [batch-06.md](batch-06.md) | 14 |
| 07 | [batch-07.md](batch-07.md) | 14 |
| 08 | [batch-08.md](batch-08.md) | 14 |
| 09 | [batch-09.md](batch-09.md) | 14 |
| 10 | [batch-10.md](batch-10.md) | 14 |
| 11 | [batch-11.md](batch-11.md) | 14 |
| 12 | [batch-12.md](batch-12.md) | 14 |
| 13 | [batch-13.md](batch-13.md) | 14 |
| 14 | [batch-14.md](batch-14.md) | 14 |
| 15 | [batch-15.md](batch-15.md) | 13 |

## 6. 全局汇总排序表

按优先级（P0→P1→P2→搁置）+ 证据强度（强→中→弱→无）+ 字母序排列。批次列指向对应调研文件。

<!-- TABLE_START -->
| # | ID | 证据 | 实现路径 | 优先级 | 批次 |
|---|---|---|---|---|---|
| 1 | abacus | 强 | 薄封装 | P0 | [03](batch-03.md) |
| 2 | abliteration_ai | 强 | 薄封装 | P0 | [03](batch-03.md) |
| 3 | aiand | 强 | 薄封装 | P0 | [03](batch-03.md) |
| 4 | ambient | 强 | 薄封装 | P0 | [04](batch-04.md) |
| 5 | umans_ai | 强 | 薄封装 | P0 | [12](batch-12.md) |
| 6 | venice | 强 | 薄封装 | P0 | [12](batch-12.md) |
| 7 | aki_io | 强 | 薄封装 | P1 | [03](batch-03.md) |
| 8 | alibaba_coding_plan | 强 | 薄封装 | P1 | [04](batch-04.md) |
| 9 | alibaba_coding_plan_cn | 强 | 薄封装 | P1 | [04](batch-04.md) |
| 10 | alibaba_token_plan | 强 | 薄封装 | P1 | [14](batch-14.md) |
| 11 | alibaba_token_plan_cn | 强 | 薄封装 | P1 | [14](batch-14.md) |
| 12 | anyapi | 强 | 薄封装 | P1 | [04](batch-04.md) |
| 13 | auriko | 强 | 薄封装 | P1 | [04](batch-04.md) |
| 14 | baidu_v2 | 强 | 薄封装 | P1 | [04](batch-04.md) |
| 15 | bailing | 强 | 薄封装 | P1 | [04](batch-04.md) |
| 16 | bedrock_mantle | 强 | 薄封装 | P1 | [14](batch-14.md) |
| 17 | berget | 强 | 薄封装 | P1 | [04](batch-04.md) |
| 18 | cherryin | 强 | 薄封装 | P1 | [14](batch-14.md) |
| 19 | chutes | 强 | 薄封装 | P1 | [01](batch-01.md) |
| 20 | claudinio | 强 | 薄封装 | P1 | [04](batch-04.md) |
| 21 | cloudferro_sherlock | 强 | 薄封装 | P1 | [05](batch-05.md) |
| 22 | cloudflare_workers_ai | 强 | 薄封装 | P1 | [05](batch-05.md) |
| 23 | cohere_chat | 强 | 原生 | P1 | [05](batch-05.md) |
| 24 | cortecs | 强 | 薄封装 | P1 | [05](batch-05.md) |
| 25 | crof | 强 | 薄封装 | P1 | [05](batch-05.md) |
| 26 | crossmodel | 强 | 薄封装 | P1 | [05](batch-05.md) |
| 27 | crusoe | 强 | 薄封装 | P1 | [05](batch-05.md) |
| 28 | daoxe | 强 | 薄封装 | P1 | [05](batch-05.md) |
| 29 | digitalocean | 强 | 薄封装 | P1 | [14](batch-14.md) |
| 30 | dinference | 强 | 薄封装 | P1 | [05](batch-05.md) |
| 31 | doubao | 强 | 薄封装 | P1 | [14](batch-14.md) |
| 32 | drun | 强 | 薄封装 | P1 | [05](batch-05.md) |
| 33 | empiriolabs | 强 | 薄封装 | P1 | [06](batch-06.md) |
| 34 | evroc | 强 | 薄封装 | P1 | [14](batch-14.md) |
| 35 | exa_ai | 强 | 模态专用 | P1 | [02](batch-02.md) |
| 36 | firecrawl | 强 | 模态专用 | P1 | [02](batch-02.md) |
| 37 | frogbot | 强 | 薄封装 | P1 | [06](batch-06.md) |
| 38 | gmicloud | 强 | 薄封装 | P1 | [06](batch-06.md) |
| 39 | google_vertex_anthropic | 强 | 原生 | P1 | [06](batch-06.md) |
| 40 | hpc_ai | 强 | 薄封装 | P1 | [06](batch-06.md) |
| 41 | inceptron | 强 | 薄封装 | P1 | [06](batch-06.md) |
| 42 | inferx | 强 | 薄封装 | P1 | [07](batch-07.md) |
| 43 | io_net | 强 | 薄封装 | P1 | [07](batch-07.md) |
| 44 | jiekou | 强 | 薄封装 | P1 | [07](batch-07.md) |
| 45 | jina_ai | 强 | 模态专用 | P1 | [02](batch-02.md) |
| 46 | kenari | 强 | 薄封装 | P1 | [07](batch-07.md) |
| 47 | kimi | 强 | 薄封装 | P1 | [07](batch-07.md) |
| 48 | kimi_for_coding | 强 | 薄封装 | P1 | [07](batch-07.md) |
| 49 | lilac | 强 | 薄封装 | P1 | [07](batch-07.md) |
| 50 | llama | 强 | 薄封装 | P1 | [07](batch-07.md) |
| 51 | llmgateway | 强 | 薄封装 | P1 | [07](batch-07.md) |
| 52 | llmtr | 强 | 薄封装 | P1 | [08](batch-08.md) |
| 53 | lucidquery | 强 | 薄封装 | P1 | [08](batch-08.md) |
| 54 | meganova | 强 | 薄封装 | P1 | [08](batch-08.md) |
| 55 | merge_gateway | 强 | 薄封装 | P1 | [08](batch-08.md) |
| 56 | meta | 强 | 薄封装 | P1 | [01](batch-01.md) |
| 57 | mimo | 强 | 薄封装 | P1 | [08](batch-08.md) |
| 58 | minimax_cn | 强 | 薄封装 | P1 | [08](batch-08.md) |
| 59 | mixlayer | 强 | 薄封装 | P1 | [08](batch-08.md) |
| 60 | moark | 强 | 薄封装 | P1 | [08](batch-08.md) |
| 61 | model_oracle_ai | 强 | 薄封装 | P1 | [09](batch-09.md) |
| 62 | nearai | 强 | 薄封装 | P1 | [15](batch-15.md) |
| 63 | neon | 强 | 薄封装 | P1 | [09](batch-09.md) |
| 64 | neuralwatt | 强 | 薄封装 | P1 | [09](batch-09.md) |
| 65 | oci | 强 | 薄封装 | P1 | [15](batch-15.md) |
| 66 | ofox | 强 | 薄封装 | P1 | [09](batch-09.md) |
| 67 | perplexity_agent | 强 | 薄封装 | P1 | [09](batch-09.md) |
| 68 | poe | 强 | 薄封装 | P1 | [01](batch-01.md) |
| 69 | poolside | 强 | 薄封装 | P1 | [10](batch-10.md) |
| 70 | qihang_ai | 强 | 薄封装 | P1 | [10](batch-10.md) |
| 71 | recraft | 强 | 共享层扩展 | P1 | [02](batch-02.md) |
| 72 | regolo_ai | 强 | 薄封装 | P1 | [15](batch-15.md) |
| 73 | runwayml | 强 | 模态专用 | P1 | [02](batch-02.md) |
| 74 | serper | 强 | 模态专用 | P1 | [02](batch-02.md) |
| 75 | snowflake_cortex | 强 | 薄封装 | P1 | [10](batch-10.md) |
| 76 | stability | 强 | 模态专用 | P1 | [03](batch-03.md) |
| 77 | stackit | 强 | 薄封装 | P1 | [10](batch-10.md) |
| 78 | stepfun_ai_step_plan | 强 | 薄封装 | P1 | [10](batch-10.md) |
| 79 | stepfun_step_plan | 强 | 薄封装 | P1 | [10](batch-10.md) |
| 80 | subconscious | 强 | 薄封装 | P1 | [10](batch-10.md) |
| 81 | tencent_tokenhub | 强 | 薄封装 | P1 | [11](batch-11.md) |
| 82 | the_grid_ai | 强 | 薄封装 | P1 | [11](batch-11.md) |
| 83 | tokenflux | 强 | 薄封装 | P1 | [11](batch-11.md) |
| 84 | trustedrouter | 强 | 薄封装 | P1 | [11](batch-11.md) |
| 85 | vertex_ai_anthropic_models | 强 | 共享层扩展/原生 | P1 | [12](batch-12.md) |
| 86 | vertex_ai_language_models | 强 | 原生 | P1 | [15](batch-15.md) |
| 87 | vivgrid | 强 | 薄封装 | P1 | [13](batch-13.md) |
| 88 | volc_engine | 强 | 薄封装 | P1 | [13](batch-13.md) |
| 89 | vultr | 强 | 薄封装 | P1 | [13](batch-13.md) |
| 90 | wandb | 强 | 薄封装 | P1 | [01](batch-01.md) |
| 91 | xunfei | 强 | 薄封装 | P1 | [13](batch-13.md) |
| 92 | zai_coding_plan | 强 | 薄封装 | P1 | [13](batch-13.md) |
| 93 | zenmux | 强 | 薄封装 | P1 | [15](batch-15.md) |
| 94 | zhipu_v4 | 强 | 薄封装 | P1 | [13](batch-13.md) |
| 95 | zhipuai_coding_plan | 强 | 薄封装 | P1 | [14](batch-14.md) |
| 96 | ai_router | 中 | 薄封装 | P1 | [03](batch-03.md) |
| 97 | ebcloud | 中 | 薄封装 | P1 | [06](batch-06.md) |
| 98 | llamagate | 中 | 薄封装 | P1 | [14](batch-14.md) |
| 99 | ppinfra | 中 | 薄封装 | P1 | [10](batch-10.md) |
| 100 | routing_run | 中 | 薄封装 | P1 | [10](batch-10.md) |
| 101 | unorouter | 中 | 薄封装 | P1 | [12](batch-12.md) |
| 102 | atomic_chat | 强 | 薄封装 | P2 | [04](batch-04.md) |
| 103 | aws_polly | 强 | 模态专用 | P2 | [01](batch-01.md) |
| 104 | blueclaw | 强 | 薄封装 | P2 | [04](batch-04.md) |
| 105 | cloudflare | 强 | 薄封装 | P2 | [05](batch-05.md) |
| 106 | darkbloom | 强 | 薄封装 | P2 | [01](batch-01.md) |
| 107 | firepass | 强 | 薄封装 | P2 | [06](batch-06.md) |
| 108 | google_pse | 强 | 模态专用 | P2 | [02](batch-02.md) |
| 109 | jina | 强 | 模态专用 | P2 | [07](batch-07.md) |
| 110 | lemonade | 强 | 薄封装 | P2 | [07](batch-07.md) |
| 111 | libertai | 强 | 薄封装 | P2 | [01](batch-01.md) |
| 112 | linkup | 强 | 模态专用 | P2 | [02](batch-02.md) |
| 113 | lynkr | 强 | 薄封装 | P2 | [08](batch-08.md) |
| 114 | minimax_cn_coding_plan | 强 | 薄封装 | P2 | [08](batch-08.md) |
| 115 | opencode | 强 | 薄封装 | P2 | [09](batch-09.md) |
| 116 | parallel_ai | 强 | 模态专用 | P2 | [02](batch-02.md) |
| 117 | pinstripes | 强 | 薄封装 | P2 | [01](batch-01.md) |
| 118 | privatemode_ai | 强 | 薄封装 | P2 | [15](batch-15.md) |
| 119 | reducto | 强 | 模态专用 | P2 | [02](batch-02.md) |
| 120 | searxng | 强 | 模态专用 | P2 | [02](batch-02.md) |
| 121 | snowflake | 强 | 薄封装 | P2 | [15](batch-15.md) |
| 122 | soniox | 强 | 模态专用 | P2 | [02](batch-02.md) |
| 123 | sora | 强 | 模态专用 | P2 | [10](batch-10.md) |
| 124 | synthetic | 强 | 薄封装 | P2 | [01](batch-01.md) |
| 125 | tencent_coding_plan | 强 | 薄封装 | P2 | [11](batch-11.md) |
| 126 | tencent_token_plan | 强 | 薄封装 | P2 | [11](batch-11.md) |
| 127 | tencent_token_plan_enterprise_auto | 强 | 薄封装 | P2 | [11](batch-11.md) |
| 128 | tencent_token_plan_enterprise_pro | 强 | 薄封装 | P2 | [11](batch-11.md) |
| 129 | tencent_token_plan_general_personal | 强 | 薄封装 | P2 | [11](batch-11.md) |
| 130 | tencent_token_plan_hy_personal | 强 | 薄封装 | P2 | [11](batch-11.md) |
| 131 | tensormesh | 强 | 薄封装 | P2 | [01](batch-01.md) |
| 132 | text_completion_codestral | 强 | 模态专用 | P2 | [15](batch-15.md) |
| 133 | thinkingmachines | 强 | 薄封装 | P2 | [11](batch-11.md) |
| 134 | tinfoil | 强 | 薄封装 | P2 | [11](batch-11.md) |
| 135 | vertex_ai_ai21_models | 强 | 共享层扩展/原生 | P2 | [12](batch-12.md) |
| 136 | vertex_ai_deepseek_models | 强 | 共享层扩展/原生 | P2 | [12](batch-12.md) |
| 137 | vertex_ai_llama_models | 强 | 共享层扩展/原生 | P2 | [12](batch-12.md) |
| 138 | vertex_ai_minimax_models | 强 | 共享层扩展/原生 | P2 | [12](batch-12.md) |
| 139 | vertex_ai_mistral_models | 强 | 共享层扩展/原生 | P2 | [12](batch-12.md) |
| 140 | vertex_ai_moonshot_models | 强 | 共享层扩展/原生 | P2 | [12](batch-12.md) |
| 141 | vertex_ai_openai_models | 强 | 共享层扩展/原生 | P2 | [12](batch-12.md) |
| 142 | vertex_ai_qwen_models | 强 | 共享层扩展/原生 | P2 | [12](batch-12.md) |
| 143 | vertex_ai_zai_models | 强 | 共享层扩展/原生 | P2 | [12](batch-12.md) |
| 144 | watsonx | 强 | 原生 | P2 | [15](batch-15.md) |
| 145 | watsonx_text | 强 | 原生 | P2 | [13](batch-13.md) |
| 146 | xpersona | 强 | 薄封装 | P2 | [13](batch-13.md) |
| 147 | zeldoc | 强 | 薄封装 | P2 | [13](batch-13.md) |
| 148 | apertis | 中 | 薄封装 | P2 | [01](batch-01.md) |
| 149 | aws | 中 | 原生 | P2 | [04](batch-04.md) |
| 150 | azure_text | 中 | 原生 | P2 | [14](batch-14.md) |
| 151 | dataforseo | 中 | 模态专用 | P2 | [02](batch-02.md) |
| 152 | freemodel | 中 | 薄封装 | P2 | [06](batch-06.md) |
| 153 | gmi | 中 | 薄封装 | P2 | [06](batch-06.md) |
| 154 | google_vertex | 中 | 原生 | P2 | [14](batch-14.md) |
| 155 | hetzner | 中 | 薄封装 | P2 | [06](batch-06.md) |
| 156 | iflowcn | 中 | 薄封装 | P2 | [06](batch-06.md) |
| 157 | kuae_cloud_coding_plan | 中 | 薄封装 | P2 | [07](batch-07.md) |
| 158 | maritalk | 中 | 待定 | P2 | [08](batch-08.md) |
| 159 | minimax_coding_plan | 中 | 薄封装 | P2 | [08](batch-08.md) |
| 160 | moonshotai_cn | 中 | 薄封装 | P2 | [09](batch-09.md) |
| 161 | netlify | 中 | 原生 | P2 | [09](batch-09.md) |
| 162 | publicai | 中 | 薄封装 | P2 | [01](batch-01.md) |
| 163 | sagemaker | 中 | 原生 | P2 | [15](batch-15.md) |
| 164 | sagemaker_chat | 中 | 原生 | P2 | [10](batch-10.md) |
| 165 | sagemaker_nova | 中 | 原生 | P2 | [10](batch-10.md) |
| 166 | sap_ai_core | 中 | 原生 | P2 | [10](batch-10.md) |
| 167 | text_completion_inception | 中 | 模态专用 | P2 | [15](batch-15.md) |
| 168 | vidu | 中 | 模态专用 | P2 | [13](batch-13.md) |
| 169 | xiaomi_token_plan_ams | 中 | 薄封装 | P2 | [13](batch-13.md) |
| 170 | xiaomi_token_plan_cn | 中 | 薄封装 | P2 | [13](batch-13.md) |
| 171 | xiaomi_token_plan_sgp | 中 | 薄封装 | P2 | [13](batch-13.md) |
| 172 | alibaba_cn | 强 | 薄封装 | 搁置 | [03](batch-03.md) |
| 173 | cloudflare_ai_gateway | 强 | 待定 | 搁置 | [14](batch-14.md) |
| 174 | dify | 强 | 原生 | 搁置 | [05](batch-05.md) |
| 175 | doubao_video | 强 | 模态专用 | 搁置 | [05](batch-05.md) |
| 176 | gitlab | 强 | 待定 | 搁置 | [06](batch-06.md) |
| 177 | ollama_chat | 强 | 薄封装 | 搁置 | [09](batch-09.md) |
| 178 | oobabooga | 强 | 薄封装 | 搁置 | [09](batch-09.md) |
| 179 | pa_lm | 强 | 原生 | 搁置 | [09](batch-09.md) |
| 180 | tavily | 强 | 模态专用 | 搁置 | [03](batch-03.md) |
| 181 | tinyfish | 强 | 模态专用 | 搁置 | [03](batch-03.md) |
| 182 | umans_ai_coding_plan | 强 | 薄封装 | 搁置 | [12](batch-12.md) |
| 183 | you_com | 强 | 模态专用 | 搁置 | [03](batch-03.md) |
| 184 | ali | 中 | 原生 | 搁置 | [03](batch-03.md) |
| 185 | anthropic_text | 中 | 原生 | 搁置 | [04](batch-04.md) |
| 186 | hyper | 中 | 薄封装 | 搁置 | [06](batch-06.md) |
| 187 | jimeng | 中 | 模态专用 | 搁置 | [07](batch-07.md) |
| 188 | kling | 中 | 模态专用 | 搁置 | [07](batch-07.md) |
| 189 | new_api | 中 | 薄封装 | 搁置 | [09](batch-09.md) |
| 190 | palm | 中 | 原生 | 搁置 | [15](batch-15.md) |
| 191 | ragflow | 中 | 原生 | 搁置 | [01](batch-01.md) |
| 192 | suno_api | 中 | 模态专用 | 搁置 | [11](batch-11.md) |
| 193 | triton | 中 | 待定 | 搁置 | [11](batch-11.md) |
| 194 | vertex_ai_text_models | 中 | 原生 | 搁置 | [15](batch-15.md) |
| 195 | azure_cognitive_services | 弱 | 待定 | 搁置 | [14](batch-14.md) |
| 196 | burncloud | 弱 | 待定 | 搁置 | [14](batch-14.md) |
| 197 | chat_gpt_subscription_codex | 弱 | 待定 | 搁置 | [04](batch-04.md) |
| 198 | duckduckgo | 弱 | 模态专用 | 搁置 | [02](batch-02.md) |
| 199 | empower | 弱 | 待定 | 搁置 | [01](batch-01.md) |
| 200 | zenifra | 弱 | 待定 | 搁置 | [13](batch-13.md) |
| 201 | advanced_custom | 无 | 待定 | 搁置 | [03](batch-03.md) |
| 202 | aiproxy | 无 | 待定 | 搁置 | [03](batch-03.md) |
| 203 | aiproxy_library | 无 | 待定 | 搁置 | [03](batch-03.md) |
| 204 | custom_provider | 无 | 待定 | 搁置 | [05](batch-05.md) |
| 205 | midjourney | 无 | 待定 | 搁置 | [08](batch-08.md) |
| 206 | midjourney_plus | 无 | 待定 | 搁置 | [08](batch-08.md) |
| 207 | moka_ai | 无 | 待定 | 搁置 | [09](batch-09.md) |
| 208 | nova | 无 | 待定 | 搁置 | [09](batch-09.md) |
| 209 | sub2_api | 无 | 待定 | 搁置 | [10](batch-10.md) |
<!-- TABLE_END -->

## 7. 建议开工顺序

### 第一批（P0 + 高价值 P1 薄封装，立即可做）

6 个 P0 + ~90 个 P1 薄封装，共约 96 个 OpenAI 兼容薄封装。这批只需配置 base URL/名称/凭据/profile，复用 OpenAI 共享层，可高密度并行实现。建议按地域/厂商分组批量推进。

### 第二批（P1 原生 + 共享层扩展）

cohere_chat、google_vertex_anthropic、vertex_ai_anthropic_models、recraft 等。需原生实现或共享层扩展，每个独立处理。

### 第三批（P2 薄封装 + 模态专用）

P2 薄封装可随时插入第一批；模态专用（stability/runwayml/aws_polly/sora 等）依赖 image/video/audio 模态基建就绪。

### 阻塞项（需先做 core 契约变更）

1. **search trait**：解锁 11 个 search provider
2. **AWS SigV4 签名共享能力**：解锁 aws/sagemaker/aws_polly/bedrock_mantle
3. **Vertex AI partner 扩展**：解锁 10 个 vertex_ai_*_models

### 搁置项（38 个，暂不投入）

证据不足 15 个、已有别名覆盖 6 个、已弃用 4 个、非真实 provider 6 个、定位不符 4 个、自托管无固定端点 3 个。

## 8. 中间产物

- `_research-input.json`：209 个 provider 的调研基础数据
- `_summary.json`：各 provider 证据强度/实现路径/优先级结构化汇总
- `_global_table.md`：全局排序表（本 README 第 6 节的数据源）
- `batches/batch-XX.json`：15 批输入数据
- `_template.md`：调研记录模板
