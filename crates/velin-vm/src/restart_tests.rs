use super::*;
use velin_compile::{InitialFrame, ProgramBuilder};
use velin_syntax::{Builtin, Expr};

#[test]
fn restart_reuses_a_machine_and_resets_rng_state() {
    let mut builder = ProgramBuilder::new();
    let value = builder.slot("value");
    let random = builder.expr(
        &Expr::Invoke {
            function: Builtin::Random,
            arguments: vec![
                Expr::Value(Value::Integer(1)),
                Expr::Value(Value::Integer(100)),
            ],
        },
        1,
    );
    builder.push(Op::Set {
        slot: value,
        value: random,
    });
    let program = builder.build();
    let initial = InitialFrame::from_named_values(&program.slots, []).unwrap();
    let mut machine = Machine::with_seed(program, 7).unwrap();
    machine.run().unwrap();
    let first = machine.variable("value").cloned();
    machine.restart(&initial, 7).unwrap();
    assert_eq!(machine.rng_state(), Some(7));
    machine.run().unwrap();
    assert_eq!(machine.variable("value").cloned(), first);
}
