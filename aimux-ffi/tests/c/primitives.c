#define _POSIX_C_SOURCE 200809L
#include "test_support.h"
#include <pthread.h>
#include <stdatomic.h>
#include <time.h>

static char sentinel;
static void rejection_callback(const char *frame, void *context) {
    (void)frame;
    ++*(unsigned *)context;
}

static void invalid_inputs(void) {
    const char invalid_utf8[] = {(char)0xff, 0};
    char *out = &sentinel;
    check_error(aimux_manifest_json(NULL), 200);
    check_error(aimux_call(0, "runtime.describe", "{}", 0, NULL), 200);
    check_error(aimux_call(0, NULL, "{}", 0, &out), 200);
    assert(out == NULL);
    out = &sentinel;
    check_error(aimux_call(0, "runtime.describe", NULL, 0, &out), 200);
    assert(out == NULL);
    out = &sentinel;
    check_error(aimux_call(0, invalid_utf8, "{}", 0, &out), 201);
    assert(out == NULL);
    out = &sentinel;
    check_error(aimux_call(0, "runtime.describe", invalid_utf8, 0, &out), 201);
    assert(out == NULL);
    out = &sentinel;
    check_error(aimux_call(0, "runtime.describe", "{", 0, &out), 202);
    assert(out == NULL);
    out = &sentinel;
    check_error(aimux_call(0, "not.an.op", "{}", 0, &out), 300);
    assert(out == NULL);

    int32_t state = 99;
    out = &sentinel;
    check_error(aimux_operation_next_part(UINT64_MAX, 0, &out, &state), 203);
    assert(out == NULL && state == 0);
    state = 99;
    check_error(aimux_operation_next_part(UINT64_MAX, 0, NULL, &state), 200);
    assert(state == 0);
    out = &sentinel;
    check_error(aimux_operation_next_part(UINT64_MAX, 0, &out, NULL), 200);
    assert(out == NULL);
    check_error(aimux_operation_push(UINT64_MAX, NULL, 1), 200);
    /* Empty input is valid at the pointer boundary, then fails handle lookup. */
    check_error(aimux_operation_push(UINT64_MAX, NULL, 0), 203);
    uint8_t byte = 0;
    aimux_error_t *error = aimux_operation_push(UINT64_MAX, &byte, SIZE_MAX);
    assert(error != NULL);
    /* Rejection may happen at length validation or handle lookup, never a read. */
    int32_t code = aimux_error_code(error);
    assert(code == 203 || code == 301 || code == 305);
    check_error(error, code);

    unsigned callbacks = 0;
    check_error(aimux_stream(UINT64_MAX, "text.stream", "{}", 0,
                             rejection_callback, &callbacks), 203);
    assert(callbacks == 0);
    check_error(aimux_stream(UINT64_MAX, "text.stream", "{}", 0, NULL, NULL), 200);
}

static void wrong_kind(void) {
    uint64_t handle = create_handle("abort.create", 0, "{}");
    const char *ops[] = {"model.info", "provider.model", "registry.provider",
                         "text.generate", "embedding.embed", "operation.status"};
    for (size_t i = 0; i < sizeof ops / sizeof *ops; ++i) {
        char *out = &sentinel;
        aimux_error_t *error = aimux_call(handle, ops[i], "{}", 0, &out);
        assert(error != NULL);
        char *envelope = aimux_error_json(error);
        assert(envelope != NULL);
        assert(strstr(envelope, "\"expected\"") != NULL);
        assert(strstr(envelope, "\"actual\"") != NULL);
        aimux_free_string(envelope);
        check_error(error, 203);
        assert(out == NULL);
    }
    aimux_abort(handle);
    aimux_drop(handle);
    aimux_drop(handle);
    char *out = &sentinel;
    check_error(aimux_call(handle, "abort.signal", "{}", 0, &out), 203);
    assert(out == NULL);
}

