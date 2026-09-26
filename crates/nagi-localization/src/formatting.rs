use core::cmp::Ordering;
use core::fmt;

use crate::locale::LocaleId;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Date {
    year: i32,
    month: u8,
    day: u8,
}

impl Date {
    pub fn new(year: i32, month: u8, day: u8) -> Result<Self, FormatError> {
        if year < 1 || !(1..=12).contains(&month) {
            return Err(FormatError::InvalidDate);
        }
        let days_in_month = match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if is_leap_year(year) => 29,
            2 => 28,
            _ => unreachable!("month is validated"),
        };
        if day == 0 || day > days_in_month {
            return Err(FormatError::InvalidDate);
        }
        Ok(Self { year, month, day })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Time {
    hour: u8,
    minute: u8,
}

impl Time {
    pub fn new(hour: u8, minute: u8) -> Result<Self, FormatError> {
        if hour > 23 || minute > 59 {
            return Err(FormatError::InvalidTime);
        }
        Ok(Self { hour, minute })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DateTime {
    pub date: Date,
    pub time: Time,
}

impl DateTime {
    pub fn new(date: Date, time: Time) -> Self {
        Self { date, time }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NumberOptions {
    pub minimum_fraction_digits: u8,
    pub maximum_fraction_digits: u8,
}

impl NumberOptions {
    pub const INTEGER: Self = Self {
        minimum_fraction_digits: 0,
        maximum_fraction_digits: 0,
    };

    pub const DECIMAL: Self = Self {
        minimum_fraction_digits: 0,
        maximum_fraction_digits: 2,
    };

    pub fn new(
        minimum_fraction_digits: u8,
        maximum_fraction_digits: u8,
    ) -> Result<Self, FormatError> {
        if minimum_fraction_digits > maximum_fraction_digits || maximum_fraction_digits > 6 {
            return Err(FormatError::InvalidNumberOptions);
        }
        Ok(Self {
            minimum_fraction_digits,
            maximum_fraction_digits,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Unit {
    Byte,
    Meter,
    Kilometer,
    Kilogram,
    Celsius,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Currency {
    Usd,
    Jpy,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FormatError {
    InvalidDate,
    InvalidTime,
    InvalidNumberOptions,
    NonFiniteNumber,
}

impl fmt::Display for FormatError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let description = match self {
            Self::InvalidDate => "date is outside the Gregorian calendar range",
            Self::InvalidTime => "time is outside the 24-hour clock range",
            Self::InvalidNumberOptions => "fraction digit limits must satisfy 0 <= min <= max <= 6",
            Self::NonFiniteNumber => "number must be finite",
        };
        formatter.write_str(description)
    }
}

impl std::error::Error for FormatError {}

/// Nagi-owned deterministic formatting for the initial `en-US` and `ja-JP`
/// region profiles. It never reads the host process locale or timezone.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Formatter {
    region: LocaleId,
}

impl Formatter {
    pub fn new(region: LocaleId) -> Self {
        Self { region }
    }

    pub fn region(&self) -> &LocaleId {
        &self.region
    }

    pub fn format_date(&self, date: Date) -> String {
        if self.region.language() == "ja" {
            format!("{}年{}月{}日", date.year, date.month, date.day)
        } else {
            format!("{}/{}/{}", date.month, date.day, date.year)
        }
    }

    pub fn format_time(&self, time: Time) -> String {
        if self.region.language() == "ja" {
            format!("{:02}:{:02}", time.hour, time.minute)
        } else {
            let suffix = if time.hour < 12 { "AM" } else { "PM" };
            let hour = match time.hour % 12 {
                0 => 12,
                hour => hour,
            };
            format!("{hour}:{:02} {suffix}", time.minute)
        }
    }

    pub fn format_date_time(&self, value: DateTime) -> String {
        if self.region.language() == "ja" {
            format!(
                "{} {}",
                self.format_date(value.date),
                self.format_time(value.time)
            )
        } else {
            format!(
                "{}, {}",
                self.format_date(value.date),
                self.format_time(value.time)
            )
        }
    }

    pub fn format_integer(&self, value: i64) -> String {
        let magnitude = value.unsigned_abs().to_string();
        let mut result = String::new();
        if value.is_negative() {
            result.push('-');
        }
        result.push_str(&group_integer(&magnitude));
        result
    }

    pub fn format_decimal(
        &self,
        value: f64,
        options: NumberOptions,
    ) -> Result<String, FormatError> {
        if !value.is_finite() {
            return Err(FormatError::NonFiniteNumber);
        }
        if options.minimum_fraction_digits > options.maximum_fraction_digits
            || options.maximum_fraction_digits > 6
        {
            return Err(FormatError::InvalidNumberOptions);
        }

        let precision = usize::from(options.maximum_fraction_digits);
        let mut rendered = format!("{:.precision$}", value.abs());
        if let Some(decimal_point) = rendered.find('.') {
            while rendered.len() - decimal_point - 1 > usize::from(options.minimum_fraction_digits)
                && rendered.ends_with('0')
            {
                rendered.pop();
            }
            if rendered.ends_with('.') {
                rendered.pop();
            }
        }

        let (integer, fraction) = rendered
            .split_once('.')
            .map_or((rendered.as_str(), None), |(integer, fraction)| {
                (integer, Some(fraction))
            });
        let mut result = String::new();
        if value.is_sign_negative() {
            result.push('-');
        }
        result.push_str(&group_integer(integer));
        if let Some(fraction) = fraction {
            result.push('.');
            result.push_str(fraction);
        } else if options.minimum_fraction_digits > 0 {
            result.push('.');
            result.push_str(&"0".repeat(usize::from(options.minimum_fraction_digits)));
        }
        Ok(result)
    }

    /// `value` is a ratio: `0.125` displays as `12.5%`.
    pub fn format_percent(
        &self,
        value: f64,
        options: NumberOptions,
    ) -> Result<String, FormatError> {
        let percentage = value * 100.0;
        let mut result = self.format_decimal(percentage, options)?;
        result.push('%');
        Ok(result)
    }

    /// Format a currency amount without performing currency conversion.
    /// The initial profiles cover USD and JPY only.
    pub fn format_currency(&self, amount: f64, currency: Currency) -> Result<String, FormatError> {
        let (symbol, fraction_digits) = match (self.region.language(), currency) {
            ("ja", Currency::Usd) => ("US$", 2),
            ("ja", Currency::Jpy) => ("￥", 0),
            (_, Currency::Usd) => ("$", 2),
            (_, Currency::Jpy) => ("¥", 0),
        };
        let options = NumberOptions {
            minimum_fraction_digits: fraction_digits,
            maximum_fraction_digits: fraction_digits,
        };
        Ok(format!("{symbol}{}", self.format_decimal(amount, options)?))
    }

    pub fn format_measurement(
        &self,
        value: f64,
        unit: Unit,
        options: NumberOptions,
    ) -> Result<String, FormatError> {
        let number = self.format_decimal(value, options)?;
        let symbol = match unit {
            Unit::Byte => "B",
            Unit::Meter => "m",
            Unit::Kilometer => "km",
            Unit::Kilogram => "kg",
            Unit::Celsius => "°C",
        };
        Ok(format!("{number} {symbol}"))
    }
}

/// Extension point for locale-specific collation data. Nagi 0.1 does not ship
/// a dictionary-based Japanese collator; callers must not treat byte order as
/// linguistic sorting.
pub trait Collator {
    fn locale(&self) -> &LocaleId;
    fn compare(&self, left: &str, right: &str) -> Ordering;
}

fn group_integer(integer: &str) -> String {
    let mut grouped = String::with_capacity(integer.len() + integer.len() / 3);
    for (index, digit) in integer.chars().enumerate() {
        if index > 0 && (integer.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

fn is_leap_year(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

#[cfg(test)]
mod tests {
    use super::{Currency, Date, DateTime, Formatter, NumberOptions, Time, Unit};
    use crate::locale::LocaleId;

    fn formatter(region: &str) -> Formatter {
        Formatter::new(LocaleId::parse(region).unwrap())
    }

    #[test]
    fn formats_dates_times_and_date_times_for_english_and_japanese() {
        let date = Date::new(2026, 3, 5).unwrap();
        let time = Time::new(13, 4).unwrap();
        assert_eq!(formatter("en-US").format_date(date), "3/5/2026");
        assert_eq!(formatter("ja-JP").format_date(date), "2026年3月5日");
        assert_eq!(formatter("en-US").format_time(time), "1:04 PM");
        assert_eq!(formatter("ja-JP").format_time(time), "13:04");
        assert_eq!(
            formatter("ja-JP").format_date_time(DateTime::new(date, time)),
            "2026年3月5日 13:04"
        );
    }

    #[test]
    fn formats_grouped_numbers_percent_and_units_deterministically() {
        let english = formatter("en-US");
        let japanese = formatter("ja-JP");
        assert_eq!(english.format_integer(1234567), "1,234,567");
        assert_eq!(
            english
                .format_decimal(1234.5, NumberOptions::DECIMAL)
                .unwrap(),
            "1,234.5"
        );
        assert_eq!(
            japanese
                .format_decimal(1234.5, NumberOptions::DECIMAL)
                .unwrap(),
            "1,234.5"
        );
        assert_eq!(
            english
                .format_percent(0.125, NumberOptions::DECIMAL)
                .unwrap(),
            "12.5%"
        );
        assert_eq!(
            english.format_currency(1234.5, Currency::Usd).unwrap(),
            "$1,234.50"
        );
        assert_eq!(
            japanese.format_currency(1234.6, Currency::Jpy).unwrap(),
            "￥1,235"
        );
        assert_eq!(
            japanese
                .format_measurement(20.0, Unit::Celsius, NumberOptions::DECIMAL)
                .unwrap(),
            "20 °C"
        );
    }

    #[test]
    fn validates_calendar_clock_and_numeric_inputs() {
        assert!(Date::new(2025, 2, 29).is_err());
        assert!(Date::new(2024, 2, 29).is_ok());
        assert!(Time::new(24, 0).is_err());
        assert!(NumberOptions::new(3, 2).is_err());
        assert!(formatter("en-US")
            .format_decimal(f64::NAN, NumberOptions::DECIMAL)
            .is_err());
    }

    #[test]
    fn covers_gregorian_century_and_clock_boundaries() {
        assert!(Date::new(1900, 2, 29).is_err());
        assert!(Date::new(2000, 2, 29).is_ok());

        let english = formatter("en-US");
        let japanese = formatter("ja-JP");
        assert_eq!(english.format_time(Time::new(0, 0).unwrap()), "12:00 AM");
        assert_eq!(english.format_time(Time::new(12, 0).unwrap()), "12:00 PM");
        assert_eq!(english.format_time(Time::new(23, 59).unwrap()), "11:59 PM");
        assert_eq!(japanese.format_time(Time::new(0, 0).unwrap()), "00:00");
        assert_eq!(japanese.format_time(Time::new(23, 59).unwrap()), "23:59");
    }

    #[test]
    fn handles_integer_extremes_and_invalid_public_number_options() {
        let english = formatter("en-US");
        assert_eq!(
            english.format_integer(i64::MIN),
            "-9,223,372,036,854,775,808"
        );
        assert_eq!(
            english
                .format_decimal(
                    1.2,
                    NumberOptions {
                        minimum_fraction_digits: 0,
                        maximum_fraction_digits: 7,
                    },
                )
                .unwrap_err(),
            super::FormatError::InvalidNumberOptions
        );
        assert_eq!(
            english
                .format_percent(f64::MAX, NumberOptions::DECIMAL)
                .unwrap_err(),
            super::FormatError::NonFiniteNumber
        );
    }

    #[test]
    fn uses_region_independently_of_system_language_without_process_locale() {
        let english_ui_japanese_region = formatter("ja-JP");
        assert_eq!(
            english_ui_japanese_region.format_date(Date::new(2026, 9, 26).unwrap()),
            "2026年9月26日"
        );
    }
}
