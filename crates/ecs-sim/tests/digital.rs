//! Digital and mixed-signal tests (spec section 11, phase P5).

use ecs_lib::library;
use ecs_sim::{Circuit, Netlist, Options};

fn lib(id: &str) -> std::sync::Arc<ecs_model::ComponentDef> {
    library().get(id).unwrap_or_else(|| panic!("{id}")).clone()
}

fn run(c: &mut Circuit, t: f64, h: f64) {
    let stop = c.time() + t;
    while c.time() < stop - 1e-12 {
        c.step_interactive(h.min(stop - c.time()));
    }
}

#[test]
fn gate_truth_tables() {
    let cases: [(&str, [bool; 4]); 6] = [
        ("core.and", [false, false, false, true]),
        ("core.or", [false, true, true, true]),
        ("core.nand", [true, true, true, false]),
        ("core.nor", [true, false, false, false]),
        ("core.xor", [false, true, true, false]),
        ("core.xnor", [true, false, false, true]),
    ];
    for (id, table) in cases {
        let mut n = Netlist::new();
        n.add("SA", lib("core.logic_toggle"), &["a"], &[]);
        n.add("SB", lib("core.logic_toggle"), &["b"], &[]);
        n.add("U1", lib(id), &["a", "b", "y"], &[]);
        n.add("R1", lib("core.resistor"), &["y", "0"], &[("R", 100e3)]);
        let mut c = Circuit::compile_with(&n, Options::interactive()).unwrap();
        c.reset().unwrap();
        let (sa, sb) = (
            c.instance_index("SA").unwrap(),
            c.instance_index("SB").unwrap(),
        );
        for (k, want) in table.iter().enumerate() {
            c.control(sa, "on", (k & 1) as f64);
            c.control(sb, "on", (k >> 1) as f64);
            run(&mut c, 2e-6, 1e-7);
            let y = c.voltage("y").unwrap();
            assert_eq!(y > 2.5, *want, "{id} a={} b={} y={y}", k & 1, k >> 1);
        }
    }
}

#[test]
fn inverter_settles_at_start() {
    let mut n = Netlist::new();
    n.add("U1", lib("core.not"), &["0", "y"], &[]);
    let mut c = Circuit::compile(&n).unwrap();
    c.reset().unwrap();
    assert!(c.voltage("y").unwrap() > 4.9, "{}", c.voltage("y").unwrap());
}

#[test]
fn dff_divides_the_clock_by_two() {
    let mut n = Netlist::new();
    n.add("CK", lib("core.clock"), &["clk"], &[("freq", 1e3)]);
    n.add("U1", lib("core.dff"), &["qn", "clk", "q", "qn"], &[]);
    let mut c = Circuit::compile_with(&n, Options::interactive()).unwrap();
    c.reset().unwrap();
    let mut edges = 0;
    let mut last = c.voltage("q").unwrap() > 2.5;
    for _ in 0..10_000 {
        c.step_interactive(1e-6);
        let now = c.voltage("q").unwrap() > 2.5;
        if now && !last {
            edges += 1;
        }
        last = now;
    }
    // 10 ms of a 1 kHz clock: Q rises five times.
    assert!((4..=6).contains(&edges), "{edges}");
}

