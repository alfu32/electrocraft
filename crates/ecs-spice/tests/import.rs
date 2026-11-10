//! Phase P7 import tests. Closed-form references always; ngspice when installed.

use ecs_sim::analysis::{self, Progress};
use ecs_sim::{Circuit, Options};
use ecs_spice::{AnalysisCard, export, import};
use std::path::Path;

const OPAMP_LIB: &str = r#"
* Vendor-style op-amp macromodel: input resistance, a single-pole gain stage
* with GBW = 1 MHz (A0 = 1e5, pole at 10 Hz) and a buffered output.
.subckt OPA1 inp inn out
Rin inp inn 10meg
E1 n1 0 inp inn 1e5
R1 n1 n2 1k
C1 n2 0 {1/(2*3.14159265*10*1k)}
E2 n3 0 n2 0 1
Rout n3 out 10
.ends OPA1
"#;

fn load(text: &str) -> ecs_spice::Import {
    import(text, "test.cir", Path::new("."), &|p| {
        if p.ends_with("opa1.lib") {
            Some(OPAMP_LIB.to_string())
        } else {
            None
        }
    })
    .unwrap_or_else(|e| panic!("{e}"))
}

fn trace<'a>(r: &'a analysis::ResultSet, n: &str) -> &'a analysis::Trace {
    r.traces.iter().find(|t| t.name == n).unwrap_or_else(|| {
        panic!(
            "{n} in {:?}",
            r.traces.iter().map(|t| &t.name).collect::<Vec<_>>()
        )
    })
}

#[test]
fn transistor_model_imports_with_a_report_and_biases_correctly() {
    let imp = load(
        "* common-emitter bias
.model Q2N2222 NPN(IS=14.34f XTI=3 EG=1.11 VAF=74.03 BF=255.9 NE=1.307 ISE=14.34f IKF=.2847
+ BR=6.092 RC=1 CJC=7.306p CJE=22.01p TF=411.1p RB=10 Mfg=Philips)
.param rbias=1meg
VCC vcc 0 DC 12
RB vcc b {rbias}
RC vcc c 2.2k
Q1 c b 0 Q2N2222
.op
.end
",
    );
    assert_eq!(imp.title, "common-emitter bias");
    let q = imp.defs.iter().find(|d| d.id == "spice.q2n2222").unwrap();
    assert_eq!(
        q.params.iter().find(|p| p.name == "Bf").unwrap().default,
        255.9
    );
    assert!(
        imp.report
            .iter()
            .any(|r| r.contains("Q2N2222") && r.contains("IKF")),
        "{:?}",
        imp.report
    );
    assert_eq!(imp.analyses, vec![AnalysisCard::Op]);
    let mut c = Circuit::compile(&imp.netlist).unwrap();
    c.dc_op().unwrap();
    // Hand analysis in the active region: Ib = (12 - Vbe) / 1M and
    // Ic = Bf * Ib * (1 + Vcb / Vaf) (the transport model's Early factor).
    let vbe = c.voltage("b").unwrap();
    let ib = (12.0 - vbe) / 1e6;
    let vc = c.voltage("c").unwrap();
    let ic = (12.0 - vc) / 2.2e3;
    let beta = ic / ib;
    let expected = 255.9 * (1.0 + (vc - vbe) / 74.03);
    assert!(
        (beta - expected).abs() / expected < 0.01,
        "beta {beta} vs {expected}"
    );
    assert!((0.55..0.75).contains(&vbe), "{vbe}");
}

