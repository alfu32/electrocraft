//! Bridges the component library (`ecs-lib`) and the schematic: palette
//! entries, parameter values and netlist building.

use ecs_model::{ComponentDef, DefKind, Library, ParamKind};
use ecs_schematic::{Id, Part, Schematic};
use ecs_sim::Netlist;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Palette categories in display order; others follow alphabetically.
pub const CATEGORIES: [&str; 16] = [
    "Passive",
    "Sources",
    "Switches",
    "Diodes",
    "Transistors",
    "Analog ICs",
    "Digital",
    "Digital I/O",
    "74xx logic",
    "4000 logic",
    "Microcontrollers",
    "Indicators",
    "Instruments",
    "Controlled sources",
    "Semiconductors",
    "User",
];

/// A microcontroller definition running a program, and its assembly errors.
type Programmed = (Arc<ComponentDef>, Vec<String>);

/// The component catalog.
pub struct Catalog {
    pub lib: Library,
    /// Microcontroller definitions with assembled programs, by (id, source).
    programs: std::sync::Mutex<std::collections::HashMap<(String, String), Programmed>>,
}

/// Result of building a netlist from a schematic.
#[derive(Debug, Default, Clone)]
pub struct Built {
    pub netlist: Netlist,
    /// Netlist node index per net name.
    pub nodes: BTreeMap<String, usize>,
    /// Net names per part pin (symbol pin order).
    pub pins: BTreeMap<Id, Vec<String>>,
    /// Net name per wire.
    pub wires: BTreeMap<Id, String>,
    /// Part id per instance name (refdes).
    pub parts: BTreeMap<String, Id>,
    /// Problems found while building (unknown parts, unreadable values...).
    pub problems: Vec<String>,
    pub unconnected: Vec<(Id, usize)>,
    /// Parameters given as `{expression}` of a component's parameters, by
    /// instance; only meaningful inside a schematic-defined component.
    pub param_exprs: BTreeMap<String, Vec<(String, ecs_expr::Expr)>>,
}

impl Catalog {
    /// The built-in catalog.
    pub fn builtin() -> Self {
        let mut lib = ecs_lib::library();
        // Reference components shipped as JavaScript (spec 6.3).
        for (file, src) in ecs_script::REFERENCE {
            if let Ok(def) = ecs_script::load_component(src, file) {
                lib.insert(crate::packages::ensure_symbol(def));
            }
        }
        register_ic_symbols();
        Self {
            lib,
            programs: Default::default(),
        }
    }

    /// The definition a microcontroller part runs: its program assembled
    /// (cached by source, so an unchanged program keeps its definition).
    pub fn programmed(
        &self,
        def: &Arc<ComponentDef>,
        code: &str,
    ) -> (Arc<ComponentDef>, Vec<String>) {
        let key = (def.id.clone(), code.to_string());
        if let Ok(mut m) = self.programs.lock() {
            if let Some(hit) = m.get(&key) {
                return hit.clone();
            }
            let (d, errors) = ecs_mcu::with_program(def, code);
            let entry = (
                Arc::new(d),
                errors.iter().map(|e| e.to_string()).collect::<Vec<_>>(),
            );
            if m.len() > 256 {
                m.clear();
            }
            m.insert(key, entry.clone());
            return entry;
        }
        (def.clone(), Vec::new())
    }

    /// Every category present, in display order.
    pub fn categories(&self) -> Vec<String> {
        let mut v: Vec<String> = CATEGORIES.iter().map(|c| c.to_string()).collect();
        let mut extra: Vec<String> = self
            .lib
            .iter()
            .map(|d| d.meta.category.clone())
            .filter(|c| !v.contains(c))
            .collect();
        extra.sort();
        extra.dedup();
        v.extend(extra);
        v.retain(|c| !self.category(c).is_empty());
        v
    }

    /// Definition by id.
    pub fn def(&self, id: &str) -> Option<&Arc<ComponentDef>> {
        self.lib.get(id)
    }

    /// Definitions of a category, in a pleasant order.
    pub fn category(&self, cat: &str) -> Vec<Arc<ComponentDef>> {
        let order = [
            "core.resistor",
            "core.capacitor",
            "core.inductor",
            "core.potentiometer",
            "core.ground",
            "core.vdc",
            "core.battery",
            "core.vsine",
            "core.funcgen",
            "core.idc",
            "core.switch",
            "core.pushbutton",
            "core.spdt",
            "core.diode",
            "core.led",
            "core.zener",
            "core.schottky",
            "core.npn",
            "core.pnp",
            "core.nmos",
            "core.pmos",
            "core.opamp",
            "core.comparator",
        ];
        let mut v: Vec<Arc<ComponentDef>> = self
            .lib
            .iter()
            .filter(|d| d.meta.category == cat || (cat == "Indicators" && d.id == "core.led"))
            .cloned()
            .collect();
        v.sort_by_key(|d| order.iter().position(|o| *o == d.id).unwrap_or(99));
        v
    }

