//! A small, stable C boundary for the Velin bytecode runtime.
//!
//! The ABI deliberately exposes opaque program and machine handles. The only
//! data crossing the boundary are fixed-layout tags, integers, booleans and
//! owned UTF-8 buffers. Programs are loaded from JSON once at the raw ABI
//! boundary and immediately retained as a validated proof. Host effects use
//! the binary `VelinValue` array; opt-in JSON functions provide reversible
//! List and Record payloads without changing the legacy compound tag.

#![allow(clippy::missing_safety_doc)]
#![allow(clippy::module_name_repetitions)]
#![allow(unsafe_op_in_unsafe_fn)]

mod handles;
mod result;
mod value;

pub use handles::{VelinMachine, VelinProgram};
pub use result::VelinYield;
pub use result::{VelinBatch, VelinEffect};
use result::{
    batch_from_result, batch_from_result_json, error_yield, yield_from_result,
    yield_from_result_json,
};
use std::slice;
use std::sync::Arc;
use velin_bytecode::{InitialFrame, Program, ValidatedProgram};
use velin_vm::{ExecutionPolicy, Machine};

pub use result::{
    VELIN_BATCH_EFFECTS, VELIN_BATCH_EMPTY, VELIN_BATCH_ERROR, VELIN_YIELD_ERROR,
    VELIN_YIELD_FINISHED, VELIN_YIELD_HOST,
};
pub use value::{
    VELIN_VALUE_BOOLEAN, VELIN_VALUE_COMPOUND, VELIN_VALUE_INTEGER, VELIN_VALUE_JSON,
    VELIN_VALUE_STRING, VelinValue,
};

/// Version of the exported C data structures and function contract.
pub const VELIN_C_API_VERSION: u32 = 1;

/// Numeric execution limits accepted by the append-only policy constructor.
///
/// The C boundary cannot carry a Rust progress callback. Hosts can request
/// cancellation explicitly with [`velin_machine_cancel`] between calls.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VelinExecutionPolicy {
    pub max_fuel: u64,
    pub max_immediate_fuel: u64,
    pub max_host_effects: usize,
    pub max_call_depth: usize,
    pub max_value_values: usize,
    pub max_value_text_bytes: usize,
    pub max_machine_values: usize,
    pub max_machine_text_bytes: usize,
    pub max_host_payload_values: usize,
    pub max_host_payload_text_bytes: usize,
    pub max_host_queue_events: usize,
    pub max_host_queue_values: usize,
    pub max_host_queue_text_bytes: usize,
    pub progress_interval: u64,
}

impl From<ExecutionPolicy> for VelinExecutionPolicy {
    fn from(policy: ExecutionPolicy) -> Self {
        Self {
            max_fuel: policy.max_fuel,
            max_immediate_fuel: policy.max_immediate_fuel,
            max_host_effects: policy.max_host_effects,
            max_call_depth: policy.max_call_depth,
            max_value_values: policy.max_value_values,
            max_value_text_bytes: policy.max_value_text_bytes,
            max_machine_values: policy.max_machine_values,
            max_machine_text_bytes: policy.max_machine_text_bytes,
            max_host_payload_values: policy.max_host_payload_values,
            max_host_payload_text_bytes: policy.max_host_payload_text_bytes,
            max_host_queue_events: policy.max_host_queue_events,
            max_host_queue_values: policy.max_host_queue_values,
            max_host_queue_text_bytes: policy.max_host_queue_text_bytes,
            progress_interval: policy.progress_interval,
        }
    }
}

impl From<&VelinExecutionPolicy> for ExecutionPolicy {
    fn from(policy: &VelinExecutionPolicy) -> Self {
        Self {
            max_fuel: policy.max_fuel,
            max_immediate_fuel: policy.max_immediate_fuel,
            max_host_effects: policy.max_host_effects,
            max_call_depth: policy.max_call_depth,
            max_value_values: policy.max_value_values,
            max_value_text_bytes: policy.max_value_text_bytes,
            max_machine_values: policy.max_machine_values,
            max_machine_text_bytes: policy.max_machine_text_bytes,
            max_host_payload_values: policy.max_host_payload_values,
            max_host_payload_text_bytes: policy.max_host_payload_text_bytes,
            max_host_queue_events: policy.max_host_queue_events,
            max_host_queue_values: policy.max_host_queue_values,
            max_host_queue_text_bytes: policy.max_host_queue_text_bytes,
            progress_interval: policy.progress_interval.max(1),
            progress_callback: None,
        }
    }
}

/// Returns the default numeric execution policy for a new machine.
#[unsafe(no_mangle)]
pub extern "C" fn velin_execution_policy_default() -> VelinExecutionPolicy {
    ExecutionPolicy::default().into()
}

/// Returns the C ABI contract version compiled into the library.
#[unsafe(no_mangle)]
pub extern "C" fn velin_c_api_version() -> u32 {
    VELIN_C_API_VERSION
}

