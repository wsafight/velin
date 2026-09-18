#include "velin.h"

#include <assert.h>
#include <string.h>

/* ABI v1 compatibility fixture: changing this contract must fail CI. */
_Static_assert(VELIN_C_API_VERSION == 1u, "unexpected Velin C ABI version");

int main(void) {
    assert(velin_c_api_version() == 1u);
    static const char program_json[] =
        "{\"ops\":[{\"Set\":{\"slot\":0,\"value\":0}},\"Halt\"],"
        "\"chunks\":[{\"ops\":[{\"Const\":{\"dst\":0,\"constant\":0}}],"
        "\"constants\":[42],\"registers\":1,\"result\":0,\"line\":1}],"
        "\"slots\":[\"result\"]}";
    uint8_t *error = NULL;
    size_t error_len = 0;
    size_t error_capacity = 0;
    VelinProgram *program = velin_program_load_json(
        (const uint8_t *)program_json, strlen(program_json), &error, &error_len,
        &error_capacity);
    assert(program != NULL);
    assert(error == NULL);
    assert(error_len == 0);

    VelinMachine *machine = velin_machine_new(program, 0, &error, &error_len, &error_capacity);
    assert(machine != NULL);
    VelinExecutionPolicy policy = velin_execution_policy_default();
    VelinMachine *policy_machine = velin_machine_new_with_policy(
        program, 0, &policy, &error, &error_len, &error_capacity);
    assert(policy_machine != NULL);
    velin_machine_cancel(policy_machine);
    velin_machine_clear_cancellation(policy_machine);
    velin_machine_free(policy_machine);
    VelinYield result = velin_machine_run(machine);
    assert(result.kind == VELIN_YIELD_FINISHED);
    velin_yield_free(&result);
    VelinYield restarted = velin_machine_restart(machine, 1);
    assert(restarted.kind == VELIN_YIELD_FINISHED);
    velin_yield_free(&restarted);
    velin_machine_free(machine);
    velin_program_free(program);
    return 0;
}
