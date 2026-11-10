//! Built-in component library (spec section 7), written against the same
//! [`ComponentDef`] contract user components use. Every equation is an
//! expression; nothing here touches solver internals.

mod digital;

use ecs_expr::{Expr, parse};
use ecs_model::{
    BranchDef, ComponentDef, ControlDef, DefKind, Library, LimitDef, ParamDef, ParamKind, PinDef,
    PinKind, ProbeDef, StateDef,
};

pub(crate) fn e(src: &str) -> Expr {
    parse(src).unwrap_or_else(|err| panic!("built-in expression {src:?}: {err}"))
}

fn pins(def: &mut ComponentDef, names: &[&str]) {
    def.pins = names
        .iter()
        .map(|n| PinDef {
            name: n.to_string(),
            kind: PinKind::Analog,
        })
        .collect();
}

fn current(def: &mut ComponentDef, a: &str, b: &str, i: &str, q: Option<&str>) {
    def.branches.push(BranchDef::Current {
        name: None,
        a: a.into(),
        b: b.into(),
        i: e(i),
        q: q.map(e),
    });
}

fn voltage(def: &mut ComponentDef, name: &str, a: &str, b: &str, v: &str, flux: Option<&str>) {
    def.branches.push(BranchDef::Voltage {
        name: name.into(),
        a: a.into(),
        b: b.into(),
        v: e(v),
        flux: flux.map(e),
    });
}

pub(crate) fn probe(def: &mut ComponentDef, name: &str, expr: &str, unit: &str) {
    def.probes.push(ProbeDef {
        name: name.into(),
        expr: e(expr),
        unit: unit.into(),
    });
}

pub(crate) fn describe(mut def: ComponentDef, text: &str) -> ComponentDef {
    def.meta.description = text.into();
    def
}

fn ground() -> ComponentDef {
    let mut d = ComponentDef::new("core.ground", "Ground", "Sources", "GND", "ground");
    pins(&mut d, &["G"]);
    d.kind = DefKind::Ground;
    describe(d, "Reference node, 0 V.")
}

fn resistor() -> ComponentDef {
    let mut d = ComponentDef::new("core.resistor", "Resistor", "Passive", "R", "resistor");
    pins(&mut d, &["A", "B"]);
    d.params
        .push(ParamDef::real("R", 1e3, "Ω", "Resistance").range(1e-6, 1e12));
    current(&mut d, "A", "B", "V(A, B) / R", None);
    probe(&mut d, "I", "V(A, B) / R", "A");
    probe(&mut d, "P", "V(A, B) * V(A, B) / R", "W");
    describe(d, "Ideal linear resistor.")
}

fn capacitor() -> ComponentDef {
    let mut d = ComponentDef::new("core.capacitor", "Capacitor", "Passive", "C", "capacitor");
    pins(&mut d, &["A", "B"]);
    d.params
        .push(ParamDef::real("C", 1e-6, "F", "Capacitance").range(1e-15, 1e3));
    d.params
        .push(ParamDef::real("Rleak", 1e12, "Ω", "Leakage resistance").range(1.0, 1e15));
    current(&mut d, "A", "B", "V(A, B) / Rleak", Some("C * V(A, B)"));
    probe(&mut d, "V", "V(A, B)", "V");
    probe(&mut d, "E", "0.5 * C * V(A, B) * V(A, B)", "J");
    describe(d, "Capacitor in charge form, with a leakage resistance.")
}

fn inductor() -> ComponentDef {
    let mut d = ComponentDef::new("core.inductor", "Inductor", "Passive", "L", "inductor");
    pins(&mut d, &["A", "B"]);
    d.params
        .push(ParamDef::real("L", 1e-3, "H", "Inductance").range(1e-12, 1e3));
    d.params
        .push(ParamDef::real("Rs", 0.0, "Ω", "Series resistance").range(0.0, 1e6));
    voltage(&mut d, "L", "A", "B", "Rs * I(L)", Some("L * I(L)"));
    probe(&mut d, "I", "I(L)", "A");
    probe(&mut d, "E", "0.5 * L * I(L) * I(L)", "J");
    describe(d, "Inductor as a flux branch.")
}

