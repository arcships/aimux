# aimux 0.6.0 release checklist

> Internal working checklist. The public migration guide is
> [docs/upgrading-to-0.6.md](../upgrading-to-0.6.md).

## Prepared on `release/v0.6.0`

- [x] Workspace version and internal path deps bumped to `0.6.0`.
- [x] `aimux-ffi`'s `aimux-ops` pin bumped to `0.6.0`.
- [x] Node `package.json` / lock / generated `index.js` bumped.
- [x] Python `Cargo.toml` / `pyproject.toml` bumped.
- [x] Java, Kotlin, Flutter (including example lock) and web-console versions bumped.
- [x] `CHANGELOG.md` has a dated `[0.6.0]` section and compare links.
- [x] Public 0.6 upgrade guide written: [docs/upgrading-to-0.6.md](../upgrading-to-0.6.md).
- [x] `docs/API.md`, all `docs/api/*.md` guides, `docs/README.md` and the
      binding READMEs updated for ops 1.0 / ABI 1.
- [x] `docs/error-model.md` includes ops protocol codes `300..306`.
- [x] Provider doc generator and `docs/api/providers.md` use
      `provider.create` / `provider.model` instead of `provider(name, ...)`.
- [x] `cargo check --workspace`, `cargo fmt --check`,
      `cargo run -p aimux-codegen -- --check`,
      `scripts/gen_providers_doc.py --check` pass.

## Still required before the tag

- [ ] Merge the release PR after CI is green.
- [ ] Confirm the remaining 0.6.0 scope PRs are merged or explicitly deferred.
- [ ] Decide whether to pin `rust-toolchain.toml` (currently `stable`).
- [ ] Run the full local gate (`scripts/local-ci.sh`) or rely on the full CI
      matrix; keep the run URL as release evidence.
- [ ] Dry-run the first-publish crate: `cargo publish --dry-run -p aimux-ops`.
- [ ] Dry-run Node/Python/Flutter packaging:
      `npm pack --dry-run`, `maturin build --release`,
      `flutter pub publish --dry-run`.
- [ ] Verify the `release` environment secrets and the PyPI trusted publisher.
- [ ] Decide the Python wheel platform set. 0.5.0 published only macOS arm64,
      Linux x86_64 and Windows x64; add macOS x64 / Linux arm64 legs if 0.6.0
      promises them.
- [ ] Decide the Go module tag: add `bindings/go/v0.6.0` on the release commit
      if `go get` should resolve a release version.
- [ ] Address the Maven Central 0.5.0 deployment (it is still absent) or
      explicitly supersede it with 0.6.0.
- [ ] Confirm the release asset list (21 assets) and the 0.5.0 fixes:
      per-ABI Android names, zipped xcframework, `aimux.h`.
- [ ] Tag `v0.6.0` on the release commit and push; watch the Release workflow.
- [ ] After the JVM job, manually release the Central Portal deployment and
      verify the `aimux-java` / `aimux-kotlin` POMs are HTTP 200.
- [ ] Watch `flutter-publish`; re-run only that channel if it fails, after
      checking whether the version was already published.
- [ ] Verify crates.io, npm + all six platform packages, PyPI, Maven Central,
      pub.dev and the GitHub Release assets.
- [ ] Update `ROADMAP.md` to the 0.6.0 baseline; close or update #273/#418/#401.
- [ ] Add the post-run `.a` sizes to RFC-0043 §15.1.
- [ ] Create the `release/0.6` maintenance branch.

## Known deferred work (not 0.6.0 blockers)

- #270 recording schema 3 and transport replay.
- #372 panic strategy (`abort` vs `unwind`).
- #267 performance gate, size audit and resource limits (P/S3/D5); toolchain
  pinning is still a separate decision.
- #449 model-limit docs draft; #448 CI restructuring draft.
- Full #277 RFC/status sweep (the 0.6 migration guide and API/README updates are
  the release-critical subset).
