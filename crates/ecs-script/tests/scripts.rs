//! Phase P6 acceptance: the spec 6.2 examples load and simulate; the sandbox holds.

use ecs_script::{REFERENCE, load_component};
use ecs_sim::{Circuit, Netlist, Options};
use std::sync::Arc;

fn reference(name: &str) -> ecs_model::ComponentDef {
    let src = REFERENCE.iter().find(|r| r.0 == name).unwrap().1;
    load_component(src, name).unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn simple_diode_matches_the_builtin() {
    let js = Arc::new(reference("simple_diode.js"));
    assert_eq!(js.id, "user.simple_diode");
    assert_eq!(js.pins.len(), 2);
    let lib = ecs_lib::library();
    let op = |d: Arc<ecs_model::ComponentDef>| {
        let mut n = Netlist::new();
        n.add(
            "V1",
            lib.get("core.vdc").unwrap().clone(),
            &["in", "0"],
            &[("V", 5.0)],
        );
        n.add(
            "R1",
            lib.get("core.resistor").unwrap().clone(),
            &["in", "a"],
            &[("R", 1e3)],
        );
        n.add("D1", d, &["a", "0"], &[]);
        let mut c = Circuit::compile(&n).unwrap();
        c.dc_op().unwrap();
        c.voltage("a").unwrap()
    };
    let builtin = op(lib.get("core.diode").unwrap().clone());
    let script = op(js);
    // The built-in adds breakdown and a gmin term; forward drops agree closely.
    assert!((builtin - script).abs() < 1e-3, "{builtin} vs {script}");
}

#[test]
fn function_components_match_template_components() {
    let lib = ecs_lib::library();
    let run = |d: ecs_model::ComponentDef, v: f64| {
        let mut n = Netlist::new();
        n.add(
            "V1",
            lib.get("core.vdc").unwrap().clone(),
            &["in", "0"],
            &[("V", v)],
        );
        n.add(
            "R1",
            lib.get("core.resistor").unwrap().clone(),
            &["in", "a"],
            &[("R", 1e3)],
        );
        n.add("D1", Arc::new(d), &["a", "0"], &[]);
        let mut c = Circuit::compile(&n).unwrap();
        c.dc_op().unwrap();
        let k = c.instance_index("D1").unwrap();
        (c.voltage("a").unwrap(), c.probes(k)[0])
    };
    let f = reference("function_diode.js");
    assert_eq!(f.id, "user.function_diode");
    assert_eq!(f.lets.len(), 1);
    assert!(matches!(f.limits[0], ecs_model::LimitDef::Pn { .. }));
    for v in [0.3, 2.0, 5.0, -5.0] {
        let (a, pa) = run(reference("simple_diode.js"), v);
        let (b, pb) = run(f.clone(), v);
        assert!((a - b).abs() < 1e-6, "{v} V: {a} vs {b}");
        assert!(
            (pa - pb).abs() < 1e-9 + 1e-6 * pa.abs(),
            "{v} V: power {pa} vs {pb}"
        );
    }
    // Breakdown: 150 V reverse through 1 kΩ clamps near -100 V.
    let (a, _) = run(f.clone(), -150.0);
    assert!((a + 100.0).abs() < 1.5, "{a}");
    // Transient: the charge function is differentiable and finite past Fc·Vj.
    let mut n = Netlist::new();
    n.add(
        "V1",
        lib.get("core.vsine").unwrap().clone(),
        &["in", "0"],
        &[("Vamp", 5.0), ("freq", 1e6)],
    );
    n.add(
        "R1",
        lib.get("core.resistor").unwrap().clone(),
        &["in", "a"],
        &[("R", 100.0)],
    );
    n.add("D1", Arc::new(f), &["a", "0"], &[]);
    let mut c = Circuit::compile(&n).unwrap();
    c.dc_op().unwrap();
    for _ in 0..2000 {
        c.step_interactive(1e-9);
        assert!(c.voltage("a").unwrap().is_finite());
    }
}

#[test]
fn function_errors_name_the_line() {
    let src = "function helper(x) {\n  while (x > 0) {}\n  return x;\n}\nexport default component({\n  id: \"user.bad\",\n  pins: [\"A\", \"B\"],\n  equations(c) { c.current(\"A\", \"B\", () => helper(V(\"A\", \"B\"))); },\n});\n";
    let e = ecs_script::load_component(src, "bad.js").unwrap_err();
    assert_eq!((e.file.as_str(), e.line), ("bad.js", 2), "{e}");
    assert!(e.message.contains("while"), "{e}");
}

#[test]
fn js_dff_divides_a_clock() {
    let dff = Arc::new(reference("dff.js"));
    let lib = ecs_lib::library();
    let mut n = Netlist::new();
    n.add(
        "CK",
        lib.get("core.clock").unwrap().clone(),
        &["clk"],
        &[("freq", 1e3)],
    );
    n.add("U1", dff, &["qn", "clk", "q", "qn"], &[]);
    let mut c = Circuit::compile_with(&n, Options::interactive()).unwrap();
    c.reset().unwrap();
    let mut rises = 0;
    let mut last = c.voltage("q").unwrap() > 2.5;
    for _ in 0..10_000 {
        c.step_interactive(1e-6);
        let now = c.voltage("q").unwrap() > 2.5;
        if now && !last {
            rises += 1;
        }
        last = now;
    }
    assert!((4..=6).contains(&rises), "{rises}");
}

fn hook_error(src: &str) -> Option<String> {
    let def = Arc::new(load_component(src, "evil.js").unwrap());
    let lib = ecs_lib::library();
    let mut n = Netlist::new();
    n.add(
        "SW",
        lib.get("core.logic_toggle").unwrap().clone(),
        &["a"],
        &[],
    );
    n.add("U1", def.clone(), &["a", "y"], &[]);
    let mut c = Circuit::compile_with(&n, Options::interactive()).unwrap();
    let start = std::time::Instant::now();
    c.reset().unwrap();
    let sw = c.instance_index("SW").unwrap();
    c.control(sw, "on", 1.0);
    for _ in 0..20 {
        c.step_interactive(1e-6);
    }
    assert!(
        start.elapsed().as_secs() < 5,
        "a runaway hook must be stopped"
    );
    def.behavior.as_ref().unwrap().diagnostics()
}

const EVIL: &str = r#"
export default component({
  id: "test.evil", pins: { A: digital(), Y: digitalOut() },
  onInputs(ctx) { BODY }
});
"#;

#[test]
fn sandbox_blocks_eval_function_and_host_access() {
    for body in [
        "eval('1')",
        "Function('return 1')()",
        "(function(){}).constructor('return 1')()",
        "(async function(){}).constructor('return 1')()",
        "require('fs')",
        "std.loadFile('/etc/passwd')",
        "os.exec(['ls'])",
        "fetch('http://example.com')",
        "setTimeout(() => {}, 1)",
    ] {
        let err = hook_error(&EVIL.replace("BODY", body));
        assert!(err.is_some(), "{body} should fail");
    }
}

#[test]
fn runaway_hook_is_stopped_by_the_time_budget() {
    let err = hook_error(&EVIL.replace("BODY", "while (true) {}")).expect("stopped");
    assert!(err.contains("time budget"), "{err}");
}

#[test]
fn runaway_load_is_stopped() {
    let err = load_component(
        "while (true) {}\nexport default component({id: 'x', pins: []});",
        "loop.js",
    )
    .unwrap_err();
    assert!(err.message.contains("time budget"), "{err}");
}

#[test]
fn template_errors_point_at_the_file_line() {
    let src = "export default component({\n  id: 'bad', pins: ['A', 'K'],\n  params: { Is: param(1e-14) },\n  equations(c) {\n    c.current('A', 'K', e`Is * (V(A, K)`);\n  },\n});\n";
    let err = load_component(src, "bad.js").unwrap_err();
    assert_eq!(err.file, "bad.js");
    assert_eq!(err.line, 5, "{err}");
    assert!(err.message.contains("')'"), "{err}");
}
