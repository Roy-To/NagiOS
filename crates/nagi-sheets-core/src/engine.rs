use crate::*;
use std::cmp::Ordering;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub(crate) struct Binding {
    pub sheet: SheetId,
    pub start: CellAddress,
    pub end: CellAddress,
}
#[derive(Clone, Debug)]
pub(crate) struct Formula {
    pub ast: ParsedFormula,
    pub bindings: BTreeMap<usize, Binding>,
}
enum Value {
    Scalar(CellValue),
    Range(Vec<CellValue>),
}
impl Value {
    fn scalar(self) -> Result<CellValue, CellError> {
        match self {
            Self::Scalar(CellValue::Error(e)) => Err(e),
            Self::Scalar(v) => Ok(v),
            Self::Range(_) => Err(CellError::ValueError),
        }
    }
    fn values(self) -> Vec<CellValue> {
        match self {
            Self::Scalar(v) => vec![v],
            Self::Range(v) => v,
        }
    }
}
fn number(v: CellValue) -> Result<f64, CellError> {
    match v {
        CellValue::Number(n) => Ok(n),
        CellValue::Empty => Ok(0.0),
        CellValue::Boolean(b) => Ok(if b { 1.0 } else { 0.0 }),
        CellValue::Error(e) => Err(e),
        _ => Err(CellError::ValueError),
    }
}
fn boolean(v: CellValue) -> Result<bool, CellError> {
    match v {
        CellValue::Boolean(b) => Ok(b),
        CellValue::Empty => Ok(false),
        CellValue::Number(n) => Ok(n != 0.0),
        CellValue::Error(e) => Err(e),
        _ => Err(CellError::ValueError),
    }
}
fn finite(n: f64) -> Result<CellValue, CellError> {
    if n.is_finite() {
        Ok(CellValue::Number(n))
    } else {
        Err(CellError::NumericError)
    }
}
fn text(v: CellValue) -> Result<String, CellError> {
    match v {
        CellValue::Text(s) => Ok(s),
        CellValue::Empty => Ok(String::new()),
        CellValue::Number(n) => Ok(n.to_string()),
        CellValue::Boolean(b) => Ok(if b { "TRUE" } else { "FALSE" }.into()),
        CellValue::Error(e) => Err(e),
        _ => Err(CellError::ValueError),
    }
}
fn count(v: CellValue) -> Result<usize, CellError> {
    let n = number(v)?;
    if n < 0.0 || n.fract() != 0.0 {
        Err(CellError::ValueError)
    } else if n > MAX_TEXT_BYTES as f64 {
        Err(CellError::LimitExceeded)
    } else {
        Ok(n as usize)
    }
}
/// ROUNDUP (away from zero) / ROUNDDOWN (toward zero) at `digits` decimal
/// places. Rounding is performed on the shortest round-trip decimal
/// representation of `n`, so binary artefacts such as `1.1 * 100 =
/// 110.00000000000001` never push ROUNDUP(1.1, 2) to 1.11. The result is
/// parsed back with correct rounding; overflow yields NUMERIC_ERROR and
/// -0 is normalised to 0.
fn round_directed(n: f64, digits: i32, away: bool) -> Result<CellValue, CellError> {
    if !n.is_finite() {
        return Err(CellError::NumericError);
    }
    if n == 0.0 {
        return Ok(CellValue::Number(0.0));
    }
    let repr = format!("{:e}", n.abs());
    let (mantissa, exponent) = repr.split_once('e').ok_or(CellError::NumericError)?;
    let exponent: i64 = exponent.parse().map_err(|_| CellError::NumericError)?;
    let mut digit_bytes: Vec<u8> = mantissa.bytes().filter(u8::is_ascii_digit).collect();
    // Value = 0.d1d2..dk * 10^(exponent + 1); keep digits whose place value is
    // at least 10^-digits.
    let keep = exponent + i64::from(digits) + 1;
    if keep >= digit_bytes.len() as i64 {
        return finite(n);
    }
    let negative = n < 0.0;
    let magnitude = if keep <= 0 {
        if !away {
            return Ok(CellValue::Number(0.0));
        }
        // Every kept digit is zero and a nonzero digit was discarded.
        format!("1e{}", -i64::from(digits))
    } else {
        let discarded_nonzero = digit_bytes[keep as usize..].iter().any(|&d| d != b'0');
        digit_bytes.truncate(keep as usize);
        if away && discarded_nonzero {
            let mut carry = true;
            for d in digit_bytes.iter_mut().rev() {
                if *d == b'9' {
                    *d = b'0';
                } else {
                    *d += 1;
                    carry = false;
                    break;
                }
            }
            if carry {
                // 99..9 rounded away from zero becomes 100..0 at the same scale.
                digit_bytes.insert(0, b'1');
            }
        }
        let scale = exponent - keep + 1;
        let text = String::from_utf8(digit_bytes).map_err(|_| CellError::NumericError)?;
        format!("{text}e{scale}")
    };
    signed(magnitude, negative)
}
fn signed(magnitude: String, negative: bool) -> Result<CellValue, CellError> {
    let m: f64 = magnitude.parse().map_err(|_| CellError::NumericError)?;
    if m == 0.0 {
        return Ok(CellValue::Number(0.0));
    }
    finite(if negative { -m } else { m })
}
impl Formula {
    pub fn evaluate(&self, book: &Workbook) -> CellValue {
        match self.eval(self.ast.root, book).and_then(Value::scalar) {
            Ok(v) => v,
            Err(e) => CellValue::Error(e),
        }
    }
    fn scalar(&self, i: usize, book: &Workbook) -> Result<CellValue, CellError> {
        self.eval(i, book)?.scalar()
    }
    fn eval(&self, i: usize, book: &Workbook) -> Result<Value, CellError> {
        let v = match &self.ast.nodes[i] {
            Expr::Literal(v) => v.clone(),
            Expr::Reference(_) | Expr::Range(_, _) => {
                let b = &self.bindings[&i];
                if book.sheet(b.sheet).is_none() {
                    return Err(CellError::InvalidReference);
                }
                if matches!(&self.ast.nodes[i], Expr::Reference(_)) {
                    book.get_cell(CellKey::new(b.sheet, b.start))?.clone()
                } else {
                    let mut values = Vec::new();
                    for row in b.start.row()..=b.end.row() {
                        for column in b.start.column()..=b.end.column() {
                            values.push(
                                book.get_cell(CellKey::new(
                                    b.sheet,
                                    CellAddress::new(row, column)?,
                                ))?
                                .clone(),
                            );
                        }
                    }
                    return Ok(Value::Range(values));
                }
            }
            Expr::Unary(op, a) => {
                let n = number(self.scalar(*a, book)?)?;
                finite(if *op == UnaryOp::Minus { -n } else { n })?
            }
            Expr::Binary(op, a, b) => {
                let a = self.scalar(*a, book)?;
                let b = self.scalar(*b, book)?;
                match op {
                    BinaryOp::Add | BinaryOp::Subtract | BinaryOp::Multiply | BinaryOp::Divide => {
                        let a = number(a)?;
                        let b = number(b)?;
                        finite(match op {
                            BinaryOp::Add => a + b,
                            BinaryOp::Subtract => a - b,
                            BinaryOp::Multiply => a * b,
                            _ => {
                                if b == 0.0 {
                                    return Err(CellError::DivByZero);
                                }
                                a / b
                            }
                        })?
                    }
                    _ => {
                        let ordering = match (a, b) {
                            (CellValue::Text(a), CellValue::Text(b)) => a.cmp(&b),
                            (a, b) => number(a)?
                                .partial_cmp(&number(b)?)
                                .ok_or(CellError::NumericError)?,
                        };
                        CellValue::Boolean(match op {
                            BinaryOp::Equal => ordering == Ordering::Equal,
                            BinaryOp::NotEqual => ordering != Ordering::Equal,
                            BinaryOp::Less => ordering == Ordering::Less,
                            BinaryOp::LessEqual => ordering != Ordering::Greater,
                            BinaryOp::Greater => ordering == Ordering::Greater,
                            _ => ordering != Ordering::Less,
                        })
                    }
                }
            }
            Expr::Call(f, args) => return self.call(*f, args, book).map(Value::Scalar),
        };
        Ok(Value::Scalar(v))
    }
    fn call(&self, f: FunctionId, args: &[usize], book: &Workbook) -> Result<CellValue, CellError> {
        use FunctionId::*;
        let valid = match f {
            If => args.len() == 3,
            IfError | Round | RoundUp | RoundDown | Left | Right => args.len() == 2,
            Ifs => args.len() >= 2 && args.len().is_multiple_of(2),
            Mid => args.len() == 3,
            Not | Len | Trim => args.len() == 1,
            _ => !args.is_empty(),
        };
        if !valid {
            return Err(CellError::ValueError);
        }
        match f {
            If => {
                let cond = boolean(self.scalar(args[0], book)?)?;
                self.scalar(args[if cond { 1 } else { 2 }], book)
            }
            Ifs => {
                // Conditions are tested left to right; only the first true
                // condition's value is evaluated. Later pairs stay unevaluated.
                for pair in args.chunks_exact(2) {
                    if boolean(self.scalar(pair[0], book)?)? {
                        return self.scalar(pair[1], book);
                    }
                }
                Err(CellError::NotAvailable)
            }
            IfError => match self.scalar(args[0], book) {
                Ok(v) => Ok(v),
                Err(_) => self.scalar(args[1], book),
            },
            Not => Ok(CellValue::Boolean(!boolean(self.scalar(args[0], book)?)?)),
            And | Or => {
                for &arg in args {
                    for value in self.eval(arg, book)?.values() {
                        let b = boolean(value)?;
                        if (f == And && !b) || (f == Or && b) {
                            return Ok(CellValue::Boolean(b));
                        }
                    }
                }
                Ok(CellValue::Boolean(f == And))
            }
            Round => {
                let n = number(self.scalar(args[0], book)?)?;
                let digits = number(self.scalar(args[1], book)?)?;
                if digits.fract() != 0.0 || !(-308.0..=308.0).contains(&digits) {
                    return Err(CellError::NumericError);
                }
                let result = if digits >= 0.0 {
                    let factor = 10f64.powi(digits as i32);
                    let scaled = n * factor;
                    if !scaled.is_finite() {
                        return Err(CellError::NumericError);
                    }
                    scaled.round() / factor
                } else {
                    let factor = 10f64.powi(-digits as i32);
                    (n / factor).round() * factor
                };
                finite(result)
            }
            RoundUp | RoundDown => {
                let n = number(self.scalar(args[0], book)?)?;
                let digits = number(self.scalar(args[1], book)?)?;
                if digits.fract() != 0.0 || !(-308.0..=308.0).contains(&digits) {
                    return Err(CellError::NumericError);
                }
                round_directed(n, digits as i32, f == RoundUp)
            }
            Left | Right | Mid | Len | Trim => {
                let s = text(self.scalar(args[0], book)?)?;
                if f == Len {
                    return finite(s.chars().count() as f64);
                }
                if f == Trim {
                    return Ok(CellValue::Text(
                        s.split_whitespace().collect::<Vec<_>>().join(" "),
                    ));
                }
                let chars: Vec<char> = s.chars().collect();
                let n = count(self.scalar(args[1], book)?)?;
                let (start, len) = match f {
                    Left => (0, n),
                    Right => (chars.len().saturating_sub(n), n),
                    _ => {
                        if n == 0 {
                            return Err(CellError::ValueError);
                        }
                        (n - 1, count(self.scalar(args[2], book)?)?)
                    }
                };
                Ok(CellValue::Text(
                    chars.iter().skip(start).take(len).collect(),
                ))
            }
            Concat => {
                let mut s = String::new();
                for &arg in args {
                    for v in self.eval(arg, book)?.values() {
                        let part = text(v)?;
                        if s.len() + part.len() > MAX_TEXT_BYTES {
                            return Err(CellError::LimitExceeded);
                        }
                        s.push_str(&part);
                    }
                }
                Ok(CellValue::Text(s))
            }
            _ => {
                let mut numbers = 0usize;
                let mut occupied = 0usize;
                let mut sum = 0.0;
                let mut min = f64::INFINITY;
                let mut max = f64::NEG_INFINITY;
                for &arg in args {
                    for v in self.eval(arg, book)?.values() {
                        if !matches!(v, CellValue::Empty) {
                            occupied += 1;
                        }
                        match v {
                            CellValue::Error(e) if !matches!(f, Count | CountA) => return Err(e),
                            CellValue::Number(n) => {
                                numbers += 1;
                                sum += n;
                                min = min.min(n);
                                max = max.max(n);
                            }
                            _ => {}
                        }
                    }
                }
                match f {
                    Count => finite(numbers as f64),
                    CountA => finite(occupied as f64),
                    Average => {
                        if numbers == 0 {
                            Err(CellError::DivByZero)
                        } else {
                            finite(sum / numbers as f64)
                        }
                    }
                    Min => finite(if numbers == 0 { 0.0 } else { min }),
                    Max => finite(if numbers == 0 { 0.0 } else { max }),
                    _ => finite(sum),
                }
            }
        }
    }
}
