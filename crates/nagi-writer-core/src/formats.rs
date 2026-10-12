//! Conservative Markdown subset. Unsupported syntax is retained literally and
//! reported, not interpreted as HTML, fetched as media, or silently discarded.
//!
//! `BlockKind::List` stores no per-item ordinal, and export always numbers an
//! ordered list `1.`, `2.`, ... An ordered run is therefore imported as a list
//! only when every marker is exactly that canonical sequence. Any other
//! numbering (non-1 start, gaps, leading zeros, a run resumed after another
//! block, numbers beyond any integer type) keeps the whole run literally as a
//! paragraph with one `UnsupportedSyntax` warning per line, so the original
//! numbers are never rewritten. Markers are compared as decimal text, never
//! parsed, so arbitrarily long numbers cannot overflow.
use crate::*;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Format {
    Markdown,
    PlainText,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WarningKind {
    UnsupportedSyntax,
    UnclosedFence,
    NativeMetadata,
    UnsupportedBlock,
    StructuralLoss,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnsupportedWarning {
    pub kind: WarningKind,
    pub line: Option<usize>,
    pub object: Option<ObjectId>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportResult {
    pub document: Document,
    pub warnings: Vec<UnsupportedWarning>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExportResult {
    pub text: String,
    pub warnings: Vec<UnsupportedWarning>,
}
fn warn(kind: WarningKind, line: Option<usize>, object: Option<ObjectId>) -> UnsupportedWarning {
    UnsupportedWarning { kind, line, object }
}
fn fence(line: &str) -> Option<(char, usize, &str)> {
    let c = line.chars().next()?;
    if c != '`' && c != '~' {
        return None;
    }
    let count = line.chars().take_while(|ch| *ch == c).count();
    (count >= 3).then(|| (c, count, &line[count..]))
}
fn list_line(line: &str) -> Option<(bool, &str)> {
    if let Some(t) = line
        .strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .or_else(|| line.strip_prefix("+ "))
    {
        return Some((false, t));
    }
    let n = line.bytes().take_while(u8::is_ascii_digit).count();
    if n > 0 {
        line[n..].strip_prefix(". ").map(|t| (true, t))
    } else {
        None
    }
}
/// True when the ordered marker of `line` is exactly `position + 1` in
/// canonical decimal form, i.e. what `export` would write for that item.
fn canonical_ordinal(line: &str, position: usize) -> bool {
    let n = line.bytes().take_while(u8::is_ascii_digit).count();
    position
        .checked_add(1)
        .is_some_and(|expected| line[..n] == expected.to_string())
}
fn heading(line: &str) -> Option<(u8, &str)> {
    let n = line.bytes().take_while(|b| *b == b'#').count();
    if (1..=6).contains(&n) {
        line[n..].strip_prefix(' ').map(|t| (n as u8, t))
    } else {
        None
    }
}
fn unsupported(line: &str) -> bool {
    // Subset is block-only; inline styling/links, HTML, images, tables,
    // indented/nested blocks, setext headings and thematic breaks are literal.
    let mut escaped = false;
    for c in line.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' {
            escaped = true;
            continue;
        }
        if matches!(c, '[' | ']' | '<' | '>' | '|' | '*' | '_' | '`') {
            return true;
        }
    }
    line.starts_with(' ')
        || line.starts_with('\t')
        || matches!(line, "---" | "===" | "***")
        || line.ends_with("  ")
        || line.ends_with('\\')
}
fn unescape(text: &str) -> String {
    let mut result = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek().is_some_and(|c| c.is_ascii_punctuation()) {
            result.push(chars.next().expect("peeked character"));
        } else {
            result.push(c);
        }
    }
    result
}
fn escape(text: &str) -> String {
    let mut result = String::new();
    for c in text.chars() {
        if c.is_ascii_punctuation() {
            result.push('\\');
        }
        result.push(c);
    }
    result
}
/// IDs come from an injected platform allocator. Fresh text imports create a
/// fresh document; text formats do not carry stable native identity/history.
pub fn import(
    format: Format,
    text: &str,
    id: DocumentId,
    mut next_id: impl FnMut() -> Result<ObjectId, Error>,
    limits: Limits,
    _provenance: Provenance,
) -> Result<ImportResult, Error> {
    if text.len() > limits.max_bytes {
        return Err(Error::LimitExceeded);
    }
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = normalized.split('\n').collect();
    let section = next_id()?;
    let mut document = Document::new(id, RevisionId(1), String::new())?;
    document.issue(section)?;
    document.sections.push(Section {
        id: section,
        title: String::new(),
        blocks: vec![],
    });
    let mut warnings = vec![];
    let mut i = 0;
    while i < lines.len() {
        if lines[i].is_empty() {
            i += 1;
            continue;
        }
        if document.issued_ids.len() >= limits.max_objects {
            return Err(Error::LimitExceeded);
        }
        let start = i;
        let line = lines[i];
        let kind;
        if format == Format::PlainText {
            while i < lines.len() && !lines[i].is_empty() {
                i += 1;
            }
            kind = BlockKind::Paragraph(lines[start..i].join("\n"));
        } else if let Some((c, n, language)) = fence(line) {
            i += 1;
            let content = i;
            while i < lines.len()
                && !fence(lines[i]).is_some_and(|(end_c, end_n, suffix)| {
                    c == end_c && end_n >= n && suffix.trim().is_empty()
                })
            {
                i += 1;
            }
            if i == lines.len() {
                kind = BlockKind::Paragraph(lines[start..i].join("\n"));
                warnings.push(warn(WarningKind::UnclosedFence, Some(start + 1), None));
            } else {
                kind = BlockKind::Code {
                    language: language.trim().into(),
                    text: lines[content..i].join("\n"),
                };
                i += 1;
            }
        } else if let Some((level, text)) = heading(line) {
            if unsupported(text) {
                warnings.push(warn(WarningKind::UnsupportedSyntax, Some(start + 1), None));
            }
            kind = BlockKind::Heading {
                level,
                text: if unsupported(text) {
                    text.into()
                } else {
                    unescape(text)
                },
            };
            i += 1;
        } else if let Some((ordered, _)) = list_line(line) {
            let mut raw = vec![];
            let mut canonical = true;
            while i < lines.len() {
                let Some((order, text)) = list_line(lines[i]) else {
                    break;
                };
                if ordered != order {
                    break;
                }
                if ordered && !canonical_ordinal(lines[i], raw.len()) {
                    canonical = false;
                }
                raw.push(text);
                i += 1;
            }
            if canonical {
                let mut items = vec![];
                for (offset, text) in raw.into_iter().enumerate() {
                    if unsupported(text) {
                        warnings.push(warn(
                            WarningKind::UnsupportedSyntax,
                            Some(start + offset + 1),
                            None,
                        ));
                        items.push(text.into());
                    } else {
                        items.push(unescape(text));
                    }
                }
                kind = BlockKind::List { ordered, items };
            } else {
                // The model cannot hold these ordinals: keep every line
                // byte-for-byte instead of renumbering, and report each one.
                for line_number in start + 1..=i {
                    warnings.push(warn(
                        WarningKind::UnsupportedSyntax,
                        Some(line_number),
                        None,
                    ));
                }
                kind = BlockKind::Paragraph(lines[start..i].join("\n"));
            }
        } else if line.starts_with("> ") || line == ">" {
            let mut content = vec![];
            while i < lines.len() && (lines[i].starts_with("> ") || lines[i] == ">") {
                let t = lines[i].strip_prefix("> ").unwrap_or_default();
                if unsupported(t) {
                    warnings.push(warn(WarningKind::UnsupportedSyntax, Some(i + 1), None));
                    content.push(t.into());
                } else {
                    content.push(unescape(t));
                }
                i += 1;
            }
            kind = BlockKind::Quote(content.join("\n"));
        } else {
            i += 1;
            while i < lines.len()
                && !lines[i].is_empty()
                && heading(lines[i]).is_none()
                && fence(lines[i]).is_none()
                && list_line(lines[i]).is_none()
                && !lines[i].starts_with("> ")
                && lines[i] != ">"
            {
                i += 1;
            }
            let mut content = vec![];
            for (offset, t) in lines[start..i].iter().enumerate() {
                if unsupported(t) {
                    warnings.push(warn(
                        WarningKind::UnsupportedSyntax,
                        Some(start + offset + 1),
                        None,
                    ));
                    content.push((*t).into());
                } else {
                    content.push(unescape(t));
                }
            }
            kind = BlockKind::Paragraph(content.join("\n"));
        }
        let object = next_id()?;
        document.issue(object)?;
        document.sections[0].blocks.push(Block::new(object, kind));
    }
    document.validate(limits)?;
    if format == Format::PlainText {
        let canonical = document.sections[0]
            .blocks
            .iter()
            .filter_map(|b| b.kind.text())
            .collect::<Vec<_>>()
            .join("\n\n");
        if canonical != normalized {
            warnings.push(warn(WarningKind::StructuralLoss, None, None));
        }
    }
    Ok(ImportResult { document, warnings })
}
/// Returns warnings with output before any caller writes a file. Native IDs,
/// styles, comments and review metadata require a native provider snapshot.
pub fn export(format: Format, document: &Document, limits: Limits) -> Result<ExportResult, Error> {
    document.validate(limits)?;
    let mut parts = vec![];
    let mut warnings = vec![warn(WarningKind::NativeMetadata, None, None)];
    for s in &document.sections {
        if !s.title.is_empty() {
            parts.push(if format == Format::Markdown {
                escape(&s.title)
            } else {
                s.title.clone()
            });
            warnings.push(warn(WarningKind::StructuralLoss, None, Some(s.id)));
        }
        for b in &s.blocks {
            if b.kind
                .text()
                .is_some_and(|t| t.contains('\r') || t.starts_with('\n') || t.ends_with('\n'))
            {
                warnings.push(warn(WarningKind::StructuralLoss, None, Some(b.id)));
            }
            let text = match (&b.kind, format) {
                (BlockKind::Paragraph(t), Format::Markdown) => {
                    if t.is_empty()
                        || t.contains("\n\n")
                        || t.lines()
                            .any(|l| l.starts_with(' ') || l.starts_with('\t') || l.ends_with(' '))
                    {
                        warnings.push(warn(WarningKind::StructuralLoss, None, Some(b.id)));
                    }
                    escape(t)
                }
                (BlockKind::Heading { level, text }, Format::Markdown) if !text.contains('\n') => {
                    format!("{} {}", "#".repeat(*level as usize), escape(text))
                }
                (BlockKind::List { ordered, items }, Format::Markdown) => {
                    if items.is_empty() || items.iter().any(|s| s.contains('\n')) {
                        warnings.push(warn(WarningKind::StructuralLoss, None, Some(b.id)));
                    }
                    items
                        .iter()
                        .enumerate()
                        .map(|(i, s)| {
                            format!(
                                "{} {}",
                                if *ordered {
                                    format!("{}.", i + 1)
                                } else {
                                    "-".into()
                                },
                                escape(s)
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                }
                (BlockKind::Quote(t), Format::Markdown) => t
                    .split('\n')
                    .map(|l| format!("> {}", escape(l)))
                    .collect::<Vec<_>>()
                    .join("\n"),
                (BlockKind::Code { language, text }, Format::Markdown)
                    if !language.contains(['\n', '\r', '`']) =>
                {
                    let max_run = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
                    let f = "`".repeat(3.max(max_run + 1));
                    format!("{f}{language}\n{text}\n{f}")
                }
                (BlockKind::Table(table), _) => {
                    warnings.push(warn(WarningKind::UnsupportedBlock, None, Some(b.id)));
                    let raw = table
                        .rows
                        .iter()
                        .map(|r| r.join("\t"))
                        .collect::<Vec<_>>()
                        .join("\n");
                    if format == Format::Markdown {
                        escape(&raw)
                    } else {
                        raw
                    }
                }
                (BlockKind::List { items, .. }, _) => {
                    warnings.push(warn(WarningKind::StructuralLoss, None, Some(b.id)));
                    items.join("\n")
                }
                (BlockKind::Citation(r) | BlockKind::LinkedReference(r), _) => {
                    warnings.push(warn(WarningKind::UnsupportedBlock, None, Some(b.id)));
                    let raw = format!(
                        "{} [resource:{} revision:{:?} object:{:?}] {}",
                        r.label,
                        r.resource.0,
                        r.revision.map(|r| r.0),
                        r.object.map(|o| o.0),
                        r.locator.as_deref().unwrap_or_default()
                    );
                    if format == Format::Markdown {
                        escape(&raw)
                    } else {
                        raw
                    }
                }
                (BlockKind::Figure { alt, source }, _) => {
                    warnings.push(warn(WarningKind::UnsupportedBlock, None, Some(b.id)));
                    let raw = format!("{alt} {source:?}");
                    if format == Format::Markdown {
                        escape(&raw)
                    } else {
                        raw
                    }
                }
                (kind, _) => {
                    if !matches!(kind, BlockKind::Paragraph(_)) {
                        warnings.push(warn(WarningKind::StructuralLoss, None, Some(b.id)));
                    }
                    let t = kind.text().ok_or(Error::Unsupported)?;
                    if format == Format::Markdown {
                        escape(t)
                    } else {
                        t.into()
                    }
                }
            };
            parts.push(text);
        }
    }
    let text = parts.join("\n\n");
    if text.len() > limits.max_bytes {
        return Err(Error::LimitExceeded);
    }
    Ok(ExportResult { text, warnings })
}
