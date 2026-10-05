use core::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TemplatePart {
    Literal(String),
    Argument(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TemplateError {
    byte_offset: usize,
    reason: &'static str,
}

impl TemplateError {
    fn new(byte_offset: usize, reason: &'static str) -> Self {
        Self {
            byte_offset,
            reason,
        }
    }
}

impl fmt::Display for TemplateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} at byte {}", self.reason, self.byte_offset)
    }
}

impl std::error::Error for TemplateError {}

pub(crate) fn parse_template(template: &str) -> Result<Vec<TemplatePart>, TemplateError> {
    let chars: Vec<(usize, char)> = template.char_indices().collect();
    let mut parts = Vec::new();
    let mut literal = String::new();
    let mut index = 0;

    while index < chars.len() {
        let (offset, character) = chars[index];
        if character == '{' {
            if chars.get(index + 1).is_some_and(|(_, next)| *next == '{') {
                literal.push('{');
                index += 2;
                continue;
            }
            if !literal.is_empty() {
                parts.push(TemplatePart::Literal(core::mem::take(&mut literal)));
            }
            let start = index + 1;
            let mut end = start;
            while end < chars.len() && chars[end].1 != '}' {
                if chars[end].1 == '{' {
                    return Err(TemplateError::new(chars[end].0, "nested opening brace"));
                }
                end += 1;
            }
            if end == chars.len() {
                return Err(TemplateError::new(offset, "unclosed placeholder"));
            }
            let name: String = chars[start..end].iter().map(|(_, ch)| *ch).collect();
            if !valid_argument_name(&name) {
                return Err(TemplateError::new(offset, "invalid placeholder name"));
            }
            parts.push(TemplatePart::Argument(name));
            index = end + 1;
        } else if character == '}' {
            if chars.get(index + 1).is_some_and(|(_, next)| *next == '}') {
                literal.push('}');
                index += 2;
            } else {
                return Err(TemplateError::new(offset, "unmatched closing brace"));
            }
        } else {
            literal.push(character);
            index += 1;
        }
    }
    if !literal.is_empty() {
        parts.push(TemplatePart::Literal(literal));
    }
    Ok(parts)
}

pub(crate) fn valid_argument_name(name: &str) -> bool {
    !name.is_empty()
        && name.as_bytes()[0].is_ascii_alphabetic()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

#[cfg(test)]
mod tests {
    use super::{parse_template, TemplatePart};

    #[test]
    fn supports_named_arguments_and_escaped_braces() {
        assert_eq!(
            parse_template("{{{name}}}").unwrap(),
            [
                TemplatePart::Literal("{".to_owned()),
                TemplatePart::Argument("name".to_owned()),
                TemplatePart::Literal("}".to_owned())
            ]
        );
    }

    #[test]
    fn rejects_malformed_argument_syntax() {
        for template in ["{name", "name}", "{}", "{a b}", "{{name}}", "{outer{name}}"] {
            if template == "{{name}}" {
                // Doubled braces intentionally escape a literal placeholder.
                assert_eq!(
                    parse_template(template).unwrap(),
                    [TemplatePart::Literal("{name}".into())]
                );
            } else {
                assert!(parse_template(template).is_err(), "accepted {template:?}");
            }
        }
    }
}
