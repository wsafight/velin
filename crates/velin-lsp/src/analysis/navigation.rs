//! Label definition and reference analysis.

use super::SourceRange;
use velin::{Stmt, parse_program_recovering};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LabelOccurrence {
    pub(super) name: String,
    pub(super) range: SourceRange,
    pub(super) declaration: bool,
}

/// Finds the declaration targeted by the label occurrence under the cursor.
#[must_use]
pub fn label_definition(text: &str, line: usize, column: usize) -> Option<SourceRange> {
    let lines: Vec<&str> = text.lines().collect();
    let recovered = parse_program_recovering(text);
    let occurrences = label_occurrences(&recovered.statements, &lines);
    let selected = occurrence_at(&occurrences, line, column)?;
    occurrences
        .iter()
        .find(|item| item.declaration && item.name == selected.name)
        .map(|item| item.range)
}

/// Finds every use of the label occurrence under the cursor.
#[must_use]
pub fn label_references(
    text: &str,
    line: usize,
    column: usize,
    include_declaration: bool,
) -> Vec<SourceRange> {
    let lines: Vec<&str> = text.lines().collect();
    let recovered = parse_program_recovering(text);
    let occurrences = label_occurrences(&recovered.statements, &lines);
    let Some(selected) = occurrence_at(&occurrences, line, column) else {
        return Vec::new();
    };
    occurrences
        .iter()
        .filter(|item| item.name == selected.name && (include_declaration || !item.declaration))
        .map(|item| item.range)
        .collect()
}

pub(super) fn label_occurrences(statements: &[Stmt], lines: &[&str]) -> Vec<LabelOccurrence> {
    let mut occurrences = Vec::new();
    collect_label_occurrences(statements, lines, &mut occurrences);
    occurrences
}

fn collect_label_occurrences(
    statements: &[Stmt],
    lines: &[&str],
    occurrences: &mut Vec<LabelOccurrence>,
) {
    for statement in statements {
        match statement {
            Stmt::Label { name, body, line } => {
                if let Some(range) = statement_name_range(lines, *line, "label", name) {
                    occurrences.push(LabelOccurrence {
                        name: name.clone(),
                        range,
                        declaration: true,
                    });
                }
                collect_label_occurrences(body, lines, occurrences);
            }
            Stmt::Jump { label, line } => {
                if let Some(range) = statement_name_range(lines, *line, "jump", label) {
                    occurrences.push(LabelOccurrence {
                        name: label.clone(),
                        range,
                        declaration: false,
                    });
                }
            }
            Stmt::If {
                branches,
                otherwise,
            } => {
                for branch in branches {
                    collect_label_occurrences(&branch.body, lines, occurrences);
                }
                if let Some(body) = otherwise {
                    collect_label_occurrences(body, lines, occurrences);
                }
            }
            Stmt::While { body, .. } => collect_label_occurrences(body, lines, occurrences),
            Stmt::Default { .. } | Stmt::Set { .. } | Stmt::Perform { .. } => {}
        }
    }
}

fn statement_name_range(
    lines: &[&str],
    line: usize,
    keyword: &str,
    name: &str,
) -> Option<SourceRange> {
    let raw = lines.get(line.checked_sub(1)?)?;
    let trimmed = raw.trim_start_matches(' ');
    let indentation = raw[..raw.len() - trimmed.len()].chars().count();
    let rest = trimmed.strip_prefix(keyword)?;
    let spacing_bytes = rest.len() - rest.trim_start().len();
    let spacing = rest[..spacing_bytes].chars().count();
    let candidate = rest.trim_start();
    if !candidate.starts_with(name)
        || candidate
            .chars()
            .nth(name.chars().count())
            .is_some_and(is_identifier_char)
    {
        return None;
    }
    let start_column = indentation + keyword.chars().count() + spacing + 1;
    Some(SourceRange {
        line,
        start_column,
        end_column: start_column + name.chars().count(),
    })
}

fn occurrence_at(
    occurrences: &[LabelOccurrence],
    line: usize,
    column: usize,
) -> Option<&LabelOccurrence> {
    occurrences.iter().find(|item| {
        item.range.line == line
            && column >= item.range.start_column
            && column <= item.range.end_column
    })
}

const fn is_identifier_char(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_'
}
