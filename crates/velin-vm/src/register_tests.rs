use super::*;
use velin_compile::ProgramBuilder;
use velin_syntax::Expr;

#[test]
fn register_plan_evaluates_long_scalar_expression() {
    let mut builder = ProgramBuilder::new();
    let _left = builder.slot("left");
    let _right = builder.slot("right");
    let target = builder.slot("target");
    let mut expression = Expr::Variable("left".into());
    for _ in 0..8 {
        expression = Expr::Binary {
            left: Box::new(expression),
            op: velin_syntax::BinaryOp::Add,
            right: Box::new(Expr::Variable("right".into())),
        };
    }
    let value = builder.expr(&expression, 1);
    builder.push(Op::Set {
        slot: target,
        value,
    });
    let mut machine = Machine::new(builder.build()).unwrap();
    machine.set_variable("left", Value::Integer(3));
    machine.set_variable("right", Value::Integer(4));
    assert_eq!(machine.run().unwrap(), Yield::Finished);
    assert_eq!(machine.variable("target"), Some(&Value::Integer(35)));
}

#[test]
fn register_plan_preserves_operator_errors() {
    let mut builder = ProgramBuilder::new();
    let _left = builder.slot("left");
    let _right = builder.slot("right");
    let target = builder.slot("target");
    let mut expression = Expr::Variable("left".into());
    for _ in 0..6 {
        expression = Expr::Binary {
            left: Box::new(expression),
            op: velin_syntax::BinaryOp::Add,
            right: Box::new(Expr::Variable("right".into())),
        };
    }
    let expression = Expr::Binary {
        left: Box::new(expression),
        op: velin_syntax::BinaryOp::Divide,
        right: Box::new(Expr::Variable("right".into())),
    };
    let value = builder.expr(&expression, 4);
    builder.push(Op::Set {
        slot: target,
        value,
    });
    let mut machine = Machine::new(builder.build()).unwrap();
    machine.set_variable("left", Value::Integer(1));
    machine.set_variable("right", Value::Integer(0));
    assert!(machine.run().unwrap_err().message.contains("zero"));
}
