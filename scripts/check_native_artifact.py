#!/usr/bin/env python3
"""Measure a shipped native library, check static archives for LLVM bitcode,
and validate Mach-O dynamic framework binaries (RFC-0043 §3.1, §5.2).

    check_native_artifact.py [--strip-bitcode] [--expect-macho-dylib]
        [--expect-arch ARCH] [--expect-install-name NAME]
        [--expect-platform PLATFORM] [--expect-min-os VER]
        [--expect-header-exports] [--max-bytes N] <file>...

Every file gets one line, `size <path> <bytes> bytes`: the raw_bytes
measurement of RFC-0043 §3.1, so the CI log of each artifact job carries the
number the RFC's tables are filled from.

`--strip-bitcode` first removes the bitcode sections from every member of
each archive, in place, with the `llvm-objcopy` of rustup's `llvm-tools`
component (`AIMUX_OBJCOPY` names another binary). That is
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

When `--expect-macho-dylib` (or any Mach-O `--expect-*` option) is passed,
each file must be a Mach-O `MH_DYLIB` carrying zero LLVM bitcode, matching
the requested architecture, `LC_ID_DYLIB` install name, platform/min-OS,
size ceiling, and exported C ABI symbols from `aimux-ffi/include/aimux.h`.

The archive, ELF, Mach-O and COFF headers are read directly, so the check
is the same on the Linux, macOS and Windows runners and needs no binutils
(except `nm -gU` when `--expect-header-exports` is requested).
"""

import argparse
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
FAT_MAGICS = {
    b"\xca\xfe\xba\xbe": (">", False),
    b"\xbe\xba\xfe\xca": ("<", False),
    b"\xca\xfe\xba\xbf": (">", True),
    b"\xbf\xba\xfe\xca": ("<", True),
}
MACHO_FILETYPES = {
    1: "MH_OBJECT",
    2: "MH_EXECUTE",
    6: "MH_DYLIB",
    8: "MH_BUNDLE",
}
MACHO_ARCHS = {
    7: "i386",
    12: "arm",
    0x01000007: "x86_64",
    0x0100000C: "arm64",
}
MACHO_PLATFORMS = {
    1: "macos",
    2: "ios",
    3: "tvos",
    4: "watchos",
    7: "ios-simulator",
    8: "tvos-simulator",
    9: "watchos-simulator",
}
LC_VERSION_MIN_PLATFORMS = {0x24: 1, 0x25: 2, 0x2F: 3, 0x30: 4}
LC_VERSION_MIN_SIM_PLATFORMS = {0x25: 7, 0x2F: 8, 0x30: 9}
X86_CPU_TYPES = {7, 0x01000007}


def header_exports(root):
    """`aimux_*` functions declared in aimux.h, comments removed first so a
    name mentioned in a `/* ... */` or `//` comment is not taken for one."""
    text = (root / "aimux-ffi" / "include" / "aimux.h").read_text(encoding="utf-8")
    text = re.sub(r"/\*.*?\*/", " ", text, flags=re.S)
    text = re.sub(r"//[^\n]*", " ", text)
    return set(re.findall(r"(?<![\w(])(aimux_\w+)\s*\(", text))


def format_macho_version(ver):
    major, minor, patch = (ver >> 16) & 0xFFFF, (ver >> 8) & 0xFF, ver & 0xFF
    return f"{major}.{minor}.{patch}" if patch else f"{major}.{minor}"


def normalize_version(ver):
    parts = [int(p) for p in ver.strip().split(".")]
    while len(parts) > 2 and parts[-1] == 0:
        parts.pop()
    if len(parts) == 1:
        parts.append(0)
    return ".".join(str(p) for p in parts)


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
            if name.startswith(b"__.SYMDEF"):
                continue
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


