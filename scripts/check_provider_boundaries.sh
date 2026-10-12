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
#   6. The Anthropic family (anthropic, anthropic_aws, anthropic on Vertex)
#      keeps no builder-era `AnthropicConfig` state, and the canonical
#      providerOptions key is spelled once, in `anthropic/options.rs`.
#   7. Google, Vertex and Bedrock keep no builder-era config state, the SigV4
#      shim stays deleted, and their providerOptions / providerMetadata keys
#      are spelled once, in `google/options.rs` and `bedrock/options.rs`.
#   8. Azure, xAI, Mistral, Cohere, Hugging Face, Codex, Open Responses, Voyage
#      and ElevenLabs keep no builder-era config state (and `StaticBearerConfig`
#      stays deleted), and their providerOptions / providerMetadata keys are
#      spelled once, in each package's `options.rs`.
#
#   9. No provider code calls the operation-level retry: `prepare_retries` and
#      `aimux_core::retry` are not used anywhere in aimux-providers/src (the
#      exception list is empty). A provider's own stages (poll, download) are
#      bounded by package constants through `shared::poll`; the call-level
#      retry belongs to Core.
#  10. The single-modality vendor packages (search, speech, transcription,
#      image, video, reranking) keep no builder-era config state, and their
#      providerOptions namespace keys are spelled once, in each package's
#      `options.rs`.
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
    'fn specification_version' \
    'StaticBearerConfig' \
    'api_key_source' \
    'body_merge|apply_body_overrides|deep_merge_json'; do
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

# ── Rule 6: the Anthropic family reads its settings only through the model config ─
#
# No `AnthropicConfig` / `body_overrides` / `api_key_source` / `from_env`, and
# the providerOptions namespace goes through `options::anthropic_options` /
# `CANONICAL`: a `"anthropic"` literal anywhere else would read or write a key
# that ignores the provider's custom name.
anthropic_family='aimux-providers/src/anthropic aimux-providers/src/anthropic_aws aimux-providers/src/vertex/anthropic_model.rs'
rule6_hits="$(
    # shellcheck disable=SC2086
    grep -rnE 'AnthropicConfig\b|body_overrides|api_key_source|from_env' \
        $anthropic_family --include='*.rs' || true
)"
violation 'Anthropic family reads builder-era AnthropicConfig state' "$rule6_hits"
rule6b_hits="$(
    # shellcheck disable=SC2086
    { grep -rnE '"anthropic"' $anthropic_family --include='*.rs' || true; } |
        grep -v '^aimux-providers/src/anthropic/options\.rs:' ||
        true
)"
violation '"anthropic" literal outside aimux-providers/src/anthropic/options.rs (use options::CANONICAL / anthropic_options)' "$rule6b_hits"

# ── Rule 7: Google, Vertex and Bedrock read their settings through the model config ─
#
# No `GoogleConfig` / `VertexProviderConfig` / `BedrockProviderConfig` /
# `body_overrides` / `api_key_source` / `from_env`; `bedrock::sigv4` is gone
# (signing is the `SigV4Fetch` decorator); and the canonical namespace keys
# (`google`, `googleVertex`, `amazonBedrock`) are quoted only in the two
# `options.rs` helpers (comments, unit tests and the SigV4 service name
# aside). The historical `vertex` / `bedrock` aliases are not ported at all, so
# they are flagged too: no model may read or write them.
gvb_family='aimux-providers/src/google aimux-providers/src/vertex aimux-providers/src/bedrock'
rule7_hits="$(
    # shellcheck disable=SC2086
    grep -rnE 'GoogleConfig\b|VertexProviderConfig|BedrockProviderConfig|VertexAuth\b|BedrockAuth\b|api_key_source|from_env|body_overrides' \
        $gvb_family --include='*.rs' || true
)"
violation 'Google / Vertex / Bedrock reads builder-era config state' "$rule7_hits"
rule7b_hits="$(
    search_removed 'bedrock::sigv4|bedrock/sigv4' | grep -v '^\./scripts/check_provider_boundaries\.sh:' || true
)"
violation 'bedrock::sigv4 is back (use SigV4Fetch)' "$rule7b_hits"
rule7c_hits="$(
    # Non-test, non-comment lines of every file but the two helpers.
    find $gvb_family -name '*.rs' ! -name options.rs -print0 |
        xargs -0 awk '
            FNR == 1 { in_tests = 0 }
            /^#\[cfg\(test\)\]/ { in_tests = 1 }
            in_tests { next }
            /^[[:space:]]*\/\// { next }
            /"(google|googleVertex|vertex|amazonBedrock|bedrock)"/ && !/SIGV4_SERVICE/ {
                printf "%s:%d:%s\n", FILENAME, FNR, $0
            }
        ' || true
)"
violation 'providerOptions namespace key spelled outside google/options.rs or bedrock/options.rs' "$rule7c_hits"

