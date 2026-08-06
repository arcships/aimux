// example.c — C example using aimux-ffi
//
// Build: gcc -o example example.c -I../../aimux-ffi -L../../target/debug -laimux_ffi -lpthread -ldl -lm
// Run:   OPENAI_API_KEY=sk-... ./example

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "aimux-ffi.h"

// Callback for streaming
static void on_part(const char *json) {
    // In a real app, parse the StreamPart JSON and extract text deltas.
    // Here we just print the raw JSON.
    printf("PART: %s\n", json);
}

static void on_done(void) {
    printf("\n[stream done]\n");
}

static void on_error(const char *err_json) {
    fprintf(stderr, "STREAM ERROR: %s\n", err_json);
}

// Parse a constructor's JSON result (`{"handle":<u64>}` on success,
// `{"error":"..."}` on failure). Frees the string. On failure, prints the
// detailed engine error to stderr (the whole point of the JSON envelope vs.
// the old bare-u64-handle ABI, which had no room for one) and returns 0.
static uint64_t extract_handle(char *json) {
    if (!json) {
        fprintf(stderr, "constructor returned null\n");
        return 0;
    }
    uint64_t handle = 0;
    if (strstr(json, "\"error\":")) {
        fprintf(stderr, "construction failed: %s\n", json);
    } else {
        const char *h = strstr(json, "\"handle\":");
        if (h) handle = strtoull(h + strlen("\"handle\":"), NULL, 10);
    }
    aimux_free_string(json);
    return handle;
}

int main(void) {
    const char *api_key = getenv("OPENAI_API_KEY");
    if (!api_key) {
        fprintf(stderr, "Please set OPENAI_API_KEY\n");
        return 1;
    }

    // 1. Create model
    uint64_t handle = extract_handle(aimux_openai_new(api_key, "gpt-4o-mini"));
    if (handle == 0) {
        fprintf(stderr, "Failed to create model\n");
        return 1;
    }
    printf("Model created: handle=%lu\n", (unsigned long)handle);

    // 2. Generate text (non-streaming)
    const char *prompt = "\"Explain Rust ownership in one sentence.\"";
    char *result = aimux_generate_text(handle, prompt, NULL);
    if (result) {
        printf("Result: %s\n", result);
        aimux_free_string(result);
    }

    // 3. Stream text
    printf("\n--- Streaming ---\n");
    aimux_stream_text(handle, "\"Write a haiku about Rust.\"", NULL,
                      on_part, on_done, on_error);

    // 3.1 Generate text as OpenAI Chat Completion (RFC-0026)
    printf("\n--- OpenAI-compatible output ---\n");
    char *oai_result = aimux_generate_text_as_openai(handle, prompt, NULL);
    if (oai_result) {
        printf("ChatCompletion: %s\n", oai_result);
        aimux_free_string(oai_result);
    }

    // 3.2 Stream text as OpenAI Chat Completion chunks
    printf("\n--- OpenAI-compatible streaming ---\n");
    aimux_stream_text_as_openai(handle, "\"Write a haiku about Rust.\"", NULL,
                                on_part, on_done, on_error);

    // 3.5 Registry provider (RFC-0017 phase 4): construct DeepSeek via the
    // provider registry. NULL api_key reads DEEPSEEK_API_KEY from the env.
    uint64_t ds_handle = extract_handle(aimux_provider_new("deepseek", NULL, "deepseek-chat", NULL));
    if (ds_handle != 0) {
        printf("DeepSeek (registry): handle=%lu\n", (unsigned long)ds_handle);
        aimux_drop_handle(ds_handle);
    }
    // On failure extract_handle already printed the detailed engine error
    // (e.g. "is DEEPSEEK_API_KEY set?" is now part of that message).

    // 4. Cleanup
    aimux_drop_handle(handle);
    printf("Handle dropped\n");

    return 0;
}
