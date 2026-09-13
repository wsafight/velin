use velin_bytecode::{InitialFrame, ValidatedProgram};
use velin_vm::Machine;

/// Opaque validated program handle exposed through the C ABI.
#[repr(C)]
pub struct VelinProgram {
    pub(crate) program: ValidatedProgram,
    pub(crate) initial: InitialFrame,
}

/// Opaque VM instance handle exposed through the C ABI.
#[repr(C)]
pub struct VelinMachine {
    pub(crate) machine: Machine,
    pub(crate) initial: InitialFrame,
}
