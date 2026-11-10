//! Vector symbols (spec 9.3): lines, arcs, circles, polygons, text and pin
//! points, in symbol-local coordinates on a 20-unit grid. ANSI style.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{OnceLock, RwLock};

/// Schematic grid pitch in world units. Pins sit on multiples of it.
pub const GRID: f64 = 20.0;

/// A 2-D point.
pub type P2 = [f64; 2];

/// A drawing primitive.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Prim {
    Line {
        a: P2,
        b: P2,
    },
    Polyline {
        points: Vec<P2>,
        closed: bool,
        fill: bool,
    },
    Circle {
        c: P2,
        r: f64,
        fill: bool,
    },
    /// Arc from angle `a0` to `a1` (radians, y down).
    Arc {
        c: P2,
        r: f64,
        a0: f64,
        a1: f64,
    },
    Text {
        at: P2,
        text: String,
        size: f64,
    },
}

/// A pin connection point.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SymPin {
    pub name: String,
    pub at: P2,
}

/// A symbol (`.sym.json` shape).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Symbol {
    pub id: String,
    /// Always drawn.
    pub prims: Vec<Prim>,
    /// Drawn when the part's control state is 0 (switch open). Empty if none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prims_off: Vec<Prim>,
    /// Drawn when the part's control state is 1 (switch closed).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prims_on: Vec<Prim>,
    pub pins: Vec<SymPin>,
    /// Bounding box `[min, max]` used for selection.
    pub bbox: [P2; 2],
    /// Region that glows (LED, lamp): centre and radius.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glow: Option<(P2, f64)>,
    /// Regions filled when a probe of the part is above 0.5 (7-segment):
    /// probe name and polygon.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lit: Vec<(String, Vec<P2>)>,
}

fn line(a: P2, b: P2) -> Prim {
    Prim::Line { a, b }
}
fn poly(points: &[P2], closed: bool, fill: bool) -> Prim {
    Prim::Polyline {
        points: points.to_vec(),
        closed,
        fill,
    }
}
fn circle(c: P2, r: f64, fill: bool) -> Prim {
    Prim::Circle { c, r, fill }
}
fn arc(c: P2, r: f64, a0: f64, a1: f64) -> Prim {
    Prim::Arc { c, r, a0, a1 }
}
fn text(at: P2, t: &str, size: f64) -> Prim {
    Prim::Text {
        at,
        text: t.into(),
        size,
    }
}
fn pin(name: &str, at: P2) -> SymPin {
    SymPin {
        name: name.into(),
        at,
    }
}

fn sym(id: &str, prims: Vec<Prim>, pins: Vec<SymPin>, bbox: [P2; 2]) -> Symbol {
    Symbol {
        id: id.into(),
        prims,
        prims_off: Vec::new(),
        prims_on: Vec::new(),
        pins,
        bbox,
        glow: None,
        lit: Vec::new(),
    }
}

fn two_pin_h(id: &str, mut body: Vec<Prim>, half: f64, h: f64) -> Symbol {
    body.push(line([-40.0, 0.0], [-half, 0.0]));
    body.push(line([half, 0.0], [40.0, 0.0]));
    sym(
        id,
        body,
        vec![pin("A", [-40.0, 0.0]), pin("B", [40.0, 0.0])],
        [[-40.0, -h], [40.0, h]],
    )
}

fn zigzag() -> Vec<P2> {
    let mut pts = vec![[-24.0, 0.0]];
    for k in 0..6 {
        let x = -20.0 + 8.0 * k as f64;
        pts.push([x, if k % 2 == 0 { -8.0 } else { 8.0 }]);
    }
    pts.push([24.0, 0.0]);
    pts
}

fn source_circle(id: &str, mut inner: Vec<Prim>, p: &str, n: &str) -> Symbol {
    inner.push(circle([0.0, 0.0], 18.0, false));
    inner.push(line([0.0, -40.0], [0.0, -18.0]));
    inner.push(line([0.0, 18.0], [0.0, 40.0]));
    sym(
        id,
        inner,
        vec![pin(p, [0.0, -40.0]), pin(n, [0.0, 40.0])],
        [[-20.0, -40.0], [20.0, 40.0]],
    )
}

