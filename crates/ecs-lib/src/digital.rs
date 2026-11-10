//! Digital parts (spec section 7, phase P5): behaviors written in Rust
//! against the same contract JavaScript components use.

use crate::{describe, e, probe};
use ecs_model::{
    Behavior, BranchDef, ComponentDef, ControlDef, ControlEvent, EventCtx, Level, ParamDef,
    ParamKind, PinDef, PinKind, StateDef,
};
use std::sync::Arc;

type Hook = fn(&mut EventCtx<'_>);

/// A behavior made of plain functions.
struct Fns {
    start: Option<Hook>,
    inputs: Option<Hook>,
    timer: Option<fn(&mut EventCtx<'_>, u32)>,
    control: Option<fn(&mut EventCtx<'_>, &ControlEvent)>,
}

impl Behavior for Fns {
    fn on_start(&self, ctx: &mut EventCtx<'_>) {
        match self.start {
            Some(f) => f(ctx),
            None => self.on_inputs(ctx),
        }
    }
    fn on_inputs(&self, ctx: &mut EventCtx<'_>) {
        if let Some(f) = self.inputs {
            f(ctx)
        }
    }
    fn on_timer(&self, ctx: &mut EventCtx<'_>, tag: u32) {
        if let Some(f) = self.timer {
            f(ctx, tag)
        }
    }
    fn on_control(&self, ctx: &mut EventCtx<'_>, c: &ControlEvent) {
        match self.control {
            Some(f) => f(ctx, c),
            None => {
                if let ControlEvent::SetState { state, value } = c {
                    ctx.set(state, *value);
                }
            }
        }
    }
}

fn behavior(inputs: Hook) -> Arc<dyn Behavior> {
    Arc::new(Fns {
        start: None,
        inputs: Some(inputs),
        timer: None,
        control: None,
    })
}

fn new(id: &str, name: &str, symbol: &str, prefix: &str) -> ComponentDef {
    let mut d = ComponentDef::new(id, name, "Digital", prefix, symbol);
    let mut fam = ParamDef::real("family", 0.0, "", "Logic family");
    fam.kind = ParamKind::Enum(vec!["5 V CMOS".into(), "3.3 V CMOS".into(), "TTL".into()]);
    d.params.push(fam);
    d.params
        .push(ParamDef::real("tpd", 10e-9, "s", "Propagation delay").range(0.0, 1.0));
    d
}

fn ins(d: &mut ComponentDef, names: &[&str]) {
    for n in names {
        d.pins.push(PinDef {
            name: n.to_string(),
            kind: PinKind::DigitalIn(String::new()),
        });
    }
}

fn outs(d: &mut ComponentDef, names: &[&str]) {
    for n in names {
        d.pins.push(PinDef {
            name: n.to_string(),
            kind: PinKind::DigitalOut(String::new()),
        });
    }
}

fn state(d: &mut ComponentDef, name: &str, initial: f64) {
    d.states.push(StateDef {
        name: name.into(),
        initial,
    });
}

/// Two-input gate with X propagation; dominant inputs decide despite X.
fn gate2(ctx: &mut EventCtx<'_>, f: fn(bool, bool) -> bool, dominant: Option<(bool, bool)>) {
    let (a, b) = (ctx.level("A"), ctx.level("B"));
    let out = match (a, b) {
        (Level::X, _) | (_, Level::X) => match dominant {
            Some((input, output))
                if a == Level::from_bool(input) || b == Level::from_bool(input) =>
            {
                Level::from_bool(output)
            }
            _ => Level::X,
        },
        _ => Level::from_bool(f(a.is_high(), b.is_high())),
    };
    let tpd = ctx.param("tpd");
    ctx.drive("Y", out, tpd);
}

fn gate(id: &str, name: &str, symbol: &str, f: Hook) -> ComponentDef {
    let mut d = new(id, name, symbol, "U");
    ins(&mut d, &["A", "B"]);
    outs(&mut d, &["Y"]);
    d.behavior = Some(behavior(f));
    describe(d, &format!("{name} gate."))
}

fn not_gate(id: &str, name: &str, symbol: &str, invert: bool) -> ComponentDef {
    let mut d = new(id, name, symbol, "U");
    ins(&mut d, &["A"]);
    outs(&mut d, &["Y"]);
    d.behavior = Some(behavior(if invert {
        |c| {
            let (l, t) = (c.level("A").invert(), c.param("tpd"));
            c.drive("Y", l, t)
        }
    } else {
        |c| {
            let (l, t) = (c.level("A"), c.param("tpd"));
            c.drive("Y", l, t)
        }
    }));
    describe(d, if invert { "Inverter." } else { "Buffer." })
}

fn tristate() -> ComponentDef {
    let mut d = new("core.tribuf", "Tri-state buffer", "tribuf", "U");
    ins(&mut d, &["A", "EN"]);
    outs(&mut d, &["Y"]);
    // The engine scales the output resistance by __oe_Y (1 enabled, 0 off).
    state(&mut d, "__oe_Y", 1.0);
    d.behavior = Some(behavior(|c| {
        let en = c.level("EN").is_high();
        c.set("__oe_Y", if en { 1.0 } else { 0.0 });
        let (l, t) = (c.level("A"), c.param("tpd"));
        c.drive("Y", l, t);
    }));
    describe(d, "Buffer whose output floats while EN is low.")
}

fn qout(c: &mut EventCtx<'_>, q: bool) {
    c.set("q", if q { 1.0 } else { 0.0 });
    let t = c.param("tpd");
    c.drive("Q", Level::from_bool(q), t);
    c.drive("QN", Level::from_bool(!q), t);
}

fn flipflops() -> Vec<ComponentDef> {
    let mut d = new("core.dff", "D flip-flop", "dff", "U");
    ins(&mut d, &["D", "CLK"]);
    outs(&mut d, &["Q", "QN"]);
    state(&mut d, "q", 0.0);
    d.behavior = Some(Arc::new(Fns {
        start: Some(|c| {
            let q = c.get("q") > 0.5;
            qout(c, q)
        }),
        inputs: Some(|c| {
            if c.rising("CLK") {
                let q = c.level("D").is_high();
                qout(c, q);
            }
        }),
        timer: None,
        control: None,
    }));
    let dff = describe(d, "Rising-edge D flip-flop.");

    let mut d = new("core.jkff", "JK flip-flop", "jkff", "U");
    ins(&mut d, &["J", "K", "CLK"]);
    outs(&mut d, &["Q", "QN"]);
    state(&mut d, "q", 0.0);
    d.behavior = Some(Arc::new(Fns {
        start: Some(|c| {
            let q = c.get("q") > 0.5;
            qout(c, q)
        }),
        inputs: Some(|c| {
            if c.rising("CLK") {
                let q = c.get("q") > 0.5;
                let (j, k) = (c.level("J").is_high(), c.level("K").is_high());
                let next = match (j, k) {
                    (false, false) => q,
                    (true, false) => true,
                    (false, true) => false,
                    (true, true) => !q,
                };
                qout(c, next);
            }
        }),
        timer: None,
        control: None,
    }));
    let jk = describe(d, "Rising-edge JK flip-flop.");

    let mut d = new("core.tff", "T flip-flop", "tff", "U");
    ins(&mut d, &["T", "CLK"]);
    outs(&mut d, &["Q", "QN"]);
    state(&mut d, "q", 0.0);
    d.behavior = Some(Arc::new(Fns {
        start: Some(|c| {
            let q = c.get("q") > 0.5;
            qout(c, q)
        }),
        inputs: Some(|c| {
            if c.rising("CLK") && c.level("T").is_high() {
                let q = c.get("q") > 0.5;
                qout(c, !q);
            }
        }),
        timer: None,
        control: None,
    }));
    vec![dff, jk, describe(d, "Rising-edge toggle flip-flop.")]
}

fn bits(c: &mut EventCtx<'_>, names: &[&str], value: u32) {
    let t = c.param("tpd");
    for (k, n) in names.iter().enumerate() {
        c.drive(n, Level::from_bool(value >> k & 1 == 1), t);
    }
}

fn counter() -> ComponentDef {
    let mut d = new("core.counter4", "4-bit counter", "counter4", "U");
    ins(&mut d, &["CLK", "RST"]);
    outs(&mut d, &["Q0", "Q1", "Q2", "Q3"]);
    state(&mut d, "count", 0.0);
    d.behavior = Some(Arc::new(Fns {
        start: Some(|c| {
            let n = c.get("count") as u32;
            bits(c, &["Q0", "Q1", "Q2", "Q3"], n)
        }),
        inputs: Some(|c| {
            let mut n = c.get("count") as u32;
            if c.level("RST").is_high() {
                n = 0;
            } else if c.rising("CLK") {
                n = (n + 1) % 16;
            }
            c.set("count", n as f64);
            bits(c, &["Q0", "Q1", "Q2", "Q3"], n);
        }),
        timer: None,
        control: None,
    }));
    describe(
        d,
        "Binary up-counter, rising edge, synchronous reset while RST is high.",
    )
}

fn shift() -> ComponentDef {
    let mut d = new("core.shift4", "4-bit shift register", "shift4", "U");
    ins(&mut d, &["D", "CLK"]);
    outs(&mut d, &["Q0", "Q1", "Q2", "Q3"]);
    state(&mut d, "reg", 0.0);
    d.behavior = Some(Arc::new(Fns {
        start: Some(|c| {
            let n = c.get("reg") as u32;
            bits(c, &["Q0", "Q1", "Q2", "Q3"], n)
        }),
        inputs: Some(|c| {
            if c.rising("CLK") {
                let n = ((c.get("reg") as u32) << 1 | u32::from(c.level("D").is_high())) & 15;
                c.set("reg", n as f64);
                bits(c, &["Q0", "Q1", "Q2", "Q3"], n);
            }
        }),
        timer: None,
        control: None,
    }));
    describe(d, "Serial-in parallel-out shift register; D enters at Q0.")
}

fn mux() -> ComponentDef {
    let mut d = new("core.mux2", "Multiplexer 2:1", "mux2", "U");
    ins(&mut d, &["A", "B", "S"]);
    outs(&mut d, &["Y"]);
    d.behavior = Some(behavior(|c| {
        let l = if c.level("S").is_high() {
            c.level("B")
        } else {
            c.level("A")
        };
        let t = c.param("tpd");
        c.drive("Y", l, t);
    }));
    describe(d, "Y follows A while S is low, B while S is high.")
}

const SEGMENTS: [u8; 16] = [
    0x3F, 0x06, 0x5B, 0x4F, 0x66, 0x6D, 0x7D, 0x07, 0x7F, 0x6F, 0x77, 0x7C, 0x39, 0x5E, 0x79, 0x71,
];
const SEG: [&str; 7] = ["a", "b", "c", "d", "e", "f", "g"];

fn bcd7() -> ComponentDef {
    let mut d = new("core.bcd7seg", "BCD to 7-segment", "bcd7seg", "U");
    ins(&mut d, &["A", "B", "C", "D"]);
    outs(&mut d, &SEG);
    d.behavior = Some(behavior(|c| {
        let n = ["A", "B", "C", "D"]
            .iter()
            .enumerate()
            .map(|(k, p)| u32::from(c.level(p).is_high()) << k)
            .sum::<u32>();
        let pattern = SEGMENTS[n as usize & 15] as u32;
        bits(c, &SEG, pattern);
    }));
    describe(
        d,
        "Decodes a 4-bit value (A is the LSB) to segments a to g, hex digits included.",
    )
}

fn toggle() -> ComponentDef {
    let mut d = new("core.logic_toggle", "Logic toggle", "logic_toggle", "SW");
    outs(&mut d, &["OUT"]);
    state(&mut d, "on", 0.0);
    d.controls.push(ControlDef::Toggle {
        state: "on".into(),
        label: "High".into(),
    });
    d.behavior = Some(Arc::new(Fns {
        start: Some(|c| {
            let on = c.get("on") > 0.5;
            c.drive("OUT", Level::from_bool(on), 0.0)
        }),
        inputs: None,
        timer: None,
        control: Some(|c, ev| {
            if let ControlEvent::SetState { state, value } = ev {
                c.set(state, *value);
                c.drive("OUT", Level::from_bool(*value > 0.5), 0.0);
            }
        }),
    }));
    describe(d, "A logic input you click while the circuit runs.")
}

fn clock() -> ComponentDef {
    let mut d = new("core.clock", "Clock", "clock", "CLK");
    d.params
        .push(ParamDef::real("freq", 1.0e3, "Hz", "Frequency").range(1e-3, 1e9));
    d.params
        .push(ParamDef::real("duty", 0.5, "", "High fraction").range(0.01, 0.99));
    outs(&mut d, &["OUT"]);
    d.behavior = Some(Arc::new(Fns {
        start: Some(|c| {
            c.drive("OUT", Level::High, 0.0);
            let t = c.param("duty") / c.param("freq");
            c.timer(t, 0);
        }),
        inputs: None,
        timer: Some(|c, tag| {
            let (f, duty) = (c.param("freq"), c.param("duty"));
            if tag == 0 {
                c.drive("OUT", Level::Low, 0.0);
                c.timer((1.0 - duty) / f, 1);
            } else {
                c.drive("OUT", Level::High, 0.0);
                c.timer(duty / f, 0);
            }
        }),
        control: None,
    }));
    describe(d, "Square-wave logic clock.")
}

fn logic_probe() -> ComponentDef {
    let mut d = new("core.logic_probe", "Logic probe", "logic_probe", "LP");
    ins(&mut d, &["IN"]);
    state(&mut d, "lit", 0.0);
    d.behavior = Some(behavior(|c| {
        let v = if c.level("IN").is_high() { 1.0 } else { 0.0 };
        c.set("lit", v)
    }));
    probe(&mut d, "brightness", "lit", "");
    describe(d, "Lights while its input is high.")
}

fn sevenseg() -> ComponentDef {
    let mut d = new("core.sevenseg", "7-segment display", "sevenseg", "DS");
    ins(&mut d, &SEG);
    for s in SEG {
        state(&mut d, &format!("s{s}"), 0.0);
        probe(&mut d, &format!("seg_{s}"), &format!("s{s}"), "");
    }
    d.behavior = Some(behavior(|c| {
        for s in SEG {
            let v = if c.level(s).is_high() { 1.0 } else { 0.0 };
            c.set(&format!("s{s}"), v);
        }
    }));
    describe(
        d,
        "Common-cathode display driven by logic levels on a to g.",
    )
}

fn ne555() -> ComponentDef {
    let mut d = ComponentDef::new("core.ne555", "555 timer", "Analog ICs", "U", "ne555");
    for p in ["VCC", "GND", "TRIG", "THR", "DIS", "RST"] {
        d.pins.push(PinDef {
            name: p.into(),
            kind: PinKind::Analog,
        });
    }
    d.pins.push(PinDef {
        name: "OUT".into(),
        kind: PinKind::DigitalOut("rail:VCC:GND".into()),
    });
    d.params
        .push(ParamDef::real("tpd", 100e-9, "s", "Propagation delay").range(0.0, 1e-3));
    state(&mut d, "dis", 1.0);
    state(&mut d, "q", 0.0);
    // Internal 3 x 5 kΩ divider, discharge transistor, high-impedance inputs.
    d.branches.push(BranchDef::Current {
        name: None,
        a: "VCC".into(),
        b: "GND".into(),
        i: e("V(VCC, GND) / 15k"),
        q: None,
    });
    d.branches.push(BranchDef::Current {
        name: None,
        a: "DIS".into(),
        b: "GND".into(),
        i: e("V(DIS, GND) * select(dis > 0.5, 1 / 10, 1e-9)"),
        q: None,
    });
    for p in ["TRIG", "THR", "RST"] {
        d.branches.push(BranchDef::Current {
            name: None,
            a: p.into(),
            b: "GND".into(),
            i: e(&format!("V({p}, GND) / 1G")),
            q: None,
        });
    }
    d.watches
        .push(("trig".into(), e("V(TRIG, GND) < V(VCC, GND) / 3")));
    d.watches
        .push(("thr".into(), e("V(THR, GND) > 2 * V(VCC, GND) / 3")));
    d.watches.push(("rst".into(), e("V(RST, GND) < 0.7")));
    fn update(c: &mut EventCtx<'_>) {
        let mut q = c.get("q") > 0.5;
        if c.level("rst").is_high() {
            q = false;
        } else if c.level("trig").is_high() {
            q = true;
        } else if c.level("thr").is_high() {
            q = false;
        }
        c.set("q", if q { 1.0 } else { 0.0 });
        c.set("dis", if q { 0.0 } else { 1.0 });
        let t = c.param("tpd");
        c.drive("OUT", Level::from_bool(q), t);
    }
    d.behavior = Some(behavior(update));
    probe(&mut d, "Vout", "V(OUT, GND)", "V");
    describe(
        d,
        "Behavioural CMOS 555 (7555-like output): trigger below VCC/3 sets, threshold above 2/3 VCC resets, RST below 0.7 V resets.",
    )
}

/// Every digital definition and the 555.
pub fn definitions() -> Vec<ComponentDef> {
    let mut v = vec![
        gate("core.and", "AND", "and", |c| {
            gate2(c, |a, b| a && b, Some((false, false)))
        }),
        gate("core.or", "OR", "or", |c| {
            gate2(c, |a, b| a || b, Some((true, true)))
        }),
        gate("core.nand", "NAND", "nand", |c| {
            gate2(c, |a, b| !(a && b), Some((false, true)))
        }),
        gate("core.nor", "NOR", "nor", |c| {
            gate2(c, |a, b| !(a || b), Some((true, false)))
        }),
        gate("core.xor", "XOR", "xor", |c| gate2(c, |a, b| a != b, None)),
        gate("core.xnor", "XNOR", "xnor", |c| {
            gate2(c, |a, b| a == b, None)
        }),
        not_gate("core.not", "NOT", "not", true),
        not_gate("core.buffer", "Buffer", "buf", false),
        tristate(),
        counter(),
        shift(),
        mux(),
        bcd7(),
        toggle(),
        clock(),
        logic_probe(),
        sevenseg(),
        ne555(),
    ];
    v.extend(flipflops());
    for d in &mut v {
        if matches!(
            d.id.as_str(),
            "core.logic_toggle" | "core.clock" | "core.logic_probe"
        ) {
            d.meta.category = "Digital I/O".into();
        }
        if d.id == "core.sevenseg" {
            d.meta.category = "Indicators".into();
        }
    }
    v
}
