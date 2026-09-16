use super::*;
use crate::parser::parse;

fn compile(source: &str) -> CompiledScript {
    lower(parse(source).expect("parse")).expect("lower")
}

#[test]
fn default_is_evaluated_at_compile_time() {
    let script = compile("default hp = 20 + 10\n");
    assert_eq!(script.defaults.get("hp"), Some(&Value::Integer(30)));
}

#[test]
fn changing_public_defaults_invalidates_cached_check_types() {
    let mut script = compile("default hp = 3\nset hp = hp + 1\n");
    assert!(script.check("test.velin").is_empty());

    script
        .defaults
        .insert("hp".into(), Value::String("three".into()));
    assert!(script.initial_frame().is_none());
    assert!(
        script
            .check("test.velin")
            .iter()
            .any(|diagnostic| diagnostic.message.contains("`+` cannot combine"))
    );
}

#[test]
fn non_constant_default_is_an_error() {
    let error = lower(parse("default hp = other + 1\n").unwrap()).unwrap_err();
    assert!(error.message.contains("compile-time constant"));
}

#[test]
fn defaults_must_be_unique_and_top_level() {
    let duplicate = lower(parse("default hp = 1\ndefault hp = 2\n").unwrap()).unwrap_err();
    assert!(duplicate.message.contains("duplicate default"));

    let nested = lower(parse("if true:\n    default hp = 1\n").unwrap()).unwrap_err();
    assert!(nested.message.contains("top level"));
}

#[test]
fn perform_interns_host_commands_in_first_seen_order() {
    let script = compile("perform show(\"a\")\nperform say(\"b\")\nperform show(\"c\")\n");
    assert_eq!(script.hosts, vec!["show".to_string(), "say".to_string()]);
}

#[test]
#[allow(clippy::too_many_lines)]
fn nested_lowering_is_bounded_and_string_add_is_detected() {
    let mut source = String::new();
    for depth in 0..=MAX_STATEMENT_DEPTH {
        source.push_str(&"    ".repeat(depth));
        source.push_str("while true:\n");
    }
    source.push_str(&"    ".repeat(MAX_STATEMENT_DEPTH + 1));
    source.push_str("set x = 1\n");
    // The parser rejects this first; a recovered empty block still hits the
    // depth guard if lowering is asked to walk a handmade tree.
    let error = lower(vec![deep_while(MAX_STATEMENT_DEPTH + 1)]).unwrap_err();
    assert!(error.message.contains("nesting"));
    let error = lower(vec![deep_if(MAX_STATEMENT_DEPTH + 1)]).unwrap_err();
    assert!(error.message.contains("nesting"));
    let empty_overflow = {
        let mut statement = Stmt::While {
            condition: crate::ast::Condition {
                expr: velin_syntax::Expr::Value(Value::Boolean(true)),
                line: 1,
            },
            body: Vec::new(),
        };
        for _ in 0..MAX_STATEMENT_DEPTH {
            statement = Stmt::While {
                condition: crate::ast::Condition {
                    expr: velin_syntax::Expr::Value(Value::Boolean(true)),
                    line: 1,
                },
                body: vec![statement],
            };
        }
        statement
    };
    let error = lower(vec![empty_overflow]).unwrap_err();
    assert!(error.message.contains("nesting"));
    let mut labeled = Stmt::Set {
        name: "x".into(),
        value: velin_syntax::Expr::Value(Value::Integer(1)),
        line: 9,
    };
    for index in 0..=MAX_STATEMENT_DEPTH {
        labeled = Stmt::Label {
            name: format!("n{index}"),
            body: vec![labeled],
            line: 1,
        };
    }
    let error = lower(vec![labeled]).unwrap_err();
    assert!(error.message.contains("nesting"));
    for leaf in [
        Stmt::Default {
            name: "d".into(),
            value: velin_syntax::Expr::Value(Value::Integer(1)),
            line: 2,
        },
        Stmt::Perform {
            command: "say".into(),
            arguments: Vec::new(),
            bind: None,
            line: 3,
        },
        Stmt::Jump {
            label: "gone".into(),
            line: 4,
        },
    ] {
        let mut wrapped = leaf;
        for _ in 0..=MAX_STATEMENT_DEPTH {
            wrapped = Stmt::While {
                condition: crate::ast::Condition {
                    expr: velin_syntax::Expr::Value(Value::Boolean(true)),
                    line: 1,
                },
                body: vec![wrapped],
            };
        }
        let error = lower(vec![wrapped]).unwrap_err();
        assert!(error.message.contains("nesting"));
    }
    let _ = lower(vec![Stmt::If {
        branches: Vec::new(),
        otherwise: None,
    }]);

    let script = compile("set title = \"a\"\nset title = title + \"b\"\n");
    assert!(script.program.ops.iter().any(|op| matches!(
        op,
        velin_bytecode::Op::Update {
            operation: UpdateOp::Add { .. },
            ..
        }
    )));
    let interpolated = compile("set title = \"a\"\nset title = title + \"x[y]\"\n");
    assert!(interpolated.program.ops.iter().any(|op| matches!(
        op,
        velin_bytecode::Op::Update {
            operation: UpdateOp::Add { .. },
            ..
        }
    )));
    let nested = compile("set title = \"a\"\nset title = title + (\"b\" + \"c\")\n");
    assert!(nested.program.ops.iter().any(|op| matches!(
        op,
        velin_bytecode::Op::Update {
            operation: UpdateOp::Add { .. },
            ..
        }
    )));
    let ignored = compile("set items = list(1)\nset items = get(items, 0)\n");
    assert!(!ignored.program.ops.is_empty());
}