fn potentiometer() -> ComponentDef {
    let mut d = ComponentDef::new(
        "core.potentiometer",
        "Potentiometer",
        "Passive",
        "RV",
        "potentiometer",
    );
    pins(&mut d, &["A", "W", "B"]);
    d.params
        .push(ParamDef::real("R", 10e3, "Ω", "Total resistance").range(1.0, 1e9));
    d.params.push(
        ParamDef::real("position", 0.5, "", "Wiper position, 0 at A")
            .range(0.0, 1.0)
            .live(),
    );
    current(
        &mut d,
        "A",
        "W",
        "V(A, W) / (R * max(position, 1e-4))",
        None,
    );
    current(
        &mut d,
        "W",
        "B",
        "V(W, B) / (R * max(1 - position, 1e-4))",
        None,
    );
    probe(&mut d, "Vw", "V(W, B)", "V");
    d.controls.push(ControlDef::Slider {
        param: "position".into(),
        label: "Wiper".into(),
    });
    describe(
        d,
        "Three-terminal potentiometer; the wiper is a live parameter.",
    )
}

fn lamp() -> ComponentDef {
    let mut d = ComponentDef::new("core.lamp", "Lamp", "Indicators", "LP", "lamp");
    pins(&mut d, &["A", "B"]);
    d.params
        .push(ParamDef::real("R", 100.0, "Ω", "Hot resistance").range(1e-3, 1e9));
    d.params
        .push(ParamDef::real("Prated", 0.25, "W", "Rated power").range(1e-6, 1e6));
    current(&mut d, "A", "B", "V(A, B) / R", None);
    probe(&mut d, "I", "V(A, B) / R", "A");
    probe(&mut d, "P", "V(A, B) * V(A, B) / R", "W");
    probe(
        &mut d,
        "brightness",
        "min(V(A, B) * V(A, B) / R / Prated, 1)",
        "",
    );
    describe(d, "Incandescent lamp; glows in proportion to its power.")
}

fn vdc() -> ComponentDef {
    let mut d = ComponentDef::new("core.vdc", "DC voltage", "Sources", "V", "vdc");
    pins(&mut d, &["P", "N"]);
    d.params
        .push(ParamDef::real("V", 5.0, "V", "Voltage").live());
    d.params
        .push(ParamDef::real("AC", 0.0, "V", "AC analysis magnitude"));
    voltage(&mut d, "V", "P", "N", "V", None);
    probe(&mut d, "I", "-I(V)", "A");
    probe(&mut d, "P", "-V * I(V)", "W");
    describe(d, "Ideal DC voltage source.")
}

fn battery() -> ComponentDef {
    let mut d = ComponentDef::new("core.battery", "Battery", "Sources", "B", "battery");
    pins(&mut d, &["P", "N"]);
    d.params
        .push(ParamDef::real("V", 9.0, "V", "Open-circuit voltage").live());
    d.params
        .push(ParamDef::real("r", 0.5, "Ω", "Internal resistance").range(0.0, 1e6));
    voltage(&mut d, "B", "P", "N", "V + r * I(B)", None);
    probe(&mut d, "I", "-I(B)", "A");
    describe(d, "Battery with internal resistance.")
}

fn idc() -> ComponentDef {
    let mut d = ComponentDef::new("core.idc", "DC current", "Sources", "I", "idc");
    pins(&mut d, &["P", "N"]);
    d.params
        .push(ParamDef::real("I", 1e-3, "A", "Current out of P").live());
    d.params
        .push(ParamDef::real("AC", 0.0, "A", "AC analysis magnitude"));
    current(&mut d, "N", "P", "I", None);
    probe(&mut d, "V", "V(P, N)", "V");
    describe(d, "Ideal DC current source; current leaves through P.")
}

