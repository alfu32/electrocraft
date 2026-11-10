//! SI values and engineering notation.
//!
//! Parsing accepts the forms people type in a schematic inspector and in SPICE
//! netlists: `4k7`, `100n`, `2.2M`, `1meg`, `10uF`, `4.7 kΩ`, `1e-3`, `-5V`.
//! Case matters for `m` (milli) and `M` (mega), as in most schematic tools;
//! the SPICE spelling `meg` is also accepted for mega.

use thiserror::Error;

/// Error returned when a value cannot be parsed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("cannot read \"{text}\" as a value: {reason}")]
pub struct ParseError {
    /// The text that was rejected.
    pub text: String,
    /// Why it was rejected.
    pub reason: &'static str,
}

/// SI prefixes, largest first, as `(symbol, exponent)`.
const PREFIXES: [(&str, i32); 11] = [
    ("T", 12),
    ("G", 9),
    ("M", 6),
    ("k", 3),
    ("", 0),
    ("m", -3),
    ("µ", -6),
    ("n", -9),
    ("p", -12),
    ("f", -15),
    ("a", -18),
];

fn prefix_exponent(prefix: &str) -> Option<i32> {
    match prefix {
        "T" => Some(12),
        "G" => Some(9),
        "M" | "meg" | "Meg" | "MEG" => Some(6),
        "k" | "K" => Some(3),
        "m" => Some(-3),
        "u" | "µ" | "μ" => Some(-6),
        "n" | "N" => Some(-9),
        "p" | "P" => Some(-12),
        "f" | "F" => Some(-15),
        "a" => Some(-18),
        _ => None,
    }
}

/// Parse a number in engineering notation. A trailing unit (`V`, `Ω`, `ohm`,
/// `F`, `H`, `Hz`, `A`, `s`, `W`, `%`) is ignored, so `4.7kΩ` reads as 4700.
///
/// ```
/// assert_eq!(ecs_units::parse("4k7").unwrap(), 4700.0);
/// assert_eq!(ecs_units::parse("100n").unwrap(), 100e-9);
/// assert_eq!(ecs_units::parse("2.2M").unwrap(), 2.2e6);
/// ```
pub fn parse(text: &str) -> Result<f64, ParseError> {
    let fail = |reason| ParseError {
        text: text.to_string(),
        reason,
    };
    let s: String = text.trim().chars().filter(|c| !c.is_whitespace()).collect();
    if s.is_empty() {
        return Err(fail("empty"));
    }
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut mantissa = String::new();
    if chars[i] == '+' || chars[i] == '-' {
        mantissa.push(chars[i]);
        i += 1;
    }
    let digits_start = i;
    while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
        mantissa.push(chars[i]);
        i += 1;
    }
    if i == digits_start {
        return Err(fail("no digits"));
    }
    // Exponent: e/E followed by an optional sign and digits.
    if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
        let mut j = i + 1;
        if j < chars.len() && (chars[j] == '+' || chars[j] == '-') {
            j += 1;
        }
        if j < chars.len() && chars[j].is_ascii_digit() {
            mantissa.push('e');
            mantissa.extend(&chars[i + 1..j]);
            while j < chars.len() && chars[j].is_ascii_digit() {
                mantissa.push(chars[j]);
                j += 1;
            }
            i = j;
        }
    }
    let rest: String = chars[i..].iter().collect();
    let (exponent, after) = split_prefix(&rest);
    let mut digits = mantissa.clone();
    let mut tail = after;
    // "4k7" style: digits after the prefix are the fractional part.
    if exponent.is_some() && !mantissa.contains('.') && !mantissa.contains('e') {
        let frac: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !frac.is_empty() {
            digits = format!("{mantissa}.{frac}");
            tail = &tail[frac.len()..];
        }
    }
    if !is_unit(tail) {
        return Err(fail("unknown suffix"));
    }
    // Parse one decimal "<digits>e<exponent>" so the result is correctly
    // rounded (100n is exactly the double nearest 1e-7).
    let exp = exponent.unwrap_or(0);
    let value: f64 = if digits.contains('e') {
        let v: f64 = digits.parse().map_err(|_| fail("bad number"))?;
        v * 10f64.powi(exp)
    } else {
        format!("{digits}e{exp}")
            .parse()
            .map_err(|_| fail("bad number"))?
    };
    if !value.is_finite() {
        return Err(fail("not finite"));
    }
    Ok(value)
}

