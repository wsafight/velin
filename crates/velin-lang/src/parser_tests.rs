use super::*;
use velin_syntax::{BinaryOp, Value};

#[test]
fn parses_label_default_and_set() {
    let stmts = parse("label start:\ndefault hp = 30\nset hp = hp + 10\n").unwrap();
    assert_eq!(stmts.len(), 3);
    assert!(matches!(stmts[0], Stmt::Label { .. }));
    assert!(matches!(stmts[1], Stmt::Default { .. }));
    match &stmts[2] {
        Stmt::Set { name, value, .. } => {
            assert_eq!(name, "hp");
            assert!(matches!(
                value.unspanned(),
                Expr::Binary {
                    op: BinaryOp::Add,
                    ..
                }
            ));
        }
        other => panic!("expected Set, got {other:?}"),
    }
}

#[test]
fn parses_perform_bound_and_unbound() {
    let stmts = parse("perform say(\"hi\")\nchoice = perform ask(\"go?\")\n").unwrap();
    match &stmts[0] {
        Stmt::Perform {
            command,
            arguments,
            bind,
            ..
        } => {
            assert_eq!(command, "say");
            assert_eq!(arguments.len(), 1);
            assert!(bind.is_none());
        }
        other => panic!("expected Perform, got {other:?}"),
    }
    match &stmts[1] {
        Stmt::Perform { command, bind, .. } => {
            assert_eq!(command, "ask");
            assert_eq!(bind.as_deref(), Some("choice"));
        }
        other => panic!("expected bound Perform, got {other:?}"),
    }
}

#[test]
fn parses_if_elif_else_with_bodies() {
    let src = "if hp > 20:\n\
                   \x20\x20\x20\x20set ok = true\n\
                   elif hp > 0:\n\
                   \x20\x20\x20\x20set ok = false\n\
                   else:\n\
                   \x20\x20\x20\x20jump dead\n";
    let stmts = parse(src).unwrap();
    assert_eq!(stmts.len(), 1);
    match &stmts[0] {
        Stmt::If {
            branches,
            otherwise,
        } => {
            assert_eq!(branches.len(), 2);
            assert_eq!(branches[0].body.len(), 1);
            assert!(otherwise.is_some());
        }
        other => panic!("expected If, got {other:?}"),
    }
}

#[test]
fn parses_while_and_nested_block() {
    let src = "while count < 3:\n\
                   \x20\x20\x20\x20set count = count + 1\n\
                   \x20\x20\x20\x20perform tick()\n";
    let stmts = parse(src).unwrap();
    match &stmts[0] {
        Stmt::While { body, .. } => assert_eq!(body.len(), 2),
        other => panic!("expected While, got {other:?}"),
    }
}

#[test]
fn expression_errors_report_source_character_columns() {
    let ascii = parse("set x = \"a\" +\n").unwrap_err();
    let unicode = parse("set x = \"\u{4f60}\" +\n").unwrap_err();
    let unicode_spacing = parse("set\u{a0}x\u{a0}= \"a\" +\n").unwrap_err();
    let host_argument = parse("perform say(\"a\", 1 + )\n").unwrap_err();

    assert_eq!(ascii.column, 14);
    assert_eq!(unicode.column, 14);
    assert_eq!(unicode_spacing.column, 14);
    assert_eq!(host_argument.column, 22);
}

#[test]
fn bare_assignment_without_set_keyword_is_sugar() {
    let stmts = parse("score = 1\n").unwrap();
    match &stmts[0] {
        Stmt::Set { name, value, .. } => {
            assert_eq!(name, "score");
            assert_eq!(value.unspanned(), &Expr::Value(Value::Integer(1)));
        }
        other => panic!("expected Set, got {other:?}"),
    }
}

#[test]
fn missing_block_and_bad_indent_are_errors() {
    assert!(parse("if hp > 0:\nset x = 1\n").is_err()); // body not indented
    assert!(parse("\x20\x20\x20\x20set x = 1\n").is_err()); // leading indent with no header
    assert!(parse("label 1bad:\n").is_err()); // invalid identifier
}