fn vsine() -> ComponentDef {
    let mut d = ComponentDef::new("core.vsine", "Sine source", "Sources", "V", "vac");
    pins(&mut d, &["P", "N"]);
    d.params
        .push(ParamDef::real("Vamp", 5.0, "V", "Amplitude").live());
    d.params.push(
        ParamDef::real("freq", 1e3, "Hz", "Frequency")
            .range(0.0, 1e12)
            .live(),
    );
    d.params
        .push(ParamDef::real("Voff", 0.0, "V", "Offset").live());
    d.params
        .push(ParamDef::real("phase", 0.0, "°", "Phase").live());
    d.params
        .push(ParamDef::real("AC", 1.0, "V", "AC analysis magnitude"));
    voltage(
        &mut d,
        "V",
        "P",
        "N",
        "Voff + Vamp * sin(2 * pi * freq * t + phase * pi / 180)",
        None,
    );
    probe(&mut d, "I", "-I(V)", "A");
    describe(d, "Sinusoidal voltage source.")
}

fn funcgen() -> ComponentDef {
    let mut d = ComponentDef::new(
        "core.funcgen",
        "Function generator",
        "Sources",
        "FG",
        "funcgen",
    );
    pins(&mut d, &["P", "N"]);
    let mut wave = ParamDef::real("wave", 1.0, "", "Waveform").live();
    wave.kind = ParamKind::Enum(vec![
        "sine".into(),
        "square".into(),
        "triangle".into(),
        "sawtooth".into(),
    ]);
    d.params.push(wave);
    d.params
        .push(ParamDef::real("Vlow", 0.0, "V", "Low level").live());
    d.params
        .push(ParamDef::real("Vhigh", 5.0, "V", "High level").live());
    d.params.push(
        ParamDef::real("freq", 1e3, "Hz", "Frequency")
            .range(0.0, 1e12)
            .live(),
    );
    d.params.push(
        ParamDef::real("duty", 0.5, "", "Duty cycle (square)")
            .range(0.0, 1.0)
            .live(),
    );
    d.lets.push(("ph".into(), e("freq * t - floor(freq * t)")));
    d.lets.push((
        "unit".into(),
        e("select(wave < 0.5, 0.5 + 0.5 * sin(2 * pi * ph), \
         select(wave < 1.5, select(ph < duty, 1, 0), \
         select(wave < 2.5, 1 - abs(2 * ph - 1), ph)))"),
    ));
    voltage(
        &mut d,
        "V",
        "P",
        "N",
        "Vlow + (Vhigh - Vlow) * max(min(unit, 1), 0)",
        None,
    );
    probe(&mut d, "I", "-I(V)", "A");
    describe(d, "Sine, square, triangle or sawtooth between two levels.")
}

/// Junction defaults: saturation current, emission coefficient, breakdown
/// voltage, breakdown current and junction capacitance.
struct Junction {
    is: f64,
    n: f64,
    bv: f64,
    ibv: f64,
    cj: f64,
}

fn diode_like(id: &str, name: &str, symbol: &str, j: Junction) -> ComponentDef {
    let Junction { is, n, bv, ibv, cj } = j;
    let mut d = ComponentDef::new(id, name, "Diodes", "D", symbol);
    pins(&mut d, &["A", "K"]);
    d.params
        .push(ParamDef::real("Is", is, "A", "Saturation current").range(1e-30, 1.0));
    d.params
        .push(ParamDef::real("N", n, "", "Emission coefficient").range(0.1, 10.0));
    d.params
        .push(ParamDef::real("BV", bv, "V", "Reverse breakdown voltage").range(0.1, 1e6));
    d.params
        .push(ParamDef::real("IBV", ibv, "A", "Current at breakdown").range(1e-30, 1.0));
    d.params
        .push(ParamDef::real("Cj", cj, "F", "Junction capacitance").range(0.0, 1.0));
    d.lets.push(("Id".into(), e(
        "Is * (limexp(V(A, K) / (N * vt)) - 1) - IBV * limexp(-(V(A, K) + BV) / vt) + 1e-12 * V(A, K)",
    )));
    current(&mut d, "A", "K", "Id", Some("Cj * V(A, K)"));
    d.limits.push(LimitDef::Pn {
        a: "A".into(),
        b: "K".into(),
        vte: e("N * vt"),
        is: e("Is"),
    });
    probe(&mut d, "I", "Id", "A");
    probe(&mut d, "P", "V(A, K) * Id", "W");
    d
}

