#ifndef SIM_PLATFORM_IOS_H
#define SIM_PLATFORM_IOS_H
#include <stddef.h>
#include <stdint.h>

typedef struct { const uint8_t *ptr; size_t len; } SimBorrowedBytes;
typedef struct { uint8_t *ptr; size_t len; size_t cap; } SimOwnedBytes;
typedef struct SimAbiError { char *message; } SimAbiError;
typedef struct { SimOwnedBytes bytes; SimAbiError *error; } SimCallResponse;

typedef struct {
    size_t struct_size;
    uint16_t abi_major;
    uint16_t abi_minor;
    void *(*instantiate)(void);
    void (*destroy_instance)(void *instance);
    SimCallResponse (*manifest)(void *instance);
    SimCallResponse (*call)(void *instance, const char *function, SimBorrowedBytes args);
    void (*destroy_bytes)(SimOwnedBytes bytes);
    void (*destroy_error)(SimAbiError *error);
} SimNativeLibAbiV1;

const SimNativeLibAbiV1 *sim_native_abi_v1(void);
SimCallResponse sim_ios_encode_input_json(SimBorrowedBytes json);
SimCallResponse sim_ios_decode_output_json(SimBorrowedBytes frame);
#endif
