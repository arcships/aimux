#!/usr/bin/env python3
"""Scan litellm constants.py and all transformation files for provider base URLs."""

import re, json
from pathlib import Path

LITELLM = Path(__file__).resolve().parent.parent / "reference" / "litellm" / "litellm"

results = {}

# 1. Scan constants.py
constants = (LITELLM / "constants.py").read_text(encoding="utf-8", errors="ignore")
# Find patterns like: "provider": "https://..."
for m in re.finditer(r'"(\w+)":\s*"(https://[^"]+)"', constants):
    name, url = m.group(1), m.group(2)
    if name not in results and "api" in url.lower():
        results[name] = {"base_url": url, "source": "constants.py"}

# 2. Scan all transformation.py files
llms_dir = LITELLM / "llms"
for provider_dir in sorted(llms_dir.iterdir()):
    if not provider_dir.is_dir():
        continue
    name = provider_dir.name
    for py_file in provider_dir.rglob("*.py"):
        try:
            content = py_file.read_text(encoding="utf-8", errors="ignore")
        except:
            continue
        # Look for string assignments with https URLs
        for m in re.finditer(r'(?:api_base|DEFAULT_API_BASE|base_url|API_BASE)\s*[:=]\s*["\']f?(https?://[^"\']+)["\']', content):
            url = m.group(1)
            if name not in results:
                results[name] = {"base_url": url, "source": str(py_file.relative_to(LITELLM))}
            break
        # Also look for inline https URLs in get_api_base methods
        for m in re.finditer(r'return\s+["\']f?(https?://[^"\']+)["\']', content):
            url = m.group(1)
            if name not in results and "api" in url.lower():
                results[name] = {"base_url": url, "source": str(py_file.relative_to(LITELLM))}
            break

# 3. Scan __init__.py for provider configurations
init_file = LITELLM / "__init__.py"
init_content = init_file.read_text(encoding="utf-8", errors="ignore")
# Look for provider -> api_base mappings
for m in re.finditer(r'"(\w+)":\s*"(https://[^"]+)"', init_content):
    name, url = m.group(1), m.group(2)
    if name not in results and "api" in url.lower():
        results[name] = {"base_url": url, "source": "__init__.py"}

print(json.dumps(results, indent=2, ensure_ascii=False))
print(f"\nTotal: {len(results)}", flush=True)