#[test]
fn statement_nesting_is_bounded() {
    let mut source = String::new();
    for depth in 0..=MAX_STATEMENT_DEPTH {
        source.push_str(&"    ".repeat(depth));
        source.push_str("while true:\n");
    }
    source.push_str(&"    ".repeat(MAX_STATEMENT_DEPTH + 1));
    source.push_str("set x = 1\n");
    let error = parse(&source).unwrap_err();
    assert!(error.message.contains("nesting"));
}

#[test]
fn equals_in_condition_is_not_an_assignment() {
    // `set` value contains `==`; the top-level `=` splitter must not trip.
    let stmts = parse("set ok = hp == 3\n").unwrap();
    match &stmts[0] {
        Stmt::Set { value, .. } => assert!(matches!(
            value.unspanned(),
            Expr::Binary {
                op: BinaryOp::Equal,
                ..
            }
        )),
        other => panic!("expected Set, got {other:?}"),
    }
}

#[test]
fn perform_jump_label_body_and_malformed_headers() {
    let stmts = parse("label start:\n    perform tick()\n    jump start\n").unwrap();
    assert_eq!(stmts.len(), 1);
    match &stmts[0] {
        Stmt::Label { name, body, .. } => {
            assert_eq!(name, "start");
            assert_eq!(body.len(), 2);
        }
        other => panic!("expected Label, got {other:?}"),
    }

    assert!(parse("perform say\n").unwrap_err().message.contains("`(`"));
    assert!(
        parse("perform say(\"hi\"\n")
            .unwrap_err()
            .message
            .contains("`)`")
    );
    assert!(parse("set x\n").is_err());
    assert!(parse("label start\n").unwrap_err().message.contains("`:`"));
    assert!(
        parse("if true:\n    set x = 1\nelse x:\n    set y = 1\n")
            .unwrap_err()
            .message
            .contains("takes no condition")
    );
    assert!(parse("jump\n").is_err());
}

#[test]
fn recovering_parse_keeps_valid_siblings_and_collects_errors() {
    let recovered = parse_recovering(
        "default before = 1\n\
             set broken\n\
             perform missing(\n\
             label after:\n",
    );

    assert_eq!(recovered.errors.len(), 2);
    assert_eq!(recovered.errors[0].line, 2);
    assert_eq!(recovered.errors[1].line, 3);
    assert_eq!(recovered.statements.len(), 2);
    assert!(matches!(
        &recovered.statements[0],
        Stmt::Default { name, .. } if name == "before"
    ));
    assert!(matches!(
        &recovered.statements[1],
        Stmt::Label { name, .. } if name == "after"
    ));
}

#[test]
fn recovering_parse_retains_valid_statements_inside_a_damaged_block() {
    let recovered = parse_recovering(
        "label start:\n\
             \x20\x20\x20\x20set before = 1\n\
             \x20\x20\x20\x20set broken\n\
             \x20\x20\x20\x20set after = 2\n\
             label done:\n",
    );

    assert_eq!(recovered.errors.len(), 1);
    assert_eq!(recovered.errors[0].line, 3);
    assert_eq!(recovered.statements.len(), 2);
    let Stmt::Label { body, .. } = &recovered.statements[0] else {
        panic!("expected recovered label");
    };
    assert_eq!(body.len(), 2);
    assert!(matches!(&body[0], Stmt::Set { name, .. } if name == "before"));
    assert!(matches!(&body[1], Stmt::Set { name, .. } if name == "after"));
}

#[test]
fn recovering_parse_resynchronizes_after_a_missing_block() {
    let recovered = parse_recovering("while true:\nlabel done:\n");

    assert_eq!(recovered.errors.len(), 1);
    assert_eq!(recovered.errors[0].line, 1);
    assert!(matches!(
        recovered.statements.as_slice(),
        [Stmt::Label { name, .. }] if name == "done"
    ));
}
