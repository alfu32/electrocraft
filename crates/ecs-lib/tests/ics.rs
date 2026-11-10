//! 74xx / 4000-series chips and microcontrollers in circuits.

use ecs_lib::library;
use ecs_model::{ComponentDef, PinKind};
use ecs_sim::{Circuit, Netlist, Options};
use std::collections::BTreeMap;
use std::sync::Arc;

fn lib(id: &str) -> Arc<ComponentDef> {
    library().get(id).unwrap_or_else(|| panic!("{id}")).clone()
}

/// A chip with a logic toggle on every digital input (named after the pin)
/// and a 100 kΩ load on every other pin.
struct Bench {
    c: Circuit,
    toggles: BTreeMap<String, usize>,
}

impl Bench {
    fn new(id: &str, extra: impl FnOnce(&mut Netlist)) -> Self {
        let d = lib(id);
        let mut n = Netlist::new();
        let mut names = Vec::new();
        for p in &d.pins {
            names.push(p.name.clone());
            match p.kind {
                PinKind::DigitalIn(_) => n.add(
                    &format!("T_{}", p.name),
                    lib("core.logic_toggle"),
                    &[&p.name],
                    &[],
                ),
                _ => n.add(
                    &format!("R_{}", p.name),
                    lib("core.resistor"),
                    &[&p.name, "0"],
                    &[("R", 100e3)],
                ),
            }
        }
        extra(&mut n);
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        n.add("U1", d.clone(), &refs, &[]);
        let mut c = Circuit::compile_with(&n, Options::interactive())
            .unwrap_or_else(|e| panic!("{id}: {e}"));
        c.reset().unwrap_or_else(|e| panic!("{id}: {e}"));
        let toggles = d
            .pins
            .iter()
            .filter(|p| matches!(p.kind, PinKind::DigitalIn(_)))
            .map(|p| {
                (
                    p.name.clone(),
                    c.instance_index(&format!("T_{}", p.name)).unwrap(),
                )
            })
            .collect();
        Self { c, toggles }
    }
    fn set(&mut self, pin: &str, high: bool) {
        let k = self.toggles[pin];
        self.c.control(k, "on", f64::from(u8::from(high)));
    }
    fn run(&mut self, t: f64) {
        let stop = self.c.time() + t;
        while self.c.time() < stop - 1e-12 {
            self.c.step_interactive(1e-7f64.min(stop - self.c.time()));
        }
    }
    fn hi(&self, pin: &str) -> bool {
        self.c.voltage(pin).unwrap() > 2.5
    }
    fn pulse(&mut self, pin: &str) {
        self.set(pin, true);
        self.run(1e-6);
        self.set(pin, false);
        self.run(1e-6);
    }
}

#[test]
fn every_logic_chip_simulates() {
    let ids: Vec<String> = library()
        .iter()
        .filter(|d| d.id.starts_with("ic."))
        .map(|d| d.id.clone())
        .collect();
    assert!(ids.len() >= 70, "{} chips", ids.len());
    for id in ids {
        let mut b = Bench::new(&id, |_| {});
        b.run(2e-6);
    }
}

#[test]
fn nand_truth_table() {
    let mut b = Bench::new("ic.74x00", |_| {});
    for (a, bb, y) in [
        (false, false, true),
        (true, false, true),
        (false, true, true),
        (true, true, false),
    ] {
        b.set("A3", a);
        b.set("B3", bb);
        b.run(1e-6);
        assert_eq!(b.hi("Y3"), y, "{a} {bb}");
    }
}

#[test]
fn decoder_selects_one_output() {
    let mut b = Bench::new("ic.74x138", |_| {});
    b.set("G1", true);
    for n in 0..8u32 {
        b.set("A", n & 1 == 1);
        b.set("B", n & 2 == 2);
        b.set("C", n & 4 == 4);
        b.run(1e-6);
        for k in 0..8 {
            assert_eq!(b.hi(&format!("Y{k}")), k != n, "input {n}, Y{k}");
        }
    }
    b.set("G1", false);
    b.run(1e-6);
    assert!((0..8).all(|k| b.hi(&format!("Y{k}"))));
}

#[test]
fn counters_count() {
    let mut b = Bench::new("ic.74x161", |_| {});
    for p in ["CLR", "LOAD", "ENP", "ENT"] {
        b.set(p, true);
    }
    b.run(1e-6);
    for _ in 0..11 {
        b.pulse("CLK");
    }
    let n: u32 = ["QA", "QB", "QC", "QD"]
        .iter()
        .enumerate()
        .map(|(k, p)| u32::from(b.hi(p)) << k)
        .sum();
    assert_eq!(n, 11);
    for _ in 0..4 {
        b.pulse("CLK");
    }
    assert!(b.hi("RCO"), "RCO at 15");

    let mut d = Bench::new("ic.4017", |_| {});
    for _ in 0..13 {
        d.pulse("CLK");
    }
    assert!(d.hi("Q3") && !d.hi("Q2") && d.hi("CO"), "13 mod 10 = 3");
}