fn diode_body(kind: &str) -> Vec<Prim> {
    let mut v = vec![
        poly(&[[-10.0, -11.0], [-10.0, 11.0], [10.0, 0.0]], true, true),
        line([-40.0, 0.0], [-10.0, 0.0]),
        line([10.0, 0.0], [40.0, 0.0]),
    ];
    match kind {
        "zener" => v.push(poly(
            &[[6.0, -15.0], [10.0, -11.0], [10.0, 11.0], [14.0, 15.0]],
            false,
            false,
        )),
        "schottky" => v.push(poly(
            &[
                [14.0, -7.0],
                [14.0, -11.0],
                [10.0, -11.0],
                [10.0, 11.0],
                [6.0, 11.0],
                [6.0, 7.0],
            ],
            false,
            false,
        )),
        _ => v.push(line([10.0, -11.0], [10.0, 11.0])),
    }
    v
}

fn arrow(from: P2, to: P2) -> Vec<Prim> {
    let d = [to[0] - from[0], to[1] - from[1]];
    let len = (d[0] * d[0] + d[1] * d[1]).sqrt();
    let u = [d[0] / len, d[1] / len];
    let nrm = [-u[1], u[0]];
    let base = [to[0] - u[0] * 6.0, to[1] - u[1] * 6.0];
    vec![
        line(from, base),
        poly(
            &[
                to,
                [base[0] + nrm[0] * 3.5, base[1] + nrm[1] * 3.5],
                [base[0] - nrm[0] * 3.5, base[1] - nrm[1] * 3.5],
            ],
            true,
            true,
        ),
    ]
}

