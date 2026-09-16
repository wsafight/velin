use super::*;
use velin_syntax::Value;

const HOST_PROGRAM: &[u8] = br#"{"ops":[{"Host":{"host_id":7,"args":[],"bind":0,"line":1}},"Halt"],"chunks":[],"slots":["answer"]}"#;
const BATCH_PROGRAM: &[u8] = br#"{"ops":[{"Host":{"host_id":1,"args":[],"bind":null,"line":1}},{"Host":{"host_id":2,"args":[],"bind":null,"line":2}},"Halt"],"chunks":[],"slots":[]}"#;

struct Loaded {
    program: *mut VelinProgram,
    machine: *mut VelinMachine,
}

fn load(json: &[u8]) -> Loaded {
    let mut error = std::ptr::null_mut();
    let mut error_len = 0;
    let mut error_capacity = 0;
    let program = unsafe {
        velin_program_load_json(
            json.as_ptr(),
            json.len(),
            &mut error,
            &mut error_len,
            &mut error_capacity,
        )
    };
    assert!(!program.is_null(), "load failed");
    let machine =
        unsafe { velin_machine_new(program, 0, &mut error, &mut error_len, &mut error_capacity) };
    assert!(!machine.is_null());
    Loaded { program, machine }
}

fn program_json(source: &str) -> Vec<u8> {
    let script = velin::compile("c.velin", source).expect("compile");
    serde_json::to_vec(&*script.program).expect("serialize")
}

unsafe fn free_loaded(loaded: Loaded) {
    unsafe {
        velin_machine_free(loaded.machine);
        velin_program_free(loaded.program);
    }
}

#[test]
fn reports_the_exported_abi_version() {
    assert_eq!(velin_c_api_version(), VELIN_C_API_VERSION);
}