def find_llvm_tool(name):
    """The LLVM tool `name` (objcopy, strip, ar, nm) to use: `AIMUX_<NAME>`
    (for example `AIMUX_OBJCOPY`), else the `llvm-<name>` of rustup's
    `llvm-tools` component next to the active rustc, else `llvm-<name>` on
    PATH."""
    named = os.environ.get(f"AIMUX_{name.upper()}")
    if named:
        return named
    try:
        sysroot = subprocess.run(
            ["rustc", "--print", "sysroot"], check=True, capture_output=True, text=True
        ).stdout.strip()
        verbose = subprocess.run(["rustc", "-vV"], check=True, capture_output=True, text=True).stdout
        host = re.search(r"^host: (\S+)", verbose, re.M).group(1)
        exe = ".exe" if sys.platform == "win32" else ""
        bindir = Path(sysroot) / "lib" / "rustlib" / host / "bin"
        for candidate in (bindir / f"llvm-{name}{exe}", bindir / f"rust-{name}{exe}"):
            # The rust-objcopy stub of a toolchain without llvm-tools exists
            # but cannot load libLLVM; a --version run tells the two apart.
            if (
                candidate.is_file()
                and subprocess.run([str(candidate), "--version"], capture_output=True).returncode == 0
            ):
                return str(candidate)
    except (OSError, subprocess.CalledProcessError, AttributeError):
        pass
    on_path = shutil.which(f"llvm-{name}")
    if on_path:
        return on_path
    sys.exit(
        f"no llvm-{name}: install rustup's llvm-tools component "
        f"(`rustup component add llvm-tools`) or set AIMUX_{name.upper()}"
    )


def find_objcopy():
    return find_llvm_tool("objcopy")


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


def macho_info(body):
    """Parse Mach-O header, install name, build platforms and section sizes.

    Returns None when `body` is not a single-arch Mach-O binary.
    """
    if len(body) < 28 or body[:4] not in MACHO_MAGICS:
        return None
    end, bits = MACHO_MAGICS[body[:4]]
    hdr_size = 32 if bits == 64 else 28
    if len(body) < hdr_size:
        return None
    cputype, _cpusubtype, filetype, ncmds, sizeofcmds = struct.unpack_from(end + "IIIII", body, 4)
    limit = min(len(body), hdr_size + sizeofcmds)
    offset = hdr_size
    install_name = None
    platforms = []
    text_bytes = 0
    bitcode_bytes = 0
    for _ in range(ncmds):
        if offset + 8 > limit:
            break
        cmd, cmdsize = struct.unpack_from(end + "II", body, offset)
        if cmdsize < 8 or offset + cmdsize > limit:
            break
        if cmd in (0x19, 0x1):  # LC_SEGMENT_64, LC_SEGMENT
            if cmd == 0x19 and cmdsize >= 72:
                nsects = struct.unpack_from(end + "I", body, offset + 64)[0]
                sect, step, size_fmt, size_off = offset + 72, 80, "Q", 40
            elif cmd == 0x1 and cmdsize >= 56:
                nsects = struct.unpack_from(end + "I", body, offset + 48)[0]
                sect, step, size_fmt, size_off = offset + 56, 68, "I", 36
            else:
                nsects = 0
                sect = step = size_off = 0
                size_fmt = "I"
            for _ in range(nsects):
                if sect + step > offset + cmdsize:
                    break
                sectname = body[sect : sect + 16].rstrip(b"\0").decode("utf-8", "replace")
                segname = body[sect + 16 : sect + 32].rstrip(b"\0").decode("utf-8", "replace")
                size = struct.unpack_from(end + size_fmt, body, sect + size_off)[0]
                if (segname, sectname) in MACHO_BITCODE:
                    bitcode_bytes += size
                elif segname == "__TEXT" and sectname == "__text":
                    text_bytes += size
                sect += step
        elif cmd == 0x0D and cmdsize >= 24:  # LC_ID_DYLIB
            name_off = struct.unpack_from(end + "I", body, offset + 8)[0]
            if 24 <= name_off < cmdsize:
                raw = body[offset + name_off : offset + cmdsize]
                install_name = raw.split(b"\0", 1)[0].decode("utf-8", "replace")
        elif cmd == 0x32 and cmdsize >= 24:  # LC_BUILD_VERSION
            plat, minos, sdk = struct.unpack_from(end + "III", body, offset + 8)
            platforms.append(
                (
                    MACHO_PLATFORMS.get(plat, f"platform-{plat}"),
                    format_macho_version(minos),
                    format_macho_version(sdk),
                )
            )
        elif cmd in LC_VERSION_MIN_PLATFORMS and cmdsize >= 16:
            plat = (
                LC_VERSION_MIN_SIM_PLATFORMS.get(cmd, LC_VERSION_MIN_PLATFORMS[cmd])
                if cputype in X86_CPU_TYPES
                else LC_VERSION_MIN_PLATFORMS[cmd]
            )
            minos, sdk = struct.unpack_from(end + "II", body, offset + 8)
            platforms.append(
                (
                    MACHO_PLATFORMS.get(plat, f"platform-{plat}"),
                    format_macho_version(minos),
                    format_macho_version(sdk),
                )
            )
        offset += cmdsize
    return {
        "cputype": cputype,
        "arch": MACHO_ARCHS.get(cputype, f"0x{cputype:x}"),
        "filetype": filetype,
        "filetype_name": MACHO_FILETYPES.get(filetype, f"0x{filetype:x}"),
        "install_name": install_name,
        "platforms": platforms,
        "text_bytes": text_bytes,
        "bitcode_bytes": bitcode_bytes,
    }


