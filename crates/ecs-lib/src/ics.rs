//! Standard logic ICs: the 74xx and 4000 series (spec 7). Each chip is one
//! part with all its gates or sections; pins are named after the datasheet
//! with the section number moved behind (`1A` → `A1`, `1Y` → `Y1`) so they
//! are valid names in equations. Power pins are implicit: outputs drive the
//! levels of the chosen logic family (74HC = 5 V CMOS, 74LS/74HCT = TTL
//! thresholds; the 4000 series as 5 V CMOS). Open-collector and tri-state
//! outputs float through the engine's `__oe_<pin>` states; the 4066 and 4051
//! switch analog signals through resistive branches.

use ecs_model::{
    Behavior, BranchDef, ComponentDef, EventCtx, KnownPart, Level, ParamDef, ParamKind, PinDef,
    PinKind, StateDef,
};
use std::sync::Arc;

type F = Arc<dyn Fn(&mut EventCtx<'_>) + Send + Sync>;

struct Chip {
    start: Option<F>,
    inputs: F,
}

impl Behavior for Chip {
    fn on_start(&self, ctx: &mut EventCtx<'_>) {
        match &self.start {
            Some(f) => f(ctx),
            None => (self.inputs)(ctx),
        }
    }
    fn on_inputs(&self, ctx: &mut EventCtx<'_>) {
        (self.inputs)(ctx)
    }
}

/// Symbol layout for a chip: inputs on the left, outputs on the right.
#[derive(Debug, Clone)]
pub struct IcSymbol {
    pub id: String,
    pub title: String,
    pub left: Vec<String>,
    pub right: Vec<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum Series {
    S74,
    S40,
}

struct Ic {
    def: ComponentDef,
    series: Series,
    num: String,
    left: Vec<String>,
    right: Vec<String>,
}

fn ic(series: Series, num: &str, name: &str, desc: &str) -> Ic {
    let (id, title, cat, tpd) = match series {
        Series::S74 => (
            format!("ic.74x{num}"),
            format!("74x{num}"),
            "74xx logic",
            10e-9,
        ),
        Series::S40 => (format!("ic.{num}"), num.to_string(), "4000 logic", 60e-9),
    };
    let mut d = ComponentDef::new(
        &id,
        &format!("{title} {name}"),
        cat,
        "U",
        &format!("ic:{id}"),
    );
    let mut fam = ParamDef::real(
        "family",
        0.0,
        "",
        "Logic family (74HC: 5 V CMOS, 74LS/74HCT: TTL)",
    );
    fam.kind = ParamKind::Enum(vec!["5 V CMOS".into(), "3.3 V CMOS".into(), "TTL".into()]);
    d.params.push(fam);
    d.params
        .push(ParamDef::real("tpd", tpd, "s", "Propagation delay").range(0.0, 1.0));
    d.meta.description = format!("{desc} Power pins are implicit.");
    Ic {
        def: d,
        series,
        num: num.into(),
        left: Vec::new(),
        right: Vec::new(),
    }
}

impl Ic {
    fn i(mut self, pins: &[&str]) -> Self {
        for p in pins {
            let name = p.trim_start_matches('>');
            self.def.pins.push(PinDef {
                name: name.into(),
                kind: PinKind::DigitalIn(String::new()),
            });
            self.left.push(p.to_string());
        }
        self
    }
    fn o(mut self, pins: &[&str]) -> Self {
        for p in pins {
            self.def.pins.push(PinDef {
                name: p.to_string(),
                kind: PinKind::DigitalOut(String::new()),
            });
            self.right.push(p.to_string());
        }
        self
    }
    /// Outputs that can float (tri-state or open collector).
    fn float(mut self, pins: &[String]) -> Self {
        for p in pins {
            self.def.states.push(StateDef {
                name: format!("__oe_{p}"),
                initial: 1.0,
            });
        }
        self
    }
    fn analog(mut self, pin: &str, left: bool) -> Self {
        self.def.pins.push(PinDef {
            name: pin.into(),
            kind: PinKind::Analog,
        });
        if left {
            self.left.push(pin.into());
        } else {
            self.right.push(pin.into());
        }
        self
    }
    fn state(mut self, name: &str, initial: f64) -> Self {
        self.def.states.push(StateDef {
            name: name.into(),
            initial,
        });
        self
    }
    fn eval(mut self, f: impl Fn(&mut EventCtx<'_>) + Send + Sync + 'static) -> Self {
        self.def.behavior = Some(Arc::new(Chip {
            start: None,
            inputs: Arc::new(f),
        }));
        self
    }
}

// ------------------------------------------------------------ helpers

fn hi(c: &EventCtx<'_>, p: &str) -> bool {
    c.level(p).is_high()
}
fn lo(c: &EventCtx<'_>, p: &str) -> bool {
    c.level(p) == Level::Low
}
fn out(c: &mut EventCtx<'_>, p: &str, l: Level) {
    let t = c.param("tpd");
    c.drive(p, l, t);
}
fn outb(c: &mut EventCtx<'_>, p: &str, b: bool) {
    out(c, p, Level::from_bool(b));
}
/// Tri-state output: `None` floats.
fn out3(c: &mut EventCtx<'_>, p: &str, l: Option<Level>) {
    c.set(&format!("__oe_{p}"), if l.is_some() { 1.0 } else { 0.0 });
    out(c, p, l.unwrap_or(Level::Low));
}
/// Open-collector output: low sinks, high floats.
fn out_oc(c: &mut EventCtx<'_>, p: &str, low: bool) {
    out3(c, p, if low { Some(Level::Low) } else { None });
}
/// Binary value of pins (first is the LSB); None if any is unknown.
fn word(c: &EventCtx<'_>, pins: &[String]) -> Option<u32> {
    let mut v = 0;
    for (k, p) in pins.iter().enumerate() {
        match c.level(p) {
            Level::High => v |= 1 << k,
            Level::Low => {}
            Level::X => return None,
        }
    }
    Some(v)
}
fn put(c: &mut EventCtx<'_>, pins: &[String], v: u32) {
    for (k, p) in pins.iter().enumerate() {
        outb(c, p, v >> k & 1 == 1);
    }
}
fn put_x(c: &mut EventCtx<'_>, pins: &[String]) {
    for p in pins {
        out(c, p, Level::X);
    }
}
fn names(prefix: &str, range: std::ops::RangeInclusive<u32>) -> Vec<String> {
    range.map(|k| format!("{prefix}{k}")).collect()
}
fn s(v: &[String]) -> Vec<&str> {
    v.iter().map(String::as_str).collect()
}
/// Evaluate a boolean function with unknowns: the result is known only if
/// every value of the unknown inputs gives the same answer.
fn resolve(levels: &[Level], f: &dyn Fn(&[bool]) -> bool) -> Level {
    let xs: Vec<usize> = levels
        .iter()
        .enumerate()
        .filter(|(_, l)| **l == Level::X)
        .map(|(k, _)| k)
        .collect();
    if xs.len() > 8 {
        return Level::X;
    }
    let mut b: Vec<bool> = levels.iter().map(|l| l.is_high()).collect();
    let mut res = None;
    for mask in 0..(1u32 << xs.len()) {
        for (j, &k) in xs.iter().enumerate() {
            b[k] = mask >> j & 1 == 1;
        }
        let r = f(&b);
        if res.is_some_and(|x| x != r) {
            return Level::X;
        }
        res = Some(r);
    }
    Level::from_bool(res.unwrap_or(false))
}

// ------------------------------------------------------------ gates

#[derive(Clone, Copy)]
enum G {
    And,
    Nand,
    Or,
    Nor,
    Xor,
    Xnor,
    Not,
    Buf,
}

fn g_eval(g: G, x: &[bool]) -> bool {
    match g {
        G::And => x.iter().all(|b| *b),
        G::Nand => !x.iter().all(|b| *b),
        G::Or => x.iter().any(|b| *b),
        G::Nor => !x.iter().any(|b| *b),
        G::Xor => x.iter().filter(|b| **b).count() % 2 == 1,
        G::Xnor => x.iter().filter(|b| **b).count() % 2 == 0,
        G::Not => !x[0],
        G::Buf => x[0],
    }
}

/// `count` gates of `n` inputs each. Open-collector outputs only sink.
#[allow(clippy::too_many_arguments)] // a table row: series, number, name, shape, function
fn gates(
    series: Series,
    num: &str,
    name: &str,
    count: u32,
    n: usize,
    g: G,
    oc: bool,
    desc: &str,
) -> Ic {
    const LETTERS: [&str; 8] = ["A", "B", "C", "D", "E", "F", "G", "H"];
    let mut sections: Vec<(Vec<String>, String)> = Vec::new();
    let mut c = ic(series, num, name, desc);
    for k in 1..=count {
        let ins: Vec<String> = LETTERS[..n]
            .iter()
            .map(|l| {
                if count == 1 {
                    l.to_string()
                } else {
                    format!("{l}{k}")
                }
            })
            .collect();
        let y = if count == 1 {
            "Y".to_string()
        } else {
            format!("Y{k}")
        };
        c = c.i(&s(&ins));
        sections.push((ins, y));
    }
    let ys: Vec<String> = sections.iter().map(|x| x.1.clone()).collect();
    c = c.o(&s(&ys));
    if oc {
        c = c.float(&ys);
    }
    c.eval(move |ctx| {
        for (ins, y) in &sections {
            let levels: Vec<Level> = ins.iter().map(|p| ctx.level(p)).collect();
            let l = resolve(&levels, &|b| g_eval(g, b));
            if oc {
                out_oc(ctx, y, l == Level::Low);
            } else {
                out(ctx, y, l);
            }
        }
    })
}

// ------------------------------------------------------------ 74xx

const SEGMENTS: [u8; 10] = [0x3F, 0x06, 0x5B, 0x4F, 0x66, 0x6D, 0x7D, 0x07, 0x7F, 0x6F];

fn seg_names() -> Vec<String> {
    ["a", "b", "c", "d", "e", "f", "g"]
        .iter()
        .map(|x| x.to_string())
        .collect()
}

fn s74() -> Vec<Ic> {
    use G::*;
    let t = Series::S74;
    let mut v = vec![
        gates(
            t,
            "00",
            "quad NAND",
            4,
            2,
            Nand,
            false,
            "Four 2-input NAND gates.",
        ),
        gates(
            t,
            "02",
            "quad NOR",
            4,
            2,
            Nor,
            false,
            "Four 2-input NOR gates.",
        ),
        gates(t, "04", "hex inverter", 6, 1, Not, false, "Six inverters."),
        gates(
            t,
            "06",
            "hex inverter, open collector",
            6,
            1,
            Not,
            true,
            "Six inverters with open-collector outputs: add pull-up resistors.",
        ),
        gates(
            t,
            "07",
            "hex buffer, open collector",
            6,
            1,
            Buf,
            true,
            "Six buffers with open-collector outputs: add pull-up resistors.",
        ),
        gates(
            t,
            "08",
            "quad AND",
            4,
            2,
            And,
            false,
            "Four 2-input AND gates.",
        ),
        gates(
            t,
            "10",
            "triple 3-input NAND",
            3,
            3,
            Nand,
            false,
            "Three 3-input NAND gates.",
        ),
        gates(
            t,
            "11",
            "triple 3-input AND",
            3,
            3,
            And,
            false,
            "Three 3-input AND gates.",
        ),
        gates(
            t,
            "14",
            "hex Schmitt inverter",
            6,
            1,
            Not,
            false,
            "Six Schmitt-trigger inverters.",
        ),
        gates(
            t,
            "20",
            "dual 4-input NAND",
            2,
            4,
            Nand,
            false,
            "Two 4-input NAND gates.",
        ),
        gates(
            t,
            "21",
            "dual 4-input AND",
            2,
            4,
            And,
            false,
            "Two 4-input AND gates.",
        ),
        gates(
            t,
            "27",
            "triple 3-input NOR",
            3,
            3,
            Nor,
            false,
            "Three 3-input NOR gates.",
        ),
        gates(
            t,
            "30",
            "8-input NAND",
            1,
            8,
            Nand,
            false,
            "One 8-input NAND gate.",
        ),
        gates(
            t,
            "32",
            "quad OR",
            4,
            2,
            Or,
            false,
            "Four 2-input OR gates.",
        ),
        gates(
            t,
            "86",
            "quad XOR",
            4,
            2,
            Xor,
            false,
            "Four 2-input exclusive-OR gates.",
        ),
        gates(
            t,
            "7266",
            "quad XNOR",
            4,
            2,
            Xnor,
            false,
            "Four 2-input exclusive-NOR gates.",
        ),
    ];
    // Tri-state buffers.
    for (num, active_high) in [("125", false), ("126", true)] {
        let a = names("A", 1..=4);
        let oe = names("OE", 1..=4);
        let y = names("Y", 1..=4);
        let (a2, oe2, y2) = (a.clone(), oe.clone(), y.clone());
        v.push(
            ic(
                t,
                num,
                "quad tri-state buffer",
                if active_high {
                    "Four buffers; each output floats while its OE is low."
                } else {
                    "Four buffers; each output floats while its OE is high."
                },
            )
            .i(&s(&a))
            .i(&s(&oe))
            .o(&s(&y))
            .float(&y)
            .eval(move |c| {
                for k in 0..4 {
                    let en = if active_high {
                        hi(c, &oe2[k])
                    } else {
                        lo(c, &oe2[k])
                    };
                    let l = c.level(&a2[k]);
                    out3(c, &y2[k], en.then_some(l));
                }
            }),
        );
    }
    {
        let a = names("A", 1..=8);
        let y = names("Y", 1..=8);
        let (a2, y2) = (a.clone(), y.clone());
        v.push(
            ic(t, "244", "octal tri-state buffer", "Eight buffers in two groups of four; OE1 enables Y1-Y4 and OE2 Y5-Y8 (active low).")
                .i(&s(&a))
                .i(&["OE1", "OE2"])
                .o(&s(&y))
                .float(&y)
                .eval(move |c| {
                    for k in 0..8 {
                        let en = lo(c, if k < 4 { "OE1" } else { "OE2" });
                        let l = c.level(&a2[k]);
                        out3(c, &y2[k], en.then_some(l));
                    }
                }),
        );
    }
    // 7474 dual D flip-flop with preset and clear (active low).
    v.push(
        ic(
            t,
            "74",
            "dual D flip-flop",
            "Two rising-edge D flip-flops with active-low preset and clear.",
        )
        .i(&["D1", ">CLK1", "PRE1", "CLR1", "D2", ">CLK2", "PRE2", "CLR2"])
        .o(&["Q1", "QN1", "Q2", "QN2"])
        .state("q1", 0.0)
        .state("q2", 0.0)
        .eval(|c| {
            for k in 1..=2 {
                let (pre, clr) = (lo(c, &format!("PRE{k}")), lo(c, &format!("CLR{k}")));
                let mut q = c.get(&format!("q{k}")) > 0.5;
                if pre && clr {
                    outb(c, &format!("Q{k}"), true);
                    outb(c, &format!("QN{k}"), true);
                    continue;
                } else if pre {
                    q = true;
                } else if clr {
                    q = false;
                } else if c.rising(&format!("CLK{k}")) {
                    q = hi(c, &format!("D{k}"));
                }
                c.set(&format!("q{k}"), f64::from(u8::from(q)));
                outb(c, &format!("Q{k}"), q);
                outb(c, &format!("QN{k}"), !q);
            }
        }),
    );
    // 74112 dual JK, falling edge.
    v.push(
        ic(
            t,
            "112",
            "dual JK flip-flop",
            "Two falling-edge JK flip-flops with active-low preset and clear.",
        )
        .i(&[
            "J1", "K1", ">CLK1", "PRE1", "CLR1", "J2", "K2", ">CLK2", "PRE2", "CLR2",
        ])
        .o(&["Q1", "QN1", "Q2", "QN2"])
        .state("q1", 0.0)
        .state("q2", 0.0)
        .eval(|c| {
            for k in 1..=2 {
                let (pre, clr) = (lo(c, &format!("PRE{k}")), lo(c, &format!("CLR{k}")));
                let mut q = c.get(&format!("q{k}")) > 0.5;
                if pre && clr {
                    outb(c, &format!("Q{k}"), true);
                    outb(c, &format!("QN{k}"), true);
                    continue;
                } else if pre {
                    q = true;
                } else if clr {
                    q = false;
                } else if c.falling(&format!("CLK{k}")) {
                    q = match (hi(c, &format!("J{k}")), hi(c, &format!("K{k}"))) {
                        (false, false) => q,
                        (true, false) => true,
                        (false, true) => false,
                        (true, true) => !q,
                    };
                }
                c.set(&format!("q{k}"), f64::from(u8::from(q)));
                outb(c, &format!("Q{k}"), q);
                outb(c, &format!("QN{k}"), !q);
            }
        }),
    );
    // Octal latch and register with tri-state outputs.
    for (num, name, edge) in [
        ("573", "octal transparent latch", false),
        ("574", "octal D register", true),
    ] {
        let d = names("D", 1..=8);
        let q = names("Q", 1..=8);
        let (d2, q2) = (d.clone(), q.clone());
        let clk = if edge { ">CLK" } else { "LE" };
        v.push(
            ic(t, num, name, if edge { "Eight D flip-flops loaded on the rising CLK edge; outputs float while OE is high." } else { "Eight latches, transparent while LE is high; outputs float while OE is high." })
                .i(&s(&d))
                .i(&[clk, "OE"])
                .o(&s(&q))
                .float(&q)
                .state("r", 0.0)
                .eval(move |c| {
                    let mut r = c.get("r") as u32;
                    let load = if edge { c.rising("CLK") } else { hi(c, "LE") };
                    if load && let Some(w) = word(c, &d2) {
                        r = w;
                    }
                    c.set("r", r as f64);
                    let en = lo(c, "OE");
                    for (k, q) in q2.iter().enumerate() {
                        out3(c, q, en.then_some(Level::from_bool(r >> k & 1 == 1)));
                    }
                }),
        );
    }
    // Decoders and multiplexers.
    {
        let y = names("Y", 0..=7);
        let y2 = y.clone();
        v.push(
            ic(
                t,
                "138",
                "3-to-8 decoder",
                "Output Yn goes low for input CBA = n while G1 is high and G2A, G2B are low.",
            )
            .i(&["A", "B", "C", "G1", "G2A", "G2B"])
            .o(&s(&y))
            .eval(move |c| {
                let sel = word(c, &["A".into(), "B".into(), "C".into()]);
                let en = hi(c, "G1") && lo(c, "G2A") && lo(c, "G2B");
                for k in 0..8u32 {
                    match sel {
                        Some(n) => outb(c, &y2[k as usize], !(en && n == k)),
                        None if en => out(c, &y2[k as usize], Level::X),
                        None => outb(c, &y2[k as usize], true),
                    }
                }
            }),
        );
    }
    v.push(
        ic(
            t,
            "139",
            "dual 2-to-4 decoder",
            "Two decoders: Yk_n goes low for BA = n while Gk is low.",
        )
        .i(&["A1", "B1", "G1", "A2", "B2", "G2"])
        .o(&[
            "Y1_0", "Y1_1", "Y1_2", "Y1_3", "Y2_0", "Y2_1", "Y2_2", "Y2_3",
        ])
        .eval(|c| {
            for k in 1..=2 {
                let sel = word(c, &[format!("A{k}"), format!("B{k}")]);
                let en = lo(c, &format!("G{k}"));
                for n in 0..4u32 {
                    let p = format!("Y{k}_{n}");
                    match sel {
                        Some(x) => outb(c, &p, !(en && x == n)),
                        None => out(c, &p, if en { Level::X } else { Level::High }),
                    }
                }
            }
        }),
    );
    {
        let d = names("D", 0..=7);
        let d2 = d.clone();
        v.push(
            ic(
                t,
                "151",
                "8-to-1 multiplexer",
                "Y follows the data input chosen by CBA while G is low; W is its complement.",
            )
            .i(&s(&d))
            .i(&["A", "B", "C", "G"])
            .o(&["Y", "W"])
            .eval(move |c| {
                let l = if hi(c, "G") {
                    Level::Low
                } else {
                    match word(c, &["A".into(), "B".into(), "C".into()]) {
                        Some(n) => c.level(&d2[n as usize]),
                        None => Level::X,
                    }
                };
                out(c, "Y", l);
                out(c, "W", l.invert());
            }),
        );
    }
    v.push(
        ic(
            t,
            "153",
            "dual 4-to-1 multiplexer",
            "Two multiplexers sharing the select inputs A, B; each enabled by its G (active low).",
        )
        .i(&[
            "C0_1", "C1_1", "C2_1", "C3_1", "G1", "C0_2", "C1_2", "C2_2", "C3_2", "G2", "A", "B",
        ])
        .o(&["Y1", "Y2"])
        .eval(|c| {
            let sel = word(c, &["A".into(), "B".into()]);
            for k in 1..=2 {
                let l = if hi(c, &format!("G{k}")) {
                    Level::Low
                } else {
                    sel.map_or(Level::X, |n| c.level(&format!("C{n}_{k}")))
                };
                out(c, &format!("Y{k}"), l);
            }
        }),
    );
    v.push(
        ic(
            t,
            "157",
            "quad 2-to-1 multiplexer",
            "Yn follows An while S is low and Bn while S is high; G high forces the outputs low.",
        )
        .i(&["A1", "B1", "A2", "B2", "A3", "B3", "A4", "B4", "S", "G"])
        .o(&["Y1", "Y2", "Y3", "Y4"])
        .eval(|c| {
            for k in 1..=4 {
                let l = if hi(c, "G") {
                    Level::Low
                } else {
                    match c.level("S") {
                        Level::Low => c.level(&format!("A{k}")),
                        Level::High => c.level(&format!("B{k}")),
                        Level::X => Level::X,
                    }
                };
                out(c, &format!("Y{k}"), l);
            }
        }),
    );
    // BCD to 7-segment.
    for (num, oc) in [("47", true), ("48", false)] {
        let segs = seg_names();
        let segs2 = segs.clone();
        let mut chip = ic(
            t,
            num,
            "BCD to 7-segment decoder",
            if oc {
                "Active-low open-collector segment outputs for a common-anode display (with resistors); LT low lights every segment, BI low blanks."
            } else {
                "Active-high segment outputs for a common-cathode display; LT low lights every segment, BI low blanks."
            },
        )
        .i(&["A", "B", "C", "D", "LT", "BI"])
        .o(&s(&segs));
        if oc {
            chip = chip.float(&segs);
        }
        v.push(chip.eval(move |c| {
            let n = word(c, &["A".into(), "B".into(), "C".into(), "D".into()]);
            let pattern: u32 = if lo(c, "LT") {
                0x7F
            } else if lo(c, "BI") {
                0
            } else {
                match n {
                    Some(n) if n < 10 => SEGMENTS[n as usize] as u32,
                    _ => 0,
                }
            };
            for (k, p) in segs2.iter().enumerate() {
                let on = pattern >> k & 1 == 1;
                if oc {
                    out_oc(c, p, on);
                } else {
                    outb(c, p, on);
                }
            }
        }));
    }
    // Arithmetic.
    v.push(
        ic(t, "283", "4-bit adder", "S = A + B + C0 with carry out C4.")
            .i(&["A1", "A2", "A3", "A4", "B1", "B2", "B3", "B4", "C0"])
            .o(&["S1", "S2", "S3", "S4", "C4"])
            .eval(|c| {
                let a = word(c, &names("A", 1..=4));
                let b = word(c, &names("B", 1..=4));
                let ci = word(c, &["C0".into()]);
                let outs = names("S", 1..=4);
                match (a, b, ci) {
                    (Some(a), Some(b), Some(ci)) => {
                        let sum = a + b + ci;
                        put(c, &outs, sum & 15);
                        outb(c, "C4", sum > 15);
                    }
                    _ => {
                        put_x(c, &outs);
                        out(c, "C4", Level::X);
                    }
                }
            }),
    );
    v.push(
        ic(
            t,
            "85",
            "4-bit magnitude comparator",
            "Compares A with B; equal words pass the cascade inputs through.",
        )
        .i(&[
            "A0", "A1", "A2", "A3", "B0", "B1", "B2", "B3", "IAGB", "IAEB", "IALB",
        ])
        .o(&["AGBO", "AEBO", "ALBO"])
        .eval(|c| {
            let a = word(c, &names("A", 0..=3));
            let b = word(c, &names("B", 0..=3));
            let (g, e, l) = match (a, b) {
                (Some(a), Some(b)) if a > b => (true, false, false),
                (Some(a), Some(b)) if a < b => (false, false, true),
                (Some(_), Some(_)) => {
                    let (ig, ie, il) = (hi(c, "IAGB"), hi(c, "IAEB"), hi(c, "IALB"));
                    if ie {
                        (false, true, false)
                    } else {
                        match (ig, il) {
                            (true, false) => (true, false, false),
                            (false, true) => (false, false, true),
                            (true, true) => (false, false, false),
                            (false, false) => (true, false, true),
                        }
                    }
                }
                _ => {
                    put_x(c, &["AGBO".into(), "AEBO".into(), "ALBO".into()]);
                    return;
                }
            };
            outb(c, "AGBO", g);
            outb(c, "AEBO", e);
            outb(c, "ALBO", l);
        }),
    );
    // Counters.
    for (num, sync_clear) in [("161", false), ("163", true)] {
        v.push(
            ic(
                t,
                num,
                "synchronous 4-bit counter",
                if sync_clear {
                    "Counts on the rising CLK edge while ENP and ENT are high; LOAD low loads DCBA, CLR low clears on the next edge; RCO marks 15 while ENT is high."
                } else {
                    "Counts on the rising CLK edge while ENP and ENT are high; LOAD low loads DCBA on the edge, CLR low clears at once; RCO marks 15 while ENT is high."
                },
            )
            .i(&[">CLK", "CLR", "LOAD", "ENP", "ENT", "A", "B", "C", "D"])
            .o(&["QA", "QB", "QC", "QD", "RCO"])
            .state("n", 0.0)
            .eval(move |c| {
                let mut n = c.get("n") as u32;
                let clr = lo(c, "CLR");
                if clr && !sync_clear {
                    n = 0;
                } else if c.rising("CLK") {
                    if clr {
                        n = 0;
                    } else if lo(c, "LOAD") {
                        n = word(c, &["A".into(), "B".into(), "C".into(), "D".into()]).unwrap_or(n);
                    } else if hi(c, "ENP") && hi(c, "ENT") {
                        n = (n + 1) & 15;
                    }
                }
                c.set("n", n as f64);
                put(c, &["QA".into(), "QB".into(), "QC".into(), "QD".into()], n);
                outb(c, "RCO", hi(c, "ENT") && n == 15);
            }),
        );
    }
    v.push(
        ic(t, "193", "synchronous up/down counter", "Counts up on a rising UP edge (DOWN high) and down on a rising DOWN edge (UP high); CLR high clears, LOAD low loads DCBA; CO and BO pulse low at 15 and 0.")
            .i(&[">UP", ">DOWN", "CLR", "LOAD", "A", "B", "C", "D"])
            .o(&["QA", "QB", "QC", "QD", "CO", "BO"])
            .state("n", 0.0)
            .eval(|c| {
                let mut n = c.get("n") as u32;
                if hi(c, "CLR") {
                    n = 0;
                } else if lo(c, "LOAD") {
                    n = word(c, &["A".into(), "B".into(), "C".into(), "D".into()]).unwrap_or(n);
                } else if c.rising("UP") && hi(c, "DOWN") {
                    n = (n + 1) & 15;
                } else if c.rising("DOWN") && hi(c, "UP") {
                    n = (n + 15) & 15;
                }
                c.set("n", n as f64);
                put(c, &["QA".into(), "QB".into(), "QC".into(), "QD".into()], n);
                outb(c, "CO", !(n == 15 && lo(c, "UP")));
                outb(c, "BO", !(n == 0 && lo(c, "DOWN")));
            }),
    );
    v.push(
        ic(t, "90", "decade counter", "QA divides CKA by 2 and QB-QD divide CKB by 5 (falling edges); join QA to CKB to count 0-9. R01 and R02 high reset, R91 and R92 high set 9.")
            .i(&[">CKA", ">CKB", "R01", "R02", "R91", "R92"])
            .o(&["QA", "QB", "QC", "QD"])
            .state("a", 0.0)
            .state("b", 0.0)
            .eval(|c| {
                let (mut a, mut b) = (c.get("a") as u32, c.get("b") as u32);
                if hi(c, "R91") && hi(c, "R92") {
                    a = 1;
                    b = 4;
                } else if hi(c, "R01") && hi(c, "R02") {
                    a = 0;
                    b = 0;
                } else {
                    if c.falling("CKA") {
                        a ^= 1;
                    }
                    if c.falling("CKB") {
                        b = (b + 1) % 5;
                    }
                }
                c.set("a", a as f64);
                c.set("b", b as f64);
                outb(c, "QA", a == 1);
                put(c, &["QB".into(), "QC".into(), "QD".into()], b);
            }),
    );
    v.push(
        ic(t, "93", "4-bit binary counter", "QA divides CKA by 2 and QB-QD divide CKB by 8 (falling edges); join QA to CKB to count 0-15. R01 and R02 high reset.")
            .i(&[">CKA", ">CKB", "R01", "R02"])
            .o(&["QA", "QB", "QC", "QD"])
            .state("a", 0.0)
            .state("b", 0.0)
            .eval(|c| {
                let (mut a, mut b) = (c.get("a") as u32, c.get("b") as u32);
                if hi(c, "R01") && hi(c, "R02") {
                    a = 0;
                    b = 0;
                } else {
                    if c.falling("CKA") {
                        a ^= 1;
                    }
                    if c.falling("CKB") {
                        b = (b + 1) & 7;
                    }
                }
                c.set("a", a as f64);
                c.set("b", b as f64);
                outb(c, "QA", a == 1);
                put(c, &["QB".into(), "QC".into(), "QD".into()], b);
            }),
    );
    // Shift registers.
    {
        let q = vec!["QA", "QB", "QC", "QD", "QE", "QF", "QG", "QH"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        let q2 = q.clone();
        v.push(
            ic(t, "164", "8-bit serial-in parallel-out shift register", "On a rising CLK edge A AND B enters QA and the rest shift toward QH; CLR low clears.")
                .i(&["A", "B", ">CLK", "CLR"])
                .o(&s(&q))
                .state("r", 0.0)
                .eval(move |c| {
                    let mut r = c.get("r") as u32;
                    if lo(c, "CLR") {
                        r = 0;
                    } else if c.rising("CLK") {
                        r = (r << 1 | u32::from(hi(c, "A") && hi(c, "B"))) & 0xFF;
                    }
                    c.set("r", r as f64);
                    put(c, &q2, r);
                }),
        );
    }
    {
        let q = vec!["QA", "QB", "QC", "QD", "QE", "QF", "QG", "QH"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        let q2 = q.clone();
        v.push(
            ic(t, "595", "8-bit shift register with output latches", "SER shifts in on a rising SRCLK edge; a rising RCLK edge copies the shift register to QA-QH; SRCLR low clears; OE high floats the outputs; QHS is the serial output.")
                .i(&["SER", ">SRCLK", ">RCLK", "SRCLR", "OE"])
                .o(&s(&q))
                .o(&["QHS"])
                .float(&q)
                .state("sr", 0.0)
                .state("lat", 0.0)
                .eval(move |c| {
                    let mut sr = c.get("sr") as u32;
                    let mut lat = c.get("lat") as u32;
                    // RCLK first: tied clocks latch the previous shift state, as the chip does.
                    if c.rising("RCLK") {
                        lat = sr;
                    }
                    if lo(c, "SRCLR") {
                        sr = 0;
                    } else if c.rising("SRCLK") {
                        sr = (sr << 1 | u32::from(hi(c, "SER"))) & 0xFF;
                    }
                    c.set("sr", sr as f64);
                    c.set("lat", lat as f64);
                    let en = lo(c, "OE");
                    for (k, p) in q2.iter().enumerate() {
                        out3(c, p, en.then_some(Level::from_bool(lat >> k & 1 == 1)));
                    }
                    outb(c, "QHS", sr >> 7 & 1 == 1);
                }),
        );
    }
    {
        let d = vec!["A", "B", "C", "D", "E", "F", "G", "H"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        let d2 = d.clone();
        v.push(
            ic(t, "165", "8-bit parallel-in serial-out shift register", "SH_LD low loads A-H; with SH_LD high each rising CLK edge (CLK_INH low) shifts toward QH, SER entering at A.")
                .i(&s(&d))
                .i(&["SH_LD", ">CLK", "CLK_INH", "SER"])
                .o(&["QH", "QHN"])
                .state("r", 0.0)
                .eval(move |c| {
                    let mut r = c.get("r") as u32;
                    if lo(c, "SH_LD") {
                        r = word(c, &d2).unwrap_or(r);
                    } else if c.rising("CLK") && lo(c, "CLK_INH") {
                        r = (r << 1 | u32::from(hi(c, "SER"))) & 0xFF;
                    }
                    c.set("r", r as f64);
                    let qh = r >> 7 & 1 == 1;
                    outb(c, "QH", qh);
                    outb(c, "QHN", !qh);
                }),
        );
    }
    v
}

// ------------------------------------------------------------ 4000 series

fn s40() -> Vec<Ic> {
    use G::*;
    let t = Series::S40;
    let mut v = vec![
        gates(
            t,
            "4001",
            "quad NOR",
            4,
            2,
            Nor,
            false,
            "Four 2-input NOR gates.",
        ),
        gates(
            t,
            "4002",
            "dual 4-input NOR",
            2,
            4,
            Nor,
            false,
            "Two 4-input NOR gates.",
        ),
        gates(
            t,
            "4011",
            "quad NAND",
            4,
            2,
            Nand,
            false,
            "Four 2-input NAND gates.",
        ),
        gates(
            t,
            "4012",
            "dual 4-input NAND",
            2,
            4,
            Nand,
            false,
            "Two 4-input NAND gates.",
        ),
        gates(
            t,
            "4023",
            "triple 3-input NAND",
            3,
            3,
            Nand,
            false,
            "Three 3-input NAND gates.",
        ),
        gates(
            t,
            "4025",
            "triple 3-input NOR",
            3,
            3,
            Nor,
            false,
            "Three 3-input NOR gates.",
        ),
        gates(
            t,
            "4049",
            "hex inverting buffer",
            6,
            1,
            Not,
            false,
            "Six inverting buffers.",
        ),
        gates(
            t,
            "4050",
            "hex buffer",
            6,
            1,
            Buf,
            false,
            "Six non-inverting buffers.",
        ),
        gates(
            t,
            "4068",
            "8-input NAND",
            1,
            8,
            Nand,
            false,
            "One 8-input NAND gate.",
        ),
        gates(
            t,
            "4069",
            "hex inverter",
            6,
            1,
            Not,
            false,
            "Six inverters.",
        ),
        gates(
            t,
            "4070",
            "quad XOR",
            4,
            2,
            Xor,
            false,
            "Four 2-input exclusive-OR gates.",
        ),
        gates(
            t,
            "4071",
            "quad OR",
            4,
            2,
            Or,
            false,
            "Four 2-input OR gates.",
        ),
        gates(
            t,
            "4073",
            "triple 3-input AND",
            3,
            3,
            And,
            false,
            "Three 3-input AND gates.",
        ),
        gates(
            t,
            "4075",
            "triple 3-input OR",
            3,
            3,
            Or,
            false,
            "Three 3-input OR gates.",
        ),
        gates(
            t,
            "4077",
            "quad XNOR",
            4,
            2,
            Xnor,
            false,
            "Four 2-input exclusive-NOR gates.",
        ),
        gates(
            t,
            "4078",
            "8-input NOR",
            1,
            8,
            Nor,
            false,
            "One 8-input NOR gate.",
        ),
        gates(
            t,
            "4081",
            "quad AND",
            4,
            2,
            And,
            false,
            "Four 2-input AND gates.",
        ),
        gates(
            t,
            "4082",
            "dual 4-input AND",
            2,
            4,
            And,
            false,
            "Two 4-input AND gates.",
        ),
        gates(
            t,
            "4093",
            "quad Schmitt NAND",
            4,
            2,
            Nand,
            false,
            "Four 2-input Schmitt-trigger NAND gates.",
        ),
    ];
    v.push(
        ic(
            t,
            "4013",
            "dual D flip-flop",
            "Two rising-edge D flip-flops with active-high set and reset.",
        )
        .i(&["D1", ">CLK1", "S1", "R1", "D2", ">CLK2", "S2", "R2"])
        .o(&["Q1", "QN1", "Q2", "QN2"])
        .state("q1", 0.0)
        .state("q2", 0.0)
        .eval(|c| {
            for k in 1..=2 {
                let (set, rst) = (hi(c, &format!("S{k}")), hi(c, &format!("R{k}")));
                let mut q = c.get(&format!("q{k}")) > 0.5;
                if set && rst {
                    outb(c, &format!("Q{k}"), true);
                    outb(c, &format!("QN{k}"), true);
                    continue;
                } else if set {
                    q = true;
                } else if rst {
                    q = false;
                } else if c.rising(&format!("CLK{k}")) {
                    q = hi(c, &format!("D{k}"));
                }
                c.set(&format!("q{k}"), f64::from(u8::from(q)));
                outb(c, &format!("Q{k}"), q);
                outb(c, &format!("QN{k}"), !q);
            }
        }),
    );
    v.push(
        ic(
            t,
            "4027",
            "dual JK flip-flop",
            "Two rising-edge JK flip-flops with active-high set and reset.",
        )
        .i(&[
            "J1", "K1", ">CLK1", "S1", "R1", "J2", "K2", ">CLK2", "S2", "R2",
        ])
        .o(&["Q1", "QN1", "Q2", "QN2"])
        .state("q1", 0.0)
        .state("q2", 0.0)
        .eval(|c| {
            for k in 1..=2 {
                let (set, rst) = (hi(c, &format!("S{k}")), hi(c, &format!("R{k}")));
                let mut q = c.get(&format!("q{k}")) > 0.5;
                if set && rst {
                    outb(c, &format!("Q{k}"), true);
                    outb(c, &format!("QN{k}"), true);
                    continue;
                } else if set {
                    q = true;
                } else if rst {
                    q = false;
                } else if c.rising(&format!("CLK{k}")) {
                    q = match (hi(c, &format!("J{k}")), hi(c, &format!("K{k}"))) {
                        (false, false) => q,
                        (true, false) => true,
                        (false, true) => false,
                        (true, true) => !q,
                    };
                }
                c.set(&format!("q{k}"), f64::from(u8::from(q)));
                outb(c, &format!("Q{k}"), q);
                outb(c, &format!("QN{k}"), !q);
            }
        }),
    );
    // Johnson counters with decoded outputs.
    for (num, n, name) in [
        ("4017", 10u32, "decade counter with 10 decoded outputs"),
        ("4022", 8, "octal counter with 8 decoded outputs"),
    ] {
        let q = names("Q", 0..=(n - 1));
        let q2 = q.clone();
        v.push(
            ic(t, num, name, &format!("Advances on each rising CLK edge while INH is low; RST high returns to Q0. CO is high for the first half of the cycle (0-{}).", n / 2 - 1))
                .i(&[">CLK", "INH", "RST"])
                .o(&s(&q))
                .o(&["CO"])
                .state("n", 0.0)
                .eval(move |c| {
                    let mut k = c.get("n") as u32;
                    if hi(c, "RST") {
                        k = 0;
                    } else if c.rising("CLK") && lo(c, "INH") {
                        k = (k + 1) % n;
                    }
                    c.set("n", k as f64);
                    for (j, p) in q2.iter().enumerate() {
                        outb(c, p, j as u32 == k);
                    }
                    outb(c, "CO", k < n / 2);
                }),
        );
    }
    // Ripple binary counters.
    for (num, bits, shown) in [
        (
            "4020",
            14u32,
            vec![1u32, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14],
        ),
        ("4040", 12, (1..=12).collect()),
        ("4024", 7, (1..=7).collect()),
    ] {
        let q: Vec<String> = shown.iter().map(|k| format!("Q{k}")).collect();
        let q2 = q.clone();
        let shown2 = shown.clone();
        v.push(
            ic(
                t,
                num,
                &format!("{bits}-stage ripple counter"),
                &format!(
                    "Counts falling CLK edges; Qn divides by 2^n. RST high clears. {} outputs.",
                    q.len()
                ),
            )
            .i(&[">CLK", "RST"])
            .o(&s(&q))
            .state("n", 0.0)
            .eval(move |c| {
                let mut k = c.get("n") as u32;
                if hi(c, "RST") {
                    k = 0;
                } else if c.falling("CLK") {
                    k = (k + 1) & ((1 << bits) - 1);
                }
                c.set("n", k as f64);
                for (p, b) in q2.iter().zip(&shown2) {
                    outb(c, p, k >> (b - 1) & 1 == 1);
                }
            }),
        );
    }
    for (num, modulo, name) in [
        ("4518", 10u32, "dual BCD up counter"),
        ("4520", 16, "dual 4-bit binary up counter"),
    ] {
        v.push(
            ic(t, num, name, "Each half counts on a rising CLK edge while EN is high (or a falling EN edge while CLK is low); RST high clears.")
                .i(&[">CLK1", "EN1", "RST1", ">CLK2", "EN2", "RST2"])
                .o(&["Q1_0", "Q1_1", "Q1_2", "Q1_3", "Q2_0", "Q2_1", "Q2_2", "Q2_3"])
                .state("n1", 0.0)
                .state("n2", 0.0)
                .eval(move |c| {
                    for k in 1..=2 {
                        let mut n = c.get(&format!("n{k}")) as u32;
                        let (clk, en) = (format!("CLK{k}"), format!("EN{k}"));
                        if hi(c, &format!("RST{k}")) {
                            n = 0;
                        } else if (c.rising(&clk) && hi(c, &en)) || (c.falling(&en) && lo(c, &clk)) {
                            n = (n + 1) % modulo;
                        }
                        c.set(&format!("n{k}"), n as f64);
                        put(c, &(0..4).map(|b| format!("Q{k}_{b}")).collect::<Vec<_>>(), n);
                    }
                }),
        );
    }
    {
        let q = names("Q", 0..=9);
        let q2 = q.clone();
        v.push(
            ic(
                t,
                "4028",
                "BCD to decimal decoder",
                "Output Qn is high for DCBA = n (0-9); 10-15 leave all low.",
            )
            .i(&["A", "B", "C", "D"])
            .o(&s(&q))
            .eval(move |c| {
                match word(c, &["A".into(), "B".into(), "C".into(), "D".into()]) {
                    Some(n) => {
                        for (k, p) in q2.iter().enumerate() {
                            outb(c, p, k as u32 == n);
                        }
                    }
                    None => put_x(c, &q2),
                }
            }),
        );
    }
    {
        let segs = seg_names();
        let segs2 = segs.clone();
        v.push(
            ic(t, "4511", "BCD to 7-segment latch/decoder", "Active-high outputs for a common-cathode display; LT low lights every segment, BI low blanks, LE high holds the last value; 10-15 blank.")
                .i(&["A", "B", "C", "D", "LT", "BI", "LE"])
                .o(&s(&segs))
                .state("held", 0.0)
                .eval(move |c| {
                    let mut n = c.get("held") as u32;
                    if !hi(c, "LE")
                        && let Some(w) = word(c, &["A".into(), "B".into(), "C".into(), "D".into()])
                    {
                        n = w;
                    }
                    c.set("held", n as f64);
                    let pattern: u32 = if lo(c, "LT") {
                        0x7F
                    } else if lo(c, "BI") || n > 9 {
                        0
                    } else {
                        SEGMENTS[n as usize] as u32
                    };
                    for (k, p) in segs2.iter().enumerate() {
                        outb(c, p, pattern >> k & 1 == 1);
                    }
                }),
        );
    }
    {
        let q = names("Q", 1..=8);
        let q2 = q.clone();
        v.push(
            ic(t, "4094", "8-stage shift-and-store register", "DATA shifts in on a rising CLK edge; the store latch follows while STR is high; OE low floats Q1-Q8; QS is the serial output.")
                .i(&["DATA", ">CLK", "STR", "OE"])
                .o(&s(&q))
                .o(&["QS"])
                .float(&q)
                .state("sr", 0.0)
                .state("lat", 0.0)
                .eval(move |c| {
                    let mut sr = c.get("sr") as u32;
                    let mut lat = c.get("lat") as u32;
                    if c.rising("CLK") {
                        sr = (sr << 1 | u32::from(hi(c, "DATA"))) & 0xFF;
                    }
                    if hi(c, "STR") {
                        lat = sr;
                    }
                    c.set("sr", sr as f64);
                    c.set("lat", lat as f64);
                    let en = hi(c, "OE");
                    for (k, p) in q2.iter().enumerate() {
                        out3(c, p, en.then_some(Level::from_bool(lat >> k & 1 == 1)));
                    }
                    outb(c, "QS", sr >> 7 & 1 == 1);
                }),
        );
    }
    v.push(
        ic(
            t,
            "4008",
            "4-bit full adder",
            "S = A + B + CIN with carry out COUT.",
        )
        .i(&["A1", "A2", "A3", "A4", "B1", "B2", "B3", "B4", "CIN"])
        .o(&["S1", "S2", "S3", "S4", "COUT"])
        .eval(|c| {
            let a = word(c, &names("A", 1..=4));
            let b = word(c, &names("B", 1..=4));
            let ci = word(c, &["CIN".into()]);
            let outs = names("S", 1..=4);
            match (a, b, ci) {
                (Some(a), Some(b), Some(ci)) => {
                    let sum = a + b + ci;
                    put(c, &outs, sum & 15);
                    outb(c, "COUT", sum > 15);
                }
                _ => {
                    put_x(c, &outs);
                    out(c, "COUT", Level::X);
                }
            }
        }),
    );
    // Analog switches: resistive branches whose state the control inputs set.
    for (num, ron) in [("4066", 270.0), ("4016", 400.0)] {
        let mut chip = ic(
            t,
            num,
            "quad bilateral switch",
            &format!(
                "Four analog switches: Xn and Yn connect (about {ron} Ω at 5 V) while Cn is high."
            ),
        );
        for k in 1..=4 {
            chip = chip.analog(&format!("X{k}"), true);
        }
        chip = chip.i(&["C1", "C2", "C3", "C4"]);
        for k in 1..=4 {
            chip = chip.analog(&format!("Y{k}"), false);
            chip = chip.state(&format!("on{k}"), 0.0);
            chip.def.branches.push(BranchDef::Current {
                name: None,
                a: format!("X{k}"),
                b: format!("Y{k}"),
                i: crate::e(&format!(
                    "V(X{k}, Y{k}) * select(on{k} > 0.5, 1 / Ron, 1 / Roff)"
                )),
                q: None,
            });
        }
        chip.def
            .params
            .push(ParamDef::real("Ron", ron, "Ω", "On resistance").range(1e-3, 1e6));
        chip.def
            .params
            .push(ParamDef::real("Roff", 1e11, "Ω", "Off resistance").range(1e3, 1e15));
        v.push(chip.eval(|c| {
            for k in 1..=4 {
                let on = hi(c, &format!("C{k}"));
                c.set(&format!("on{k}"), f64::from(u8::from(on)));
            }
        }));
    }
    {
        let mut chip = ic(
            t,
            "4051",
            "8-channel analog multiplexer",
            "Connects COM to channel Xn chosen by CBA (about 120 Ω); INH high opens every channel.",
        );
        for k in 0..8 {
            chip = chip.analog(&format!("X{k}"), true);
        }
        chip = chip
            .i(&["A", "B", "C", "INH"])
            .analog("COM", false)
            .state("sel", -1.0);
        for k in 0..8 {
            chip.def.branches.push(BranchDef::Current {
                name: None,
                a: format!("X{k}"),
                b: "COM".into(),
                i: crate::e(&format!(
                    "V(X{k}, COM) * select(abs(sel - {k}) < 0.5, 1 / Ron, 1 / Roff)"
                )),
                q: None,
            });
        }
        chip.def
            .params
            .push(ParamDef::real("Ron", 120.0, "Ω", "On resistance").range(1e-3, 1e6));
        chip.def
            .params
            .push(ParamDef::real("Roff", 1e11, "Ω", "Off resistance").range(1e3, 1e15));
        v.push(chip.eval(|c| {
            let sel = if hi(c, "INH") {
                -1.0
            } else {
                word(c, &["A".into(), "B".into(), "C".into()]).map_or(-1.0, |n| n as f64)
            };
            c.set("sel", sel);
        }));
    }
    v
}

fn all() -> Vec<Ic> {
    let mut v = s74();
    v.extend(s40());
    v
}

/// Every logic IC definition.
pub fn definitions() -> Vec<ComponentDef> {
    all().into_iter().map(|c| c.def).collect()
}

/// Symbol layouts for the ICs (the app registers boxed symbols).
pub fn symbols() -> Vec<IcSymbol> {
    all()
        .into_iter()
        .map(|c| IcSymbol {
            id: c.def.symbol.clone(),
            title: match c.series {
                Series::S74 => format!("74x{}", c.num),
                Series::S40 => c.num.clone(),
            },
            left: c.left,
            right: c.right,
        })
        .collect()
}

/// Part numbers: 74HC (5 V CMOS, the default), 74HCT and 74LS (TTL inputs);
/// CD4xxxB, HEF4xxxB and MC14xxxB for the 4000 series.
pub fn part_numbers() -> Vec<KnownPart> {
    let mut v = Vec::new();
    for c in all() {
        let id = c.def.id.clone();
        let note = c.def.meta.name.clone();
        let mk = |number: String, family: f64, default: bool| KnownPart {
            number,
            def: id.clone(),
            params: vec![("family".into(), family)],
            note: note.clone(),
            default,
        };
        match c.series {
            Series::S74 => {
                v.push(mk(format!("74HC{}", c.num), 0.0, true));
                v.push(mk(format!("74HCT{}", c.num), 2.0, false));
                v.push(mk(format!("74LS{}", c.num), 2.0, false));
            }
            Series::S40 => {
                let tail = &c.num[1..];
                v.push(mk(format!("CD{}B", c.num), 0.0, true));
                v.push(mk(format!("HEF{}B", c.num), 0.0, false));
                v.push(mk(format!("MC14{tail}B"), 0.0, false));
            }
        }
    }
    v
}
