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
owned locations: `schema/ops/*.json`, `aimux-ffi/include/aimux.h`, and files
starting with `ops.`, `ops-`, or `AimuxOps` with the language's source extension
in each binding's `generated/` directory. Compiled caches and other generated
binding types belong to their respective toolchains or owners.

Constants live in `bindings/{go,java,kotlin,swift,flutter,node,python}/generated/`.
Go uses package `generated`; Java/Kotlin use `ai.arcships.aimux.generated`
with `AimuxOps` / `AimuxKotlinOps` to avoid duplicate JVM classes when Kotlin
depends on Java. Swift uses `AimuxOps`. Binding owners must add these sources to their package
or import them where their build layout requires it.

The request schema describes a control record containing `handle`, `op`,
`params`, and optional `abort`. This is a validation artifact for dispatch
arguments, not a new transport or stdio protocol. Its op branches are derived
from the manifest and control keys are closed. `params` remains canonical DTO
JSON. Ops-owned `types[].schema` records are emitted as `type-*.json` and
referenced from request branches, enforcing their control fields. Business types
without schemas retain `x-aimux-type`; complete business DTO schemas belong to
RFC-0042 (G-d). There is no hand-maintained DTO field table in this generator. Error payloads
likewise retain canonical domain fields; the transport envelope is closed.

Schema IDs use `https://aimux.dev/schema/ops/<name>.json`; references resolve
against that base. The files can be registered locally without network I/O.
Provider options are explicitly open. Handles and sequence numbers are decimal
u64 strings, including the exact upper bound. The inventory lives at
`tools/aimux-codegen/generated-files.json` and records the runtime digest.

`scripts/gen_ts_types.py` stays until RFC-0042 covers its core and Web DTO outputs (G-d).
