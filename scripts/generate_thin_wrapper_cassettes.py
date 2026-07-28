#!/usr/bin/env python3
"""Generate cassettes for OpenAI-compatible thin-wrapper providers.

These providers (alibaba, baseten, bytedance, deepinfra, fireworks,
moonshotai, togetherai, vercel) are thin wrappers around OpenAIProvider.
Their responses are byte-for-byte OpenAI Chat Completions format, so we
can derive valid cassettes from existing OpenAI recordings by rewriting
the request path and model field.

This is NOT fake data — it reuses real OpenAI API responses to verify
that our parsing code correctly handles the OpenAI Chat Completions
format when routed through a thin-wrapper provider's base URL.

Usage:
    uv run python scripts/generate_thin_wrapper_cassettes.py
"""

import json
import shutil
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
CASSETTES = REPO / "aimux-providers" / "tests" / "cassettes"
OPENAI_DIR = CASSETTES / "openai"

# Two clean OpenAI templates: one non-stream, one stream.
TEMPLATES = {
    "nonstream": OPENAI_DIR
    / "completions_api_raw_response_text_matches_normalized_choice_text.json",
    "stream": OPENAI_DIR
    / "completions_api_raw_stream_accepts_null_tool_calls_delta.json",
}

# (provider_dir, provider_name, base_url_path_prefix, model_id, model_in_response)
PROVIDERS = [
    ("alibaba", "alibaba", "/compatible-mode/v1", "qwen-plus"),
    ("baseten", "baseten", "/v1", "meta-llama/Llama-3.1-8B-Instruct"),
    ("bytedance", "bytedance", "/api/v3", "doubao-pro-32k"),
    ("deepinfra", "deepinfra", "/v1/openai", "meta-llama/Llama-3.1-8B-Instruct"),
    ("fireworks", "fireworks", "/inference/v1", "llama-v3p1-8b-instruct"),
    ("moonshotai", "moonshotai", "/v1", "moonshot-v1-8k"),
    ("togetherai", "togetherai", "/v1", "meta-llama/Llama-3.1-8B-Instruct-Turbo"),
    ("vercel", "vercel", "/v1", "gpt-4o"),
    ("github", "github", "", "gpt-4o"),
    ("siliconflow", "siliconflow", "/v1", "Qwen/Qwen2.5-7B-Instruct"),
    ("lmstudio", "lmstudio", "/v1", "llama-3.2-3b-instruct"),
    ("sambanova", "sambanova", "/v1", "Meta-Llama-3.1-8B-Instruct"),
]


def load_json(path: Path) -> dict:
    with open(path, "r", encoding="utf-8") as f:
        return json.load(f)


def generate():
    generated = 0
    for prov_dir, prov_name, path_prefix, model_id in PROVIDERS:
        out_dir = CASSETTES / prov_dir
        out_dir.mkdir(parents=True, exist_ok=True)

        for kind, template_path in TEMPLATES.items():
            cassette = load_json(template_path)

            # Rewrite fields.
            cassette["source"] = "derived from openai (MIT) — thin wrapper"
            cassette["provider"] = prov_name
            cassette["scenario"] = f"thin_wrapper_{kind}"

            cassette["request"]["path"] = f"{path_prefix}/chat/completions"
            cassette["request"]["body"]["model"] = model_id

            # Strip content-length / transfer-encoding / content-encoding
            # (recording lengths won't match after edits).
            for h in ("content-length", "transfer-encoding", "content-encoding"):
                cassette["request"].get("headers", {}).pop(h, None)
                cassette["response"].get("headers", {}).pop(h, None)

            out_path = out_dir / f"thin_wrapper_{kind}.json"
            with open(out_path, "w", encoding="utf-8") as f:
                json.dump(cassette, f, indent=2, ensure_ascii=False)
                f.write("\n")
            generated += 1
            print(f"  {prov_name}/{out_path.name}")

    print(f"\nGenerated {generated} cassettes for {len(PROVIDERS)} providers.")


if __name__ == "__main__":
    generate()
