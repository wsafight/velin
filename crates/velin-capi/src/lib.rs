//! A small, stable C boundary for the Velin bytecode runtime.
//!
//! The ABI deliberately exposes opaque program and machine handles. The only
//! data crossing the boundary are fixed-layout tags, integers, booleans and
//! owned UTF-8 buffers. Programs are loaded from JSON once at the raw ABI
//! boundary and immediately retained as a validated proof; host effects use
//! the binary `VelinValue` array and do not require JSON round trips.

#![allow(clippy::missing_safety_doc)]
#![allow(clippy::module_name_repetitions)]
#![allow(unsafe_op_in_unsafe_fn)]

mod handles;
mod result;
mod value;

pub use handles::{VelinMachine, VelinProgram};
pub use result::VelinYield;
pub use result::{VelinBatch, VelinEffect};
use result::{batch_from_result, error_yield, yield_from_result};
use std::slice;
use std::sync::Arc;
use velin_bytecode::{InitialFrame, Program, ValidatedProgram};
use velin_vm::Machine;

pub use result::{
    VELIN_BATCH_EFFECTS, VELIN_BATCH_EMPTY, VELIN_BATCH_ERROR, VELIN_YIELD_ERROR,
    VELIN_YIELD_FINISHED, VELIN_YIELD_HOST,
};
pub use value::{
    VELIN_VALUE_BOOLEAN, VELIN_VALUE_COMPOUND, VELIN_VALUE_INTEGER, VELIN_VALUE_STRING, VelinValue,
};

/// Version of the exported C data structures and function contract.
pub const VELIN_C_API_VERSION: u32 = 1;

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
    let machine = Machine::from_validated_with_seed_and_frame(
        &program.program,
        seed,
        &program.initial,
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

/// Runs a machine until a host effect, completion, or an execution error.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn velin_machine_run(machine: *mut VelinMachine) -> VelinYield {
    let Some(machine) = (unsafe { machine.as_mut() }) else {
        return error_yield("machine handle is null");
    };
    yield_from_result(machine.machine.run())
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

/// Resumes a pending host effect with a scalar value, or null for no value.
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
