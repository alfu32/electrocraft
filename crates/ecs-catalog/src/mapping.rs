//! From catalog part to simulated component (spec ADD-01 section 6). Only
//! definition ids and parameter texts: the app places the part.

use crate::{ParamValue, VendorPart};

/// What "Place" creates.
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    pub def: &'static str,
    /// Parameter overrides as typed text (engineering notation).
    pub params: Vec<(String, String)>,
    /// Facts kept as sourcing metadata, not simulation parameters.
    pub metadata: Vec<(String, String)>,
    /// A generic model of the class: the inspector offers a SPICE model.
    pub generic: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Resistor,
    Capacitor,
    Inductor,
    Diode,
    Zener,
    Schottky,
    Led,
    Npn,
    Pnp,
    Nmos,
    Pmos,
    OpAmp,
    Comparator,
    Timer555,
    Other,
}

/// Classify from category, description and parameters.
pub fn classify(p: &VendorPart) -> Class {
    let text = format!("{} {} {}", p.category, p.description, p.mpn).to_lowercase();
    let has = |s: &str| text.contains(s);
    if has("potentiometer") || has("trimmer") || has("thermistor") || has("varistor") {
        return Class::Other;
    }
    if has("resistor") || (p.number("resistance").is_some() && !has("mosfet") && !has("transistor"))
    {
        return Class::Resistor;
    }
    if has("capacitor") || p.number("capacitance").is_some() && !has("diode") {
        return Class::Capacitor;
    }
    if has("inductor") || has("choke") || p.number("inductance").is_some() {
        return Class::Inductor;
    }
    if has("led") && (has("light") || has("emitting") || has("indicator") || has(" led")) {
        return Class::Led;
    }
    if has("zener") {
        return Class::Zener;
    }
    if has("schottky") {
        return Class::Schottky;
    }
    if has("diode") || has("rectifier") {
        return Class::Diode;
    }
    if has("555") || has("timer") {
        return Class::Timer555;
    }
    if has("comparator") {
        return Class::Comparator;
    }
    if has("op amp")
        || has("opamp")
        || has("operational amplifier")
        || has("amplifier") && has("ic")
    {
        return Class::OpAmp;
    }
    if has("mosfet") || has("fet") {
        return if has("p-ch") || has("p-channel") || has("pmos") {
            Class::Pmos
        } else {
            Class::Nmos
        };
    }
    if has("pnp") {
        return Class::Pnp;
    }
    if has("npn") || has("bipolar") || has("bjt") || has("transistor") {
        return Class::Npn;
    }
    Class::Other
}

fn eng(v: f64) -> String {
    ecs_units::format_digits(v, "", 4).replace(' ', "")
}

