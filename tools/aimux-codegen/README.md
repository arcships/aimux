# aimux-codegen

The release generator reads `aimux_ops::manifest()` and checks its canonical
JSON SHA-256 against `aimux_ops::manifest_digest()`. It emits the C ABI header,
JSON Schema 2020-12 control schemas, and Go, Java, Kotlin, Swift, Dart,
TypeScript, and Python op constants. No build script generates these files.

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
owned locations: `schema/ops/*.json`, `aimux-ffi/include/aimux.h`,
`bindings/flutter/native/aimux_ops.h` (an identical packaged header), and files
starting with `ops.`, `ops-`, or `AimuxOps` with the language's source extension
in each binding's `generated/` directory. Compiled caches and other generated
binding types belong to their respective toolchains or owners.

Constants live in `bindings/{go,java,kotlin,swift,flutter,node,python}/generated/`.
Go uses package `generated`; Java/Kotlin use `ai.arcships.aimux.generated`
with `AimuxOps` / `AimuxKotlinOps` to avoid duplicate JVM classes when Kotlin
depends on Java. Swift uses `AimuxOps`. Binding owners must add these sources to their package
or import them where their build layout requires it.

The request schema validates dispatch arguments: `handle`, `op`, `params`,
and optional `abort`. Its manifest-derived branches close the control keys;
`params` retains canonical DTO JSON. Ops-owned `types[].schema` records emit
`type-*.json` and are referenced from request branches. Business types without
schemas retain `x-aimux-type`; business DTO schemas remain separate work.
Error payloads retain canonical domain fields inside a closed transport envelope.

Schema IDs use `https://aimux.dev/schema/ops/<name>.json`; references resolve
against that base. The files can be registered locally without network I/O.
Provider options are explicitly open. Handles and sequence numbers are decimal
u64 strings, including the exact upper bound. The inventory lives at
`tools/aimux-codegen/generated-files.json` and records the runtime digest.

`scripts/gen_ts_types.py` still owns core and Web DTO outputs.
