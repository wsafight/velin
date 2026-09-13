use super::*;
use velin_compile::ProgramBuilder;
use velin_syntax::{BinaryOp, Expr};

#[test]
fn batches_side_effects_and_stops_before_bound_hosts() {
    let mut builder = ProgramBuilder::new();
    let answer = builder.slot("answer");
    builder.push(Op::host(1, Vec::new(), None, 1));
    builder.push(Op::host(2, Vec::new(), None, 2));
    builder.push(Op::host(3, Vec::new(), Some(answer), 3));
    let mut machine = Machine::new(builder.build()).unwrap();

    let effects = machine.run_effect_batch(8).unwrap();
    assert_eq!(
        effects,
        vec![
            HostEffect {
                host_id: 1,
                values: Vec::new(),
            },
            HostEffect {
                host_id: 2,
                values: Vec::new(),
            },
        ]
    );
    assert!(matches!(
        machine.run().unwrap(),
        Yield::Host { host_id: 3, values } if values.is_empty()
    ));
    assert!(machine.run_effect_batch(8).is_err());
    assert_eq!(
        machine.resume(Some(Value::Integer(7))).unwrap(),
        Yield::Finished
    );
}

#[test]
fn batch_limit_preserves_order_and_finished_state() {
    let mut builder = ProgramBuilder::new();
    builder.push(Op::host(1, Vec::new(), None, 1));
    builder.push(Op::host(2, Vec::new(), None, 2));
    let mut machine = Machine::new(builder.build()).unwrap();

    assert_eq!(machine.run_effect_batch(0).unwrap_err().line, 1);
    assert_eq!(machine.run_effect_batch(1).unwrap()[0].host_id, 1);
    assert_eq!(machine.run_effect_batch(1).unwrap()[0].host_id, 2);
    assert!(machine.run_effect_batch(1).unwrap().is_empty());
    assert_eq!(machine.run().unwrap(), Yield::Finished);
}

#[test]
fn reusable_batch_buffer_can_be_drained_without_losing_capacity() {
    let mut builder = ProgramBuilder::new();
    builder.push(Op::host(1, Vec::new(), None, 1));
    builder.push(Op::host(2, Vec::new(), None, 2));
    let mut machine = Machine::new(builder.build()).unwrap();

    assert_eq!(machine.run_effect_batch_reusable(2).unwrap(), 2);
    assert_eq!(machine.effect_batch()[1].host_id, 2);
    let mut drained = Vec::new();
    machine.drain_effect_batch(&mut drained);
    assert_eq!(drained.len(), 2);
    assert!(machine.effect_batch().is_empty());

    builder = ProgramBuilder::new();
    builder.push(Op::host(3, Vec::new(), None, 3));
    let mut next = Machine::new(builder.build()).unwrap();
    assert_eq!(next.run_effect_batch_reusable(1).unwrap(), 1);
    assert_eq!(next.effect_batch()[0].host_id, 3);
}

#[test]
fn batch_delivers_collected_effects_before_reporting_an_error() {
    let mut builder = ProgramBuilder::new();
    let error_argument = builder.expr(
        &Expr::Binary {
            left: Box::new(Expr::Value(Value::Integer(1))),
            op: BinaryOp::Divide,
            right: Box::new(Expr::Value(Value::Integer(0))),
        },
        2,
    );
    builder.push(Op::host(1, Vec::new(), None, 1));
    builder.push(Op::host(2, vec![error_argument], None, 2));
    let mut machine = Machine::new(builder.build()).unwrap();

    let effects = machine.run_effect_batch(8).unwrap();
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].host_id, 1);

    let error = machine.run_effect_batch(8).unwrap_err();
    assert_eq!(error.line, 2);
    assert!(error.message.contains("division by zero"));
    assert_eq!(machine.run_effect_batch(8), Err(error));
}

#[test]
fn batch_caps_preallocation_and_records_host_profile_hits() {
    let mut builder = ProgramBuilder::new();
    builder.push(Op::host(1, Vec::new(), None, 1));
    builder.push(Op::Halt);
    let mut machine = Machine::new(builder.build()).unwrap();

    assert_eq!(machine.run_effect_batch(usize::MAX).unwrap().len(), 1);
    assert_eq!(machine.profile().op_hits()[0], 1);
}