fn led() -> ComponentDef {
    let mut d = diode_like(
        "core.led",
        "LED",
        "led",
        Junction {
            is: 1e-18,
            n: 2.0,
            bv: 5.0,
            ibv: 1e-10,
            cj: 0.0,
        },
    );
    d.meta.category = "Diodes".into();
    let mut color = ParamDef::real("color", 0.0, "", "Colour");
    color.kind = ParamKind::Enum(vec![
        "red".into(),
        "green".into(),
        "blue".into(),
        "yellow".into(),
        "white".into(),
    ]);
    d.params.push(color);
    d.params
        .push(ParamDef::real("Imax", 20e-3, "A", "Current for full brightness").range(1e-6, 10.0));
    probe(&mut d, "brightness", "max(min(Id / Imax, 1), 0)", "");
    describe(
        d,
        "Light-emitting diode; glows in proportion to its current.",
    )
}

fn switch_spst() -> ComponentDef {
    let mut d = ComponentDef::new("core.switch", "Switch (SPST)", "Switches", "S", "switch");
    pins(&mut d, &["A", "B"]);
    d.params
        .push(ParamDef::real("Ron", 1e-3, "Ω", "Closed resistance").range(1e-9, 1e3));
    d.params
        .push(ParamDef::real("Roff", 1e10, "Ω", "Open resistance").range(1e3, 1e15));
    d.states.push(StateDef {
        name: "closed".into(),
        initial: 0.0,
    });
    current(
        &mut d,
        "A",
        "B",
        "V(A, B) * select(closed > 0.5, 1 / Ron, 1 / Roff)",
        None,
    );
    probe(
        &mut d,
        "I",
        "V(A, B) * select(closed > 0.5, 1 / Ron, 1 / Roff)",
        "A",
    );
    d.controls.push(ControlDef::Toggle {
        state: "closed".into(),
        label: "Closed".into(),
    });
    describe(
        d,
        "Single-pole single-throw switch. Click it while the circuit runs.",
    )
}

fn push_button() -> ComponentDef {
    let mut d = ComponentDef::new(
        "core.pushbutton",
        "Push button",
        "Switches",
        "S",
        "pushbutton",
    );
    pins(&mut d, &["A", "B"]);
    d.params
        .push(ParamDef::real("Ron", 1e-3, "Ω", "Closed resistance").range(1e-9, 1e3));
    d.params
        .push(ParamDef::real("Roff", 1e10, "Ω", "Open resistance").range(1e3, 1e15));
    d.states.push(StateDef {
        name: "pressed".into(),
        initial: 0.0,
    });
    current(
        &mut d,
        "A",
        "B",
        "V(A, B) * select(pressed > 0.5, 1 / Ron, 1 / Roff)",
        None,
    );
    probe(
        &mut d,
        "I",
        "V(A, B) * select(pressed > 0.5, 1 / Ron, 1 / Roff)",
        "A",
    );
    d.controls.push(ControlDef::Momentary {
        state: "pressed".into(),
        label: "Pressed".into(),
    });
    describe(d, "Normally open momentary push button.")
}

fn switch_spdt() -> ComponentDef {
    let mut d = ComponentDef::new("core.spdt", "Switch (SPDT)", "Switches", "S", "spdt");
    pins(&mut d, &["C", "A", "B"]);
    d.params
        .push(ParamDef::real("Ron", 1e-3, "Ω", "Closed resistance").range(1e-9, 1e3));
    d.params
        .push(ParamDef::real("Roff", 1e10, "Ω", "Open resistance").range(1e3, 1e15));
    d.states.push(StateDef {
        name: "position".into(),
        initial: 0.0,
    });
    current(
        &mut d,
        "C",
        "A",
        "V(C, A) * select(position < 0.5, 1 / Ron, 1 / Roff)",
        None,
    );
    current(
        &mut d,
        "C",
        "B",
        "V(C, B) * select(position < 0.5, 1 / Roff, 1 / Ron)",
        None,
    );
    d.controls.push(ControlDef::Toggle {
        state: "position".into(),
        label: "Throw".into(),
    });
    describe(d, "Single-pole double-throw switch: common C to A or to B.")
}

