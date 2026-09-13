use crate::value::{VelinValue, to_c_value};
use velin_vm::Yield;

/// The machine finished normally.
pub const VELIN_YIELD_FINISHED: u32 = 0;
/// The machine yielded a host effect.
pub const VELIN_YIELD_HOST: u32 = 1;
/// The machine stopped with an error.
pub const VELIN_YIELD_ERROR: u32 = 2;

/// A fixed-layout result returned by `run`, `resume`, and `restart`.
#[repr(C)]
pub struct VelinYield {
    /// One of the `VELIN_YIELD_*` constants.
    pub kind: u32,
    /// Host command ID when `kind == VELIN_YIELD_HOST`.
    pub host_id: u32,
    /// Owned host argument array. Release with `velin_yield_free`.
    pub values: *mut VelinValue,
    /// Number of entries in `values`.
    pub values_len: usize,
    /// Owned UTF-8 error bytes when `kind == VELIN_YIELD_ERROR`.
    pub error_ptr: *mut u8,
    /// Number of error bytes.
    pub error_len: usize,
    /// Allocation capacity for `error_ptr`.
    pub error_capacity: usize,
}

pub fn yield_from_result<E: std::fmt::Display>(result: Result<Yield, E>) -> VelinYield {
    match result {
        Ok(Yield::Finished) => finished_yield(),
        Ok(Yield::Host { host_id, values }) => {
            let values = values
                .iter()
                .map(to_c_value)
                .collect::<Vec<_>>()
                .into_boxed_slice();
            let values_len = values.len();
            let values = Box::into_raw(values) as *mut VelinValue;
            VelinYield {
                kind: VELIN_YIELD_HOST,
                host_id,
                values,
                values_len,
                error_ptr: std::ptr::null_mut(),
                error_len: 0,
                error_capacity: 0,
            }
        }
        Err(error) => error_yield(&error.to_string()),
    }
}

pub fn finished_yield() -> VelinYield {
    VelinYield {
        kind: VELIN_YIELD_FINISHED,
        host_id: 0,
        values: std::ptr::null_mut(),
        values_len: 0,
        error_ptr: std::ptr::null_mut(),
        error_len: 0,
        error_capacity: 0,
    }
}

pub fn error_yield(message: &str) -> VelinYield {
    let (error_ptr, error_len, error_capacity) = crate::value::owned_buffer(message.as_bytes());
    VelinYield {
        kind: VELIN_YIELD_ERROR,
        host_id: 0,
        values: std::ptr::null_mut(),
        values_len: 0,
        error_ptr,
        error_len,
        error_capacity,
    }
}

pub unsafe fn free_yield(result: &mut VelinYield) {
    if !result.values.is_null() {
        let values = unsafe {
            Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                result.values,
                result.values_len,
            ))
        };
        for value in values.iter() {
            if !value.text_ptr.is_null() && value.text_capacity != 0 {
                drop(unsafe {
                    Vec::from_raw_parts(value.text_ptr, value.text_len, value.text_capacity)
                });
            }
        }
    }
    if !result.error_ptr.is_null() && result.error_capacity != 0 {
        drop(unsafe {
            Vec::from_raw_parts(result.error_ptr, result.error_len, result.error_capacity)
        });
    }
    *result = finished_yield();
}
