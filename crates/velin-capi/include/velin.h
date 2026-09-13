#ifndef VELIN_H
#define VELIN_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct VelinProgram VelinProgram;
typedef struct VelinMachine VelinMachine;

#define VELIN_C_API_VERSION 1u

enum {
    VELIN_VALUE_INTEGER = 1,
    VELIN_VALUE_BOOLEAN = 2,
    VELIN_VALUE_STRING = 3,
    VELIN_VALUE_COMPOUND = 4
};

enum {
    VELIN_YIELD_FINISHED = 0,
    VELIN_YIELD_HOST = 1,
    VELIN_YIELD_ERROR = 2
};

typedef struct VelinValue {
    uint32_t tag;
    int64_t integer;
    uint8_t boolean;
    uint8_t *text_ptr;
    size_t text_len;
    size_t text_capacity;
} VelinValue;

typedef struct VelinYield {
    uint32_t kind;
    uint32_t host_id;
    VelinValue *values;
    size_t values_len;
    uint8_t *error_ptr;
    size_t error_len;
    size_t error_capacity;
} VelinYield;

uint32_t velin_c_api_version(void);

VelinProgram *velin_program_load_json(const uint8_t *bytes, size_t len,
                                      uint8_t **error_ptr, size_t *error_len,
                                      size_t *error_capacity);
void velin_program_free(VelinProgram *program);

VelinMachine *velin_machine_new(const VelinProgram *program, int64_t seed,
                                uint8_t **error_ptr, size_t *error_len,
                                size_t *error_capacity);
void velin_machine_free(VelinMachine *machine);
VelinYield velin_machine_run(VelinMachine *machine);
VelinYield velin_machine_resume(VelinMachine *machine, const VelinValue *value);
VelinYield velin_machine_restart(VelinMachine *machine, int64_t seed);

void velin_yield_free(VelinYield *result);
void velin_buffer_free(uint8_t *ptr, size_t len, size_t capacity);

#ifdef __cplusplus
}
#endif

#endif
