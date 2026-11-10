//! Parameter normalization: provider attribute names → canonical keys
//! (`resistance`, `tolerance`, `power_rating`, `capacitance`,
//! `voltage_rating`, `dielectric`, `inductance`, `current_rating`,
//! `dc_resistance`, `esr`, `zener_voltage`, `package`, `color`) with SI values
//! parsed through `ecs-units`. Attributes that do not map are kept as raw text
//! under their own name.

use crate::{ParamValue, VendorPart};

/// Canonical keys and what they hold.
pub const KEYS: [(&str, &str); 13] = [
    ("resistance", "Ω"),
    ("tolerance", "fraction"),
    ("power_rating", "W"),
    ("capacitance", "F"),
    ("voltage_rating", "V"),
    ("dielectric", "text"),
    ("inductance", "H"),
    ("current_rating", "A"),
    ("dc_resistance", "Ω"),
    ("esr", "Ω"),
    ("zener_voltage", "V"),
    ("package", "text"),
    ("color", "text"),
];

/// Canonical key for an attribute label, if any.
pub fn canonical(label: &str) -> Option<&'static str> {
    let l = label.trim().to_lowercase();
    let has = |s: &str| l.contains(s);
    Some(if has("zener") && has("voltage") {
        "zener_voltage"
    } else if has("dc resistance") || l == "dcr" || has("dcr ") || has("(dcr)") {
        "dc_resistance"
    } else if has("esr") || has("equivalent series resistance") {
        "esr"
    } else if has("resistance")
        && !has("insulation")
        && !has("thermal")
        && !has("on-state")
        && !has("rds")
        && !has("drain-source")
    {
        "resistance"
    } else if has("tolerance") {
        "tolerance"
    } else if has("power")
        && (has("rating")
            || has("max")
            || has("watt")
            || l == "power"
            || has("(watts)")
            || has("power(w"))
    {
        "power_rating"
    } else if has("capacitance") {
        "capacitance"
    } else if has("inductance") {
        "inductance"
    } else if has("dielectric") {
        "dielectric"
    } else if has("voltage") && (has("rated") || has("rating") || has("dc") || has("working")) {
        "voltage_rating"
    } else if has("current") && (has("rated") || has("rating")) {
        "current_rating"
    } else if has("package") || has("case") || l == "footprint" {
        "package"
    } else if has("color") || has("colour") {
        "color"
    } else {
        return None;
    })
}

fn text_key(key: &str) -> bool {
    matches!(key, "dielectric" | "package" | "color")
}

/// Parse a value text to SI: "10 kOhms" → 1e4, "±1%" → 0.01, "1/10W" → 0.1.
pub fn parse_si(text: &str, key: &str) -> Option<f64> {
    let t = text.trim().replace(['±', '+'], "");
    let lower = t.to_lowercase();
    // Fractions of a watt: "1/10W".
    if key == "power_rating"
        && let Some((a, b)) = t.split_once('/')
        && let (Ok(n), Some(d)) = (
            a.trim().parse::<f64>(),
            b.trim()
                .trim_end_matches(['W', 'w'])
                .split([' ', ','])
                .next()
                .and_then(|x| x.parse::<f64>().ok()),
        )
        && d != 0.0
    {
        return Some(n / d);
    }
    // First chunk up to a separator.
    let chunk: String = t
        .split([',', '(', '@', ';', '~'])
        .next()
        .unwrap_or("")
        .to_string();
    // Qualifiers: "120mOhm Max", "5 V typ".
    let chunk = chunk
        .split_whitespace()
        .filter(|w| {
            !matches!(
                w.to_lowercase().as_str(),
                "max" | "min" | "typ" | "nom" | "max." | "min." | "typ."
            )
        })
        .collect::<Vec<_>>()
        .join(" ");
    let cleaned = chunk
        .replace("Ohms", "")
        .replace("ohms", "")
        .replace("Ohm", "")
        .replace("ohm", "")
        .replace('Ω', "")
        .replace("Farads", "")
        .replace(' ', "");
    let cleaned = cleaned
        .trim_end_matches(['F', 'H', 'V', 'A', 'W'])
        .to_string();
    let percent = lower.contains('%');
    let cleaned = cleaned.trim_end_matches('%');
    if cleaned.is_empty() {
        return None;
    }
    let v = ecs_units::parse(cleaned).ok()?;
    Some(if percent || key == "tolerance" && v > 0.5 {
        v / 100.0
    } else {
        v
    })
}

