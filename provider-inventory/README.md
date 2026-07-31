# Provider statistics & extraction results

Data comes from 7 directory/constant sources across 6 local reference
repositories, deduplicated by canonical provider ID.

## Summary

- Raw provider records: **650**
- Deduplicated providers: **325**
- aimux native modules: **172**
- Direct matches with aimux modules: **116**
- Not directly matched with aimux modules: **209**
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

## Compatibility tier (auto-extracted / inferred)

| Tier | Count |
|---|---:|
| L1 | 9 |
| L2 | 65 |
| L3 | 27 |
| L4 | 5 |
| unknown | 219 |

## Output files

- `providers.json`: complete structured data for 325 deduplicated providers.
- `providers.csv`: a flat list for easy filtering and spreadsheet processing.
- `raw-provider-records.jsonl`: 650 un-deduplicated source records, with
  source tracing preserved.

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
