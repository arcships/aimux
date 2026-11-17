#!/usr/bin/env python3
"""Unit tests for scripts/check_layout.py."""

import contextlib
import io
import pathlib
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent))
import check_layout  # noqa: E402


class CheckLayoutTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.tmp.name)
        self.src = self.root / "src"
        self.src.mkdir()

    def tearDown(self):
        self.tmp.cleanup()

    def _write_src(self, rel: str, content: str = "// stub\n"):
        path = self.src / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")

    def _write_layout(self, content: str) -> pathlib.Path:
        path = self.root / "layout.toml"
        path.write_text(content, encoding="utf-8")
        return path

    def test_valid_layout_with_tests_and_aimux_entries(self):
        self._write_src("demo/mod.rs")
        self._write_src("demo/provider.rs")
        self._write_src("demo/extra.rs")
        self._write_src("demo/tests/provider.rs")
        self._write_src("demo/sub/mod.rs")
        layout = self._write_layout(
            """
[demo]
package = "@ai-sdk/demo"

[[demo.files]]
upstream = "src/index.ts"
rust = "demo/mod.rs"
status = "mapped"

[demo.files.tests]
ts-only = [
  { case = "demo > skips js thing", reason = "js only" },
]

[[demo.files]]
upstream = "src/demo-provider.ts"
rust = "demo/provider.rs"
status = "mapped"

[demo.files.tests]

[[demo.files]]
upstream = "src/missing.ts"
status = "missing"
issue = 275

[[demo.files]]
rust = "demo/extra.rs"
status = "aimux"
reason = "aimux extension"
"""
        )
        errors = check_layout.check_layout_files(layout, self.src)
        self.assertEqual(errors, [])

    def test_subset_parser_matches_complex_toml_features(self):
        parsed = check_layout.parse_toml_subset(
            """
# Leading comment
[alpha]
package = "@ai-sdk/alpha"

[[alpha.files]]
upstream = "src/a.ts"
rust = "alpha/a.rs"
status = "mapped"

[alpha.files.tests]
sub-feature = [
  { case = "a > b \\"quoted\\"", issue = 275, reason = "not yet" },
]

[[alpha.files]]
upstream = "src/b.ts"
status = "missing"
issue = 276
"""
        )
        self.assertEqual(parsed["alpha"]["package"], "@ai-sdk/alpha")
        self.assertEqual(len(parsed["alpha"]["files"]), 2)
        self.assertEqual(
            parsed["alpha"]["files"][0]["tests"]["sub-feature"][0]["case"],
            'a > b "quoted"',
        )
        self.assertEqual(parsed["alpha"]["files"][1]["issue"], 276)

    def test_rejects_missing_rust_file_on_disk(self):
        self._write_src("demo/mod.rs")
        layout = self._write_layout(
            """
[demo]
package = "@ai-sdk/demo"

[[demo.files]]
upstream = "src/demo.ts"
rust = "demo/does_not_exist.rs"
status = "mapped"

[demo.files.tests]
"""
        )
        errors = check_layout.check_layout_files(layout, self.src)
        self.assertTrue(
            any("does not exist under src/: demo/does_not_exist.rs" in e for e in errors),
            errors,
        )

    def test_rejects_invalid_status(self):
        self._write_src("demo/provider.rs")
        layout = self._write_layout(
            """
[demo]
package = "@ai-sdk/demo"

[[demo.files]]
upstream = "src/demo.ts"
rust = "demo/provider.rs"
status = "bogus"
"""
        )
        errors = check_layout.check_layout_files(layout, self.src)
        self.assertTrue(any("invalid status 'bogus'" in e for e in errors), errors)

    def test_rejects_aimux_without_reason(self):
        self._write_src("demo/extra.rs")
        layout = self._write_layout(
            """
[demo]
package = "@ai-sdk/demo"

[[demo.files]]
rust = "demo/extra.rs"
status = "aimux"
"""
        )
        errors = check_layout.check_layout_files(layout, self.src)
        self.assertTrue(any('status = "aimux" requires `reason`' in e for e in errors), errors)

    def test_rejects_missing_without_issue(self):
        self._write_src("demo/mod.rs")
        layout = self._write_layout(
            """
[demo]
package = "@ai-sdk/demo"

[[demo.files]]
upstream = "src/todo.ts"
status = "missing"
"""
        )
        errors = check_layout.check_layout_files(layout, self.src)
        self.assertTrue(
            any('status = "missing" requires a positive integer `issue`' in e for e in errors),
            errors,
        )

    def test_rejects_unlisted_source_file_in_covered_vendor(self):
        self._write_src("demo/mod.rs")
        self._write_src("demo/unlisted.rs")
        layout = self._write_layout(
            """
[demo]
package = "@ai-sdk/demo"

[[demo.files]]
upstream = "src/index.ts"
rust = "demo/mod.rs"
status = "mapped"

[demo.files.tests]
"""
        )
        errors = check_layout.check_layout_files(layout, self.src)
        self.assertTrue(any("unlisted Rust source file" in e for e in errors), errors)

    def test_repo_layout_toml_passes_cli(self):
        stdout, stderr = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            code = check_layout.main([])
        self.assertEqual(code, 0, stderr.getvalue())
        self.assertIn("layout.toml: ok", stdout.getvalue())


if __name__ == "__main__":
    unittest.main()
