#!/usr/bin/env python3
"""Update inventory: mark all aimux ❌ as ✅薄 since we generated all providers."""

import re
from pathlib import Path

INV = Path(__file__).resolve().parent.parent / "rfc" / "0004-provider-inventory.md"
content = INV.read_text(encoding="utf-8")

cross = "\u274C"  # ❌
check_thin = "\u2705\u8584"  # ✅薄

# In table rows, the aimux column is the second column (after the provider name).
# Pattern: | provider_name | ❌ | ...
# Replace the first ❌ in each table row with ✅薄
lines = content.split("\n")
changed = 0
new_lines = []
for line in lines:
    if line.startswith("|") and cross in line:
        # Only replace the FIRST ❌ (which is the aimux column)
        new_line = line.replace(cross, check_thin, 1)
        if new_line != line:
            changed += 1
        new_lines.append(new_line)
    else:
        new_lines.append(line)

INV.write_text("\n".join(new_lines), encoding="utf-8")
print(f"Updated {changed} lines")
