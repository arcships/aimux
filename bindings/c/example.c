// example.c — C example using aimux-ffi
//
// Build: gcc -o example example.c -I../../aimux-ffi -L../../target/release -laimux_ffi -lpthread -ldl -lm
// Run:   OPENAI_API_KEY=sk-... ./example
//
// Failure: return value is 0 / NULL. Details (optional) via AimuxError *err.

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "aimux-error.h"
#include "aimux-ffi.h"

// Print the failure, then release e->message (owned by the caller).
static void report(const char *what, AimuxError *e) {
    if (!e || e->code == AIMUX_OK) {
        fprintf(stderr, "%s: failed\n", what);
        return;
    }
    switch (e->code) {
    case AIMUX_E_RATE_LIMITED:
        fprintf(stderr, "%s: rate limited, retry in %lldms: %s\n", what,
                (long long)e->retry_ms, e->message);
        break;
    case AIMUX_E_AUTH:
    case AIMUX_E_TOKEN_EXPIRED:
        if (e->status >= 0) {
            fprintf(stderr, "%s: auth (HTTP %d): %s\n", what, e->status, e->message);
        } else {
            fprintf(stderr, "%s: auth: %s\n", what, e->message);
        }
        break;
    default:
        fprintf(stderr, "%s: %s\n", what, e->message);
        break;
    }
    aimux_free_string(e->message);
    aimux_free_string(e->error_value);
    e->message = NULL;
    e->error_value = NULL;
}

static void on_part(const char *json, void *stream_ctx) {
    (void)stream_ctx;
    printf("PART: %s\n", json);
}

static void on_done(void *stream_ctx) {
    (void)stream_ctx;
    printf("\n[stream done]\n");
}

int main(void) {
    const char *api_key = getenv("OPENAI_API_KEY");
    if (!api_key) {
        fprintf(stderr, "Please set OPENAI_API_KEY\n");
        return 1;
    }

    AimuxError err;
    aimux_error_clear(&err); // message must start NULL so it is safe to free

    // 1. Create model — failure: handle == 0
    uint64_t handle = aimux_openai_new(api_key, "gpt-4o-mini", &err);
    if (!handle) {
        report("openai", &err);
        return 1;
    }
    printf("Model created: handle=%lu\n", (unsigned long)handle);

    // 2. Generate text (non-streaming) — failure: result == NULL
    const char *prompt = "\"Explain Rust ownership in one sentence.\"";
    char *result = aimux_generate_text(handle, prompt, NULL, &err);
    if (!result) {
        report("generate_text", &err);
    } else {
        printf("Result: %s\n", result);
        aimux_free_string(result);
    }

    // 3. Stream — failure: return == 0; success: non-zero
    printf("\n--- Streaming ---\n");
    if (!aimux_stream_text(handle, "\"Write a haiku about Rust.\"", NULL, on_part,
                           on_done, NULL, &err)) {
        report("stream", &err);
    }

    // 3.1 OpenAI-compatible output
    printf("\n--- OpenAI-compatible output ---\n");
    char *oai_result = aimux_generate_text_as_openai(handle, prompt, NULL, &err);
    if (!oai_result) {
        report("generate_text_as_openai", &err);
    } else {
        printf("ChatCompletion: %s\n", oai_result);
        aimux_free_string(oai_result);
    }

    // 3.2 OpenAI-compatible stream
    printf("\n--- OpenAI-compatible streaming ---\n");
    if (!aimux_stream_text_as_openai(handle, "\"Write a haiku about Rust.\"", NULL,
                                     on_part, on_done, NULL, &err)) {
        report("stream_as_openai", &err);
    }

    // 3.5 Registry provider
    uint64_t ds_handle =
        aimux_provider_new("deepseek", NULL, "deepseek-chat", NULL, &err);
    if (!ds_handle) {
        report("provider deepseek", &err);
    } else {
        printf("DeepSeek (registry): handle=%lu\n", (unsigned long)ds_handle);
        aimux_drop_handle(ds_handle);
    }

    // 3.6 Provider handle (RFC-0027)
    uint64_t p_handle = aimux_provider_handle_new("deepseek", NULL, NULL, &err);
    if (!p_handle) {
        report("provider_handle", &err);
    } else {
        printf("Provider handle: handle=%lu\n", (unsigned long)p_handle);
        char *models_json = aimux_provider_list_models(p_handle, &err);
        if (!models_json) {
            report("list_models", &err);
        } else {
            printf("Models: %s\n", models_json);
            aimux_free_string(models_json);
        }
        uint64_t m_handle = aimux_provider_model(p_handle, "deepseek-chat", &err);
        if (!m_handle) {
            report("provider_model", &err);
        } else {
            printf("Model from provider handle: handle=%lu\n",
                   (unsigned long)m_handle);
            aimux_drop_handle(m_handle);
        }
        aimux_drop_handle(p_handle);
    }

    // 4. Cleanup
    aimux_drop_handle(handle);
    printf("Handle dropped\n");
    return 0;
}
