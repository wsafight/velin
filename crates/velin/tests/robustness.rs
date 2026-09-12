//! Property tests for the untrusted parser, bytecode, and execution boundaries.

use proptest::prelude::*;
use std::collections::BTreeMap;
use velin::{
    BinaryOp, Builtin, Expr, Machine, Program, ProgramBuilder, UnaryOp, Value, Yield, compile,
    evaluate_with_rng, parse_expression,
};

fn value() -> impl Strategy<Value = Value> {
    prop_oneof![
        (-1000_i64..=1000).prop_map(Value::Integer),
        any::<bool>().prop_map(Value::Boolean),
        "[a-z]{0,16}".prop_map(|value| Value::String(value.into())),
    ]
}

fn unary() -> impl Strategy<Value = UnaryOp> {
    any::<bool>().prop_map(|negate| {
        if negate {
            UnaryOp::Negate
        } else {
            UnaryOp::Not
        }
    })
}

fn binary() -> impl Strategy<Value = BinaryOp> {
    (0_u8..12).prop_map(|index| match index {
        0 => BinaryOp::Add,
        1 => BinaryOp::Subtract,
        2 => BinaryOp::Multiply,
        3 => BinaryOp::Divide,
        4 => BinaryOp::Equal,
        5 => BinaryOp::NotEqual,
        6 => BinaryOp::Less,
        7 => BinaryOp::LessEqual,
        8 => BinaryOp::Greater,
        9 => BinaryOp::GreaterEqual,
        10 => BinaryOp::And,
        _ => BinaryOp::Or,
    })
}

fn expression() -> impl Strategy<Value = Expr> {
    let leaf = prop_oneof![
        value().prop_map(Expr::Value),
        prop_oneof![Just("number"), Just("flag"), Just("text"), Just("missing")]
            .prop_map(|name| Expr::Variable(name.to_owned())),
    ];
    leaf.prop_recursive(5, 96, 4, |inner| {
        let unary_call = (
            prop_oneof![Just(Builtin::Len), Just(Builtin::Chance)],
            inner.clone(),
        )
            .prop_map(|(function, argument)| Expr::Invoke {
                function,
                arguments: vec![argument],
            });
        let binary_call = (
            prop_oneof![
                Just(Builtin::Push),
                Just(Builtin::Remove),
                Just(Builtin::Contains),
                Just(Builtin::Random),
            ],
            inner.clone(),
            inner.clone(),
        )
            .prop_map(|(function, first, second)| Expr::Invoke {
                function,
                arguments: vec![first, second],
            });
        let ternary_call =
            (inner.clone(), inner.clone(), inner.clone()).prop_map(|(first, second, third)| {
                Expr::Invoke {
                    function: Builtin::Put,
                    arguments: vec![first, second, third],
                }
            });
        prop_oneof![
            (unary(), inner.clone()).prop_map(|(op, value)| Expr::Unary {
                op,
                value: Box::new(value),
            }),
            (inner.clone(), binary(), inner.clone()).prop_map(|(left, op, right)| Expr::Binary {
                left: Box::new(left),
                op,
                right: Box::new(right),
            }),
            prop::collection::vec(inner.clone(), 0..=4).prop_map(|arguments| Expr::Invoke {
                function: Builtin::List,
                arguments,
            }),
            unary_call,
            binary_call,
            ternary_call,
        ]
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn arbitrary_text_never_panics_the_source_frontends(source in any::<String>()) {
        let _ = parse_expression(&source, "property.velin", 1, 1);
        let _ = compile("property.velin", &source);
    }

    #[test]
    fn arbitrary_json_never_bypasses_program_validation(source in any::<String>()) {
        if let Ok(program) = serde_json::from_str::<Program>(&source) {
            prop_assert!(program.validate().is_ok());
            prop_assert!(Machine::new(program).is_ok());
        }
    }

    #[test]
    fn generated_expressions_match_between_tree_walker_and_vm(expr in expression()) {
        let variables = BTreeMap::from([
            ("number".to_owned(), Value::Integer(7)),
            ("flag".to_owned(), Value::Boolean(true)),
            ("text".to_owned(), Value::String("velin".into())),
        ]);
        let mut walk_state = 41;
        let walked = evaluate_with_rng(&expr, &variables, &mut walk_state, 1);

        let mut builder = ProgramBuilder::new();
        let result_slot = builder.slot("__property_result");
        let chunk = builder.expr(&expr, 1);
        builder.push(velin::Op::Set {
            slot: result_slot,
            value: chunk,
        });
        let mut machine = Machine::with_seed(builder.build(), 41).unwrap();
        for (name, value) in variables {
            machine.set_variable(&name, value);
        }
        let executed = match machine.run() {
            Ok(Yield::Finished) => Ok(machine.variable("__property_result").unwrap().clone()),
            Ok(Yield::Host { .. }) => unreachable!("generated expressions contain no host ops"),
            Err(error) => Err(error),
        };

        prop_assert_eq!(walked, executed);
        if let Some(machine_state) = machine.rng_state() {
            prop_assert_eq!(walk_state, machine_state);
        }
    }
}
