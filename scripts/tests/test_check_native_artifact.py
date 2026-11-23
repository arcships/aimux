#!/usr/bin/env python3
"""Self-test of scripts/check_native_artifact.py on synthetic archives.

    python3 scripts/tests/test_check_native_artifact.py

The fixtures are built here, byte by byte: a GNU `ar` archive whose members
are minimal ELF64 relocatable objects with a `.text` section and, where the
case wants it, a `.llvmbc` section. No compiler, linker or binutils is
needed, so the test runs the same on every CI runner.

The negative case is the one that matters (eric's review of #377): a
workspace member (`aimux_core-*.o`) that carries bitcode is a fat-LTO
archive, and `--strip-bitcode` must fail it *before* any strip, never strip
the bitcode away and then pass the archive.
"""

import contextlib
import io
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import check_native_artifact as check  # noqa: E402
import merge_archive  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]


def elf_object(sections):
    """A minimal ELF64 little-endian relocatable object with the given
    (name, bytes) sections, plus the null section and `.shstrtab`."""
    names = [b""] + [n.encode() for n, _ in sections] + [b".shstrtab"]
    shstrtab = b"\0".join(names) + b"\0"
    name_off = {}
    off = 0
    for n in names:
        name_off[n] = off
        off += len(n) + 1
    blobs = [b for _, b in sections] + [shstrtab]
    ehsize, shentsize = 64, 64
    data_off = ehsize
    offsets = []
    body = b""
    for b in blobs:
        offsets.append(data_off + len(body))
        body += b + (b"\0" * (-len(b) % 8))
    shoff = data_off + len(body)
    shnum = len(blobs) + 1
    headers = struct.pack("<IIQQQQIIQQ", 0, 0, 0, 0, 0, 0, 0, 0, 0, 0)  # null
    for i, (n, b) in enumerate(zip(names[1:], blobs)):
        sh_type = 3 if n == b".shstrtab" else 1  # STRTAB / PROGBITS
        headers += struct.pack("<IIQQQQIIQQ", name_off[n], sh_type, 0, 0, offsets[i], len(b), 0, 0, 1, 0)
    ident = b"\x7fELF" + bytes([2, 1, 1, 0]) + b"\0" * 8
    header = ident + struct.pack(
        "<HHIQQQIHHHHHH", 1, 0x3E, 1, 0, 0, shoff, 0, ehsize, 0, 0, shentsize, shnum, shnum - 1
    )
    assert len(header) == ehsize
    return header + body + headers


def gnu_archive(members):
    """A GNU `ar` archive of (short name, bytes) members, no symbol index."""
    out = b"!<arch>\n"
    for name, body in members:
        assert len(name) <= 15, name
        out += f"{name + '/':<16}{0:<12}{0:<6}{0:<6}{0o644:<8o}{len(body):<10}`\n".encode()
        out += body + (b"\n" if len(body) % 2 else b"")
    return out


def bsd_archive(members):
    """A BSD `ar` archive using `#1/<len>` extended names (including `__.SYMDEF SORTED`)."""
    out = b"!<arch>\n"
    for name, body in members:
        raw_name = name.encode("utf-8") + b"\0"
        pad = (-len(raw_name)) % 4
        name_field = raw_name + b"\0" * pad
        payload = name_field + body
        tag = f"#1/{len(name_field)}"
        out += f"{tag:<16}{0:<12}{0:<6}{0:<6}{0o644:<8o}{len(payload):<10}`\n".encode()
        out += payload + (b"\n" if len(payload) % 2 else b"")
    return out


