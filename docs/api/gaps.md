# Binding API gaps (historical)

> **Archived.** This page described the 0.5 hand-written binding surface and
> is no longer current. The 0.5 constructors, per-feature C exports and DTO
> copies were removed in 0.6.0.

Current sources of truth:

- [Shared operations](ops.md) and `runtime.describe` — operation names,
  handle kinds and DTO schemas.
- Each binding's `README.md` under [`bindings/`](../../bindings/) — current
  API, limits and known differences.
- [aimux-ffi/include/aimux.h](../../aimux-ffi/include/aimux.h) — the generated
  C header.
- [0.6 upgrade guide](../upgrading-to-0.6.md) — old-to-new migration.

A binding that lacks an operation reports it through the manifest /
`runtime.describe`; it is not tracked as a hand-written gap table here.
