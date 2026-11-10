//! Components defined inside a schematic (spec 5.5, 9.7): each has its own
//! symbol and ports and is realised by a nested or external schematic, or by
//! inline or external JavaScript. They compile into the catalog as `local.*`.

use crate::catalog::{self, Catalog};
use ecs_model::{
    BranchDef, ComponentDef, ControlDef, DefKind, LimitDef, ParamDef, ParamKind, PinDef, PinKind,
    SubElement, Subckt,
};
use ecs_schematic::symbols::{self, Symbol};
use ecs_schematic::{Implementation, LocalComponent, LocalParam, Schematic};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Symbol registry id of a local component.
pub fn symbol_id(comp_id: &str) -> String {
    format!("doc:{comp_id}")
}

/// A fresh, unused `local.*` id.
pub fn new_id(doc: &Schematic, name: &str) -> String {
    let base: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect::<String>()
        .to_lowercase();
    let base = if base.is_empty() {
        "component".to_string()
    } else {
        base
    };
    let mut id = format!("local.{base}");
    let mut k = 2;
    while doc.components.iter().any(|c| c.id == id) {
        id = format!("local.{base}{k}");
        k += 1;
    }
    id
}

fn resolve(base: Option<&Path>, path: &str) -> PathBuf {
    let p = PathBuf::from(path);
    match base {
        Some(b) if p.is_relative() => b.join(p),
        _ => p,
    }
}

/// Load an external schematic file.
pub fn load_schematic(base: Option<&Path>, path: &str) -> Result<Schematic, String> {
    let full = resolve(base, path);
    let text = std::fs::read_to_string(&full).map_err(|e| format!("{}: {e}", full.display()))?;
    Schematic::from_json(&text).map_err(|e| format!("{}: {e}", full.display()))
}

/// Parameters of a schematic-defined component.
fn params_of(c: &LocalComponent) -> Vec<ParamDef> {
    c.params
        .iter()
        .map(|p: &LocalParam| {
            ParamDef::real(
                &p.name,
                ecs_units::parse(&p.default).unwrap_or(0.0),
                &p.unit,
                &p.description,
            )
        })
        .collect()
}

/// Compile one component against the catalog as it is now.
pub fn compile(
    cat: &Catalog,
    c: &LocalComponent,
    base: Option<&Path>,
) -> Result<ComponentDef, String> {
    let sym = symbol_id(&c.id);
    let pins: Vec<String> = c.symbol.pins.iter().map(|p| p.name.clone()).collect();
    if pins.is_empty() {
        return Err("the symbol has no pins".into());
    }
    let mut def = match &c.implementation {
        Implementation::Script { source } => script_def(source, &c.id)?,
        Implementation::ExternalScript { path } => {
            let full = resolve(base, path);
            let src =
                std::fs::read_to_string(&full).map_err(|e| format!("{}: {e}", full.display()))?;
            script_def(&src, &full.display().to_string())?
        }
        Implementation::Schematic { schematic } => schematic_def(cat, c, schematic)?,
        Implementation::ExternalSchematic { path } => {
            schematic_def(cat, c, &load_schematic(base, path)?)?
        }
    };
    def.id = c.id.clone();
    def.meta.name = c.name.clone();
    if !c.category.is_empty() {
        def.meta.category = c.category.clone();
    }
    if !c.prefix.is_empty() {
        def.meta.prefix = c.prefix.clone();
    }
    if !c.description.is_empty() {
        def.meta.description = c.description.clone();
    }
    def.symbol = sym;
    for p in &def.pins {
        if !pins.contains(&p.name) {
            return Err(format!(
                "pin {} is not on the symbol (symbol pins: {})",
                p.name,
                pins.join(", ")
            ));
        }
    }
    Ok(def)
}

fn script_def(source: &str, file: &str) -> Result<ComponentDef, String> {
    ecs_script::load_component(source, file).map_err(|e| e.to_string())
}

