use std::collections::{HashMap, HashSet};
use std::fmt;

use nagi_model::{ObjectId, WorkspaceId};

use crate::domain::{AlbertReference, Block, BlockKind, NoteDocument, Timestamp};
use crate::identity::ObjectIdSource;
use crate::references::{object_id_from_uri, valid_image_target, valid_web_url};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MarkdownError {
    MissingFrontMatter,
    InvalidFrontMatter,
    MissingField(&'static str),
    InvalidField(&'static str),
    UnsupportedVersion,
}

impl fmt::Display for MarkdownError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingFrontMatter => f.write_str("missing Nagi Notes front matter"),
            Self::InvalidFrontMatter => f.write_str("invalid Nagi Notes front matter"),
            Self::MissingField(field) => write!(f, "missing front matter field: {field}"),
            Self::InvalidField(field) => write!(f, "invalid front matter field: {field}"),
            Self::UnsupportedVersion => f.write_str("unsupported Nagi Notes file version"),
        }
    }
}

impl std::error::Error for MarkdownError {}

pub type StoredNoteError = MarkdownError;

/// Import portable Markdown. The first level-one heading becomes the note
/// title; other Markdown constructs remain represented as blocks.
pub fn import_markdown(
    id: ObjectId,
    markdown: &str,
    ids: &dyn ObjectIdSource,
    now: Timestamp,
) -> NoteDocument {
    let mut title = None;
    let mut body = Vec::new();
    for line in markdown.lines() {
        if title.is_none() {
            if let Some((_, text)) = heading(line).filter(|(level, _)| *level == 1) {
                title = Some(text.trim().to_owned());
                continue;
            }
        }
        body.push(line);
    }
    NoteDocument {
        id,
        title: title.unwrap_or_else(|| "Untitled".to_owned()),
        blocks: parse_blocks(&body.join("\n"), ids),
        created_at: now,
        updated_at: now,
        revision: 0,
        workspace_id: None,
        workspace_ids: Vec::new(),
        tags: Vec::new(),
        deleted: false,
    }
}

/// Export ordinary Markdown without Nagi's persistence metadata or block IDs.
pub fn export_markdown(note: &NoteDocument) -> String {
    let title = note.title.lines().collect::<Vec<_>>().join(" ");
    let body = render_blocks(&note.blocks, false);
    if body.is_empty() {
        format!("# {title}\n")
    } else {
        format!("# {title}\n\n{body}\n")
    }
}

pub(crate) fn encode_stored(note: &NoteDocument) -> String {
    let workspace = note
        .workspace_id
        .map(|id| format!("{:016x}", id.0))
        .unwrap_or_else(|| "-".to_owned());
    format!(
        "---\n\
         nagi-notes-version: 1\n\
         object-id: {:016x}\n\
         title: \"{}\"\n\
         created-at-ms: {}\n\
         updated-at-ms: {}\n\
         revision: {}\n\
         workspace-id: {}\n\
         workspace-ids: {}\n\
         tags: {}\n\
         deleted: {}\n\
         ---\n\
         {}\n",
        note.id.0,
        escape_yaml(&note.title),
        note.created_at.0,
        note.updated_at.0,
        note.revision,
        workspace,
        encode_string_list(
            &note
                .workspace_ids
                .iter()
                .map(|id| format!("{:016x}", id.0))
                .collect::<Vec<_>>()
        ),
        encode_string_list(&note.tags),
        note.deleted,
        render_blocks(&note.blocks, true)
    )
}

