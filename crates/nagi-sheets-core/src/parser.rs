use crate::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FunctionId {
    Sum,
    Average,
    Min,
    Max,
    Count,
    CountA,
    If,
    And,
    Or,
    Not,
    Round,
    IfError,
    Left,
    Right,
    Mid,
    Len,
    Trim,
    Concat,
    Ifs,
    RoundUp,
    RoundDown,
}
impl FunctionId {
    pub fn resolve(name: &str) -> Result<Self, CellError> {
        match name.to_ascii_uppercase().as_str() {
            "SUM" => Ok(Self::Sum),
            "AVERAGE" => Ok(Self::Average),
            "MIN" => Ok(Self::Min),
            "MAX" => Ok(Self::Max),
            "COUNT" => Ok(Self::Count),
            "COUNTA" => Ok(Self::CountA),
            "IF" => Ok(Self::If),
            "AND" => Ok(Self::And),
            "OR" => Ok(Self::Or),
            "NOT" => Ok(Self::Not),
            "ROUND" => Ok(Self::Round),
            "IFERROR" => Ok(Self::IfError),
            "LEFT" => Ok(Self::Left),
            "RIGHT" => Ok(Self::Right),
            "MID" => Ok(Self::Mid),
            "LEN" => Ok(Self::Len),
            "TRIM" => Ok(Self::Trim),
            "CONCAT" => Ok(Self::Concat),
            "IFS" => Ok(Self::Ifs),
            "ROUNDUP" => Ok(Self::RoundUp),
            "ROUNDDOWN" => Ok(Self::RoundDown),
            _ => Err(CellError::NameError),
        }
    }
    pub const fn canonical_name(self) -> &'static str {
        match self {
            Self::Sum => "SUM",
            Self::Average => "AVERAGE",
            Self::Min => "MIN",
            Self::Max => "MAX",
            Self::Count => "COUNT",
            Self::CountA => "COUNTA",
            Self::If => "IF",
            Self::And => "AND",
            Self::Or => "OR",
            Self::Not => "NOT",
            Self::Round => "ROUND",
            Self::IfError => "IFERROR",
            Self::Left => "LEFT",
            Self::Right => "RIGHT",
            Self::Mid => "MID",
            Self::Len => "LEN",
            Self::Trim => "TRIM",
            Self::Concat => "CONCAT",
            Self::Ifs => "IFS",
            Self::RoundUp => "ROUNDUP",
            Self::RoundDown => "ROUNDDOWN",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    pub sheet: Option<String>,
    pub address: CellAddress,
    pub absolute_row: bool,
    pub absolute_column: bool,
}
impl Reference {
    pub(crate) fn parse(sheet: Option<String>, text: &str) -> Result<Self, CellError> {
        let bytes = text.as_bytes();
        let mut i = 0;
        let absolute_column = bytes.first() == Some(&b'$');
        if absolute_column {
            i += 1;
        }
        let start = i;
        let mut column = 0u32;
        while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
            column = column
                .checked_mul(26)
                .and_then(|n| n.checked_add((bytes[i].to_ascii_uppercase() - b'A' + 1) as u32))
                .ok_or(CellError::InvalidReference)?;
            i += 1;
        }
        if i == start {
            return Err(CellError::InvalidReference);
        }
        let absolute_row = bytes.get(i) == Some(&b'$');
        if absolute_row {
            i += 1;
        }
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == start || i != bytes.len() {
            return Err(CellError::InvalidReference);
        }
        let row = text[start..]
            .parse()
            .map_err(|_| CellError::InvalidReference)?;
        Ok(Self {
            sheet,
            address: CellAddress::new(row, column)?,
            absolute_row,
            absolute_column,
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnaryOp {
    Plus,
    Minus,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
}
/// Flat arena; child indices always refer to earlier nodes. Callers cannot forge an arena.
#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Literal(CellValue),
    Reference(Reference),
    Range(Reference, Reference),
    Unary(UnaryOp, usize),
    Binary(BinaryOp, usize, usize),
    Call(FunctionId, Vec<usize>),
}
#[derive(Clone, Debug, PartialEq)]
pub struct ParsedFormula {
    pub(crate) nodes: Vec<Expr>,
    pub(crate) root: usize,
}
impl ParsedFormula {
    pub fn nodes(&self) -> &[Expr] {
        &self.nodes
    }
    pub fn root(&self) -> usize {
        self.root
    }
}
#[derive(Clone, Debug, PartialEq)]
enum Token {
    Word(String),
    Quoted(String),
    Text(String),
    Number(f64),
    Symbol(char),
    Compare(BinaryOp),
    End,
}
fn lex(text: &str) -> Result<Vec<Token>, CellError> {
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if out.len() >= MAX_AST_NODES * 4 {
            return Err(CellError::LimitExceeded);
        }
        if c == '"' || c == '\'' {
            let quote = c;
            i += 1;
            let mut s = String::new();
            let mut closed = false;
            while i < chars.len() {
                let c = chars[i];
                i += 1;
                if c == quote {
                    if chars.get(i) == Some(&quote) {
                        s.push(c);
                        i += 1;
                    } else {
                        closed = true;
                        break;
                    }
                } else {
                    s.push(c);
                }
            }
            if !closed {
                return Err(CellError::InvalidSyntax);
            }
            out.push(if quote == '"' {
                Token::Text(s)
            } else {
                Token::Quoted(s)
            });
        } else if c.is_ascii_digit() || c == '.' {
            let start = i;
            i += 1;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            if matches!(chars.get(i), Some('e' | 'E')) {
                i += 1;
                if matches!(chars.get(i), Some('+' | '-')) {
                    i += 1;
                }
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
            }
            let n: f64 = chars[start..i]
                .iter()
                .collect::<String>()
                .parse()
                .map_err(|_| CellError::InvalidSyntax)?;
            if !n.is_finite() {
                return Err(CellError::NumericError);
            }
            out.push(Token::Number(n));
        } else if c.is_alphabetic() || c == '_' || c == '$' {
            let start = i;
            i += 1;
            while i < chars.len()
                && (chars[i].is_alphanumeric() || matches!(chars[i], '_' | '$' | '.'))
            {
                i += 1;
            }
            out.push(Token::Word(chars[start..i].iter().collect()));
        } else if matches!(c, '=' | '<' | '>') {
            i += 1;
            let op = match (c, chars.get(i)) {
                ('<', Some('=')) => {
                    i += 1;
                    BinaryOp::LessEqual
                }
                ('>', Some('=')) => {
                    i += 1;
                    BinaryOp::GreaterEqual
                }
                ('<', Some('>')) => {
                    i += 1;
                    BinaryOp::NotEqual
                }
                ('<', _) => BinaryOp::Less,
                ('>', _) => BinaryOp::Greater,
                _ => BinaryOp::Equal,
            };
            out.push(Token::Compare(op));
        } else if matches!(c, '+' | '-' | '*' | '/' | '(' | ')' | ',' | ':' | '!') {
            out.push(Token::Symbol(c));
            i += 1;
        } else {
            return Err(CellError::InvalidSyntax);
        }
    }
    out.push(Token::End);
    Ok(out)
}
struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    nodes: Vec<Expr>,
    depths: Vec<usize>,
}
impl Parser {
    fn peek(&self) -> &Token {
        &self.tokens[self.pos]
    }
    fn take(&mut self) -> Token {
        let t = self.tokens[self.pos].clone();
        if t != Token::End {
            self.pos += 1;
        }
        t
    }
    fn symbol(&mut self, c: char) -> bool {
        if self.peek() == &Token::Symbol(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn node(&mut self, expr: Expr) -> Result<usize, CellError> {
        let depth = match &expr {
            Expr::Unary(_, a) => self.depths[*a] + 1,
            Expr::Binary(_, a, b) => self.depths[*a].max(self.depths[*b]) + 1,
            Expr::Call(_, args) => args.iter().map(|i| self.depths[*i]).max().unwrap_or(0) + 1,
            _ => 1,
        };
        if depth > MAX_AST_DEPTH || self.nodes.len() >= MAX_AST_NODES {
            return Err(CellError::LimitExceeded);
        }
        let i = self.nodes.len();
        self.nodes.push(expr);
        self.depths.push(depth);
        Ok(i)
    }
    fn expr(&mut self, min: u8, depth: usize) -> Result<usize, CellError> {
        if depth >= MAX_AST_DEPTH {
            return Err(CellError::LimitExceeded);
        }
        let mut left = self.atom(depth + 1)?;
        loop {
            let (op, precedence) = match self.peek() {
                Token::Compare(op) => (*op, 1),
                Token::Symbol('+') => (BinaryOp::Add, 2),
                Token::Symbol('-') => (BinaryOp::Subtract, 2),
                Token::Symbol('*') => (BinaryOp::Multiply, 3),
                Token::Symbol('/') => (BinaryOp::Divide, 3),
                _ => break,
            };
            if precedence < min {
                break;
            }
            self.take();
            let right = self.expr(precedence + 1, depth + 1)?;
            left = self.node(Expr::Binary(op, left, right))?;
        }
        Ok(left)
    }
    fn atom(&mut self, depth: usize) -> Result<usize, CellError> {
        if depth >= MAX_AST_DEPTH {
            return Err(CellError::LimitExceeded);
        }
        match self.take() {
            Token::Number(n) => self.node(Expr::Literal(CellValue::Number(n))),
            Token::Text(s) => self.node(Expr::Literal(CellValue::Text(s))),
            Token::Symbol(c @ ('+' | '-')) => {
                let i = self.atom(depth + 1)?;
                self.node(Expr::Unary(
                    if c == '+' {
                        UnaryOp::Plus
                    } else {
                        UnaryOp::Minus
                    },
                    i,
                ))
            }
            Token::Symbol('(') => {
                let i = self.expr(1, depth + 1)?;
                if !self.symbol(')') {
                    return Err(CellError::InvalidSyntax);
                }
                Ok(i)
            }
            Token::Word(s) | Token::Quoted(s) => {
                if self.symbol('!') {
                    let Token::Word(address) = self.take() else {
                        return Err(CellError::InvalidReference);
                    };
                    let r = Reference::parse(Some(s), &address)?;
                    return self.reference(r);
                }
                if self.symbol('(') {
                    let f = FunctionId::resolve(&s)?;
                    let mut args = Vec::new();
                    if !self.symbol(')') {
                        loop {
                            args.push(self.expr(1, depth + 1)?);
                            if self.symbol(')') {
                                break;
                            }
                            if !self.symbol(',') {
                                return Err(CellError::InvalidSyntax);
                            }
                        }
                    }
                    return self.node(Expr::Call(f, args));
                }
                match s.to_ascii_uppercase().as_str() {
                    "TRUE" => self.node(Expr::Literal(CellValue::Boolean(true))),
                    "FALSE" => self.node(Expr::Literal(CellValue::Boolean(false))),
                    _ => {
                        let r = Reference::parse(None, &s)?;
                        self.reference(r)
                    }
                }
            }
            _ => Err(CellError::InvalidSyntax),
        }
    }
    fn reference(&mut self, start: Reference) -> Result<usize, CellError> {
        if !self.symbol(':') {
            return self.node(Expr::Reference(start));
        }
        let Token::Word(s) = self.take() else {
            return Err(CellError::InvalidReference);
        };
        let end = Reference::parse(start.sheet.clone(), &s)?;
        range_size(start.address, end.address)?;
        self.node(Expr::Range(start, end))
    }
}
pub(crate) fn range_size(start: CellAddress, end: CellAddress) -> Result<usize, CellError> {
    if start.row() > end.row() || start.column() > end.column() {
        return Err(CellError::InvalidReference);
    }
    let size = (end.row() - start.row() + 1) as u64 * (end.column() - start.column() + 1) as u64;
    if size > MAX_RANGE_CELLS as u64 {
        Err(CellError::LimitExceeded)
    } else {
        Ok(size as usize)
    }
}
pub fn parse_formula(text: &str) -> Result<ParsedFormula, CellError> {
    if text.len() > MAX_FORMULA_BYTES {
        return Err(CellError::LimitExceeded);
    }
    let Some(text) = text.strip_prefix('=') else {
        return Err(CellError::InvalidSyntax);
    };
    let mut p = Parser {
        tokens: lex(text)?,
        pos: 0,
        nodes: Vec::new(),
        depths: Vec::new(),
    };
    let root = p.expr(1, 0)?;
    if p.peek() != &Token::End {
        return Err(CellError::InvalidSyntax);
    }
    Ok(ParsedFormula {
        nodes: p.nodes,
        root,
    })
}
