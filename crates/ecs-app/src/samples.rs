//! Built-in sample circuits shown on the Start screen.

use crate::catalog::Catalog;
use ecs_schematic::{Command, Id, P2, Schematic, SimSettings, route};

/// A sample: id, title, subtitle and builder.
pub struct Sample {
    pub id: &'static str,
    pub title: &'static str,
    pub subtitle: &'static str,
    pub build: fn(&Catalog) -> Schematic,
}

/// Every sample in display order.
pub fn all() -> Vec<Sample> {
    vec![
        Sample {
            id: "led-switch",
            title: "LED and Switch",
            subtitle: "Click the switch while it runs: the LED lights and current dots flow",
            build: led_switch,
        },
        Sample {
            id: "rc-lowpass",
            title: "RC Low-pass",
            subtitle: "1 kHz sine into a 1 kHz corner; scope shows input and output",
            build: rc_lowpass,
        },
        Sample {
            id: "astable",
            title: "Astable Multivibrator",
            subtitle: "Two transistors take turns: the LEDs blink about every 0.3 s",
            build: astable,
        },
        Sample {
            id: "counter",
            title: "Digital Counter",
            subtitle: "A 555 clocks a 4-bit counter; a decoder drives the 7-segment display",
            build: counter,
        },
        Sample {
            id: "rectifier",
            title: "Half-wave Rectifier",
            subtitle: "Diode and reservoir capacitor smooth a 50 Hz sine",
            build: rectifier,
        },
        Sample {
            id: "rc-blocks",
            title: "Custom Component",
            subtitle: "Two copies of an RC block drawn as a schematic; Ctrl+Enter on one opens it",
            build: rc_blocks,
        },
        Sample {
            id: "divider",
            title: "Potentiometer Divider",
            subtitle: "Drag the wiper slider in the inspector and watch the meter",
            build: divider,
        },
    ]
}

struct B<'a> {
    cat: &'a Catalog,
    doc: Schematic,
}

impl B<'_> {
    fn part(&mut self, def: &str, at: P2, rot: u8, mirror: bool, params: &[(&str, &str)]) -> Id {
        let d = self
            .cat
            .def(def)
            .unwrap_or_else(|| panic!("sample uses unknown {def}"))
            .clone();
        let mut p = self.cat.new_part(&self.doc, &d, at);
        p.rot = rot;
        p.mirror = mirror;
        for (k, v) in params {
            p.params.insert(k.to_string(), v.to_string());
        }
        match self.doc.apply(Command::AddPart(p))[0] {
            ecs_schematic::Item::Part(id) => id,
            _ => unreachable!(),
        }
    }
    /// A part of a component defined in this document.
    fn local(&mut self, comp: &str, at: P2, params: &[(&str, &str)]) -> Id {
        let c = self
            .doc
            .components
            .iter()
            .find(|c| c.id == comp)
            .expect("component")
            .clone();
        let refdes = self.doc.next_refdes(&c.prefix);
        let mut p =
            ecs_schematic::new_part(&c.id, &crate::components::symbol_id(&c.id), refdes, at);
        for (k, v) in params {
            p.params.insert(k.to_string(), v.to_string());
        }
        match self.doc.apply(Command::AddPart(p))[0] {
            ecs_schematic::Item::Part(id) => id,
            _ => unreachable!(),
        }
    }
    fn pin(&self, part: Id, name: &str) -> P2 {
        self.doc
            .part(part)
            .and_then(|p| p.pins().into_iter().find(|x| x.0 == name))
            .map(|x| x.1)
            .expect("pin")
    }
    fn wire(&mut self, pts: &[P2]) {
        self.doc.apply(Command::AddWire(pts.to_vec()));
    }
    fn route(&mut self, a: P2, b: P2, horizontal_first: bool) {
        self.doc
            .apply(Command::AddWire(route(a, b, horizontal_first)));
    }
    fn label(&mut self, at: P2, name: &str) {
        self.doc.apply(Command::AddLabel {
            at,
            name: name.into(),
        });
    }
    fn scope(&mut self, name: &str, expr: &str) {
        self.doc.scope.push(ecs_schematic::ScopeChannel {
            name: name.into(),
            expr: expr.into(),
        });
    }
    fn note(&mut self, at: P2, text: &str) {
        self.doc.apply(Command::AddNote {
            at,
            text: text.into(),
        });
    }
    fn finish(mut self, title: &str, speed: f64, step: f64) -> Schematic {
        self.doc.metadata.title = title.into();
        self.doc.simulation = SimSettings {
            speed,
            time_step: step,
        };
        self.doc
    }
}

