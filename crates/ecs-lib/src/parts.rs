//! Part numbers: real devices mapped to a definition and typical model
//! parameters (from manufacturers' published SPICE models, rounded). Picking
//! one in the inspector applies its parameters; any text is accepted too.

use std::sync::OnceLock;

pub use ecs_model::KnownPart;

fn p(number: &str, def: &str, params: &[(&str, f64)], note: &str) -> KnownPart {
    KnownPart {
        number: number.into(),
        def: def.into(),
        params: params.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        note: note.into(),
        default: false,
    }
}

fn discrete() -> Vec<KnownPart> {
    let mut v = vec![
        // Small-signal and rectifier diodes.
        p(
            "1N4148",
            "core.diode",
            &[
                ("Is", 2.52e-9),
                ("N", 1.752),
                ("BV", 100.0),
                ("IBV", 100e-6),
                ("Cj", 4e-12),
            ],
            "Small-signal switching diode, 100 V",
        ),
        p(
            "1N914",
            "core.diode",
            &[
                ("Is", 2.52e-9),
                ("N", 1.752),
                ("BV", 100.0),
                ("IBV", 100e-6),
                ("Cj", 4e-12),
            ],
            "Small-signal switching diode, 100 V",
        ),
        p(
            "1N4001",
            "core.diode",
            &[
                ("Is", 14.1e-9),
                ("N", 1.984),
                ("BV", 50.0),
                ("IBV", 5e-6),
                ("Cj", 25.9e-12),
            ],
            "1 A rectifier, 50 V",
        ),
        p(
            "1N4002",
            "core.diode",
            &[
                ("Is", 14.1e-9),
                ("N", 1.984),
                ("BV", 100.0),
                ("IBV", 5e-6),
                ("Cj", 25.9e-12),
            ],
            "1 A rectifier, 100 V",
        ),
        p(
            "1N4004",
            "core.diode",
            &[
                ("Is", 14.1e-9),
                ("N", 1.984),
                ("BV", 400.0),
                ("IBV", 5e-6),
                ("Cj", 25.9e-12),
            ],
            "1 A rectifier, 400 V",
        ),
        p(
            "1N4007",
            "core.diode",
            &[
                ("Is", 14.1e-9),
                ("N", 1.984),
                ("BV", 1000.0),
                ("IBV", 5e-6),
                ("Cj", 25.9e-12),
            ],
            "1 A rectifier, 1000 V",
        ),
        p(
            "1N5817",
            "core.schottky",
            &[
                ("Is", 31.7e-6),
                ("N", 1.373),
                ("BV", 20.0),
                ("IBV", 1e-3),
                ("Cj", 190e-12),
            ],
            "1 A Schottky, 20 V",
        ),
        p(
            "1N5819",
            "core.schottky",
            &[
                ("Is", 31.7e-6),
                ("N", 1.373),
                ("BV", 40.0),
                ("IBV", 1e-3),
                ("Cj", 190e-12),
            ],
            "1 A Schottky, 40 V",
        ),
        p(
            "BAT85",
            "core.schottky",
            &[
                ("Is", 0.2e-6),
                ("N", 1.03),
                ("BV", 30.0),
                ("IBV", 10e-6),
                ("Cj", 10e-12),
            ],
            "Small-signal Schottky, 30 V",
        ),
        // BJTs.
        p(
            "2N2222A",
            "core.npn",
            &[
                ("Is", 14.34e-15),
                ("Bf", 255.9),
                ("Br", 6.092),
                ("Vaf", 74.03),
                ("Cje", 22.01e-12),
                ("Cjc", 7.306e-12),
            ],
            "General-purpose NPN, 40 V 600 mA",
        ),
        p(
            "2N2222",
            "core.npn",
            &[
                ("Is", 14.34e-15),
                ("Bf", 255.9),
                ("Br", 6.092),
                ("Vaf", 74.03),
                ("Cje", 22.01e-12),
                ("Cjc", 7.306e-12),
            ],
            "General-purpose NPN, 30 V 600 mA",
        ),
        p(
            "2N3904",
            "core.npn",
            &[
                ("Is", 6.734e-15),
                ("Bf", 416.4),
                ("Br", 0.7371),
                ("Vaf", 74.03),
                ("Cje", 4.493e-12),
                ("Cjc", 3.638e-12),
            ],
            "Small-signal NPN, 40 V 200 mA",
        ),
        p(
            "2N4401",
            "core.npn",
            &[
                ("Is", 26.03e-15),
                ("Bf", 4292.0),
                ("Br", 1.01),
                ("Vaf", 90.7),
                ("Cje", 22.01e-12),
                ("Cjc", 7.306e-12),
            ],
            "General-purpose NPN, 40 V 600 mA",
        ),
        p(
            "BC547B",
            "core.npn",
            &[
                ("Is", 7.049e-15),
                ("Bf", 374.5),
                ("Br", 1.3),
                ("Vaf", 62.79),
                ("Cje", 11.5e-12),
                ("Cjc", 5.25e-12),
            ],
            "Small-signal NPN, 45 V 100 mA",
        ),
        p(
            "BC337",
            "core.npn",
            &[
                ("Is", 41.1e-15),
                ("Bf", 400.0),
                ("Br", 35.5),
                ("Vaf", 72.0),
                ("Cje", 47e-12),
                ("Cjc", 15e-12),
            ],
            "NPN, 45 V 800 mA",
        ),
        p(
            "2N3906",
            "core.pnp",
            &[
                ("Is", 1.41e-15),
                ("Bf", 180.7),
                ("Br", 4.977),
                ("Vaf", 18.7),
                ("Cje", 8.063e-12),
                ("Cjc", 4.017e-12),
            ],
            "Small-signal PNP, 40 V 200 mA",
        ),
        p(
            "2N2907A",
            "core.pnp",
            &[
                ("Is", 650.6e-18),
                ("Bf", 231.7),
                ("Br", 3.563),
                ("Vaf", 115.7),
                ("Cje", 19.82e-12),
                ("Cjc", 14.76e-12),
            ],
            "General-purpose PNP, 60 V 600 mA",
        ),
        p(
            "BC557B",
            "core.pnp",
            &[
                ("Is", 10.2e-15),
                ("Bf", 290.0),
                ("Br", 6.0),
                ("Vaf", 52.0),
                ("Cje", 15e-12),
                ("Cjc", 9e-12),
            ],
            "Small-signal PNP, 45 V 100 mA",
        ),
        // MOSFETs (level-1 fits).
        p(
            "2N7000",
            "core.nmos",
            &[
                ("Vto", 2.1),
                ("Kp", 0.32),
                ("lambda", 0.01),
                ("Cgs", 20e-12),
                ("Cgd", 5e-12),
            ],
            "Small-signal N-MOSFET, 60 V 200 mA",
        ),
        p(
            "BS170",
            "core.nmos",
            &[
                ("Vto", 1.8),
                ("Kp", 0.25),
                ("lambda", 0.01),
                ("Cgs", 24e-12),
                ("Cgd", 5e-12),
            ],
            "Small-signal N-MOSFET, 60 V 500 mA",
        ),
        p(
            "IRF540N",
            "core.nmos",
            &[
                ("Vto", 3.5),
                ("Kp", 20.0),
                ("lambda", 0.005),
                ("Cgs", 1.7e-9),
                ("Cgd", 0.15e-9),
            ],
            "Power N-MOSFET, 100 V 33 A",
        ),
        p(
            "IRLZ44N",
            "core.nmos",
            &[
                ("Vto", 1.6),
                ("Kp", 35.0),
                ("lambda", 0.005),
                ("Cgs", 1.6e-9),
                ("Cgd", 0.2e-9),
            ],
            "Logic-level power N-MOSFET, 55 V 47 A",
        ),
        p(
            "BS250",
            "core.pmos",
            &[
                ("Vto", 2.5),
                ("Kp", 0.15),
                ("lambda", 0.01),
                ("Cgs", 25e-12),
                ("Cgd", 6e-12),
            ],
            "Small-signal P-MOSFET, 45 V 230 mA",
        ),
        p(
            "IRF9540N",
            "core.pmos",
            &[
                ("Vto", 3.5),
                ("Kp", 10.0),
                ("lambda", 0.005),
                ("Cgs", 1.3e-9),
                ("Cgd", 0.2e-9),
            ],
            "Power P-MOSFET, 100 V 23 A",
        ),
        // Op-amps and comparators (the macromodel has gain, rails and impedances).
        p(
            "LM741",
            "core.opamp",
            &[("A", 2e5), ("Rin", 2e6), ("Rout", 75.0)],
            "General-purpose op-amp",
        ),
        p(
            "LM358",
            "core.opamp",
            &[("A", 1e5), ("Rin", 1e9), ("Rout", 50.0)],
            "Dual single-supply op-amp",
        ),
        p(
            "TL071",
            "core.opamp",
            &[("A", 2e5), ("Rin", 1e12), ("Rout", 50.0)],
            "Low-noise JFET-input op-amp",
        ),
        p(
            "TL072",
            "core.opamp",
            &[("A", 2e5), ("Rin", 1e12), ("Rout", 50.0)],
            "Dual JFET-input op-amp",
        ),
        p(
            "NE5532",
            "core.opamp",
            &[("A", 1e5), ("Rin", 300e3), ("Rout", 30.0)],
            "Low-noise audio op-amp",
        ),
        p(
            "LM393",
            "core.comparator",
            &[],
            "Dual open-collector comparator",
        ),
        p("LM311", "core.comparator", &[], "Comparator"),
        // Timers.
        p("NE555", "core.ne555", &[], "Bipolar 555 timer"),
        p("LM555", "core.ne555", &[], "Bipolar 555 timer"),
        p("ICM7555", "core.ne555", &[], "CMOS 555 timer"),
        p("TLC555", "core.ne555", &[], "CMOS 555 timer"),
    ];
    // Zeners: the 1N47xxA series by voltage.
    for (n, bv) in [
        (28, 3.3),
        (29, 3.6),
        (30, 3.9),
        (31, 4.3),
        (32, 4.7),
        (33, 5.1),
        (34, 5.6),
        (35, 6.2),
        (36, 6.8),
        (37, 7.5),
        (38, 8.2),
        (39, 9.1),
        (40, 10.0),
        (41, 11.0),
        (42, 12.0),
        (43, 13.0),
        (44, 15.0),
        (45, 16.0),
        (46, 18.0),
        (47, 20.0),
        (48, 22.0),
        (49, 24.0),
    ] {
        v.push(p(
            &format!("1N47{n}A"),
            "core.zener",
            &[("BV", bv), ("IBV", 1e-3)],
            &format!("1 W Zener, {bv} V"),
        ));
    }
    v
}

