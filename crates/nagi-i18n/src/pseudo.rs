use core::fmt;

use crate::template::{parse_template, TemplateError, TemplatePart};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PseudoLocaleError(TemplateError);

impl fmt::Display for PseudoLocaleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "cannot pseudo-localize malformed message: {}",
            self.0
        )
    }
}

impl std::error::Error for PseudoLocaleError {}

/// Expand and mark literal text while leaving named interpolation arguments
/// intact. The result is for layout and fallback-path testing, not translation.
pub fn pseudo_localize(source: &str) -> Result<String, PseudoLocaleError> {
    let parts = parse_template(source).map_err(PseudoLocaleError)?;
    let mut result = String::from("⟦");
    for part in parts {
        match part {
            TemplatePart::Literal(text) => {
                for character in text.chars() {
                    match character.to_ascii_lowercase() {
                        'a' => result.push_str("áa"),
                        'e' => result.push_str("ée"),
                        'i' => result.push_str("íi"),
                        'o' => result.push_str("óo"),
                        'u' => result.push_str("úu"),
                        _ => result.push(character),
                    }
                }
            }
            TemplatePart::Argument(name) => {
                result.push('{');
                result.push_str(&name);
                result.push('}');
            }
        }
    }
    result.push('⟧');
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::pseudo_localize;

    #[test]
    fn expands_literals_without_changing_placeholders() {
        assert_eq!(pseudo_localize("Save {name}").unwrap(), "⟦Sáavée {name}⟧");
        assert!(pseudo_localize("broken {name").is_err());
    }
}
