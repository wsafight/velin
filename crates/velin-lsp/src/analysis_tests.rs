use super::*;

const SCRIPT: &str = "\
default hp = 30
label start:
    choice = perform ask(\"go?\")
    if choice == 1:
        set hp = hp + 10
    while hp > 0:
        set hp = hp - 1
label done:
    perform say(\"bye\")
";

#[test]
fn a_clean_script_has_no_error_diagnostics() {
    let diagnostics = diagnostics("t.velin", SCRIPT);
    assert!(diagnostics.iter().all(|d| !d.is_error()), "{diagnostics:?}");
}

#[test]
fn parse_errors_yield_multiple_diagnostics_with_the_file_name() {
    let diagnostics = diagnostics(
        "bad.velin",
        "set first\nperform missing(\nlabel still_visible:\n",
    );
    assert_eq!(diagnostics.len(), 2);
    assert!(diagnostics.iter().all(|item| item.file == "bad.velin"));
    assert!(diagnostics.iter().all(Diagnostic::is_error));
}

#[test]
fn completions_include_keywords_builtins_and_declared_names() {
    let items = completions(SCRIPT);
    let has = |label: &str, kind: CompletionKind| {
        items.iter().any(|c| c.label == label && c.kind == kind)
    };
    assert!(has("while", CompletionKind::Keyword));
    assert!(has("len", CompletionKind::Function));
    assert!(has("random", CompletionKind::Function));
    assert!(has("chance", CompletionKind::Function));
    assert!(has("hp", CompletionKind::Variable));
    assert!(has("choice", CompletionKind::Variable));
    assert!(has("start", CompletionKind::Label));
}

#[test]
fn completions_retain_names_around_a_parse_error() {
    let items = completions("set before = 1\nset broken\nlabel after:\n");
    assert!(items.iter().any(|c| c.label == "if"));
    assert!(
        items
            .iter()
            .any(|c| c.label == "before" && c.kind == CompletionKind::Variable)
    );
    assert!(
        items
            .iter()
            .any(|c| c.label == "after" && c.kind == CompletionKind::Label)
    );
}

#[test]
fn document_symbols_list_labels_with_lines() {
    let symbols = document_symbols(SCRIPT);
    let names: Vec<_> = symbols.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["start", "done"]);
    assert_eq!(symbols[0].line, 2);
}

#[test]
fn names_and_symbols_walk_else_while_and_jump() {
    let source = "\
if true:
    label inner:
        set x = 1
        jump done
else:
    label other:
        set y = 1
while false:
    label looped:
        set z = 1
label done:
    perform say(\"ok\")
";
    let items = completions(source);
    assert!(
        items
            .iter()
            .any(|c| c.label == "inner" && c.kind == CompletionKind::Label)
    );
    assert!(
        items
            .iter()
            .any(|c| c.label == "y" && c.kind == CompletionKind::Variable)
    );
    let names: Vec<_> = document_symbols(source)
        .into_iter()
        .map(|symbol| symbol.name)
        .collect();
    assert!(names.contains(&"inner".to_owned()));
    assert!(names.contains(&"other".to_owned()));
    assert!(names.contains(&"looped".to_owned()));
}

#[test]
fn symbols_survive_a_malformed_block_before_a_valid_label() {
    let source = "while true:\n    set broken\nlabel done:\n";
    assert_eq!(
        document_symbols(source),
        vec![LabelSymbol {
            name: "done".to_owned(),
            line: 3,
        }]
    );
}

#[test]
fn hover_describes_language_names_in_recovered_source() {
    let source = "set score = 1\nset broken\nperform notify(len(score))\n";

    let keyword = hover(source, 1, 2).unwrap();
    assert!(keyword.contents.contains("keyword"));
    assert_eq!(keyword.range.start_column, 1);

    let builtin = hover(source, 3, 17).unwrap();
    assert!(builtin.contents.contains("len(collection)"));

    let variable = hover(source, 3, 21).unwrap();
    assert!(variable.contents.contains("variable"));

    let host = hover(source, 3, 10).unwrap();
    assert!(host.contents.contains("host command"));
    assert!(hover(source, 2, 5).is_none());
}

#[test]
fn label_navigation_survives_an_error_between_references() {
    let source = "label start:\n\
                      \x20\x20\x20\x20jump start\n\
                      \x20\x20\x20\x20set broken\n\
                      \x20\x20\x20\x20jump start\n\
                      label done:\n";

    assert_eq!(
        label_definition(source, 4, 10),
        Some(SourceRange {
            line: 1,
            start_column: 7,
            end_column: 12,
        })
    );
    assert_eq!(label_references(source, 1, 7, true).len(), 3);
    let references = label_references(source, 2, 10, false);
    assert_eq!(references.len(), 2);
    assert!(references.iter().all(|range| range.line > 1));
    assert!(label_definition(source, 5, 7).is_some());
}

#[test]
fn host_schema_drives_diagnostics_completion_hover_and_signature_help() {
    let schema = HostSchema::new().declare(
        HostCommand::new(
            "ask",
            velin::HostSignature::exact(
                vec![velin::Type::String, velin::Type::Integer],
                Some(velin::Type::Boolean),
            ),
        )
        .description("Ask a question with a retry limit."),
    );
    let source = "answer = perform ask(\"Ready?\", 3)\n";

    assert!(diagnostics_with_host_schema("t.velin", source, &schema).is_empty());
    assert!(
        diagnostics_with_host_schema("t.velin", "perform missing()\n", &schema)
            .iter()
            .any(|diagnostic| diagnostic.message.contains("not declared"))
    );
    let completion = completions_with_host_schema(source, &schema)
        .into_iter()
        .find(|completion| completion.label == "ask")
        .unwrap();
    assert_eq!(completion.detail, "ask(string, integer) -> boolean");
    assert_eq!(
        completion.documentation.as_deref(),
        Some("Ask a question with a retry limit.")
    );
    let hover = hover_with_host_schema(source, 1, 20, &schema).unwrap();
    assert!(hover.contents.contains("ask(string, integer) -> boolean"));
    assert!(hover.contents.contains("retry limit"));

    let help = signature_help(source, 1, 33, &schema).unwrap();
    assert_eq!(help.label, "ask(string, integer) -> boolean");
    assert_eq!(help.parameters, vec!["string", "integer"]);
    assert_eq!(help.active_parameter, 1);

    let quoted_keyword = "answer = perform ask(\"perform fake(\", 3)\n";
    let help = signature_help(quoted_keyword, 1, 40, &schema).unwrap();
    assert_eq!(help.active_parameter, 1);
}
