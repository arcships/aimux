#!/usr/bin/env python3
"""Validate aimux-providers/layout.toml against aimux-providers/src/.

Checks:
  1. Each `[<vendor>]` table names a `package` and an existing `src/<vendor>/` directory.
  2. Each `[[<vendor>.files]]` entry uses a valid `status`:
     `mapped`, `missing`, `elsewhere`, `not-applicable`, or `aimux`.
  3. Every `rust = "..."` path resolves to an existing file under `aimux-providers/src/`.
  4. Every `status = "aimux"` entry carries a non-empty `reason` (and `rust`).
  5. Every `status = "missing"` entry carries a positive integer `issue` (and `upstream`).
  6. Every `status = "mapped"` or `"elsewhere"` entry carries `upstream`, `rust`,
     and a `tests` table.
  7. Every `.rs` file inside a `layout.toml`-covered vendor directory
     (excluding `mod.rs` and `tests/`) is registered in `layout.toml`.

Supports Python 3.9+ standard library only (uses `tomllib` when available,
falling back to a pure-stdlib TOML subset parser).

Usage:
    python3 scripts/check_layout.py
"""

import argparse
import pathlib
import re
import sys

try:
    import tomllib  # type: ignore[import-not-found]
except ModuleNotFoundError:
    tomllib = None

ROOT = pathlib.Path(__file__).resolve().parent.parent
DEFAULT_LAYOUT = ROOT / "aimux-providers" / "layout.toml"
DEFAULT_SRC = ROOT / "aimux-providers" / "src"

VALID_STATUSES = frozenset({"mapped", "missing", "elsewhere", "not-applicable", "aimux"})
_BARE_KEY_RE = re.compile(r"[A-Za-z0-9_-]+")
_INT_RE = re.compile(r"[+-]?(?:0|[1-9][0-9_]*)")


class TomlParseError(ValueError):
    """Raised when layout.toml contains invalid TOML syntax."""


