use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use std::ptr;
use velin::compile;
use velin_capi::{
    VELIN_BATCH_EFFECTS, VELIN_BATCH_EMPTY, VelinMachine, VelinProgram, velin_batch_free,
    velin_machine_free, velin_machine_new, velin_machine_run_batch, velin_program_free,
    velin_program_load_json,
};

fn load_program(script: &velin::CompiledScript) -> *mut VelinProgram {
    let json = serde_json::to_vec(&*script.program).unwrap();
    let mut error_ptr = ptr::null_mut();
    let mut error_len = 0;
    let mut error_capacity = 0;
    let program = unsafe {
        velin_program_load_json(
            json.as_ptr(),
            json.len(),
            &mut error_ptr,
            &mut error_len,
            &mut error_capacity,
        )
    };
    assert!(!program.is_null(), "C ABI program load failed");
    if !error_ptr.is_null() {
        unsafe { velin_capi::velin_buffer_free(error_ptr, error_len, error_capacity) };
    }
    program
}

fn new_machine(program: *const VelinProgram) -> *mut VelinMachine {
    let mut error_ptr = ptr::null_mut();
    let mut error_len = 0;
    let mut error_capacity = 0;
    let machine = unsafe {
        velin_machine_new(
            program,
            0,
            &mut error_ptr,
            &mut error_len,
            &mut error_capacity,
        )
    };
    assert!(!machine.is_null(), "C ABI machine creation failed");
    if !error_ptr.is_null() {
        unsafe { velin_capi::velin_buffer_free(error_ptr, error_len, error_capacity) };
    }
    machine
}

fn run_batch(machine: *mut VelinMachine) -> usize {
    let mut effects = 0;
    loop {
        let mut batch = unsafe { velin_machine_run_batch(machine, 64) };
        match batch.kind {
            VELIN_BATCH_EFFECTS => {
                effects += batch.effects_len;
                unsafe { velin_batch_free(&mut batch) };
            }
            VELIN_BATCH_EMPTY => {
                unsafe { velin_batch_free(&mut batch) };
                return effects;
            }
            _ => panic!("C ABI batch failed"),
        }
    }
}

fn bench_c_api_batch(c: &mut Criterion) {
    let source = (0..128)
        .map(|index| format!("perform emit({index})\n"))
        .collect::<String>();
    let script = compile("c-api-bench.velin", &source).unwrap();
    let program = load_program(&script);
    c.bench_function("host/c_abi_batch", |b| {
        b.iter(|| {
            let machine = new_machine(program);
            let effects = run_batch(machine);
            unsafe { velin_machine_free(machine) };
            black_box(effects)
        });
    });
    unsafe { velin_program_free(program) };
}

criterion_group!(benches, bench_c_api_batch);
criterion_main!(benches);