def macho_slices(body):
    """Return a list of parsed `macho_info` dicts for a single-arch or universal
    (fat) Mach-O binary, or None if `body` is not a Mach-O binary."""
    if len(body) < 8:
        return None
    if body[:4] in MACHO_MAGICS:
        info = macho_info(body)
        return [info] if info is not None else None
    if body[:4] in FAT_MAGICS:
        end, is64 = FAT_MAGICS[body[:4]]
        nfat = struct.unpack_from(end + "I", body, 4)[0]
        entry_size = 32 if is64 else 20
        if nfat == 0 or 8 + nfat * entry_size > len(body):
            return None
        slices = []
        for i in range(nfat):
            off = 8 + i * entry_size
            if is64:
                _, _, slice_off, slice_size, _ = struct.unpack_from(end + "IIQQI", body, off)
            else:
                _, _, slice_off, slice_size, _ = struct.unpack_from(end + "IIIII", body, off)
            if slice_off + slice_size > len(body):
                return None
            info = macho_info(body[slice_off : slice_off + slice_size])
            if info is None:
                return None
            slices.append(info)
        return slices
    return None


def defined_exports_macho(path, arch=None):
    """Return defined global `aimux_*` symbol names (without leading `_`) in `path`."""
    nm = os.environ.get("AIMUX_NM") or shutil.which("nm") or find_llvm_tool("nm")
    cmd = [nm]
    if arch:
        cmd += ["-arch", arch]
    cmd += ["-gU", str(path)]
    out = subprocess.run(cmd, check=True, capture_output=True, text=True).stdout
    names = set()
    for line in out.splitlines():
        parts = line.split()
        if parts and re.fullmatch(r"_?aimux_\w+", parts[-1]):
            sym = parts[-1]
            names.add(sym[1:] if sym.startswith("_aimux_") else sym)
    return names