/// A nested schematic as a subcircuit: net labels named like the symbol's
/// pins are the ports; `{expr}` parameter values refer to the component's
/// parameters.
fn schematic_def(
    cat: &Catalog,
    c: &LocalComponent,
    inner: &Schematic,
) -> Result<ComponentDef, String> {
    let built = catalog::build(cat, inner);
    if let Some(p) = built.problems.first() {
        return Err(format!("inside: {p}"));
    }
    let mut def = ComponentDef::new(&c.id, &c.name, "User", &c.prefix, "");
    for p in &c.symbol.pins {
        // A port label that nothing inside touches yet is an open port.
        if !built.nodes.contains_key(&p.name) && !inner.labels.iter().any(|l| l.name == p.name) {
            return Err(format!(
                "pin {0}: place a net label named {0} inside",
                p.name
            ));
        }
        def.pins.push(PinDef {
            name: p.name.clone(),
            kind: PinKind::Analog,
        });
    }
    def.params = params_of(c);
    let mut elements = Vec::new();
    for inst in &built.netlist.instances {
        let exprs: BTreeMap<String, ecs_expr::Expr> = built
            .param_exprs
            .get(&inst.name)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect();
        for e in exprs.values() {
            let mut bad = None;
            e.visit(&mut |x| {
                if let ecs_expr::Expr::Name(n) = x
                    && !def.params.iter().any(|p| &p.name == n)
                {
                    bad = Some(n.clone());
                }
            });
            if let Some(n) = bad {
                return Err(format!(
                    "{}: {n} is not a parameter of {}",
                    inst.name, c.name
                ));
            }
        }
        elements.push(SubElement {
            name: inst.name.clone(),
            def: inst.def.clone(),
            nodes: inst
                .pins
                .iter()
                .map(|&k| built.netlist.nodes[k].clone())
                .collect(),
            params: inst.params.clone(),
            exprs,
        });
    }
    if elements.is_empty() {
        return Err("the schematic is empty".into());
    }
    def.kind = DefKind::Subcircuit(Arc::new(Subckt { elements }));
    Ok(def)
}

/// Compile every local component of `doc` into `cat` (replacing the old
/// ones). Components may use each other; returns problems by component.
pub fn compile_all(cat: &mut Catalog, doc: &Schematic, base: Option<&Path>) -> Vec<String> {
    cat.lib.remove_prefix("local.");
    for c in &doc.components {
        let mut s = c.symbol.clone();
        s.id = symbol_id(&c.id);
        if !symbols::exists(&s.id) || symbols::get(&s.id) != &s {
            symbols::register(s);
        }
    }
    let mut pending: Vec<&LocalComponent> = doc.components.iter().collect();
    let mut errors: BTreeMap<String, String> = BTreeMap::new();
    // Repeat while progress is made, so a component can use one defined later.
    loop {
        let mut next = Vec::new();
        let before = pending.len();
        for c in pending {
            match compile(cat, c, base) {
                Ok(def) => {
                    errors.remove(&c.id);
                    cat.lib.insert(def);
                }
                Err(e) => {
                    errors.insert(c.id.clone(), e);
                    next.push(c);
                }
            }
        }
        pending = next;
        if pending.is_empty() || pending.len() == before {
            break;
        }
    }
    errors
        .into_iter()
        .map(|(id, e)| {
            let name = doc
                .components
                .iter()
                .find(|c| c.id == id)
                .map_or(id.clone(), |c| c.name.clone());
            format!("Component {name}: {e}")
        })
        .collect()
}

fn js_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

fn js_num(v: f64) -> String {
    if v.is_finite() {
        format!("{v:?}")
    } else {
        "0".into()
    }
}