fn bjt(id: &str, name: &str, symbol: &str, pnp: bool) -> ComponentDef {
    let mut d = ComponentDef::new(id, name, "Transistors", "Q", symbol);
    pins(&mut d, &["C", "B", "E"]);
    d.params
        .push(ParamDef::real("Is", 1e-14, "A", "Transport saturation current").range(1e-30, 1.0));
    d.params
        .push(ParamDef::real("Nf", 1.0, "", "Forward emission coefficient").range(0.5, 4.0));
    d.params
        .push(ParamDef::real("Bf", 100.0, "", "Forward beta").range(1e-3, 1e6));
    d.params
        .push(ParamDef::real("Br", 1.0, "", "Reverse beta").range(1e-3, 1e6));
    d.params
        .push(ParamDef::real("Vaf", 100.0, "V", "Forward Early voltage").range(1e-3, 1e6));
    d.params
        .push(ParamDef::real("Cje", 10e-12, "F", "B-E capacitance").range(0.0, 1.0));
    d.params
        .push(ParamDef::real("Cjc", 5e-12, "F", "B-C capacitance").range(0.0, 1.0));
    // Ebers-Moll transport form. For PNP every junction voltage is reversed.
    let (be, bc) = if pnp {
        ("V(E, B)", "V(C, B)")
    } else {
        ("V(B, E)", "V(B, C)")
    };
    d.lets.push((
        "If".into(),
        e(&format!("Is * (limexp({be} / (Nf * vt)) - 1)")),
    ));
    d.lets
        .push(("Ir".into(), e(&format!("Is * (limexp({bc} / vt) - 1)"))));
    d.lets
        .push(("It".into(), e(&format!("(If - Ir) * (1 - {bc} / Vaf)"))));
    let (c, b, em) = ("C", "B", "E");
    if pnp {
        current(&mut d, em, c, "It", None);
        current(
            &mut d,
            em,
            b,
            &format!("If / Bf + 1e-12 * {be}"),
            Some(&format!("Cje * {be}")),
        );
        current(
            &mut d,
            c,
            b,
            &format!("Ir / Br + 1e-12 * {bc}"),
            Some(&format!("Cjc * {bc}")),
        );
        d.limits.push(LimitDef::Pn {
            a: "E".into(),
            b: "B".into(),
            vte: e("Nf * vt"),
            is: e("Is"),
        });
        d.limits.push(LimitDef::Pn {
            a: "C".into(),
            b: "B".into(),
            vte: e("vt"),
            is: e("Is"),
        });
        probe(&mut d, "Ic", "It - Ir / Br", "A");
    } else {
        current(&mut d, c, em, "It", None);
        current(
            &mut d,
            b,
            em,
            &format!("If / Bf + 1e-12 * {be}"),
            Some(&format!("Cje * {be}")),
        );
        current(
            &mut d,
            b,
            c,
            &format!("Ir / Br + 1e-12 * {bc}"),
            Some(&format!("Cjc * {bc}")),
        );
        d.limits.push(LimitDef::Pn {
            a: "B".into(),
            b: "E".into(),
            vte: e("Nf * vt"),
            is: e("Is"),
        });
        d.limits.push(LimitDef::Pn {
            a: "B".into(),
            b: "C".into(),
            vte: e("vt"),
            is: e("Is"),
        });
        probe(&mut d, "Ic", "It - Ir / Br", "A");
    }
    probe(&mut d, "Ib", "If / Bf + Ir / Br", "A");
    describe(
        d,
        "Bipolar transistor, Ebers-Moll transport model with Early effect.",
    )
}