def check_macho_dylib(
    path,
    data,
    root,
    *,
    expect_arch=None,
    expect_install_name=None,
    expect_platform=None,
    expect_min_os=None,
    expect_header_exports=False,
):
    slices = macho_slices(data)
    if not slices:
        print(f"{path}: expected a Mach-O MH_DYLIB, but file is not a Mach-O binary", file=sys.stderr)
        return False
    ok = True
    actual_archs = {info["arch"] for info in slices}
    if expect_arch:
        wanted_archs = {a.strip() for a in expect_arch.split(",") if a.strip()}
        if actual_archs != wanted_archs:
            print(
                f"{path}: expected arch {','.join(sorted(wanted_archs))}, got {','.join(sorted(actual_archs))}",
                file=sys.stderr,
            )
            ok = False
    for info in slices:
        plat_summary = ", ".join(f"{p} {m}" for p, m, _ in info["platforms"]) or "none"
        print(
            f"  {info['filetype_name']} {info['arch']}, install_name {info['install_name']}, "
            f"platforms {plat_summary}, .text {info['text_bytes']} bytes, bitcode {info['bitcode_bytes']} bytes"
        )
        if info["filetype"] != 6:
            print(
                f"{path} ({info['arch']}): expected Mach-O MH_DYLIB (6), got {info['filetype_name']} ({info['filetype']})",
                file=sys.stderr,
            )
            ok = False
        if info["bitcode_bytes"] != 0:
            print(
                f"{path} ({info['arch']}): carries {info['bitcode_bytes']} bytes of LLVM bitcode (expected 0)",
                file=sys.stderr,
            )
            ok = False
        if expect_install_name and info["install_name"] != expect_install_name:
            print(
                f"{path} ({info['arch']}): expected install_name {expect_install_name!r}, got {info['install_name']!r}",
                file=sys.stderr,
            )
            ok = False
        if expect_platform:
            matching = [m for p, m, _ in info["platforms"] if p == expect_platform]
            if not matching:
                print(
                    f"{path} ({info['arch']}): expected platform {expect_platform!r}, got {[p for p, _, _ in info['platforms']]}",
                    file=sys.stderr,
                )
                ok = False
            elif expect_min_os and normalize_version(matching[0]) != normalize_version(expect_min_os):
                print(
                    f"{path} ({info['arch']}): expected min-os {expect_min_os!r} for {expect_platform}, got {matching[0]!r}",
                    file=sys.stderr,
                )
                ok = False
        elif expect_min_os:
            if not any(normalize_version(m) == normalize_version(expect_min_os) for _, m, _ in info["platforms"]):
                print(
                    f"{path} ({info['arch']}): expected min-os {expect_min_os!r}, got {[m for _, m, _ in info['platforms']]}",
                    file=sys.stderr,
                )
                ok = False
    if expect_header_exports:
        declared = header_exports(root)
        check_archs = [info["arch"] for info in slices] if len(slices) > 1 else [None]
        for arch in check_archs:
            defined = defined_exports_macho(path, arch=arch)
            missing = sorted(declared - defined)
            if missing:
                label = f" ({arch})" if arch else ""
                print(f"{path}{label}: missing exported C ABI symbols: {', '.join(missing)}", file=sys.stderr)
                ok = False
        if ok:
            print(f"  {len(declared)} required C ABI exports present")
    return ok


def parse_args(argv):
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument("--strip-bitcode", action="store_true")
    parser.add_argument("--expect-macho-dylib", action="store_true")
    parser.add_argument("--expect-arch")
    parser.add_argument("--expect-install-name")
    parser.add_argument("--expect-platform")
    parser.add_argument("--expect-min-os")
    parser.add_argument("--expect-header-exports", action="store_true")
    parser.add_argument("--max-bytes", type=int)
    parser.add_argument("-h", "--help", action="store_true")
    parser.add_argument("files", nargs="*")
    args = parser.parse_args(argv)
    if args.help or not args.files:
        sys.exit(__doc__)
    return args


def main(argv):
    args = parse_args(argv)
    root = Path(__file__).resolve().parent.parent
    crates = workspace_crates(root)
    check_dylib = bool(
        args.expect_macho_dylib
        or args.expect_arch
        or args.expect_install_name
        or args.expect_platform
        or args.expect_min_os
        or args.expect_header_exports
    )
    ok = True
    for arg in args.files:
        path = Path(arg)
        data = path.read_bytes()
        is_archive = data.startswith(b"!<arch>\n")
        if args.strip_bitcode and is_archive:
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
        if args.max_bytes is not None and len(data) > args.max_bytes:
            print(f"{path}: {len(data)} bytes exceeds max {args.max_bytes} bytes", file=sys.stderr)
            ok = False
        if is_archive:
            ok = check_archive(path, data, crates, expect_no_bitcode=args.strip_bitcode) and ok
        if check_dylib:
            ok = (
                check_macho_dylib(
                    path,
                    data,
                    root,
                    expect_arch=args.expect_arch,
                    expect_install_name=args.expect_install_name,
                    expect_platform=args.expect_platform,
                    expect_min_os=args.expect_min_os,
                    expect_header_exports=args.expect_header_exports,
                )
                and ok
            )
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main(sys.argv[1:])