/// The component "Place" creates, or None (assign only).
pub fn placement(p: &VendorPart) -> Option<Placement> {
    let mut params = Vec::new();
    let mut metadata = Vec::new();
    let meta = |m: &mut Vec<(String, String)>, key: &str, unit: &str| match p.params.get(key) {
        Some(ParamValue::Number(v)) => m.push((key.into(), ecs_units::format_digits(*v, unit, 3))),
        Some(ParamValue::Text(t)) => m.push((key.into(), t.clone())),
        None => {}
    };
    let (def, generic) = match classify(p) {
        Class::Resistor => {
            params.push(("R".into(), eng(p.number("resistance")?)));
            meta(&mut metadata, "tolerance", "");
            meta(&mut metadata, "power_rating", "W");
            ("core.resistor", false)
        }
        Class::Capacitor => {
            params.push(("C".into(), eng(p.number("capacitance")?)));
            // The built-in capacitor has no ESR parameter: keep it as metadata.
            meta(&mut metadata, "esr", "Ω");
            meta(&mut metadata, "voltage_rating", "V");
            meta(&mut metadata, "dielectric", "");
            meta(&mut metadata, "tolerance", "");
            ("core.capacitor", false)
        }
        Class::Inductor => {
            params.push(("L".into(), eng(p.number("inductance")?)));
            if let Some(r) = p.number("dc_resistance") {
                params.push(("Rs".into(), eng(r)));
            }
            meta(&mut metadata, "current_rating", "A");
            ("core.inductor", false)
        }
        Class::Zener => {
            if let Some(v) = p
                .number("zener_voltage")
                .or_else(|| p.number("voltage_rating"))
            {
                params.push(("BV".into(), eng(v)));
            }
            ("core.zener", false)
        }
        Class::Schottky => ("core.schottky", false),
        Class::Diode => ("core.diode", false),
        Class::Led => {
            if let Some(ParamValue::Text(c)) = p.params.get("color") {
                let c = c.to_lowercase();
                if let Some(name) = ["red", "green", "blue", "yellow", "white"]
                    .iter()
                    .find(|n| c.contains(*n))
                {
                    params.push(("color".into(), name.to_string()));
                }
            }
            ("core.led", false)
        }
        Class::Npn => ("core.npn", true),
        Class::Pnp => ("core.pnp", true),
        Class::Nmos => ("core.nmos", true),
        Class::Pmos => ("core.pmos", true),
        Class::OpAmp => ("core.opamp", true),
        Class::Comparator => ("core.comparator", true),
        Class::Timer555 => ("core.ne555", true),
        Class::Other => return None,
    };
    Some(Placement {
        def,
        params,
        metadata,
        generic,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::normalize;

    fn part(category: &str, desc: &str, attrs: &[(&str, &str)]) -> VendorPart {
        let mut p = VendorPart {
            category: category.into(),
            description: desc.into(),
            ..Default::default()
        };
        for (k, v) in attrs {
            normalize::add(&mut p, k, v);
        }
        p
    }

    #[test]
    fn each_category_maps() {
        let r = placement(&part(
            "Chip Resistor - Surface Mount",
            "RES 10K OHM 1% 1/10W 0603",
            &[
                ("Resistance", "10 kOhms"),
                ("Tolerance", "±1%"),
                ("Power (Watts)", "0.1W, 1/10W"),
            ],
        ))
        .unwrap();
        assert_eq!(
            (r.def, r.params.clone(), r.generic),
            (
                "core.resistor",
                vec![("R".to_string(), "10k".to_string())],
                false
            )
        );
        assert!(
            r.metadata
                .iter()
                .any(|(k, v)| k == "power_rating" && v.contains("100"))
        );
        let c = placement(&part(
            "Ceramic Capacitors",
            "CAP CER 100NF 50V X7R 0402",
            &[
                ("Capacitance", "0.1 µF"),
                ("Voltage - Rated", "50V"),
                ("Temperature Coefficient", "X7R"),
            ],
        ))
        .unwrap();
        assert_eq!(c.def, "core.capacitor");
        assert_eq!(c.params[0], ("C".to_string(), "100n".to_string()));
        assert!(
            c.metadata
                .iter()
                .any(|(k, v)| k == "dielectric" && v == "X7R")
        );
        let l = placement(&part(
            "Fixed Inductors",
            "10µH",
            &[
                ("Inductance", "10 µH"),
                ("DC Resistance (DCR)", "120mOhm Max"),
                ("Current Rating (Amps)", "1.2 A"),
            ],
        ))
        .unwrap();
        assert_eq!(l.def, "core.inductor");
        assert!(l.params.contains(&("Rs".to_string(), "120m".to_string())));
        let z = placement(&part(
            "Diodes - Zener - Single",
            "DIODE ZENER 5.1V 1W",
            &[("Voltage - Zener (Nom) (Vz)", "5.1 V")],
        ))
        .unwrap();
        assert_eq!((z.def, z.params[0].1.as_str()), ("core.zener", "5.1"));
        let q = placement(&part(
            "Bipolar Transistors - BJT",
            "TRANS NPN 40V 0.6A TO92",
            &[],
        ))
        .unwrap();
        assert_eq!((q.def, q.generic), ("core.npn", true));
        let m = placement(&part("MOSFETs", "MOSFET P-CH 20V", &[])).unwrap();
        assert_eq!(m.def, "core.pmos");
        assert!(placement(&part("Connectors", "HEADER 2.54MM 10POS", &[])).is_none());
    }
}