/// Every known part number: discrete parts and the logic and microcontroller
/// families (which register their own).
pub fn known_parts() -> &'static [KnownPart] {
    static ALL: OnceLock<Vec<KnownPart>> = OnceLock::new();
    ALL.get_or_init(|| {
        let mut v = discrete();
        v.extend(crate::ics::part_numbers());
        v.extend(ecs_mcu::part_numbers());
        v
    })
}

/// Known part numbers for a definition.
pub fn for_def(def: &str) -> Vec<&'static KnownPart> {
    known_parts().iter().filter(|k| k.def == def).collect()
}

/// A known part by number (case-insensitive).
pub fn find(number: &str) -> Option<&'static KnownPart> {
    let n = number.trim();
    known_parts()
        .iter()
        .find(|k| k.number.eq_ignore_ascii_case(n))
}

/// The number a new part of this definition starts with, if any.
pub fn default_for(def: &str) -> Option<&'static KnownPart> {
    known_parts().iter().find(|k| k.def == def && k.default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_known_part_names_real_parameters() {
        let lib = crate::library();
        for k in known_parts() {
            let def = lib
                .get(&k.def)
                .unwrap_or_else(|| panic!("{}: unknown {}", k.number, k.def));
            for (name, _) in &k.params {
                assert!(
                    def.params.iter().any(|p| &p.name == name),
                    "{}: {} has no {name}",
                    k.number,
                    k.def
                );
            }
        }
        let mut seen = std::collections::BTreeSet::new();
        for k in known_parts() {
            assert!(
                seen.insert(k.number.to_uppercase()),
                "duplicate {}",
                k.number
            );
        }
        assert_eq!(find("2n2222a").unwrap().def, "core.npn");
    }
}
