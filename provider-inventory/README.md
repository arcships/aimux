# Provider statistics & extraction results

Data comes from 7 directory/constant sources across 6 local reference
repositories, **merged with the 104-project reference audit**
(`docs/internal/reference-audit/`, 2026-08-01), deduplicated by canonical
provider ID.

## Summary

- Raw provider records: **650**
- Deduplicated providers (v1.0): **325**
- Audit canonicals merged in (2026-08-01): **552**(2026-08-01 裁剪后仅保留 2 个:**codex**、**azure_ai_foundry**)
- **Total providers (v1.1,裁剪后): 327** = 325 原有 + 2 保留
- Audit-matched existing providers: **293**
- aimux native modules: **172** (157 have same-name inventory entries)
- With endpoint metadata: **190**
- With model metadata: **262**

## Records by source

| Source | Records |
|---|---:|
| litellm_constants | 110 |
| litellm_prices | 122 |
| mastra_registry | 160 |
| new_api | 57 |
| pydantic_ai | 17 |
| rust_genai | 29 |
| tokenhub | 155 |
| reference-audit (104 projects) | 552 new(保留 2) |

## Classification (provider_kind, v1.1)

| Category | Total | 说明 |
|---|---:|---|
| model_vendor | 274 | 云 LLM 厂商/API 服务 |
| gateway_aggregator | 10 | 网关/聚合/路由服务(litellm、openrouter、one-api…) |
| deployment_platform | 11 | 模型托管/部署平台(replicate、baseten、modal…) |
| cloud_platform | 11 | 云平台通道(azure_openai、google_vertex、bedrock、azure_ai_foundry…) |
| local_runtime | 10 | 本地/自托管推理引擎(ollama、vllm、lmstudio…) |
| subscription_proxy | 3 | 订阅/登录通道(github_copilot、chatgpt、codex) |
| modal_service | 4 | 模态服务(elevenlabs、midjourney、stability…) |
| embedding | 2 | 向量嵌入服务(voyage、jina) |
| search_provider | 2 | 搜索服务(tavily、serper) |

> 分类为启发式推断(名称名单 + 审计 access 证据),新条目 `trust.status=review`,
> 协议未经官方文档核验,集成前必须按 [RFC-0006](../rfc/0006-provider-development.md) 复核。

## Integration candidates (from audit)

审计数据曾为新条目标注 `integration_candidate.priority`(按支持项目数);2026-08-01 已裁剪,仅保留 `codex` 与 `azure_ai_foundry` 两个候选:
**high ≥4 项目(10 个)、medium 2–3 项目(37 个)、low 1 项目(505 个)**;裁剪后仅保留 high 中的 codex 与 azure_ai_foundry。
候选历史见 [INTEGRATION-CANDIDATES.md](INTEGRATION-CANDIDATES.md)(留档),聚焦版见 [INTEGRATION-PRIORITY.md](INTEGRATION-PRIORITY.md)。

## Compatibility tier (auto-extracted / inferred)

| Tier | Count |
|---|---:|
| L1 | 9 |
| L2 | 65 |
| L3 | 27 |
| L4 | 5 |
| unknown | 221 |

## Output files

- `providers.json`: complete structured data for 327 providers(裁剪后) (schema 1.1.0,
  新增 `audit` 字段与 `integration_candidate` 字段).
- `providers.csv`: flat list with `audit_projects` / `audit_count` /
  `integration_candidate` columns.
- `raw-provider-records.jsonl`: 650 un-deduplicated source records.
- 审计采集与分析文档(104 项目 md/json、去重清单、候选历史、经验核对)已移至 `reference/audit/`(gitignore,不入库);inventory 仅保留合并后的数据\n- 保留候选(codex、azure_ai_foundry)见 RFC-0018 与 `reference/audit/INTEGRATION-PRIORITY.md`

## Development entry point

When adding or redoing a provider, verify the identity and protocol required
for this capability per [RFC-0006: Provider development
specification](../rfc/0006-provider-development.md), choose an implementation
path, and complete the corresponding tests. The inventory fields are for
candidate filtering and source tracing only and cannot replace the provider's
official protocol documentation.

> "Not directly matched with an aimux module" only means the canonical ID has
> no same-named module; it does not confirm a missing implementation. Aliases
> and aggregate entry points still require manual review.
