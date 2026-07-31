# RFC-0012 §3.5 — Responses API 相似度审计报告

工作目录: `/media/eric8810/fast-deliver/code/aimux`

## 1. 文件规模

| 文件 | 原始行数 | 归一化后代码行（去注释/空白） |
|---|---:|---:|
| `aimux-providers/src/open_responses.rs` | 1290 | 1128 |
| `aimux-providers/src/huggingface/responses.rs` | 1196 | 651 |
| `aimux-providers/src/azure/responses.rs` | 1106 | 938 |
| `aimux-providers/src/openai/responses/mod.rs` | 969 | 846 |
| `aimux-providers/src/openai/responses/convert.rs` | 1088 | 914 |
| `aimux-providers/src/xai/responses/mod.rs` | 954 | 817 |
| `aimux-providers/src/xai/responses/convert.rs` | 819 | 729 |
| **合计** | **7422** | **6023** |

## 2. 两两相似度（归一化后，顺序敏感 SequenceMatcher）

| A | B | A行 | B行 | 匹配行数 | 相似度 |
|---|---|---:|---:|---:|---:|
| `aimux-providers/src/open_responses.rs` | `aimux-providers/src/huggingface/responses.rs` | 1128 | 651 | 258 | 29.0% |
| `aimux-providers/src/open_responses.rs` | `aimux-providers/src/azure/responses.rs` | 1128 | 938 | 257 | 24.9% |
| `aimux-providers/src/open_responses.rs` | `aimux-providers/src/openai/responses/mod.rs` | 1128 | 846 | 257 | 26.0% |
| `aimux-providers/src/open_responses.rs` | `aimux-providers/src/openai/responses/convert.rs` | 1128 | 914 | 104 | 10.2% |
| `aimux-providers/src/open_responses.rs` | `aimux-providers/src/xai/responses/mod.rs` | 1128 | 817 | 152 | 15.6% |
| `aimux-providers/src/open_responses.rs` | `aimux-providers/src/xai/responses/convert.rs` | 1128 | 729 | 90 | 9.7% |
| `aimux-providers/src/huggingface/responses.rs` | `aimux-providers/src/azure/responses.rs` | 651 | 938 | 241 | 30.3% |
| `aimux-providers/src/huggingface/responses.rs` | `aimux-providers/src/openai/responses/mod.rs` | 651 | 846 | 257 | 34.3% |
| `aimux-providers/src/huggingface/responses.rs` | `aimux-providers/src/openai/responses/convert.rs` | 651 | 914 | 69 | 8.8% |
| `aimux-providers/src/huggingface/responses.rs` | `aimux-providers/src/xai/responses/mod.rs` | 651 | 817 | 157 | 21.4% |
| `aimux-providers/src/huggingface/responses.rs` | `aimux-providers/src/xai/responses/convert.rs` | 651 | 729 | 93 | 13.5% |
| `aimux-providers/src/azure/responses.rs` | `aimux-providers/src/openai/responses/mod.rs` | 938 | 846 | 794 | 89.0% |
| `aimux-providers/src/azure/responses.rs` | `aimux-providers/src/openai/responses/convert.rs` | 938 | 914 | 81 | 8.7% |
| `aimux-providers/src/azure/responses.rs` | `aimux-providers/src/xai/responses/mod.rs` | 938 | 817 | 193 | 22.0% |
| `aimux-providers/src/azure/responses.rs` | `aimux-providers/src/xai/responses/convert.rs` | 938 | 729 | 45 | 5.4% |
| `aimux-providers/src/openai/responses/mod.rs` | `aimux-providers/src/openai/responses/convert.rs` | 846 | 914 | 81 | 9.2% |
| `aimux-providers/src/openai/responses/mod.rs` | `aimux-providers/src/xai/responses/mod.rs` | 846 | 817 | 214 | 25.7% |
| `aimux-providers/src/openai/responses/mod.rs` | `aimux-providers/src/xai/responses/convert.rs` | 846 | 729 | 40 | 5.1% |
| `aimux-providers/src/openai/responses/convert.rs` | `aimux-providers/src/xai/responses/mod.rs` | 914 | 817 | 55 | 6.4% |
| `aimux-providers/src/openai/responses/convert.rs` | `aimux-providers/src/xai/responses/convert.rs` | 914 | 729 | 149 | 18.1% |
| `aimux-providers/src/xai/responses/mod.rs` | `aimux-providers/src/xai/responses/convert.rs` | 817 | 729 | 42 | 5.4% |

## 3. 两两相似度（归一化后，集合 Jaccard，忽略顺序）

