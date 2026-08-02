#!/usr/bin/env python3
"""Migrate remaining shell-type constructions in openai_compatible_test.rs.

Pattern: `let cfg = XxxConfig::new("test-api-key").with_base_url(server.uri());
          let provider = XxxProvider::new(cfg);
          let model = provider.model("...");`
-> provider("xxx", Some(key), "model", Some(ProviderOptions { base_url }))

Usage: uv run python scripts/migrate_openai_compatible_standalone.py
"""

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
F = ROOT / "aimux-providers" / "tests" / "openai_compatible_test.rs"
JSON = ROOT / "aimux-providers" / "src" / "provider_registry.json"


def to_type(n: str) -> str:
    parts = n.split("_")
    return "".join(p[0].upper() + p[1:] if p and not p[0].isdigit() else p for p in parts)


def main() -> int:
    entries = json.loads(JSON.read_text(encoding="utf-8"))
    type_to_name = {to_type(e["name"]) + "Config": e["name"] for e in entries}

    text = F.read_text(encoding="utf-8")

    pattern = re.compile(
        r'let (?:config|cfg) = (\w+Config)::new\("test-api-key"\)\.with_base_url\(server\.uri\(\)\);\n'
        r"\s*let provider = \w+Provider::new\(\1\);\n"
        r'\s*let model = provider\.model\("([^"]+)"\);'
    )

    def repl(m: re.Match) -> str:
        config_ty = m.group(1)
        model_id = m.group(2)
        name = type_to_name.get(config_ty)
        if name is None:
            raise ValueError(f"no registry name for {config_ty}")
        return (
            "let model = provider(\n"
            f'                    "{name}",\n'
            '                    Some("test-api-key".to_string()),\n'
            f'                    "{model_id}",\n'
            "                    Some(ProviderOptions {\n"
            "                        base_url: Some(server.uri()),\n"
            "                        ..Default::default()\n"
            "                    }),\n"
            "                )\n"
            '                .expect("provider construction");'
        )

    text, n = pattern.subn(repl, text)
    F.write_text(text, encoding="utf-8")
    print(f"standalone constructions replaced: {n}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