pub(crate) fn decode_stored(
    input: &str,
    ids: &dyn ObjectIdSource,
) -> Result<NoteDocument, MarkdownError> {
    let (metadata, body) = split_front_matter(input)?;
    let mut fields = HashMap::<&str, &str>::new();
    for line in metadata.lines() {
        let (key, value) = line
            .split_once(':')
            .ok_or(MarkdownError::InvalidFrontMatter)?;
        if fields.insert(key.trim(), value.trim()).is_some() {
            return Err(MarkdownError::InvalidFrontMatter);
        }
    }
    let field = |name: &'static str| {
        fields
            .get(name)
            .copied()
            .ok_or(MarkdownError::MissingField(name))
    };
    if field("nagi-notes-version")? != "1" {
        return Err(MarkdownError::UnsupportedVersion);
    }
    let id = parse_id(field("object-id")?).ok_or(MarkdownError::InvalidField("object-id"))?;
    let title = unescape_yaml(field("title")?).ok_or(MarkdownError::InvalidField("title"))?;
    let created_at = parse_u64(field("created-at-ms")?, "created-at-ms")?;
    let updated_at = parse_u64(field("updated-at-ms")?, "updated-at-ms")?;
    let revision = parse_u64(field("revision")?, "revision")?;
    if revision == 0 {
        return Err(MarkdownError::InvalidField("revision"));
    }
    let workspace_value = field("workspace-id")?;
    let workspace_id = if workspace_value == "-" {
        None
    } else {
        Some(WorkspaceId(
            parse_id(workspace_value)
                .ok_or(MarkdownError::InvalidField("workspace-id"))?
                .0,
        ))
    };
    let workspace_ids = decode_string_list(field("workspace-ids")?)
        .ok_or(MarkdownError::InvalidField("workspace-ids"))?
        .into_iter()
        .map(|value| {
            parse_id(&value)
                .map(|id| WorkspaceId(id.0))
                .ok_or(MarkdownError::InvalidField("workspace-ids"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let tags = decode_string_list(field("tags")?).ok_or(MarkdownError::InvalidField("tags"))?;
    let deleted = match field("deleted")? {
        "true" => true,
        "false" => false,
        _ => return Err(MarkdownError::InvalidField("deleted")),
    };
    Ok(NoteDocument {
        id,
        title,
        blocks: parse_blocks(body, ids),
        created_at: Timestamp(created_at),
        updated_at: Timestamp(updated_at),
        revision,
        workspace_id,
        workspace_ids,
        tags,
        deleted,
    })
}

fn split_front_matter(input: &str) -> Result<(&str, &str), MarkdownError> {
    let input = input
        .strip_prefix("---\n")
        .ok_or(MarkdownError::MissingFrontMatter)?;
    let end = input
        .find("\n---\n")
        .ok_or(MarkdownError::InvalidFrontMatter)?;
    let body_start = end + "\n---\n".len();
    Ok((&input[..end], &input[body_start..]))
}

fn parse_u64(value: &str, field: &'static str) -> Result<u64, MarkdownError> {
    value
        .parse()
        .map_err(|_| MarkdownError::InvalidField(field))
}

fn parse_id(value: &str) -> Option<ObjectId> {
    if value.len() != 16 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    u64::from_str_radix(value, 16).ok().map(ObjectId)
}

fn escape_yaml(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            other => escaped.push(other),
        }
    }
    escaped
}

fn encode_string_list(values: &[String]) -> String {
    if values.is_empty() {
        return "[]".to_owned();
    }
    let encoded = values
        .iter()
        .map(|value| format!("\"{}\"", escape_yaml(value)))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{encoded}]")
}

fn decode_string_list(input: &str) -> Option<Vec<String>> {
    let mut input = input.strip_prefix('[')?.strip_suffix(']')?.trim();
    let mut values = Vec::new();
    while !input.is_empty() {
        let end = quoted_string_end(input)?;
        values.push(unescape_yaml(&input[..end])?);
        input = input[end..].trim_start();
        if input.is_empty() {
            break;
        }
        input = input.strip_prefix(',')?.trim_start();
    }
    Some(values)
}

fn quoted_string_end(input: &str) -> Option<usize> {
    if !input.starts_with('"') {
        return None;
    }
    let mut escaped = false;
    for (index, character) in input.char_indices().skip(1) {
        if escaped {
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == '"' {
            return Some(index + character.len_utf8());
        }
    }
    None
}

fn unescape_yaml(value: &str) -> Option<String> {
    let value = value.strip_prefix('"')?.strip_suffix('"')?;
    let mut output = String::with_capacity(value.len());
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            output.push(character);
            continue;
        }
        output.push(match characters.next()? {
            '\\' => '\\',
            '"' => '"',
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            _ => return None,
        });
    }
    Some(output)
}

fn render_blocks(blocks: &[Block], include_ids: bool) -> String {
    let mut output = String::new();
    for (index, block) in blocks.iter().enumerate() {
        if index > 0 {
            output.push_str("\n\n");
        }
        if include_ids {
            if let BlockKind::AlbertReference(reference) = &block.kind {
                output.push_str(&format!(
                    "<!--nagi:block-id={:016x};albert-page={:016x};albert-title={};albert-url={};albert-selection={}-->\n",
                    block.id.0,
                    reference.page_id.0,
                    hex_encode(reference.title.as_bytes()),
                    hex_encode(reference.url.as_bytes()),
                    reference
                        .selection
                        .as_deref()
                        .map(|selection| hex_encode(selection.as_bytes()))
                        .unwrap_or_else(|| "-".to_owned()),
                ));
            } else {
                output.push_str(&format!("<!--nagi:block-id={:016x}-->\n", block.id.0));
            }
        }
        render_block(&mut output, &block.kind);
    }
    output
}

