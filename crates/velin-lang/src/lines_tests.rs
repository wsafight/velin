use super::*;

#[test]
fn drops_blank_and_comment_lines_and_measures_indent() {
    let source = "label start:\n\
                  \n\
                  # a comment\n\
                  \x20\x20\x20\x20set hp = 1\n";
    let lines = read(source, "t.velin").unwrap();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].indent, 0);
    assert_eq!(lines[0].content, "label start:");
    assert_eq!(lines[1].indent, 1);
    assert_eq!(lines[1].content, "set hp = 1");
    assert_eq!(lines[1].column, 5);
    assert_eq!(lines[1].number, 4);
}

#[test]
fn hash_inside_string_is_not_a_comment() {
    let lines = read("perform say(\"score #1\") # real comment\n", "t.velin").unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].content, "perform say(\"score #1\")");
}

#[test]
fn escaped_quote_does_not_end_string() {
    let lines = read("set s = \"a\\\"b # c\"\n", "t.velin").unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].content, "set s = \"a\\\"b # c\"");
}

#[test]
fn hash_inside_an_interpolation_hole_string_is_not_a_comment() {
    let source = r##"perform say("[contains("a#]", "#")]") # real comment"##;
    let lines = read(source, "t.velin").unwrap();
    assert_eq!(
        lines[0].content,
        r##"perform say("[contains("a#]", "#")]")"##
    );
}

#[test]
fn tabs_and_odd_indentation_are_rejected() {
    assert!(read("\tset x = 1\n", "t.velin").is_err());
    assert!(read("  set x = 1\n", "t.velin").is_err()); // 2 spaces, not a multiple of 4
}

#[test]
fn source_size_and_line_count_are_bounded() {
    let oversized = "x".repeat(MAX_SOURCE_BYTES + 1);
    assert!(
        read(&oversized, "t.velin")
            .unwrap_err()
            .message
            .contains("MiB")
    );

    let too_many_lines = "\n".repeat(MAX_SOURCE_LINES + 1);
    assert!(
        read(&too_many_lines, "t.velin")
            .unwrap_err()
            .message
            .contains("lines")
    );
}

#[test]
fn comments_survive_nested_holes_and_escapes() {
    let escaped_hole = r#"perform say("[contains("a\"b#]", "x")]") # keep"#;
    let lines = read(escaped_hole, "t.velin").unwrap();
    assert!(lines[0].content.contains("contains"));
    assert!(!lines[0].content.contains("keep"));

    let nested = r#"perform say("[list([1])] # still string") # real"#;
    let lines = read(nested, "t.velin").unwrap();
    assert!(lines[0].content.contains("list([1])"));
    assert!(!lines[0].content.contains("real"));

    let escaped_bracket = r#"perform say("foo [[ # not") # yes"#;
    let lines = read(escaped_bracket, "t.velin").unwrap();
    assert!(lines[0].content.contains("[["));
    assert!(!lines[0].content.contains("yes"));
}
