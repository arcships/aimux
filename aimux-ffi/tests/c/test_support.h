#ifndef AIMUX_C_TEST_SUPPORT_H
#define AIMUX_C_TEST_SUPPORT_H
#include "aimux.h"
#include <assert.h>
#include <inttypes.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* All getter results are owned, even while the error remains alive. */
static void check_error(aimux_error_t *error, int32_t expected) {
    assert(error != NULL);
    assert(aimux_error_code(error) == expected);
    char *message = aimux_error_message(error);
    char *json = aimux_error_json(error);
    assert(message != NULL && json != NULL);
    assert(strstr(json, "\"kind\"") != NULL);
    assert(strstr(json, "\"error\"") != NULL);
    aimux_free_string(message);
    aimux_free_string(json);
    aimux_error_free(error);
}

static uint64_t create_handle(const char *op, uint64_t parent, const char *params) {
    char *json = NULL;
    aimux_error_t *error = aimux_call(parent, op, params, 0, &json);
    if (error) {
        char *message = aimux_error_message(error);
        fprintf(stderr, "%s: %s\n", op, message ? message : "(no message)");
        aimux_free_string(message);
        aimux_error_free(error);
        abort();
    }
    assert(json != NULL);
    /* Handle-producing control DTOs return {"handle":"<decimal>"}. */
    const char *value = strstr(json, "\"handle\"");
    assert(value != NULL);
    value = strchr(value + 8, ':');
    assert(value != NULL);
    ++value;
    while (*value == ' ' || *value == '\n' || *value == '\r' || *value == '\t') ++value;
    assert(*value++ == '"');
    char *end = NULL;
    uint64_t handle = strtoull(value, &end, 10);
    assert(end != value && *end == '"' && handle != 0);
    aimux_free_string(json);
    return handle;
}
#endif