fn render_block(output: &mut String, block: &BlockKind) {
    match block {
        BlockKind::Heading { level, text } => {
            let level = (*level).clamp(1, 6);
            output.push_str(&"#".repeat(level as usize));
            output.push(' ');
            output.push_str(text);
        }
        BlockKind::Paragraph(text) => output.push_str(text),
        BlockKind::Checklist { text, completed } => {
            output.push_str(if *completed { "- [x] " } else { "- [ ] " });
            output.push_str(text);
        }
        BlockKind::Code { language, text } => {
            let marker = fence_for(text);
            output.push_str(&marker);
            if let Some(language) = language {
                output.push_str(language);
            }
            output.push('\n');
            output.push_str(text);
            if !text.ends_with('\n') {
                output.push('\n');
            }
            output.push_str(&marker);
        }
        BlockKind::Quote(text) => {
            for (index, line) in text.lines().enumerate() {
                if index > 0 {
                    output.push('\n');
                }
                output.push_str("> ");
                output.push_str(line);
            }
        }
        BlockKind::ImageReference { alt, target } => {
            output.push_str("![");
            output.push_str(alt);
            output.push_str("](");
            output.push_str(target);
            output.push(')');
        }
        BlockKind::FileReference { label, object_id } => {
            output.push('[');
            output.push_str(label);
            output.push_str("](nagi-object://");
            output.push_str(&format!("{:016x}", object_id.0));
            output.push(')');
        }
        BlockKind::WebReference { title, url } => {
            output.push('[');
            output.push_str(title);
            output.push_str("](");
            output.push_str(url);
            output.push(')');
        }
        BlockKind::AlbertReference(reference) => {
            output.push('[');
            output.push_str(&reference.title);
            output.push_str("](");
            output.push_str(&reference.url);
            output.push(')');
            if let Some(selection) = &reference.selection {
                for line in selection.lines() {
                    output.push_str("\n> ");
                    output.push_str(line);
                }
                if selection.is_empty() {
                    output.push_str("\n>");
                }
            }
        }
        BlockKind::Table { rows } => {
            let width = rows.iter().map(Vec::len).max().unwrap_or(0);
            for (index, row) in rows.iter().enumerate() {
                if index > 0 {
                    output.push('\n');
                }
                render_table_row(output, row, width);
                if index == 0 {
                    output.push('\n');
                    output.push('|');
                    for _ in 0..width {
                        output.push_str(" --- |");
                    }
                }
            }
        }
        BlockKind::Callout { kind, text } => {
            let mut lines = text.lines();
            output.push_str("> [!");
            output.push_str(kind);
            output.push_str("] ");
            output.push_str(lines.next().unwrap_or_default());
            for line in lines {
                output.push_str("\n> ");
                output.push_str(line);
            }
        }
    }
}

fn render_table_row(output: &mut String, row: &[String], width: usize) {
    output.push('|');
    for index in 0..width {
        output.push(' ');
        if let Some(cell) = row.get(index) {
            output.push_str(&cell.replace('|', "\\|"));
        }
        output.push_str(" |");
    }
}

fn fence_for(text: &str) -> String {
    let longest = text
        .split(|character| character != '\x60')
        .map(str::len)
        .max()
        .unwrap_or(0);
    "\x60".repeat(longest.max(2) + 1)
}