| A | B | Jaccard |
|---|---|---:|
| `aimux-providers/src/open_responses.rs` | `aimux-providers/src/huggingface/responses.rs` | 19.6% |
| `aimux-providers/src/open_responses.rs` | `aimux-providers/src/azure/responses.rs` | 13.4% |
| `aimux-providers/src/open_responses.rs` | `aimux-providers/src/openai/responses/mod.rs` | 14.7% |
| `aimux-providers/src/open_responses.rs` | `aimux-providers/src/openai/responses/convert.rs` | 10.2% |
| `aimux-providers/src/open_responses.rs` | `aimux-providers/src/xai/responses/mod.rs` | 11.5% |
| `aimux-providers/src/open_responses.rs` | `aimux-providers/src/xai/responses/convert.rs` | 10.1% |
| `aimux-providers/src/huggingface/responses.rs` | `aimux-providers/src/azure/responses.rs` | 18.8% |
| `aimux-providers/src/huggingface/responses.rs` | `aimux-providers/src/openai/responses/mod.rs` | 23.2% |
| `aimux-providers/src/huggingface/responses.rs` | `aimux-providers/src/openai/responses/convert.rs` | 8.5% |
| `aimux-providers/src/huggingface/responses.rs` | `aimux-providers/src/xai/responses/mod.rs` | 16.9% |
| `aimux-providers/src/huggingface/responses.rs` | `aimux-providers/src/xai/responses/convert.rs` | 10.4% |
| `aimux-providers/src/azure/responses.rs` | `aimux-providers/src/openai/responses/mod.rs` | 71.2% |
| `aimux-providers/src/azure/responses.rs` | `aimux-providers/src/openai/responses/convert.rs` | 3.9% |
| `aimux-providers/src/azure/responses.rs` | `aimux-providers/src/xai/responses/mod.rs` | 15.4% |
| `aimux-providers/src/azure/responses.rs` | `aimux-providers/src/xai/responses/convert.rs` | 3.4% |
| `aimux-providers/src/openai/responses/mod.rs` | `aimux-providers/src/openai/responses/convert.rs` | 3.7% |
| `aimux-providers/src/openai/responses/mod.rs` | `aimux-providers/src/xai/responses/mod.rs` | 19.7% |
| `aimux-providers/src/openai/responses/mod.rs` | `aimux-providers/src/xai/responses/convert.rs` | 3.5% |
| `aimux-providers/src/openai/responses/convert.rs` | `aimux-providers/src/xai/responses/mod.rs` | 3.3% |
| `aimux-providers/src/openai/responses/convert.rs` | `aimux-providers/src/xai/responses/convert.rs` | 18.2% |
| `aimux-providers/src/xai/responses/mod.rs` | `aimux-providers/src/xai/responses/convert.rs` | 3.4% |

## 4. 各文件与其余文件的平均共享度

| 文件 | 平均相似度(对较小文件) | 平均 Jaccard |
|---|---:|---:|
| `aimux-providers/src/open_responses.rs` | 23.3% | 13.2% |
| `aimux-providers/src/huggingface/responses.rs` | 27.7% | 16.2% |
| `aimux-providers/src/azure/responses.rs` | 32.6% | 21.0% |
| `aimux-providers/src/openai/responses/mod.rs` | 33.9% | 22.7% |
| `aimux-providers/src/openai/responses/convert.rs` | 10.3% | 8.0% |
| `aimux-providers/src/xai/responses/mod.rs` | 18.3% | 11.7% |
| `aimux-providers/src/xai/responses/convert.rs` | 11.4% | 8.2% |

## 5. 结论：可合并部分 vs 真实差异

基于上面的相似度矩阵，识别出以下结构性观察：

- **mod.rs（openai/xai）与 azure/responses.rs、open_responses.rs、huggingface/responses.rs** 都实现了 `LanguageModel` trait 的 `do_generate` / `do_stream`，其流式事件解析主循环（`response.created -> output_item.added -> output_text.delta -> output_text.done -> output_item.done -> response.completed`）结构高度同构，差异主要在：endpoint 拼接、header 构造、个别事件名/字段名。
- **convert.rs（openai/xai）与各 responses 文件中的 build_request_body / convert_to_*_input** 承担请求体构建与 input 转换，结构同构但厂商字段（reasoning、metadata、provider options）有真实差异。
- **usage 提取**（`extract_usage` / `convert_usage` / `convert_responses_usage`）和 **finish_reason 映射**（`map_*_finish_reason`）是最易合并的小函数。
- **媒体类型解析**（base64 data URL 拆分、top-level media type）在 huggingface/azure 重复实现，可共享。
- 真实差异（必须保留为厂商覆盖）：endpoint/base_url、model id 校验与映射、provider-specific provider options（openai 的 item_id/phase/namespace、xai 的 source/tool 解析、azure 的 deployment 前缀注入、huggingface 的消息格式）。
- **不强行合并到单一函数**：各厂商 responses 实现有真实协议差异，只提取共享框架，差异以厂商覆盖形式保留。

