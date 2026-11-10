//! Analytic circuits (spec section 11): results within 0.1% of closed form.

use ecs_lib::library;
use ecs_sim::{Circuit, Method, Netlist, OpMethod};

fn lib(id: &str) -> std::sync::Arc<ecs_model::ComponentDef> {
    library().get(id).unwrap_or_else(|| panic!("{id}")).clone()
}

fn rel(a: f64, b: f64) -> f64 {
    (a - b).abs() / b.abs().max(1e-12)
}

#[test]
fn voltage_divider() {
    let mut n = Netlist::new();
    n.add("V1", lib("core.vdc"), &["in", "0"], &[("V", 10.0)]);
    n.add("R1", lib("core.resistor"), &["in", "out"], &[("R", 3e3)]);
    n.add("R2", lib("core.resistor"), &["out", "0"], &[("R", 1e3)]);
    let mut c = Circuit::compile(&n).unwrap();
    assert!(c.is_linear());
    assert_eq!(c.dc_op().unwrap(), OpMethod::Newton);
    assert!(rel(c.voltage("out").unwrap(), 2.5) < 1e-6);
    // Source delivers 2.5 mA.
    let k = c.instance_index("V1").unwrap();
    assert!(rel(c.probes(k)[0], 2.5e-3) < 1e-6);
}

#[test]
fn wheatstone_bridge() {
    let mut n = Netlist::new();
    n.add("V1", lib("core.vdc"), &["top", "0"], &[("V", 12.0)]);
    n.add("R1", lib("core.resistor"), &["top", "a"], &[("R", 1e3)]);
    n.add("R2", lib("core.resistor"), &["a", "0"], &[("R", 2e3)]);
    n.add("R3", lib("core.resistor"), &["top", "b"], &[("R", 3e3)]);
    n.add("R4", lib("core.resistor"), &["b", "0"], &[("R", 4e3)]);
    n.add("R5", lib("core.resistor"), &["a", "b"], &[("R", 5e3)]);
    let mut c = Circuit::compile(&n).unwrap();
    c.dc_op().unwrap();
    // Nodal analysis by hand: solve the 2x2 system.
    let (g1, g2, g3, g4, g5) = (1e-3, 0.5e-3, 1.0 / 3e3, 0.25e-3, 0.2e-3);
    let a11 = g1 + g2 + g5;
    let a22 = g3 + g4 + g5;
    let (b1, b2) = (12.0 * g1, 12.0 * g3);
    let det = a11 * a22 - g5 * g5;
    let va = (b1 * a22 + g5 * b2) / det;
    let vb = (a11 * b2 + g5 * b1) / det;
    assert!(rel(c.voltage("a").unwrap(), va) < 1e-6);
    assert!(rel(c.voltage("b").unwrap(), vb) < 1e-6);
}

#[test]
fn rc_charge_and_factor_reuse() {
    let (r, cap, v) = (1e3, 1e-6, 5.0);
    let tau = r * cap;
    let mut n = Netlist::new();
    n.add("V1", lib("core.vdc"), &["in", "0"], &[("V", v)]);
    n.add("R1", lib("core.resistor"), &["in", "out"], &[("R", r)]);
    n.add("C1", lib("core.capacitor"), &["out", "0"], &[("C", cap)]);
    // Start from the all-zero state: the capacitor is discharged at t = 0.
    let mut c = Circuit::compile(&n).unwrap();
    let h = tau / 1000.0;
    for _ in 0..2000 {
        c.step(h, Method::Trapezoidal).unwrap();
    }
    let t = c.time();
    let expected = v * (1.0 - (-t / tau).exp());
    assert!(
        rel(c.voltage("out").unwrap(), expected) < 1e-3,
        "{} vs {expected}",
        c.voltage("out").unwrap()
    );
    // Linear fast path: one factorization for the Euler step, one for trapezoidal.
    assert!(c.factorizations() <= 2, "{}", c.factorizations());
}

#[test]
fn rl_current_rise() {
    let (r, l, v) = (100.0, 10e-3, 1.0);
    let tau = l / r;
    let mut n = Netlist::new();
    n.add("V1", lib("core.vdc"), &["in", "0"], &[("V", v)]);
    n.add("R1", lib("core.resistor"), &["in", "x"], &[("R", r)]);
    n.add("L1", lib("core.inductor"), &["x", "0"], &[("L", l)]);
    let mut c = Circuit::compile(&n).unwrap();
    // Start from zero current rather than the DC point (inductor would be shorted).
    let h = tau / 1000.0;
    for _ in 0..1000 {
        c.step(h, Method::Trapezoidal).unwrap();
    }
    let k = c.instance_index("L1").unwrap();
    let i = c.branch_currents(k)[0];
    let expected = v / r * (1.0 - (-c.time() / tau).exp());
    assert!(rel(i, expected) < 1e-3, "{i} vs {expected}");
}