fn mosfet(id: &str, name: &str, symbol: &str, pmos: bool) -> ComponentDef {
    let mut d = ComponentDef::new(id, name, "Transistors", "M", symbol);
    pins(&mut d, &["D", "G", "S"]);
    d.params.push(
        ParamDef::real("Kp", 20e-3, "A/V²", "Transconductance parameter (µCox·W/L)")
            .range(1e-9, 1e3),
    );
    d.params
        .push(ParamDef::real("Vto", 1.5, "V", "Threshold voltage magnitude").range(0.0, 100.0));
    d.params
        .push(ParamDef::real("lambda", 0.01, "1/V", "Channel-length modulation").range(0.0, 1.0));
    d.params
        .push(ParamDef::real("Cgs", 100e-12, "F", "Gate-source capacitance").range(0.0, 1.0));
    d.params
        .push(ParamDef::real("Cgd", 20e-12, "F", "Gate-drain capacitance").range(0.0, 1.0));
    let (gs, ds, gd) = if pmos {
        ("V(S, G)", "V(S, D)", "V(D, G)")
    } else {
        ("V(G, S)", "V(D, S)", "V(G, D)")
    };
    // Level 1 (Shichman-Hodges), symmetric in drain and source.
    d.lets.push(("vgs".into(), e(gs)));
    d.lets.push(("vds".into(), e(ds)));
    d.lets.push(("vgd".into(), e(&format!("-({gd})"))));
    d.lets.push(("fwd".into(), e(
        "select(vgs - Vto <= 0, 0, select(vds < vgs - Vto, Kp * ((vgs - Vto) * vds - vds * vds / 2), Kp / 2 * (vgs - Vto) * (vgs - Vto))) * (1 + lambda * vds)",
    )));
    d.lets.push(("rev".into(), e(
        "select(-vgd - Vto <= 0, 0, select(-vds < -vgd - Vto, Kp * ((-vgd - Vto) * -vds - vds * vds / 2), Kp / 2 * (-vgd - Vto) * (-vgd - Vto))) * (1 - lambda * vds)",
    )));
    d.lets
        .push(("Id".into(), e("select(vds >= 0, fwd, -rev) + 1e-12 * vds")));
    let _ = gd;
    if pmos {
        current(&mut d, "S", "D", "Id", None);
        current(&mut d, "S", "G", "0", Some("Cgs * V(S, G)"));
        current(&mut d, "D", "G", "0", Some("Cgd * V(D, G)"));
    } else {
        current(&mut d, "D", "S", "Id", None);
        current(&mut d, "G", "S", "0", Some("Cgs * V(G, S)"));
        current(&mut d, "G", "D", "0", Some("Cgd * V(G, D)"));
    }
    probe(&mut d, "Id", "Id", "A");
    probe(&mut d, "P", "vds * Id", "W");
    describe(
        d,
        "MOSFET, level 1 (Shichman-Hodges), symmetric, with gate capacitances.",
    )
}

fn jfet(id: &str, name: &str, symbol: &str, p: bool) -> ComponentDef {
    let mut d = ComponentDef::new(id, name, "Transistors", "J", symbol);
    pins(&mut d, &["D", "G", "S"]);
    d.params
        .push(ParamDef::real("Beta", 1e-3, "A/V²", "Transconductance parameter").range(1e-9, 1e3));
    d.params
        .push(ParamDef::real("Vto", -2.0, "V", "Pinch-off voltage (negative)").range(-100.0, 0.0));
    d.params
        .push(ParamDef::real("lambda", 0.0, "1/V", "Channel-length modulation").range(0.0, 1.0));
    d.params.push(
        ParamDef::real("Is", 1e-14, "A", "Gate junction saturation current").range(1e-30, 1.0),
    );
    d.params
        .push(ParamDef::real("Cgs", 5e-12, "F", "Gate-source capacitance").range(0.0, 1.0));
    d.params
        .push(ParamDef::real("Cgd", 2e-12, "F", "Gate-drain capacitance").range(0.0, 1.0));
    // n-channel voltages; a p-channel device mirrors every polarity.
    let (gs, ds, gd) = if p {
        ("V(S, G)", "V(S, D)", "V(D, G)")
    } else {
        ("V(G, S)", "V(D, S)", "V(G, D)")
    };
    d.lets.push(("vgs".into(), e(gs)));
    d.lets.push(("vds".into(), e(ds)));
    d.lets.push(("vgd".into(), e(gd)));
    // Shichman-Hodges, symmetric in drain and source.
    d.lets.push(("fwd".into(), e(
        "select(vgs - Vto <= 0, 0, select(vds < vgs - Vto, Beta * vds * (2 * (vgs - Vto) - vds), Beta * (vgs - Vto) * (vgs - Vto))) * (1 + lambda * vds)",
    )));
    d.lets.push(("rev".into(), e(
        "select(vgd - Vto <= 0, 0, select(-vds < vgd - Vto, Beta * -vds * (2 * (vgd - Vto) + vds), Beta * (vgd - Vto) * (vgd - Vto))) * (1 - lambda * vds)",
    )));
    d.lets
        .push(("Id".into(), e("select(vds >= 0, fwd, -rev) + 1e-12 * vds")));
    d.lets
        .push(("Igs".into(), e("Is * (limexp(vgs / vt) - 1)")));
    d.lets
        .push(("Igd".into(), e("Is * (limexp(vgd / vt) - 1)")));
    if p {
        current(&mut d, "S", "D", "Id", None);
        current(&mut d, "S", "G", "Igs", Some("Cgs * V(S, G)"));
        current(&mut d, "D", "G", "Igd", Some("Cgd * V(D, G)"));
        d.limits.push(LimitDef::Pn {
            a: "S".into(),
            b: "G".into(),
            vte: e("vt"),
            is: e("Is"),
        });
    } else {
        current(&mut d, "D", "S", "Id", None);
        current(&mut d, "G", "S", "Igs", Some("Cgs * V(G, S)"));
        current(&mut d, "G", "D", "Igd", Some("Cgd * V(G, D)"));
        d.limits.push(LimitDef::Pn {
            a: "G".into(),
            b: "S".into(),
            vte: e("vt"),
            is: e("Is"),
        });
    }
    probe(&mut d, "Id", "Id", "A");
    describe(d, "Junction FET, Shichman-Hodges model with gate diodes.")
}

