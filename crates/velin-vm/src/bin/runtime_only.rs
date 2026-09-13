//! Minimal runtime-only embedding example.
//!
//! Build with `cargo run -p velin-vm --no-default-features --bin runtime_only`.

use velin_bytecode::{ExprChunk, ExprOp, Op, Program, SlotTable};
use velin_syntax::{BinaryOp, Value};
use velin_vm::{Machine, Yield};

fn main() {
    let mut slots = SlotTable::new();
    let answer = slots.intern("answer");

    let mut expression = ExprChunk::new(1);
    let left = expression.constant(Value::Integer(40));
    let right = expression.constant(Value::Integer(2));
    let right_register = expression.register();
    expression.push(ExprOp::Const {
        dst: expression.result,
        constant: left,
    });
    expression.push(ExprOp::Const {
        dst: right_register,
        constant: right,
    });
    expression.push(ExprOp::Binary {
        dst: expression.result,
        left: expression.result,
        op: BinaryOp::Add,
        right: right_register,
    });

    let program = Program::from_chunks(
        vec![
            Op::Set {
                slot: answer,
                value: 0,
            },
            Op::Halt,
        ],
        vec![expression],
        slots,
    );
    let mut machine = Machine::new(program).expect("the embedded program is valid");
    assert_eq!(
        machine.run().expect("runtime execution succeeds"),
        Yield::Finished
    );
    println!(
        "{}",
        machine
            .variable("answer")
            .expect("answer is assigned")
            .to_display()
    );
}