#[test]
fn series_rlc_ringing_frequency() {
    let (r, l, cap) = (10.0, 1e-3, 1e-6);
    let mut n = Netlist::new();
    n.add("C1", lib("core.capacitor"), &["a", "0"], &[("C", cap)]);
    n.add("R1", lib("core.resistor"), &["a", "b"], &[("R", r)]);
    n.add("L1", lib("core.inductor"), &["b", "0"], &[("L", l)]);
    n.add("V1", lib("core.vdc"), &["p", "0"], &[("V", 1.0)]);
    n.add("S1", lib("core.switch"), &["p", "a"], &[]);
    let mut c = Circuit::compile(&n).unwrap();
    // Charge the capacitor through the closed switch, then open it.
    let s = c.instance_index("S1").unwrap();
    c.set_state(s, "closed", 1.0);
    c.dc_op().unwrap();
    // Opening the switch leaves 1 V on the capacitor and 0.1 A in the inductor.
    c.set_state(s, "closed", 0.0);
    let alpha = r / (2.0 * l);
    let w0 = 1.0 / (l * cap).sqrt();
    let wd = (w0 * w0 - alpha * alpha).sqrt();
    let h = 2.0 * std::f64::consts::PI / wd / 2000.0;
    let mut crossings = Vec::new();
    let mut prev = c.voltage("a").unwrap();
    let t0 = c.time();
    for _ in 0..12000 {
        c.step(h, Method::Trapezoidal).unwrap();
        let v = c.voltage("a").unwrap();
        if prev < 0.0 && v >= 0.0 {
            let frac = prev / (prev - v);
            crossings.push(c.time() - h + frac * h - t0);
        }
        prev = v;
    }
    assert!(crossings.len() >= 4, "{crossings:?}");
    let period = (crossings[crossings.len() - 1] - crossings[0]) / (crossings.len() - 1) as f64;
    let expected = 2.0 * std::f64::consts::PI / wd;
    assert!(rel(period, expected) < 1e-3, "{period} vs {expected}");
}

#[test]
fn diode_forward_drop_and_limiting() {
    let mut n = Netlist::new();
    n.add("V1", lib("core.vdc"), &["in", "0"], &[("V", 10.0)]);
    n.add("R1", lib("core.resistor"), &["in", "a"], &[("R", 1e3)]);
    n.add("D1", lib("core.diode"), &["a", "0"], &[]);
    let mut c = Circuit::compile(&n).unwrap();
    assert!(!c.is_linear());
    c.dc_op().unwrap();
    let vd = c.voltage("a").unwrap();
    // Solve i = (10 - v)/1k = Is (exp(v/vt) - 1) by bisection for the reference.
    let vt = 8.617_333_262e-5 * 300.15;
    let f = |v: f64| (10.0 - v) / 1e3 - 1e-14 * ((v / vt).exp() - 1.0) - 1e-12 * v;
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if f(mid) > 0.0 { lo = mid } else { hi = mid }
    }
    assert!((vd - lo).abs() < 1e-5, "{vd} vs {lo}");
}

#[test]
fn rc_lowpass_driven_by_sine_settles_to_expected_amplitude() {
    let (r, cap, f) = (1e3, 1e-6, 159.154_943);
    let mut n = Netlist::new();
    n.add(
        "V1",
        lib("core.vsine"),
        &["in", "0"],
        &[("Vamp", 1.0), ("freq", f)],
    );
    n.add("R1", lib("core.resistor"), &["in", "out"], &[("R", r)]);
    n.add("C1", lib("core.capacitor"), &["out", "0"], &[("C", cap)]);
    let mut c = Circuit::compile(&n).unwrap();
    let period = 1.0 / f;
    let h = period / 2000.0;
    let mut peak: f64 = 0.0;
    for k in 0..(20 * 2000) {
        c.step(h, Method::Trapezoidal).unwrap();
        if k > 15 * 2000 {
            peak = peak.max(c.voltage("out").unwrap());
        }
    }
    // At the corner frequency the gain is 1/sqrt(2).
    assert!(rel(peak, std::f64::consts::FRAC_1_SQRT_2) < 2e-3, "{peak}");
}

#[test]
fn voltage_source_loop_is_reported() {
    let mut n = Netlist::new();
    n.add("V1", lib("core.vdc"), &["a", "0"], &[]);
    n.add("V2", lib("core.vdc"), &["a", "0"], &[]);
    assert!(matches!(
        Circuit::compile(&n),
        Err(ecs_sim::SimError::VoltageLoop(_))
    ));
}

#[test]
fn floating_node_is_warned() {
    let mut n = Netlist::new();
    n.add("V1", lib("core.vdc"), &["a", "0"], &[]);
    n.add("VM1", lib("core.voltmeter"), &["b", "0"], &[]);
    let c = Circuit::compile(&n).unwrap();
    assert!(
        c.warnings.iter().any(|w| w.contains("node b")),
        "{:?}",
        c.warnings
    );
}

#[test]
fn astable_multivibrator_oscillates() {
    let mut n = Netlist::new();
    n.add("V1", lib("core.vdc"), &["vcc", "0"], &[("V", 5.0)]);
    n.add("RC1", lib("core.resistor"), &["vcc", "c1"], &[("R", 1e3)]);
    n.add("RC2", lib("core.resistor"), &["vcc", "c2"], &[("R", 1e3)]);
    n.add("RB1", lib("core.resistor"), &["vcc", "b1"], &[("R", 47e3)]);
    n.add("RB2", lib("core.resistor"), &["vcc", "b2"], &[("R", 47e3)]);
    n.add("C1", lib("core.capacitor"), &["c1", "b2"], &[("C", 10e-9)]);
    n.add("C2", lib("core.capacitor"), &["c2", "b1"], &[("C", 10e-9)]);
    n.add("Q1", lib("core.npn"), &["c1", "b1", "0"], &[]);
    n.add(
        "Q2",
        lib("core.npn"),
        &["c2", "b2", "0"],
        &[("Is", 1.1e-14)],
    );
    let mut c = Circuit::compile(&n).unwrap();
    c.reset().unwrap();
    // Expected period about 2 * 0.69 * 47k * 10n = 0.65 ms.
    let h = 1e-6;
    let mut transitions = 0;
    let mut high = c.voltage("c1").unwrap() > 2.5;
    for _ in 0..4000 {
        c.step_interactive(h);
        let now = c.voltage("c1").unwrap() > 2.5;
        if now != high {
            transitions += 1;
            high = now;
        }
    }
    assert!(transitions >= 6, "only {transitions} transitions in 4 ms");
}
