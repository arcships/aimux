#!/usr/bin/env python3
"""Measure a shipped native library and, for a static archive, check its
members for LLVM bitcode (RFC-0043 §3.1, §5.2).

    check_native_artifact.py [--strip-bitcode] <file>...

Every file gets one line, `size <path> <bytes> bytes`: the raw_bytes
measurement of RFC-0043 §3.1, so the CI log of each artifact job carries the
number the RFC's tables are filled from.

`--strip-bitcode` first removes the bitcode sections from every member of
each archive, in place, with `rust-objcopy` (the `llvm-objcopy` of rustup's
`llvm-tools` component; `AIMUX_OBJCOPY` names another binary). That is
RFC-0043 §5.3 S1b: rustup's sysroot objects (std, alloc, core, ...) are
compiled with `-C embed-bitcode=yes` and keep their LLVM IR even when the
workspace builds with `lto = false`, about 10 MB of every archive that no
consumer's linker reads. The fat-LTO check below runs on the members as
built, before the strip, so a strip cannot "cure" a workspace object that
carries bitcode and hide a fat-LTO archive; after the strip the archive must
carry no bitcode at all, and the `size` line is the stripped archive, the
file the release ships. `scripts/tests/test_check_native_artifact.py` keeps
the negative case (a workspace member with `.llvmbc`) failing both ways.

A static archive (`!<arch>`) is also opened member by member. A workspace
crate's object that carries a bitcode section means the archive was built
with a fat-LTO profile instead of `staticlib-release`: cargo passes
`-C embed-bitcode=yes` under fat LTO and every codegen unit ships its LLVM
IR next to the machine code, which the consumer's linker never reads and
which doubles the archive (RFC-0043 §5.1, §15.1). The sysroot's own objects
(std, alloc, core, ...) always carry bitcode, because rustup ships them that
way, so without `--strip-bitcode` they are only counted and reported.

The archive, ELF, Mach-O and COFF headers are read directly, so the check
is the same on the Linux, macOS and Windows runners and needs no binutils.
"""

import os
import re
import shutil
import struct
import subprocess
import sys
from pathlib import Path

ELF_BITCODE = {".llvmbc", ".llvmcmd"}
MACHO_BITCODE = {("__LLVM", "__bitcode"), ("__LLVM", "__cmdline")}
COFF_MACHINES = {0x14C, 0x8664, 0xAA64, 0x1C0, 0x1C4}
MACHO_MAGICS = {
    b"\xfe\xed\xfa\xce": (">", 32),
    b"\xce\xfa\xed\xfe": ("<", 32),
    b"\xfe\xed\xfa\xcf": (">", 64),
    b"\xcf\xfa\xed\xfe": ("<", 64),
}


def workspace_crates(root):
    """Crate names of the workspace members in the root Cargo.toml, as they
    appear in object names (`aimux-core` -> `aimux_core`)."""
    text = (root / "Cargo.toml").read_text(encoding="utf-8")
    block = re.search(r"^\[workspace\]\s*\nmembers\s*=\s*\[(.*?)\]", text, re.S | re.M)
    if not block:
        sys.exit("Cargo.toml: no [workspace] members list")
    return {Path(m).name.replace("-", "_") for m in re.findall(r'"([^"]+)"', block.group(1))}


def archive_members(data):
    """Yield (name, bytes) for each member of a GNU or BSD `ar` archive."""
    offset = 8
    long_names = b""
    while offset + 60 <= len(data):
        header = data[offset : offset + 60]
        if header[58:60] != b"`\n":
            sys.exit(f"bad archive member header at {offset}")
        name = header[:16].rstrip()
        size = int(header[48:58])
        body = data[offset + 60 : offset + 60 + size]
        offset += 60 + size + (size & 1)
        if name == b"//":
            long_names = body
            continue
        if name in (b"/", b"/SYM64/") or name.startswith(b"__.SYMDEF"):
            continue
        if name.startswith(b"#1/"):  # BSD: the name leads the body
            length = int(name[3:])
            name, body = body[:length].rstrip(b"\0"), body[length:]
        elif name.startswith(b"/"):  # GNU: offset into the long-name table
            start = int(name[1:])
            name = long_names[start : long_names.index(b"/\n", start)]
        else:
            name = name.rstrip(b"/")
        yield name.decode("utf-8", "replace"), body