#[test]
fn host_yield_and_scalar_resume_cross_the_c_boundary() {
    let loaded = load(HOST_PROGRAM);
    let mut yielded = unsafe { velin_machine_run(loaded.machine) };
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
    let mut finished = unsafe { velin_machine_resume(loaded.machine, &answer) };
    assert_eq!(finished.kind, VELIN_YIELD_FINISHED);
    unsafe {
        velin_yield_free(&mut finished);
        free_loaded(loaded);
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

    let program = unsafe {
        velin_program_load_json(
            std::ptr::null(),
            4,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    assert!(program.is_null());
}

#[test]
fn invalid_json_and_invalid_programs_report_errors() {
    let mut error = std::ptr::null_mut();
    let mut error_len = 0;
    let mut error_capacity = 0;
    let program = unsafe {
        velin_program_load_json(
            b"{".as_ptr(),
            1,
            &mut error,
            &mut error_len,
            &mut error_capacity,
        )
    };
    assert!(program.is_null());
    unsafe { velin_buffer_free(error, error_len, error_capacity) };

    let invalid = br#"{"ops":[{"Jump":99}],"chunks":[],"slots":[]}"#;
    let program = unsafe {
        velin_program_load_json(
            invalid.as_ptr(),
            invalid.len(),
            &mut error,
            &mut error_len,
            &mut error_capacity,
        )
    };
    assert!(program.is_null());
    unsafe { velin_buffer_free(error, error_len, error_capacity) };
}

#[test]
fn null_handles_return_errors_and_null_frees_are_accepted() {
    let mut error = std::ptr::null_mut();
    let mut error_len = 0;
    let mut error_capacity = 0;
    let machine = unsafe {
        velin_machine_new(
            std::ptr::null(),
            0,
            &mut error,
            &mut error_len,
            &mut error_capacity,
        )
    };
    assert!(machine.is_null());
    unsafe { velin_buffer_free(error, error_len, error_capacity) };

    let mut run = unsafe { velin_machine_run(std::ptr::null_mut()) };
    assert_eq!(run.kind, VELIN_YIELD_ERROR);
    unsafe { velin_yield_free(&mut run) };

    let mut batch = unsafe { velin_machine_run_batch(std::ptr::null_mut(), 1) };
    assert_eq!(batch.kind, VELIN_BATCH_ERROR);
    unsafe { velin_batch_free(&mut batch) };

    let mut resume = unsafe { velin_machine_resume(std::ptr::null_mut(), std::ptr::null()) };
    assert_eq!(resume.kind, VELIN_YIELD_ERROR);
    unsafe { velin_yield_free(&mut resume) };

    let mut restart = unsafe { velin_machine_restart(std::ptr::null_mut(), 0) };
    assert_eq!(restart.kind, VELIN_YIELD_ERROR);
    unsafe { velin_yield_free(&mut restart) };

    let mut run_json = unsafe { velin_machine_run_json(std::ptr::null_mut()) };
    assert_eq!(run_json.kind, VELIN_YIELD_ERROR);
    unsafe { velin_yield_free(&mut run_json) };

    let mut batch_json = unsafe { velin_machine_run_batch_json(std::ptr::null_mut(), 1) };
    assert_eq!(batch_json.kind, VELIN_BATCH_ERROR);
    unsafe { velin_batch_free(&mut batch_json) };

    let mut resume_json =
        unsafe { velin_machine_resume_json(std::ptr::null_mut(), std::ptr::null()) };
    assert_eq!(resume_json.kind, VELIN_YIELD_ERROR);
    unsafe { velin_yield_free(&mut resume_json) };

    let mut restart_json = unsafe { velin_machine_restart_json(std::ptr::null_mut(), 0) };
    assert_eq!(restart_json.kind, VELIN_YIELD_ERROR);
    unsafe { velin_yield_free(&mut restart_json) };

    unsafe {
        velin_program_free(std::ptr::null_mut());
        velin_machine_free(std::ptr::null_mut());
        velin_yield_free(std::ptr::null_mut());
        velin_batch_free(std::ptr::null_mut());
        velin_buffer_free(std::ptr::null_mut(), 0, 0);
    }
}

#[test]
fn host_arguments_cover_every_c_value_shape() {
    let json = program_json("perform emit(1, true, \"hi\", list(1))\n");
    let loaded = load(&json);
    let mut yielded = unsafe { velin_machine_run(loaded.machine) };
    assert_eq!(yielded.kind, VELIN_YIELD_HOST);
    assert_eq!(yielded.values_len, 4);
    let values = unsafe { std::slice::from_raw_parts(yielded.values, yielded.values_len) };
    assert_eq!(values[0].tag, VELIN_VALUE_INTEGER);
    assert_eq!(values[1].tag, VELIN_VALUE_BOOLEAN);
    assert_eq!(values[2].tag, VELIN_VALUE_STRING);
    assert_eq!(values[3].tag, VELIN_VALUE_COMPOUND);
    unsafe {
        velin_yield_free(&mut yielded);
        free_loaded(loaded);
    }
}

#[test]
fn resume_accepts_scalars_and_rejects_compound_or_invalid_tags() {
    let loaded = load(HOST_PROGRAM);
    let mut yielded = unsafe { velin_machine_run(loaded.machine) };
    unsafe { velin_yield_free(&mut yielded) };

    let mut text = b"ok".to_vec();
    let string = VelinValue {
        tag: VELIN_VALUE_STRING,
        integer: 0,
        boolean: 0,
        text_ptr: text.as_mut_ptr(),
        text_len: text.len(),
        text_capacity: 0,
    };
    let mut finished = unsafe { velin_machine_resume(loaded.machine, &string) };
    assert_eq!(finished.kind, VELIN_YIELD_FINISHED);
    unsafe { velin_yield_free(&mut finished) };

    let loaded = load(HOST_PROGRAM);
    let mut yielded = unsafe { velin_machine_run(loaded.machine) };
    unsafe { velin_yield_free(&mut yielded) };
    let boolean = VelinValue {
        tag: VELIN_VALUE_BOOLEAN,
        integer: 0,
        boolean: 1,
        text_ptr: std::ptr::null_mut(),
        text_len: 0,
        text_capacity: 0,
    };
    let mut finished = unsafe { velin_machine_resume(loaded.machine, &boolean) };
    assert_eq!(finished.kind, VELIN_YIELD_FINISHED);
    unsafe {
        velin_yield_free(&mut finished);
        free_loaded(loaded);
    }

    let loaded = load(HOST_PROGRAM);
    let mut yielded = unsafe { velin_machine_run(loaded.machine) };
    unsafe { velin_yield_free(&mut yielded) };
    let compound = VelinValue {
        tag: VELIN_VALUE_COMPOUND,
        integer: 0,
        boolean: 0,
        text_ptr: std::ptr::null_mut(),
        text_len: 0,
        text_capacity: 0,
    };
    let mut error = unsafe { velin_machine_resume(loaded.machine, &compound) };
    assert_eq!(error.kind, VELIN_YIELD_ERROR);
    unsafe {
        velin_yield_free(&mut error);
        free_loaded(loaded);
    }

    let loaded = load(HOST_PROGRAM);
    let mut yielded = unsafe { velin_machine_run(loaded.machine) };
    unsafe { velin_yield_free(&mut yielded) };
    let unknown = VelinValue {
        tag: 99,
        integer: 0,
        boolean: 0,
        text_ptr: std::ptr::null_mut(),
        text_len: 0,
        text_capacity: 0,
    };
    let mut error = unsafe { velin_machine_resume(loaded.machine, &unknown) };
    assert_eq!(error.kind, VELIN_YIELD_ERROR);
    unsafe {
        velin_yield_free(&mut error);
        free_loaded(loaded);
    }
}

#[test]
fn resume_string_values_reject_null_pointers_and_invalid_utf8() {
    let loaded = load(HOST_PROGRAM);
    let mut yielded = unsafe { velin_machine_run(loaded.machine) };
    unsafe { velin_yield_free(&mut yielded) };
    let empty = VelinValue {
        tag: VELIN_VALUE_STRING,
        integer: 0,
        boolean: 0,
        text_ptr: std::ptr::null_mut(),
        text_len: 0,
        text_capacity: 0,
    };
    let mut finished = unsafe { velin_machine_resume(loaded.machine, &empty) };
    assert_eq!(finished.kind, VELIN_YIELD_FINISHED);
    unsafe {
        velin_yield_free(&mut finished);
        free_loaded(loaded);
    }

    let loaded = load(HOST_PROGRAM);
    let mut yielded = unsafe { velin_machine_run(loaded.machine) };
    unsafe { velin_yield_free(&mut yielded) };
    let null_text = VelinValue {
        tag: VELIN_VALUE_STRING,
        integer: 0,
        boolean: 0,
        text_ptr: std::ptr::null_mut(),
        text_len: 3,
        text_capacity: 0,
    };
    let mut error = unsafe { velin_machine_resume(loaded.machine, &null_text) };
    assert_eq!(error.kind, VELIN_YIELD_ERROR);
    unsafe {
        velin_yield_free(&mut error);
        free_loaded(loaded);
    }

    let loaded = load(HOST_PROGRAM);
    let mut yielded = unsafe { velin_machine_run(loaded.machine) };
    unsafe { velin_yield_free(&mut yielded) };
    let mut invalid = [0xff_u8, 0xfe];
    let utf8 = VelinValue {
        tag: VELIN_VALUE_STRING,
        integer: 0,
        boolean: 0,
        text_ptr: invalid.as_mut_ptr(),
        text_len: invalid.len(),
        text_capacity: 0,
    };
    let mut error = unsafe { velin_machine_resume(loaded.machine, &utf8) };
    assert_eq!(error.kind, VELIN_YIELD_ERROR);
    unsafe {
        velin_yield_free(&mut error);
        free_loaded(loaded);
    }
}

#[test]
fn batch_returns_ordered_side_effects_and_can_be_freed() {
    let loaded = load(BATCH_PROGRAM);
    let mut batch = unsafe { velin_machine_run_batch(loaded.machine, 8) };
    assert_eq!(batch.kind, VELIN_BATCH_EFFECTS);
    assert_eq!(batch.effects_len, 2);
    let effects = unsafe { std::slice::from_raw_parts(batch.effects, batch.effects_len) };
    assert_eq!(effects[0].host_id, 1);
    assert_eq!(effects[1].host_id, 2);
    unsafe {
        velin_batch_free(&mut batch);
        free_loaded(loaded);
    }
    assert_eq!(batch.kind, VELIN_BATCH_EMPTY);
}

#[test]
fn batch_string_arguments_are_freed_with_the_batch() {
    let json = program_json("perform emit(\"one\")\nperform emit(\"two\")\n");
    let loaded = load(&json);
    let mut batch = unsafe { velin_machine_run_batch(loaded.machine, 8) };
    assert_eq!(batch.kind, VELIN_BATCH_EFFECTS);
    unsafe {
        velin_batch_free(&mut batch);
        free_loaded(loaded);
    }
}

#[test]
fn empty_batches_runtime_errors_and_restart_are_covered() {
    let json = program_json("set value = 1\n");
    let loaded = load(&json);
    let mut batch = unsafe { velin_machine_run_batch(loaded.machine, 4) };
    assert_eq!(batch.kind, VELIN_BATCH_EMPTY);
    unsafe {
        velin_batch_free(&mut batch);
        free_loaded(loaded);
    }

    let json = program_json("set value = 1 / 0\n");
    let loaded = load(&json);
    let mut failed = unsafe { velin_machine_run(loaded.machine) };
    assert_eq!(failed.kind, VELIN_YIELD_ERROR);
    unsafe {
        velin_yield_free(&mut failed);
        free_loaded(loaded);
    }

    let json = program_json("perform emit(\"hi\")\n");
    let loaded = load(&json);
    let mut yielded = unsafe { velin_machine_run(loaded.machine) };
    unsafe { velin_yield_free(&mut yielded) };
    let mut finished = unsafe { velin_machine_resume(loaded.machine, std::ptr::null()) };
    assert_eq!(finished.kind, VELIN_YIELD_FINISHED);
    unsafe { velin_yield_free(&mut finished) };
    let mut restarted = unsafe { velin_machine_restart(loaded.machine, 1) };
    assert_eq!(restarted.kind, VELIN_YIELD_HOST);
    unsafe {
        velin_yield_free(&mut restarted);
        free_loaded(loaded);
    }
}

#[test]
fn value_helpers_round_trip_language_values() {
    let integer = value::to_c_value(&Value::Integer(3));
    assert_eq!(integer.tag, VELIN_VALUE_INTEGER);
    let boolean = value::to_c_value(&Value::Boolean(true));
    assert_eq!(boolean.tag, VELIN_VALUE_BOOLEAN);
    let string = value::to_c_value(&Value::String("hi".into()));
    assert_eq!(string.tag, VELIN_VALUE_STRING);
    let list = value::to_c_value(&Value::List(std::sync::Arc::new(vec![Value::Integer(1)])));
    assert_eq!(list.tag, VELIN_VALUE_COMPOUND);
    unsafe {
        velin_buffer_free(string.text_ptr, string.text_len, string.text_capacity);
        velin_buffer_free(list.text_ptr, list.text_len, list.text_capacity);
    }
}