/// Loads and validates a JSON-encoded bytecode `Program`.
///
/// `error_ptr` and `error_len` receive an owned UTF-8 error buffer on failure.
/// Release it with [`velin_buffer_free`]. The input is borrowed for the call.
///
/// # Panics
/// Panics only if JSON deserialization and the follow-up validation proof
/// disagree, which would be a loader bug.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn velin_program_load_json(
    bytes: *const u8,
    len: usize,
    error_ptr: *mut *mut u8,
    error_len: *mut usize,
    error_capacity: *mut usize,
) -> *mut VelinProgram {
    clear_error(error_ptr, error_len, error_capacity);
    if bytes.is_null() && len != 0 {
        write_error(
            "program input is null",
            error_ptr,
            error_len,
            error_capacity,
        );
        return std::ptr::null_mut();
    }
    let input = if len == 0 {
        &[]
    } else {
        unsafe { slice::from_raw_parts(bytes, len) }
    };
    let program = match serde_json::from_slice::<Program>(input) {
        Ok(program) => program,
        Err(error) => {
            write_error(
                &format!("invalid program JSON: {error}"),
                error_ptr,
                error_len,
                error_capacity,
            );
            return std::ptr::null_mut();
        }
    };
    // JSON deserialization already ran `Program::validate`.
    let program = ValidatedProgram::new(Arc::new(program))
        .expect("deserialized programs have already been validated");
    let initial = InitialFrame::from_named_values(
        &program.program().slots,
        std::iter::empty::<(&str, &velin_syntax::Value)>(),
    )
    .expect("an empty initial frame cannot fail");
    Box::into_raw(Box::new(VelinProgram { program, initial }))
}

/// Releases a program handle. Null is accepted.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn velin_program_free(program: *mut VelinProgram) {
    if !program.is_null() {
        drop(unsafe { Box::from_raw(program) });
    }
}

/// Creates a machine from a loaded program using the supplied RNG seed.
///
/// The program remains independently owned and may be freed after this call.
///
/// # Panics
/// Panics only if the empty initial frame stored with the program cannot
/// instantiate the validated machine.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn velin_machine_new(
    program: *const VelinProgram,
    seed: i64,
    error_ptr: *mut *mut u8,
    error_len: *mut usize,
    error_capacity: *mut usize,
) -> *mut VelinMachine {
    let policy = velin_execution_policy_default();
    unsafe {
        velin_machine_new_with_policy(program, seed, &policy, error_ptr, error_len, error_capacity)
    }
}

/// Creates a machine with an explicit numeric execution policy.
///
/// The policy is copied during construction and may be released immediately.
/// Progress callbacks are not part of the C ABI; use the cancellation
/// functions between calls when a host needs cooperative cancellation.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn velin_machine_new_with_policy(
    program: *const VelinProgram,
    seed: i64,
    policy: *const VelinExecutionPolicy,
    error_ptr: *mut *mut u8,
    error_len: *mut usize,
    error_capacity: *mut usize,
) -> *mut VelinMachine {
    clear_error(error_ptr, error_len, error_capacity);
    let Some(program) = (unsafe { program.as_ref() }) else {
        write_error(
            "program handle is null",
            error_ptr,
            error_len,
            error_capacity,
        );
        return std::ptr::null_mut();
    };
    let Some(policy) = (unsafe { policy.as_ref() }) else {
        write_error(
            "execution policy pointer is null",
            error_ptr,
            error_len,
            error_capacity,
        );
        return std::ptr::null_mut();
    };
    let machine = Machine::from_validated_with_seed_and_frame_and_policy(
        &program.program,
        seed,
        &program.initial,
        ExecutionPolicy::from(policy),
    )
    .expect("the empty initial frame matches the validated program");
    Box::into_raw(Box::new(VelinMachine {
        machine,
        initial: program.initial.clone(),
    }))
}

/// Releases a machine handle. Null is accepted.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn velin_machine_free(machine: *mut VelinMachine) {
    if !machine.is_null() {
        drop(unsafe { Box::from_raw(machine) });
    }
}

/// Requests cooperative cancellation at the next VM fuel checkpoint.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn velin_machine_cancel(machine: *mut VelinMachine) {
    if let Some(machine) = unsafe { machine.as_mut() } {
        machine.machine.cancel();
    }
}

/// Clears a previous cooperative cancellation request.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn velin_machine_clear_cancellation(machine: *mut VelinMachine) {
    if let Some(machine) = unsafe { machine.as_mut() } {
        machine.machine.clear_cancellation();
    }
}

/// Runs a machine until a host effect, completion, or an execution error.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn velin_machine_run(machine: *mut VelinMachine) -> VelinYield {
    let Some(machine) = (unsafe { machine.as_mut() }) else {
        return error_yield("machine handle is null");
    };
    yield_from_result(machine.machine.run())
}

