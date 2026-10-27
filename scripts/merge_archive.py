#!/usr/bin/env python3
"""Merge a `libaimux_ffi.a` into one object rooted at its C API and strip the
local symbols (RFC-0043 §5.3 S1c).

    merge_archive.py <archive> [--out <archive>]

rustc gives every function its own section, so a staticlib's members spend a
third of their bytes on section headers, section-name tables and string
tables that the consumer's linker only ever reads to throw most of them away.
This script does that work once, at release time:

1. the exported C API is read twice and must agree: the `aimux_*` functions
   declared in `aimux-ffi/include/aimux.h`, and the `aimux_*` symbols the
   `aimux_ffi` members define (`llvm-nm`);
2. `ld -r --gc-sections -u <export>...` links every member into one
   relocatable object and drops every section no export reaches (GNU ld, the
   linker the archive's consumers use: the file it writes is what they read);
3. `llvm-strip --strip-unneeded` removes the local symbols the merged object
   no longer needs for relocation;
4. `llvm-ar` writes the archive back with a single member named
   `aimux_ffi.merged.o`, so `check_native_artifact.py` still finds a
   workspace crate among the members; an archive that already has only that
   member is left alone, so the step can run twice.

The linked program is unchanged: the same code, the same exports. Only ELF
and COFF archives are merged; a Mach-O archive is left as it is (ld64's `-r`
does not dead-strip, and the Mach-O archives are inside the budget without
this step). `AIMUX_LD` names the linker; `AIMUX_STRIP`, `AIMUX_AR`,
`AIMUX_NM` name the LLVM tools (see check_native_artifact.find_llvm_tool).
"""

import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_native_artifact as check  # noqa: E402

MERGED_MEMBER = "aimux_ffi.merged.o"


def header_exports(root):
    text = (root / "aimux-ffi" / "include" / "aimux.h").read_text(encoding="utf-8")
    return set(re.findall(r"(?<![\w(])(aimux_\w+)\s*\(", text))


def defined_exports(nm, members):
    """`aimux_*` symbols defined by the workspace crate `aimux_ffi`'s members
    (`members` are (path on disk, member name) pairs)."""
    names = set()
    for path, name in members:
        if check.crate_of(name) != "aimux_ffi":
            continue
        out = subprocess.run(
            [nm, "--defined-only", "--extern-only", str(path)], check=True, capture_output=True, text=True
        ).stdout
        for line in out.splitlines():
            parts = line.split()
            if parts and re.fullmatch(r"_?aimux_\w+", parts[-1]):
                names.add(parts[-1].lstrip("_") if parts[-1].startswith("_aimux_") else parts[-1])
    return names


def object_format(data):
    for _, body in check.archive_members(data):
        if body[:4] == b"\x7fELF":
            return "elf"
        if body[:4] in check.MACHO_MAGICS:
            return "macho"
        if len(body) >= 20 and check.struct.unpack_from("<H", body)[0] in check.COFF_MACHINES:
            return "coff"
    sys.exit("archive has no ELF, Mach-O or COFF member")


def main(argv):
    out = None
    if "--out" in argv:
        i = argv.index("--out")
        out = Path(argv[i + 1])
        argv = argv[:i] + argv[i + 2 :]
    if len(argv) != 1:
        sys.exit(__doc__)
    path = Path(argv[0])
    out = out or path
    root = Path(__file__).resolve().parent.parent
    data = path.read_bytes()
    if not data.startswith(b"!<arch>\n"):
        sys.exit(f"{path}: not an archive")
    names = [name for name, body in check.archive_members(data) if check.member_report(body) is not None]
    if names == [MERGED_MEMBER]:
        print(f"{path}: already merged ({len(data)} bytes)")
        if out != path:
            shutil.copyfile(path, out)
        return
    fmt = object_format(data)
    if fmt == "macho":
        print(f"{path}: Mach-O archive, left unmerged ({len(data)} bytes)")
        if out != path:
            shutil.copyfile(path, out)
        return
    crates = check.workspace_crates(root)
    ld = os.environ.get("AIMUX_LD") or shutil.which("ld") or sys.exit("no ld on PATH (set AIMUX_LD)")
    nm, strip, ar = (check.find_llvm_tool(t) for t in ("nm", "strip", "ar"))

    with tempfile.TemporaryDirectory() as tmp:
        tmp = Path(tmp)
        members = []
        for i, (name, body) in enumerate(check.archive_members(data)):
            if check.member_report(body) is None:
                continue
            member = tmp / f"{i:05d}-{Path(name).name}"
            member.write_bytes(body)
            members.append((member, name))
        exports = defined_exports(nm, members)
        declared = header_exports(root)
        if exports != declared:
            sys.exit(
                f"{path}: exported C API differs from aimux-ffi/include/aimux.h: "
                f"only in archive {sorted(exports - declared)}, only in header {sorted(declared - exports)}"
            )
        print(f"  {len(members)} members, {len(exports)} exports: {' '.join(sorted(exports))}")

        merged = tmp / MERGED_MEMBER
        cmd = [ld, "-r", "--gc-sections"]
        for export in sorted(exports):
            cmd += ["-u", export]
        cmd += [str(m) for m, _ in members] + ["-o", str(merged)]
        subprocess.run(cmd, check=True)
        print(f"  before merge: {len(data)} bytes; merged object {merged.stat().st_size} bytes ({ld} -r --gc-sections)")
        subprocess.run([strip, "--strip-unneeded", str(merged)], check=True)
        print(f"  after strip --strip-unneeded: {merged.stat().st_size} bytes ({strip})")

        merged_archive = tmp / "merged.a"
        subprocess.run([ar, "rcs", "--format=gnu", str(merged_archive), str(merged)], check=True)
        shutil.copyfile(merged_archive, out)

    result = out.read_bytes()
    print(f"size {out} {len(result)} bytes")
    if not check.check_archive(out, result, crates, expect_no_bitcode=True):
        sys.exit(1)


if __name__ == "__main__":
    main(sys.argv[1:])
