#!/usr/bin/env python3
"""Extract OpenAI-compatible provider base URLs from litellm source.

Scans litellm/litellm/llms/*/chat/transformation.py for api_base
definitions, and categorizes providers as OpenAI-compatible thin wrappers
vs. native-protocol providers.

Usage:
    uv run python scripts/extract_litellm_bases.py
"""

import re
import json
from pathlib import Path

LITELLM_LLM = Path(__file__).resolve().parent.parent / "reference" / "litellm" / "litellm" / "llms"

# Providers we already have implemented in aimux
ALREADY_DONE = {
    "openai", "anthropic", "anthropic_aws", "azure", "bedrock", "google", "vertex",
    "mistral", "cohere", "xai", "deepseek", "groq", "fireworks_ai", "together_ai",
    "perplexity", "moonshot_ai", "cerebras", "alibaba", "baseten", "bytedance",
    "deepinfra", "huggingface", "vercel", "openrouter", "copilot", "llamafile",
    "mistralrs", "doubleword", "voyage", "cartesia", "elevenlabs", "hume", "lmnt",
    "assemblyai", "deepgram", "fal_ai", "gladia", "revai", "black_forest_labs",
    "luma", "prodia", "replicate", "klingai", "open_responses",
    "ollama", "zai", "github", "siliconflow", "lmstudio", "sambanova",
}

# litellm provider name -> (base_url, env_var, is_openai_compatible)
# We extract from transformation.py files
def extract_from_transformation(filepath: Path) -> dict | None:
    """Extract api_base and env var from a transformation.py file."""
    try:
        content = filepath.read_text(encoding="utf-8", errors="ignore")
    except Exception:
        return None

    # Look for api_base patterns
    base_url = None
    # Pattern 1: self.api_base = "https://..."
    # Pattern 2: DEFAULT_API_BASE = "https://..."
    # Pattern 3: api_base = "https://..."
    for pattern in [
        r'(?:DEFAULT_API_BASE|api_base)\s*[:=]\s*["\']f?(https?://[^"\']+)["\']',
        r'self\.api_base\s*=\s*["\']f?(https?://[^"\']+)["\']',
        r'api_base\s*=\s*["\']f?(https?://[^"\']+)["\']',
    ]:
        m = re.search(pattern, content)
        if m:
            base_url = m.group(1)
            break

    # Look for env var pattern
    env_var = None
    for pattern in [
        r'os\.getenv\(["\'](\w+_(?:API_KEY|TOKEN|KEY))["\']',
        r'os\.environ\.get\(["\'](\w+_(?:API_KEY|TOKEN|KEY))["\']',
        r'ENV_VAR\s*=\s*["\'](\w+_(?:API_KEY|TOKEN|KEY))["\']',
    ]:
        m = re.search(pattern, content)
        if m:
            env_var = m.group(1)
            break

    # Check if it's OpenAI-compatible (extends OpenAIConfig or similar)
    is_openai_compat = bool(
        re.search(r'OpenAIConfig|BaseConfig.*openai|openai.*compatible', content, re.IGNORECASE)
    ) or bool(
        re.search(r'class \w+Config\(.*BaseConfig\)', content)
    )

    return {
        "base_url": base_url,
        "env_var": env_var,
        "is_openai_compat": is_openai_compat,
    }


def main():
    providers = {}

    for provider_dir in sorted(LITELLM_LLM.iterdir()):
        if not provider_dir.is_dir():
            continue
        provider_name = provider_dir.name

        # Skip already done
        if provider_name in ALREADY_DONE:
            continue

        # Skip deprecated
        if provider_name in ("deprecated_providers", "base_llm", "aiohttp_openai",
                             "custom_httpx", "a2a", "empower"):
            continue

        # Look for chat/transformation.py
        trans_file = provider_dir / "chat" / "transformation.py"
        if not trans_file.exists():
            # Try chat/*.py
            chat_dir = provider_dir / "chat"
            if chat_dir.exists():
                py_files = list(chat_dir.glob("*.py"))
                if py_files:
                    trans_file = py_files[0]
                else:
                    continue
            else:
                continue

        info = extract_from_transformation(trans_file)
        if info and info["base_url"]:
            providers[provider_name] = info

    # Output as JSON
    print(json.dumps(providers, indent=2, ensure_ascii=False))
    print(f"\nTotal: {len(providers)} providers with base_url found", flush=True)


if __name__ == "__main__":
    main()
