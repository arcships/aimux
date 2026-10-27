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
import struct
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import check_native_artifact as check  # noqa: E402

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


if __name__ == "__main__":
    unittest.main(verbosity=2)
