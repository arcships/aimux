#!/usr/bin/env python3
"""Migrate openai_compatible_test.rs off the retired shell types (phase 4).

- Drop the 250-type import block -> provider/ProviderOptions.
- Retarget the test-generating macro to provider(name, ...).
- Rewrite call sites to (mod_name, "name", "model").

Usage: uv run python scripts/migrate_openai_compatible_test.py
Idempotent: pattern-based edits.
"""

import re
import sys
from pathlib import Path

F = Path("aimux-providers/tests/openai_compatible_test.rs")


def main() -> int:
    text = F.read_text(encoding="utf-8")

    # 1. Drop the big shell-type import block.
    new_import = "use aimux_providers::{provider, ProviderOptions};"
    text, n_imp = re.subn(
        r"use aimux_providers::\{.*?\};", new_import, text, count=1, flags=re.DOTALL
    )

    # 2. Macro signature: (mod_name, provider_name_literal, model_id_literal).
    text, n_sig = re.subn(
        r"macro_rules! openai_compatible_tests \{\n"
        r"    \(\n"
        r"        \$mod_name:ident,\n"
        r"        \$config:ty,\n"
        r"        \$provider:ty,\n"
        r"        \$model_id:literal\n"
        r"    \) => \{",
        "macro_rules! openai_compatible_tests {\n"
        "    (\n"
        "        $mod_name:ident,\n"
        "        $provider_name:literal,\n"
        "        $model_id:literal\n"
        "    ) => {",
        text,
        count=1,
    )

    # 3. make_provider body: shell types -> provider() with base_url override.
    text, n_fac = re.subn(
        r"fn make_provider\(server: &MockServer\) -> \$provider \{\n"
        r"                let config = <\$config>::new\(\"test-api-key\"\)\.with_base_url\(server\.uri\(\)\);\n"
        r"                <\$provider>::new\(config\)\n"
        r"            \}",
        "fn make_provider(server: &MockServer) -> Box<dyn LanguageModel> {\n"
        "                provider(\n"
        "                    $provider_name,\n"
        "                    Some(\"test-api-key\".to_string()),\n"
        "                    $model_id,\n"
        "                    Some(ProviderOptions {\n"
        "                        base_url: Some(server.uri()),\n"
        "                        ..Default::default()\n"
        "                    }),\n"
        "                )\n"
        "                .expect(\"provider construction\")\n"
        "            }",
        text,
        count=1,
    )

    # 4. All `let provider = make_provider(&server); let model = provider.model($model_id);`
    text, n_use = re.subn(
        r"let provider = make_provider\(&server\);\n\s*let model = provider\.model\(\$model_id\);",
        "let model = make_provider(&server);",
        text,
    )

    # 5. Call sites (single- and multi-line):
    #    openai_compatible_tests!(name, NameConfig, NameProvider, "model");
    text, n_calls = re.subn(
        r"openai_compatible_tests!\(\s*(\w+),\s*\w+Config,\s*\w+Provider,\s*(\"[^\"]*\")\s*\)",
        r'openai_compatible_tests!(\1, "\1", \2)',
        text,
    )

    # 5b. Tool-test call sites: mod name is "<name>_tools", provider is "<name>".
    text, n_tool_calls = re.subn(
        r"openai_compatible_tool_tests!\(\s*(\w+)_tools,\s*\w+Config,\s*\w+Provider,\s*(\"[^\"]*\")\s*\)",
        r'openai_compatible_tool_tests!(\1_tools, "\1", \2)',
        text,
    )

    # 5c. Tool-test macro signature + factory (same shape as the main macro).
    text, n_tool_sig = re.subn(
        r"macro_rules! openai_compatible_tool_tests \{\n"
        r"    \(\n"
        r"        \$mod_name:ident,\n"
        r"        \$config:ty,\n"
        r"        \$provider:ty,\n"
        r"        \$model_id:literal\n"
        r"    \) => \{",
        "macro_rules! openai_compatible_tool_tests {\n"
        "    (\n"
        "        $mod_name:ident,\n"
        "        $provider_name:literal,\n"
        "        $model_id:literal\n"
        "    ) => {",
        text,
        count=1,
    )
    text, n_tool_fac = re.subn(
        r"fn make_provider\(server: &MockServer\) -> \$provider \{\n"
        r"                let config = <\$config>::new\(\"test-api-key\"\)\.with_base_url\(server\.uri\(\)\);\n"
        r"                <\$provider>::new\(config\)\n"
        r"            \}",
        "fn make_provider(server: &MockServer) -> Box<dyn LanguageModel> {\n"
        "                provider(\n"
        "                    $provider_name,\n"
        "                    Some(\"test-api-key\".to_string()),\n"
        "                    $model_id,\n"
        "                    Some(ProviderOptions {\n"
        "                        base_url: Some(server.uri()),\n"
        "                        ..Default::default()\n"
        "                    }),\n"
        "                )\n"
        "                .expect(\"provider construction\")\n"
        "            }",
        text,
        count=1,
    )

    # 6. Doc comment example near the macro.
    text, _ = re.subn(
        r"//   openai_compatible_tests!\(groq, Groq, GroqConfig, GroqProvider, \"llama-3\.3-70b-versatile\"\);",
        '//   openai_compatible_tests!(groq, "groq", "llama-3.3-70b-versatile");',
        text,
        count=1,
    )

    F.write_text(text, encoding="utf-8")
    print(
        f"imports={n_imp} signature={n_sig} factory={n_fac} uses={n_use} "
        f"calls={n_calls} tool_calls={n_tool_calls} tool_sig={n_tool_sig} tool_fac={n_tool_fac}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
