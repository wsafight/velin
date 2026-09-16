use super::*;

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
fn json_api_marshals_list_and_record_arguments_stably() {
    let json = program_json("perform emit(list(1, record(\"b\", true, \"a\", 2)))\n");
    let loaded = load(&json);
    let mut yielded = unsafe { velin_machine_run_json(loaded.machine) };
    assert_eq!(yielded.kind, VELIN_YIELD_HOST);
    let values = unsafe { std::slice::from_raw_parts(yielded.values, yielded.values_len) };
    assert_eq!(values.len(), 1);
    assert_eq!(values[0].tag, VELIN_VALUE_JSON);
    let text = unsafe { std::slice::from_raw_parts(values[0].text_ptr, values[0].text_len) };
    assert_eq!(text, br#"[1,{"a":2,"b":true}]"#);
    unsafe {
        velin_yield_free(&mut yielded);
        free_loaded(loaded);
    }
}

#[test]
fn c_artifact_executes_p2_collection_and_loop_syntax() {
    let json = program_json(
        "set total = 0\nfor item in [1, 2, 3]:\n    total += item\nperform emit({total: total})\n",
    );
    let loaded = load(&json);
    let mut yielded = unsafe { velin_machine_run_json(loaded.machine) };
    assert_eq!(yielded.kind, VELIN_YIELD_HOST);
    let values = unsafe { std::slice::from_raw_parts(yielded.values, yielded.values_len) };
    let text = unsafe { std::slice::from_raw_parts(values[0].text_ptr, values[0].text_len) };
    assert_eq!(text, br#"{"total":6}"#);
    unsafe {
        velin_yield_free(&mut yielded);
        free_loaded(loaded);
    }
}

#[test]
fn json_api_resumes_compounds_and_preserves_them_at_the_next_effect() {
    let json = program_json("answer = perform ask()\nperform emit(answer)\n");
    let loaded = load(&json);
    let mut first = unsafe { velin_machine_run_json(loaded.machine) };
    assert_eq!(first.kind, VELIN_YIELD_HOST);
    unsafe { velin_yield_free(&mut first) };

    let mut json = br#"[1,{"a":true}]"#.to_vec();
    let value = VelinValue {
        tag: VELIN_VALUE_JSON,
        integer: 0,
        boolean: 0,
        text_ptr: json.as_mut_ptr(),
        text_len: json.len(),
        text_capacity: 0,
    };
    let mut second = unsafe { velin_machine_resume_json(loaded.machine, &value) };
    assert_eq!(second.kind, VELIN_YIELD_HOST);
    let values = unsafe { std::slice::from_raw_parts(second.values, second.values_len) };
    assert_eq!(values[0].tag, VELIN_VALUE_JSON);
    let text = unsafe { std::slice::from_raw_parts(values[0].text_ptr, values[0].text_len) };
    assert_eq!(text, json.as_slice());
    unsafe {
        velin_yield_free(&mut second);
        free_loaded(loaded);
    }
}

#[test]
fn json_batch_and_restart_use_the_opt_in_encoding() {
    let json = program_json("perform emit(list(1))\nperform emit(record(\"x\", 2))\n");
    let loaded = load(&json);
    let mut batch = unsafe { velin_machine_run_batch_json(loaded.machine, 8) };
    assert_eq!(batch.kind, VELIN_BATCH_EFFECTS);
    let effects = unsafe { std::slice::from_raw_parts(batch.effects, batch.effects_len) };
    let first = unsafe { std::slice::from_raw_parts(effects[0].values, effects[0].values_len) };
    let second = unsafe { std::slice::from_raw_parts(effects[1].values, effects[1].values_len) };
    assert_eq!(first[0].tag, VELIN_VALUE_JSON);
    assert_eq!(second[0].tag, VELIN_VALUE_JSON);
    unsafe { velin_batch_free(&mut batch) };

    let mut restarted = unsafe { velin_machine_restart_json(loaded.machine, 1) };
    assert_eq!(restarted.kind, VELIN_YIELD_HOST);
    let values = unsafe { std::slice::from_raw_parts(restarted.values, restarted.values_len) };
    assert_eq!(values[0].tag, VELIN_VALUE_JSON);
    unsafe {
        velin_yield_free(&mut restarted);
        free_loaded(loaded);
    }
}

#[test]
fn json_resume_rejects_invalid_and_oversized_payloads() {
    let mut invalid = b"null".to_vec();
    let value = VelinValue {
        tag: VELIN_VALUE_JSON,
        integer: 0,
        boolean: 0,
        text_ptr: invalid.as_mut_ptr(),
        text_len: invalid.len(),
        text_capacity: 0,
    };
    assert!(unsafe { value::from_c_value(&value) }.is_err());

    let value = VelinValue {
        text_len: 1024 * 1024 + 1,
        ..value
    };
    assert!(
        unsafe { value::from_c_value(&value) }
            .unwrap_err()
            .contains("1 MiB")
    );
}
