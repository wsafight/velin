//! Cursor-sensitive identifier and host-call parsing.

use super::SourceRange;

pub(super) fn word_at(text: &str, line: usize, column: usize) -> Option<(&str, SourceRange)> {
    let source_line = text.lines().nth(line.checked_sub(1)?)?;
    let mut cursor = column.saturating_sub(1);
    let char_count = source_line.chars().count();
    if cursor > char_count {
        return None;
    }
    if cursor == char_count
        || !source_line
            .chars()
            .nth(cursor)
            .is_some_and(is_identifier_char)
    {
        cursor = cursor.checked_sub(1)?;
    }
    if !source_line
        .chars()
        .nth(cursor)
        .is_some_and(is_identifier_char)
    {
        return None;
    }

    let chars: Vec<(usize, char)> = source_line.char_indices().collect();
    let mut start = cursor;
    while start > 0 && is_identifier_char(chars[start - 1].1) {
        start -= 1;
    }
    let mut end = cursor + 1;
    while end < chars.len() && is_identifier_char(chars[end].1) {
        end += 1;
    }
    let start_byte = chars[start].0;
    let end_byte = chars.get(end).map_or(source_line.len(), |item| item.0);
    Some((
        &source_line[start_byte..end_byte],
        SourceRange {
            line,
            start_column: start + 1,
            end_column: end + 1,
        },
    ))
}

pub(super) fn host_call_at(text: &str, line: usize, column: usize) -> Option<(String, usize)> {
    let source_line = text.lines().nth(line.checked_sub(1)?)?;
    let prefix: String = source_line.chars().take(column.saturating_sub(1)).collect();
    let perform = keyword_positions(&prefix, "perform").last().copied()?;
    let after_keyword = &prefix[perform + "perform".len()..];
    let trimmed = after_keyword.trim_start();
    let name_end = trimmed
        .char_indices()
        .find_map(|(index, character)| (!is_identifier_char(character)).then_some(index))
        .unwrap_or(trimmed.len());
    let name = trimmed.get(..name_end)?;
    if name.is_empty() {
        return None;
    }
    let arguments = trimmed.get(name_end..)?.trim_start().strip_prefix('(')?;
    active_host_parameter(arguments).map(|active| (name.to_owned(), active))
}

fn keyword_positions(text: &str, keyword: &str) -> Vec<usize> {
    let mut positions = Vec::new();
    let mut quoted = false;
    let mut escaped = false;
    for (index, character) in text.char_indices() {
        if quoted {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                quoted = false;
            }
            continue;
        }
        if character == '"' {
            quoted = true;
            continue;
        }
        let Some(rest) = text.get(index..) else {
            continue;
        };
        if !rest.starts_with(keyword) {
            continue;
        }
        let before = text[..index].chars().next_back();
        let after = rest[keyword.len()..].chars().next();
        if !before.is_some_and(is_identifier_char) && !after.is_some_and(is_identifier_char) {
            positions.push(index);
        }
    }
    positions
}

fn active_host_parameter(arguments: &str) -> Option<usize> {
    let mut active = 0usize;
    let mut depth = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    for character in arguments.chars() {
        if quoted {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                quoted = false;
            }
            continue;
        }
        match character {
            '"' => quoted = true,
            '(' => depth += 1,
            ')' if depth == 0 => return None,
            ')' => depth -= 1,
            ',' if depth == 0 => active += 1,
            _ => {}
        }
    }
    Some(active)
}

const fn is_identifier_char(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_'
}
