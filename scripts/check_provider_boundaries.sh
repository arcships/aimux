#!/usr/bin/env bash
# Provider boundary grep gates (RFC-0036, AI SDK architecture alignment).
#
# Keeps the provider layer on the same side of the line as the AI SDK:
#   1. Providers never read call-level plumbing (retry budget, timeouts,
#      session / call ids) from `CallOptions`. Core owns retry and recording;
#      a provider hands the options to `HttpRequest::new(url, headers, options)`
#      and lets the transport read what it needs.
#   2. Names removed by the provider-factory rewrite stay removed.
#   3. The OpenAI package keeps no builder-era `OpenAIConfig` reads.
#   4. The shared protocol packages never compare a provider name: vendor
#      behavior is injected (flags, hooks, a dialect), not branched on.
#   5. The builder-era compat types and the placeholder key stay removed.
#
# Usage: bash scripts/check_provider_boundaries.sh   (exit 1 on any violation)

set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

status=0

# violation <rule> <grep output>: print and remember a failure when the grep
# output is non-empty.
violation() {
    local rule="$1" hits="$2"
    if [[ -n "$hits" ]]; then
        printf 'provider boundary violation: %s\n%s\n\n' "$rule" "$hits" >&2
        status=1
    fi
}

# ── Rule 1: no call-level reads inside aimux-providers/src ───────────────────
#
# Allowed: lines that build `HttpRequest::new(`, and aimux-providers/src/replay.rs.
# WebSocket realtime sessions are the one transport that the provider itself
# opens, so they carry `options.timeout` into `WebSocketRequest` (no core
# operation owns that timeout); the two sites are listed explicitly.
rule1_pattern='options\.(max_retries|timeout|session_id|call_id)\b'
rule1_hits="$(
    { grep -rnE "$rule1_pattern" aimux-providers/src --include='*.rs' || true; } |
        grep -v 'HttpRequest::new(' |
        grep -v '^aimux-providers/src/replay\.rs:' |
        grep -vE '^aimux-providers/src/(elevenlabs\.rs|openai/transcription\.rs):[0-9]+: *timeout: options\.timeout,$' ||
        true
)"
violation 'aimux-providers/src reads options.max_retries / timeout / session_id / call_id' "$rule1_hits"

# ── Rule 2: removed names stay removed (whole repository) ────────────────────
#
# Source files only; docs/, rfc/, fixtures/, build output and vendored
# node_modules are history or data, not code.
search_removed() {
    local pattern="$1"
    grep -rnE "$pattern" . \
        --include='*.rs' --include='*.py' --include='*.pyi' --include='*.ts' \
        --include='*.js' --include='*.go' --include='*.java' --include='*.kt' \
        --include='*.swift' --include='*.dart' --include='*.c' --include='*.h' \
        --exclude-dir=target --exclude-dir=docs --exclude-dir=rfc \
        --exclude-dir=fixtures --exclude-dir=node_modules --exclude-dir=.git ||
        true
}

for pattern in \
    'fn config_snapshot' \
    'fn retry_config' \
    'RetryConfig' \
    'Provider::name' \
    'fn specification_version'; do
    violation "removed name \`$pattern\` is back" "$(search_removed "$pattern")"
done

# `shared_client()` lives only in the injectable transport.
shared_client_hits="$(
    search_removed 'shared_client\(\)' |
        grep -v '^\./aimux-provider-utils/src/fetch\.rs:' ||
        true
)"
violation 'shared_client() used outside aimux-provider-utils/src/fetch.rs' "$shared_client_hits"

# ── Rule 3: the OpenAI package reads its settings only through the model config ─
#
# `openai/` has no `OpenAIConfig` field reads, builder-era names or `from_env`:
# the credential is loaded in the request headers, the URL comes from the
# config's `url` closure and provider-level body rewrites are
# `transform_request_body`.
rule3_hits="$(
    grep -rnE 'config\.api_key\b|config\.base_url\b|body_overrides|api_key_source|from_env' \
        aimux-providers/src/openai --include='*.rs' || true
)"
violation 'aimux-providers/src/openai reads builder-era OpenAIConfig state' "$rule3_hits"

# ── Rule 4: no provider-name comparisons in the shared protocol packages ─────
#
# `openai/`, `openai_compatible/`, `anthropic/`, `google/`, `bedrock/` and
# `cohere/` implement a protocol for every vendor that speaks it. Which vendor
# is in play is data the caller injects (`ChatDialect`, hooks, settings), so a
# `== "groq"` / `!= "openai"` comparison in there is the branch the factory
# rewrite removed. The names checked are every registry row plus the native
# packages; comparisons against media types, tool names and wire tags are fine.
# (Unit tests live next to the code and compare no provider names.)
rule4_names="$(
    python3 - <<'PY'
import json

with open("aimux-providers/src/provider_registry.json", encoding="utf-8") as f:
    names = {row["name"] for row in json.load(f)}
names |= {
    "openai", "anthropic", "google", "vertex", "bedrock", "cohere", "mistral", "xai",
    "azure", "groq", "deepseek", "codex", "huggingface", "openai_compatible",
}
print("|".join(sorted(names, key=lambda n: (-len(n), n))))
PY
)"
rule4_hits="$(
    grep -rnE "(==|!=)[[:space:]]*\"($rule4_names)\"" \
        aimux-providers/src/openai aimux-providers/src/openai_compatible \
        aimux-providers/src/anthropic aimux-providers/src/google \
        aimux-providers/src/bedrock aimux-providers/src/cohere --include='*.rs' || true
)"
violation 'provider-name comparison in a shared protocol package (aimux-providers/src/{openai,openai_compatible,anthropic,google,bedrock,cohere})' "$rule4_hits"

# `"groq"` / `"deepseek"` literals belong to their own packages and the registry.
rule4b_hits="$(
    grep -rnE '"(groq|deepseek)"' \
        aimux-providers/src/openai aimux-providers/src/openai_compatible --include='*.rs' || true
)"
violation 'groq / deepseek literal in the shared OpenAI or compatible package' "$rule4b_hits"

# ── Rule 5: the builder-era compat names stay removed in the providers crate ──
rule5_hits="$(
    grep -rnE 'OpenAIConfig\b|OpenAIConfigProvider|OpenAICompatProfile|PLACEHOLDER_API_KEY|openai_legacy' \
        aimux-providers/src aimux-providers/tests --include='*.rs' || true
)"
violation 'OpenAIConfig / OpenAIConfigProvider / OpenAICompatProfile / PLACEHOLDER_API_KEY / openai_legacy is back in aimux-providers' "$rule5_hits"

if [[ "$status" -eq 0 ]]; then
    echo 'provider boundaries: ok'
fi
exit "$status"