struct race { uint64_t handle; atomic_bool start; };
static void *drop_thread(void *arg) {
    struct race *race = arg;
    while (!atomic_load(&race->start)) { }
    aimux_drop(race->handle);
    return NULL;
}
static void drop_races(void) {
    for (unsigned i = 0; i < 100; ++i) {
        struct race race = {create_handle("abort.create", 0, "{}"), ATOMIC_VAR_INIT(0)};
        pthread_t thread;
        assert(pthread_create(&thread, NULL, drop_thread, &race) == 0);
        atomic_store(&race.start, 1);
        char *out = &sentinel;
        aimux_error_t *error = aimux_call(race.handle, "abort.signal", "{}", 0, &out);
        if (error) { check_error(error, 203); assert(out == NULL); }
        else { assert(out != NULL); aimux_free_string(out); }
        aimux_abort(race.handle);
        aimux_drop(race.handle);
        assert(pthread_join(thread, NULL) == 0);
    }
}

struct stream_context {
    uint64_t abort_handle;
    char *last_frame;
    unsigned frames, ends;
    int returned;
};
static void copy_frame(const char *frame, void *arg) {
    struct stream_context *ctx = arg;
    assert(!ctx->returned && frame != NULL && ctx->ends == 0);
    free(ctx->last_frame);
    ctx->last_frame = strdup(frame);
    assert(ctx->last_frame != NULL);
    ++ctx->frames;
    /* Match compact or spaced JSON without retaining a borrowed pointer. */
    const char *type = strstr(frame, "\"type\"");
    assert(type != NULL);
    const char *value = strchr(type + 6, ':');
    assert(value != NULL);
    ++value;
    while (*value == ' ' || *value == '\n' || *value == '\r' || *value == '\t') ++value;
    if (strncmp(value, "\"end\"", 5) == 0) ++ctx->ends;
    aimux_abort(ctx->abort_handle); /* specifically permitted from callback */
}
static void stream_lifetime(void) {
    const char *provider = getenv("AIMUX_C_TEST_PROVIDER_PARAMS");
    const char *model = getenv("AIMUX_C_TEST_MODEL_PARAMS");
    const char *request = getenv("AIMUX_C_TEST_STREAM_PARAMS");
    if (!provider || !model || !request) {
        puts("SKIP accepted stream lifetime: set AIMUX_C_TEST_{PROVIDER,MODEL,STREAM}_PARAMS");
        return;
    }
    uint64_t p = create_handle("provider.create", 0, provider);
    uint64_t m = create_handle("provider.model", p, model);
    struct stream_context ctx = {create_handle("abort.create", 0, "{}"), NULL, 0, 0, 0};
    aimux_error_t *error = aimux_stream(m, "text.stream", request, ctx.abort_handle, copy_frame, &ctx);
    if (error) {
        char *message = aimux_error_message(error);
        fprintf(stderr, "stream rejected: %s\n", message);
        aimux_free_string(message);
        aimux_error_free(error);
        abort();
    }
    ctx.returned = 1;
    assert(ctx.frames > 0 && ctx.ends == 1 && ctx.last_frame != NULL);
    unsigned frames = ctx.frames;
    struct timespec delay = {0, 50000000};
    nanosleep(&delay, NULL);
    assert(ctx.frames == frames);
    free(ctx.last_frame);
    aimux_drop(ctx.abort_handle);
    aimux_drop(m);
    aimux_drop(p);
    puts("PASS accepted stream lifetime / callback abort / one terminal");
}
int main(void) {
    assert(aimux_abi_version() == (1u << 16));
    char *manifest = &sentinel;
    assert(aimux_manifest_json(&manifest) == NULL);
    assert(manifest != NULL && strstr(manifest, "opsVersion") != NULL);
    aimux_free_string(manifest);
    aimux_free_string(NULL);
    aimux_drop(0);
    aimux_handle_finalizer(NULL);
    invalid_inputs();
    puts("PASS C11 manifest / errors / sentinels / UTF-8 / JSON / size boundaries");
    fflush(stdout);
    wrong_kind();
    drop_races();
    stream_lifetime();
    puts("PASS C11 primitives / errors / sentinels / wrong kinds / 100 drop races");
    return 0;
}