#[test]
fn counter_clocked_by_555_drives_a_display_with_expected_timing() {
    let mut n = Netlist::new();
    n.add("V1", lib("core.vdc"), &["vcc", "0"], &[("V", 5.0)]);
    n.add("RA", lib("core.resistor"), &["vcc", "dis"], &[("R", 1e3)]);
    n.add("RB", lib("core.resistor"), &["dis", "thr"], &[("R", 10e3)]);
    n.add("C1", lib("core.capacitor"), &["thr", "0"], &[("C", 100e-9)]);
    // 555 pins: VCC GND TRIG THR DIS RST OUT
    n.add(
        "U1",
        lib("core.ne555"),
        &["vcc", "0", "thr", "thr", "dis", "vcc", "clk"],
        &[],
    );
    n.add(
        "U2",
        lib("core.counter4"),
        &["clk", "0", "q0", "q1", "q2", "q3"],
        &[],
    );
    n.add(
        "U3",
        lib("core.bcd7seg"),
        &["q0", "q1", "q2", "q3", "a", "b", "c", "d", "e", "f", "g"],
        &[],
    );
    n.add(
        "DS1",
        lib("core.sevenseg"),
        &["a", "b", "c", "d", "e", "f", "g"],
        &[],
    );
    let mut c = Circuit::compile_with(&n, Options::interactive()).unwrap();
    c.reset().unwrap();
    let mut rises = 0;
    let mut last = c.voltage("clk").unwrap() > 2.5;
    let t_end = 20e-3;
    while c.time() < t_end {
        c.step_interactive(1e-6);
        let now = c.voltage("clk").unwrap() > 2.5;
        if now && !last {
            rises += 1;
        }
        last = now;
    }
    // f = 1.44 / ((RA + 2 RB) C) = 686 Hz; the first cycle is longer (charging from 0).
    let expected = 1.44 / ((1e3 + 2.0 * 10e3) * 100e-9) * t_end;
    assert!(
        (rises as f64 - expected).abs() <= 2.0,
        "{rises} clock edges, expected about {expected:.1}"
    );
    let count: u32 = ["q0", "q1", "q2", "q3"]
        .iter()
        .enumerate()
        .map(|(k, q)| u32::from(c.voltage(q).unwrap() > 2.5) << k)
        .sum();
    assert_eq!(count, rises % 16);
    // The display shows the count: segment probes match the decoder pattern.
    let ds = c.instance_index("DS1").unwrap();
    let lit: Vec<bool> = c.probes(ds).iter().map(|v| *v > 0.5).collect();
    let pattern = [
        0x3F, 0x06, 0x5B, 0x4F, 0x66, 0x6D, 0x7D, 0x07, 0x7F, 0x6F, 0x77, 0x7C, 0x39, 0x5E, 0x79,
        0x71,
    ][count as usize];
    let want: Vec<bool> = (0..7).map(|k| pattern >> k & 1 == 1).collect();
    assert_eq!(lit, want, "count {count}");
}

#[test]
fn subcircuit_flattens_and_reads_foreign_currents() {
    use ecs_model::{ComponentDef, DefKind, PinDef, PinKind, SubElement, Subckt};
    use std::collections::BTreeMap;
    // A divider subcircuit with ports IN, OUT.
    let mut sub = ComponentDef::new("test.div", "Divider", "Test", "X", "box");
    for p in ["IN", "OUT"] {
        sub.pins.push(PinDef {
            name: p.into(),
            kind: PinKind::Analog,
        });
    }
    let r = |v: f64| BTreeMap::from([("R".to_string(), v)]);
    sub.kind = DefKind::Subcircuit(std::sync::Arc::new(Subckt {
        elements: vec![
            SubElement {
                name: "R1".into(),
                def: lib("core.resistor"),
                nodes: vec!["IN".into(), "OUT".into()],
                params: r(1e3),
                exprs: Default::default(),
            },
            // R2 follows the subcircuit parameter Rb.
            SubElement {
                name: "R2".into(),
                def: lib("core.resistor"),
                nodes: vec!["OUT".into(), "0".into()],
                params: BTreeMap::new(),
                exprs: BTreeMap::from([("R".to_string(), ecs_expr::parse("Rb").unwrap())]),
            },
        ],
    }));
    sub.params
        .push(ecs_model::ParamDef::real("Rb", 1e3, "Ω", "Lower resistor"));
    let mut n = Netlist::new();
    n.add("V1", lib("core.vdc"), &["in", "0"], &[("V", 4.0)]);
    n.add(
        "X1",
        std::sync::Arc::new(sub),
        &["in", "out"],
        &[("Rb", 3e3)],
    );
    // A CCCS reading V1's current through the global I(V1) reference.
    let mut cccs = (*lib("core.vccs")).clone();
    cccs.id = "test.mirror".into();
    cccs.branches = vec![ecs_model::BranchDef::Current {
        name: None,
        a: "N".into(),
        b: "P".into(),
        i: ecs_expr::parse("-I(V1)").unwrap(),
        q: None,
    }];
    n.add("F1", std::sync::Arc::new(cccs), &["m", "0", "0", "0"], &[]);
    n.add("RM", lib("core.resistor"), &["m", "0"], &[("R", 1e3)]);
    let mut c = Circuit::compile(&n).unwrap();
    c.dc_op().unwrap();
    // 4 V across 1k + 3k (the instance's Rb).
    assert!((c.voltage("out").unwrap() - 3.0).abs() < 1e-6);
    assert!(c.voltage("X1.OUT").is_none(), "ports map to outer nodes");
    // V1 delivers 1 mA; the mirror pushes 1 mA into 1 kΩ.
    assert!(
        (c.voltage("m").unwrap() - 1.0).abs() < 1e-3,
        "{}",
        c.voltage("m").unwrap()
    );
}