class _SubsetTomlParser:
    """Minimal pure-stdlib TOML parser for Python 3.9+."""

    def __init__(self, text: str) -> None:
        self.text = text
        self.pos = 0
        self.length = len(text)

    def _error(self, msg: str) -> TomlParseError:
        line = self.text.count("\n", 0, self.pos) + 1
        return TomlParseError(f"line {line}: {msg}")

    def _peek(self, offset: int = 0) -> str:
        idx = self.pos + offset
        return self.text[idx] if idx < self.length else ""

    def _skip_ws_inline(self) -> None:
        while self.pos < self.length and self.text[self.pos] in " \t":
            self.pos += 1

    def _skip_ws_and_comments(self) -> None:
        while self.pos < self.length:
            ch = self.text[self.pos]
            if ch in " \t\r\n":
                self.pos += 1
            elif ch == "#":
                while self.pos < self.length and self.text[self.pos] != "\n":
                    self.pos += 1
            else:
                break

    def _skip_line_end(self) -> None:
        self._skip_ws_inline()
        if self._peek() == "#":
            while self.pos < self.length and self.text[self.pos] != "\n":
                self.pos += 1
        if self.pos >= self.length:
            return
        if self.text[self.pos] == "\r":
            self.pos += 1
        if self.pos < self.length:
            if self.text[self.pos] != "\n":
                raise self._error(f"unexpected trailing character {self.text[self.pos]!r}")
            self.pos += 1

    def _parse_basic_string(self) -> str:
        self.pos += 1  # opening '"'
        out = []
        escapes = {
            '"': '"',
            "\\": "\\",
            "b": "\b",
            "f": "\f",
            "n": "\n",
            "r": "\r",
            "t": "\t",
        }
        while self.pos < self.length:
            ch = self.text[self.pos]
            self.pos += 1
            if ch == '"':
                return "".join(out)
            if ch == "\n":
                raise self._error("unterminated basic string")
            if ch == "\\":
                if self.pos >= self.length:
                    raise self._error("unterminated escape sequence")
                esc = self.text[self.pos]
                self.pos += 1
                if esc in escapes:
                    out.append(escapes[esc])
                elif esc == "u":
                    hex_digits = self.text[self.pos : self.pos + 4]
                    if len(hex_digits) != 4:
                        raise self._error("invalid \\u escape")
                    self.pos += 4
                    out.append(chr(int(hex_digits, 16)))
                elif esc == "U":
                    hex_digits = self.text[self.pos : self.pos + 8]
                    if len(hex_digits) != 8:
                        raise self._error("invalid \\U escape")
                    self.pos += 8
                    out.append(chr(int(hex_digits, 16)))
                else:
                    raise self._error(f"unsupported escape \\{esc}")
            else:
                out.append(ch)
        raise self._error("unterminated basic string")

    def _parse_literal_string(self) -> str:
        self.pos += 1  # opening "'"
        start = self.pos
        while self.pos < self.length:
            ch = self.text[self.pos]
            if ch == "'":
                val = self.text[start : self.pos]
                self.pos += 1
                return val
            if ch == "\n":
                raise self._error("unterminated literal string")
            self.pos += 1
        raise self._error("unterminated literal string")

    def _parse_key_part(self) -> str:
        ch = self._peek()
        if ch == '"':
            return self._parse_basic_string()
        if ch == "'":
            return self._parse_literal_string()
        m = _BARE_KEY_RE.match(self.text, self.pos)
        if not m:
            raise self._error(f"expected key at {ch!r}")
        self.pos = m.end()
        return m.group(0)

    def _parse_dotted_key(self) -> list:
        parts = [self._parse_key_part()]
        while True:
            self._skip_ws_inline()
            if self._peek() != ".":
                break
            self.pos += 1
            self._skip_ws_inline()
            parts.append(self._parse_key_part())
        return parts

    def _parse_value(self):
        self._skip_ws_inline()
        ch = self._peek()
        if ch == '"':
            return self._parse_basic_string()
        if ch == "'":
            return self._parse_literal_string()
        if ch == "[":
            return self._parse_array()
        if ch == "{":
            return self._parse_inline_table()
        if self.text.startswith("true", self.pos) and not _BARE_KEY_RE.match(
            self.text, self.pos + 4
        ):
            self.pos += 4
            return True
        if self.text.startswith("false", self.pos) and not _BARE_KEY_RE.match(
            self.text, self.pos + 5
        ):
            self.pos += 5
            return False
        m = _INT_RE.match(self.text, self.pos)
        if m:
            self.pos = m.end()
            return int(m.group(0).replace("_", ""))
        raise self._error(f"unexpected value starting with {ch!r}")

    def _parse_array(self) -> list:
        self.pos += 1  # '['
        items = []
        while True:
            self._skip_ws_and_comments()
            if self._peek() == "]":
                self.pos += 1
                return items
            items.append(self._parse_value())
            self._skip_ws_and_comments()
            ch = self._peek()
            if ch == ",":
                self.pos += 1
            elif ch == "]":
                self.pos += 1
                return items
            else:
                raise self._error(f"expected ',' or ']' in array, got {ch!r}")

    def _parse_inline_table(self) -> dict:
        self.pos += 1  # '{'
        table = {}
        self._skip_ws_inline()
        if self._peek() == "}":
            self.pos += 1
            return table
        while True:
            self._skip_ws_inline()
            parts = self._parse_dotted_key()
            self._skip_ws_inline()
            if self._peek() != "=":
                raise self._error("expected '=' in inline table")
            self.pos += 1
            val = self._parse_value()
            self._assign_dotted(table, parts, val)
            self._skip_ws_inline()
            ch = self._peek()
            if ch == ",":
                self.pos += 1
            elif ch == "}":
                self.pos += 1
                return table
            else:
                raise self._error(f"expected ',' or '}}' in inline table, got {ch!r}")

    def _assign_dotted(self, target: dict, parts: list, value) -> None:
        cur = target
        for part in parts[:-1]:
            if part not in cur:
                cur[part] = {}
            elif not isinstance(cur[part], dict):
                raise self._error(f"key {part!r} is already a non-table")
            cur = cur[part]
        leaf = parts[-1]
        if leaf in cur:
            raise self._error(f"duplicate key {leaf!r}")
        cur[leaf] = value

    def _resolve_table_path(self, root: dict, parts: list, is_array: bool) -> dict:
        cur = root
        for part in parts[:-1]:
            if part not in cur:
                cur[part] = {}
            nxt = cur[part]
            if isinstance(nxt, list):
                if not nxt:
                    raise self._error(f"empty array of tables for {part!r}")
                nxt = nxt[-1]
            if not isinstance(nxt, dict):
                raise self._error(f"cannot traverse non-table {part!r}")
            cur = nxt
        leaf = parts[-1]
        if is_array:
            arr = cur.setdefault(leaf, [])
            if not isinstance(arr, list):
                raise self._error(f"key {leaf!r} is not an array of tables")
            new_table = {}
            arr.append(new_table)
            return new_table
        if leaf not in cur:
            cur[leaf] = {}
        elif not isinstance(cur[leaf], dict):
            raise self._error(f"key {leaf!r} is not a table")
        return cur[leaf]

    def parse(self) -> dict:
        root: dict = {}
        current = root
        while True:
            self._skip_ws_and_comments()
            if self.pos >= self.length:
                return root
            if self._peek() == "[":
                self.pos += 1
                is_array = self._peek() == "["
                if is_array:
                    self.pos += 1
                self._skip_ws_inline()
                parts = self._parse_dotted_key()
                self._skip_ws_inline()
                if is_array:
                    if not self.text.startswith("]]", self.pos):
                        raise self._error("expected ']]'")
                    self.pos += 2
                else:
                    if self._peek() != "]":
                        raise self._error("expected ']'")
                    self.pos += 1
                self._skip_line_end()
                current = self._resolve_table_path(root, parts, is_array)
            else:
                parts = self._parse_dotted_key()
                self._skip_ws_inline()
                if self._peek() != "=":
                    raise self._error("expected '='")
                self.pos += 1
                value = self._parse_value()
                self._skip_line_end()
                self._assign_dotted(current, parts, value)