#[test]
fn shift_register_latches_on_rclk() {
    let mut b = Bench::new("ic.74x595", |_| {});
    b.set("SRCLR", true);
    b.run(1e-6);
    // Shift in 1,0,1,1,0,0,0,0 (first bit ends at QH).
    for bit in [true, false, true, true, false, false, false, false] {
        b.set("SER", bit);
        b.pulse("SRCLK");
    }
    assert!(!b.hi("QH"), "outputs change only on RCLK");
    b.pulse("RCLK");
    let got: Vec<bool> = ["QA", "QB", "QC", "QD", "QE", "QF", "QG", "QH"]
        .iter()
        .map(|p| b.hi(p))
        .collect();
    assert_eq!(
        got,
        vec![false, false, false, false, true, true, false, true]
    );
    // OE high floats the outputs: the 100 kΩ load pulls them to 0 V.
    b.set("OE", true);
    b.run(1e-6);
    assert!(b.c.voltage("QH").unwrap() < 0.5);
}

#[test]
fn open_collector_needs_a_pull_up() {
    let mut b = Bench::new("ic.74x06", |n| {
        n.add("VCC", lib("core.vdc"), &["vcc", "0"], &[("V", 5.0)]);
        n.add("RP", lib("core.resistor"), &["vcc", "Y1"], &[("R", 10e3)]);
    });
    b.set("A1", false);
    b.run(1e-6);
    // Inverted high: the output floats and the pull-up wins over the 100 kΩ load.
    let v = b.c.voltage("Y1").unwrap();
    assert!((v - 5.0 * 100.0 / 110.0).abs() < 0.05, "{v}");
    b.set("A1", true);
    b.run(1e-6);
    assert!(b.c.voltage("Y1").unwrap() < 0.5);
}

#[test]
fn bilateral_switch_conducts_when_on() {
    let mut b = Bench::new("ic.4066", |n| {
        n.add("VS", lib("core.vdc"), &["X1", "0"], &[("V", 1.0)]);
    });
    b.run(1e-6);
    assert!(b.c.voltage("Y1").unwrap() < 1e-3, "off");
    b.set("C1", true);
    b.run(1e-6);
    let v = b.c.voltage("Y1").unwrap();
    assert!((v - 100e3 / (100e3 + 270.0)).abs() < 1e-3, "{v}");
}

#[test]
fn seven_segment_decoders() {
    let mut b = Bench::new("ic.4511", |_| {});
    b.set("LT", true);
    b.set("BI", true);
    // 7 = segments a, b, c.
    for p in ["A", "B", "C"] {
        b.set(p, true);
    }
    b.run(1e-6);
    let on: Vec<bool> = ["a", "b", "c", "d", "e", "f", "g"]
        .iter()
        .map(|p| b.hi(p))
        .collect();
    assert_eq!(on, vec![true, true, true, false, false, false, false]);
}

/// The PIC templates blink their pin at 2 Hz (250 ms per toggle) in a circuit.
#[test]
fn microcontroller_templates_blink() {
    for id in ["mcu.pic16f84a", "mcu.pic24fj64ga002", "mcu.pic32mx250f128b"] {
        let base = lib(id);
        let (def, errors) = ecs_mcu::with_program(&base, &ecs_mcu::template(id));
        assert!(errors.is_empty(), "{id}: {errors:?}");
        let mut n = Netlist::new();
        let names: Vec<String> = def.pins.iter().map(|p| p.name.clone()).collect();
        for p in &names {
            n.add(
                &format!("R_{p}"),
                lib("core.resistor"),
                &[p, "0"],
                &[("R", 100e3)],
            );
        }
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        n.add("U1", Arc::new(def), &refs, &[]);
        let mut c = Circuit::compile_with(&n, Options::interactive()).unwrap();
        c.reset().unwrap();
        let mut last = c.voltage("RB0").unwrap() > 1.0;
        let mut toggles = Vec::new();
        while c.time() < 0.62 {
            c.step_interactive(50e-6);
            let now = c.voltage("RB0").unwrap() > 1.0;
            if now != last {
                toggles.push(c.time());
            }
            last = now;
        }
        // Some templates toggle right after start-up; then one every 250 ms.
        let timed: Vec<f64> = toggles.iter().copied().filter(|t| *t > 0.01).collect();
        assert_eq!(timed.len(), 2, "{id}: {toggles:?}");
        assert!(
            (timed[0] - 0.25).abs() < 0.005 && (timed[1] - 0.5).abs() < 0.005,
            "{id}: {toggles:?}"
        );
    }
}
