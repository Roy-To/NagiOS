use crate::CellError;

struct Decimal {
    digits: Vec<u8>,
    position: i64,
}

/// Convert the public numeric digits argument into the bounded decimal-place
/// range supported by this formula kernel.
pub(crate) fn decimal_places(value: f64) -> Result<i32, CellError> {
    if !value.is_finite() || value.fract() != 0.0 || !(-308.0..=308.0).contains(&value) {
        return Err(CellError::NumericError);
    }
    Ok(value as i32)
}

/// Round from the shortest decimal spelling that round-trips to `value`.
/// `away_from_zero` selects ROUNDUP; false selects ROUNDDOWN.
pub(crate) fn apply(value: f64, places: i32, away_from_zero: bool) -> Result<f64, CellError> {
    if !value.is_finite() {
        return Err(CellError::NumericError);
    }
    if value == 0.0 {
        return Ok(0.0);
    }

    let negative = value.is_sign_negative();
    let mut decimal = Decimal::from_shortest(value.abs())?;
    let retained_count = decimal
        .position
        .checked_add(i64::from(places))
        .ok_or(CellError::NumericError)?;

    if retained_count >= decimal.digits.len() as i64 {
        return Ok(value);
    }
    if retained_count <= 0 {
        if !away_from_zero {
            return Ok(0.0);
        }
        return materialize(negative, vec![b'1'], 1 - i64::from(places));
    }

    let retained_count = retained_count as usize;
    let discarded_nonzero = decimal.digits[retained_count..]
        .iter()
        .any(|digit| *digit != b'0');
    let mut retained = decimal.digits[..retained_count].to_vec();
    if !discarded_nonzero {
        return Ok(value);
    }
    if !away_from_zero {
        return materialize(negative, retained, decimal.position);
    }

    if increment(&mut retained) {
        decimal.position = decimal
            .position
            .checked_add(1)
            .ok_or(CellError::NumericError)?;
        retained.clear();
        retained.push(b'1');
    }
    materialize(negative, retained, decimal.position)
}

impl Decimal {
    fn from_shortest(value: f64) -> Result<Self, CellError> {
        let spelling = value.to_string();
        let (mantissa, exponent) = match spelling.find(['e', 'E']) {
            Some(index) => {
                let exponent = spelling[index + 1..]
                    .parse::<i64>()
                    .map_err(|_| CellError::NumericError)?;
                (&spelling[..index], exponent)
            }
            None => (spelling.as_str(), 0),
        };
        let decimal_index = mantissa.find('.').unwrap_or(mantissa.len());
        let mut digits: Vec<u8> = mantissa.bytes().filter(|byte| *byte != b'.').collect();
        if digits.is_empty() || digits.iter().any(|digit| !digit.is_ascii_digit()) {
            return Err(CellError::NumericError);
        }

        let mut position = (decimal_index as i64)
            .checked_add(exponent)
            .ok_or(CellError::NumericError)?;
        let first_nonzero = digits
            .iter()
            .position(|digit| *digit != b'0')
            .ok_or(CellError::NumericError)?;
        digits.drain(..first_nonzero);
        position = position
            .checked_sub(first_nonzero as i64)
            .ok_or(CellError::NumericError)?;

        while digits.len() > 1 && digits.last() == Some(&b'0') {
            digits.pop();
        }

        Ok(Self { digits, position })
    }
}

/// Increase the retained base-10 coefficient; true means its width grew.
fn increment(digits: &mut Vec<u8>) -> bool {
    for digit in digits.iter_mut().rev() {
        if *digit == b'9' {
            *digit = b'0';
        } else {
            *digit += 1;
            return false;
        }
    }
    digits.insert(0, b'1');
    true
}

fn materialize(negative: bool, digits: Vec<u8>, position: i64) -> Result<f64, CellError> {
    let width = i64::try_from(digits.len()).map_err(|_| CellError::NumericError)?;
    let exponent = position.checked_sub(width).ok_or(CellError::NumericError)?;
    let coefficient = std::str::from_utf8(&digits).map_err(|_| CellError::NumericError)?;
    let spelling = format!("{coefficient}e{exponent}");
    let magnitude = spelling
        .parse::<f64>()
        .map_err(|_| CellError::NumericError)?;
    if !magnitude.is_finite() {
        return Err(CellError::NumericError);
    }
    if magnitude == 0.0 {
        return Ok(0.0);
    }
    Ok(if negative { -magnitude } else { magnitude })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_spelling_tracks_finite_extremes_and_subnormals() {
        let max = Decimal::from_shortest(f64::MAX).unwrap();
        assert_eq!(max.position, 309);
        assert_eq!(max.digits[0], b'1');

        let subnormal = Decimal::from_shortest(f64::from_bits(1)).unwrap();
        assert_eq!(subnormal.position, -323);
        assert_eq!(subnormal.digits, b"5");
    }

    #[test]
    fn place_validation_is_integral_and_bounded() {
        assert_eq!(decimal_places(-308.0), Ok(-308));
        assert_eq!(decimal_places(308.0), Ok(308));
        assert_eq!(decimal_places(1.5), Err(CellError::NumericError));
        assert_eq!(decimal_places(309.0), Err(CellError::NumericError));
        assert_eq!(decimal_places(f64::INFINITY), Err(CellError::NumericError));
    }
}