fn parse_blocks(markdown: &str, ids: &dyn ObjectIdSource) -> Vec<Block> {
    let lines = markdown.lines().collect::<Vec<_>>();
    let mut output = Vec::new();
    let mut seen = HashSet::new();
    let mut pending_marker = None;
    let mut index = 0;

    while index < lines.len() {
        let line = lines[index].trim_end_matches('\r');
        if line.trim().is_empty() {
            index += 1;
            continue;
        }
        if let Some(marker) = block_marker(line) {
            pending_marker = Some(marker);
            index += 1;
            continue;
        }

        let albert_reference = pending_marker
            .as_ref()
            .and_then(|marker: &BlockMarker| marker.albert_reference.clone());
        let kind = if let Some(reference) = albert_reference {
            index += 1;
            if reference.selection.is_some() {
                while index < lines.len()
                    && lines[index]
                        .trim_end_matches('\r')
                        .trim_start()
                        .starts_with('>')
                {
                    index += 1;
                }
            }
            BlockKind::AlbertReference(reference)
        } else if let Some((level, text)) = heading(line) {
            index += 1;
            BlockKind::Heading { level, text }
        } else if let Some((marker, language)) = opening_fence(line) {
            index += 1;
            let mut code = Vec::new();
            while index < lines.len() && !closes_fence(lines[index], &marker) {
                code.push(lines[index].trim_end_matches('\r'));
                index += 1;
            }
            if index < lines.len() {
                index += 1;
            }
            BlockKind::Code {
                language,
                text: code.join("\n"),
            }
        } else if is_table_start(&lines, index) {
            let mut rows = vec![parse_table_row(lines[index])];
            index += 2;
            while index < lines.len() && lines[index].trim_start().starts_with('|') {
                rows.push(parse_table_row(lines[index]));
                index += 1;
            }
            BlockKind::Table { rows }
        } else if let Some((text, completed)) = checklist(line) {
            index += 1;
            BlockKind::Checklist { text, completed }
        } else if let Some((alt, target)) = image_reference(line) {
            index += 1;
            if valid_image_target(&target) {
                BlockKind::ImageReference { alt, target }
            } else {
                BlockKind::Paragraph(line.to_owned())
            }
        } else if let Some((label, target)) = markdown_link(line) {
            index += 1;
            if let Some(object_id) = object_id_from_uri(&target) {
                BlockKind::FileReference { label, object_id }
            } else if valid_web_url(&target) {
                BlockKind::WebReference {
                    title: label,
                    url: target,
                }
            } else {
                BlockKind::Paragraph(line.to_owned())
            }
        } else if line.trim_start().starts_with('>') {
            let mut quote_lines = Vec::new();
            while index < lines.len() && lines[index].trim_start().starts_with('>') {
                let trimmed = lines[index].trim_start();
                let quoted = trimmed
                    .strip_prefix('>')
                    .unwrap_or_default()
                    .strip_prefix(' ')
                    .unwrap_or_else(|| trimmed.strip_prefix('>').unwrap_or_default());
                quote_lines.push(quoted);
                index += 1;
            }
            let text = quote_lines.join("\n");
            if let Some((kind, first)) = parse_callout_header(&text) {
                let rest = text.find('\n').map(|offset| &text[offset + 1..]);
                BlockKind::Callout {
                    kind,
                    text: match rest {
                        Some(rest) if !rest.is_empty() => format!("{first}\n{rest}"),
                        _ => first,
                    },
                }
            } else {
                BlockKind::Quote(text)
            }
        } else {
            let mut paragraph = vec![line];
            index += 1;
            while index < lines.len()
                && !lines[index].trim().is_empty()
                && !starts_block(&lines, index)
            {
                paragraph.push(lines[index].trim_end_matches('\r'));
                index += 1;
            }
            BlockKind::Paragraph(paragraph.join("\n"))
        };

        let id = pending_marker
            .take()
            .map(|marker| marker.id)
            .unwrap_or_else(|| ids.next_object_id());
        let id = if seen.insert(id.0) {
            id
        } else {
            let mut replacement = ids.next_object_id();
            while !seen.insert(replacement.0) {
                replacement = ids.next_object_id();
            }
            replacement
        };
        output.push(Block::new(id, kind));
    }
    output
}

fn starts_block(lines: &[&str], index: usize) -> bool {
    let line = lines[index].trim_end_matches('\r');
    heading(line).is_some()
        || opening_fence(line).is_some()
        || checklist(line).is_some()
        || image_reference(line).is_some()
        || markdown_link(line).is_some()
        || line.trim_start().starts_with('>')
        || is_table_start(lines, index)
        || block_marker(line).is_some()
}

fn heading(line: &str) -> Option<(u8, String)> {
    let prefix_len = line.bytes().take_while(|byte| *byte == b'#').count();
    if prefix_len == 0 || prefix_len > 6 || line.as_bytes().get(prefix_len) != Some(&b' ') {
        return None;
    }
    Some((
        prefix_len as u8,
        line[prefix_len + 1..].trim_end().to_owned(),
    ))
}

