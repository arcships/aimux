# 测试可复用性报告（doc 12）审计与统计修正

审计范围：`reference\ai\packages\` 全部测试文件。
统计方法：对每个 `.test.ts` / `.test-d.ts` 文件内容做大小写敏感正则匹配 `\bit\(` 与 `\btest\(`（词边界，匹配直接调用，排除 `it.skip` / `it.only` / `it.each` / `xit` / `fit`）。`it.each` 表驱动用例不计入（与原报告同口径限制）。
统计工具：PowerShell `Get-Content -Raw` + `[regex]::Matches`。

---

## 结论速览（TL;DR）

原报告**严重不完整且数字多处不准**：

| 维度 | 原报告声称 | 实测 | 偏差 |
|---|---|---|---|
| OpenAI | 89 主 + 45 纯函数 = 134 it / 3 文件 | **611 it / 26 文件** | 漏 23 文件、477 it |
| Anthropic | 277 主 + 148 纯函数 = 425 it / 4 文件 | **472 it + 2 test / 14 文件** | 漏 10 文件、47 it |
| provider-utils | retry 19 + handle 7 + get 16 + extract 8 = 50 / 4 文件 | **802 it + 6 test / 88 文件** | 漏 84 文件；retry 项不存在；handle/get 各少 1 |
| packages/ai/src | 只列文件名无数 | **2926 it + 47 test / 143 文件** | 用例数此前完全缺失 |
| 总计 P0+P1 | 约 346 例 | 仅审计的 4 核心包就 **4811 it + 55 test** | 346 与其自身分项之和(609)都不一致 |

原报告分项之和 = 134 + 425 + 50 = **609**，与其自称的"总计约 346"内部矛盾。346 这一总数既不等于分项之和，也不等于任何合理聚合（仅 OpenAI+Anthropic 两个主测试文件就 89+277=366）。

---

## 1. OpenAI 测试完整清单（文件 + it 数 + 遗漏项）

路径：`packages\openai\src\`，共 **26 个测试文件，611 it，0 test**。

| 文件 | it | test | 原报告是否覆盖 |
|---|---:|---:|---|
| chat\convert-to-openai-chat-messages.test.ts | 33 | 0 | ✅ 纯函数(convert-to-messages 33) |
| chat\openai-chat-language-model.test.ts | 89 | 0 | ✅ 主测试 89 |
| chat\openai-chat-prepare-tools.test.ts | 12 | 0 | ✅ 纯函数(prepare-tools 12) |
| completion\openai-completion-language-model.test.ts | 18 | 0 | ❌ 遗漏 |
| embedding\openai-embedding-model.test.ts | 7 | 0 | ❌ 遗漏 |
| files\openai-files.test.ts | 8 | 0 | ❌ 遗漏 |
| image\openai-image-model.test.ts | 28 | 0 | ❌ 遗漏 |
| openai-error.test.ts | 1 | 0 | ❌ 遗漏 |
| openai-forward-compatible-defaults.test.ts | 3 | 0 | ❌ 遗漏 |
| openai-language-model-capabilities.test.ts | 0 | 0 | ❌ 遗漏 |
| openai-provider.test.ts | 7 | 0 | ❌ 遗漏 |
| realtime\openai-realtime-event-mapper.test.ts | 2 | 0 | ❌ 遗漏 |
| realtime\openai-realtime-model.test.ts | 2 | 0 | ❌ 遗漏 |
| **responses\convert-to-openai-responses-input.test.ts** | **120** | 0 | ❌ 遗漏（大） |
| responses\convert-to-openai-responses-input-tool-search.test.ts | 1 | 0 | ❌ 遗漏 |
| responses\openai-responses-api.test.ts | 6 | 0 | ❌ 遗漏 |
| responses\openai-responses-computer.test.ts | 3 | 0 | ❌ 遗漏 |
| **responses\openai-responses-language-model.test.ts** | **175** | 0 | ❌ 遗漏（大） |
| **responses\openai-responses-prepare-tools.test.ts** | **52** | 0 | ❌ 遗漏（大） |
| skills\openai-skills.test.ts | 6 | 0 | ❌ 遗漏 |
| speech\openai-speech-model.test.ts | 8 | 0 | ❌ 遗漏 |
| transcription\openai-transcription-model.test.ts | 27 | 0 | ❌ 遗漏 |
| tool\computer.test-d.ts | 1 | 0 | ❌ 遗漏(type test) |
| tool\local-shell.test-d.ts | 1 | 0 | ❌ 遗漏(type test) |
| tool\programmatic-tool-calling.test-d.ts | 0 | 0 | ❌ 遗漏(type test) |
| tool\web-search.test-d.ts | 1 | 0 | ❌ 遗漏(type test) |
| **合计** | **611** | **0** | 仅覆盖 134 it / 3 文件 |

### OpenAI 遗漏要点
- **整个 `responses/` 子目录被忽略**：Responses API 是 OpenAI 当前主推接口，其测试共 175+120+52+6+3+1 = **357 it**，是原报告 OpenAI 总数(134)的 2.7 倍。
- **"主测试"概念不完整**：OpenAI 有**两个** language-model 主测试——chat(89) 与 responses(175)——原报告只算了 chat。
- **"纯函数"概念不完整**：Responses 路径同样有 convert-to-input(120) 与 prepare-tools(52)，原报告只算了 chat 路径的 33+12=45。实际纯函数类用例 = 33+12+120+1+52 = **218 it**。
- 漏掉 completion / embedding / image / speech / transcription / realtime / files / skills / error / provider / capabilities 共 11 类模型与工具测试。

---

## 2. Anthropic 测试完整清单（文件 + it 数 + 遗漏项）

路径：`packages\anthropic\src\`，共 **14 个测试文件，472 it，2 test**。

| 文件 | it | test | 原报告是否覆盖 |
|---|---:|---:|---|
| anthropic-error.test.ts | 1 | 0 | ❌ 遗漏 |
| anthropic-files.test.ts | 12 | 0 | ❌ 遗漏 |
| **anthropic-language-model.test.ts** | **277** | 0 | ✅ 主测试 277 |
| anthropic-prepare-tools.test.ts | 51 | 0 | ✅ 纯函数(prepare-tools 51) |
| anthropic-provider.test.ts | 12 | 2 | ❌ 遗漏 |
| anthropic-unknown-model-max-output-tokens.test.ts | 3 | 0 | ❌ 遗漏 |
| convert-anthropic-usage.test.ts | 15 | 0 | ✅ 纯函数(convert-usage 15) |
| convert-to-anthropic-prompt.test.ts | 82 | 0 | ✅ 纯函数(convert-to-prompt 82) |
| sanitize-json-schema.test.ts | 5 | 0 | ❌ 遗漏（纯函数类，漏算） |
| skills\anthropic-skills.test.ts | 6 | 0 | ❌ 遗漏 |
| tool\bash_20241022.test.ts | 1 | 0 | ❌ 遗漏 |
| tool\bash_20241022.test-d.ts | 3 | 0 | ❌ 遗漏(type test) |
| tool\bash_20250124.test.ts | 1 | 0 | ❌ 遗漏 |
| tool\bash_20250124.test-d.ts | 3 | 0 | ❌ 遗漏(type test) |
| **合计** | **472** | **2** | 仅覆盖 425 it / 4 文件 |

### Anthropic 遗漏要点
- 原报告覆盖的 4 个文件数字本身**准确**（277/82/51/15 全对）。
- 但漏掉 10 个文件共 **47 it + 2 test**：error、files、provider、unknown-model-max-output-tokens、sanitize-json-schema、skills、4 个 bash tool 测试。
- `sanitize-json-schema.test.ts`(5) 属纯函数类，原报告"纯函数"清单也漏了它（实际纯函数 = 148+5 = 153）。
- 两个 `bash_*.test-d.ts` 是 type 测试，各 3 it，原报告完全未提 type 测试体系。

---

## 3. provider-utils 测试完整清单（文件 + it 数 + 验证）

路径：`packages\provider-utils\src\`，共 **88 个测试文件，802 it，6 test**。

### 3.1 原报告 4 项声称的逐项验证

| 原报告项 | 原报告数 | 实测文件 | 实测 it | 实测 test | 验证结论 |
|---|---:|---|---:|---:|---|
| retry | 19 | **无此测试文件** | — | — | ❌ **不存在**。provider-utils 仅有源码 `retry-with-exponential-backoff.ts`，无对应测试。全仓唯一的 retry 测试在 `packages\ai\src\util\retry-with-exponential-backoff.test.ts`，仅 15 it。包归属错、数字错。 |
| handle-fetch-error | 7 | handle-fetch-error.test.ts | **8** | 0 | ⚠️ 少算 1（实测 8 条 `it(...)`，逐条核对均为真实用例） |
| get-from-api | 16 | get-from-api.test.ts | **17** | 0 | ⚠️ 少算 1（实测 17 条，逐条核对均为真实用例） |
| extract-lines | 8 | extract-lines.test.ts | 8 | 0 | ✅ 准确 |
| **小计** | **50** | — | **33（实）+19（虚）** | 0 | 4 项中 1 项不存在、2 项各少 1、1 项正确 |

### 3.2 provider-utils 全部 88 文件清单（按 it 降序，节选重点 + 全量）

> 下表为全量 88 文件。原报告仅触及其中 3 个（handle-fetch-error / get-from-api / extract-lines）。

| 文件 | it | test |
|---|---:|---:|
| detect-media-type.test.ts | 68 | 0 |
| validate-download-url.test.ts | 54 | 0 |
| types\tool.test-d.ts | 35 | 0 |
| is-url-supported.test.ts | 28 | 0 |
| transcription-stream-envelope.test.ts | 25 | 0 |
| schema.test.ts | 24 | 0 |
| to-json-schema\zod3-to-json-schema\refs.test.ts | 23 | 0 |
| download-blob.test.ts | 21 | 0 |
| inject-json-instruction.test.ts | 20 | 0 |
| resolve.test.ts | 18 | 0 |
| streaming-tool-call-tracker.test.ts | 18 | 0 |
| to-json-schema\zod3-to-json-schema\zod3-to-json-schema.test.ts | 18 | 0 |
| get-from-api.test.ts | 17 | 0 |
| fetch-with-validated-redirects.test.ts | 16 | 0 |
| parse-json.test.ts | 15 | 0 |
| map-reasoning-to-provider.test.ts | 14 | 0 |
| delay.test.ts | 13 | 0 |
| convert-to-form-data.test.ts | 12 | 0 |
| secure-json-parse.test.ts | 12 | 0 |
| types\content-part.test-d.ts | 25 | 0 |
| to-json-schema\zod3-to-json-schema\parsers\string.test.ts | 31 | 3 |
| add-additional-properties-to-json-schema.test.ts | 9 | 0 |
| connect-to-websocket.test.ts | 9 | 0 |
| delayed-promise.test.ts | 9 | 0 |
| is-provider-reference.test.ts | 9 | 0 |
| read-response-with-size-limit.test.ts | 9 | 0 |
| to-json-schema\zod3-to-json-schema\parse-def.test.ts | 9 | 0 |
| to-json-schema\zod3-to-json-schema\parsers\union.test.ts | 9 | 0 |
| types\infer-tool-set-context.test-d.ts | 7 | 0 |
| convert-image-model-file-to-data-uri.test.ts | 7 | 0 |
| response-handler.test.ts | 7 | 0 |
| serialize-model-options.test.ts | 7 | 0 |
| to-json-schema\zod3-to-json-schema\parsers\optional.test.ts | 7 | 0 |
| create-tool-name-mapping.test.ts | 6 | 0 |
| normalize-headers.test.ts | 6 | 0 |
| to-json-schema\zod3-to-json-schema\parsers\array.test.ts | 6 | 0 |
| to-json-schema\zod3-to-json-schema\parsers\native-enum.test.ts | 6 | 0 |
| to-json-schema\zod3-to-json-schema\parsers\number.test.ts | 6 | 0 |
| to-json-schema\zod3-to-json-schema\parsers\object.test.ts | 6 | 0 |
| to-json-schema\zod3-to-json-schema\parsers\record.test.ts | 6 | 0 |
| websocket.test.ts | 6 | 0 |
| has-required-key.test-d.ts | 4 | 0 |
| generate-id.test.ts | 4 | 0 |
| convert-async-iterator-to-readable-stream.test.ts | 4 | 0 |
| get-runtime-environment-user-agent.test.ts | 4 | 0 |
| is-same-origin.test.ts | 4 | 0 |
| resolve-full-media-type.test.ts | 8 | 0 |
| handle-fetch-error.test.ts | 8 | 0 |
| is-json-serializable.test.ts | 8 | 0 |
| extract-lines.test.ts | 8 | 0 |
| with-user-agent-suffix.test.ts | 5 | 0 |
| remove-undefined-entries.test.ts | 5 | 0 |
| resolve-provider-reference.test.ts | 5 | 0 |
| to-json-schema\zod3-to-json-schema\parsers\date.test.ts | 5 | 0 |
| types\infer-tool-context.test-d.ts | 5 | 0 |
| validate-types.test.ts | 4 | 0 |
| to-json-schema\zod3-to-json-schema\parsers\bigint.test.ts | 4 | 0 |
| as-array.test.ts | 3 | 0 |
| cancel-response-body.test.ts | 3 | 0 |
| to-json-schema\zod3-to-json-schema\parsers\default.test.ts | 3 | 0 |
| to-json-schema\zod3-to-json-schema\parsers\intersection.test.ts | 3 | 0 |
| to-json-schema\zod3-to-json-schema\parsers\pipe.test.ts | 3 | 0 |
| types\executable-tool.test.ts | 4 | 0 |
| types\executable-tool.test-d.ts | 3 | 0 |
| types\execute-tool.test.ts | 3 | 0 |
| types\never-optional.test-d.ts | 3 | 0 |
| types\tool-execute-function.test-d.ts | 3 | 0 |
| without-trailing-slash.test.ts | 3 | 0 |
| to-json-schema\zod3-to-json-schema\parsers\map.test.ts | 2 | 0 |
| to-json-schema\zod3-to-json-schema\parsers\nullable.test.ts | 2 | 0 |
| to-json-schema\zod3-to-json-schema\parsers\tuple.test.ts | 2 | 0 |
| is-browser-runtime.test.ts | 2 | 0 |
| filter-nullable.test.ts | 2 | 0 |
| types\execute-tool.test-d.ts | 2 | 0 |
| types\infer-tool-output.test-d.ts | 2 | 0 |
| validate-base-url.test.ts | 2 | 0 |
| to-json-schema\zod3-to-json-schema\parsers\branded.test.ts | 1 | 0 |
| to-json-schema\zod3-to-json-schema\parsers\catch.test.ts | 1 | 0 |
| to-json-schema\zod3-to-json-schema\parsers\promise.test.ts | 1 | 0 |
| to-json-schema\zod3-to-json-schema\parsers\readonly.test.ts | 1 | 0 |
| to-json-schema\zod3-to-json-schema\parsers\set.test.ts | 1 | 0 |
| types\infer-tool-input.test-d.ts | 1 | 0 |
| types\tool-needs-approval-function.test-d.ts | 1 | 0 |
| schema.test-d.ts | 1 | 0 |
| to-json-schema\zod3-to-json-schema\parsers\effects.test.ts | 2 | 1 |
| types\sandbox.test-d.ts | 0 | 2 |
| media-type-to-extension.test.ts | 0 | 0 |
| strip-file-extension.test.ts | 4 | 0 |
| **合计** | **802** | **6** |

### provider-utils 遗漏要点
- 原报告仅点了 4 个文件（其中 retry 还不存在），**漏掉 84 个文件、约 775 个用例**。
- provider-utils 实为"工具函数可复用测试富矿"：detect-media-type(68)、validate-download-url(54)、is-url-supported(28)、transcription-stream-envelope(25)、schema(24)、download-blob(21)、inject-json-instruction(20)、resolve(18)、streaming-tool-call-tracker(18)、fetch-with-validated-redirects(16)、parse-json(15)、map-reasoning-to-provider(14)、delay(13) 等大量纯函数测试，皆可直接复用。
- **整个 `to-json-schema/zod3-to-json-schema/` 子树**（1 主测试 + parse-def + refs + 18 个 parser 测试 ≈ 130 it）原报告完全未提，是 zod→JSON Schema 转换的可复用测试集。
- **整个 `types/*.test-d.ts`** type 测试子树（tool 35、content-part 25 等）未提。

---

## 4. packages/ai/src 核心测试完整清单（文件 + it 数）

路径：`packages\ai\src\`，共 **143 个测试文件，2926 it，47 test**。原报告此前只列文件名、未给用例数，此处补齐。按目录分组：

### generate-text/ （核心，19 文件）
| 文件 | it | test |
|---|---:|---:|
| stream-text.test.ts | 405 | 0 |
| generate-text.test.ts | 259 | 0 |
| output.test.ts | 62 | 0 |
| stream-text.test-d.ts | 59 | 0 |
| generate-text.test-d.ts | 49 | 0 |
| execute-tool-call.test.ts | 38 | 0 |
| to-response-messages.test.ts | 28 | 0 |
| resolve-tool-approval.test.ts | 25 | 0 |
| parse-tool-call.test.ts | 22 | 0 |
| stream-language-model-call.test.ts | 21 | 0 |
| tool-execution-events.test-d.ts | 20 | 0 |
| smooth-stream.test.ts | 34 | 0 |
| execute-tools-from-stream.test.ts | 16 | 0 |
| tool-approval-configuration.test-d.ts | 16 | 0 |
| restricted-telemetry-dispatcher.test.ts | 15 | 0 |
| collect-tool-approvals.test.ts | 10 | 0 |
| tool-approval-signature.test.ts | 10 | 0 |
| validate-tool-approvals.test.ts | 11 | 0 |
| tools-context-parameter.test-d.ts | 15 | 0 |
| prune-messages.test.ts | 9 | 0 |
| tool-fingerprint.test.ts | 9 | 0 |
| stop-condition.test.ts | 12 | 0 |
| calculate-tokens-per-second.test.ts | 6 | 0 |
| stream-text-timeout.test.ts | 6 | 0 |
| sum-token-counts.test.ts | 3 | 0 |
| validate-tool-context.test.ts | 3 | 0 |
| filter-active-tools.test.ts | 4 | 0 |
| filter-active-tools.test-d.ts | 7 | 0 |
| restricted-telemetry-dispatcher.test-d.ts | 2 | 0 |
| invoke-tool-callbacks-from-stream.test.ts | 1 | 0 |

### generate-object/ （4 文件）
| 文件 | it | test |
|---|---:|---:|
| stream-object.test.ts | 59 | 0 |
| generate-object.test.ts | 41 | 0 |
| generate-object.test-d.ts | 6 | 0 |
| stream-object.test-d.ts | 7 | 0 |
| inject-json-instruction.test.ts | 12 | 0 |

### agent/ （4 文件）
| 文件 | it | test |
|---|---:|---:|
| tool-loop-agent.test.ts | 105 | 0 |
| tool-loop-agent.test-d.ts | 42 | 0 |
| create-agent-ui-stream-response.test.ts | 4 | 0 |
| infer-agent-ui-message.test-d.ts | 2 | 0 |

### ui/ + ui-message-stream/ （20 文件）
| 文件 | it | test |
|---|---:|---:|
| process-ui-message-stream.test.ts | 103 | 0 |
| convert-to-model-messages.test.ts | 64 | 0 |
| validate-ui-messages.test.ts | 55 | 0 |
| chat.test-d.ts | 8 | 0 |
| chat.test.ts | 28 | 0 |
| handle-ui-message-stream-finish.test.ts | 22 | 0 |
| ui-messages.test-d.ts | 3 | 0 |
| ui-messages.test.ts | 7 | 0 |
| last-assistant-message-is-complete-with-tool-calls.test.ts | 12 | 0 |
| last-assistant-message-is-complete-with-approval-responses.test.ts | 11 | 0 |
| to-ui-message-chunk.test.ts | 11 | 0 |
| create-ui-message-stream.test.ts | 14 | 0 |
| direct-chat-transport.test.ts | 7 | 0 |
| create-ui-message-stream-response.test.ts | 7 | 0 |
| get-response-ui-message-id.test.ts | 5 | 0 |
| http-chat-transport.test.ts | 4 | 0 |
| transform-text-to-ui-message-stream.test.ts | 3 | 0 |
| pipe-ui-message-stream-to-response.test.ts | 3 | 0 |
| read-ui-message-stream.test.ts | 3 | 0 |
| ui-message-chunks.test.ts | 3 | 0 |
| process-text-stream.test.ts | 2 | 0 |
| to-ui-message-stream.test.ts | 8 | 0 |

### prompt/ （8 文件）
| 文件 | it | test |
|---|---:|---:|
| convert-to-language-model-prompt.test.ts | 68 | 0 |
| prepare-language-model-call-options.test.ts | 32 | 0 |
| create-tool-model-output.test.ts | 22 | 0 |
| file-part-data.test.ts | 18 | 0 |
| prepare-tools.test.ts | 10 | 0 |
| standardize-prompt.test.ts | 10 | 0 |
| prepare-tool-choice.test.ts | 5 | 0 |
| convert-to-language-model-prompt.validation.test.ts | 4 | 0 |

### model/ （13 文件）
| 文件 | it | test |
|---|---:|---:|
| resolve-model.test.ts | 39 | 0 |
| as-transcription-model-v3.test.ts | 20 | 0 |
| as-embedding-model-v3.test.ts | 18 | 0 |
| as-image-model-v3.test.ts | 17 | 0 |
| as-speech-model-v3.test.ts | 16 | 0 |
| as-language-model-v3.test.ts | 19 | 0 |
| as-embedding-model-v4.test.ts | 8 | 0 |
| as-image-model-v4.test.ts | 7 | 0 |
| as-language-model-v4.test.ts | 9 | 0 |
| as-provider-v4.test.ts | 7 | 0 |
| as-speech-model-v4.test.ts | 7 | 0 |
| as-transcription-model-v4.test.ts | 7 | 0 |
| as-reranking-model-v4.test.ts | 6 | 0 |
| as-video-model-v4.test.ts | 6 | 0 |

### registry/ （4 文件）
| 文件 | it | test |
|---|---:|---:|
| provider-registry.test.ts | 43 | 0 |
| custom-provider.test.ts | 37 | 0 |
| provider-registry.test-d.ts | 14 | 0 |
| custom-provider.test-d.ts | 19 | 0 |

### middleware/ （10 文件）
| 文件 | it | test |
|---|---:|---:|
| default-settings-middleware.test.ts | 22 | 0 |
| extract-json-middleware.test.ts | 23 | 0 |
| wrap-language-model.test.ts | 18 | 0 |
| add-tool-input-examples-middleware.test.ts | 13 | 0 |
| extract-reasoning-middleware.test.ts | 12 | 0 |
| wrap-image-model.test.ts | 14 | 0 |
| wrap-embedding-model.test.ts | 15 | 0 |
| default-embedding-settings-middleware.test.ts | 7 | 0 |
| simulate-streaming-middleware.test.ts | 8 | 0 |
| wrap-provider.test.ts | 3 | 0 |

### util/ （含 retry；25 文件）
| 文件 | it | test |
|---|---:|---:|
| retry-with-exponential-backoff.test.ts | 15 | 0 |
| prepare-retries.test.ts | 1 | 0 |
| merge-abort-signals.test.ts | 16 | 0 |
| is-deep-equal-data.test.ts | 14 | 0 |
| create-stitchable-stream.test.ts | 14 | 0 |
| merge-objects.test.ts | 12 | 0 |
| notify.test.ts | 12 | 0 |
| download\download.test.ts | 15 | 0 |
| set-abort-timeout.test.ts | 7 | 0 |
| simulate-readable-stream.test.ts | 7 | 0 |
| async-iterable-stream.test.ts | 11 | 0 |
| get-potential-start-index.test.ts | 6 | 0 |
| canonical-hash.test.ts | 6 | 0 |
| serial-job-executor.test.ts | 6 | 0 |
| write-to-server-response.test.ts | 6 | 0 |
| cosine-similarity.test.ts | 5 | 0 |
| get-own.test.ts | 5 | 0 |
| prepare-headers.test.ts | 5 | 0 |
| create-id-map.test.ts | 3 | 0 |
| merge-callbacks.test.ts | 3 | 0 |
| parse-partial-json.test.ts | 4 | 0 |
| split-array.test.ts | 8 | 0 |
| fix-json.test.ts | 0 | **47** |
| (其余 util 子项见全量) | … | … |

### 其余目录（telemetry/realtime/embed/transcribe/rerank/text-stream/upload-*/generate-*/logger/test/）
| 文件 | it | test |
|---|---:|---:|
| generate-video\generate-video.test.ts | 37 | 0 |
| telemetry\create-telemetry-dispatcher.test.ts | 31 | 0 |
| rerank\rerank.test.ts | 30 | 0 |
| embed\embed-many.test.ts | 34 | 0 |
| generate-image\generate-image.test.ts | 34 | 0 |
| embed\embed.test.ts | 27 | 0 |
| transcribe\stream-transcribe.test.ts | 15 | 0 |
| telemetry\tracing-channel.test.ts | 15 | 0 |
| upload-file\upload-file.test.ts | 10 | 0 |
| transcribe\transcribe.test.ts | 9 | 0 |
| generate-speech\generate-speech.test.ts | 8 | 0 |
| logger\log-warnings.test.ts | 16 | 0 |
| telemetry\telemetry-registry.test.ts | 5 | 0 |
| realtime\browser-realtime-transport.test.ts | 5 | 0 |
| upload-skill\upload-skill.test.ts | 5 | 0 |
| realtime\realtime-session.test.ts | 4 | 0 |
| realtime\realtime-event-reducer.test.ts | 3 | 0 |
| text-stream\pipe-text-stream-to-response.test.ts | 3 | 0 |
| telemetry\tracing-channel-publisher.test.ts | 2 | 0 |
| test\mock-language-model.test.ts | 6 | 0 |
| test\mock-embedding-model.test.ts | 2 | 0 |
| text-stream\create-text-stream-response.test.ts | 2 | 0 |
| text-stream\to-text-stream.test.ts | 1 | 0 |
| **packages/ai/src 合计** | **2926** | **47** |

### packages/ai/src 要点
- 用例数此前完全缺失，现补齐：单 `stream-text.test.ts` 就 405、`generate-text.test.ts` 259、`tool-loop-agent.test.ts` 105、`process-ui-message-stream.test.ts` 103。
- **retry 测试真实位置在此**：`util\retry-with-exponential-backoff.test.ts` = 15 it（原报告却把它算进 provider-utils 并记为 19，归属与数字双错）。
- `util\fix-json.test.ts` 用 `test()` 而非 `it()`，共 47 例——按 `it(` 统计会漏，本报告已合并 `it+test`。

---

## 5. 其他 provider 测试抽查

对 13 个其他 provider 包做了文件数 + 用例数清点（均存在可复用测试，原报告一个未提）：

| 包 | 文件数 | it | test |
|---|---:|---:|---:|
| google | 26 | 701 | 2 |
| xai | 18 | 416 | 0 |
| amazon-bedrock | 17 | 410 | 0 |
| google-vertex | 18 | 250 | 0 |
| groq | 6 | 98 | 0 |
| mistral | 7 | 91 | 0 |
| cohere | 5 | 75 | 0 |
| azure | 1 | 63 | 0 |
| fireworks | 2 | 52 | 0 |
| togetherai | 3 | 41 | 0 |
| deepseek | 3 | 40 | 0 |
| perplexity | 2 | 35 | 0 |
| cerebras | 2 | 13 | 0 |
| **小计** | **110** | **2285** | **2** |

要点：google(701)、xai(416)、amazon-bedrock(410)、google-vertex(250) 体量与 OpenAI/Anthropic 同级，均为高价值可复用测试源，原报告完全遗漏。其余 openai-compatible、cohere、mistral、groq 等亦各有数十例可复用。

---

## 6. 统计修正（原报告数字哪些不准）

| 原报告声称 | 实测 | 修正 |
|---|---|---|
| OpenAI 主测试 89 it | chat 89 ✅，但另有 responses 175、completion 18 等共 11 个模型测试文件 | "主测试"应至少含 chat+responses 两个 language-model = 264 it；全包 611 it |
| OpenAI 纯函数 45（convert-to-messages 33 + prepare-tools 12） | chat 路径 33+12 ✅，但漏 responses 路径 convert-to-input 120 + prepare-tools 52 + tool-search 1 | 实际纯函数类 218 it |
| Anthropic 主测试 277 it | 277 ✅ | 准确 |
| Anthropic 纯函数 148（82+51+15） | 82+51+15 ✅，但漏 sanitize-json-schema 5 | 实际 153 it |
| retry 19 例（provider-utils） | provider-utils 无 retry 测试文件；全仓唯一 retry 测试在 ai/src/util，15 it | **该项应删除/改归 ai/src，数字 19→15** |
| handle-fetch-error 7 例 | 8 例 | **7→8** |
| get-from-api 16 例 | 17 例 | **16→17** |
| extract-lines 8 例 | 8 例 | ✅ 准确 |
| 总计 P0+P1 约 346 例 | 与其自身分项之和(609)矛盾；实测仅 4 核心包就 4811 it+55 test | **346 不可用，见下节** |

### 原报告结构性遗漏
1. **OpenAI 漏整个 `responses/` 子目录**（357 it）——最严重遗漏。
2. **provider-utils 漏 84/88 文件**——仅点了 4 个且 1 个不存在；漏掉整个 zod3-to-json-schema 子树与 types type-test 子树。
3. **packages/ai/src 用例数全缺**——2926 it 此前未统计。
4. **其他 provider 全部未提**——google/xai/bedrock/vertex 等共 2285 it 可复用资源被忽略。
5. **`.test-d.ts` 类型测试体系**（OpenAI tool/、Anthropic bash/、provider-utils types/、ai/src 多处）原报告完全未纳入。

---

## 7. 修正后的总计

按"可复用测试用例"口径（it + test 合计），审计覆盖范围：

| 范围 | 文件数 | it | test | 用例合计 |
|---|---:|---:|---:|---:|
| OpenAI | 26 | 611 | 0 | 611 |
| Anthropic | 14 | 472 | 2 | 474 |
| provider-utils | 88 | 802 | 6 | 808 |
| packages/ai/src | 143 | 2926 | 47 | 2973 |
| **4 核心包小计** | **271** | **4811** | **55** | **4866** |
| 其他 provider 抽查(13 包) | 110 | 2285 | 2 | 2287 |
| **审计总计** | **381** | **7096** | **57** | **7153** |

> 注：审计总计仅含上述 17 个包；`reference\ai\packages\` 下共有 70+ 个包（含 alibaba、bytedance、moonshotai、replicate、voyage、assemblyai、cartesia、elevenlabs、deepgram 等），全仓可复用测试实际更多。原报告"346 例"相对审计总计 7153 例，**低估约 20 倍**；即便只看其声称涉及的 4 核心包(4866)，也低估约 14 倍。

### 修正后建议口径
- 若仍要保留"主测试 + 纯函数"分层：OpenAI 主测试(chat+responses) 264、纯函数 218；Anthropic 主测试 277、纯函数 153；二者合计 912 it，远超原报告的 559(134+425)。
- provider-utils 应整包纳入（808 用例 / 88 文件），而非仅 4 个文件。
- retry 项应从 provider-utils 移除，并入 packages/ai/src/util（15 it）。