fn start(cat: &Catalog) -> B<'_> {
    B {
        cat,
        doc: Schematic::default(),
    }
}

fn led_switch(cat: &Catalog) -> Schematic {
    let mut b = start(cat);
    b.part("core.battery", [0.0, 0.0], 0, false, &[("V", "9")]);
    b.part("core.switch", [80.0, -80.0], 0, false, &[]);
    b.part("core.resistor", [200.0, -80.0], 0, false, &[("R", "470")]);
    b.part("core.led", [280.0, 0.0], 1, false, &[("color", "red")]);
    b.part("core.ground", [140.0, 120.0], 0, false, &[]);
    b.route([0.0, -40.0], [40.0, -80.0], false);
    b.wire(&[[120.0, -80.0], [160.0, -80.0]]);
    b.route([240.0, -80.0], [280.0, -40.0], true);
    b.route([280.0, 40.0], [140.0, 120.0], false);
    b.route([0.0, 40.0], [140.0, 120.0], false);
    b.note([-20.0, -140.0], "Click the switch to close it");
    b.finish("LED and Switch", 1e-3, 1e-6)
}

fn rc_lowpass(cat: &Catalog) -> Schematic {
    let mut b = start(cat);
    b.part(
        "core.vsine",
        [0.0, 0.0],
        0,
        false,
        &[("Vamp", "1"), ("freq", "1k")],
    );
    b.part("core.resistor", [80.0, -80.0], 0, false, &[("R", "1k")]);
    b.part("core.capacitor", [160.0, 0.0], 1, false, &[("C", "159n")]);
    b.part("core.ground", [80.0, 120.0], 0, false, &[]);
    b.route([0.0, -40.0], [40.0, -80.0], false);
    b.route([120.0, -80.0], [160.0, -40.0], true);
    b.route([160.0, 40.0], [80.0, 120.0], false);
    b.route([0.0, 40.0], [80.0, 120.0], false);
    b.label([0.0, -80.0], "in");
    b.label([160.0, -80.0], "out");
    b.scope("in", "in");
    b.scope("out", "out");
    b.finish("RC Low-pass", 2e-3, 2e-6)
}

fn astable(cat: &Catalog) -> Schematic {
    let mut b = start(cat);
    b.part("core.vdc", [-80.0, 40.0], 0, false, &[("V", "9")]);
    b.part("core.led", [0.0, -60.0], 1, false, &[("color", "red")]);
    b.part("core.resistor", [0.0, 40.0], 1, false, &[("R", "470")]);
    b.part("core.npn", [20.0, 120.0], 0, true, &[]);
    b.part("core.led", [320.0, -60.0], 1, false, &[("color", "green")]);
    b.part("core.resistor", [320.0, 40.0], 1, false, &[("R", "470")]);
    b.part("core.npn", [300.0, 120.0], 0, false, &[("Is", "11f")]);
    b.part("core.resistor", [120.0, -60.0], 1, false, &[("R", "47k")]);
    b.part("core.resistor", [200.0, -60.0], 1, false, &[("R", "47k")]);
    b.part("core.capacitor", [60.0, 60.0], 0, false, &[("C", "10n")]);
    b.part("core.capacitor", [260.0, 20.0], 0, false, &[("C", "10n")]);
    b.part("core.ground", [160.0, 200.0], 0, false, &[]);
    // Supply rail and drops.
    b.wire(&[
        [-80.0, 0.0],
        [-80.0, -120.0],
        [320.0, -120.0],
        [320.0, -100.0],
    ]);
    for x in [0.0, 120.0, 200.0] {
        b.wire(&[[x, -100.0], [x, -120.0]]);
    }
    // LEDs into collector resistors.
    b.wire(&[[0.0, -20.0], [0.0, 0.0]]);
    b.wire(&[[320.0, -20.0], [320.0, 0.0]]);
    // Base resistors to the bases.
    b.route([120.0, -20.0], [60.0, 120.0], false);
    b.route([200.0, -20.0], [260.0, 120.0], false);
    // Cross-coupling capacitors.
    b.wire(&[[100.0, 60.0], [200.0, 60.0]]);
    b.route([20.0, 60.0], [0.0, 80.0], false);
    b.wire(&[[220.0, 20.0], [120.0, 20.0]]);
    b.route([300.0, 20.0], [320.0, 80.0], false);
    // Emitters and supply return.
    b.wire(&[[0.0, 160.0], [0.0, 200.0], [320.0, 200.0], [320.0, 160.0]]);
    b.wire(&[[-80.0, 80.0], [-80.0, 200.0], [0.0, 200.0]]);
    // Scope: Q2's collector and base, by part code and terminal.
    b.scope("Q2 collector", "Q2.C.V");
    b.scope("Q2 base", "Q2.B.V");
    b.finish("Astable Multivibrator", 1e-3, 1e-6)
}