fn build() -> BTreeMap<String, Symbol> {
    let mut m = BTreeMap::new();
    let mut add = |s: Symbol| {
        m.insert(s.id.clone(), s);
    };

    add(two_pin_h(
        "resistor",
        vec![poly(&zigzag(), false, false)],
        24.0,
        10.0,
    ));
    add(two_pin_h(
        "capacitor",
        vec![
            line([-5.0, -16.0], [-5.0, 16.0]),
            line([5.0, -16.0], [5.0, 16.0]),
        ],
        5.0,
        16.0,
    ));
    add(two_pin_h(
        "inductor",
        (0..4)
            .map(|k| {
                arc(
                    [-18.0 + 12.0 * k as f64, 0.0],
                    6.0,
                    std::f64::consts::PI,
                    2.0 * std::f64::consts::PI,
                )
            })
            .collect(),
        24.0,
        8.0,
    ));
    // IEC 60617 style: a rectangle (`set_resistor_style`).
    let rect = || {
        poly(
            &[[-24.0, -8.0], [24.0, -8.0], [24.0, 8.0], [-24.0, 8.0]],
            true,
            false,
        )
    };
    add(two_pin_h("resistor_iec", vec![rect()], 24.0, 10.0));
    for (id, body) in [
        ("potentiometer", poly(&zigzag(), false, false)),
        ("potentiometer_iec", rect()),
    ] {
        let mut s = two_pin_h(id, vec![body], 24.0, 10.0);
        s.prims.extend(arrow(
            [0.0, -40.0],
            [0.0, if id.ends_with("_iec") { -8.0 } else { -10.0 }],
        ));
        s.pins = vec![
            pin("A", [-40.0, 0.0]),
            pin("W", [0.0, -40.0]),
            pin("B", [40.0, 0.0]),
        ];
        s.bbox = [[-40.0, -40.0], [40.0, 10.0]];
        add(s);
    }
    {
        let r = 13.0;
        let d = r * std::f64::consts::FRAC_1_SQRT_2;
        let mut s = two_pin_h(
            "lamp",
            vec![
                circle([0.0, 0.0], r, false),
                line([-d, -d], [d, d]),
                line([-d, d], [d, -d]),
            ],
            r,
            r,
        );
        s.glow = Some(([0.0, 0.0], 22.0));
        add(s);
    }
    add(source_circle(
        "vdc",
        vec![
            line([-5.0, -8.0], [5.0, -8.0]),
            line([0.0, -13.0], [0.0, -3.0]),
            line([-5.0, 8.0], [5.0, 8.0]),
        ],
        "P",
        "N",
    ));
    add(sym(
        "battery",
        vec![
            line([0.0, -40.0], [0.0, -6.0]),
            line([-16.0, -6.0], [16.0, -6.0]),
            line([-8.0, 0.0], [8.0, 0.0]),
            line([-16.0, 6.0], [16.0, 6.0]),
            line([-8.0, 12.0], [8.0, 12.0]),
            line([0.0, 12.0], [0.0, 40.0]),
            text([12.0, -16.0], "+", 10.0),
        ],
        vec![pin("P", [0.0, -40.0]), pin("N", [0.0, 40.0])],
        [[-16.0, -40.0], [16.0, 40.0]],
    ));
    add(source_circle(
        "idc",
        arrow([0.0, 11.0], [0.0, -11.0]),
        "P",
        "N",
    ));
    {
        let pts: Vec<P2> = (0..=24)
            .map(|k| {
                let x = -11.0 + 22.0 * k as f64 / 24.0;
                [x, -7.0 * (std::f64::consts::PI * (x + 11.0) / 11.0).sin()]
            })
            .collect();
        add(source_circle(
            "vac",
            vec![poly(&pts, false, false)],
            "P",
            "N",
        ));
    }
    add(source_circle(
        "funcgen",
        vec![poly(
            &[
                [-11.0, 5.0],
                [-6.0, 5.0],
                [-6.0, -5.0],
                [0.0, -5.0],
                [0.0, 5.0],
                [6.0, 5.0],
                [6.0, -5.0],
                [11.0, -5.0],
            ],
            false,
            false,
        )],
        "P",
        "N",
    ));
    add(sym(
        "ground",
        vec![
            line([0.0, 0.0], [0.0, 10.0]),
            line([-14.0, 10.0], [14.0, 10.0]),
            line([-9.0, 15.0], [9.0, 15.0]),
            line([-4.0, 20.0], [4.0, 20.0]),
        ],
        vec![pin("G", [0.0, 0.0])],
        [[-14.0, -2.0], [14.0, 22.0]],
    ));
    for kind in ["diode", "zener", "schottky"] {
        add(sym(
            kind,
            diode_body(kind),
            vec![pin("A", [-40.0, 0.0]), pin("K", [40.0, 0.0])],
            [[-40.0, -15.0], [40.0, 15.0]],
        ));
    }
    {
        let mut body = diode_body("diode");
        body.extend(arrow([2.0, -14.0], [12.0, -26.0]));
        body.extend(arrow([10.0, -12.0], [20.0, -24.0]));
        let mut s = sym(
            "led",
            body,
            vec![pin("A", [-40.0, 0.0]), pin("K", [40.0, 0.0])],
            [[-40.0, -26.0], [40.0, 12.0]],
        );
        s.glow = Some(([0.0, 0.0], 20.0));
        add(s);
    }
    {
        let mut s = sym(
            "switch",
            vec![
                line([-40.0, 0.0], [-20.0, 0.0]),
                line([20.0, 0.0], [40.0, 0.0]),
                circle([-20.0, 0.0], 3.0, true),
                circle([20.0, 0.0], 3.0, true),
            ],
            vec![pin("A", [-40.0, 0.0]), pin("B", [40.0, 0.0])],
            [[-40.0, -18.0], [40.0, 6.0]],
        );
        s.prims_off = vec![line([-20.0, 0.0], [17.0, -15.0])];
        s.prims_on = vec![line([-20.0, 0.0], [20.0, -3.0])];
        add(s);
    }
    {
        let mut s = sym(
            "pushbutton",
            vec![
                line([-40.0, 0.0], [-20.0, 0.0]),
                line([20.0, 0.0], [40.0, 0.0]),
                circle([-20.0, 0.0], 3.0, true),
                circle([20.0, 0.0], 3.0, true),
            ],
            vec![pin("A", [-40.0, 0.0]), pin("B", [40.0, 0.0])],
            [[-40.0, -24.0], [40.0, 6.0]],
        );
        s.prims_off = vec![
            line([-22.0, -10.0], [22.0, -10.0]),
            line([0.0, -10.0], [0.0, -24.0]),
        ];
        s.prims_on = vec![
            line([-22.0, -4.0], [22.0, -4.0]),
            line([0.0, -4.0], [0.0, -18.0]),
        ];
        add(s);
    }
    {
        let mut s = sym(
            "spdt",
            vec![
                line([-40.0, 0.0], [-20.0, 0.0]),
                line([20.0, -20.0], [40.0, -20.0]),
                line([20.0, 20.0], [40.0, 20.0]),
                circle([-20.0, 0.0], 3.0, true),
                circle([20.0, -20.0], 3.0, true),
                circle([20.0, 20.0], 3.0, true),
            ],
            vec![
                pin("C", [-40.0, 0.0]),
                pin("A", [40.0, -20.0]),
                pin("B", [40.0, 20.0]),
            ],
            [[-40.0, -24.0], [40.0, 24.0]],
        );
        s.prims_off = vec![line([-20.0, 0.0], [18.0, -18.0])];
        s.prims_on = vec![line([-20.0, 0.0], [18.0, 18.0])];
        add(s);
    }
    for (id, pnp) in [("npn", false), ("pnp", true)] {
        let mut body = vec![
            circle([4.0, 0.0], 24.0, false),
            line([-40.0, 0.0], [-10.0, 0.0]),
            line([-10.0, -15.0], [-10.0, 15.0]),
            line([-10.0, -7.0], [20.0, -24.0]),
            line([20.0, -24.0], [20.0, -40.0]),
            line([20.0, 24.0], [20.0, 40.0]),
        ];
        if pnp {
            body.push(line([20.0, 24.0], [2.0, 13.8]));
            body.extend(arrow([10.0, 18.3], [-9.0, 7.5]));
        } else {
            body.extend(arrow([-10.0, 7.0], [20.0, 24.0]));
        }
        add(sym(
            id,
            body,
            vec![
                pin("C", [20.0, -40.0]),
                pin("B", [-40.0, 0.0]),
                pin("E", [20.0, 40.0]),
            ],
            [[-40.0, -40.0], [28.0, 40.0]],
        ));
    }
    for (id, p) in [("nmos", false), ("pmos", true)] {
        let mut body = vec![
            line([-40.0, 0.0], [-14.0, 0.0]),
            line([-14.0, -14.0], [-14.0, 14.0]),
            line([-8.0, -16.0], [-8.0, -8.0]),
            line([-8.0, -4.0], [-8.0, 4.0]),
            line([-8.0, 8.0], [-8.0, 16.0]),
            line([-8.0, -12.0], [20.0, -12.0]),
            line([20.0, -12.0], [20.0, -40.0]),
            line([-8.0, 12.0], [20.0, 12.0]),
            line([20.0, 12.0], [20.0, 40.0]),
            line([-8.0, 0.0], [20.0, 0.0]),
            line([20.0, 0.0], [20.0, 12.0]),
        ];
        if p {
            body.extend(arrow([-6.0, 0.0], [10.0, 0.0]));
        } else {
            body.extend(arrow([10.0, 0.0], [-6.0, 0.0]));
        }
        add(sym(
            id,
            body,
            vec![
                pin("D", [20.0, -40.0]),
                pin("G", [-40.0, 0.0]),
                pin("S", [20.0, 40.0]),
            ],
            [[-40.0, -40.0], [24.0, 40.0]],
        ));
    }
    for (id, glyph) in [("opamp", ""), ("comparator", "⎍")] {
        let mut body = vec![
            poly(&[[-30.0, -40.0], [-30.0, 40.0], [40.0, 0.0]], true, false),
            line([-60.0, -20.0], [-30.0, -20.0]),
            line([-60.0, 20.0], [-30.0, 20.0]),
            line([40.0, 0.0], [60.0, 0.0]),
            line([-25.0, -20.0], [-17.0, -20.0]),
            line([-25.0, 20.0], [-17.0, 20.0]),
            line([-21.0, 16.0], [-21.0, 24.0]),
        ];
        if !glyph.is_empty() {
            body.push(poly(
                &[[-6.0, 6.0], [2.0, 6.0], [2.0, -6.0], [10.0, -6.0]],
                false,
                false,
            ));
        }
        add(sym(
            id,
            body,
            vec![
                pin("INP", [-60.0, 20.0]),
                pin("INN", [-60.0, -20.0]),
                pin("OUT", [60.0, 0.0]),
            ],
            [[-60.0, -40.0], [60.0, 40.0]],
        ));
    }
    for (id, current_out) in [
        ("vcvs", false),
        ("vccs", true),
        ("ccvs", false),
        ("cccs", true),
    ] {
        let mut body = vec![
            poly(
                &[[0.0, -18.0], [18.0, 0.0], [0.0, 18.0], [-18.0, 0.0]],
                true,
                false,
            ),
            line([0.0, -40.0], [0.0, -18.0]),
            line([0.0, 18.0], [0.0, 40.0]),
            line([-40.0, -20.0], [-30.0, -20.0]),
            line([-40.0, 20.0], [-30.0, 20.0]),
            text(
                [-34.0, -8.0],
                if id.starts_with("cc") { "I" } else { "V" },
                9.0,
            ),
        ];
        if current_out {
            body.extend(arrow([0.0, 9.0], [0.0, -9.0]));
        } else {
            body.push(line([-4.0, -7.0], [4.0, -7.0]));
            body.push(line([0.0, -11.0], [0.0, -3.0]));
            body.push(line([-4.0, 7.0], [4.0, 7.0]));
        }
        add(sym(
            id,
            body,
            vec![
                pin("P", [0.0, -40.0]),
                pin("N", [0.0, 40.0]),
                pin("CP", [-40.0, -20.0]),
                pin("CN", [-40.0, 20.0]),
            ],
            [[-40.0, -40.0], [20.0, 40.0]],
        ));
    }
    for (id, letter) in [("voltmeter", "V"), ("ammeter", "A")] {
        let mut s = two_pin_h(
            id,
            vec![
                circle([0.0, 0.0], 16.0, false),
                text([0.0, 0.0], letter, 16.0),
                text([-26.0, -8.0], "+", 9.0),
            ],
            16.0,
            16.0,
        );
        s.pins = vec![pin("P", [-40.0, 0.0]), pin("N", [40.0, 0.0])];
        add(s);
    }
    // Logic gates (ANSI): inputs A (-40,-20) and B (-40,20), output Y (40,0).
    let and_body = |out: f64| -> Vec<Prim> {
        let mut v = vec![
            poly(
                &[[0.0, -20.0], [-20.0, -20.0], [-20.0, 20.0], [0.0, 20.0]],
                false,
                false,
            ),
            arc(
                [0.0, 0.0],
                20.0,
                -std::f64::consts::FRAC_PI_2,
                std::f64::consts::FRAC_PI_2,
            ),
        ];
        v.push(line([out, 0.0], [40.0, 0.0]));
        v
    };
    let or_body = |out: f64, xor: bool| -> Vec<Prim> {
        let curve = |x0: f64| -> Vec<P2> {
            (0..=12)
                .map(|k| {
                    let t = -1.0 + 2.0 * k as f64 / 12.0;
                    [x0 + 8.0 * (1.0 - t * t), 20.0 * t]
                })
                .collect()
        };
        let top: Vec<P2> = (0..=12)
            .map(|k| {
                let t = k as f64 / 12.0;
                [-22.0 + 42.0 * t, -20.0 + 20.0 * t * t]
            })
            .collect();
        let bottom: Vec<P2> = (0..=12)
            .map(|k| {
                let t = k as f64 / 12.0;
                [-22.0 + 42.0 * t, 20.0 - 20.0 * t * t]
            })
            .collect();
        let mut v = vec![
            poly(&curve(-22.0), false, false),
            poly(&top, false, false),
            poly(&bottom, false, false),
            line([out, 0.0], [40.0, 0.0]),
        ];
        if xor {
            v.push(poly(&curve(-28.0), false, false));
        }
        v
    };
    let gate_pins = || {
        vec![
            pin("A", [-40.0, -20.0]),
            pin("B", [-40.0, 20.0]),
            pin("Y", [40.0, 0.0]),
        ]
    };
    let leads = |v: &mut Vec<Prim>, x: f64| {
        v.push(line([-40.0, -20.0], [x, -20.0]));
        v.push(line([-40.0, 20.0], [x, 20.0]));
    };
    for (id, kind, bubble) in [
        ("and", 0, false),
        ("nand", 0, true),
        ("or", 1, false),
        ("nor", 1, true),
        ("xor", 2, false),
        ("xnor", 2, true),
    ] {
        let out = if bubble { 28.0 } else { 20.0 };
        let mut body = if kind == 0 {
            and_body(out)
        } else {
            or_body(out, kind == 2)
        };
        leads(&mut body, if kind == 0 { -20.0 } else { -17.0 });
        if bubble {
            body.push(circle([24.0, 0.0], 4.0, false));
        }
        add(sym(id, body, gate_pins(), [[-40.0, -22.0], [40.0, 22.0]]));
    }
    for (id, bubble) in [("not", true), ("buf", false), ("tribuf", false)] {
        let mut body = vec![
            poly(&[[-20.0, -16.0], [-20.0, 16.0], [16.0, 0.0]], true, false),
            line([-40.0, 0.0], [-20.0, 0.0]),
        ];
        let mut pins = vec![pin("A", [-40.0, 0.0]), pin("Y", [40.0, 0.0])];
        if bubble {
            body.push(circle([20.0, 0.0], 4.0, false));
            body.push(line([24.0, 0.0], [40.0, 0.0]));
        } else {
            body.push(line([16.0, 0.0], [40.0, 0.0]));
        }
        if id == "tribuf" {
            body.push(line([0.0, 40.0], [0.0, 8.0]));
            pins.insert(1, pin("EN", [0.0, 40.0]));
        }
        add(sym(
            id,
            body,
            pins,
            [
                [-40.0, -18.0],
                [40.0, if id == "tribuf" { 40.0 } else { 18.0 }],
            ],
        ));
    }
    // IC boxes.
    add(boxed("dff", "D FF", &["D", ">CLK"], &["Q", "QN"]));
    add(boxed(
        "jkff",
        "JK FF",
        &["J", ">CLK", "K"],
        &["Q", "", "QN"],
    ));
    add(boxed("tff", "T FF", &["T", ">CLK"], &["Q", "QN"]));
    add(boxed(
        "counter4",
        "CTR4",
        &[">CLK", "RST"],
        &["Q0", "Q1", "Q2", "Q3"],
    ));
    add(boxed(
        "shift4",
        "SR4",
        &["D", ">CLK"],
        &["Q0", "Q1", "Q2", "Q3"],
    ));
    add(boxed("mux2", "MUX", &["A", "B", "S"], &["Y"]));
    add(boxed(
        "bcd7seg",
        "BCD→7",
        &["A", "B", "C", "D"],
        &["a", "b", "c", "d", "e", "f", "g"],
    ));
    add(boxed(
        "ne555",
        "555",
        &["TRIG", "THR", "RST", "DIS"],
        &["VCC", "OUT", "GND"],
    ));
    {
        let mut s = sym(
            "logic_toggle",
            vec![
                poly(
                    &[[-20.0, -14.0], [16.0, -14.0], [16.0, 14.0], [-20.0, 14.0]],
                    true,
                    false,
                ),
                line([16.0, 0.0], [40.0, 0.0]),
            ],
            vec![pin("OUT", [40.0, 0.0])],
            [[-22.0, -16.0], [40.0, 16.0]],
        );
        s.prims_off = vec![text([-2.0, 0.0], "L", 14.0)];
        s.prims_on = vec![text([-2.0, 0.0], "H", 14.0)];
        add(s);
    }
    add(sym(
        "clock",
        vec![
            poly(
                &[[-24.0, -16.0], [16.0, -16.0], [16.0, 16.0], [-24.0, 16.0]],
                true,
                false,
            ),
            poly(
                &[
                    [-18.0, 6.0],
                    [-12.0, 6.0],
                    [-12.0, -6.0],
                    [-4.0, -6.0],
                    [-4.0, 6.0],
                    [4.0, 6.0],
                    [4.0, -6.0],
                    [10.0, -6.0],
                ],
                false,
                false,
            ),
            line([16.0, 0.0], [40.0, 0.0]),
        ],
        vec![pin("OUT", [40.0, 0.0])],
        [[-26.0, -18.0], [40.0, 18.0]],
    ));
    {
        let mut s = sym(
            "logic_probe",
            vec![
                line([-40.0, 0.0], [-12.0, 0.0]),
                circle([0.0, 0.0], 12.0, false),
            ],
            vec![pin("IN", [-40.0, 0.0])],
            [[-40.0, -14.0], [14.0, 14.0]],
        );
        s.glow = Some(([0.0, 0.0], 18.0));
        add(s);
    }
    {
        // 7-segment display: inputs a..g on the left, segments lit by probes seg_a..seg_g.
        let names = ["a", "b", "c", "d", "e", "f", "g"];
        let mut prims = vec![poly(
            &[[-20.0, -70.0], [40.0, -70.0], [40.0, 70.0], [-20.0, 70.0]],
            true,
            false,
        )];
        let mut pins = Vec::new();
        for (k, n) in names.iter().enumerate() {
            let y = -60.0 + 20.0 * k as f64;
            pins.push(pin(n, [-40.0, y]));
            prims.push(line([-40.0, y], [-20.0, y]));
            prims.push(text([-13.0, y], n, 8.0));
        }
        let seg = |x0: f64, y0: f64, x1: f64, y1: f64| -> Vec<P2> {
            let w = 2.6;
            if (y0 - y1).abs() < 1e-9 {
                vec![
                    [x0 + w, y0 - w],
                    [x1 - w, y0 - w],
                    [x1, y0],
                    [x1 - w, y0 + w],
                    [x0 + w, y0 + w],
                    [x0, y0],
                ]
            } else {
                vec![
                    [x0, y0 + w],
                    [x0 + w, y0 + 2.0 * w],
                    [x0 + w, y1 - 2.0 * w],
                    [x0, y1 - w],
                    [x0 - w, y1 - 2.0 * w],
                    [x0 - w, y0 + 2.0 * w],
                ]
            }
        };
        let (l, r, t, m, b) = (2.0, 30.0, -44.0, 0.0, 44.0);
        let segs = [
            seg(l, t, r, t),
            seg(r, t, r, m),
            seg(r, m, r, b),
            seg(l, b, r, b),
            seg(l, m, l, b),
            seg(l, t, l, m),
            seg(l, m, r, m),
        ];
        let mut s = sym("sevenseg", prims, pins, [[-40.0, -72.0], [42.0, 72.0]]);
        s.lit = names
            .iter()
            .zip(segs)
            .map(|(n, p)| (format!("seg_{n}"), p))
            .collect();
        add(s);
    }
    {
        // Oscilloscope: a screen with a trace; CH1..CH4 on the left.
        let mut prims = vec![
            poly(
                &[[-20.0, -50.0], [70.0, -50.0], [70.0, 50.0], [-20.0, 50.0]],
                true,
                false,
            ),
            poly(
                &[[-10.0, -40.0], [60.0, -40.0], [60.0, 20.0], [-10.0, 20.0]],
                true,
                false,
            ),
            poly(
                &[
                    [-6.0, 0.0],
                    [6.0, 0.0],
                    [12.0, -24.0],
                    [24.0, 12.0],
                    [34.0, -14.0],
                    [44.0, 0.0],
                    [56.0, 0.0],
                ],
                false,
                false,
            ),
            circle([10.0, 36.0], 5.0, false),
            circle([30.0, 36.0], 5.0, false),
            circle([50.0, 36.0], 5.0, false),
        ];
        let mut pins = Vec::new();
        for (k, n) in ["CH1", "CH2", "CH3", "CH4"].iter().enumerate() {
            let y = -40.0 + 20.0 * k as f64;
            pins.push(pin(n, [-40.0, y]));
            prims.push(line([-40.0, y], [-20.0, y]));
        }
        add(sym("scope", prims, pins, [[-40.0, -52.0], [72.0, 52.0]]));
    }
    for (id, p) in [("njf", false), ("pjf", true)] {
        let mut body = vec![
            circle([4.0, 0.0], 24.0, false),
            line([-10.0, -16.0], [-10.0, 16.0]),
            line([-10.0, -10.0], [20.0, -10.0]),
            line([20.0, -10.0], [20.0, -40.0]),
            line([-10.0, 10.0], [20.0, 10.0]),
            line([20.0, 10.0], [20.0, 40.0]),
        ];
        if p {
            body.push(line([-40.0, 0.0], [-10.0, 0.0]));
            body.extend(arrow([-12.0, 0.0], [-30.0, 0.0]));
        } else {
            body.extend(arrow([-40.0, 0.0], [-11.0, 0.0]));
        }
        add(sym(
            id,
            body,
            vec![
                pin("D", [20.0, -40.0]),
                pin("G", [-40.0, 0.0]),
                pin("S", [20.0, 40.0]),
            ],
            [[-40.0, -40.0], [28.0, 40.0]],
        ));
    }
    add(sym(
        "box",
        vec![poly(
            &[[-30.0, -30.0], [30.0, -30.0], [30.0, 30.0], [-30.0, 30.0]],
            true,
            false,
        )],
        Vec::new(),
        [[-30.0, -30.0], [30.0, 30.0]],
    ));
    m
}