fn checklist(line: &str) -> Option<(String, bool)> {
    let line = line.trim_start();
    if let Some(text) = line.strip_prefix("- [ ] ") {
        Some((text.to_owned(), false))
    } else {
        line.strip_prefix("- [x] ")
            .or_else(|| line.strip_prefix("- [X] "))
            .map(|text| (text.to_owned(), true))
    }
}

fn opening_fence(line: &str) -> Option<(String, Option<String>)> {
    let trimmed = line.trim_start();
    let marker = trimmed.chars().next()?;
    if marker != '\x60' && marker != '~' {
        return None;
    }
    let count = trimmed
        .chars()
        .take_while(|character| *character == marker)
        .count();
    if count < 3 {
        return None;
    }
    let language = trimmed[count..].trim();
    Some((
        marker.to_string().repeat(count),
        (!language.is_empty()).then(|| language.to_owned()),
    ))
}

fn closes_fence(line: &str, marker: &str) -> bool {
    let trimmed = line.trim_start();
    let expected = marker.chars().next().unwrap_or('\x60');
    let count = trimmed
        .chars()
        .take_while(|character| *character == expected)
        .count();
    count >= marker.len() && trimmed[count..].trim().is_empty()
}

fn is_table_start(lines: &[&str], index: usize) -> bool {
    if index + 1 >= lines.len() || !lines[index].trim_start().starts_with('|') {
        return false;
    }
    let cells = parse_table_row(lines[index + 1]);
    !cells.is_empty()
        && cells.iter().all(|cell| {
            let cell = cell.trim_matches(':').trim();
            !cell.is_empty() && cell.chars().all(|character| character == '-')
        })
}

fn parse_table_row(line: &str) -> Vec<String> {
    line.trim()
        .trim_start_matches('|')
        .trim_end_matches('|')
        .split('|')
        .map(|cell| cell.replace("\\|", "|").trim().to_owned())
        .collect()
}

fn image_reference(line: &str) -> Option<(String, String)> {
    let link = line.strip_prefix("![")?;
    let end = link.find("](")?;
    let target = link[end + 2..].strip_suffix(')')?;
    Some((link[..end].to_owned(), target.to_owned()))
}

fn markdown_link(line: &str) -> Option<(String, String)> {
    if line.starts_with("![") {
        return None;
    }
    let link = line.strip_prefix('[')?;
    let end = link.find("](")?;
    let target = link[end + 2..].strip_suffix(')')?;
    Some((link[..end].to_owned(), target.to_owned()))
}

fn parse_callout_header(text: &str) -> Option<(String, String)> {
    let header = text.strip_prefix("[!")?;
    let end = header.find(']')?;
    let kind = header[..end].to_owned();
    let first = header[end + 1..].trim_start().to_owned();
    Some((kind, first))
}

#[derive(Clone, Debug)]
struct BlockMarker {
    id: ObjectId,
    albert_reference: Option<AlbertReference>,
}

fn block_marker(line: &str) -> Option<BlockMarker> {
    let marker = line
        .trim()
        .strip_prefix("<!--nagi:block-id=")?
        .strip_suffix("-->")?;
    let Some((id, metadata)) = marker.split_once(';') else {
        return Some(BlockMarker {
            id: parse_id(marker)?,
            albert_reference: None,
        });
    };
    let id = parse_id(id)?;
    let mut fields = metadata.split(';');
    let page_id = parse_id(fields.next()?.strip_prefix("albert-page=")?)?;
    let title = hex_decode_utf8(fields.next()?.strip_prefix("albert-title=")?)?;
    let url = hex_decode_utf8(fields.next()?.strip_prefix("albert-url=")?)?;
    let selection_value = fields.next()?.strip_prefix("albert-selection=")?;
    let selection = if selection_value == "-" {
        None
    } else {
        Some(hex_decode_utf8(selection_value)?)
    };
    if fields.next().is_some() {
        return None;
    }
    Some(BlockMarker {
        id,
        albert_reference: Some(AlbertReference {
            page_id,
            title,
            url,
            selection,
        }),
    })
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn hex_decode_utf8(value: &str) -> Option<String> {
    if !value.len().is_multiple_of(2) || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let bytes = value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = (pair[0] as char).to_digit(16)? as u8;
            let low = (pair[1] as char).to_digit(16)? as u8;
            Some((high << 4) | low)
        })
        .collect::<Option<Vec<_>>>()?;
    String::from_utf8(bytes).ok()
}