fn counter(cat: &Catalog) -> Schematic {
    let mut b = start(cat);
    b.part("core.vdc", [-200.0, 40.0], 0, false, &[("V", "5")]);
    b.part("core.ground", [-200.0, 100.0], 0, false, &[]);
    b.part("core.ne555", [0.0, 0.0], 0, false, &[]);
    b.part("core.resistor", [-140.0, -40.0], 1, false, &[("R", "1k")]);
    b.part("core.resistor", [-140.0, 40.0], 1, false, &[("R", "10k")]);
    b.part("core.capacitor", [-140.0, 120.0], 1, false, &[("C", "1u")]);
    b.part("core.ground", [-140.0, 160.0], 0, false, &[]);
    b.part("core.ground", [80.0, 40.0], 0, false, &[]);
    b.part("core.counter4", [200.0, 0.0], 0, false, &[]);
    b.part("core.ground", [120.0, 20.0], 0, false, &[]);
    b.part("core.bcd7seg", [400.0, 40.0], 0, false, &[]);
    b.part("core.sevenseg", [540.0, 40.0], 0, false, &[]);
    // Supply rail and 555 timing network.
    b.wire(&[[-200.0, 0.0], [-200.0, -80.0], [60.0, -80.0], [60.0, -20.0]]);
    b.wire(&[[-200.0, 80.0], [-200.0, 100.0]]);
    b.wire(&[[-60.0, 20.0], [-80.0, 20.0], [-80.0, -80.0]]);
    b.wire(&[
        [-60.0, -20.0],
        [-100.0, -20.0],
        [-100.0, 80.0],
        [-140.0, 80.0],
    ]);
    b.wire(&[[-60.0, 0.0], [-100.0, 0.0]]);
    b.wire(&[[-60.0, 40.0], [-120.0, 40.0], [-120.0, 0.0], [-140.0, 0.0]]);
    b.wire(&[[60.0, 20.0], [80.0, 20.0], [80.0, 40.0]]);
    // Clock into the counter; RST held low.
    b.wire(&[[60.0, 0.0], [80.0, 0.0], [80.0, -20.0], [140.0, -20.0]]);
    b.wire(&[[140.0, 0.0], [120.0, 0.0], [120.0, 20.0]]);
    // Counter to decoder to display.
    for k in 0..4 {
        let y = -20.0 + 20.0 * k as f64;
        b.wire(&[[260.0, y], [340.0, y]]);
    }
    for k in 0..7 {
        let y = -20.0 + 20.0 * k as f64;
        b.wire(&[[460.0, y], [500.0, y]]);
    }
    b.note(
        [-200.0, -140.0],
        "f = 1.44 / ((RA + 2 RB) C) ≈ 69 Hz, shown at 20 ms/s",
    );
    b.scope("timing capacitor", "C1.V");
    b.scope("clock", "U1.OUT.V");
    b.finish("Digital Counter", 0.02, 10e-6)
}

fn rectifier(cat: &Catalog) -> Schematic {
    let mut b = start(cat);
    b.part(
        "core.vsine",
        [0.0, 40.0],
        0,
        false,
        &[("Vamp", "10"), ("freq", "50")],
    );
    b.part("core.diode", [80.0, -40.0], 0, false, &[]);
    b.part("core.capacitor", [160.0, 40.0], 1, false, &[("C", "100u")]);
    b.part("core.resistor", [240.0, 40.0], 1, false, &[("R", "1k")]);
    b.part("core.ground", [120.0, 120.0], 0, false, &[]);
    b.route([0.0, 0.0], [40.0, -40.0], false);
    b.route([120.0, -40.0], [160.0, 0.0], true);
    b.route([160.0, -40.0], [240.0, 0.0], true);
    b.wire(&[[0.0, 80.0], [0.0, 120.0], [240.0, 120.0], [240.0, 80.0]]);
    b.wire(&[[160.0, 80.0], [160.0, 120.0]]);
    b.label([0.0, -40.0], "ac");
    b.label([240.0, -40.0], "dc");
    b.scope("ac", "ac");
    b.scope("dc", "dc");
    b.scope("ripple current", "C1.I");
    b.finish("Half-wave Rectifier", 0.05, 20e-6)
}