/// JavaScript source equivalent to a library definition, for editing a copy.
/// Definitions with Rust behaviour or subcircuits have no JS form.
pub fn def_to_js(def: &ComponentDef, id: &str) -> Result<String, String> {
    if def.behavior.is_some() {
        return Err(format!(
            "{} is event-driven in Rust; start from a template instead",
            def.meta.name
        ));
    }
    if matches!(def.kind, DefKind::Subcircuit(_)) {
        return Err(format!(
            "{} is a subcircuit; edit it as a schematic instead",
            def.meta.name
        ));
    }
    if def.kind == DefKind::Ground {
        return Err("ground has no equations".into());
    }
    let mut o = String::new();
    o.push_str(&format!(
        "// Copied from {}.\nexport default component({{\n",
        def.id
    ));
    o.push_str(&format!(
        "  id: {},\n  name: {},\n  category: {},\n  prefix: {},\n",
        js_str(id),
        js_str(&def.meta.name),
        js_str(&def.meta.category),
        js_str(&def.meta.prefix)
    ));
    if !def.meta.description.is_empty() {
        o.push_str(&format!(
            "  description: {},\n",
            js_str(&def.meta.description)
        ));
    }
    let analog = def.pins.iter().all(|p| p.kind == PinKind::Analog);
    if analog {
        let names: Vec<String> = def.pins.iter().map(|p| js_str(&p.name)).collect();
        o.push_str(&format!("  pins: [{}],\n", names.join(", ")));
    } else {
        o.push_str("  pins: {\n");
        for p in &def.pins {
            let k = match &p.kind {
                PinKind::Analog => "null".to_string(),
                PinKind::DigitalIn(f) => format!("digital({})", js_str(f)),
                PinKind::DigitalOut(f) => format!("digitalOut({})", js_str(f)),
            };
            o.push_str(&format!("    {}: {k},\n", js_str(&p.name)));
        }
        o.push_str("  },\n");
    }
    if !def.params.is_empty() {
        o.push_str("  params: {\n");
        for p in &def.params {
            let mut opts = Vec::new();
            if p.live {
                opts.push("live: true".to_string());
            }
            if let Some(v) = p.min {
                opts.push(format!("min: {}", js_num(v)));
            }
            if let Some(v) = p.max {
                opts.push(format!("max: {}", js_num(v)));
            }
            if let ParamKind::Enum(names) = &p.kind {
                opts.push(format!(
                    "values: [{}]",
                    names
                        .iter()
                        .map(|n| js_str(n))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            let opts = if opts.is_empty() {
                String::new()
            } else {
                format!(", {{ {} }}", opts.join(", "))
            };
            o.push_str(&format!(
                "    {}: param({}, {}, {}{opts}),\n",
                js_str(&p.name),
                js_num(p.default),
                js_str(&p.unit),
                js_str(&p.description)
            ));
        }
        o.push_str("  },\n");
    }
    if !def.states.is_empty() {
        let s: Vec<String> = def
            .states
            .iter()
            .map(|s| format!("{}: {}", js_str(&s.name), js_num(s.initial)))
            .collect();
        o.push_str(&format!("  states: {{ {} }},\n", s.join(", ")));
    }
    if !def.controls.is_empty() {
        o.push_str("  controls: [\n");
        for c in &def.controls {
            match c {
                ControlDef::Toggle { state, label } => o.push_str(&format!(
                    "    {{ kind: \"toggle\", state: {}, label: {} }},\n",
                    js_str(state),
                    js_str(label)
                )),
                ControlDef::Momentary { state, label } => o.push_str(&format!(
                    "    {{ kind: \"momentary\", state: {}, label: {} }},\n",
                    js_str(state),
                    js_str(label)
                )),
                ControlDef::Slider { param, label } => o.push_str(&format!(
                    "    {{ kind: \"slider\", param: {}, label: {} }},\n",
                    js_str(param),
                    js_str(label)
                )),
            }
        }
        o.push_str("  ],\n");
    }
    let e = |x: &ecs_expr::Expr| format!("e`{x}`");
    o.push_str("  equations(c) {\n");
    for n in &def.internal_nodes {
        o.push_str(&format!("    c.node({});\n", js_str(n)));
    }
    for (name, x) in &def.lets {
        o.push_str(&format!("    c.let({}, {});\n", js_str(name), e(x)));
    }
    for b in &def.branches {
        match b {
            BranchDef::Current { a, b, i, q, .. } => {
                let q = q
                    .as_ref()
                    .map(|q| format!(", {}", e(q)))
                    .unwrap_or_default();
                o.push_str(&format!(
                    "    c.current({}, {}, {}{q});\n",
                    js_str(a),
                    js_str(b),
                    e(i)
                ));
            }
            BranchDef::Voltage {
                name,
                a,
                b,
                v,
                flux,
            } => {
                let flux = flux.as_ref().map_or("null".to_string(), e);
                o.push_str(&format!(
                    "    c.voltage({}, {}, {}, {flux}, {});\n",
                    js_str(a),
                    js_str(b),
                    e(v),
                    js_str(name)
                ));
            }
        }
    }
    for l in &def.limits {
        let LimitDef::Pn { a, b, vte, is } = l;
        o.push_str(&format!(
            "    c.limit(\"pn\", e`V({a}, {b})`, {{ vte: {}, is: {} }});\n",
            e(vte),
            e(is)
        ));
    }
    for p in &def.probes {
        o.push_str(&format!(
            "    c.probe({}, {}, {});\n",
            js_str(&p.name),
            e(&p.expr),
            js_str(&p.unit)
        ));
    }
    o.push_str("  },\n});\n");
    Ok(o)
}

/// Starting JavaScript for a new component with the given pins, written
/// with plain functions (an `e`-template variant is shown in a comment).
pub fn template_js(id: &str, name: &str, pins: &[String]) -> String {
    let list: Vec<String> = pins.iter().map(|p| js_str(p)).collect();
    let (a, b) = (
        pins.first().cloned().unwrap_or("A".into()),
        pins.get(1).cloned().unwrap_or("B".into()),
    );
    let (qa, qb) = (js_str(&a), js_str(&b));
    format!(
        "// Equations are plain functions: Electrocraft compiles them (spec 6.1).
// Their parameters name the component's parameters, states and c.let values;
// V(\"A\", \"B\") reads a voltage and I(\"name\") a branch current. Inside:
// const/let, if/else, return, ?:, Math.*, limexp, select, smoothstep and
// helper functions declared in this file.

/** Ohm's law with an optional soft current limit. */
function conductance(v, R, Imax) {{
  const i = v / R;
  if (Imax <= 0) {{
    return i;
  }}
  return Imax * Math.tanh(i / Imax);
}}

export default component({{
  id: {},
  name: {},
  category: \"User\",
  prefix: \"U\",
  pins: [{}],
  params: {{
    R: param(1e3, \"Ω\", \"Resistance\"),
    Imax: param(0, \"A\", \"Current limit (0: none)\"),
  }},
  equations(c) {{
    c.let(\"Ir\", ({{ R, Imax }}) => conductance(V({qa}, {qb}), R, Imax));
    c.current({qa}, {qb}, ({{ Ir }}) => Ir);
    // The same with a template: c.current({qa}, {qb}, e`V({a}, {b}) / R`);
    c.probe(\"P\", ({{ Ir }}) => V({qa}, {qb}) * Ir, \"W\");
  }},
}});
",
        js_str(id),
        js_str(name),
        list.join(", "),
    )
}

/// An editable local copy of a library definition, keeping its symbol.
pub fn local_copy(doc: &Schematic, def: &ComponentDef) -> (LocalComponent, Option<String>) {
    let id = new_id(doc, &def.meta.name);
    let mut symbol: Symbol = symbols::get(&def.symbol).clone();
    symbol.id = symbol_id(&id);
    let (source, note) = match def_to_js(def, &id) {
        Ok(js) => (js, None),
        Err(why) => {
            let pins: Vec<String> = def.pins.iter().map(|p| p.name.clone()).collect();
            (template_js(&id, &def.meta.name, &pins), Some(why))
        }
    };
    let c = LocalComponent {
        id,
        name: format!("{} (copy)", def.meta.name),
        category: "User".into(),
        prefix: def.meta.prefix.clone(),
        description: def.meta.description.clone(),
        symbol,
        params: Vec::new(),
        implementation: Implementation::Script { source },
    };
    (c, note)
}

/// A new component with a two-pin box symbol.
pub fn blank(doc: &Schematic) -> LocalComponent {
    let id = new_id(doc, "Component");
    let pins = vec!["A".to_string(), "B".to_string()];
    let mut symbol = symbols::boxed(&symbol_id(&id), "NEW", &["A"], &["B"]);
    symbol.id = symbol_id(&id);
    LocalComponent {
        id: id.clone(),
        name: "New component".into(),
        category: "User".into(),
        prefix: "U".into(),
        description: String::new(),
        symbol,
        params: Vec::new(),
        implementation: Implementation::Script {
            source: template_js(&id, "New component", &pins),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ecs_schematic::{Command, Item, P2};

    fn add(
        doc: &mut Schematic,
        def: &str,
        symbol: &str,
        refdes: &str,
        at: P2,
        params: &[(&str, &str)],
    ) -> Id {
        let mut p = ecs_schematic::new_part(def, symbol, refdes.into(), at);
        for (k, v) in params {
            p.params.insert(k.to_string(), v.to_string());
        }
        match doc.apply(Command::AddPart(Box::new(p)))[0] {
            Item::Part(id) => id,
            _ => unreachable!(),
        }
    }
    fn pin(doc: &Schematic, id: Id, name: &str) -> P2 {
        doc.part(id)
            .unwrap()
            .pins()
            .into_iter()
            .find(|p| p.0 == name)
            .unwrap()
            .1
    }
    fn label(doc: &mut Schematic, at: P2, name: &str) {
        doc.apply(Command::AddLabel {
            at,
            name: name.into(),
        });
    }
    fn ground_at(doc: &mut Schematic, at: P2, refdes: &str) {
        let g = symbols::get("ground").pins[0].at;
        add(
            doc,
            "core.ground",
            "ground",
            refdes,
            [at[0] - g[0], at[1] - g[1]],
            &[],
        );
    }
    use ecs_schematic::Id;

    /// A divider block drawn as a schematic, with a parameter for its lower resistor.
    fn divider_doc() -> Schematic {
        let mut inner = Schematic::default();
        let r1 = add(
            &mut inner,
            "core.resistor",
            "resistor",
            "R1",
            [0.0, 0.0],
            &[("R", "1k")],
        );
        let r2 = add(
            &mut inner,
            "core.resistor",
            "resistor",
            "R2",
            [200.0, 0.0],
            &[("R", "{Rb}")],
        );
        let (a1, b1, a2, b2) = (
            pin(&inner, r1, "A"),
            pin(&inner, r1, "B"),
            pin(&inner, r2, "A"),
            pin(&inner, r2, "B"),
        );
        label(&mut inner, a1, "IN");
        label(&mut inner, b1, "OUT");
        label(&mut inner, a2, "OUT");
        ground_at(&mut inner, b2, "G1");
        let mut symbol = symbols::boxed("doc:local.div", "DIV", &["IN"], &["OUT"]);
        symbol.id = symbol_id("local.div");
        let comp = LocalComponent {
            id: "local.div".into(),
            name: "Divider".into(),
            category: "User".into(),
            prefix: "X".into(),
            description: String::new(),
            symbol,
            params: vec![LocalParam {
                name: "Rb".into(),
                default: "1k".into(),
                unit: "Ω".into(),
                description: String::new(),
            }],
            implementation: Implementation::Schematic {
                schematic: Box::new(inner),
            },
        };
        let mut doc = Schematic::default();
        doc.components.push(comp);
        let v1 = add(&mut doc, "core.vdc", "vdc", "V1", [0.0, 0.0], &[("V", "4")]);
        let (p, n) = (pin(&doc, v1, "P"), pin(&doc, v1, "N"));
        label(&mut doc, p, "in");
        ground_at(&mut doc, n, "G1");
        doc
    }

    #[test]
    fn schematic_component_compiles_to_a_parameterised_subcircuit() {
        let mut cat = Catalog::builtin();
        let mut doc = divider_doc();
        let problems = compile_all(&mut cat, &doc, None);
        assert!(problems.is_empty(), "{problems:?}");
        let x1 = add(
            &mut doc,
            "local.div",
            &symbol_id("local.div"),
            "X1",
            [300.0, 0.0],
            &[("Rb", "3k")],
        );
        let (i, o) = (pin(&doc, x1, "IN"), pin(&doc, x1, "OUT"));
        label(&mut doc, i, "in");
        label(&mut doc, o, "out");
        let built = catalog::build(&cat, &doc);
        assert!(built.problems.is_empty(), "{:?}", built.problems);
        let mut c = ecs_sim::Circuit::compile(&built.netlist).unwrap();
        c.dc_op().unwrap();
        let v = c.voltage("out").unwrap();
        assert!((v - 3.0).abs() < 1e-6, "{v}");
        // The scope sees the block's terminals.
        let vars = crate::scope::variables(&cat, &doc, &built);
        assert!(vars.get("X1.OUT.V").is_some());
    }

    #[test]
    fn missing_ports_and_unknown_parameters_are_reported() {
        let mut cat = Catalog::builtin();
        let mut doc = divider_doc();
        if let Implementation::Schematic { schematic } = &mut doc.components[0].implementation {
            schematic.labels.retain(|l| l.name != "OUT");
        }
        let problems = compile_all(&mut cat, &doc, None);
        assert!(problems[0].contains("OUT"), "{problems:?}");
        let mut doc = divider_doc();
        doc.components[0].params.clear();
        let problems = compile_all(&mut cat, &doc, None);
        assert!(problems[0].contains("Rb"), "{problems:?}");
    }

    #[test]
    fn library_parts_convert_to_javascript_and_back() {
        let cat = Catalog::builtin();
        let mut converted = 0;
        for def in cat.lib.iter() {
            let Ok(js) = def_to_js(def, "local.copy") else {
                continue;
            };
            let back = ecs_script::load_component(&js, "copy.js")
                .unwrap_or_else(|e| panic!("{}: {e}\n{js}", def.id));
            converted += 1;
            assert_eq!(back.pins.len(), def.pins.len(), "{}", def.id);
            assert_eq!(back.params.len(), def.params.len(), "{}", def.id);
            for (a, b) in back.params.iter().zip(&def.params) {
                assert_eq!(
                    (a.name.as_str(), a.default, a.live),
                    (b.name.as_str(), b.default, b.live),
                    "{}",
                    def.id
                );
            }
            assert_eq!(back.branches.len(), def.branches.len(), "{}", def.id);
            for (a, b) in back.branches.iter().zip(&def.branches) {
                assert_eq!(format!("{a:?}"), format!("{b:?}"), "{}", def.id);
            }
            assert_eq!(back.lets.len(), def.lets.len(), "{}", def.id);
            for (a, b) in back.lets.iter().zip(&def.lets) {
                assert_eq!(a, b, "{}", def.id);
            }
            assert_eq!(back.probes.len(), def.probes.len(), "{}", def.id);
            assert_eq!(back.controls, def.controls, "{}", def.id);
        }
        assert!(converted >= 15, "{converted}");
    }

    #[test]
    fn copies_and_templates_compile() {
        let cat = Catalog::builtin();
        let doc = Schematic::default();
        for id in ["core.resistor", "core.npn", "core.and", "core.opamp"] {
            let def = cat.def(id).unwrap();
            let (mut c, _) = local_copy(&doc, def);
            c.symbol.id = symbol_id(&c.id);
            symbols::register(c.symbol.clone());
            compile(&cat, &c, None).unwrap_or_else(|e| panic!("{id}: {e}"));
        }
        let mut b = blank(&doc);
        b.symbol.id = symbol_id(&b.id);
        symbols::register(b.symbol.clone());
        compile(&cat, &b, None).unwrap();
    }
}