def parse_toml(text: str) -> dict:
    """Parse TOML text using `tomllib` when available or the stdlib subset parser."""
    if tomllib is not None:
        return tomllib.loads(text)
    return _SubsetTomlParser(text).parse()


def parse_toml_subset(text: str) -> dict:
    """Parse TOML text using the pure-stdlib subset parser."""
    return _SubsetTomlParser(text).parse()


def _is_nonempty_str(val) -> bool:
    return isinstance(val, str) and bool(val.strip())


def _is_safe_relative_path(rel: str) -> bool:
    p = pathlib.PurePosixPath(rel)
    return not p.is_absolute() and ".." not in p.parts


def validate_layout(layout: dict, src_dir: pathlib.Path) -> list:
    """Return a list of validation error strings for a parsed layout mapping."""
    errors = []
    if not isinstance(layout, dict) or not layout:
        return ["layout.toml contains no vendor tables"]

    for vendor, table in layout.items():
        if not isinstance(table, dict):
            errors.append(f"[{vendor}]: expected a table")
            continue
        if not _is_nonempty_str(table.get("package")):
            errors.append(f"[{vendor}]: missing or empty `package`")

        vendor_dir = src_dir / vendor
        if not vendor_dir.is_dir():
            errors.append(f"[{vendor}]: vendor directory does not exist: {vendor_dir}")

        files = table.get("files")
        if not isinstance(files, list):
            errors.append(f"[{vendor}]: missing `files` array")
            continue

        listed_rust_paths = set()
        for idx, entry in enumerate(files):
            label = f"[{vendor}].files[{idx}]"
            if not isinstance(entry, dict):
                errors.append(f"{label}: expected a table")
                continue

            status = entry.get("status")
            if status not in VALID_STATUSES:
                errors.append(
                    f"{label}: invalid status {status!r} "
                    f"(expected one of {sorted(VALID_STATUSES)})"
                )
                continue

            rust = entry.get("rust")
            if rust is not None:
                if not _is_nonempty_str(rust) or not _is_safe_relative_path(rust):
                    errors.append(f"{label}: invalid `rust` path {rust!r}")
                else:
                    listed_rust_paths.add(rust)
                    rust_file = src_dir / rust
                    if not rust_file.is_file():
                        errors.append(
                            f"{label}: `rust` file does not exist under src/: {rust}"
                        )

            if status == "aimux":
                if not _is_nonempty_str(rust):
                    errors.append(f"{label}: status = \"aimux\" requires `rust`")
                if not _is_nonempty_str(entry.get("reason")):
                    errors.append(f"{label}: status = \"aimux\" requires `reason`")
            elif status == "missing":
                if not _is_nonempty_str(entry.get("upstream")):
                    errors.append(f"{label}: status = \"missing\" requires `upstream`")
                issue = entry.get("issue")
                if not isinstance(issue, int) or isinstance(issue, bool) or issue <= 0:
                    errors.append(
                        f"{label}: status = \"missing\" requires a positive integer `issue`"
                    )
            elif status in ("mapped", "elsewhere"):
                if not _is_nonempty_str(entry.get("upstream")):
                    errors.append(f"{label}: status = {status!r} requires `upstream`")
                if not _is_nonempty_str(rust):
                    errors.append(f"{label}: status = {status!r} requires `rust`")
                if not isinstance(entry.get("tests"), dict):
                    errors.append(f"{label}: status = {status!r} requires a `.tests` table")
            elif status == "not-applicable":
                if not _is_nonempty_str(entry.get("upstream")):
                    errors.append(f"{label}: status = \"not-applicable\" requires `upstream`")

        if vendor_dir.is_dir():
            for path in sorted(vendor_dir.rglob("*.rs")):
                rel_to_vendor = path.relative_to(vendor_dir)
                if "tests" in rel_to_vendor.parts or path.name == "mod.rs":
                    continue
                rel_to_src = path.relative_to(src_dir).as_posix()
                if rel_to_src not in listed_rust_paths:
                    errors.append(
                        f"[{vendor}]: unlisted Rust source file under {vendor}/: {rel_to_src}"
                    )

    return errors


def check_layout_files(layout_path: pathlib.Path, src_dir: pathlib.Path) -> list:
    """Read, parse, and validate `layout_path` against `src_dir`."""
    try:
        text = layout_path.read_text(encoding="utf-8")
    except OSError as exc:
        return [f"unable to read {layout_path}: {exc}"]
    try:
        parsed = parse_toml(text)
    except Exception as exc:  # noqa: BLE001 - tomllib.TOMLDecodeError or TomlParseError
        return [f"invalid TOML in {layout_path}: {exc}"]
    return validate_layout(parsed, src_dir)


def main(argv: list = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--layout", type=pathlib.Path, default=DEFAULT_LAYOUT)
    parser.add_argument("--src", type=pathlib.Path, default=DEFAULT_SRC)
    args = parser.parse_args(argv)

    errors = check_layout_files(args.layout, args.src)
    if errors:
        for err in errors:
            print(f"layout check error: {err}", file=sys.stderr)
        return 1

    parsed = parse_toml(args.layout.read_text(encoding="utf-8"))
    total_entries = sum(
        len(tbl.get("files", [])) for tbl in parsed.values() if isinstance(tbl, dict)
    )
    print(f"layout.toml: ok ({len(parsed)} vendors, {total_entries} file entries)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