/// A labelled IC box: pins on the left and right on the 20-unit grid. A pin
/// name starting with `>` gets a clock wedge; an empty name leaves a gap.
pub fn boxed(id: &str, title: &str, left: &[&str], right: &[&str]) -> Symbol {
    let rows = left.len().max(right.len()).max(1);
    let top = -((rows as f64 - 1.0) / 2.0).floor() * GRID;
    let bottom = top + (rows as f64 - 1.0) * GRID;
    let (x0, x1) = (-30.0, 30.0);
    let mut prims = vec![poly(
        &[
            [x0, top - 16.0],
            [x1, top - 16.0],
            [x1, bottom + 16.0],
            [x0, bottom + 16.0],
        ],
        true,
        false,
    )];
    let mut pins = Vec::new();
    let clean = |n: &str| n.trim_start_matches('>').to_string();
    for (k, n) in left.iter().enumerate() {
        if n.is_empty() {
            continue;
        }
        let y = top + GRID * k as f64;
        pins.push(pin(&clean(n), [-60.0, y]));
        prims.push(line([-60.0, y], [x0, y]));
        if n.starts_with('>') {
            prims.push(poly(
                &[[x0, y - 5.0], [x0 + 7.0, y], [x0, y + 5.0]],
                false,
                false,
            ));
            prims.push(text([x0 + 18.0, y], &clean(n), 8.0));
        } else {
            prims.push(text([x0 + 12.0, y], n, 8.0));
        }
    }
    for (k, n) in right.iter().enumerate() {
        if n.is_empty() {
            continue;
        }
        let y = top + GRID * k as f64;
        pins.push(pin(n, [60.0, y]));
        prims.push(line([x1, y], [60.0, y]));
        prims.push(text([x1 - 12.0, y], n, 8.0));
    }
    prims.push(text([0.0, top - 24.0], title, 9.0));
    sym(
        id,
        prims,
        pins,
        [[-60.0, top - 30.0], [60.0, bottom + 18.0]],
    )
}