/// Add an attribute to a part, normalized when possible.
pub fn add(part: &mut VendorPart, label: &str, value: &str) {
    let value = value.trim();
    if value.is_empty() || value == "-" {
        return;
    }
    match canonical(label) {
        Some(key) if text_key(key) => {
            if key == "package" && part.package.is_empty() {
                part.package = value.to_string();
            }
            part.params
                .insert(key.into(), ParamValue::Text(value.into()));
        }
        Some(key) => match parse_si(value, key) {
            Some(v) => {
                part.params.insert(key.into(), ParamValue::Number(v));
            }
            None => {
                part.params
                    .insert(label.trim().into(), ParamValue::Text(value.into()));
            }
        },
        None => {
            // Dielectric codes hide under "Temperature Coefficient".
            let up = value.to_uppercase();
            if [
                "X7R", "X5R", "C0G", "NP0", "Y5V", "X6S", "X7S", "X8R", "Z5U",
            ]
            .iter()
            .any(|d| up.starts_with(d))
            {
                part.params.insert(
                    "dielectric".into(),
                    ParamValue::Text(up.split_whitespace().next().unwrap_or("").into()),
                );
            }
            part.params
                .insert(label.trim().into(), ParamValue::Text(value.into()));
        }
    }
}

/// Values a description names (`10kΩ`, `100nF`, `±1%`, `50V`, `100mW`, `10uH`).
pub fn from_description(part: &mut VendorPart) {
    let desc = part.description.clone();
    for raw in desc.split([' ', ',', ';']) {
        let w = raw.trim();
        // "25volts", "5volt" → "25V".
        let lower = w.to_lowercase();
        let spelled = lower
            .strip_suffix("volts")
            .or_else(|| lower.strip_suffix("volt"))
            .filter(|n| !n.is_empty())
            .map(|n| format!("{n}V"));
        let w = spelled.as_deref().unwrap_or(w);
        if w.len() < 2
            || !w
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit() || c == '±' || c == '.')
        {
            continue;
        }
        let key = if w.ends_with('Ω')
            || w.to_lowercase().ends_with("ohm")
            || w.to_lowercase().ends_with("ohms")
        {
            "resistance"
        } else if w.ends_with('%') && w.starts_with('±') {
            "tolerance"
        } else if w.ends_with('F')
            && w.len() > 2
            && w.chars().rev().nth(1).is_some_and(|c| "pnuµμm".contains(c))
        {
            "capacitance"
        } else if w.ends_with('H') && w.chars().rev().nth(1).is_some_and(|c| "pnuµμm".contains(c))
        {
            "inductance"
        } else if w.ends_with('W') {
            "power_rating"
        } else if w.ends_with('V') && !w.contains('~') {
            "voltage_rating"
        } else {
            continue;
        };
        if part.params.contains_key(key) {
            continue;
        }
        if let Some(v) = parse_si(w, key) {
            part.params.insert(key.into(), ParamValue::Number(v));
        }
    }
    let up = desc.to_uppercase();
    for d in ["X7R", "X5R", "C0G", "NP0", "Y5V", "X6S", "X7S"] {
        if up.contains(d) && !part.params.contains_key("dielectric") {
            part.params
                .insert("dielectric".into(), ParamValue::Text(d.into()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_and_values() {
        assert_eq!(canonical("Resistance"), Some("resistance"));
        assert_eq!(canonical("Power (Watts)"), Some("power_rating"));
        assert_eq!(canonical("Voltage - Rated"), Some("voltage_rating"));
        assert_eq!(canonical("DC Resistance (DCR)"), Some("dc_resistance"));
        assert_eq!(
            canonical("Voltage - Zener (Nom) (Vz)"),
            Some("zener_voltage")
        );
        assert_eq!(canonical("Package / Case"), Some("package"));
        assert_eq!(canonical("Rds On (Max) @ Id, Vgs"), None);
        let close =
            |a: Option<f64>, b: f64| a.is_some_and(|a| (a - b).abs() <= 1e-9 * b.abs().max(1.0));
        assert!(close(parse_si("10 kOhms", "resistance"), 1e4));
        assert!(close(parse_si("4.7kΩ", "resistance"), 4700.0));
        assert!(close(parse_si("±1%", "tolerance"), 0.01));
        assert!(close(parse_si("0.1W, 1/10W", "power_rating"), 0.1));
        assert!(close(parse_si("1/8W", "power_rating"), 0.125));
        assert!(close(parse_si("100nF", "capacitance"), 100e-9));
        assert!(close(parse_si("50V", "voltage_rating"), 50.0));
        assert!(close(parse_si("10µH", "inductance"), 10e-6));
        assert!(close(parse_si("2.2 µF", "capacitance"), 2.2e-6));
    }

    #[test]
    fn descriptions() {
        let mut p = VendorPart {
            description:
                "-55℃~+155℃ 100mW 10kΩ 75V Thick Film Resistor ±1% ±100ppm/℃ 0603 Chip Resistor"
                    .into(),
            ..Default::default()
        };
        from_description(&mut p);
        assert_eq!(p.number("resistance"), Some(1e4));
        assert_eq!(p.number("tolerance"), Some(0.01));
        assert_eq!(p.number("power_rating"), Some(0.1));
        assert_eq!(p.number("voltage_rating"), Some(75.0));
        let mut c = VendorPart {
            description: "50V 100nF X7R ±10% 0402 Multilayer Ceramic Capacitors".into(),
            ..Default::default()
        };
        from_description(&mut c);
        assert_eq!(c.number("capacitance"), Some(100e-9));
        assert_eq!(
            c.params.get("dielectric"),
            Some(&ParamValue::Text("X7R".into()))
        );
    }
}
