//! A small, stable C boundary for the Velin bytecode runtime.
//!
//! The ABI deliberately exposes opaque program and machine handles. The only
//! data crossing the boundary are fixed-layout tags, integers, booleans and
//! owned UTF-8 buffers. Programs are loaded from JSON once; host effects use
//! the binary `VelinValue` array and do not require JSON round trips.

#![allow(clippy::missing_safety_doc)]
#![allow(clippy::module_name_repetitions)]
#![allow(unsafe_op_in_unsafe_fn)]

mod handles;
mod result;
mod value;

pub use handles::{VelinMachine, VelinProgram};
pub use result::VelinYield;
use result::{error_yield, yield_from_result};
use std::slice;
use std::sync::Arc;
use velin_bytecode::Program;
use velin_vm::Machine;

pub use result::{VELIN_YIELD_ERROR, VELIN_YIELD_FINISHED, VELIN_YIELD_HOST};
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
    if let Err(error) = program.validate() {
        write_error(
            &format!("invalid program: {error}"),
            error_ptr,
            error_len,
            error_capacity,
        );
        return std::ptr::null_mut();
    }
    Box::into_raw(Box::new(VelinProgram {
        program: Arc::new(program),
    }))
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
    let machine = match Machine::with_seed(Arc::clone(&program.program), seed) {
        Ok(machine) => machine,
        Err(error) => {
            write_error(
                &format!("cannot create machine: {error}"),
                error_ptr,
                error_len,
                error_capacity,
            );
            return std::ptr::null_mut();
        }
    };
    Box::into_raw(Box::new(VelinMachine { machine }))
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
#[unsafe(no_mangle)]
pub unsafe extern "C" fn velin_machine_restart(
    machine: *mut VelinMachine,
    seed: i64,
) -> VelinYield {
    let Some(machine) = (unsafe { machine.as_mut() }) else {
        return error_yield("machine handle is null");
    };
    let program = machine.machine.program().clone();
    machine.machine = match Machine::with_seed(program, seed) {
        Ok(machine) => machine,
        Err(error) => return error_yield(&format!("cannot restart machine: {error}")),
    };
    yield_from_result(machine.machine.run())
}

/// Releases all buffers owned by a yield result and clears it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn velin_yield_free(result: *mut VelinYield) {
    if !result.is_null() {
        unsafe { result::free_yield(&mut *result) };
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
mod tests {
    use super::*;

    const HOST_PROGRAM: &[u8] = br#"{"ops":[{"Host":{"host_id":7,"args":[],"bind":0,"line":1}},"Halt"],"chunks":[],"slots":["answer"]}"#;

    #[test]
    fn host_yield_and_scalar_resume_cross_the_c_boundary() {
        let mut error = std::ptr::null_mut();
        let mut error_len = 0;
        let mut error_capacity = 0;
        let program = unsafe {
            velin_program_load_json(
                HOST_PROGRAM.as_ptr(),
                HOST_PROGRAM.len(),
                &mut error,
                &mut error_len,
                &mut error_capacity,
            )
        };
        assert!(!program.is_null());
        assert!(error.is_null());
        let machine = unsafe {
            velin_machine_new(program, 0, &mut error, &mut error_len, &mut error_capacity)
        };
        assert!(!machine.is_null());
        let mut yielded = unsafe { velin_machine_run(machine) };
        assert_eq!(yielded.kind, VELIN_YIELD_HOST);
        assert_eq!(yielded.host_id, 7);
        assert_eq!(yielded.values_len, 0);
        unsafe { velin_yield_free(&mut yielded) };

        let answer = VelinValue {
            tag: VELIN_VALUE_INTEGER,
            integer: 42,
            boolean: 0,
            text_ptr: std::ptr::null_mut(),
            text_len: 0,
            text_capacity: 0,
        };
        let mut finished = unsafe { velin_machine_resume(machine, &answer) };
        assert_eq!(finished.kind, VELIN_YIELD_FINISHED);
        unsafe {
            velin_yield_free(&mut finished);
            velin_machine_free(machine);
            velin_program_free(program);
        }
    }

    #[test]
    fn malformed_input_returns_owned_error_bytes() {
        let mut error = std::ptr::null_mut();
        let mut error_len = 0;
        let mut error_capacity = 0;
        let program = unsafe {
            velin_program_load_json(
                std::ptr::null(),
                0,
                &mut error,
                &mut error_len,
                &mut error_capacity,
            )
        };
        assert!(program.is_null());
        assert!(!error.is_null());
        assert!(error_len > 0);
        unsafe { velin_buffer_free(error, error_len, error_capacity) };
    }
}