def macho64_object(sections):
    """A minimal 64-bit little-endian Mach-O MH_OBJECT with one LC_SEGMENT_64."""
    cmdsize = 72 + 80 * len(sections)
    header = struct.pack(
        "<IIIIIIII",
        0xFEEDFACF,  # MH_MAGIC_64
        0x0100000C,  # CPU_TYPE_ARM64
        0,           # CPU_SUBTYPE_ARM64_ALL
        1,           # MH_OBJECT
        1,           # ncmds
        cmdsize,     # sizeofcmds
        0,           # flags
        0,           # reserved
    )
    data_offset = len(header) + cmdsize
    blobs = []
    sect_headers = b""
    for seg, sect, blob in sections:
        sect_headers += struct.pack(
            "<16s16sQQIIIIIIII",
            sect.encode("ascii"),
            seg.encode("ascii"),
            0,           # addr
            len(blob),   # size
            data_offset + sum(len(b) for b in blobs),
            0,           # align
            0,           # reloff
            0,           # nreloc
            0,           # flags
            0, 0, 0,
        )
        blobs.append(blob)
    seg_cmd = struct.pack(
        "<II16sQQQQIIII",
        0x19,        # LC_SEGMENT_64
        cmdsize,
        b"",         # unnamed segment for MH_OBJECT
        0,           # vmaddr
        sum(len(b) for b in blobs),
        data_offset,
        sum(len(b) for b in blobs),
        7, 7,        # maxprot, initprot
        len(sections),
        0,
    )
    return header + seg_cmd + sect_headers + b"".join(blobs)


def macho64_dylib(
    sections,
    *,
    cputype=0x0100000C,
    filetype=6,
    install_name="@rpath/aimux_ffi.framework/aimux_ffi",
    platform=2,
    minos=(12 << 16),
    sdk=(18 << 16),
):
    """A minimal 64-bit little-endian Mach-O MH_DYLIB with LC_SEGMENT_64,
    LC_ID_DYLIB, and LC_BUILD_VERSION."""
    cmds = []
    seg_cmdsize = 72 + 80 * len(sections)

    if install_name is not None:
        raw_name = install_name.encode("utf-8") + b"\0"
        pad = (-len(raw_name)) % 8
        id_cmdsize = 24 + len(raw_name) + pad
        cmds.append(
            struct.pack("<IIIIII", 0x0D, id_cmdsize, 24, 0, 0x00010000, 0x00010000)
            + raw_name
            + b"\0" * pad
        )
    if platform is not None:
        cmds.append(struct.pack("<IIIIII", 0x32, 24, platform, minos, sdk, 0))

    sizeofcmds = seg_cmdsize + sum(len(c) for c in cmds)
    header = struct.pack(
        "<IIIIIIII",
        0xFEEDFACF,  # MH_MAGIC_64
        cputype,
        0,
        filetype,
        1 + len(cmds),
        sizeofcmds,
        0,
        0,
    )
    data_offset = len(header) + sizeofcmds
    blobs = []
    sect_headers = b""
    for seg, sect, blob in sections:
        sect_headers += struct.pack(
            "<16s16sQQIIIIIIII",
            sect.encode("ascii"),
            seg.encode("ascii"),
            0,
            len(blob),
            data_offset + sum(len(b) for b in blobs),
            0,
            0,
            0,
            0,
            0,
            0,
            0,
        )
        blobs.append(blob)
    seg_cmd = struct.pack(
        "<II16sQQQQIIII",
        0x19,
        seg_cmdsize,
        b"__TEXT",
        0,
        sum(len(b) for b in blobs),
        data_offset,
        sum(len(b) for b in blobs),
        5,
        5,
        len(sections),
        0,
    )
    return header + seg_cmd + sect_headers + b"".join(cmds) + b"".join(blobs)


TEXT = b"\x90" * 64
BITCODE = b"BC\xc0\xde" + b"\x01" * 60


def run_main(argv):
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        try:
            check.main(argv)
        except SystemExit as e:
            code = e.code
        else:
            code = 0
    return code, out.getvalue(), err.getvalue()