def elf_sections(body):
    """(name, size) of each section of an ELF object."""
    bits = {1: 32, 2: 64}[body[4]]
    end = {1: "<", 2: ">"}[body[5]]
    if bits == 64:
        shoff = struct.unpack_from(end + "Q", body, 0x28)[0]
        shentsize, shnum, shstrndx = struct.unpack_from(end + "HHH", body, 0x3A)
        fmt = end + "IIQQQQIIQQ"
    else:
        shoff = struct.unpack_from(end + "I", body, 0x20)[0]
        shentsize, shnum, shstrndx = struct.unpack_from(end + "HHH", body, 0x2E)
        fmt = end + "IIIIIIIIII"
    if shoff == 0 or shoff + shentsize > len(body):
        return  # no section header table (e.g. a truncated or stripped-down object)
    # An object with 65280+ sections (a merged `ld -r` archive member) keeps
    # the real counts in section header 0 (ELF extended numbering).
    first = struct.unpack_from(fmt, body, shoff)
    if shnum == 0:
        shnum = first[5]
    if shstrndx == 0xFFFF:
        shstrndx = first[6]
    headers = [struct.unpack_from(fmt, body, shoff + i * shentsize) for i in range(shnum)]
    str_off, str_size = headers[shstrndx][4], headers[shstrndx][5]
    strtab = body[str_off : str_off + str_size]
    for h in headers:
        name = strtab[h[0] : strtab.index(b"\0", h[0])].decode()
        yield name, h[5]


def macho_sections(body):
    """((segment, section), size) of each section of a Mach-O object."""
    end, bits = MACHO_MAGICS[body[:4]]
    ncmds = struct.unpack_from(end + "I", body, 16)[0]
    offset = 32 if bits == 64 else 28
    for _ in range(ncmds):
        cmd, cmdsize = struct.unpack_from(end + "II", body, offset)
        if cmd in (0x19, 0x1):  # LC_SEGMENT_64, LC_SEGMENT
            # An object file has one unnamed segment; each section names
            # the segment it will land in.
            if cmd == 0x19:
                nsects = struct.unpack_from(end + "I", body, offset + 64)[0]
                sect, step, size_fmt, size_off = offset + 72, 80, "Q", 40
            else:
                nsects = struct.unpack_from(end + "I", body, offset + 48)[0]
                sect, step, size_fmt, size_off = offset + 56, 68, "I", 36
            for _ in range(nsects):
                sectname = body[sect : sect + 16].rstrip(b"\0").decode()
                segname = body[sect + 16 : sect + 32].rstrip(b"\0").decode()
                size = struct.unpack_from(end + size_fmt, body, sect + size_off)[0]
                yield (segname, sectname), size
                sect += step
        offset += cmdsize


def coff_sections(body):
    """(name, size) of each section of a COFF object (x86_64-pc-windows-gnu)."""
    nsections, _, _, _, opt = struct.unpack_from("<HIIIH", body, 2)
    offset = 20 + opt
    for _ in range(nsections):
        name = body[offset : offset + 8].rstrip(b"\0").decode("ascii", "replace")
        size = struct.unpack_from("<I", body, offset + 16)[0]
        yield name, size
        offset += 40


def member_report(body):
    """(text_bytes, bitcode_bytes) of one archive member; None if not an object."""
    text = bitcode = 0
    if body[:4] == b"\x7fELF":
        for name, size in elf_sections(body):
            if name in ELF_BITCODE:
                bitcode += size
            elif name == ".text" or name.startswith(".text."):
                text += size
    elif body[:4] in MACHO_MAGICS:
        for (seg, sect), size in macho_sections(body):
            if (seg, sect) in MACHO_BITCODE:
                bitcode += size
            elif seg == "__TEXT" and sect == "__text":
                text += size
    elif len(body) >= 20 and struct.unpack_from("<H", body)[0] in COFF_MACHINES:
        for name, size in coff_sections(body):
            if name in ELF_BITCODE:
                bitcode += size
            elif name == ".text" or name.startswith(".text$"):
                text += size
    elif body[:4] == b"BC\xc0\xde":  # a bitcode-only member
        bitcode = len(body)
    else:
        return None
    return text, bitcode


def crate_of(member_name):
    return re.split(r"[-.]", member_name, maxsplit=1)[0]