/// Runs like [`velin_machine_run`], encoding List and Record arguments as JSON.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn velin_machine_run_json(machine: *mut VelinMachine) -> VelinYield {
    let Some(machine) = (unsafe { machine.as_mut() }) else {
        return error_yield("machine handle is null");
    };
    yield_from_result_json(machine.machine.run())
}

/// Runs side-effect-only host commands in source order until `limit`, a bound
/// host command, completion, or an execution error.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn velin_machine_run_batch(
    machine: *mut VelinMachine,
    limit: usize,
) -> VelinBatch {
    let Some(machine) = (unsafe { machine.as_mut() }) else {
        return result::batch_from_result::<&str>(Err("machine handle is null"));
    };
    let result = machine.machine.run_effect_batch(limit);
    batch_from_result(result)
}

/// Runs a batch with List and Record arguments encoded as stable JSON.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn velin_machine_run_batch_json(
    machine: *mut VelinMachine,
    limit: usize,
) -> VelinBatch {
    let Some(machine) = (unsafe { machine.as_mut() }) else {
        return result::batch_from_result_json::<&str>(Err("machine handle is null"));
    };
    batch_from_result_json(machine.machine.run_effect_batch(limit))
}

/// Resumes a pending host effect with a tagged value, or null for no value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn velin_machine_resume(
    machine: *mut VelinMachine,
    value: *const VelinValue,
) -> VelinYield {
    let Some(machine) = (unsafe { machine.as_mut() }) else {
        return error_yield("machine handle is null");
    };
    let value = if value.is_null() {
        None
    } else {
        match unsafe { value::from_c_value(&*value) } {
            Ok(value) => Some(value),
            Err(error) => return error_yield(&error),
        }
    };
    yield_from_result(machine.machine.resume(value))
}

/// Resumes like [`velin_machine_resume`], encoding the next compounds as JSON.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn velin_machine_resume_json(
    machine: *mut VelinMachine,
    value: *const VelinValue,
) -> VelinYield {
    let Some(machine) = (unsafe { machine.as_mut() }) else {
        return error_yield("machine handle is null");
    };
    let value = if value.is_null() {
        None
    } else {
        match unsafe { value::from_c_value(&*value) } {
            Ok(value) => Some(value),
            Err(error) => return error_yield(&error),
        }
    };
    yield_from_result_json(machine.machine.resume(value))
}

/// Restarts a machine with an empty frame and a new RNG seed, then runs it.
///
/// # Panics
/// Panics only if restarting with the machine's own initial frame fails.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn velin_machine_restart(
    machine: *mut VelinMachine,
    seed: i64,
) -> VelinYield {
    let Some(machine) = (unsafe { machine.as_mut() }) else {
        return error_yield("machine handle is null");
    };
    machine
        .machine
        .restart(&machine.initial, seed)
        .expect("restarting with the program's own initial frame succeeds");
    yield_from_result(machine.machine.run())
}

/// Restarts like [`velin_machine_restart`], encoding compounds as JSON.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn velin_machine_restart_json(
    machine: *mut VelinMachine,
    seed: i64,
) -> VelinYield {
    let Some(machine) = (unsafe { machine.as_mut() }) else {
        return error_yield("machine handle is null");
    };
    machine
        .machine
        .restart(&machine.initial, seed)
        .expect("restarting with the program's own initial frame succeeds");
    yield_from_result_json(machine.machine.run())
}

/// Releases all buffers owned by a yield result and clears it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn velin_yield_free(result: *mut VelinYield) {
    if !result.is_null() {
        unsafe { result::free_yield(&mut *result) };
    }
}

/// Releases every allocation owned by a batch result and clears it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn velin_batch_free(result: *mut VelinBatch) {
    if !result.is_null() {
        unsafe { result::free_batch(&mut *result) };
    }
}

/// Releases an error or value text buffer returned by the ABI.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn velin_buffer_free(ptr: *mut u8, len: usize, capacity: usize) {
    if !ptr.is_null() && capacity != 0 {
        drop(unsafe { Vec::from_raw_parts(ptr, len, capacity) });
    }
}

fn clear_error(error_ptr: *mut *mut u8, error_len: *mut usize, error_capacity: *mut usize) {
    if !error_ptr.is_null() {
        unsafe { *error_ptr = std::ptr::null_mut() };
    }
    if !error_len.is_null() {
        unsafe { *error_len = 0 };
    }
    if !error_capacity.is_null() {
        unsafe { *error_capacity = 0 };
    }
}

fn write_error(
    message: &str,
    error_ptr: *mut *mut u8,
    error_len: *mut usize,
    error_capacity: *mut usize,
) {
    if error_ptr.is_null() || error_len.is_null() || error_capacity.is_null() {
        return;
    }
    let (ptr, len, capacity) = value::owned_buffer(message.as_bytes());
    unsafe {
        *error_ptr = ptr;
        *error_len = len;
        *error_capacity = capacity;
    }
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "json_tests.rs"]
mod json_tests;
