#define _POSIX_C_SOURCE 200809L
#include <errno.h>
#include "../../native/aimux_ops.h"
#include <pthread.h>
#include <stdatomic.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

struct aimux_error { int code; const char *json; };
static atomic_uint_fast64_t next_handle = 1;
static atomic_int aborted[4096], drops[4096];
static atomic_int produced, completed, pushes;
static void delay_ms(int ms) {
    struct timespec t = {ms / 1000, (ms % 1000) * 1000000L};
    /* The Dart VM signals its threads on Linux; resume after EINTR. */
    while (nanosleep(&t, &t) == -1 && errno == EINTR) {}
}
static char *copy(const char *value) {
    char *out = malloc(strlen(value) + 1);
    strcpy(out, value);
    return out;
}
static aimux_error_t *failure(void) {
    aimux_error_t *e = malloc(sizeof(*e));
    *e = (aimux_error_t){203,
      "{\"kind\":\"binding\",\"code\":203,\"message\":\"bad handle\","
      "\"error\":{\"name\":\"InvalidHandle\",\"futureField\":[1,2]}}"};
    return e;
}
uint32_t aimux_abi_version(void) {
#ifdef BAD_ABI
    return 2 << 16;
#else
    return 1 << 16;
#endif
}
aimux_error_t *aimux_manifest_json(char **out) {
#ifdef BAD_MANIFEST
    *out = copy("{\"abiVersion\":65536,\"opsVersion\":\"2.0\"}");
#else
    *out = copy("{\"abiVersion\":65536,\"opsVersion\":\"1.0\",\"manifestRevision\":\"fixture\","
        "\"ops\":[{\"name\":\"runtime.describe\"},{\"name\":\"text.stream\"}],"
        "\"handleKinds\":[\"Language\",\"Abort\"],\"errorCodes\":[]}");
#endif
    return NULL;
}
aimux_error_t *aimux_call(uint64_t handle, const char *op, const char *params,
                         uint64_t abort, char **out) {
    (void)abort;
    *out = NULL;
    if (!strcmp(op, "test.fail")) return failure();
    if (!strcmp(op, "abort.create") || !strcmp(op, "test.handle")) {
        char json[96];
        snprintf(json, sizeof(json), "{\"handle\":\"%llu\"}",
                 (unsigned long long)atomic_fetch_add(&next_handle, 1));
        *out = copy(json);
    } else if (!strcmp(op, "test.sleep")) {
        delay_ms(180);
        *out = copy("{\"ok\":true}");
    } else if (!strcmp(op, "test.stats")) {
        char json[200];
        snprintf(json, sizeof(json),
            "{\"produced\":%d,\"completed\":%d,\"drops\":%d,\"pushes\":%d}",
            atomic_load(&produced), atomic_load(&completed),
            handle < 4096 ? atomic_load(&drops[handle]) : 0, atomic_load(&pushes));
        *out = copy(json);
    } else if (!strcmp(op, "test.reset")) {
        atomic_store(&produced, 0);
        atomic_store(&completed, 0);
        *out = copy("{}");
    } else {
        *out = copy(params); /* Verify bytes, open fields and handle encoding. */
    }
    return NULL;
}
struct stream_args {
    const char *op;
    uint64_t abort;
    aimux_event_cb event;
    void *ctx;
};
static void *produce(void *data) {
    struct stream_args *a = data;
    if (!strcmp(a->op, "test.noTerminal")) return NULL;
    if (!strcmp(a->op, "test.oversize")) {
        char *frame = malloc(1048578);
        memset(frame, 'x', 1048577);
        frame[1048577] = 0;
        a->event(frame, a->ctx);
        free(frame);
    } else if (!strcmp(a->op, "test.badFrame")) {
        a->event("{", a->ctx);
    } else {
        for (int i = 0; i < 1000 && !atomic_load(&aborted[a->abort]); i++) {
            char frame[180];
            snprintf(frame, sizeof(frame),
                "{\"type\":\"event\",\"seq\":\"%d\",\"event\":{\"type\":\"text-delta\",\"text\":\"%d\"}}", i, i);
            atomic_fetch_add(&produced, 1);
            a->event(frame, a->ctx);
        }
    }
    const char *end = !strcmp(a->op, "test.terminalError") ?
        "{\"type\":\"end\",\"seq\":\"1000\",\"outcome\":\"error\",\"error\":{\"kind\":\"aimux\",\"code\":11,\"message\":\"offline\",\"error\":{\"name\":\"AI_APICallError\",\"isRetryable\":false}}}" :
        atomic_load(&aborted[a->abort]) ?
        "{\"type\":\"end\",\"seq\":\"1000\",\"outcome\":\"cancelled\"}" :
        "{\"type\":\"end\",\"seq\":\"1000\",\"outcome\":\"success\"}";
    a->event(end, a->ctx);
    atomic_fetch_add(&completed, 1);
    return NULL;
}
aimux_error_t *aimux_stream(uint64_t handle, const char *op, const char *params,
                           uint64_t abort, aimux_event_cb event, void *ctx) {
    (void)handle; (void)params;
    if (!strcmp(op, "test.reject")) return failure();
    struct stream_args a = {op, abort, event, ctx};
    pthread_t thread;
    pthread_create(&thread, NULL, produce, &a);
    pthread_join(thread, NULL);
    return NULL;
}
aimux_error_t *aimux_operation_push(uint64_t operation, const uint8_t *data, size_t len) {
    (void)operation;
    if (len && !data) return failure();
    atomic_fetch_add(&pushes, 1);
    return NULL;
}
aimux_error_t *aimux_operation_next_part(uint64_t operation, int64_t timeout,
                                        char **out, int32_t *state) {
    (void)operation;
    *out = NULL; *state = 0;
    if (timeout == 99) return failure();
    if (timeout == 1) { *out = copy("{\"type\":\"transcript\"}"); *state = 1; }
    else if (timeout == 2) *state = 2;
    else *state = 3;
    return NULL;
}
void aimux_abort(uint64_t handle) {
    if (handle < 4096) atomic_store(&aborted[handle], 1);
}
void aimux_drop(uint64_t handle) {
    if (handle && handle < 4096) atomic_fetch_add(&drops[handle], 1);
}
void aimux_free_string(char *value) { free(value); }
void aimux_error_free(aimux_error_t *value) { free(value); }
int32_t aimux_error_code(const aimux_error_t *value) { return value->code; }
#ifndef OMIT_ERROR_MESSAGE
char *aimux_error_message(const aimux_error_t *value) { (void)value; return copy("bad handle"); }
#endif
char *aimux_error_json(const aimux_error_t *value) { return copy(value->json); }

#ifndef OMIT_HANDLE_FINALIZER
void aimux_handle_finalizer(void *token) { aimux_drop((uint64_t)(uintptr_t)token); }
#endif