class CheckNativeArtifact(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.cwd = os.getcwd()

    def tearDown(self):
        self.tmp.cleanup()

    def write(self, name, members):
        path = Path(self.tmp.name) / name
        path.write_bytes(gnu_archive(members))
        return path

    def test_fixture_parses(self):
        obj = elf_object([(".text", TEXT), (".llvmbc", BITCODE)])
        self.assertEqual(dict(check.elf_sections(obj)), {"": 0, ".text": 64, ".llvmbc": 64, ".shstrtab": 25})
        self.assertEqual(check.member_report(obj), (64, 64))
        archive = gnu_archive([("a.o", obj), ("b.o", TEXT)])
        self.assertEqual([n for n, _ in check.archive_members(archive)], ["a.o", "b.o"])
        self.assertIn("aimux_core", check.workspace_crates(ROOT))

    def test_fat_lto_workspace_member_fails_without_strip(self):
        path = self.write(
            "fat.a",
            [("aimux_core-1.o", elf_object([(".text", TEXT), (".llvmbc", BITCODE)])), ("std-2.o", elf_object([(".text", TEXT)]))],
        )
        code, out, err = run_main([str(path)])
        self.assertEqual(code, 1)
        self.assertIn("workspace objects carry LLVM bitcode", err)

    def test_fat_lto_workspace_member_fails_before_any_strip(self):
        # If the strip ran first it would remove the bitcode and the guard
        # could never fire. The archive must be judged as built, and the
        # objcopy must not even be looked up (AIMUX_OBJCOPY points nowhere).
        path = self.write(
            "fat.a",
            [("aimux_core-1.o", elf_object([(".text", TEXT), (".llvmbc", BITCODE)])), ("std-2.o", elf_object([(".text", TEXT), (".llvmbc", BITCODE)]))],
        )
        before = path.read_bytes()
        os.environ["AIMUX_OBJCOPY"] = str(Path(self.tmp.name) / "no-such-objcopy")
        try:
            code, out, err = run_main(["--strip-bitcode", str(path)])
        finally:
            del os.environ["AIMUX_OBJCOPY"]
        self.assertEqual(code, 1)
        self.assertIn("workspace objects carry LLVM bitcode", err)
        self.assertNotIn("before bitcode strip", out)
        self.assertEqual(path.read_bytes(), before, "a failed archive is left as it is")

    def test_clean_archive_passes_and_reports_sysroot_bitcode(self):
        path = self.write(
            "ok.a",
            [("aimux_ffi-1.o", elf_object([(".text", TEXT)])), ("std-2.o", elf_object([(".text", TEXT), (".llvmbc", BITCODE)]))],
        )
        code, out, err = run_main([str(path)])
        self.assertEqual(code, 0, err)
        self.assertIn("bitcode 64 bytes in 1 objects (0 of them workspace crates)", out)
        self.assertIn(f"size {path} ", out)

    def test_no_workspace_member_fails(self):
        path = self.write("sysroot.a", [("std-2.o", elf_object([(".text", TEXT)]))])
        code, out, err = run_main([str(path)])
        self.assertEqual(code, 1)
        self.assertIn("no workspace crate among the archive members", err)

    def test_strip_removes_sysroot_bitcode(self):
        try:
            with contextlib.redirect_stderr(io.StringIO()):
                check.find_objcopy()
        except SystemExit:
            self.skipTest("rust-objcopy / llvm-objcopy not available (rustup component add llvm-tools)")
        path = self.write(
            "ok.a",
            [("aimux_ffi-1.o", elf_object([(".text", TEXT)])), ("std-2.o", elf_object([(".text", TEXT), (".llvmbc", BITCODE)]))],
        )
        code, out, err = run_main(["--strip-bitcode", str(path)])
        self.assertEqual(code, 0, err)
        self.assertIn("before bitcode strip", out)
        self.assertIn("bitcode 0 bytes in 0 objects", out)
        reports = [check.member_report(b) for _, b in check.archive_members(path.read_bytes())]
        self.assertEqual([r[1] for r in reports if r], [0, 0])
        # Running it again is a no-op that still passes.
        code, out, err = run_main(["--strip-bitcode", str(path)])
        self.assertEqual(code, 0, err)

    def test_bsd_symdef_sorted_is_skipped_and_macho_sections_parse(self):
        obj = macho64_object([("__TEXT", "__text", TEXT), ("__LLVM", "__bitcode", BITCODE)])
        self.assertEqual(
            dict(check.macho_sections(obj)),
            {("__TEXT", "__text"): 64, ("__LLVM", "__bitcode"): 64},
        )
        self.assertEqual(check.member_report(obj), (64, 64))
        archive = bsd_archive([("__.SYMDEF SORTED", b"\x00" * 16), ("aimux_ffi-1.o", obj)])
        self.assertEqual([n for n, _ in check.archive_members(archive)], ["aimux_ffi-1.o"])

    def test_macho_fat_lto_workspace_member_fails_before_any_strip(self):
        path = Path(self.tmp.name) / "fat_macho.a"
        path.write_bytes(
            bsd_archive(
                [
                    ("__.SYMDEF SORTED", b"\x00" * 16),
                    ("aimux_core-1.o", macho64_object([("__TEXT", "__text", TEXT), ("__LLVM", "__bitcode", BITCODE)])),
                    ("std-2.o", macho64_object([("__TEXT", "__text", TEXT)])),
                ]
            )
        )
        before = path.read_bytes()
        os.environ["AIMUX_OBJCOPY"] = str(Path(self.tmp.name) / "no-such-objcopy")
        try:
            code, out, err = run_main(["--strip-bitcode", str(path)])
        finally:
            del os.environ["AIMUX_OBJCOPY"]
        self.assertEqual(code, 1)
        self.assertIn("workspace objects carry LLVM bitcode", err)
        self.assertNotIn("before bitcode strip", out)
        self.assertEqual(path.read_bytes(), before)

    def test_macho_platform_flags_and_legacy_sim(self):
        def macho_cmd(cputype, cmd_bytes, sizeofcmds=None):
            sz = len(cmd_bytes) if sizeofcmds is None else sizeofcmds
            return struct.pack("<IIIIIIII", 0xFEEDFACF, cputype, 0, 1, 1, sz, 0, 0) + cmd_bytes

        arm64, x86_64 = 0x0100000C, 0x01000007
        bv11 = macho_cmd(arm64, struct.pack("<IIIIII", 0x32, 24, 1, 11 << 16, 15 << 16, 0))
        bv14 = macho_cmd(arm64, struct.pack("<IIIIII", 0x32, 24, 1, 14 << 16, (15 << 16) | (4 << 8), 0))
        self.assertEqual(
            merge_archive.macho_platform_flags([bv11, bv14]),
            ["-platform_version", "macos", "14.0", "15.4"],
        )

        # Legacy LC_VERSION_MIN_IPHONEOS (0x25) maps to ios on arm64 and ios-simulator on x86_64.
        ios_dev = macho_cmd(arm64, struct.pack("<IIII", 0x25, 16, 12 << 16, 16 << 16))
        ios_sim = macho_cmd(x86_64, struct.pack("<IIII", 0x25, 16, 12 << 16, 16 << 16))
        self.assertEqual(merge_archive.macho_platform_flags([ios_dev]), ["-platform_version", "ios", "12.0", "16.0"])
        self.assertEqual(
            merge_archive.macho_platform_flags([ios_sim]),
            ["-platform_version", "ios-simulator", "12.0", "16.0"],
        )

        # Truncated load command must not crash or read past sizeofcmds.
        truncated = macho_cmd(arm64, struct.pack("<III", 0x32, 24, 1), sizeofcmds=12)
        self.assertEqual(merge_archive.macho_platform_flags([truncated]), [])

    def test_macho_already_merged_with_bitcode_is_rejected(self):
        path = Path(self.tmp.name) / "poisoned_merged.a"
        obj = macho64_object([("__TEXT", "__text", TEXT), ("__LLVM", "__bitcode", BITCODE)])
        path.write_bytes(bsd_archive([("__.SYMDEF SORTED", b"\x00" * 16), (merge_archive.MERGED_MEMBER, obj)]))
        with self.assertRaises(SystemExit) as cm:
            with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                merge_archive.main([str(path)])
        self.assertEqual(cm.exception.code, 1)

    @unittest.skipUnless(sys.platform == "darwin" and shutil.which("clang"), "requires Apple toolchain")
    def test_macho_merge_archive_end_to_end(self):
        tmp = Path(self.tmp.name)
        exports = sorted(merge_archive.header_exports(ROOT))
        c_exports = tmp / "aimux_ffi_stub.c"
        c_helper = tmp / "std_stub.c"
        c_main = tmp / "main.c"
        o_exports = tmp / "aimux_ffi-stub.o"
        o_helper = tmp / "std-stub.o"
        archive = tmp / "libaimux_ffi_test.a"

        lines = ["int internal_helper(void);"]
        for name in exports:
            lines.append(f"int {name}(void) {{ return internal_helper(); }}")
        c_exports.write_text("\n".join(lines) + "\n", encoding="utf-8")
        c_helper.write_text(
            "int internal_helper(void) { return 65536; }\n"
            "void rust_eh_personality(void) {}\n",
            encoding="utf-8",
        )
        c_main.write_text(
            "int aimux_abi_version(void);\n"
            "int main(void) { return aimux_abi_version() == 65536 ? 0 : 1; }\n",
            encoding="utf-8",
        )

        subprocess.run(["clang", "-c", str(c_exports), "-o", str(o_exports)], check=True)
        subprocess.run(["clang", "-c", str(c_helper), "-o", str(o_helper)], check=True)
        subprocess.run(["ar", "rcs", str(archive), str(o_exports), str(o_helper)], check=True)

        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            merge_archive.main([str(archive)])
        self.assertIn(f"size {archive}", buf.getvalue())

        members = [n for n, b in check.archive_members(archive.read_bytes()) if check.member_report(b) is not None]
        self.assertEqual(members, [merge_archive.MERGED_MEMBER])

        # Internal helper must be localized/stripped; C ABI exports and _rust_eh_personality remain global.
        nm_global = subprocess.run(["nm", "-gU", str(archive)], check=True, capture_output=True, text=True).stdout
        global_syms = {line.split()[-1] for line in nm_global.splitlines() if line.split()}
        for name in exports:
            self.assertIn(f"_{name}", global_syms)
        self.assertIn("_rust_eh_personality", global_syms)
        self.assertNotIn("_internal_helper", global_syms)

        # Idempotent on an already-merged archive.
        buf2 = io.StringIO()
        with contextlib.redirect_stdout(buf2):
            merge_archive.main([str(archive)])
        self.assertIn("already merged", buf2.getvalue())

        # Both normal static linking and CocoaPods-style -force_load must succeed.
        for label, link_flags in [("normal", [str(archive)]), ("force_load", [f"-Wl,-force_load,{archive}"])]:
            exe = tmp / f"test_{label}"
            subprocess.run(["clang", str(c_main), *link_flags, "-o", str(exe)], check=True)
            subprocess.run([str(exe)], check=True)

    def test_macho_dylib_valid_ios_and_simulator(self):
        for name, plat_id, plat_name in [("ios.dylib", 2, "ios"), ("sim.dylib", 7, "ios-simulator")]:
            path = Path(self.tmp.name) / name
            path.write_bytes(macho64_dylib([("__TEXT", "__text", TEXT)], platform=plat_id))
            code, out, err = run_main(
                [
                    "--expect-macho-dylib",
                    "--expect-arch",
                    "arm64",
                    "--expect-install-name",
                    "@rpath/aimux_ffi.framework/aimux_ffi",
                    "--expect-platform",
                    plat_name,
                    "--expect-min-os",
                    "12.0",
                    "--max-bytes",
                    "4096",
                    str(path),
                ]
            )
            self.assertEqual(code, 0, err)
            self.assertIn(f"MH_DYLIB arm64, install_name @rpath/aimux_ffi.framework/aimux_ffi, platforms {plat_name} 12.0", out)

    def test_macho_dylib_rejects_archive_object_bitcode_and_mismatches(self):
        # Static archive must fail --expect-macho-dylib.
        arch_path = self.write("static.a", [("aimux_ffi-1.o", elf_object([(".text", TEXT)]))])
        code, _, err = run_main(["--expect-macho-dylib", str(arch_path)])
        self.assertEqual(code, 1)
        self.assertIn("not a single-arch Mach-O binary", err)

        # MH_OBJECT must fail --expect-macho-dylib.
        obj_path = Path(self.tmp.name) / "obj.o"
        obj_path.write_bytes(macho64_object([("__TEXT", "__text", TEXT)]))
        code, _, err = run_main(["--expect-macho-dylib", str(obj_path)])
        self.assertEqual(code, 1)
        self.assertIn("expected Mach-O MH_DYLIB (6), got MH_OBJECT (1)", err)

        # Bitcode inside MH_DYLIB must fail.
        bc_path = Path(self.tmp.name) / "bitcode.dylib"
        bc_path.write_bytes(macho64_dylib([("__TEXT", "__text", TEXT), ("__LLVM", "__bitcode", BITCODE)]))
        code, _, err = run_main(["--expect-macho-dylib", str(bc_path)])
        self.assertEqual(code, 1)
        self.assertIn("carries 64 bytes of LLVM bitcode", err)

        # Wrong install_name, arch, platform, min-os, and max-bytes must each fail.
        bad = Path(self.tmp.name) / "bad.dylib"
        bad.write_bytes(
            macho64_dylib(
                [("__TEXT", "__text", TEXT)],
                cputype=0x01000007,  # x86_64
                install_name="/usr/local/lib/libaimux_ffi.dylib",
                platform=1,  # macos
                minos=(14 << 16),
            )
        )
        code, _, err = run_main(
            [
                "--expect-macho-dylib",
                "--expect-arch",
                "arm64",
                "--expect-install-name",
                "@rpath/aimux_ffi.framework/aimux_ffi",
                "--expect-platform",
                "ios",
                "--expect-min-os",
                "12.0",
                "--max-bytes",
                "10",
                str(bad),
            ]
        )
        self.assertEqual(code, 1)
        self.assertIn("exceeds max 10 bytes", err)
        self.assertIn("expected arch arm64, got x86_64", err)
        self.assertIn("expected install_name", err)
        self.assertIn("expected platform 'ios'", err)

    @unittest.skipUnless(sys.platform == "darwin" and shutil.which("clang"), "requires Apple toolchain")
    def test_macho_dylib_exports_end_to_end(self):
        tmp = Path(self.tmp.name)
        exports = sorted(check.header_exports(ROOT))
        self.assertEqual(len(exports), 14)

        full_c = tmp / "full.c"
        full_c.write_text("\n".join(f"int {name}(void) {{ return 0; }}" for name in exports) + "\n", encoding="utf-8")
        full_dylib = tmp / "aimux_ffi"
        subprocess.run(
            [
                "clang",
                "-dynamiclib",
                "-Wl,-install_name,@rpath/aimux_ffi.framework/aimux_ffi",
                str(full_c),
                "-o",
                str(full_dylib),
            ],
            check=True,
        )
        code, out, err = run_main(
            [
                "--expect-macho-dylib",
                "--expect-install-name",
                "@rpath/aimux_ffi.framework/aimux_ffi",
                "--expect-header-exports",
                str(full_dylib),
            ]
        )
        self.assertEqual(code, 0, err)
        self.assertIn("14 required C ABI exports present", out)

        missing_c = tmp / "missing.c"
        missing_c.write_text(
            "\n".join(f"int {name}(void) {{ return 0; }}" for name in exports[1:]) + "\n",
            encoding="utf-8",
        )
        missing_dylib = tmp / "aimux_ffi_missing"
        subprocess.run(
            [
                "clang",
                "-dynamiclib",
                "-Wl,-install_name,@rpath/aimux_ffi.framework/aimux_ffi",
                str(missing_c),
                "-o",
                str(missing_dylib),
            ],
            check=True,
        )
        code, _, err = run_main(["--expect-macho-dylib", "--expect-header-exports", str(missing_dylib)])
        self.assertEqual(code, 1)
        self.assertIn(f"missing exported C ABI symbols: {exports[0]}", err)


if __name__ == "__main__":
    unittest.main(verbosity=2)