fn split_prefix(rest: &str) -> (Option<i32>, &str) {
    for candidate in ["meg", "Meg", "MEG"] {
        if let Some(after) = rest.strip_prefix(candidate) {
            return (Some(6), after);
        }
    }
    // A lone unit letter that is also a prefix (F for farad, a for ...) is a unit.
    if is_unit(rest) && !rest.is_empty() && prefix_exponent(rest).is_none() {
        return (None, rest);
    }
    if rest == "F" || rest == "A" || rest == "H" {
        return (None, rest);
    }
    // "2R2": R marks the decimal point of a plain ohm value.
    if let Some(after) = rest.strip_prefix('R')
        && after.starts_with(|c: char| c.is_ascii_digit())
    {
        return (Some(0), after);
    }
    let mut it = rest.char_indices();
    if let Some((_, c)) = it.next() {
        let end = c.len_utf8();
        let p = &rest[..end];
        if let Some(e) = prefix_exponent(p) {
            return (Some(e), &rest[end..]);
        }
    }
    (None, rest)
}

fn is_unit(tail: &str) -> bool {
    matches!(
        tail,
        "" | "V"
            | "v"
            | "A"
            | "Ω"
            | "ohm"
            | "Ohm"
            | "R"
            | "F"
            | "H"
            | "Hz"
            | "hz"
            | "s"
            | "W"
            | "%"
            | "°"
            | "deg"
            | "S"
    )
}

/// Format a value with an SI prefix and three significant digits: `4.70 kΩ`.
/// Trailing zeros are dropped (`4.7 kΩ`, `100 nF`).
pub fn format(value: f64, unit: &str) -> String {
    format_digits(value, unit, 3)
}

/// Like [`format`] with a chosen number of significant digits.
pub fn format_digits(value: f64, unit: &str, digits: usize) -> String {
    if !value.is_finite() {
        return format!("{value} {unit}").trim_end().to_string();
    }
    let sep = if unit.is_empty() { "" } else { " " };
    if value == 0.0 {
        return format!("0{sep}{unit}");
    }
    let magnitude = value.abs();
    let mut chosen = PREFIXES[PREFIXES.len() - 1];
    for p in PREFIXES {
        if magnitude >= 10f64.powi(p.1) * 0.9995 {
            chosen = p;
            break;
        }
    }
    let scaled = value / 10f64.powi(chosen.1);
    let int_digits = (scaled.abs().log10().floor() as i32 + 1).max(1) as usize;
    let decimals = digits.saturating_sub(int_digits);
    let mut text = format!("{scaled:.decimals$}");
    if text.contains('.') {
        text = text.trim_end_matches('0').trim_end_matches('.').to_string();
    }
    let space = if chosen.0.is_empty() && unit.is_empty() {
        ""
    } else {
        " "
    };
    let _ = sep;
    format!("{text}{space}{}{unit}", chosen.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-12 * b.abs().max(1e-30)
    }

    #[test]
    fn parses_engineering_forms() {
        for (text, value) in [
            ("4k7", 4700.0),
            ("100n", 100e-9),
            ("2.2M", 2.2e6),
            ("1meg", 1e6),
            ("10uF", 10e-6),
            ("10µF", 10e-6),
            ("4.7 kΩ", 4700.0),
            ("1e-3", 1e-3),
            ("-5V", -5.0),
            ("3m", 3e-3),
            ("1F", 1.0),
            ("47p", 47e-12),
            ("1.5", 1.5),
            ("2R2", 2.2),
            ("60Hz", 60.0),
        ] {
            let parsed = parse(text).unwrap_or_else(|e| panic!("{e}"));
            assert!(close(parsed, value), "{text}: {parsed} != {value}");
        }
    }

    #[test]
    fn rejects_garbage() {
        for text in ["", "abc", "4x", "--1", "k"] {
            assert!(parse(text).is_err(), "{text} should fail");
        }
    }

    #[test]
    fn formats_with_prefixes() {
        assert_eq!(format(4700.0, "Ω"), "4.7 kΩ");
        assert_eq!(format(100e-9, "F"), "100 nF");
        assert_eq!(format(0.0, "V"), "0 V");
        assert_eq!(format(-3.3, "V"), "-3.3 V");
        assert_eq!(format(1e-6, "A"), "1 µA");
        assert_eq!(format(1234.0, ""), "1.23 k");
        assert_eq!(format(5.0, ""), "5");
    }

    #[test]
    fn format_then_parse_round_trips() {
        for v in [1.0, 4.7e3, 2.2e-6, 33e-12, 1.5e6, 0.12] {
            let text = format(v, "");
            assert!(close(parse(&text).unwrap(), v), "{text}");
        }
    }
}
