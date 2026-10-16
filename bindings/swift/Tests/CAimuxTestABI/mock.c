/* Test-only ABI implementation. No provider behavior is claimed by these tests. */
#include "TestABI.h"
#include "../../../../aimux-ffi/include/aimux.h"
#include <stdatomic.h>
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <unistd.h>
#include <pthread.h>

#define BASE UINT64_C(9007199254740992)
#define SLOTS 65536
static atomic_uint next_id = 1;
static atomic_int cancelled[SLOTS], drops[SLOTS];
static atomic_int active, strings, errors, aborts, emitted, main_calls;
static atomic_uint version = 1 << 16;
struct aimux_error { int32_t code; char *json; };
static unsigned index_of(uint64_t id) { return id > BASE && id - BASE < SLOTS ? (unsigned)(id - BASE) : 0; }
static void background(void) { if (pthread_main_np()) atomic_fetch_add(&main_calls, 1); }
static char *owned(const char *s) { atomic_fetch_add(&strings, 1); return strdup(s); }
static aimux_error_t *failure(int code, const char *json) {
    aimux_error_t *e = malloc(sizeof(*e)); e->code = code; e->json = strdup(json);
    atomic_fetch_add(&errors, 1); return e;
}
static const char *unknown = "{\"kind\":\"aimux\",\"code\":1,\"message\":\"未来错误 🌍\",\"error\":{\"name\":\"AI_FutureError\",\"opaque\":{\"newField\":[true,null,7]}},\"retryAfterMs\":123}";
uint64_t aimux_test_new_handle(void) { return BASE + atomic_fetch_add(&next_id, 1); }
int aimux_test_drops(uint64_t id) { return atomic_load(&drops[index_of(id)]); }
int aimux_test_active(void) { return atomic_load(&active); }
int aimux_test_strings(void) { return atomic_load(&strings); }
int aimux_test_errors(void) { return atomic_load(&errors); }
int aimux_test_aborts(void) { return atomic_load(&aborts); }
int aimux_test_emitted(void) { return atomic_load(&emitted); }
int aimux_test_main_calls(void) { return atomic_load(&main_calls); }
void aimux_test_version(uint32_t v) { atomic_store(&version, v); }
uint32_t aimux_abi_version(void) { return atomic_load(&version); }
aimux_error_t *aimux_manifest_json(char **out) {
    background(); *out = owned("{\"abiVersion\":65536,\"opsVersion\":\"1.0\",\"ops\":[]}"); return NULL;
}
aimux_error_t *aimux_call(uint64_t id, const char *op, const char *params, uint64_t abort, char **out) {
    background(); *out = NULL;
    if (!strcmp(op, "abort.create") || !strcmp(op, "provider.create")) {
        char value[128]; snprintf(value, sizeof(value), "{\"handle\":\"%llu\",\"kind\":\"%s\"}", (unsigned long long)aimux_test_new_handle(), !strcmp(op, "abort.create") ? "Abort" : "Provider");
        *out = owned(value); return NULL;
    }
    if (!strcmp(op, "operation.cancel")) {
        atomic_store(&cancelled[index_of(id)], 1); *out = owned("{}"); return NULL;
    }
    if (!strcmp(op, "error")) return failure(1, unknown);
    if (!strcmp(op, "slow") || !strcmp(op, "wait")) {
        atomic_fetch_add(&active, 1);
        if (!strcmp(op, "slow")) usleep(150000);
        else while (!atomic_load(&cancelled[index_of(abort)])) usleep(1000);
        atomic_fetch_sub(&active, 1);
        if (!strcmp(op, "wait")) return failure(1, unknown);
    }
    *out = owned(params); return NULL;
}
aimux_error_t *aimux_stream(uint64_t id, const char *op, const char *params, uint64_t abort,
                            aimux_event_cb cb, void *ctx) {
    (void)id; (void)params; background();
    if (!strcmp(op, "reject")) return failure(1, unknown);
    atomic_fetch_add(&active, 1);
    unsigned seq = 0;
    int malformed = !strcmp(op, "malformed"), missing = !strcmp(op, "missingEnd");
    int oversized = !strcmp(op, "oversized"), error = !strcmp(op, "terminalError") || !strcmp(op, "oneError");
    int count = (!strcmp(op, "many") || !strcmp(op, "wait")) ? 10000 : 4;
    if (error) count = !strcmp(op, "oneError") ? 1 : 0;
    if (!strcmp(op, "idle")) count = 1;
    if (!strcmp(op, "connecting")) count = 0;
    for (int i = 0; i < count && !atomic_load(&cancelled[index_of(abort)]); ++i) {
        if (malformed && i == 1) { cb("invalid", ctx); break; }
        if (oversized) {
            char *big = malloc(1048578); memset(big, 'x', 1048577); big[1048577] = 0;
            cb(big, ctx); free(big); break;
        }
        char frame[256]; snprintf(frame, sizeof(frame), "{\"type\":\"event\",\"seq\":\"%u\",\"event\":{\"type\":\"text-delta\",\"text\":\"你好 🌍\",\"id\":\"%d\"}}", seq++, i);
        atomic_fetch_add(&emitted, 1); cb(frame, ctx);
    }
    if (!strcmp(op, "idle") || !strcmp(op, "connecting")) {
        while (!atomic_load(&cancelled[index_of(abort)])) usleep(1000);
    }
    if (!missing) {
        char end[1024];
        if (error) snprintf(end, sizeof(end), "{\"type\":\"end\",\"seq\":\"%u\",\"outcome\":\"error\",\"error\":%s}", seq, unknown);
        else snprintf(end, sizeof(end), "{\"type\":\"end\",\"seq\":\"%u\",\"outcome\":\"%s\"}", seq,
                      atomic_load(&cancelled[index_of(abort)]) ? "cancelled" : "success");
        cb(end, ctx);
    }
    atomic_fetch_sub(&active, 1); return NULL;
}
aimux_error_t *aimux_operation_push(uint64_t id, const uint8_t *data, size_t length) {
    (void)data; background();
    if (length == 8) {
        atomic_fetch_add(&active, 1);
        while (!atomic_load(&cancelled[index_of(id)])) usleep(1000);
        atomic_fetch_sub(&active, 1);
    }
    return NULL;
}
aimux_error_t *aimux_operation_next_part(uint64_t id, int64_t timeout, char **out, int32_t *state) {
    background(); *out = NULL; *state = 0;
    if (timeout == -1) {
        atomic_fetch_add(&active, 1);
        while (!atomic_load(&cancelled[index_of(id)])) usleep(1000);
        atomic_fetch_sub(&active, 1); *state = 2;
    } else if (timeout == 0) *state = 3;
    else if (timeout == 1) *state = 2;
    else { *state = 1; *out = owned("{\"text\":\"hello\"}"); }
    return NULL;
}
void aimux_abort(uint64_t abort) { atomic_store(&cancelled[index_of(abort)], 1); atomic_fetch_add(&aborts, 1); }
void aimux_drop(uint64_t id) { background(); if (id) atomic_fetch_add(&drops[index_of(id)], 1); }
void aimux_handle_finalizer(void *token) { aimux_drop((uint64_t)(uintptr_t)token); }
void aimux_free_string(char *s) { if (s) { atomic_fetch_sub(&strings, 1); free(s); } }
void aimux_error_free(aimux_error_t *e) { if (e) { atomic_fetch_sub(&errors, 1); free(e->json); free(e); } }
int32_t aimux_error_code(const aimux_error_t *e) { return e->code; }
char *aimux_error_message(const aimux_error_t *e) { (void)e; return owned("future failure"); }
char *aimux_error_json(const aimux_error_t *e) { return owned(e->json); }
