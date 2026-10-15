/* Offline C11 example. See scripts/test-c-abi.sh. */
#include "aimux.h"
#include <stdio.h>

int main(void) {
    char *manifest = NULL;
    aimux_error_t *error = aimux_manifest_json(&manifest);
    if (error) {
        char *message = aimux_error_message(error);
        char *json = aimux_error_json(error);
        fprintf(stderr, "aimux (%d): %s\n%s\n", aimux_error_code(error),
                message ? message : "", json ? json : "");
        aimux_free_string(message);
        aimux_free_string(json);
        aimux_error_free(error);
        return 1;
    }
    puts(manifest);
    aimux_free_string(manifest);
    char *description = NULL;
    error = aimux_call(0, "runtime.describe", "{}", 0, &description);
    if (error) {
        char *message = aimux_error_message(error);
        fprintf(stderr, "%s\n", message ? message : "runtime.describe failed");
        aimux_free_string(message);
        aimux_error_free(error);
        return 1;
    }
    puts(description);
    aimux_free_string(description);
    return 0;
}
