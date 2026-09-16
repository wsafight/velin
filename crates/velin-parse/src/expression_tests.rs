use super::*;

fn parse(input: &str) -> Result<Expr, Diagnostic> {
    parse_expression(input, "test.rl", 1, 1)
}

#[test]
fn parses_expression_lists_with_source_spans() {
    let source = SharedString::from("test.rl");
    let expressions = parse_expression_list_with_source("1, value + 2", &source, 3, 5).unwrap();

    assert_eq!(expressions.len(), 2);
    assert_eq!(expressions[0].span().unwrap().column, 5);
    assert_eq!(expressions[1].span().unwrap().column, 14);
    assert_eq!(expressions[1].span().unwrap().line, 3);
    assert_eq!(expressions[1].span().unwrap().source, "test.rl");
}

#[test]
fn expression_lists_enforce_the_host_argument_limit() {
    let source = SharedString::from("test.rl");
    let arguments = std::iter::repeat_n("1", 129).collect::<Vec<_>>().join(",");
    let error = parse_expression_list_with_source(&arguments, &source, 1, 1).unwrap_err();

    assert!(error.message.contains("invalid argument count"));
}

#[test]
fn parses_arithmetic_with_precedence() {
    let expr = parse("1 + 2 * 3").unwrap();
    // 1 + (2 * 3)
    let Expr::Binary { op, right, .. } = expr.unspanned() else {
        panic!("expected binary");
    };
    assert_eq!(*op, BinaryOp::Add);
    assert!(matches!(
        right.unspanned(),
        Expr::Binary {
            op: BinaryOp::Multiply,
            ..
        }
    ));
}

#[test]
fn parses_collection_literals_and_postfix_indexes() {
    let expression = parse_expression(
        "[{name: \"Ada\"}, {\"name\": \"Lin\"}][1][\"name\"]",
        "x",
        1,
        1,
    )
    .unwrap();
    let Expr::Invoke {
        function: Builtin::Get,
        arguments,
    } = expression.unspanned()
    else {
        panic!("expected outer index")
    };
    assert_eq!(arguments.len(), 2);
    assert!(matches!(
        arguments[0].unspanned(),
        Expr::Invoke {
            function: Builtin::Get,
            ..
        }
    ));
}

#[test]
fn parses_builtin_calls_and_rejects_unknown() {
    assert!(parse("len(bag)").is_ok());
    assert!(parse("push(bag, \"x\")").is_ok());
    assert!(parse("random(1, 6)").is_ok());
    assert!(parse("chance(50)").is_ok());
    assert!(parse("random(1)").is_err());
    assert!(parse("chance(1, 2)").is_err());
    assert!(parse("exec(\"cmd\")").is_err());
}

#[test]
fn parses_boolean_and_comparison_operators() {
    assert!(parse("score >= 1 and not done").is_ok());
    assert!(parse("a == b or c != d").is_ok());
}

#[test]
fn rejects_unbalanced_and_trailing_tokens() {
    assert!(parse("(1 + 2").is_err());
    assert!(parse("1 2").is_err());
    assert!(parse("=").is_err());
}

#[test]
fn plain_string_without_holes_stays_a_value() {
    // Zero-regression guarantee: no `[expr]` means a plain string value.
    assert_eq!(
        parse("\"just text\"").unwrap().into_unspanned(),
        Expr::Value(Value::String("just text".into()))
    );
    // `[[` / `]]` escape to literal brackets and keep it a plain value.
    assert_eq!(
        parse("\"a [[b]] c\"").unwrap().into_unspanned(),
        Expr::Value(Value::String("a [b] c".into()))
    );
}

#[test]
fn interpolation_splits_literals_and_holes() {
    let expression = parse("\"hp is [hp] now\"").unwrap();
    let Expr::Interpolate { parts } = expression.unspanned() else {
        panic!("expected interpolation");
    };
    assert_eq!(parts.len(), 3);
    assert_eq!(parts[0], StrPart::Literal("hp is ".into()));
    assert_eq!(parts[2], StrPart::Literal(" now".into()));
    match &parts[1] {
        StrPart::Hole(expr) => {
            assert_eq!(expr.unspanned(), &Expr::Variable("hp".into()));
        }
        StrPart::Literal(text) => panic!("expected hole, got literal {text:?}"),
    }
}