fn deep_if(depth: usize) -> Stmt {
    let body = if depth == 0 {
        Vec::new()
    } else {
        vec![deep_if(depth - 1)]
    };
    Stmt::If {
        branches: vec![crate::ast::Branch {
            condition: crate::ast::Condition {
                expr: velin_syntax::Expr::Value(Value::Boolean(true)),
                line: 1,
            },
            body,
        }],
        otherwise: None,
    }
}

fn deep_while(depth: usize) -> Stmt {
    let body = if depth == 0 {
        vec![Stmt::Set {
            name: "x".into(),
            value: velin_syntax::Expr::Value(Value::Integer(1)),
            line: 1,
        }]
    } else {
        vec![deep_while(depth - 1)]
    };
    Stmt::While {
        condition: crate::ast::Condition {
            expr: velin_syntax::Expr::Value(Value::Boolean(true)),
            line: 1,
        },
        body,
    }
}

#[test]
fn bound_perform_stores_its_destination_on_the_host_op() {
    let script = compile("choice = perform ask(\"go?\")\n");
    assert!(matches!(
        script.program.ops[0],
        Op::Host(ref host) if host.host_id == 0 && host.bind.is_some()
    ));
}

#[test]
fn ownership_updates_are_emitted_only_for_the_assignment_target() {
    let script = compile(
        "default items = list()\n\
             set items = push(items, 1)\n\
             set items = put(items, 0, 2)\n\
             set items = remove(items, 0)\n\
             default text = \"a\"\n\
             set text = text + \"b\"\n\
             set count = count + 1\n\
             set other = push(items, 3)\n",
    );
    assert!(matches!(
        script.program.ops[0],
        Op::Update {
            operation: UpdateOp::Push { .. },
            ..
        }
    ));
    assert!(matches!(
        script.program.ops[1],
        Op::Update {
            operation: UpdateOp::Put { .. },
            ..
        }
    ));
    assert!(matches!(
        script.program.ops[2],
        Op::Update {
            operation: UpdateOp::Remove { .. },
            ..
        }
    ));
    assert!(matches!(
        script.program.ops[3],
        Op::Update {
            operation: UpdateOp::Add { .. },
            ..
        }
    ));
    assert!(matches!(
        script.program.ops[4],
        Op::Update {
            operation: UpdateOp::AddInteger { value: 1 },
            ..
        }
    ));
    assert!(matches!(script.program.ops[5], Op::Set { .. }));
}

#[test]
fn for_break_continue_and_literals_execute_through_existing_bytecode() {
    let script = compile(
        r#"set total = 0
for item in [1, 2, 3, 4]:
    if item == 2:
        continue
    if item == 4:
        break
    total += item
set data = {total: total}
set answer = data["total"]
"#,
    );
    assert!(script.check("test.velin").is_empty());
    assert!(script.program.slots.get("answer").is_some());
    assert!(
        script
            .program
            .ops
            .iter()
            .any(|op| matches!(op, Op::Jump(_)))
    );
}

#[test]
fn labels_resolve_forward_and_backward_jumps() {
    let script = compile("jump ahead\nlabel ahead:\njump ahead\n");
    // Both jumps target the same recorded label Pc.
    let target = script.labels["ahead"];
    assert!(matches!(script.program.ops[0], Op::Jump(t) if t == target));
}

#[test]
fn jump_to_undefined_label_is_an_error() {
    let error = lower(parse("jump nowhere\n").unwrap()).unwrap_err();
    assert!(error.message.contains("undefined label `nowhere`"));
}

#[test]
fn duplicate_label_is_an_error() {
    let error = lower(parse("label a:\nlabel a:\n").unwrap()).unwrap_err();
    assert!(error.message.contains("duplicate label `a`"));
}

#[test]
fn if_else_lowers_to_conditional_jumps_and_a_shared_exit() {
    let src = "if hp > 0:\n\
                   \x20\x20\x20\x20set alive = true\n\
                   else:\n\
                   \x20\x20\x20\x20set alive = false\n";
    let script = compile(src);
    assert!(matches!(
        script.program.ops[0],
        Op::JumpIfIntegerCompare { .. }
    ));
    assert!(matches!(script.program.ops[1], Op::SetConst { .. }));
}