## 6. 合并策略（对应 RFC §3.5 步骤 1-2）

1. 在 `openai/responses/responses_convert.rs` 提取共享框架：请求体构建通用片段、流式事件解析通用片段、usage 提取通用片段、媒体类型 data-URL 拆分 helper。
2. 各厂商只保留差异覆盖：endpoint 拼接、model id 映射、provider-specific 字段，调用共享框架函数。

## 7. 实施结果（实施后）

共享框架已落地为 [`openai/responses/responses_convert.rs`](../../aimux-providers/src/openai/responses/responses_convert.rs)，包含：
- `build_header_list`（OpenAI / HuggingFace / xAI 三处字节相同副本归一）
- `build_responses_generate_result`（非流式输出解析：OpenAI 与 Azure 字节相同）
- `build_responses_event_stream`（流式 SSE 事件归约：OpenAI 与 Azure 字节相同）+ 共享状态结构
  `OngoingToolCall` / `ReasoningState` / `SummaryStatus`

各厂商只保留差异覆盖并调用共享框架：
- `openai/responses/mod.rs`、`azure/responses.rs`：`do_generate` / `do_stream` 缩为薄封装
  （保留各自 endpoint、auth、file-id prefix、provider key、retry config）。
- `huggingface/responses.rs`、`xai/responses/mod.rs`：复用共享 `build_header_list`。
- `open_responses.rs`：保留独立实现（协议差异真实，见下）。

| 文件 | 实施前 | 实施后 | Δ |
|---|---:|---:|---:|
| `open_responses.rs` | 1,290 | 1,290 | 0 |
| `huggingface/responses.rs` | 1,196 | 1,187 | -9 |
| `azure/responses.rs` | 1,106 | 329 | -777 |
| `openai/responses/mod.rs` | 969 | 191 | -778 |
| `openai/responses/convert.rs` | 1,088 | 1,088 | 0 |
| `openai/responses/responses_convert.rs` | — | 876 | +876（新增） |
| `xai/responses/mod.rs` | 954 | 945 | -9 |
| `xai/responses/convert.rs` | 819 | 819 | 0 |
| **合计** | **7,422** | **6,725** | **-697** |

**对 RFC 估算的修正（审计确认）**：RFC §3.5 估算 ~7,400→~4,000 行 / 7→4 文件。逐行审计
**修正了该估算**，原因如下：

1. **只有 azure↔openai-mod 一对达到 89% 相似**（71% Jaccard），这是唯一可安全合并的大块；
   已提取，单对净减 -697 行（毛减 -1,555，新增共享框架 +876）。
2. **其余文件两两相似度仅 5–34%**，且流式事件集 genuinely 不同（见 §5）：
   - `open_responses` / `huggingface` 用更简的事件集与不同的输出解析（error 用
     `ApiCall`、reasoning 走 `content[].text`、`provider_metadata: None`、timestamp 格式不同）；
   - `xai` 事件集最大且最发散（`response.done` / `output_text.annotation.added` / `custom_tool_call_input.done` 等）。
   按约束 4（"不强行合并"）与约束 5（不引入新测试失败，覆盖 180 个厂商专属测试），不合并。
3. **文件数无法降到 4**：`OpenResponsesModel` / `HuggingFaceResponsesModel` /
   `AzureResponsesModel` / `XaiResponsesModel` 均为对外公共类型（约束：公共 API 不变），不可删除；
   共享框架 `responses_convert.rs` 是 RFC §3.5 步骤 1 明确要求新增的文件。

因此实际净减 **-697 行**（7→8 文件），显著低于 RFC 估算的 -3,400 行 / -3 文件。
这正符合 RFC 自身的约束："实施前需先做逐行相似度审计确认"——审计确认估算过于乐观。

**验证**：
- `cargo check -p aimux-providers --lib`：通过。
- `cargo clippy -p aimux-providers --lib`：0 warning（改动文件无新增 lint）。
- `cargo fmt`：改动文件 0 diff（其余 139 个文件的 fmt diff 为预先存在，不在本次范围）。
- 5 个 Responses 测试 target 共 232 项全部通过（azure 24 / huggingface 31 /
  open_responses 67 / openai 28 / xai 82），无新增失败。