#[test]
fn interpolation_holes_carry_full_expressions_and_report_errors() {
    // A hole is a full sub-expression, so operators and builtins work.
    let expression = parse("\"[hp + 1]\"").unwrap();
    let Expr::Interpolate { parts } = expression.unspanned() else {
        panic!("expected interpolation");
    };
    assert!(matches!(
        &parts[0],
        StrPart::Hole(expr) if matches!(expr.unspanned(), Expr::Binary { op: BinaryOp::Add, .. })
    ));
    // An unterminated hole and a broken sub-expression are both errors.
    assert!(parse("\"open [hp\"").is_err());
    assert!(parse("\"[1 +]\"").is_err());
}

#[test]
fn brackets_inside_hole_strings_do_not_close_the_hole() {
    let expression = parse(r#""[contains("a]", "x")]""#).unwrap();
    let Expr::Interpolate { parts } = expression.unspanned() else {
        panic!("expected interpolation");
    };
    assert_eq!(parts.len(), 1);

    let expression = parse(r#""[contains("a\"b]", "x")]""#).unwrap();
    let Expr::Interpolate { parts } = expression.unspanned() else {
        panic!("expected interpolation");
    };
    assert_eq!(parts.len(), 1);
}

#[test]
fn diagnostics_use_unicode_character_columns() {
    let ascii = parse_expression("\"a\" +", "test.rl", 1, 9).unwrap_err();
    let unicode = parse_expression("\"\u{4f60}\" +", "test.rl", 1, 9).unwrap_err();

    assert_eq!(ascii.column, 14);
    assert_eq!(unicode.column, 14);
}

#[test]
fn lexer_and_parser_reject_budget_and_token_errors() {
    assert!(
        parse(&"x".repeat(65_537))
            .unwrap_err()
            .message
            .contains("64 KiB")
    );
    let too_many_tokens = (0..300).map(|_| "1+").collect::<String>() + "1";
    assert!(
        parse(&too_many_tokens)
            .unwrap_err()
            .message
            .contains("512 tokens")
    );
    let nested = format!("{}1{}", "(".repeat(33), ")".repeat(33));
    assert!(parse(&nested).unwrap_err().message.contains("nesting"));
    assert!(
        parse("@")
            .unwrap_err()
            .message
            .contains("unexpected character")
    );
    assert!(parse("1 = 2").unwrap_err().message.contains("`==`"));
    assert!(parse("1 ! 2").unwrap_err().message.contains("`!=`"));
    assert!(
        parse("\"abc")
            .unwrap_err()
            .message
            .contains("unterminated string")
    );
    assert!(
        parse("\"\\q\"")
            .unwrap_err()
            .message
            .contains("unsupported escape")
    );
    assert!(
        parse("9223372036854775808")
            .unwrap_err()
            .message
            .contains("out of range")
    );
    assert!(
        parse("len(1 2)")
            .unwrap_err()
            .message
            .contains("expected `,` or `)`")
    );
}

#[test]
fn nested_interpolation_uses_one_shared_budget() {
    let mut expression = "1".to_owned();
    for _ in 0..=MAX_INTERPOLATION_DEPTH {
        expression = format!(r#""[{expression}]""#);
    }
    let error = parse(&expression).unwrap_err();
    assert!(error.message.contains("interpolation nesting"));
}

#[test]
fn string_escapes_and_nested_holes_parse() {
    assert_eq!(
        parse(r#""a\n\r\t\"\\b""#).unwrap().into_unspanned(),
        Expr::Value(Value::String("a\n\r\t\"\\b".into()))
    );
    let expression = parse(r#""pre[hp]post""#).unwrap();
    let Expr::Interpolate { parts } = expression.unspanned() else {
        panic!("expected interpolation");
    };
    assert_eq!(parts[0], StrPart::Literal("pre".into()));
    assert_eq!(parts[2], StrPart::Literal("post".into()));
    let expression = parse(r#""[list(1, list(2))]""#).unwrap();
    let Expr::Interpolate { parts } = expression.unspanned() else {
        panic!("expected nested hole");
    };
    assert_eq!(parts.len(), 1);
    assert!(parse("\"[hp\"").is_err());
}
