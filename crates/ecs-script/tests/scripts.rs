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