#[test]
fn opamp_subckt_active_lowpass_bode_matches_closed_form() {
    // Inverting low-pass: gain -R2/R1 with corner 1/(2π R2 C).
    let imp = load(
        "Active low-pass
.include opa1.lib
VIN in 0 DC 0 AC 1
R1 in inv 10k
R2 inv out 100k
C2 inv out 1.59155n
X1 0 inv out OPA1
.ac dec 10 10 100k
.end
",
    );
    assert_eq!(imp.title, "Active low-pass");
    let AnalysisCard::Ac {
        per_decade,
        start,
        stop,
    } = imp.analyses[0]
    else {
        panic!()
    };
    let freqs = analysis::log_points(start, stop, per_decade);
    let r = analysis::ac(
        &imp.netlist,
        &freqs,
        Options::analysis(),
        &Progress::default(),
    )
    .unwrap();
    let out = trace(&r, "V(out)");
    let (mag, ph) = (out.magnitude(), out.phase_deg());
    for (k, f) in freqs.iter().enumerate().filter(|(_, f)| **f <= 10e3) {
        // Ideal response; the op-amp's finite GBW matters only well above 10 kHz.
        let wrc = 2.0 * std::f64::consts::PI * f * 100e3 * 1.59155e-9;
        let ideal = 10.0 / (1.0 + wrc * wrc).sqrt();
        assert!(
            (mag[k] - ideal).abs() / ideal < 0.02,
            "{f} Hz: {} vs {ideal}",
            mag[k]
        );
        let ideal_ph = 180.0 - wrc.atan().to_degrees();
        let got = if ph[k] < 0.0 { ph[k] + 360.0 } else { ph[k] };
        assert!(
            (got - ideal_ph).abs() < 2.0,
            "{f} Hz phase {got} vs {ideal_ph}"
        );
    }
    // The subcircuit is a library component with its ports as pins.
    let x = imp.defs.iter().find(|d| d.id == "spice.opa1").unwrap();
    assert_eq!(
        x.pins.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
        ["inp", "inn", "out"]
    );
}

#[test]
fn behavioural_and_current_controlled_sources() {
    let imp = load(
        "* B and F sources
V1 a 0 DC 2
R1 a 0 1k
VS a b DC 0
R2 b 0 1k
B1 y 0 V={V(a)*V(a)}
R3 y 0 1k
F1 0 m VS 3
R4 m 0 1k
.op
.end
",
    );
    let mut c = Circuit::compile(&imp.netlist).unwrap();
    c.dc_op().unwrap();
    assert!((c.voltage("y").unwrap() - 4.0).abs() < 1e-6);
    // VS carries 2 mA; F1 mirrors 6 mA into 1 kΩ.
    assert!(
        (c.voltage("m").unwrap() - 6.0).abs() < 1e-3,
        "{}",
        c.voltage("m").unwrap()
    );
}

#[test]
fn export_round_trips_the_operating_point() {
    let imp = load(
        "* divider\nV1 in 0 DC 9\nR1 in out 1k\nR2 out 0 2k\nD1 out 0 DX\n.model DX D(IS=1e-14 N=1)\n.op\n.end\n",
    );
    let mut a = Circuit::compile(&imp.netlist).unwrap();
    a.dc_op().unwrap();
    let text = export(&imp.netlist, "round trip");
    let again = load(&text);
    let mut b = Circuit::compile(&again.netlist).unwrap();
    b.dc_op().unwrap();
    assert!(
        (a.voltage("out").unwrap() - b.voltage("out").unwrap()).abs() < 1e-6,
        "{text}"
    );
}

#[test]
fn matches_ngspice_when_installed() {
    let Ok(out) = std::process::Command::new("ngspice")
        .arg("--version")
        .output()
    else {
        eprintln!("ngspice not installed: skipped");
        return;
    };
    if !out.status.success() {
        return;
    }
    let deck = "* ngspice cross-check\nV1 in 0 DC 5\nR1 in a 1k\nD1 a b DX\nR2 b 0 2k\nQ1 c b 0 QX\nRC in c 1k\n.model DX D(IS=1e-14 N=1)\n.model QX NPN(IS=1e-14 BF=100 BR=1 VAF=100)\n.end\n";
    let imp = load(deck);
    let mut c = Circuit::compile(&imp.netlist).unwrap();
    c.dc_op().unwrap();
    let dir = std::env::temp_dir().join(format!("ecs-ngspice-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("deck.cir");
    std::fs::write(
        &file,
        deck.replace(".end\n", ".control\nop\nprint all\n.endc\n.end\n"),
    )
    .unwrap();
    let out = std::process::Command::new("ngspice")
        .arg("-b")
        .arg(&file)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    for node in ["a", "b", "c"] {
        let line = text
            .lines()
            .find(|l| l.trim_start().starts_with(&format!("{node} = ")))
            .unwrap_or_else(|| panic!("{node} in {text}"));
        let theirs: f64 = line.split('=').nth(1).unwrap().trim().parse().unwrap();
        let ours = c.voltage(node).unwrap();
        assert!(
            (ours - theirs).abs() <= 0.01 * theirs.abs().max(0.1),
            "{node}: {ours} vs ngspice {theirs}"
        );
    }
}
