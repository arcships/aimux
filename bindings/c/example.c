// example.c — C example using aimux-ffi
//
// Build: gcc -o example example.c -I../../aimux-ffi -L../../target/release -laimux_ffi -lpthread -ldl -lm
// Run:   OPENAI_API_KEY=sk-... ./example
//
// Every fallible call returns aimux_error_t * (NULL = success, result in
// the trailing out-param). Read its unified code and aimux_error_* getters,
// then release it with aimux_error_free(). Codes 200..206 mean the C call
// itself was malformed (NULL argument, malformed JSON text, dead handle).

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "aimux-error.h"
#include "aimux-ffi.h"

static void print_owned(const char *label, char *s) {
    if (s) {
        fprintf(stderr, "  %s: %s\n", label, s);
        aimux_free_string(s);
    }
}

// Print the failure, release the returned error, and hand back the callee's
// retry verdict: 1 when retrying may help, 0 when it will not.
//
// Payload getters answer only under their own code (NULL / -1 otherwise) —
// read them inside the matching case.
static int report(const char *what, aimux_error_t *e) {
    int32_t code = aimux_error_code(e);
    if (code >= AIMUX_E_FFI_NULL_POINTER && code <= AIMUX_E_FFI_CALLBACK_FAILURE) {
        // Not the model's fault: this call was malformed. Fix the call.
        char *message = aimux_error_message(e);
        fprintf(stderr, "%s: bad call: %s\n", what, message ? message : "");
        aimux_free_string(message);
        aimux_error_free(e);
        return 0;
    }
    char *message = aimux_error_message(e);
    int retryable = aimux_error_retryable(e);
    switch (code) {
    // Every HTTP-shaped failure arrives as AIMUX_E_API_CALL; classify on status.
    case AIMUX_E_API_CALL: {
        int status = aimux_error_status(e);
        int64_t retry_ms = aimux_error_retry_ms(e);
        if (status == 429) {
            // retry_ms is -1 when the 429 carried no retry-after header —
            // fall back to your own exponential backoff.
            if (retry_ms >= 0) {
                fprintf(stderr, "%s: rate limited, retry in %lldms: %s\n", what,
                        (long long)retry_ms, message);
            } else {
                fprintf(stderr, "%s: rate limited, no retry-after hint, back off "
                                "exponentially: %s\n",
                        what, message);
            }
        } else if (status == 401) {
            fprintf(stderr, "%s: auth (HTTP 401): %s\n", what, message);
        } else if (status == 404) {
            fprintf(stderr, "%s: model not found (HTTP 404): %s\n", what, message);
        } else if (status >= 0) {
            fprintf(stderr, "%s: HTTP %d: %s\n", what, status, message);
        } else {
            // status == -1: no HTTP response was ever observed — a missing API
            // key, an error built without a request, or a transport failure.
            // Which one it was is retryable, never the status: those shapes
            // both report -1 and disagree about whether a retry helps.
            fprintf(stderr, "%s: no HTTP response (%s): %s\n", what,
                    retryable ? "retryable" : "not retryable", message);
        }
        // Payload strings are NULL when the provider did not send them.
        print_owned("provider code", aimux_error_provider_code(e));
        print_owned("provider message", aimux_error_provider_message(e));
        print_owned("request id", aimux_error_request_id(e));
        break;
    }
    case AIMUX_E_TOKEN_EXPIRED:
        fprintf(stderr, "%s: token expired (HTTP %d): %s\n", what, aimux_error_status(e),
                message);
        break;
    case AIMUX_E_NO_SUCH_PROVIDER: {
        char *id = aimux_error_provider_id(e);
        fprintf(stderr, "%s: no provider \"%s\": %s\n", what, id ? id : "?", message);
        aimux_free_string(id);
        break;
    }
    case AIMUX_E_NO_SUCH_MODEL: {
        char *id = aimux_error_model_id(e);
        char *type = aimux_error_model_type(e);
        fprintf(stderr, "%s: no %s model \"%s\": %s\n", what, type ? type : "?", id ? id : "?",
                message);
        aimux_free_string(id);
        aimux_free_string(type);
        break;
    }
    default:
        fprintf(stderr, "%s: %s\n", what, message);
        break;
    }
    aimux_free_string(message);
    aimux_error_free(e);
    return retryable;
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

    aimux_error_t *err = NULL;

    // 1. Create model — failure: err != NULL, handle stays 0
    uint64_t handle = 0;
    if ((err = aimux_openai_new(api_key, "gpt-4o-mini", &handle))) {
        report("openai", err);
        return 1;
    }
    printf("Model created: handle=%lu\n", (unsigned long)handle);

    // 2. Generate text (non-streaming) — failure: err != NULL, result stays NULL
    const char *prompt = "\"Explain Rust ownership in one sentence.\"";
    char *result = NULL;
    if ((err = aimux_generate_text(handle, prompt, NULL, &result)) &&
        report("generate_text", err)) {
        // Retry on the callee's verdict, not on the status sentinel.
        if ((err = aimux_generate_text(handle, prompt, NULL, &result))) {
            report("generate_text (retry)", err);
        }
    }
    if (result) {
        printf("Result: %s\n", result);
        aimux_free_string(result);
    }

    // 3. Stream — no out-param; NULL after on_done
    printf("\n--- Streaming ---\n");
    if ((err = aimux_stream_text(handle, "\"Write a haiku about Rust.\"", NULL, on_part,
                                 on_done, NULL))) {
        report("stream", err);
    }

    // 3.1 OpenAI-compatible output
    printf("\n--- OpenAI-compatible output ---\n");
    char *oai_result = NULL;
    if ((err = aimux_generate_text_as_openai(handle, prompt, NULL, &oai_result))) {
        report("generate_text_as_openai", err);
    } else {
        printf("ChatCompletion: %s\n", oai_result);
        aimux_free_string(oai_result);
    }

    // 3.2 OpenAI-compatible stream
    printf("\n--- OpenAI-compatible streaming ---\n");
    if ((err = aimux_stream_text_as_openai(handle, "\"Write a haiku about Rust.\"", NULL,
                                           on_part, on_done, NULL))) {
        report("stream_as_openai", err);
    }

    // 3.5 Registry provider
    uint64_t ds_handle = 0;
    if ((err = aimux_provider_new("deepseek", NULL, "deepseek-chat", NULL, &ds_handle))) {
        report("provider deepseek", err);
    } else {
        printf("DeepSeek (registry): handle=%lu\n", (unsigned long)ds_handle);
        aimux_drop_handle(ds_handle);
    }

    // 3.6 Provider handle (RFC-0027)
    uint64_t p_handle = 0;
    if ((err = aimux_provider_handle_new("deepseek", NULL, NULL, &p_handle))) {
        report("provider_handle", err);
    } else {
        printf("Provider handle: handle=%lu\n", (unsigned long)p_handle);
        char *models_json = NULL;
        if ((err = aimux_provider_list_models(p_handle, &models_json))) {
            report("list_models", err);
        } else {
            printf("Models: %s\n", models_json);
            aimux_free_string(models_json);
        }
        uint64_t m_handle = 0;
        if ((err = aimux_provider_model(p_handle, "deepseek-chat", &m_handle))) {
            report("provider_model", err);
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