fn opamp() -> ComponentDef {
    let mut d = ComponentDef::new("core.opamp", "Op-amp", "Analog ICs", "U", "opamp");
    pins(&mut d, &["INP", "INN", "OUT"]);
    d.params
        .push(ParamDef::real("A", 1e5, "", "Open-loop gain").range(1.0, 1e9));
    d.params
        .push(ParamDef::real("Vpos", 15.0, "V", "Positive output limit"));
    d.params
        .push(ParamDef::real("Vneg", -15.0, "V", "Negative output limit"));
    d.params
        .push(ParamDef::real("Rout", 10.0, "Ω", "Output resistance").range(0.0, 1e6));
    d.params
        .push(ParamDef::real("Rin", 1e9, "Ω", "Input resistance").range(1e3, 1e15));
    d.lets.push(("half".into(), e("(Vpos - Vneg) / 2")));
    current(&mut d, "INP", "INN", "V(INP, INN) / Rin", None);
    voltage(
        &mut d,
        "O",
        "OUT",
        "0",
        "(Vpos + Vneg) / 2 + half * tanh(A * V(INP, INN) / half) + Rout * I(O)",
        None,
    );
    probe(&mut d, "Iout", "-I(O)", "A");
    describe(
        d,
        "Op-amp with finite gain, output swing limited to the rails, referenced to ground.",
    )
}

fn comparator() -> ComponentDef {
    let mut d = ComponentDef::new(
        "core.comparator",
        "Comparator",
        "Analog ICs",
        "U",
        "comparator",
    );
    pins(&mut d, &["INP", "INN", "OUT"]);
    d.params
        .push(ParamDef::real("Vhigh", 5.0, "V", "Output high"));
    d.params
        .push(ParamDef::real("Vlow", 0.0, "V", "Output low"));
    d.params
        .push(ParamDef::real("gain", 1e4, "", "Transition gain").range(1.0, 1e9));
    voltage(
        &mut d,
        "O",
        "OUT",
        "0",
        "Vlow + (Vhigh - Vlow) * (0.5 + 0.5 * tanh(gain * V(INP, INN))) + 50 * I(O)",
        None,
    );
    describe(
        d,
        "Comparator with a smooth transition, referenced to ground.",
    )
}

