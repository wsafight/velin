use std::sync::Arc;
use velin_bytecode::Program;
use velin_vm::Machine;

/// Opaque validated program handle exposed through the C ABI.
#[repr(C)]
pub struct VelinProgram {
    pub(crate) program: Arc<Program>,
}

/// Opaque VM instance handle exposed through the C ABI.
#[repr(C)]
pub struct VelinMachine {
    pub(crate) machine: Machine,
}