def find_objcopy():
    """The llvm-objcopy to strip sections with: `AIMUX_OBJCOPY`, else
    rustup's `rust-objcopy` next to the active rustc (the `llvm-tools`
    component), else an `llvm-objcopy` on PATH."""
    named = os.environ.get("AIMUX_OBJCOPY")
    if named:
        return named
    try:
        sysroot = subprocess.run(
            ["rustc", "--print", "sysroot"], check=True, capture_output=True, text=True
        ).stdout.strip()
        verbose = subprocess.run(["rustc", "-vV"], check=True, capture_output=True, text=True).stdout
        host = re.search(r"^host: (\S+)", verbose, re.M).group(1)
        exe = ".exe" if sys.platform == "win32" else ""
        candidate = Path(sysroot) / "lib" / "rustlib" / host / "bin" / f"rust-objcopy{exe}"
        if candidate.is_file():
            # The stub in a toolchain without llvm-tools exists but cannot
            # load libLLVM; a --version run tells the two apart.
            if subprocess.run([str(candidate), "--version"], capture_output=True).returncode == 0:
                return str(candidate)
    except (OSError, subprocess.CalledProcessError, AttributeError):
        pass
    on_path = shutil.which("llvm-objcopy")
    if on_path:
        return on_path
    sys.exit(
        "no llvm-objcopy: install rustup's llvm-tools component "
        "(`rustup component add llvm-tools`) or set AIMUX_OBJCOPY"
    )


def bitcode_section_names(data):
    """The bitcode section names of the archive's object format: ELF and
    COFF name them `.llvmbc` / `.llvmcmd`, Mach-O `__LLVM,__bitcode` /
    `__LLVM,__cmdline`."""
    for _, body in archive_members(data):
        if body[:4] in MACHO_MAGICS:
            return [f"{seg},{sect}" for seg, sect in sorted(MACHO_BITCODE)]
        if body[:4] == b"\x7fELF" or (
            len(body) >= 20 and struct.unpack_from("<H", body)[0] in COFF_MACHINES
        ):
            return sorted(ELF_BITCODE)
    sys.exit("archive has no ELF, Mach-O or COFF member")


def strip_bitcode(path, data):
    """Remove the bitcode sections from every member of the archive at
    `path`, in place, and return the rewritten bytes."""
    objcopy = find_objcopy()
    args = [objcopy]
    for name in bitcode_section_names(data):
        args += ["--remove-section", name]
    args.append(str(path))
    print(f"  before bitcode strip: {len(data)} bytes; {' '.join(args)}")
    subprocess.run(args, check=True)
    return path.read_bytes()


def check_archive(path, data, crates, expect_no_bitcode=False):
    members = text = bitcode = 0
    bitcode_members = 0
    workspace_members = 0
    fat = []
    for name, body in archive_members(data):
        report = member_report(body)
        if report is None:
            continue
        members += 1
        text += report[0]
        bitcode += report[1]
        if report[1]:
            bitcode_members += 1
        if crate_of(name) in crates:
            workspace_members += 1
            if report[1]:
                fat.append(name)
    print(
        f"  {members} objects, .text {text} bytes, bitcode {bitcode} bytes in "
        f"{bitcode_members} objects ({len(fat)} of them workspace crates)"
    )
    if not workspace_members:
        print(f"{path}: no workspace crate among the archive members", file=sys.stderr)
        return False
    if fat:
        print(
            f"{path}: workspace objects carry LLVM bitcode, so the archive was "
            f"built with a fat-LTO profile instead of staticlib-release: "
            f"{', '.join(fat[:5])}{' ...' if len(fat) > 5 else ''}",
            file=sys.stderr,
        )
        return False
    if expect_no_bitcode and bitcode:
        print(
            f"{path}: {bitcode} bytes of LLVM bitcode left in {bitcode_members} "
            f"objects after the strip (RFC-0043 §5.3 S1b: the shipped archive "
            f"carries none)",
            file=sys.stderr,
        )
        return False
    return True


def main(argv):
    strip = "--strip-bitcode" in argv
    files = [a for a in argv if a != "--strip-bitcode"]
    if not files:
        sys.exit(__doc__)
    crates = workspace_crates(Path(__file__).resolve().parent.parent)
    ok = True
    for arg in files:
        path = Path(arg)
        data = path.read_bytes()
        is_archive = data.startswith(b"!<arch>\n")
        if strip and is_archive:
            # The fat-LTO guard reads the members as built. Stripping first
            # would remove the very bitcode that betrays a workspace object
            # compiled under fat LTO, so the archive is judged before the
            # objcopy and skipped (left as it is) when it fails.
            if not check_archive(path, data, crates):
                print(f"size {path} {len(data)} bytes")
                ok = False
                continue
            data = strip_bitcode(path, data)
        print(f"size {path} {len(data)} bytes")
        if is_archive:
            ok = check_archive(path, data, crates, expect_no_bitcode=strip) and ok
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main(sys.argv[1:])
