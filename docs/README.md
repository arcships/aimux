# AI SDK 研究文档索引

> 本目录包含对 Vercel AI SDK（`reference/ai/`）的系统性研究文档。
> 所有文档均经源码逐条复核，复核记录附于各文档末尾。

## 文档清单

| # | 文档 | 内容 | 复核员 |
|---|------|------|--------|
| 01 | [架构与功能规划](01-architecture.md) | 三大 surface、四层消息模型、stitchable stream、V4 模型类型、关键设计决策 | Dave |
| 02 | [Provider 接口设计](02-provider-interface.md) | LanguageModelV4 契约、CallOptions 全字段、GenerateResult、StreamPart 全 variant、provider-utils | Eve |
| 03 | [Provider 实现差异](03-provider-implementations.md) | OpenAI/Anthropic/Google 三家消息格式、工具格式、流式协议、endpoint、生态全景 | Frank |
| 04 | [核心 AI 函数机制](04-core-mechanisms.md) | generate-text 多步循环、generate-object JSON 修复、middleware、agent、embed/rerank/transcribe | Kevin |
| 05 | [UI 层与框架绑定](05-ui-layer.md) | AbstractChat、ChatTransport、UIMessage 流转换、React/Svelte/Vue/Angular/RSC 绑定 | Larry |
| 06 | [外围基础设施包](06-ecosystem-packages.md) | workflow/mcp/otel/gateway/open-responses/sandbox/harness/valibot | Mike |
| 07 | [内核基础设施](07-kernel-infrastructure.md) | prompt/registry/telemetry/error/realtime/text-stream/util | Quinn |
| 08 | [补充包](08-additional-packages.md) | openai-compatible/policy-opa/harness-*/langchain/llamaindex/tui/devtools/vercel/workflow-harness | Rachel |
| 09 | [Skills/File 与 Codemod](09-skills-files-codemod.md) | SkillsV4/FilesV4 上传 surface、SharedV4ProviderReference、codemod 迁移 CLI | Vera |
| 10 | [标准参考设计](10-standards-and-reference-design.md) | V4 规范清单、Provider 实现约束、provider-utils 通用能力、覆盖范围标准 | Zoe |
| 11 | [测试体系结构](11-test-architecture.md) | Mock 模型体系、test-server、Provider 测试结构、核心 AI 函数测试 | Zane |
| 12 | [测试用例可复用性分析](12-test-reusability.md) | OpenAI/Anthropic 测试清单、conformance test、SSE/错误测试、可翻译汇总 | Zara |
| 13 | [测试统计审计](13-test-audit.md) | 逐文件用例数审计、统计修正、修正后总计（7153 用例/381 文件） | Zoe |

## 覆盖范围

### packages/ai/src/ 子目录（25 个）

| 子目录 | 覆盖文档 |
|--------|----------|
| generate-text/ | 04 |
| generate-object/ | 04 |
| generate-image/ / generate-speech/ / generate-video/ | 01（模型类型） |
| transcribe/ / rerank/ / embed/ | 04 |
| agent/ | 04 |
| model/ | 07（resolve-model） |
| ui/ / ui-message-stream/ | 05 |
| middleware/ | 04 |
| prompt/ | 07 |
| registry/ | 07 |
| telemetry/ | 07 |
| error/ | 07 |
| realtime/ | 07 |
| text-stream/ | 07 |
| util/ | 07 |
| upload-skill/ / upload-file/ | 09 |
| logger/ / test/ / types/ | 07（简述） |

### packages/ 包（70 个）

| 分类 | 数量 | 覆盖文档 |
|------|------|----------|
| 核心（ai/provider/provider-utils） | 3 | 02, 04, 07 |
| 三家主要 provider | 3 | 03 |
| OpenAI 兼容基座 | 1 | 08 |
| 同质化 LLM provider | ~22 | 03（模式），08（openai-compatible 基座） |
| 模态专用 provider | ~12 | 03（模式） |
| 网关/兼容层 | 2 | 06（gateway），08（open-responses） |
| 框架 UI 绑定 | 5 | 05 |
| Agent/Sandbox/Harness | 14 | 06（workflow/harness/sandbox），08（harness-*/workflow-harness/tui） |
| 治理/互操作 | 3 | 08（policy-opa/langchain/llamaindex） |
| 开发工具 | 3 | 08（devtools），09（codemod） |
| 其他 | 2 | 06（valibot），08（vercel） |

## 复核统计

- **13 份文档**，共 **100+ 条声明**逐条核验
- **15 位复核员**参与
- **发现并修正 20+ 处错误**（行号偏差、枚举遗漏、字段误述、路径错误、统计严重不完整等）
- **测试审计**：7153 用例 / 381 文件（17 个包），原报告低估约 20 倍
- **最终确认**：除同质化 provider 外，无实质性遗漏