/// An RC low-pass block defined by a schematic, used twice in cascade.
fn rc_blocks(cat: &Catalog) -> Schematic {
    use ecs_schematic::{Implementation, LocalComponent, LocalParam};
    let mut inner = start(cat);
    inner.part("core.resistor", [0.0, 0.0], 0, false, &[("R", "{R}")]);
    inner.part("core.capacitor", [80.0, 40.0], 1, false, &[("C", "{C}")]);
    inner.part("core.ground", [80.0, 120.0], 0, false, &[]);
    inner.wire(&[[40.0, 0.0], [80.0, 0.0], [120.0, 0.0]]);
    inner.wire(&[[80.0, 80.0], [80.0, 120.0]]);
    inner.label([-40.0, 0.0], "IN");
    inner.label([120.0, 0.0], "OUT");
    let inner = inner.finish("RC block", 1e-3, 1e-6);
    let id = "local.rc_block";
    let mut symbol = ecs_schematic::symbols::boxed(id, "RC", &["IN"], &["OUT"]);
    symbol.id = crate::components::symbol_id(id);
    ecs_schematic::symbols::register(symbol.clone());
    let mut b = start(cat);
    b.doc.components.push(LocalComponent {
        id: id.into(),
        name: "RC block".into(),
        category: "User".into(),
        prefix: "X".into(),
        description: "First-order low-pass: R in series, C to ground".into(),
        symbol,
        params: vec![
            LocalParam {
                name: "R".into(),
                default: "1k".into(),
                unit: "Ω".into(),
                description: "Series resistance".into(),
            },
            LocalParam {
                name: "C".into(),
                default: "159n".into(),
                unit: "F".into(),
                description: "Shunt capacitance".into(),
            },
        ],
        implementation: Implementation::Schematic {
            schematic: Box::new(inner),
        },
    });
    b.part(
        "core.vsine",
        [0.0, 40.0],
        0,
        false,
        &[("Vamp", "1"), ("freq", "1k")],
    );
    let x1 = b.local(id, [140.0, -40.0], &[]);
    let x2 = b.local(id, [320.0, -40.0], &[("R", "10k"), ("C", "15.9n")]);
    b.part("core.ground", [160.0, 160.0], 0, false, &[]);
    let (i1, o1, i2, o2) = (
        b.pin(x1, "IN"),
        b.pin(x1, "OUT"),
        b.pin(x2, "IN"),
        b.pin(x2, "OUT"),
    );
    b.route([0.0, 0.0], i1, false);
    b.route(o1, i2, true);
    b.wire(&[o2, [o2[0] + 40.0, o2[1]]]);
    b.route([0.0, 80.0], [160.0, 160.0], false);
    b.label([0.0, i1[1]], "in");
    b.label([o1[0] + 20.0, o1[1]], "mid");
    b.label([o2[0] + 40.0, o2[1]], "out");
    b.note(
        [60.0, -120.0],
        "Select a block and press Ctrl+Enter to edit its schematic; Ctrl+E edits its symbol",
    );
    b.scope("in", "in");
    b.scope("mid", "mid");
    b.scope("out", "out");
    b.finish("Custom Component", 2e-3, 2e-6)
}

