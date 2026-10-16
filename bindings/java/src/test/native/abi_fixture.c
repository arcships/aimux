/* Test-only native implementation of the frozen ABI, with deterministic scripted calls. */
#include <stdint.h>
#include <stddef.h>
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <stdatomic.h>
#include <time.h>
#include "aimux.h"

struct aimux_error { int32_t code; const char *json; };
static _Atomic uint64_t next_id = 1000;
static _Atomic int signals[4096];
static _Atomic int drops[4096];
static _Atomic int entered;
static _Atomic int live_strings;
static _Atomic int live_errors;
static _Thread_local int callback_depth;
static _Atomic int drop_in_callback;
static _Atomic uint64_t last_handle;
static _Atomic uint64_t last_length;
static _Atomic int64_t last_timeout;
static _Atomic uint32_t abi_version = 65536;
static _Atomic int ops_mismatch;
static _Atomic int digest_mismatch;
static _Atomic int finalizers[4096];
static _Atomic int poll_state = 1;
static _Atomic int stream_mode;
static _Atomic int callback_count;
static _Atomic int provider_calls;

static char *owned(const char *text) {
    atomic_fetch_add(&live_strings, 1);
    return strdup(text);
}
static aimux_error_t *failure(int code, const char *json) {
    aimux_error_t *error = malloc(sizeof(*error));
    error->code = code; error->json = json;
    atomic_fetch_add(&live_errors, 1);
    return error;
}
static void pause_millis(long ms) {
    struct timespec delay = { ms / 1000, (ms % 1000) * 1000000 };
    nanosleep(&delay, NULL);
}
uint32_t aimux_abi_version(void) { return atomic_load(&abi_version); }
aimux_error_t *aimux_manifest_json(char **out) {
    char json[256];
    snprintf(json, sizeof json, "{\"abiVersion\":65536,\"opsVersion\":\"%s\",\"digest\":\"%s\",\"ops\":[]}",
        atomic_load(&ops_mismatch) ? "1.1" : AIMUX_OPS_VERSION,
        atomic_load(&digest_mismatch) ? "wrong-digest" : AIMUX_MANIFEST_DIGEST);
    *out = owned(json);
    return NULL;
}
aimux_error_t *aimux_call(uint64_t handle, const char *op, const char *params,
                           uint64_t abort, char **out) {
    *out = NULL;
    atomic_store(&last_handle, handle);
    if (!strcmp(op, "abort.create")) {
        char result[80];
        uint64_t id = atomic_fetch_add(&next_id, 1);
        snprintf(result, sizeof result, "{\"handle\":\"%llu\"}", (unsigned long long) id);
        *out = owned(result);
    } else if (!strcmp(op, "provider.create")) {
        atomic_fetch_add(&provider_calls, 1);
        *out = owned("{\"handle\":\"100\"}");
    } else if (!strcmp(op, "provider.model") || !strcmp(op, "provider.files")
               || !strcmp(op, "transcription.start")) {
        char result[80];
        uint64_t id = atomic_fetch_add(&next_id, 1);
        snprintf(result, sizeof result, "{\"handle\":\"%llu\"}", (unsigned long long) id);
        *out = owned(result);
    } else if (!strcmp(op, "test.wait")) {
        atomic_fetch_add(&entered, 1);
        while (abort < 4096 && !atomic_load(&signals[abort])) pause_millis(1);
        *out = owned("null");
    } else if (!strcmp(op, "test.fail")) {
        return failure(11, "{\"kind\":\"aimux\",\"code\":11,\"message\":\"busy\",\"error\":{\"name\":\"AI_APICallError\",\"isRetryable\":true,\"statusCode\":429,\"responseHeaders\":{\"x-request-id\":\"r1\"}},\"retryAfterMs\":0}");
    } else if (!strcmp(op, "test.binding")) {
        return failure(203, "{\"kind\":\"binding\",\"code\":203,\"message\":\"wrong handle\",\"error\":{\"name\":\"InvalidHandle\",\"expected\":[\"Language\"],\"actual\":\"Image\"}}");
    } else if (!strcmp(op, "test.protocol")) {
        return failure(300, "{\"kind\":\"protocol\",\"code\":300,\"message\":\"unknown op\",\"error\":{\"name\":\"InvalidOp\"}}");
    } else if (!strcmp(op, "text.generate") || !strcmp(op, "text.consume")) {
        *out = owned("{\"text\":\"你好 🌏\"}");
    } else {
        *out = owned(params);
    }
    return NULL;
}
static void emit(aimux_event_cb cb, void *context, const char *frame) {
    char buffer[1024];
    snprintf(buffer, sizeof buffer, "%s", frame);
    callback_depth++;
    cb(buffer, context);
    callback_depth--;
    atomic_fetch_add(&callback_count, 1);
    memset(buffer, 'X', strlen(buffer)); /* callback data expires immediately */
}
aimux_error_t *aimux_stream(uint64_t handle, const char *op, const char *params,
                            uint64_t abort, aimux_event_cb cb, void *context) {
    (void) handle; (void) op; (void) params;
    int mode = atomic_load(&stream_mode);
    if (mode == 1) return failure(300, "{\"kind\":\"protocol\",\"code\":300,\"message\":\"unknown op\",\"error\":{\"name\":\"InvalidOp\"}}");
    atomic_fetch_add(&entered, 1);
    if (mode == 6 || mode == 7) {
        if (mode == 6) {
            /* No NUL within the scan budget: do not scan or copy beyond this allocation. */
            char *frame = malloc((1 << 20) + 1);
            memset(frame, 'X', (1 << 20) + 1);
            cb(frame, context);
            free(frame);
        } else cb(NULL, context);
        return NULL;
    }
    emit(cb, context, "{\"type\":\"event\",\"seq\":\"1\",\"event\":{\"type\":\"text-delta\",\"id\":\"0\",\"delta\":\"你好 🌏\"}}");
    if (mode == 2) {
        while (!atomic_load(&signals[abort])) pause_millis(1);
    }
    if (mode == 3) {
        emit(cb, context, "{\"type\":\"end\",\"seq\":\"2\",\"outcome\":\"error\",\"error\":{\"kind\":\"aimux\",\"code\":12,\"message\":\"timeout\",\"error\":{\"name\":\"TimeoutError\"}}}");
    } else if (mode == 4) {
        /* Deliberately malformed native stream. */
    } else if (mode == 5) {
        emit(cb, context, "{\"type\":\"end\",\"seq\":\"2\",\"outcome\":\"success\"}");
        emit(cb, context, "{\"type\":\"end\",\"seq\":\"3\",\"outcome\":\"success\"}");
    } else {
        /* Keep emitting after cancellation to test the first-failure latch. */
        emit(cb, context, "{\"type\":\"event\",\"seq\":\"2\",\"event\":{\"type\":\"text-delta\",\"id\":\"0\",\"delta\":\"second\"}}");
        emit(cb, context, atomic_load(&signals[abort])
            ? "{\"type\":\"end\",\"seq\":\"3\",\"outcome\":\"cancelled\"}"
            : "{\"type\":\"end\",\"seq\":\"3\",\"outcome\":\"success\"}");
    }
    return NULL;
}
aimux_error_t *aimux_operation_push(uint64_t operation, const uint8_t *data, size_t length) {
    atomic_store(&last_handle, operation); atomic_store(&last_length, length);
    if (length && (!data || data[0] != 42)) abort();
    return NULL;
}
aimux_error_t *aimux_operation_next_part(uint64_t operation, int64_t timeout_ms,
                                         char **out_part, int32_t *out_state) {
    atomic_store(&last_handle, operation); atomic_store(&last_timeout, timeout_ms);
    *out_state = atomic_load(&poll_state); *out_part = NULL;
    if (*out_state == 1) *out_part = owned("{\"type\":\"transcript\",\"text\":\"你好\"}");
    if (*out_state == 0) return failure(304, "{\"kind\":\"protocol\",\"code\":304,\"message\":\"ended\",\"error\":{\"name\":\"InvalidState\"}}");
    return NULL;
}
void aimux_abort(uint64_t abort) { if (abort < 4096) atomic_fetch_add(&signals[abort], 1); }
void aimux_drop(uint64_t handle) {
    if (callback_depth) atomic_fetch_add(&drop_in_callback, 1);
    if (handle < 4096) atomic_fetch_add(&drops[handle], 1);
}
void aimux_handle_finalizer(void *token) {
    uint64_t id = (uint64_t)(uintptr_t)token;
    if (id < 4096) atomic_fetch_add(&finalizers[id], 1);
    aimux_drop(id);
}
void aimux_free_string(char *text) { if (text) { atomic_fetch_sub(&live_strings, 1); free(text); } }
void aimux_error_free(aimux_error_t *error) { if (error) { atomic_fetch_sub(&live_errors, 1); free(error); } }
int32_t aimux_error_code(const aimux_error_t *error) { return error ? error->code : 0; }
char *aimux_error_message(const aimux_error_t *error) { return error ? owned("fixture error") : NULL; }
char *aimux_error_json(const aimux_error_t *error) { return error ? owned(error->json) : NULL; }

