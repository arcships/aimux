#!/usr/bin/env python3
"""Stage the GitHub Release assets by their list and emit/verify provenance (#273, #274)."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import zipfile
from pathlib import Path
from typing import Any

GO_TARGETS = ["x86_64-unknown-linux-gnu", "x86_64-apple-darwin", "aarch64-apple-darwin", "x86_64-pc-windows-gnu"]
CONSOLE_TOOLS = ["aimux-web", "aimux-cli", "aimux-replay"]
CONSOLE_SUFFIXES = ["-linux-x64", "-macos-arm64", "-windows-x64.exe"]
ANDROID_ABIS = ["arm64-v8a", "armeabi-v7a", "x86_64"]
XCFRAMEWORK = "aimux_ffi.xcframework"
MANIFEST_FILENAME = "assets-manifest.json"
RELEASE_RUSTC_BUILDERS: tuple[str, ...] = (
    *(f"go-{t}" for t in GO_TARGETS),
    "ffi-x86_64-unknown-linux-gnu",
    "ffi-aarch64-apple-darwin",
    "ffi-x86_64-pc-windows-msvc",
    "console-x86_64-unknown-linux-gnu",
    "console-aarch64-apple-darwin",
    "console-x86_64-pc-windows-msvc",
    "flutter-android",
    "flutter-ios",
)
RUSTC_VERSION_RE = re.compile(r"^rustc \d+\.\d+\.\d+(?:[-+][0-9A-Za-z._-]+)?(?:\s+\([^()\r\n]+\))?$")
MAX_RUSTC_PROVENANCE_BYTES = 1024

ASSETS = (
    [(f"libaimux_ffi-{t}.a", f"libaimux_ffi-{t}.a") for t in GO_TARGETS]
    + [
        ("libaimux_ffi-linux-x64.so", "libaimux_ffi-linux-x64.so"),
        ("libaimux_ffi-macos-arm64.dylib", "libaimux_ffi-macos-arm64.dylib"),
        ("aimux_ffi-windows-x64.dll", "aimux_ffi-windows-x64.dll"),
        ("aimux.h", "aimux.h"),
    ]
    + [(f"{tool}{s}", f"{tool}{s}") for tool in CONSOLE_TOOLS for s in CONSOLE_SUFFIXES]
    + [(f"libaimux_ffi-android-{abi}.so", f"android/{abi}/libaimux_ffi.so") for abi in ANDROID_ABIS]
    + [(f"{XCFRAMEWORK}.zip", XCFRAMEWORK)]
)


class StageError(ValueError):
    """Raised when staging or manifest verification fails."""


def _strip_single_v(val: str) -> str:
    s = val.strip()
    return s[1:] if s.startswith("v") else s


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def parse_workspace_version(cargo_toml: Path) -> str:
    if not cargo_toml.is_file():
        raise StageError(f"missing Cargo.toml: {cargo_toml}")
    text = cargo_toml.read_text(encoding="utf-8")
    try:
        import tomllib  # type: ignore[import-not-found]
        ver = tomllib.loads(text).get("workspace", {}).get("package", {}).get("version")
        if isinstance(ver, str) and ver.strip():
            return ver.strip()
        raise StageError(f"{cargo_toml}: missing [workspace.package] version")
    except ImportError:
        pass
    in_ws = False
    for line in text.splitlines():
        s = line.strip()
        if s.startswith("["):
            in_ws = s == "[workspace.package]"
        elif in_ws and s.startswith("version"):
            m = re.match(r'''^version\s*=\s*(?:"([^"]+)"|'([^']+)')''', s)
            if m:
                return (m.group(1) or m.group(2)).strip()
    raise StageError(f"{cargo_toml}: missing [workspace.package] version")


def resolve_source_sha(repo_root: Path, cli_sha: str | None = None) -> str:
    sha = (cli_sha if cli_sha is not None else os.environ.get("GITHUB_SHA", "")).strip()
    if not sha:
        try:
            sha = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo_root, text=True).strip()
        except (OSError, subprocess.CalledProcessError) as exc:
            raise StageError(f"failed to resolve sourceSha: {exc}") from exc
    if not re.fullmatch(r"[0-9a-fA-F]{7,64}", sha):
        raise StageError(f"invalid sourceSha format: {sha!r}")
    return sha.lower()


def resolve_release_version(repo_root: Path, cli_version: str | None = None, cli_tag: str | None = None) -> str:
    version = cli_version.strip() if cli_version is not None else parse_workspace_version(repo_root / "Cargo.toml")
    if not version or any(ch.isspace() for ch in version):
        raise StageError(f"invalid releaseVersion: {version!r}")
    tag = (cli_tag or "").strip()
    if not tag and os.environ.get("GITHUB_REF", "").startswith("refs/tags/"):
        tag = os.environ["GITHUB_REF"][len("refs/tags/") :].strip()
    if tag and _strip_single_v(tag) != version:
        raise StageError(f"releaseVersion {version!r} disagrees with tag {tag!r}")
    return version


def validate_rustc_version(raw: Any, source: str = "--rustc-version") -> str:
    if not isinstance(raw, str):
        raise StageError(f"{source}: rustcVersion must be a string")
    text = raw.strip()
    if not text or "\n" in text or "\r" in text or not RUSTC_VERSION_RE.fullmatch(text):
        raise StageError(f"{source}: invalid rustc -V string: {raw!r}")
    return text


def parse_expected_rustc_builders(raw: str | None) -> tuple[str, ...] | None:
    if raw is None:
        return None
    s = raw.strip()
    if not s:
        raise StageError("--expected-rustc-builders must not be empty")
    if s == "release":
        return RELEASE_RUSTC_BUILDERS
    items = tuple(p.strip() for p in s.split(",") if p.strip())
    if not items or len(set(items)) != len(items):
        raise StageError(f"--expected-rustc-builders has empty/duplicate entries: {raw!r}")
    return items


def collect_rustc_versions(rustc_dir: Path, expected: tuple[str, ...] | None = None) -> tuple[str, dict[str, str]]:
    if not rustc_dir.exists() or rustc_dir.is_symlink() or not rustc_dir.is_dir():
        raise StageError(f"{rustc_dir}: missing or invalid rustc provenance directory")
    versions: dict[str, str] = {}
    for entry in sorted(rustc_dir.iterdir(), key=lambda p: p.name):
        if entry.is_symlink() or not entry.is_file() or entry.suffix != ".txt" or not entry.stem:
            raise StageError(f"{entry}: expected regular *.txt provenance file")
        with entry.open("rb") as fh:
            raw_bytes = fh.read(MAX_RUSTC_PROVENANCE_BYTES + 1)
        if len(raw_bytes) > MAX_RUSTC_PROVENANCE_BYTES:
            raise StageError(f"{entry}: exceeds {MAX_RUSTC_PROVENANCE_BYTES}-byte provenance limit")
        try:
            versions[entry.stem] = validate_rustc_version(raw_bytes.decode("utf-8"), str(entry))
        except UnicodeDecodeError as exc:
            raise StageError(f"{entry}: invalid UTF-8") from exc
    if not versions:
        raise StageError(f"{rustc_dir}: contains no *.txt rustc provenance files")
    if expected is not None and set(versions) != set(expected):
        raise StageError(f"{rustc_dir}: rustc provenance builder mismatch: missing={sorted(set(expected) - set(versions))}, extra={sorted(set(versions) - set(expected))}")
    unique = sorted(set(versions.values()))
    return (unique[0] if len(unique) == 1 else "; ".join(unique)), dict(sorted(versions.items()))


def _make_deterministic_zip(src: Path, rel: str, out_zip: Path) -> Path:
    entries = sorted((src / rel).rglob("*"), key=lambda p: p.relative_to(src).as_posix())
    out_zip.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(out_zip, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as zf:
        for p in entries:
            if p.is_symlink():
                raise StageError(f"{p}: symlinks are not allowed inside {rel}")
            arcname = p.relative_to(src).as_posix()
            if p.is_dir():
                zi = zipfile.ZipInfo(arcname + "/", date_time=(1980, 1, 1, 0, 0, 0))
                zi.external_attr = (0o40755 << 16) | 0x10
                zf.writestr(zi, b"")
            elif p.is_file():
                zi = zipfile.ZipInfo(arcname, date_time=(1980, 1, 1, 0, 0, 0))
                zi.compress_type = zipfile.ZIP_DEFLATED
                zi.external_attr = (0o100755 if os.access(p, os.X_OK) else 0o100644) << 16
                zf.writestr(zi, p.read_bytes())
            else:
                raise StageError(f"{p}: unsupported non-regular entry inside {rel}")
    return out_zip


def stage_with_hashes(src: Path, out: Path) -> tuple[list[tuple[str, int, str]], list[str], list[str]]:
    out.mkdir(parents=True, exist_ok=True)
    staged, missing, used = [], [], set()
    for name, rel in ASSETS:
        path = src / rel
        if path.is_symlink():
            raise StageError(f"{path}: symlink not allowed as release asset")
        if rel == XCFRAMEWORK:
            if not path.exists():
                missing.append(rel)
                continue
            if not path.is_dir():
                raise StageError(f"{path}: expected directory for {XCFRAMEWORK}")
            archive = _make_deterministic_zip(src, rel, out / name)
            used.update(p for p in path.rglob("*") if p.is_file())
            staged.append((name, archive.stat().st_size, sha256_file(archive)))
        elif path.is_file():
            shutil.copyfile(path, out / name)
            used.add(path)
            staged.append((name, (out / name).stat().st_size, sha256_file(out / name)))
        elif path.exists():
            raise StageError(f"{path}: expected regular file for release asset {name!r}")
        else:
            missing.append(rel)
    left_out = sorted(str(p.relative_to(src)) for p in src.rglob("*") if p.is_file() and p not in used)
    return staged, missing, left_out


def stage(src: Path, out: Path):
    staged_h, missing, left_out = stage_with_hashes(src, out)
    return [(n, sz) for n, sz, _ in staged_h], missing, left_out


def verify_staged_dir(
    staged_dir: Path,
    *,
    expected_source_sha: str | None = None,
    expected_version: str | None = None,
    require_rustc_version: bool = False,
    expected_rustc_builders: tuple[str, ...] | None = None,
) -> dict[str, Any]:
    mp = staged_dir / MANIFEST_FILENAME
    if not mp.is_file() or mp.is_symlink():
        raise StageError(f"missing {mp}")
    data = json.loads(mp.read_text(encoding="utf-8"))
    for key in ("sourceSha", "releaseVersion", "assets"):
        if not data.get(key):
            raise StageError(f"{mp}: missing required field {key!r}")
    src_sha = str(data["sourceSha"]).strip()
    if not re.fullmatch(r"[0-9a-fA-F]{7,64}", src_sha):
        raise StageError(f"{mp}: invalid sourceSha {src_sha!r}")
    if expected_source_sha is not None and src_sha.lower() != expected_source_sha.strip().lower():
        raise StageError(f"{mp}: sourceSha {src_sha!r} != expected {expected_source_sha!r}")
    if expected_version is not None and _strip_single_v(str(data["releaseVersion"])) != _strip_single_v(expected_version):
        raise StageError(f"{mp}: releaseVersion {data['releaseVersion']!r} != expected {expected_version!r}")

    rustc_ver, rustc_versions = data.get("rustcVersion"), data.get("rustcVersions")
    if (require_rustc_version or expected_rustc_builders is not None) and not rustc_ver:
        raise StageError(f"{mp}: missing required 'rustcVersion'")
    if "rustcVersion" in data:
        if not isinstance(rustc_ver, str) or not rustc_ver.strip():
            raise StageError(f"{mp}: invalid rustcVersion {rustc_ver!r}")
        tokens = [validate_rustc_version(t, f"{mp}:rustcVersion") for t in rustc_ver.split("; ")]
        if len(set(tokens)) != len(tokens) or "; ".join(sorted(set(tokens))) != rustc_ver:
            raise StageError(f"{mp}: non-canonical rustcVersion {rustc_ver!r}")
    if expected_rustc_builders is not None and rustc_versions is None:
        raise StageError(f"{mp}: missing required 'rustcVersions'")
    if "rustcVersions" in data:
        if not isinstance(rustc_versions, dict) or not rustc_versions:
            raise StageError(f"{mp}: rustcVersions must be a non-empty object")
        for b, v in rustc_versions.items():
            if not isinstance(b, str) or not b.strip() or b != b.strip() or v != validate_rustc_version(v, f"{mp}:rustcVersions[{b!r}]"):
                raise StageError(f"{mp}: invalid rustcVersions entry {b!r}: {v!r}")
        if rustc_ver != "; ".join(sorted(set(rustc_versions.values()))):
            raise StageError(f"{mp}: rustcVersion disagrees with rustcVersions summary")
        if expected_rustc_builders is not None and set(rustc_versions) != set(expected_rustc_builders):
            raise StageError(f"{mp}: rustcVersions builders mismatch: missing={sorted(set(expected_rustc_builders) - set(rustc_versions))}, extra={sorted(set(rustc_versions) - set(expected_rustc_builders))}")

    assets = data["assets"]
    if not isinstance(assets, list) or not assets:
        raise StageError(f"{mp}: empty assets list")
    names, expected_names = [a["name"] for a in assets], {n for n, _ in ASSETS}
    if len(names) != len(set(names)) or set(names) != expected_names:
        raise StageError(f"{mp}: asset list disagrees with ASSETS")
    if {p.name for p in staged_dir.iterdir()} != expected_names | {MANIFEST_FILENAME}:
        raise StageError(f"{staged_dir}: directory entries mismatch manifest")
    for item in assets:
        fp = staged_dir / item["name"]
        if fp.is_symlink() or not fp.is_file() or fp.stat().st_size != item["bytes"] or sha256_file(fp) != item["sha256"]:
            raise StageError(f"{fp}: staged asset bytes/sha256 mismatch manifest")
    return data


def main(argv: list[str]) -> int:
    p = argparse.ArgumentParser(prog=Path(argv[0]).name if argv else "stage_release_assets.py")
    p.add_argument("paths", nargs="+", type=Path)
    p.add_argument("--repo-root", type=Path, default=Path(__file__).resolve().parents[1])
    p.add_argument("--source-sha", default=None)
    p.add_argument("--release-version", default=None)
    p.add_argument("--tag", default=None)
    p.add_argument("--rustc-version", default=None)
    p.add_argument("--rustc-version-dir", type=Path, default=None)
    p.add_argument("--expected-rustc-builders", default=None)
    p.add_argument("--require-rustc-version", action="store_true")
    p.add_argument("--verify", action="store_true")
    try:
        args = p.parse_args(argv[1:])
    except SystemExit as exc:
        return int(exc.code) if isinstance(exc.code, int) else 2

    try:
        if args.verify:
            if len(args.paths) != 1:
                raise StageError("--verify takes exactly one <staged-dir> argument")
            exp_ver = resolve_release_version(args.repo_root.resolve(), args.release_version, args.tag) if (args.release_version or args.tag) else None
            data = verify_staged_dir(
                args.paths[0], expected_source_sha=args.source_sha, expected_version=exp_ver,
                require_rustc_version=args.require_rustc_version, expected_rustc_builders=parse_expected_rustc_builders(args.expected_rustc_builders),
            )
            print(f"verified {args.paths[0] / MANIFEST_FILENAME} ({len(data['assets'])} assets, version={data['releaseVersion']})")
            return 0

        if len(args.paths) != 2:
            raise StageError("expected <src> <out> arguments")
        if args.rustc_version is not None and args.rustc_version_dir is not None:
            raise StageError("--rustc-version and --rustc-version-dir are mutually exclusive")
        if args.expected_rustc_builders is not None and args.rustc_version_dir is None:
            raise StageError("--expected-rustc-builders requires --rustc-version-dir")

        src, out = args.paths
        manifest_out = out / MANIFEST_FILENAME
        manifest_out.unlink(missing_ok=True)
        repo = args.repo_root.resolve()
        source_sha, release_ver = resolve_source_sha(repo, args.source_sha), resolve_release_version(repo, args.release_version, args.tag)

        rustc_ver, rustc_versions = None, None
        if args.rustc_version is not None:
            rustc_ver = validate_rustc_version(args.rustc_version)
        elif args.rustc_version_dir is not None:
            rustc_ver, rustc_versions = collect_rustc_versions(args.rustc_version_dir, parse_expected_rustc_builders(args.expected_rustc_builders))
        elif args.require_rustc_version:
            raise StageError("--require-rustc-version requires --rustc-version or --rustc-version-dir")

        staged, missing, left_out = stage_with_hashes(src, out)
    except StageError as exc:
        print(f"staging error: {exc}", file=sys.stderr)
        return 1

    for name, size, digest in staged:
        print(f"size {name} {size} bytes sha256={digest}")
    for rel in left_out:
        print(f"not on the release list, left out: {rel}")

    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as fh:
            fh.write("### Release assets\n\n| Asset | Bytes | SHA-256 |\n|---|---:|---|\n")
            fh.writelines(f"| `{n}` | {sz:,} | `{dg}` |\n" for n, sz, dg in staged)
            fh.write(f"\nProvenance: version `{release_ver}`, source `{source_sha}`" + (f", rustc `{rustc_ver}`" if rustc_ver else "") + ".\n")
            if missing:
                fh.write("\nMissing: " + ", ".join(f"`{rel}`" for rel in missing) + "\n")
    if missing:
        for rel in missing:
            print(f"missing from the downloaded artifacts: {rel}", file=sys.stderr)
        return 1

    manifest = {
        "schemaVersion": 1, "sourceSha": source_sha, "releaseVersion": release_ver,
        **({"rustcVersion": rustc_ver} if rustc_ver else {}),
        **({"rustcVersions": rustc_versions} if rustc_versions else {}),
        "assets": sorted(({"name": n, "bytes": sz, "sha256": dg} for n, sz, dg in staged), key=lambda x: x["name"]),
    }
    manifest_out.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {manifest_out} ({len(manifest['assets'])} assets)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