# ── Rule 8: the vendor packages read their settings through the model config ──
#
# No `XxxConfig` / `AzureAuth` / `TokenProvider` / `StaticBearerConfig` /
# `body_overrides` / `api_key_source` / `from_env`, and the namespace keys
# (`azure`, `xai`, `mistral`, `cohere`, `huggingface`, `voyage`, `elevenlabs`)
# are quoted only in the package's `options.rs` (comments, unit tests and the
# `DEFAULT_NAME` provider-name constants aside). Codex reads the OpenAI
# Responses namespace through the shared model, and Open Responses derives its
# key from the provider name, so neither spells a key.
vendor_family='aimux-providers/src/azure aimux-providers/src/xai aimux-providers/src/mistral aimux-providers/src/cohere aimux-providers/src/voyage aimux-providers/src/huggingface aimux-providers/src/huggingface.rs aimux-providers/src/codex.rs aimux-providers/src/open_responses.rs aimux-providers/src/elevenlabs aimux-providers/src/elevenlabs.rs'
rule8_hits="$(
    # shellcheck disable=SC2086
    grep -rnE '\b(AzureConfig|AzureAuth|TokenProvider|XAIConfig|MistralConfig|CohereConfig|VoyageConfig|HuggingFaceConfig|CodexConfig|OpenResponsesConfig|ElevenLabsConfig|StaticBearerConfig)\b|body_overrides|api_key_source|from_env' \
        $vendor_family --include='*.rs' || true
)"
violation 'Azure / xAI / Mistral / Cohere / Hugging Face / Codex / Open Responses / Voyage / ElevenLabs reads builder-era config state' "$rule8_hits"
rule8b_hits="$(
    # Non-test, non-comment lines of every file but the options helpers.
    find $vendor_family -name '*.rs' ! -name options.rs -print0 |
        xargs -0 awk '
            FNR == 1 { in_tests = 0 }
            /^#\[cfg\(test\)\]/ { in_tests = 1 }
            in_tests { next }
            /^[[:space:]]*\/\// { next }
            /"(azure|xai|mistral|cohere|huggingface|voyage|elevenlabs)"/ && !/DEFAULT_NAME/ {
                printf "%s:%d:%s\n", FILENAME, FNR, $0
            }
        ' || true
)"
violation 'vendor providerOptions namespace key spelled outside the package options.rs' "$rule8b_hits"

# ── Rule 9: no operation-level retry inside aimux-providers/src ──────────────
#
# Core owns `prepare_retries`. Providers run their poll / download stages with
# the bounded helpers of `shared::poll`, and never re-submit a job.
rule9_hits="$(
    grep -rnE 'prepare_retries|aimux_core::retry|use aimux_core::\{[^}]*\bretry\b' \
        aimux-providers/src --include='*.rs' || true
)"
violation 'prepare_retries / aimux_core::retry used in aimux-providers/src (exceptions: none)' "$rule9_hits"

# ── Rule 10: the single-modality vendor packages read their settings through the model config ─
#
# No `XxxConfig` / `api_key_source` / `from_env` / `with_base_url` / `body_overrides`,
# and the namespace keys (`lmnt`, `hume`, `deepgram`, `cartesia`, `stability`,
# `aws_polly`, `prodia`, `luma`, `klingai`, `replicate`, `fal`, `recraft`,
# `gladia`, `blackForestLabs`, `assemblyai`, `revai`, `linkup`, `jina`,
# `google_pse`) are quoted only in the package's `options.rs` (comments, unit
# tests and the `DEFAULT_NAME` provider-name constants aside).
single_family='serper prodia deepgram you_com luma lmnt klingai dataforseo replicate fal recraft aws_polly jina_ai gladia tavily linkup tinyfish black_forest_labs assemblyai revai hume cartesia searxng parallel_ai firecrawl runwayml exa_ai stability google_pse'
single_paths=''
for name in $single_family; do
    single_paths="$single_paths aimux-providers/src/$name.rs"
    [[ -d "aimux-providers/src/$name" ]] && single_paths="$single_paths aimux-providers/src/$name"
done
rule10_hits="$(
    # shellcheck disable=SC2086
    grep -rnE '\b(SerperConfig|ProdiaConfig|DeepgramConfig|YouComConfig|LumaConfig|LMNTConfig|KlingAIConfig|DataforseoConfig|ReplicateConfig|FalConfig|RecraftConfig|AwsPollyConfig|JinaAiConfig|GladiaConfig|TavilyConfig|LinkupConfig|TinyfishConfig|BlackForestLabsConfig|AssemblyAIConfig|RevaiConfig|HumeConfig|CartesiaConfig|SearxngConfig|ParallelAiConfig|FirecrawlConfig|RunwaymlConfig|ExaAiConfig|StabilityConfig|GooglePseConfig)\b|body_overrides|api_key_source|from_env|with_base_url' \
        $single_paths --include='*.rs' || true
)"
violation 'single-modality vendor packages read builder-era config state' "$rule10_hits"
rule10b_hits="$(
    # Non-test, non-comment lines of every file but the options helpers.
    # shellcheck disable=SC2086
    find $single_paths -name '*.rs' ! -name options.rs -print0 |
        xargs -0 awk '
            FNR == 1 { in_tests = 0 }
            /^#\[cfg\(test\)\]/ { in_tests = 1 }
            in_tests { next }
            /^[[:space:]]*\/\// { next }
            /"(lmnt|hume|deepgram|cartesia|stability|aws_polly|prodia|luma|klingai|replicate|fal|recraft|gladia|blackForestLabs|assemblyai|revai|linkup|jina|google_pse)"/ && !/DEFAULT_NAME/ {
                printf "%s:%d:%s\n", FILENAME, FNR, $0
            }
        ' || true
)"
violation 'single-modality providerOptions namespace key spelled outside the package options.rs' "$rule10b_hits"

if [[ "$status" -eq 0 ]]; then
    echo 'provider boundaries: ok'
fi
exit "$status"
