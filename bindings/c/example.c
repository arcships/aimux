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

    // 3.6 Provider handle (RFC-0027): create a provider handle, list its
    // models at runtime, then build a model from a discovered id. NULL
    // api_key reads DEEPSEEK_API_KEY from the env.
    uint64_t p_handle = extract_handle(aimux_provider_handle_new("deepseek", NULL, NULL));
    if (p_handle != 0) {
        printf("Provider handle: handle=%lu\n", (unsigned long)p_handle);

        // List models (runtime discovery + anya2a enrichment). Returns a JSON
        // array of ResolvedModel.
        char *models_json = aimux_provider_list_models(p_handle);
        if (models_json) {
            printf("Models: %s\n", models_json);
            aimux_free_string(models_json);
        }

        // Build a language model from a discovered id. Returns a model handle
        // usable with aimux_generate_text etc. (same envelope as
        // aimux_provider_new).
        uint64_t m_handle = extract_handle(aimux_provider_model(p_handle, "deepseek-chat"));
        if (m_handle != 0) {
            printf("Model from provider handle: handle=%lu\n", (unsigned long)m_handle);
            aimux_drop_handle(m_handle);
        }

        aimux_drop_handle(p_handle);
    }

    // 4. Recording + mock replay (RFC-0023): opt-in recording of the next
    // call, then replay the recorded response WITHOUT a real API call.
    printf("\n--- Recording (RFC-0023) ---\n");
    aimux_init_recording("./recordings");
    char *rec_result = aimux_generate_text(handle, "\"What is 2+2?\"", NULL);
    if (rec_result) {
        printf("Recorded call result: %s\n", rec_result);
        aimux_free_string(rec_result);
    }
    aimux_recording_flush();
    aimux_recording_stop();
    printf("Recording flushed to ./recordings/recordings.jsonl\n");

    printf("\n--- Mock replay (no real API) ---\n");
    FILE *rf = fopen("./recordings/recordings.jsonl", "rb");
    if (rf) {
        fseek(rf, 0, SEEK_END);
        long n = ftell(rf);
        fseek(rf, 0, SEEK_SET);
        char *buf = malloc((size_t)n + 1);
        if (buf) {
            fread(buf, 1, (size_t)n, rf);
            buf[n] = '\0';
            uint64_t mock = extract_handle(aimux_mock_replay_new(buf));
            if (mock != 0) {
                printf("Mock model created: handle=%lu\n", (unsigned long)mock);
                char *mock_result = aimux_generate_text(mock, "\"What is 2+2?\"", NULL);
                if (mock_result) {
                    printf("Mock result (recorded, no network): %s\n", mock_result);
                    aimux_free_string(mock_result);
                }
                aimux_drop_handle(mock);
            }
            free(buf);
        }
        fclose(rf);
    }

    // 5. Cleanup
    aimux_drop_handle(handle);
    printf("Handle dropped\n");

    return 0;
}