    /// Create a part for a definition at a position.
    pub fn new_part(&self, doc: &Schematic, def: &ComponentDef, at: ecs_schematic::P2) -> Part {
        let refdes = doc.next_refdes(&def.meta.prefix);
        let mut p = ecs_schematic::new_part(&def.id, &def.symbol, refdes, at);
        if let Some(k) = ecs_lib::parts::default_for(&def.id) {
            p.part_number = k.number.clone();
        }
        p.def_version = def.meta.version;
        for s in &def.states {
            p.states.insert(s.name.clone(), s.initial);
        }
        p
    }
}

/// Numeric value of a parameter: the override if it parses, else the default.
pub fn param_value(part: &Part, def: &ComponentDef, name: &str) -> f64 {
    let d = def
        .params
        .iter()
        .find(|p| p.name == name)
        .map_or(0.0, |p| p.default);
    match part.params.get(name) {
        Some(text) => parse_param(def, name, text).unwrap_or(d),
        None => d,
    }
}

/// Parse a typed parameter value; enum parameters accept their names.
pub fn parse_param(def: &ComponentDef, name: &str, text: &str) -> Option<f64> {
    if let Some(p) = def.params.iter().find(|p| p.name == name)
        && let ParamKind::Enum(names) = &p.kind
        && let Some(i) = names
            .iter()
            .position(|n| n.eq_ignore_ascii_case(text.trim()))
    {
        return Some(i as f64);
    }
    ecs_units::parse(text).ok()
}

/// Text for a parameter in the inspector.
pub fn param_text(part: &Part, def: &ComponentDef, name: &str) -> String {
    if let Some(t) = part.params.get(name) {
        return t.clone();
    }
    let p = def.params.iter().find(|p| p.name == name);
    match p {
        Some(p) => match &p.kind {
            ParamKind::Enum(names) => names.get(p.default as usize).cloned().unwrap_or_default(),
            _ => ecs_units::format(p.default, ""),
        },
        None => String::new(),
    }
}

/// The value shown next to a part on the canvas, e.g. `4.7 kΩ`.
pub fn value_label(part: &Part, def: &ComponentDef) -> String {
    let v = value_text(part, def);
    if v.is_empty() {
        part.part_number.clone()
    } else {
        v
    }
}

fn value_text(part: &Part, def: &ComponentDef) -> String {
    let main = match def.id.as_str() {
        "core.vsine" => {
            let a = param_value(part, def, "Vamp");
            let f = param_value(part, def, "freq");
            return format!(
                "{} {}",
                ecs_units::format(a, "V"),
                ecs_units::format(f, "Hz")
            );
        }
        "core.funcgen" => {
            let f = param_value(part, def, "freq");
            let w = param_value(part, def, "wave") as usize;
            let names = ["sine", "square", "triangle", "saw"];
            return format!(
                "{} {}",
                names.get(w).unwrap_or(&""),
                ecs_units::format(f, "Hz")
            );
        }
        "core.opamp" | "core.comparator" | "core.npn" | "core.pnp" | "core.nmos" | "core.pmos"
        | "core.diode" | "core.schottky" | "core.voltmeter" | "core.ammeter" | "core.ground"
        | "core.switch" | "core.spdt" | "core.pushbutton" | "core.led" => return String::new(),
        "core.zener" => "BV",
        _ if def.behavior.is_some() => return String::new(),
        _ => match def.params.first() {
            Some(p) => p.name.as_str(),
            None => return String::new(),
        },
    };
    let Some(p) = def.params.iter().find(|p| p.name == main) else {
        return String::new();
    };
    // `{expression}` of the enclosing component's parameters: show it as written.
    if let Some(text) = part.params.get(main)
        && text.trim_start().starts_with('{')
    {
        return text.trim().to_string();
    }
    ecs_units::format(param_value(part, def, main), &p.unit)
}

