//! Analysis-mode tests (spec 4.6, phase P7 engine side).

use ecs_lib::library;
use ecs_sim::analysis::{self, Progress};
use ecs_sim::{Method, Netlist, Options};

fn lib(id: &str) -> std::sync::Arc<ecs_model::ComponentDef> {
    library().get(id).unwrap().clone()
}

fn rc(freq: f64) -> Netlist {
    let mut n = Netlist::new();
    n.add(
        "V1",
        lib("core.vsine"),
        &["in", "0"],
        &[("Vamp", 1.0), ("freq", freq), ("AC", 1.0)],
    );
    n.add("R1", lib("core.resistor"), &["in", "out"], &[("R", 1e3)]);
    n.add("C1", lib("core.capacitor"), &["out", "0"], &[("C", 1e-6)]);
    n
}

fn trace<'a>(r: &'a analysis::ResultSet, name: &str) -> &'a analysis::Trace {
    r.traces
        .iter()
        .find(|t| t.name == name)
        .unwrap_or_else(|| panic!("{name}"))
}

#[test]
fn ac_rc_lowpass_corner() {
    let fc = 1.0 / (2.0 * std::f64::consts::PI * 1e3 * 1e-6);
    let r = analysis::ac(
        &rc(1e3),
        &[fc / 100.0, fc, fc * 100.0],
        Options::analysis(),
        &Progress::default(),
    )
    .unwrap();
    let out = trace(&r, "V(out)");
    let mag = out.magnitude();
    let ph = out.phase_deg();
    assert!((mag[0] - 1.0).abs() < 1e-3, "{mag:?}");
    assert!(
        (mag[1] - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-3,
        "{mag:?}"
    );
    assert!((ph[1] + 45.0).abs() < 0.1, "{ph:?}");
    assert!((mag[2] - 0.01).abs() < 1e-3, "{mag:?}");
}

#[test]
fn dc_sweep_divider_is_linear() {
    let mut n = Netlist::new();
    n.add("V1", lib("core.vdc"), &["in", "0"], &[]);
    n.add("R1", lib("core.resistor"), &["in", "out"], &[]);
    n.add("R2", lib("core.resistor"), &["out", "0"], &[]);
    let values: Vec<f64> = (0..=10).map(f64::from).collect();
    let r = analysis::dc_sweep(
        &n,
        "V1",
        "V",
        &values,
        Options::analysis(),
        &Progress::default(),
    )
    .unwrap();
    for (v, out) in values.iter().zip(&trace(&r, "V(out)").re) {
        assert!((out - v / 2.0).abs() < 1e-6);
    }
    // A non-live parameter recompiles at each point.
    let r = analysis::dc_sweep(
        &n,
        "R2",
        "R",
        &[1e3, 3e3],
        Options::analysis(),
        &Progress::default(),
    )
    .unwrap();
    // V1 keeps its default 5 V: 5 V * 3k / (1k + 3k).
    assert!((trace(&r, "V(out)").re[1] - 3.75).abs() < 1e-6);
}

#[test]
fn adaptive_transient_matches_steady_state() {
    let fc = 1.0 / (2.0 * std::f64::consts::PI * 1e-3);
    let r = analysis::transient(
        &rc(fc),
        10.0 / fc,
        1.0 / fc / 100.0,
        Options::analysis(),
        Method::Trapezoidal,
        &Progress::default(),
    )
    .unwrap();
    let out = trace(&r, "V(out)");
    let peak = r
        .axis
        .iter()
        .zip(&out.re)
        .filter(|(t, _)| **t > 8.0 / fc)
        .map(|(_, v)| *v)
        .fold(0.0f64, f64::max);
    assert!(
        (peak - std::f64::consts::FRAC_1_SQRT_2).abs() < 0.01,
        "{peak}"
    );
    assert!(
        r.axis.len() < 3000,
        "step control should not take the maximum number of steps: {}",
        r.axis.len()
    );
}

#[test]
fn parameter_step_runs_in_parallel() {
    let mut n = Netlist::new();
    n.add("V1", lib("core.vdc"), &["in", "0"], &[("V", 10.0)]);
    n.add("R1", lib("core.resistor"), &["in", "out"], &[("R", 1e3)]);
    n.add("R2", lib("core.resistor"), &["out", "0"], &[("R", 1e3)]);
    let rs = analysis::param_step(&n, "R2", "R", &[1e3, 3e3, 9e3], |n| {
        analysis::op(n, Options::analysis()).map(|r| r.0)
    });
    let outs: Vec<f64> = rs
        .iter()
        .map(|r| trace(r.as_ref().unwrap(), "V(out)").re[0])
        .collect();
    for (o, want) in outs.iter().zip([5.0, 7.5, 9.0]) {
        assert!((o - want).abs() < 1e-6, "{outs:?}");
    }
}

#[test]
fn fft_finds_a_sine() {
    let t: Vec<f64> = (0..4096).map(|k| k as f64 / 100e3).collect();
    let v: Vec<f64> = t
        .iter()
        .map(|t| 2.0 * (2.0 * std::f64::consts::PI * 1e3 * t).sin())
        .collect();
    let (f, m) = analysis::fft(&t, &v);
    let (k, peak) = m
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .unwrap();
    assert!((f[k] - 1e3).abs() < 30.0, "{}", f[k]);
    assert!((peak - 2.0).abs() < 0.4, "{peak}");
}
