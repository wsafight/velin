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
    VELIN_VALUE_COMPOUND = 4,
    VELIN_VALUE_JSON = 5
};

enum {
    VELIN_YIELD_FINISHED = 0,
    VELIN_YIELD_HOST = 1,
    VELIN_YIELD_ERROR = 2
};

enum {
    VELIN_BATCH_EMPTY = 0,
    VELIN_BATCH_EFFECTS = 1,
    VELIN_BATCH_ERROR = 2
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

typedef struct VelinEffect {
    uint32_t host_id;
    VelinValue *values;
    size_t values_len;
} VelinEffect;

typedef struct VelinBatch {
    uint32_t kind;
    VelinEffect *effects;
    size_t effects_len;
    uint8_t *error_ptr;
    size_t error_len;
    size_t error_capacity;
} VelinBatch;

/* Numeric execution limits. The C ABI uses explicit cancellation functions
 * instead of embedding a host callback in this structure. */
typedef struct VelinExecutionPolicy {
    uint64_t max_fuel;
    uint64_t max_immediate_fuel;
    size_t max_host_effects;
    size_t max_call_depth;
    size_t max_value_values;
    size_t max_value_text_bytes;
    size_t max_machine_values;
    size_t max_machine_text_bytes;
    size_t max_host_payload_values;
    size_t max_host_payload_text_bytes;
    size_t max_host_queue_events;
    size_t max_host_queue_values;
    size_t max_host_queue_text_bytes;
    uint64_t progress_interval;
} VelinExecutionPolicy;

uint32_t velin_c_api_version(void);
VelinExecutionPolicy velin_execution_policy_default(void);

VelinProgram *velin_program_load_json(const uint8_t *bytes, size_t len,
                                      uint8_t **error_ptr, size_t *error_len,
                                      size_t *error_capacity);
void velin_program_free(VelinProgram *program);

VelinMachine *velin_machine_new(const VelinProgram *program, int64_t seed,
                                uint8_t **error_ptr, size_t *error_len,
                                size_t *error_capacity);
VelinMachine *velin_machine_new_with_policy(const VelinProgram *program,
                                            int64_t seed,
                                            const VelinExecutionPolicy *policy,
                                            uint8_t **error_ptr,
                                            size_t *error_len,
                                            size_t *error_capacity);
void velin_machine_free(VelinMachine *machine);
void velin_machine_cancel(VelinMachine *machine);
void velin_machine_clear_cancellation(VelinMachine *machine);
VelinYield velin_machine_run(VelinMachine *machine);
VelinBatch velin_machine_run_batch(VelinMachine *machine, size_t limit);
VelinYield velin_machine_resume(VelinMachine *machine, const VelinValue *value);
VelinYield velin_machine_restart(VelinMachine *machine, int64_t seed);

/* Opt-in compound marshalling. List and Record text is stable JSON tagged
 * VELIN_VALUE_JSON; scalars retain their existing binary tags. */
VelinYield velin_machine_run_json(VelinMachine *machine);
VelinBatch velin_machine_run_batch_json(VelinMachine *machine, size_t limit);
VelinYield velin_machine_resume_json(VelinMachine *machine,
                                     const VelinValue *value);
VelinYield velin_machine_restart_json(VelinMachine *machine, int64_t seed);

void velin_yield_free(VelinYield *result);
void velin_batch_free(VelinBatch *result);
void velin_buffer_free(uint8_t *ptr, size_t len, size_t capacity);

#ifdef __cplusplus
}
#endif

#endif
