//! End-to-end pipeline test: parse → check → compile → run, driven through a
//! toy host. This exercises the facade the way a real embedder would.

use velin::{
    Environment, Expr, Machine, Op, ProgramBuilder, Value, Yield, check_expression,
    parse_expression,
};

/// A tiny host: it logs each effect and answers "ask" effects with a fixed
/// choice, standing in for a UI the embedder would drive.
#[derive(Default)]
struct ToyHost {
    log: Vec<String>,
}

impl ToyHost {
    fn drive(&mut self, machine: &mut Machine) {
        let mut outcome = machine.run().unwrap();
        loop {
            match outcome {
                Yield::Finished => break,
                Yield::Host { host_id, values } => {
                    self.log.push(format!("effect {host_id}: {values:?}"));
                    // host_id 1 = "say" (no return), host_id 2 = "ask" (returns 1).
                    let resume = if host_id == 2 {
                        Some(Value::Integer(1))
                    } else {
                        None
                    };
                    outcome = machine.resume(resume).unwrap();
                }
            }
        }
    }
}

#[test]
fn parse_check_compile_run_round_trip() {
    // A hand-built program standing in for compiled host source:
    //
    //   default hp = 30
    //   say("HP is {hp}")            # host effect 1
    //   choice = ask("rest?")        # host effect 2, returns 1
    //   if choice == 1:
    //       hp = hp + 10
    //   say("done")                  # host effect 1

    // 1. Type-check the guard expression the way a checker pass would.
    let guard_src = "choice == 1";
    let guard_expr = parse_expression(guard_src, "adventure.ql", 4, 8).unwrap();
    let mut env = Environment::new();
    env.insert("choice".into(), velin::Type::Integer);
    assert!(check_expression(&guard_expr, &env, "adventure.ql", 4, 8).is_empty());

    // 2. Build the program.
    let mut b = ProgramBuilder::new();
    let hp = b.slot("hp");
    let choice = b.slot("choice");

    let say_hp = b.expr(&Expr::Value(Value::String("HP is 30".into())), 1);
    b.push(Op::host(1, vec![say_hp], None, 1));

    let ask = b.expr(&Expr::Value(Value::String("rest?".into())), 3);
    b.push(Op::host(2, vec![ask], Some(choice), 3));

    let guard = b.expr(&guard_expr, 4);
    let skip = b.push(Op::JumpIfFalse {
        condition: guard,
        target: u32::MAX,
    });
    let heal = b.expr(
        &Expr::Binary {
            left: Box::new(Expr::Variable("hp".into())),
            op: velin::BinaryOp::Add,
            right: Box::new(Expr::Value(Value::Integer(10))),
        },
        5,
    );
    b.push(Op::Set {
        slot: hp,
        value: heal,
    });
    let after = b.here();
    b.patch(
        skip,
        Op::JumpIfFalse {
            condition: guard,
            target: after,
        },
    );

    let done = b.expr(&Expr::Value(Value::String("done".into())), 6);
    b.push(Op::host(1, vec![done], None, 6));

    let program = b.build();

    // 3. Seed initial state and run through the host.
    let mut machine = Machine::new(program).unwrap();
    machine.set_variable("hp", Value::Integer(30));
    let mut host = ToyHost::default();
    host.drive(&mut machine);

    // choice was 1, so the branch ran: 30 + 10 = 40.
    assert_eq!(machine.variable("hp"), Some(&Value::Integer(40)));
    assert_eq!(machine.variable("choice"), Some(&Value::Integer(1)));
    assert_eq!(host.log.len(), 3); // say, ask, say
    assert!(host.log[0].contains("effect 1"));
    assert!(host.log[1].contains("effect 2"));
}
