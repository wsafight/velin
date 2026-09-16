use super::*;
use velin_bytecode::{ExprChunk, ExprOp, Op, Program, SlotTable};

fn program_json() -> String {
    let mut slots = SlotTable::new();
    let result = slots.intern("result");
    let mut chunk = ExprChunk::new(1);
    let constant = chunk.constant(Value::Integer(42));
    chunk.push(ExprOp::Const {
        dst: chunk.result,
        constant,
    });
    let program = Program::from_chunks(
        vec![
            Op::Set {
                slot: result,
                value: 0,
            },
            Op::Halt,
        ],
        vec![chunk],
        slots,
    );
    serde_json::to_string(&program).unwrap()
}

#[test]
fn runtime_machine_executes_a_program_without_the_source_frontend() {
    let program: Program = serde_json::from_str(&program_json()).unwrap();
    let mut machine = velin_vm::Machine::new(program).unwrap();
    assert_eq!(machine.run(), Ok(Yield::Finished));
}

#[test]
fn results_have_a_small_tagged_wire_shape() {
    let json = yield_to_json(Ok::<_, String>(Yield::Finished));
    assert_eq!(json, "{\"kind\":\"finished\"}");
}

#[test]
fn batch_results_preserve_host_order() {
    let effects = vec![
        HostEffect {
            host_id: 2,
            values: vec![Value::Integer(1)],
        },
        HostEffect {
            host_id: 3,
            values: Vec::new(),
        },
    ];
    let json = batch_to_json(Ok::<_, String>(effects));
    assert!(json.contains("\"host_id\":2"));
    assert!(json.contains("\"host_id\":3"));
}

#[test]
fn runtime_results_serialize_list_and_record_values() {
    let value = Value::Record(std::sync::Arc::new(std::collections::BTreeMap::from([(
        "items".to_owned(),
        Value::List(std::sync::Arc::new(vec![Value::Integer(1)])),
    )])));
    let json = yield_to_json(Ok::<_, String>(Yield::Host {
        host_id: 1,
        values: vec![value],
    }));
    assert!(json.contains(r#""values":[{"items":[1]}]"#));
}
