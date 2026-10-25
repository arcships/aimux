/* Deterministic native double. Exercises cgo ownership and callback delivery;
 * does not implement Rust ops or prove provider behavior. */
#include "aimux.h"
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define BASE UINT64_C(9007199254740993)
#define SLOTS 32768
struct entry { int live, aborted, polled, input_done; uint64_t abort; };
struct aimux_error { int32_t code; char *json; };
static struct entry entries[SLOTS];
static uint64_t allocated;
static int strings, errors;
static int waiting_calls, waiting_streams, waiting_pushes, waiting_polls;
static pthread_mutex_t lock = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t changed = PTHREAD_COND_INITIALIZER;

static struct entry *entry(uint64_t id) {
    if (id < BASE || id - BASE >= allocated) return NULL;
    return &entries[id - BASE];
}
static int live(uint64_t id) {
    struct entry *e = entry(id);
    return e && e->live;
}
static char *owned(const char *text) {
    char *p = strdup(text);
    ++strings;
    return p;
}
static aimux_error_t *failure(int32_t code, const char *kind, const char *name) {
    aimux_error_t *e = malloc(sizeof(*e));
    char buf[1024];
    snprintf(buf, sizeof(buf), "{\"kind\":\"%s\",\"code\":%d,\"message\":\"fixture error\",\"error\":{\"name\":\"%s\",\"isRetryable\":true,\"data\":null,\"nested\":{\"kept\":7}},\"future\":true}", kind, code, name);
    e->code = code;
    e->json = strdup(buf);
    ++errors;
    return e;
}
static uint64_t new_entry(uint64_t abort_id) {
    if (allocated == SLOTS) abort();
    uint64_t id = BASE + allocated++;
    *entry(id) = (struct entry){.live=1, .abort=abort_id};
    return id;
}
static int cancelled(uint64_t id) {
    struct entry *e = entry(id);
    return e && e->aborted;
}
static void wait_abort(uint64_t id) {
    while (!cancelled(id)) pthread_cond_wait(&changed, &lock);
}
uint32_t aimux_abi_version(void) { return AIMUX_ABI_VERSION; }
aimux_error_t *aimux_manifest_json(char **out) {
    char buf[256];
    pthread_mutex_lock(&lock);
    snprintf(buf, sizeof(buf), "{\"abiVersion\":%u,\"opsVersion\":\"" AIMUX_OPS_VERSION "\",\"digest\":\""
             AIMUX_MANIFEST_DIGEST "\",\"fixture\":true}", (unsigned)AIMUX_ABI_VERSION);
    *out = owned(buf);
    pthread_mutex_unlock(&lock);
    return NULL;
}
aimux_error_t *aimux_call(uint64_t id, const char *op, const char *params, uint64_t abort_id, char **out) {
    pthread_mutex_lock(&lock);
    *out = NULL;
    aimux_error_t *err = NULL;
    if (id && !live(id)) err = failure(203, "binding", "InvalidHandle");
    else if (!strcmp(op, AIMUX_OP_ABORT_CREATE) || !strcmp(op, "test.create") || !strcmp(op, AIMUX_OP_TRANSCRIPTION_START) || !strcmp(op, AIMUX_OP_COMPOSITE_ROUTER)) {
        char buf[128];
        uint64_t created = new_entry(abort_id);
        if (!strcmp(op, AIMUX_OP_TRANSCRIPTION_START) || !strcmp(op, AIMUX_OP_COMPOSITE_ROUTER))
            snprintf(buf, sizeof(buf), "\"%llu\"", (unsigned long long)created);
        else snprintf(buf, sizeof(buf), "{\"handle\":\"%llu\"}", (unsigned long long)created);
        *out = owned(buf);
    } else if (!strcmp(op, "test.wait")) {
        ++waiting_calls;
        wait_abort(abort_id);
        --waiting_calls;
        err = failure(13, "aimux", "Aborted");
    } else if (!strcmp(op, "test.error")) err = failure(11, "aimux", "AI_APICallError");
    else if (!strcmp(op, "test.recording")) err = failure(100, "recording", "Init");
    else if (!strcmp(op, "test.binding")) err = failure(204, "binding", "ReentrantCall");
    else if (!strcmp(op, "test.protocol")) err = failure(303, "protocol", "ConcurrentConsumer");
    else if (!strcmp(op, "test.stats")) {
        unsigned int count = 0;
        for (uint64_t i=0; i<allocated; ++i) count += entries[i].live;
        char buf[256];
        snprintf(buf, sizeof(buf), "{\"live\":%u,\"strings\":%d,\"errors\":%d,\"waitingCalls\":%d,\"waitingStreams\":%d,\"waitingPushes\":%d,\"waitingPolls\":%d}", count, strings, errors, waiting_calls, waiting_streams, waiting_pushes, waiting_polls);
        *out = owned(buf);
    } else if (!strcmp(op, AIMUX_OP_OPERATION_INPUT_DONE)) {
        entry(id)->input_done = 1;
        *out = owned("{}");
    } else if (!strcmp(op, AIMUX_OP_OPERATION_STATUS)) *out = owned("{\"state\":\"running\"}");
    else if (!strcmp(op, "test.echo")) *out = owned(params);
    else if (!strcmp(op, AIMUX_OP_TEXT_GENERATE)) *out = owned("{\"text\":\"fixture\"}");
    else err = failure(300, "protocol", "InvalidOp");
    pthread_mutex_unlock(&lock);
    return err;
}
aimux_error_t *aimux_stream(uint64_t id, const char *op, const char *params, uint64_t abort_id, aimux_event_cb cb, void *context) {
    (void)params;
    pthread_mutex_lock(&lock);
    if (!live(id) || !strcmp(op, "test.reject")) {
        aimux_error_t *err = failure(!live(id) ? 203 : 301, !live(id) ? "binding" : "protocol", !live(id) ? "InvalidHandle" : "InvalidParams");
        pthread_mutex_unlock(&lock);
        return err;
    }
    pthread_mutex_unlock(&lock);
    if (!strcmp(op, "test.null")) cb(NULL, context);
    else if (!strcmp(op, "test.bad")) cb("{bad", context);
    else if (!strcmp(op, "test.sequence")) {
        cb("{\"type\":\"event\",\"seq\":\"1\",\"event\":{}}", context);
        cb("{\"type\":\"event\",\"seq\":\"1\",\"event\":{}}", context);
    } else if (!strcmp(op, "test.large")) {
        char *big = malloc((1<<20)+4098);
        memset(big, 'x', (1<<20)+4097); big[(1<<20)+4097]=0;
        cb(big, context); free(big);
    } else if (!strcmp(op, "test.limit") || !strcmp(op, "test.eventTooLarge")) {
        size_t length = (1u << 20) + (!strcmp(op, "test.eventTooLarge") ? 1 : 0);
        const char *prefix = "{\"type\":\"event\",\"seq\":\"1\",\"event\":\"";
        char *buf = malloc(length + strlen(prefix) + 2);
        strcpy(buf, prefix);
        memset(buf + strlen(prefix), 'x', length-2);
        strcpy(buf + strlen(prefix) + length-2, "\"}");
        cb(buf, context);
        free(buf);
    } else if (!strcmp(op, "test.noend")) return NULL;
    else if (!strcmp(op, "test.terminalError")) {
        cb("{\"type\":\"end\",\"seq\":\"1\",\"outcome\":\"error\",\"error\":{\"kind\":\"aimux\",\"code\":11,\"message\":\"terminal\",\"error\":{\"name\":\"AI_APICallError\",\"isRetryable\":false}}}", context);
        return NULL;
    } else if (!strcmp(op, "test.waitStream")) {
        pthread_mutex_lock(&lock);
        ++waiting_streams;
        wait_abort(abort_id);
        --waiting_streams;
        pthread_mutex_unlock(&lock);
    } else {
        int count = !strcmp(op, "test.flood") ? 1000 : 1;
        for (int i=0; i<count; ++i) {
            pthread_mutex_lock(&lock);
            int stopped = cancelled(abort_id);
            pthread_mutex_unlock(&lock);
            if (stopped) break;
            char buf[256];
            snprintf(buf, sizeof(buf), "{\"type\":\"event\",\"seq\":\"%d\",\"event\":{\"type\":\"text-delta\",\"text\":\"copied\"}}", i+1);
            cb(buf, context);
            memset(buf, 'x', sizeof(buf)); /* invalidate borrowed storage */
        }
    }
    pthread_mutex_lock(&lock);
    int stopped = cancelled(abort_id);
    pthread_mutex_unlock(&lock);
    cb(stopped ? "{\"type\":\"end\",\"seq\":\"1001\",\"outcome\":\"cancelled\"}" : "{\"type\":\"end\",\"seq\":\"1001\",\"outcome\":\"success\"}", context);
    return NULL;
}
aimux_error_t *aimux_operation_push(uint64_t id, const uint8_t *data, size_t size) {
    (void)data;
    pthread_mutex_lock(&lock);
    aimux_error_t *err = NULL;
    if (!live(id)) err = failure(203, "binding", "InvalidHandle");
    else if (cancelled(entry(id)->abort)) err = failure(13, "aimux", "Aborted");
    else if (entry(id)->input_done) err = failure(304, "protocol", "InvalidState");
    else if (size && data[0] == 255) {
        ++waiting_pushes;
        wait_abort(entry(id)->abort);
        --waiting_pushes;
        err = failure(13, "aimux", "Aborted");
    }
    pthread_mutex_unlock(&lock);
    return err;
}
aimux_error_t *aimux_operation_next_part(uint64_t id, int64_t timeout, char **out, int32_t *state) {
    pthread_mutex_lock(&lock);
    *out=NULL; *state=0;
    aimux_error_t *err=NULL;
    struct entry *e = entry(id);
    if (timeout < 0) err=failure(301, "protocol", "InvalidParams");
    else if (!live(id)) err=failure(203, "binding", "InvalidHandle");
    else if (cancelled(e->abort) && e->polled != 3) { e->polled=3; err=failure(13, "aimux", "Aborted"); }
    else if (e->polled == 3 || e->input_done) *state=2;
    else if (timeout == 0) *state=3;
    else if (!e->polled++) { *state=1; *out=owned("{\"type\":\"transcript\",\"text\":\"copied\"}"); }
    else { ++waiting_polls; wait_abort(e->abort); --waiting_polls; e->polled=3; err=failure(13, "aimux", "Aborted"); }
    pthread_mutex_unlock(&lock);
    return err;
}
void aimux_abort(uint64_t id) {
    pthread_mutex_lock(&lock);
    if (entry(id)) entry(id)->aborted=1;
    pthread_cond_broadcast(&changed);
    pthread_mutex_unlock(&lock);
}
void aimux_drop(uint64_t id) {
    pthread_mutex_lock(&lock);
    if (entry(id)) entry(id)->live=0;
    pthread_mutex_unlock(&lock);
}
void aimux_handle_finalizer(void *token) { aimux_drop((uint64_t)(uintptr_t)token); }
void aimux_free_string(char *p) {
    pthread_mutex_lock(&lock);
    if (p) { free(p); --strings; }
    pthread_mutex_unlock(&lock);
}
void aimux_error_free(aimux_error_t *e) {
    pthread_mutex_lock(&lock);
    if (e) { free(e->json); free(e); --errors; }
    pthread_mutex_unlock(&lock);
}
int32_t aimux_error_code(const aimux_error_t *e) { return e->code; }
char *aimux_error_message(const aimux_error_t *e) {
    (void)e;
    pthread_mutex_lock(&lock);
    char *s=owned("fixture error");
    pthread_mutex_unlock(&lock);
    return s;
}
char *aimux_error_json(const aimux_error_t *e) {
    pthread_mutex_lock(&lock);
    char *s=owned(e->json);
    pthread_mutex_unlock(&lock);
    return s;
}
