# Shared operations

`aimux-ops` exposes the following registered operation names. Use
`runtime.describe` for versions, manifest digest and available names. The Rust
`manifest::manifest()` and C `aimux_manifest_json` provide authoritative modes,
allowed handle kinds, cancellation and DTO schemas. See the
[crate README](../../aimux-ops/README.md) for ownership, limits and registered gaps.

| Family | Operations |
|---|---|
| abort | `abort.create`, `abort.signal` |
| auth | `auth.codexRefresh` |
| catalogue | `catalogue.fetch` |
| composite | `composite.moa`, `composite.router` |
| credential | `credential.create` |
| decision | `decision.capabilities`, `decision.decide` |
| embedding | `embedding.embed`, `embedding.embedMany` |
| files | `files.upload` |
| handle | `handle.drop` |
| image | `image.generate` |
| middleware | `middleware.create` |
| model | `model.info`, `model.resolve`, `model.wrap` |
| operation | `operation.cancel`, `operation.inputDone`, `operation.next`, `operation.push`, `operation.status` |
| output | `output.toOpenAI` |
| provider | `provider.create`, `provider.custom`, `provider.default`, `provider.files`, `provider.invoke`, `provider.model`, `provider.wrap` |
| recorder | `recorder.attach`, `recorder.create`, `recorder.export`, `recorder.flush`, `recorder.stats`, `recorder.stop` |
| registry | `registry.create`, `registry.provider` |
| replay | `replay.capabilities`, `replay.create`, `replay.dryRun`, `replay.inspect`, `replay.live` |
| reranking | `reranking.rerank` |
| runtime | `runtime.configure`, `runtime.describe` |
| search | `search.search` |
| sessionStore | `sessionStore.attach`, `sessionStore.calls`, `sessionStore.create`, `sessionStore.list` |
| speech | `speech.generate` |
| text | `text.consume`, `text.generate`, `text.generateObject`, `text.generateOpenAI`, `text.stream`, `text.streamOpenAI` |
| toolRepair | `toolRepair.apply`, `toolRepair.applyResult`, `toolRepair.context` |
| trace | `trace.aggregate`, `trace.clear`, `trace.create`, `trace.export`, `trace.sessionChain`, `trace.sessionTrajectory` |
| transcription | `transcription.generate`, `transcription.start` |
| transport | `transport.create` |
| video | `video.generate` |

`output.toOpenAI` is registered once by the output-conversion handler in misc.
`credential.create` and `replay.capabilities` advertise Future scope and return
UnsupportedFunctionality; the other registered operations use existing Rust
capabilities. Unsupported middleware kinds and native factory settings fail
explicitly. This inventory does not claim live provider or platform validation.