static DYNAMIC: OnceLock<RwLock<BTreeMap<String, &'static Symbol>>> = OnceLock::new();

/// Register a symbol at run time (imported subcircuits, script components).
/// Re-registering an id replaces it; the old symbol is leaked, which is fine
/// for the small number of reloads in a session.
pub fn register(symbol: Symbol) {
    let map = DYNAMIC.get_or_init(|| RwLock::new(BTreeMap::new()));
    if let Ok(mut m) = map.write() {
        m.insert(symbol.id.clone(), Box::leak(Box::new(symbol)));
    }
}

/// The built-in symbol library, keyed by symbol id.
pub fn library() -> &'static BTreeMap<String, Symbol> {
    static LIB: OnceLock<BTreeMap<String, Symbol>> = OnceLock::new();
    LIB.get_or_init(build)
}

/// Whether a symbol id is known (built in or registered).
pub fn exists(id: &str) -> bool {
    library().contains_key(id)
        || DYNAMIC
            .get()
            .and_then(|m| m.read().ok())
            .is_some_and(|m| m.contains_key(id))
}

static IEC_RESISTORS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Draw resistors and potentiometers as IEC rectangles (`true`) or
/// ANSI/IEEE zigzags. Pins are the same, so schematics are unaffected.
pub fn set_resistor_style(iec: bool) {
    IEC_RESISTORS.store(iec, std::sync::atomic::Ordering::Relaxed);
}

/// Whether resistors are drawn as rectangles.
pub fn resistor_style_iec() -> bool {
    IEC_RESISTORS.load(std::sync::atomic::Ordering::Relaxed)
}

/// A symbol by id, falling back to a plain box.
pub fn get(id: &str) -> &'static Symbol {
    if matches!(id, "resistor" | "potentiometer") && resistor_style_iec() {
        return &library()[&format!("{id}_iec")];
    }
    if let Some(m) = DYNAMIC.get()
        && let Ok(m) = m.read()
        && let Some(s) = m.get(id)
    {
        return s;
    }
    let lib = library();
    lib.get(id).unwrap_or_else(|| &lib["box"])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_are_on_grid() {
        for s in library().values() {
            for p in &s.pins {
                assert!(
                    (p.at[0] / GRID).fract() == 0.0 && (p.at[1] / GRID).fract() == 0.0,
                    "{} pin {} off grid",
                    s.id,
                    p.name
                );
            }
        }
    }

    #[test]
    fn symbols_serialize_as_sym_json() {
        let json = serde_json::to_string(get("resistor")).unwrap();
        let back: Symbol = serde_json::from_str(&json).unwrap();
        assert_eq!(&back, get("resistor"));
    }
}
