#!/usr/bin/env python3
"""Update lib.rs with all generated provider modules and re-exports."""

import re
from pathlib import Path

LIB_RS = Path(__file__).resolve().parent.parent / "aimux-providers" / "src" / "lib.rs"
SRC_DIR = LIB_RS.parent

# Find all .rs files that are not already declared in lib.rs
content = LIB_RS.read_text(encoding="utf-8")

# Get already declared modules
declared = set(re.findall(r'^pub mod (\w+);', content, re.MULTILINE))

# Find all .rs files to add
to_add = []
for f in sorted(SRC_DIR.glob("*.rs")):
    name = f.stem
    if name == "lib" or name in declared:
        continue
    # Extract struct names
    file_content = f.read_text(encoding="utf-8")
    config_match = re.search(r'pub struct (\w+Config)', file_content)
    provider_match = re.search(r'pub struct (\w+Provider)', file_content)
    if config_match and provider_match:
        to_add.append((name, config_match.group(1), provider_match.group(1)))

# Add module declarations after the last pub mod line
mod_lines = "\n".join(f"pub mod {name};" for name, _, _ in to_add)
# Find the last "pub mod" line and insert after it
lines = content.split("\n")
last_mod_idx = 0
for i, line in enumerate(lines):
    if line.startswith("pub mod "):
        last_mod_idx = i
lines.insert(last_mod_idx + 1, f"\n// Bulk-generated thin-wrapper providers.\n{mod_lines}")

# Add re-exports after the last pub use line
use_lines = "\n".join(f"pub use {name}::{{{cfg}, {prov}}};" for name, cfg, prov in to_add)
last_use_idx = 0
for i, line in enumerate(lines):
    if line.startswith("pub use "):
        last_use_idx = i
lines.insert(last_use_idx + 1, f"\n// Bulk-generated provider re-exports.\n{use_lines}")

LIB_RS.write_text("\n".join(lines), encoding="utf-8")
print(f"Added {len(to_add)} modules and re-exports to lib.rs")