fn controlled(
    id: &str,
    name: &str,
    symbol: &str,
    voltage_out: bool,
    current_ctrl: bool,
) -> ComponentDef {
    let mut d = ComponentDef::new(
        id,
        name,
        "Controlled sources",
        if voltage_out { "E" } else { "G" },
        symbol,
    );
    pins(&mut d, &["P", "N", "CP", "CN"]);
    let ctrl = if current_ctrl {
        // Sense the controlling current with a 0 V branch between CP and CN.
        voltage(&mut d, "S", "CP", "CN", "0", None);
        "I(S)"
    } else {
        "V(CP, CN)"
    };
    d.params
        .push(ParamDef::real("gain", 1.0, "", "Gain").live());
    if voltage_out {
        voltage(&mut d, "O", "P", "N", &format!("gain * {ctrl}"), None);
    } else {
        current(&mut d, "N", "P", &format!("gain * {ctrl}"), None);
    }
    describe(d, "Linear controlled source.")
}

fn voltmeter() -> ComponentDef {
    let mut d = ComponentDef::new(
        "core.voltmeter",
        "Voltmeter",
        "Instruments",
        "VM",
        "voltmeter",
    );
    pins(&mut d, &["P", "N"]);
    probe(&mut d, "V", "V(P, N)", "V");
    describe(d, "Ideal voltmeter: infinite impedance.")
}

fn ammeter() -> ComponentDef {
    let mut d = ComponentDef::new("core.ammeter", "Ammeter", "Instruments", "AM", "ammeter");
    pins(&mut d, &["P", "N"]);
    voltage(&mut d, "A", "P", "N", "0", None);
    probe(&mut d, "I", "I(A)", "A");
    describe(d, "Ideal ammeter: zero impedance.")
}

/// Every built-in definition.
pub fn definitions() -> Vec<ComponentDef> {
    vec![
        ground(),
        resistor(),
        capacitor(),
        inductor(),
        potentiometer(),
        lamp(),
        vdc(),
        battery(),
        idc(),
        vsine(),
        funcgen(),
        describe(
            diode_like(
                "core.diode",
                "Diode",
                "diode",
                Junction {
                    is: 1e-14,
                    n: 1.0,
                    bv: 100.0,
                    ibv: 1e-10,
                    cj: 2e-12,
                },
            ),
            "SPICE level-1 style junction diode with breakdown.",
        ),
        describe(
            diode_like(
                "core.zener",
                "Zener diode",
                "zener",
                Junction {
                    is: 1e-14,
                    n: 1.0,
                    bv: 5.1,
                    ibv: 1e-3,
                    cj: 2e-12,
                },
            ),
            "Zener diode; breakdown at BV.",
        ),
        describe(
            diode_like(
                "core.schottky",
                "Schottky diode",
                "schottky",
                Junction {
                    is: 1e-8,
                    n: 1.05,
                    bv: 40.0,
                    ibv: 1e-6,
                    cj: 10e-12,
                },
            ),
            "Low forward drop Schottky diode.",
        ),
        led(),
        switch_spst(),
        push_button(),
        switch_spdt(),
        bjt("core.npn", "NPN transistor", "npn", false),
        bjt("core.pnp", "PNP transistor", "pnp", true),
        mosfet("core.nmos", "N-MOSFET", "nmos", false),
        mosfet("core.pmos", "P-MOSFET", "pmos", true),
        jfet("core.njf", "N-JFET", "njf", false),
        jfet("core.pjf", "P-JFET", "pjf", true),
        opamp(),
        comparator(),
        controlled("core.vcvs", "VCVS", "vcvs", true, false),
        controlled("core.vccs", "VCCS", "vccs", false, false),
        controlled("core.ccvs", "CCVS", "ccvs", true, true),
        controlled("core.cccs", "CCCS", "cccs", false, true),
        voltmeter(),
        ammeter(),
    ]
}

/// The built-in library.
pub fn library() -> Library {
    let mut lib = Library::new();
    for d in definitions().into_iter().chain(digital::definitions()) {
        lib.insert(d);
    }
    lib
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_symbols_named() {
        let defs = definitions();
        let mut ids: Vec<_> = defs.iter().map(|d| d.id.clone()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), defs.len());
        assert!(
            defs.iter()
                .all(|d| !d.symbol.is_empty() && !d.meta.prefix.is_empty())
        );
    }
}
