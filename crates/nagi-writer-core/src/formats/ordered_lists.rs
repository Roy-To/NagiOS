use crate::Error;

#[derive(Debug)]
pub(super) struct Item<'a> {
    pub line: usize,
    pub text: &'a str,
}

#[derive(Debug)]
pub(super) struct Run<'a> {
    pub end: usize,
    pub items: Vec<Item<'a>>,
    pub preserve_literal: bool,
    pub separated_by_blank_line: bool,
}

#[derive(Clone, Copy)]
struct Marker<'a> {
    digits: &'a str,
    delimiter: u8,
    text: &'a str,
    canonical_spacing: bool,
}

fn marker(line: &str) -> Option<Marker<'_>> {
    let bytes = line.as_bytes();
    let digit_end = bytes
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if !(1..=9).contains(&digit_end) {
        return None;
    }

    let delimiter = *bytes.get(digit_end)?;
    if !matches!(delimiter, b'.' | b')') {
        return None;
    }

    let tail = &line[digit_end + 1..];
    let spacing = tail
        .bytes()
        .take_while(|byte| matches!(byte, b' ' | b'\t'))
        .count();
    if !tail.is_empty() && spacing == 0 {
        return None;
    }

    let text = &tail[spacing..];
    let canonical_spacing =
        delimiter == b'.' && (tail.is_empty() || (spacing == 1 && tail.as_bytes()[0] == b' '));

    Some(Marker {
        digits: &line[..digit_end],
        delimiter,
        text,
        canonical_spacing,
    })
}

fn blank(line: &str) -> bool {
    line.bytes().all(|byte| matches!(byte, b' ' | b'\t'))
}

fn starts_another_block(line: &str) -> bool {
    super::heading(line).is_some()
        || super::fence(line).is_some()
        || line == ">"
        || line.starts_with("> ")
        || line == "-"
        || line == "+"
        || line == "*"
        || line.starts_with("- ")
        || line.starts_with("+ ")
        || line.starts_with("* ")
}

pub(super) fn starts_marker(line: &str) -> bool {
    marker(line).is_some()
}

/// Classify one source run without assigning Markdown meaning to fields the
/// flat Writer list model cannot retain. A returned end index is exclusive.
pub(super) fn scan<'a>(
    lines: &[&'a str],
    start: usize,
    max_items: usize,
) -> Result<Option<Run<'a>>, Error> {
    let Some(first) = lines.get(start).and_then(|line| marker(line)) else {
        return Ok(None);
    };

    let mut items = Vec::new();
    let mut item_count = 0usize;
    let mut preserve_literal = first.digits != "1" || !first.canonical_spacing;
    let mut too_many_items = false;
    let mut separated_by_blank_line = false;
    let mut end = start;
    let mut cursor = start;

    while cursor < lines.len() {
        if blank(lines[cursor]) {
            let mut next = cursor + 1;
            while next < lines.len() && blank(lines[next]) {
                next += 1;
            }
            if next == lines.len() {
                break;
            }

            if marker(lines[next]).is_some() {
                separated_by_blank_line = true;
                cursor = next;
                continue;
            }

            if lines[next].starts_with(' ') || lines[next].starts_with('\t') {
                // Indented continuation or a child list cannot be represented
                // by this flat model, so keep it with the parent source run.
                preserve_literal = true;
                end = next + 1;
                cursor = next + 1;
                continue;
            }

            break;
        }

        if let Some(found) = marker(lines[cursor]) {
            let expected = item_count.saturating_add(1).to_string();
            if found.digits != expected
                || found.delimiter != b'.'
                || found.delimiter != first.delimiter
                || !found.canonical_spacing
            {
                preserve_literal = true;
                items.clear();
            }

            item_count = item_count.checked_add(1).ok_or(Error::LimitExceeded)?;
            if !preserve_literal {
                if items.len() >= max_items {
                    too_many_items = true;
                } else {
                    items.push(Item {
                        line: cursor + 1,
                        text: found.text,
                    });
                }
            }

            end = cursor + 1;
            cursor += 1;
            continue;
        }

        if starts_another_block(lines[cursor]) {
            break;
        }

        // An unindented lazy continuation is still part of a Markdown list
        // item. The model has no continuation-block slot, so retain the source.
        preserve_literal = true;
        items.clear();
        end = cursor + 1;
        cursor += 1;
    }

    if preserve_literal {
        items.clear();
    } else if too_many_items {
        return Err(Error::LimitExceeded);
    }

    Ok(Some(Run {
        end,
        items,
        preserve_literal,
        separated_by_blank_line,
    }))
}
