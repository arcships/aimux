// example.c — C example using aimux-ffi
//
// Build: gcc -o example example.c -L../../target/debug -laimux_ffi -lpthread -ldl -lm
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

int main(void) {
    const char *api_key = getenv("OPENAI_API_KEY");
    if (!api_key) {
        fprintf(stderr, "Please set OPENAI_API_KEY\n");
        return 1;
    }

    // 1. Create model
    uint64_t handle = aimux_openai_new(api_key, "gpt-4o-mini");
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

    // 3.5 Registry provider (RFC-0017 phase 4): construct DeepSeek via the
    // provider registry. NULL api_key reads DEEPSEEK_API_KEY from the env.
    uint64_t ds_handle = aimux_provider_new("deepseek", NULL, "deepseek-chat", NULL);
    if (ds_handle != 0) {
        printf("DeepSeek (registry): handle=%lu\n", (unsigned long)ds_handle);
        aimux_drop_handle(ds_handle);
    } else {
        fprintf(stderr, "Failed to create DeepSeek via registry "
                        "(is DEEPSEEK_API_KEY set?)\n");
    }

    // 4. Cleanup
    aimux_drop_handle(handle);
    printf("Handle dropped\n");

    return 0;
}