/// Boxed symbols for the logic ICs and microcontrollers (inputs left,
/// outputs right; `>` marks clock inputs).
fn register_ic_symbols() {
    use ecs_schematic::symbols;
    for s in ecs_lib::ics::symbols() {
        if !symbols::exists(&s.id) {
            let l: Vec<&str> = s.left.iter().map(String::as_str).collect();
            let r: Vec<&str> = s.right.iter().map(String::as_str).collect();
            symbols::register(symbols::boxed(&s.id, &s.title, &l, &r));
        }
    }
    for (id, title, left, right) in ecs_mcu::symbols() {
        if !symbols::exists(&id) {
            let l: Vec<&str> = left.iter().map(String::as_str).collect();
            let r: Vec<&str> = right.iter().map(String::as_str).collect();
            symbols::register(symbols::boxed(&id, &title, &l, &r));
        }
    }
}

/// Whether a part is a ground symbol.
pub fn is_ground(cat: &Catalog, part: &Part) -> bool {
    cat.def(&part.def)
        .is_some_and(|d| d.kind == DefKind::Ground)
}

/// Build the simulation netlist for a schematic.
pub fn build(cat: &Catalog, doc: &Schematic) -> Built {
    let conn = doc.connectivity(&|p| is_ground(cat, p));
    let mut b = Built {
        netlist: Netlist::new(),
        pins: conn.pins.clone(),
        wires: conn.wires.clone(),
        unconnected: conn.unconnected.clone(),
        ..Default::default()
    };
    b.nodes.insert("0".into(), 0);
    for part in &doc.parts {
        let Some(def) = cat.def(&part.def) else {
            b.problems
                .push(format!("{}: unknown component {}", part.refdes, part.def));
            continue;
        };
        // Microcontrollers run the part's own program.
        let programmed;
        let def = if ecs_mcu::is_mcu(&def.id) {
            let (d, errors) = cat.programmed(def, &part.code);
            for e in errors {
                b.problems.push(format!("{} program {e}", part.refdes));
            }
            programmed = d;
            &programmed
        } else {
            def
        };
        // Ground ties nets in connectivity; the oscilloscope reads node
        // voltages without loading the circuit or adding floating nodes.
        if def.kind == DefKind::Ground {
            continue;
        }
        let Some(nets) = conn.pins.get(&part.id) else {
            continue;
        };
        // Map definition pins to symbol pins by name.
        let sym_pins: Vec<String> = part.symbol().pins.iter().map(|p| p.name.clone()).collect();
        let mut nodes = Vec::new();
        let mut ok = true;
        for pin in &def.pins {
            match sym_pins.iter().position(|n| *n == pin.name) {
                Some(k) => nodes.push(nets[k].clone()),
                None => {
                    b.problems
                        .push(format!("{}: symbol has no pin {}", part.refdes, pin.name));
                    ok = false;
                }
            }
        }
        if !ok {
            continue;
        }
        let mut params = Vec::new();
        for p in &def.params {
            if let Some(text) = part.params.get(&p.name) {
                // `{expression}`: a parameter of the enclosing component.
                if let Some(inner) = text
                    .trim()
                    .strip_prefix('{')
                    .and_then(|t| t.strip_suffix('}'))
                {
                    match ecs_expr::parse(inner) {
                        Ok(e) => match ecs_expr::eval_with(&e, &|_| None) {
                            Ok(v) => params.push((p.name.clone(), v)),
                            Err(_) => {
                                b.param_exprs
                                    .entry(part.refdes.clone())
                                    .or_default()
                                    .push((p.name.clone(), e));
                            }
                        },
                        Err(err) => b
                            .problems
                            .push(format!("{}: {} = {text}: {err}", part.refdes, p.name)),
                    }
                    continue;
                }
                match parse_param(def, &p.name, text) {
                    Some(v) => params.push((p.name.clone(), v)),
                    None => b.problems.push(format!(
                        "{}: cannot read {} = \"{text}\"",
                        part.refdes, p.name
                    )),
                }
            }
        }
        let name = part.refdes.clone();
        let node_refs: Vec<&str> = nodes.iter().map(String::as_str).collect();
        let param_refs: Vec<(&str, f64)> = params.iter().map(|(k, v)| (k.as_str(), *v)).collect();
        b.netlist.add(&name, def.clone(), &node_refs, &param_refs);
        b.parts.insert(name, part.id);
    }
    for (i, n) in b.netlist.nodes.iter().enumerate() {
        b.nodes.insert(n.clone(), i);
    }
    // Saved control states go in as instance parameters are not states; the
    // app applies them after each rebuild.
    b
}