/* Test controls live in their own interface, never in production JNA declarations. */
int fixture_entered(void) { return atomic_load(&entered); }
int fixture_live_strings(void) { return atomic_load(&live_strings); }
int fixture_live_errors(void) { return atomic_load(&live_errors); }
int fixture_drop_in_callback(void) { return atomic_load(&drop_in_callback); }
int fixture_finalizers(int id) { return atomic_load(&finalizers[id]); }
int fixture_drops(int id) { return atomic_load(&drops[id]); }
int fixture_signals(int id) { return atomic_load(&signals[id]); }
int fixture_callbacks(void) { return atomic_load(&callback_count); }
uint64_t fixture_last_handle(void) { return atomic_load(&last_handle); }
uint64_t fixture_last_length(void) { return atomic_load(&last_length); }
int64_t fixture_last_timeout(void) { return atomic_load(&last_timeout); }
int fixture_provider_calls(void) { return atomic_load(&provider_calls); }
void fixture_abi(uint32_t version) { atomic_store(&abi_version, version); }
void fixture_digest(int mismatch) { atomic_store(&digest_mismatch, mismatch); }
void fixture_ops(int mismatch) { atomic_store(&ops_mismatch, mismatch); }
void fixture_poll(int state) { atomic_store(&poll_state, state); }
void fixture_stream(int mode) { atomic_store(&stream_mode, mode); }
void fixture_reset(void) {
    for (int i = 0; i < 4096; i++) { atomic_store(&signals[i], 0); atomic_store(&drops[i], 0); atomic_store(&finalizers[i], 0); }
    atomic_store(&entered, 0); atomic_store(&callback_count, 0);
    atomic_store(&drop_in_callback, 0); atomic_store(&provider_calls, 0);
    atomic_store(&abi_version, 65536); atomic_store(&ops_mismatch, 0); atomic_store(&digest_mismatch, 0);
    atomic_store(&poll_state, 1); atomic_store(&stream_mode, 0);
}