fn divider(cat: &Catalog) -> Schematic {
    let mut b = start(cat);
    b.part("core.vdc", [0.0, 40.0], 0, false, &[("V", "10")]);
    b.part(
        "core.potentiometer",
        [120.0, 40.0],
        1,
        false,
        &[("R", "10k")],
    );
    b.part("core.voltmeter", [240.0, 40.0], 1, false, &[]);
    b.part("core.ground", [60.0, 120.0], 0, false, &[]);
    b.wire(&[[0.0, 0.0], [0.0, -40.0], [120.0, -40.0], [120.0, 0.0]]);
    b.wire(&[[160.0, 40.0], [200.0, 40.0], [200.0, 0.0], [240.0, 0.0]]);
    b.wire(&[[0.0, 80.0], [0.0, 120.0], [240.0, 120.0], [240.0, 80.0]]);
    b.wire(&[[120.0, 80.0], [120.0, 120.0]]);
    b.finish("Potentiometer Divider", 1e-3, 1e-6)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_are_fully_connected_and_simulate() {
        let mut cat = Catalog::builtin();
        for s in all() {
            let doc = (s.build)(&cat);
            let problems = crate::components::compile_all(&mut cat, &doc, None);
            assert!(problems.is_empty(), "{}: {problems:?}", s.id);
            let built = crate::catalog::build(&cat, &doc);
            assert!(built.problems.is_empty(), "{}: {:?}", s.id, built.problems);
            let unconnected: Vec<_> = built
                .unconnected
                .iter()
                .filter(|(id, _)| doc.part(*id).is_some())
                .collect();
            assert!(
                unconnected.is_empty(),
                "{}: unconnected {:?}",
                s.id,
                unconnected
            );
            let mut c =
                ecs_sim::Circuit::compile_with(&built.netlist, ecs_sim::Options::interactive())
                    .unwrap_or_else(|e| panic!("{}: {e}", s.id));
            c.reset().unwrap_or_else(|e| panic!("{}: {e}", s.id));
            for _ in 0..200 {
                c.step_interactive(doc.simulation.time_step);
            }
        }
    }

    #[test]
    fn every_definition_pin_is_on_its_symbol() {
        let cat = Catalog::builtin();
        for d in cat.lib.iter() {
            let sym = ecs_schematic::symbols::get(&d.symbol);
            for p in &d.pins {
                assert!(
                    sym.pins.iter().any(|sp| sp.name == p.name),
                    "{}: symbol {} has no pin {}",
                    d.id,
                    d.symbol,
                    p.name
                );
            }
        }
    }

    #[test]
    fn counter_sample_counts() {
        let cat = Catalog::builtin();
        let doc = counter(&cat);
        let built = crate::catalog::build(&cat, &doc);
        let mut c = ecs_sim::Circuit::compile_with(&built.netlist, ecs_sim::Options::interactive())
            .unwrap();
        c.reset().unwrap();
        let u = c.instance_index("U2").unwrap();
        for _ in 0..5000 {
            c.step_interactive(10e-6);
        }
        // 50 ms at about 69 Hz: three or four counts.
        let n = c.state(u, "count").unwrap();
        assert!((3.0..=4.0).contains(&n), "{n}");
    }

    #[test]
    fn astable_sample_blinks() {
        let cat = Catalog::builtin();
        let doc = astable(&cat);
        let built = crate::catalog::build(&cat, &doc);
        let mut c = ecs_sim::Circuit::compile_with(&built.netlist, ecs_sim::Options::interactive())
            .unwrap();
        c.reset().unwrap();
        let q1 = c.instance_index("Q1").unwrap();
        let mut transitions = 0;
        let mut on = c.branch_currents(q1)[0] > 5e-3;
        for _ in 0..3000 {
            c.step_interactive(1e-6);
            let now = c.branch_currents(q1)[0] > 5e-3;
            if now != on {
                transitions += 1;
                on = now;
            }
        }
        assert!(transitions >= 4, "{transitions}");
    }

    #[test]
    fn astable_runs_without_convergence_trouble() {
        let cat = Catalog::builtin();
        let doc = astable(&cat);
        let built = crate::catalog::build(&cat, &doc);
        let mut c = ecs_sim::Circuit::compile_with(&built.netlist, ecs_sim::Options::interactive())
            .unwrap();
        c.reset().unwrap();
        for _ in 0..30_000 {
            c.step_interactive(1e-6);
        }
        let flagged: Vec<String> = (0..c.instance_count())
            .filter(|&k| c.instance_trouble(k))
            .map(|k| c.instance_name(k).to_string())
            .collect();
        assert!(flagged.is_empty(), "{flagged:?}");
    }

    #[test]
    fn led_switch_lights_when_closed() {
        let cat = Catalog::builtin();
        let doc = led_switch(&cat);
        let built = crate::catalog::build(&cat, &doc);
        let mut c = ecs_sim::Circuit::compile_with(&built.netlist, ecs_sim::Options::interactive())
            .unwrap();
        c.reset().unwrap();
        let s = c.instance_index("S1").unwrap();
        let d = c.instance_index("D1").unwrap();
        assert!(c.probes(d)[2] < 0.01);
        c.set_state(s, "closed", 1.0);
        for _ in 0..50 {
            c.step_interactive(1e-6);
        }
        let brightness = c.probes(d)[2];
        assert!(brightness > 0.5, "{brightness}");
    }
}
