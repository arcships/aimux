#ifndef AIMUX_TEST_ABI_H
#define AIMUX_TEST_ABI_H
#include <stdint.h>
uint64_t aimux_test_new_handle(void);
int aimux_test_drops(uint64_t handle);
int aimux_test_active(void);
int aimux_test_strings(void);
int aimux_test_errors(void);
int aimux_test_aborts(void);
int aimux_test_emitted(void);
int aimux_test_main_calls(void);
void aimux_test_version(uint32_t version);
#endif
