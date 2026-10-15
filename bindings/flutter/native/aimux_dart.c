/* Flutter-only transport. No provider logic or Dart callbacks on Rust threads. */
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#ifdef _WIN32
#include <windows.h>
#define EXPORT __declspec(dllexport)
typedef CRITICAL_SECTION mutex_t;
typedef CONDITION_VARIABLE cond_t;
static void mutex_init(mutex_t *m) { InitializeCriticalSection(m); }
static void mutex_destroy(mutex_t *m) { DeleteCriticalSection(m); }
static void lock(mutex_t *m) { EnterCriticalSection(m); }
static void unlock(mutex_t *m) { LeaveCriticalSection(m); }
static void cond_init(cond_t *c) { InitializeConditionVariable(c); }
static void cond_destroy(cond_t *c) { (void)c; }
static void cond_wait(cond_t *c, mutex_t *m) { SleepConditionVariableCS(c, m, INFINITE); }
static void wake(cond_t *c) { WakeAllConditionVariable(c); }
#else
#include <pthread.h>
#define EXPORT __attribute__((visibility("default")))
typedef pthread_mutex_t mutex_t;
typedef pthread_cond_t cond_t;
static void mutex_init(mutex_t *m) { pthread_mutex_init(m, NULL); }
static void mutex_destroy(mutex_t *m) { pthread_mutex_destroy(m); }
static void lock(mutex_t *m) { pthread_mutex_lock(m); }
static void unlock(mutex_t *m) { pthread_mutex_unlock(m); }
static void cond_init(cond_t *c) { pthread_cond_init(c, NULL); }
static void cond_destroy(cond_t *c) { pthread_cond_destroy(c); }
static void cond_wait(cond_t *c, mutex_t *m) { pthread_cond_wait(c, m); }
static void wake(cond_t *c) { pthread_cond_broadcast(c); }
#endif

typedef void (*drop_fn)(uint64_t);
typedef struct token {
    uint64_t handle;
    drop_fn drop;
    struct token *next;
} token;
static mutex_t queue_mutex;
static cond_t queue_ready;
static token *queue_head, *queue_tail;
static int queue_running;
#ifdef _WIN32
static DWORD WINAPI reap(void *unused)
#else
static void *reap(void *unused)
#endif
{
    (void)unused;
    for (;;) {
        lock(&queue_mutex);
        while (!queue_head) cond_wait(&queue_ready, &queue_mutex);
        token *item = queue_head;
        queue_head = item->next;
        if (!queue_head) queue_tail = NULL;
        unlock(&queue_mutex);
        item->drop(item->handle);
        free(item);
    }
    return 0;
}
#ifdef _WIN32
static INIT_ONCE queue_once = INIT_ONCE_STATIC_INIT;
static BOOL CALLBACK init_queue(PINIT_ONCE once, void *param, void **ctx) {
    (void)once; (void)param; (void)ctx;
#else
static pthread_once_t queue_once = PTHREAD_ONCE_INIT;
static void init_queue(void) {
#endif
    mutex_init(&queue_mutex);
    cond_init(&queue_ready);
#ifdef _WIN32
    HANDLE thread = CreateThread(NULL, 0, reap, NULL, 0, NULL);
    queue_running = thread != NULL;
    if (thread) CloseHandle(thread);
    return TRUE;
#else
    pthread_t thread;
    queue_running = pthread_create(&thread, NULL, reap, NULL) == 0;
    if (queue_running) pthread_detach(thread);
#endif
}
EXPORT void *aimux_dart_token_new(uint64_t handle, drop_fn drop) {
#ifdef _WIN32
    InitOnceExecuteOnce(&queue_once, init_queue, NULL, NULL);
#else
    pthread_once(&queue_once, init_queue);
#endif
    if (!queue_running || !handle || !drop) return NULL;
    token *item = malloc(sizeof(*item));
    if (item) *item = (token){handle, drop, NULL};
    return item;
}
/* Called only after NativeFinalizer.detach, on the owning Dart isolate. */
EXPORT void aimux_dart_token_dispose(void *ptr) { free(ptr); }
/* Correct NativeFinalizer ABI; queue cleanup without joining on the GC thread. */
EXPORT void aimux_dart_finalize(void *ptr) {
    token *item = ptr;
    if (!item) return;
    lock(&queue_mutex);
    if (queue_tail) queue_tail->next = item;
    else queue_head = item;
    queue_tail = item;
    wake(&queue_ready);
    unlock(&queue_mutex);
}

typedef void (*notify_fn)(char *owned_frame);
typedef struct bridge {
    mutex_t mutex;
    cond_t ready;
    notify_fn notify;
    drop_fn abort;
    uint64_t abort_handle;
    int pending, cancelled, failure;
} bridge;
EXPORT void *aimux_dart_bridge_new(notify_fn notify, drop_fn abort, uint64_t handle) {
    bridge *b = calloc(1, sizeof(*b));
    if (!b) return NULL;
    mutex_init(&b->mutex);
    cond_init(&b->ready);
    b->notify = notify;
    b->abort = abort;
    b->abort_handle = handle;
    return b;
}
EXPORT void aimux_dart_bridge_cancel(void *ptr) {
    bridge *b = ptr;
    lock(&b->mutex);
    b->cancelled = 1;
    wake(&b->ready);
    unlock(&b->mutex);
    b->abort(b->abort_handle);
}
EXPORT void aimux_dart_bridge_ack(void *ptr) {
    bridge *b = ptr;
    lock(&b->mutex);
    b->pending = 0;
    wake(&b->ready);
    unlock(&b->mutex);
}
EXPORT int32_t aimux_dart_bridge_pending(void *ptr) {
    bridge *b = ptr;
    lock(&b->mutex);
    int pending = b->pending;
    unlock(&b->mutex);
    return pending;
}
EXPORT int32_t aimux_dart_bridge_failure(void *ptr) {
    bridge *b = ptr;
    lock(&b->mutex);
    int failure = b->failure;
    unlock(&b->mutex);
    return failure;
}
EXPORT void aimux_dart_bridge_event(const char *frame, void *ptr) {
    bridge *b = ptr;
    size_t length = 0;
    if (frame) while (length <= 1048576 && frame[length]) length++;
    lock(&b->mutex);
    while (b->pending && !b->cancelled) cond_wait(&b->ready, &b->mutex);
    if (b->cancelled) { unlock(&b->mutex); return; }
    char *copy = frame && length <= 1048576 ? malloc(length + 1) : NULL;
    if (!copy) {
        b->failure = frame && length > 1048576 ? 305 : 206;
        b->cancelled = 1;
        wake(&b->ready);
        unlock(&b->mutex);
        b->abort(b->abort_handle);
        return;
    }
    memcpy(copy, frame, length + 1);
    b->pending = 1; /* Exactly one item/1 MiB of credit, including in-flight. */
    unlock(&b->mutex);
    b->notify(copy); /* NativeCallable.listener: posts to its owning isolate. */
    lock(&b->mutex);
    while (b->pending && !b->cancelled) cond_wait(&b->ready, &b->mutex);
    unlock(&b->mutex);
}
EXPORT void aimux_dart_frame_free(char *frame) { free(frame); }
/* Producer has returned AND the outstanding listener message was acknowledged. */
EXPORT void aimux_dart_bridge_free(void *ptr) {
    bridge *b = ptr;
    cond_destroy(&b->ready);
    mutex_destroy(&b->mutex);
    free(b);
}
