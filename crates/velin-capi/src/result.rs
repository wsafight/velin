use crate::value::{VelinValue, to_c_value};
use velin_vm::{HostEffect, Yield};

/// The machine finished normally.
pub const VELIN_YIELD_FINISHED: u32 = 0;
/// The machine yielded a host effect.
pub const VELIN_YIELD_HOST: u32 = 1;
/// The machine stopped with an error.
pub const VELIN_YIELD_ERROR: u32 = 2;

/// A batch contains side-effect-only host events collected in source order.
pub const VELIN_BATCH_EMPTY: u32 = 0;
pub const VELIN_BATCH_EFFECTS: u32 = 1;
pub const VELIN_BATCH_ERROR: u32 = 2;

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

/// One side-effect-only event in a [`VelinBatch`].
#[repr(C)]
pub struct VelinEffect {
    pub host_id: u32,
    pub values: *mut VelinValue,
    pub values_len: usize,
}

/// Fixed-layout result returned by the batch C API.
#[repr(C)]
pub struct VelinBatch {
    pub kind: u32,
    pub effects: *mut VelinEffect,
    pub effects_len: usize,
    pub error_ptr: *mut u8,
    pub error_len: usize,
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

pub fn batch_from_result<E: std::fmt::Display>(result: Result<Vec<HostEffect>, E>) -> VelinBatch {
    match result {
        Ok(effects) if effects.is_empty() => empty_batch(),
        Ok(effects) => {
            let effects = effects
                .into_iter()
                .map(|effect| {
                    let values = effect
                        .values
                        .iter()
                        .map(to_c_value)
                        .collect::<Vec<_>>()
                        .into_boxed_slice();
                    let values_len = values.len();
                    let values = Box::into_raw(values) as *mut VelinValue;
                    VelinEffect {
                        host_id: effect.host_id,
                        values,
                        values_len,
                    }
                })
                .collect::<Vec<_>>()
                .into_boxed_slice();
            let effects_len = effects.len();
            let effects = Box::into_raw(effects) as *mut VelinEffect;
            VelinBatch {
                kind: VELIN_BATCH_EFFECTS,
                effects,
                effects_len,
                error_ptr: std::ptr::null_mut(),
                error_len: 0,
                error_capacity: 0,
            }
        }
        Err(error) => batch_error(&error.to_string()),
    }
}

pub fn empty_batch() -> VelinBatch {
    VelinBatch {
        kind: VELIN_BATCH_EMPTY,
        effects: std::ptr::null_mut(),
        effects_len: 0,
        error_ptr: std::ptr::null_mut(),
        error_len: 0,
        error_capacity: 0,
    }
}

fn batch_error(message: &str) -> VelinBatch {
    let (error_ptr, error_len, error_capacity) = crate::value::owned_buffer(message.as_bytes());
    VelinBatch {
        kind: VELIN_BATCH_ERROR,
        effects: std::ptr::null_mut(),
        effects_len: 0,
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

pub unsafe fn free_batch(result: &mut VelinBatch) {
    if !result.effects.is_null() {
        let effects = unsafe {
            Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                result.effects,
                result.effects_len,
            ))
        };
        for effect in effects.iter() {
            if !effect.values.is_null() {
                let values = unsafe {
                    Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                        effect.values,
                        effect.values_len,
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
        }
    }
    if !result.error_ptr.is_null() && result.error_capacity != 0 {
        drop(unsafe {
            Vec::from_raw_parts(result.error_ptr, result.error_len, result.error_capacity)
        });
    }
    *result = empty_batch();
}
