#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LicenseAssessment {
    DeclaredExpression(String),
    Unknown,
    MalformedExpression(String),
}

const SPDX_LICENSE_IDS: &str = include_str!("../data/spdx-license-ids.txt");
const SPDX_EXCEPTION_IDS: &str = include_str!("../data/spdx-exception-ids.txt");
pub const SPDX_LICENSE_LIST_VERSION: &str = "3.29.0";

pub fn assess_license_expression(value: Option<&str>) -> LicenseAssessment {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return LicenseAssessment::Unknown;
    };
    if is_spdx_expression(value) {
        LicenseAssessment::DeclaredExpression(value.to_owned())
    } else if looks_like_spdx_expression_syntax(value) {
        LicenseAssessment::MalformedExpression(value.to_owned())
    } else {
        LicenseAssessment::Unknown
    }
}

fn looks_like_spdx_expression_syntax(value: &str) -> bool {
    if value.contains(['/', ';', ',']) {
        return false;
    }
    value
        .split(|character: char| !(character.is_ascii_alphanumeric() || character == '-'))
        .any(|word| matches!(word, "AND" | "OR" | "WITH"))
        || value.trim_start().starts_with('(')
        || value.trim_end().ends_with(')')
}

pub fn is_spdx_expression(value: &str) -> bool {
    let Some(tokens) = tokenize(value) else {
        return false;
    };
    let mut parser = Parser { tokens, cursor: 0 };
    parser.parse_expression() && parser.cursor == parser.tokens.len()
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    Identifier(String),
    And,
    Or,
    With,
    LeftParen,
    RightParen,
}

fn tokenize(value: &str) -> Option<Vec<Token>> {
    let mut tokens = Vec::new();
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.peek().copied() {
        if ch.is_whitespace() {
            chars.next();
            continue;
        }
        match ch {
            '(' => {
                chars.next();
                tokens.push(Token::LeftParen);
            }
            ')' => {
                chars.next();
                tokens.push(Token::RightParen);
            }
            _ => {
                let mut atom = String::new();
                while let Some(next) = chars.peek().copied() {
                    if next.is_whitespace() || matches!(next, '(' | ')') {
                        break;
                    }
                    if !(next.is_ascii_alphanumeric()
                        || matches!(next, '.' | '+' | '-' | ':' | '_'))
                    {
                        return None;
                    }
                    atom.push(next);
                    chars.next();
                }
                if atom.is_empty() {
                    return None;
                }
                tokens.push(match atom.as_str() {
                    "AND" => Token::And,
                    "OR" => Token::Or,
                    "WITH" => Token::With,
                    _ => Token::Identifier(atom),
                });
            }
        }
    }
    (!tokens.is_empty()).then_some(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    cursor: usize,
}

impl Parser {
    fn parse_expression(&mut self) -> bool {
        if !self.parse_and_expression() {
            return false;
        }
        while self.peek() == Some(&Token::Or) {
            self.cursor += 1;
            if !self.parse_and_expression() {
                return false;
            }
        }
        true
    }

    fn parse_and_expression(&mut self) -> bool {
        if !self.parse_with_expression() {
            return false;
        }
        while self.peek() == Some(&Token::And) {
            self.cursor += 1;
            if !self.parse_with_expression() {
                return false;
            }
        }
        true
    }

    fn parse_with_expression(&mut self) -> bool {
        if !self.parse_atom() {
            return false;
        }
        if self.peek() == Some(&Token::With) {
            self.cursor += 1;
            if !matches!(self.peek(), Some(Token::Identifier(id)) if is_spdx_exception_id(id)) {
                return false;
            }
            self.cursor += 1;
        }
        true
    }

    fn parse_atom(&mut self) -> bool {
        match self.peek() {
            Some(Token::Identifier(id)) if is_spdx_license_id(id) => {
                self.cursor += 1;
                true
            }
            Some(Token::LeftParen) => {
                self.cursor += 1;
                if !self.parse_expression() || self.peek() != Some(&Token::RightParen) {
                    return false;
                }
                self.cursor += 1;
                true
            }
            _ => false,
        }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.cursor)
    }
}

fn is_spdx_license_id(value: &str) -> bool {
    SPDX_LICENSE_IDS.lines().any(|candidate| candidate == value)
}

fn is_spdx_exception_id(value: &str) -> bool {
    SPDX_EXCEPTION_IDS
        .lines()
        .any(|candidate| candidate == value)
}

#[cfg(test)]
mod tests {
    use super::{assess_license_expression, is_spdx_expression, LicenseAssessment};

    #[test]
    fn preserves_explicit_spdx_expression() {
        let expression = "Apache-2.0 OR MIT";
        assert_eq!(
            assess_license_expression(Some(expression)),
            LicenseAssessment::DeclaredExpression(expression.to_owned())
        );
    }

    #[test]
    fn unknown_and_prose_are_not_guessed() {
        assert_eq!(assess_license_expression(None), LicenseAssessment::Unknown);
        assert_eq!(
            assess_license_expression(Some("")),
            LicenseAssessment::Unknown
        );
        assert!(!is_spdx_expression(
            "MIT core/Gallium; component notices required"
        ));
        assert_eq!(
            assess_license_expression(Some("MIT core/Gallium; component notices required")),
            LicenseAssessment::Unknown
        );
        assert_eq!(
            assess_license_expression(Some("DefinitelyNotAnSpdxId")),
            LicenseAssessment::Unknown
        );
    }

    #[test]
    fn parses_precedence_parentheses_and_exceptions() {
        assert!(is_spdx_expression("MIT OR Apache-2.0 AND BSD-3-Clause"));
        assert!(is_spdx_expression(
            "(MIT OR Apache-2.0) AND GPL-2.0-only WITH Classpath-exception-2.0"
        ));
        assert!(!is_spdx_expression("LicenseRef-Nagi-Custom"));
        assert!(!is_spdx_expression("DefinitelyNotAnSpdxId"));
        assert!(!is_spdx_expression("MIT WITH Not-A-Real-Exception"));
    }

    #[test]
    fn rejects_malformed_claims() {
        for value in [
            "MIT OR",
            "AND MIT",
            "MIT WITH",
            "MIT) OR Apache-2.0",
            "MIT; maybe Apache",
        ] {
            assert!(!is_spdx_expression(value), "accepted {value}");
        }
    }
}
