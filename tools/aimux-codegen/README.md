# aimux-codegen

The release generator reads `aimux_ops::manifest()` and checks its canonical
JSON SHA-256 against `aimux_ops::manifest_digest()`. It emits the C ABI header,
JSON Schema 2020-12 control schemas, and Go, Java, Kotlin, Swift, Dart,
TypeScript, and Python op constants. It also reads the manifest's `typeDefs`
(the Rust types' `schemars` schemas) and emits the business schemas
(`schema/types/*.json`), the DTO types and codecs of every binding and the web
console's wire types. No build script generates these files; ts-rs and
`scripts/gen_ts_types.py` are gone.

Run from the repository root:

```sh
cargo run -p aimux-codegen
cargo run -p aimux-codegen -- --check
cargo test -p aimux-codegen --test generation
```

`--root PATH` selects an output/check root; it does not select a different
manifest. `--check` regenerates into a temporary directory and compares file
sets and bytes, including the digest inventory. It never rewrites the checked
tree. Normal generation replaces files and removes obsolete artifacts in its
owned locations: `schema/ops/*.json`, `schema/types/*.json`, language DTO
files listed by each emitter (including `tools/aimux-web/web/src/types/generated/`),
`aimux-ffi/include/aimux.h`, `bindings/flutter/native/aimux_ops.h` (an identical
packaged header), and files starting with `ops.`, `ops-` or `AimuxOps` with the
language's source extension beside its constants file. Compiled caches belong to
their respective toolchains.

Python constants live in `bindings/python/python/aimux/ops.py` and its DTOs in
`_types.py` and `_error_payloads.py` beside it; the other bindings use their
`generated/` directories. The DTOs are `bindings/go/types_gen.go`,
`bindings/node/src/generated/types.ts`,
`bindings/flutter/lib/src/generated/types.dart`,
`bindings/swift/generated/Types.swift`, and for Java one class per type under
`bindings/java/generated/ai/arcships/aimux/types/` (Kotlin reuses them).
Go uses package `generated`; Java/Kotlin use `ai.arcships.aimux.generated`
with `AimuxOps` / `AimuxKotlinOps` to avoid duplicate JVM classes when Kotlin
depends on Java. Swift uses `AimuxOps`. Binding owners must add these sources to their package
or import them where their build layout requires it.

The request schema validates dispatch arguments: `handle`, `op`, `params`,
and optional `abort`. Its manifest-derived branches close the control keys;
`params` is validated by the business schema its branch references in
`schema/types/<Type>.json`.
Error payloads retain canonical domain fields inside a closed transport envelope.
`handle-id.json`, `error-envelope.json` and `stream-frame.json` copy the manifest's
control definitions; codegen does not compose a second set of control schemas.

Schema IDs use `https://aimux.dev/schema/ops/<name>.json` and
`https://aimux.dev/schema/types/<Type>.json`; references resolve
against that base. The files can be registered locally without network I/O.
Provider options are explicitly open. Handles and sequence numbers are decimal
u64 strings; handle decoding also accepts leading zeroes up to the exact numeric
upper bound. The inventory lives at
`tools/aimux-codegen/generated-files.json` and records the runtime digest.

The TypeScript emitter also writes the console wire types to
`tools/aimux-web/web/src/types/generated/`.

Drift is guarded by `cargo run -p aimux-codegen -- --check` (CI step "Ops manifest
and generated artifact drift") and by the DTO vectors:
`python3 scripts/check_dto_vectors.py <languages>` round-trips
`contract-tests/dto` through each binding's generated codecs.
