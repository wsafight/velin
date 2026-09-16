use super::*;
use crate::lines::Line;

fn line(content: &str) -> Line<'_> {
    Line {
        number: 1,
        indent: 0,
        content,
        column: 1,
    }
}

#[test]
fn split_assignment_skips_comparisons_and_reports_missing_assignment() {
    let assigned = line("ok = hp != 3");
    let (name, value, column, operator) = split_assignment(&assigned).unwrap();
    assert_eq!(name, "ok");
    assert_eq!(value, " hp != 3");
    assert!(column > 1);
    assert_eq!(operator, AssignmentOperator::Set);
    assert!(split_assignment(&line("hp != 3")).is_err());
    assert!(
        split_assignment(&line("flag"))
            .unwrap_err()
            .message
            .contains("name = value")
    );
    let le = line("ok = hp <= 3");
    let (name, value, _, _) = split_assignment(&le).unwrap();
    assert_eq!(name, "ok");
    assert!(value.contains("<="));
    let ge = line("ok = hp >= 3");
    let (name, value, _, _) = split_assignment(&ge).unwrap();
    assert_eq!(name, "ok");
    assert!(value.contains(">="));
}

#[test]
fn headers_require_colons_and_identifiers() {
    assert!(expect_header_name(&line("label start"), "start").is_err());
    assert_eq!(
        expect_header_name(&line("label start:"), "start:").unwrap(),
        "start"
    );
    assert!(expect_bare_header(&line("else x:"), "x:", "else").is_err());
    assert!(expect_bare_header(&line("else:"), ":", "else").is_ok());
    assert!(expect_identifier(&line("set 1x = 1"), "1x").is_err());
    assert_eq!(split_keyword("else:"), ("else", ":"));
    assert_eq!(split_keyword("= 1"), ("", "= 1"));
}
