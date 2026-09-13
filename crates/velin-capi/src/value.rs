use std::slice;
use velin_syntax::Value;

/// Integer value tag.
pub const VELIN_VALUE_INTEGER: u32 = 1;
/// Boolean value tag.
pub const VELIN_VALUE_BOOLEAN: u32 = 2;
/// UTF-8 string value tag.
pub const VELIN_VALUE_STRING: u32 = 3;
/// Compound value tag; returned values use display text and cannot be resumed.
pub const VELIN_VALUE_COMPOUND: u32 = 4;

/// A fixed-layout value crossing the C boundary.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VelinValue {
    /// One of the `VELIN_VALUE_*` constants.
    pub tag: u32,
    /// Integer payload, used when `tag == VELIN_VALUE_INTEGER`.
    pub integer: i64,
    /// Boolean payload, non-zero for true.
    pub boolean: u8,
    /// Borrowed or owned UTF-8 bytes, depending on the API operation.
    pub text_ptr: *mut u8,
    /// Number of bytes at `text_ptr`.
    pub text_len: usize,
    /// Allocation capacity; pass it to `velin_buffer_free` for owned values.
    pub text_capacity: usize,
}

pub fn to_c_value(value: &Value) -> VelinValue {
    let tag = match value {
        Value::Integer(integer) => {
            return VelinValue {
                tag: VELIN_VALUE_INTEGER,
                integer: *integer,
                boolean: 0,
                text_ptr: std::ptr::null_mut(),
                text_len: 0,
                text_capacity: 0,
            };
        }
        Value::Boolean(boolean) => {
            return VelinValue {
                tag: VELIN_VALUE_BOOLEAN,
                integer: 0,
                boolean: u8::from(*boolean),
                text_ptr: std::ptr::null_mut(),
                text_len: 0,
                text_capacity: 0,
            };
        }
        Value::String(_) => VELIN_VALUE_STRING,
        Value::List(_) | Value::Record(_) => VELIN_VALUE_COMPOUND,
    };
    let (text_ptr, text_len, text_capacity) = owned_buffer(value.to_display().as_bytes());
    VelinValue {
        tag,
        integer: 0,
        boolean: 0,
        text_ptr,
        text_len,
        text_capacity,
    }
}

/// Takes a borrowed C value and validates it before constructing a language
/// value. Compound values are intentionally rejected on resume because their
/// display form is not a stable parser input.
pub unsafe fn from_c_value(value: &VelinValue) -> Result<Value, String> {
    match value.tag {
        VELIN_VALUE_INTEGER => Ok(Value::Integer(value.integer)),
        VELIN_VALUE_BOOLEAN => Ok(Value::Boolean(value.boolean != 0)),
        VELIN_VALUE_STRING => {
            let text = borrowed_text(value.text_ptr, value.text_len)?;
            Ok(Value::String(text.into()))
        }
        VELIN_VALUE_COMPOUND => Err("compound resume values are not supported by the C ABI".into()),
        _ => Err(format!("unknown value tag {}", value.tag)),
    }
}

pub fn owned_buffer(bytes: &[u8]) -> (*mut u8, usize, usize) {
    let mut output = Vec::with_capacity(bytes.len() + 1);
    output.extend_from_slice(bytes);
    output.push(0);
    let ptr = output.as_mut_ptr();
    let len = bytes.len();
    let capacity = output.capacity();
    std::mem::forget(output);
    (ptr, len, capacity)
}

unsafe fn borrowed_text(ptr: *const u8, len: usize) -> Result<String, String> {
    if len == 0 {
        return Ok(String::new());
    }
    if ptr.is_null() {
        return Err("string value pointer is null".into());
    }
    let bytes = unsafe { slice::from_raw_parts(ptr, len) };
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| "string value is not valid UTF-8".to_owned())
}
