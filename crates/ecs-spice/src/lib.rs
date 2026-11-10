//! SPICE import and export (spec 8.1, phase P7).
//!
//! Import reads `.model`, `.subckt`, `.param`, `.include` and `.lib` plus
//! top-level element cards and analysis cards. Each `.model` becomes a copy
//! of the matching built-in definition with its parameters filled in; each
//! `.subckt` becomes a subcircuit definition flattened at compile time.
//! Anything ignored or approximated is listed in the import report.

use ecs_model::{BranchDef, ComponentDef, DefKind, Library, PinDef, PinKind, SubElement, Subckt};
use ecs_sim::Netlist;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// An import failure at a line of the input.
#[derive(Debug, Clone, PartialEq)]
pub struct SpiceError {
    pub file: String,
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for SpiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.file, self.line, self.message)
    }
}
impl std::error::Error for SpiceError {}

/// An analysis card found in the netlist.
#[derive(Debug, Clone, PartialEq)]
pub enum AnalysisCard {
    Op,
    Tran {
        step: f64,
        stop: f64,
    },
    Dc {
        source: String,
        start: f64,
        stop: f64,
        step: f64,
    },
    Ac {
        per_decade: usize,
        start: f64,
        stop: f64,
    },
}

/// Result of an import.
#[derive(Debug, Clone, Default)]
pub struct Import {
    /// Library components: one per `.model` and per `.subckt`.
    pub defs: Vec<Arc<ComponentDef>>,
    /// Top-level elements (when the text is a whole circuit).
    pub netlist: Netlist,
    pub analyses: Vec<AnalysisCard>,
    /// What was ignored or approximated, one line each.
    pub report: Vec<String>,
    pub title: String,
}

/// Parse a SPICE number: `1k`, `10u`, `2.2meg`, `1M` (milli), `3.3`, `1e-3`.
pub fn parse_value(text: &str) -> Option<f64> {
    let lower = text.trim().to_ascii_lowercase();
    let mut end = 0;
    let b = lower.as_bytes();
    while end < b.len() && (b[end].is_ascii_digit() || matches!(b[end], b'.' | b'-' | b'+')) {
        end += 1;
    }
    // Exponent only when followed by a digit or sign+digit ("1e-3", not "1meg").
    if end < b.len() && b[end] == b'e' {
        let mut k = end + 1;
        if k < b.len() && matches!(b[k], b'-' | b'+') {
            k += 1;
        }
        if k < b.len() && b[k].is_ascii_digit() {
            while k < b.len() && b[k].is_ascii_digit() {
                k += 1;
            }
            end = k;
        }
    }
    let (num, suffix) = lower.split_at(end);
    let base: f64 = num.parse().ok()?;
    let exponent = if suffix.starts_with("meg") {
        6
    } else if suffix.starts_with("mil") {
        return Some(base * 25.4e-6);
    } else {
        match suffix.chars().next() {
            None => 0,
            Some('t') => 12,
            Some('g') => 9,
            Some('k') => 3,
            Some('m') => -3,
            Some('u') | Some('µ') => -6,
            Some('n') => -9,
            Some('p') => -12,
            Some('f') => -15,
            Some(_) => 0,
        }
    };
    // Parse "<digits>e<exponent>" so the result is correctly rounded; a number
    // that already has an exponent ("1e-3") falls back to multiplication.
    format!("{num}e{exponent}")
        .parse()
        .ok()
        .or(Some(base * 10f64.powi(exponent)))
}

struct Line {
    file: String,
    number: usize,
    tokens: Vec<String>,
}

/// Join `+` continuations, drop comments, split into tokens. Parentheses,
/// commas and `=` separate tokens; `{...}` stays one token.
fn logical_lines(text: &str, file: &str) -> Vec<Line> {
    let mut out: Vec<Line> = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let mut l = raw.trim_end().to_string();
        for marker in [";", "$ "] {
            if let Some(p) = l.find(marker) {
                l.truncate(p);
            }
        }
        let t = l.trim();
        if t.is_empty() || t.starts_with('*') {
            continue;
        }
        if let Some(rest) = t.strip_prefix('+')
            && let Some(last) = out.last_mut()
        {
            last.tokens.extend(tokenize(rest));
            continue;
        }
        out.push(Line {
            file: file.into(),
            number: i + 1,
            tokens: tokenize(t),
        });
    }
    out
}

fn tokenize(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0;
    for ch in s.chars() {
        match ch {
            '{' => {
                depth += 1;
                cur.push(ch);
            }
            '}' => {
                depth -= 1;
                cur.push(ch);
            }
            c if depth > 0 => cur.push(c),
            ' ' | '\t' | ',' | '(' | ')' => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            '=' => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                out.push("=".into());
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// key=value pairs in a token list.
fn pairs(tokens: &[String]) -> Vec<(String, String)> {
    let mut v = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        if i + 2 < tokens.len() && tokens[i + 1] == "=" {
            v.push((tokens[i].to_ascii_lowercase(), tokens[i + 2].clone()));
            i += 3;
        } else {
            i += 1;
        }
    }
    v
}

struct Ctx<'a> {
    lib: &'a Library,
    params: HashMap<String, f64>,
    models: HashMap<String, Arc<ComponentDef>>,
    report: Vec<String>,
    loader: &'a dyn Fn(&Path) -> Option<String>,
    base: PathBuf,
}

impl Ctx<'_> {
    fn value(&self, tok: &str) -> Option<f64> {
        if let Some(inner) = tok.strip_prefix('{').and_then(|t| t.strip_suffix('}')) {
            let e = ecs_expr::parse(inner).ok()?;
            return ecs_expr::eval_with(&e, &|n| self.params.get(&n.to_ascii_lowercase()).copied())
                .ok();
        }
        if let Some(v) = self.params.get(&tok.to_ascii_lowercase()) {
            return Some(*v);
        }
        parse_value(tok)
    }
}

fn err(l: &Line, m: impl Into<String>) -> SpiceError {
    SpiceError {
        file: l.file.clone(),
        line: l.number,
        message: m.into(),
    }
}

/// Expand `.include` and `.lib <file> <section>` recursively.
fn expand(text: &str, file: &str, ctx: &Ctx<'_>, depth: usize) -> Result<Vec<Line>, SpiceError> {
    let mut out = Vec::new();
    let lines = logical_lines(text, file);
    let mut i = 0;
    while i < lines.len() {
        let l = &lines[i];
        let head = l.tokens[0].to_ascii_lowercase();
        if (head == ".include" || head == ".inc" || (head == ".lib" && l.tokens.len() >= 3))
            && depth < 16
        {
            let name = l
                .tokens
                .get(1)
                .map(|s| s.trim_matches('"').trim_matches('\''))
                .unwrap_or_default();
            let path = ctx.base.join(name);
            let body = (ctx.loader)(&path)
                .ok_or_else(|| err(l, format!("cannot read {}", path.display())))?;
            let mut sub = expand(&body, &path.display().to_string(), ctx, depth + 1)?;
            if head == ".lib" {
                // Keep only the named section: .lib <section> ... .endl
                let section = l.tokens[2].to_ascii_lowercase();
                let start = sub.iter().position(|x| {
                    x.tokens.len() == 2
                        && x.tokens[0].eq_ignore_ascii_case(".lib")
                        && x.tokens[1].eq_ignore_ascii_case(&section)
                });
                sub = match start {
                    Some(s) => sub
                        .into_iter()
                        .skip(s + 1)
                        .take_while(|x| !x.tokens[0].eq_ignore_ascii_case(".endl"))
                        .collect(),
                    None => return Err(err(l, format!("section {section} not found"))),
                };
            }
            out.extend(sub);
        } else {
            out.push(Line {
                file: l.file.clone(),
                number: l.number,
                tokens: l.tokens.clone(),
            });
        }
        i += 1;
    }
    Ok(out)
}

/// Built-in base and parameter map for each model type.
fn model_def(ctx: &mut Ctx<'_>, l: &Line) -> Result<Arc<ComponentDef>, SpiceError> {
    let name = l
        .tokens
        .get(1)
        .ok_or_else(|| err(l, ".model needs a name"))?
        .clone();
    let kind = l
        .tokens
        .get(2)
        .ok_or_else(|| err(l, ".model needs a type"))?
        .to_ascii_uppercase();
    let (base, map): (&str, &[(&str, &str)]) = match kind.as_str() {
        "D" => (
            "core.diode",
            &[
                ("is", "Is"),
                ("n", "N"),
                ("bv", "BV"),
                ("ibv", "IBV"),
                ("cjo", "Cj"),
                ("cj0", "Cj"),
            ],
        ),
        "NPN" => (
            "core.npn",
            &[
                ("is", "Is"),
                ("nf", "Nf"),
                ("bf", "Bf"),
                ("br", "Br"),
                ("vaf", "Vaf"),
                ("va", "Vaf"),
                ("cje", "Cje"),
                ("cjc", "Cjc"),
            ],
        ),
        "PNP" => (
            "core.pnp",
            &[
                ("is", "Is"),
                ("nf", "Nf"),
                ("bf", "Bf"),
                ("br", "Br"),
                ("vaf", "Vaf"),
                ("va", "Vaf"),
                ("cje", "Cje"),
                ("cjc", "Cjc"),
            ],
        ),
        "NMOS" => (
            "core.nmos",
            &[("kp", "Kp"), ("vto", "Vto"), ("lambda", "lambda")],
        ),
        "PMOS" => (
            "core.pmos",
            &[("kp", "Kp"), ("vto", "Vto"), ("lambda", "lambda")],
        ),
        "NJF" => (
            "core.njf",
            &[
                ("beta", "Beta"),
                ("vto", "Vto"),
                ("lambda", "lambda"),
                ("is", "Is"),
            ],
        ),
        "PJF" => (
            "core.pjf",
            &[
                ("beta", "Beta"),
                ("vto", "Vto"),
                ("lambda", "lambda"),
                ("is", "Is"),
            ],
        ),
        other => return Err(err(l, format!("model type {other} is not supported"))),
    };
    let mut def = (**ctx
        .lib
        .get(base)
        .ok_or_else(|| err(l, format!("missing built-in {base}")))?)
    .clone();
    def.id = format!("spice.{}", name.to_ascii_lowercase());
    def.meta.name = name.clone();
    def.meta.category = "Imported".into();
    def.meta.author = "SPICE import".into();
    def.meta.description = format!(
        "{kind} model {name} imported from SPICE ({}).",
        def.meta.description.trim_end_matches('.')
    );
    let mut ignored = Vec::new();
    for (k, v) in pairs(&l.tokens[3..]) {
        if k == "level" {
            let level = ctx.value(&v).unwrap_or(1.0);
            if level != 1.0 && (kind == "NMOS" || kind == "PMOS") {
                ctx.report
                    .push(format!("{name}: MOSFET level {level} mapped to level 1"));
            }
            continue;
        }
        let Some(&(_, ours)) = map.iter().find(|(s, _)| *s == k) else {
            if !matches!(
                k.as_str(),
                "mfg" | "type" | "vceo" | "icrating" | "vpk" | "ipk" | "iave" | "irms" | "diss"
            ) {
                ignored.push(k.to_ascii_uppercase());
            }
            continue;
        };
        let mut x = ctx
            .value(&v)
            .ok_or_else(|| err(l, format!("cannot read {k}={v}")))?;
        // Our MOSFET Vto is a magnitude; the P-MOSFET's SPICE VTO is negative.
        if ours == "Vto" && (kind == "PMOS" || kind == "NMOS") {
            x = x.abs();
        }
        if let Some(p) = def.params.iter_mut().find(|p| p.name == ours) {
            p.default = x;
        }
    }
    if !ignored.is_empty() {
        ctx.report
            .push(format!("{name}: ignored {}", ignored.join(", ")));
    }
    Ok(Arc::new(def))
}

fn pin(name: &str) -> PinDef {
    PinDef {
        name: name.into(),
        kind: PinKind::Analog,
    }
}

/// SPICE expression text to ours: `V(a,b)` stays, `**` and `^` agree.
fn spice_expr(s: &str) -> String {
    s.trim_matches(|c| c == '{' || c == '}').to_string()
}

/// One element card to (instance name, definition, nodes, parameters).
type Element = (
    String,
    Arc<ComponentDef>,
    Vec<String>,
    BTreeMap<String, f64>,
);

fn element(
    ctx: &mut Ctx<'_>,
    l: &Line,
    subckts: &HashMap<String, Arc<ComponentDef>>,
    raw_subckts: &HashMap<String, Vec<Line>>,
) -> Result<Option<Element>, SpiceError> {
    let t = &l.tokens;
    let name = t[0].clone();
    let lib = ctx.lib;
    let get = |id: &str| {
        lib.get(id)
            .cloned()
            .ok_or_else(|| err(l, format!("missing built-in {id}")))
    };
    let need = |n: usize| {
        if t.len() < n {
            Err(err(l, format!("{name}: expected at least {n} fields")))
        } else {
            Ok(())
        }
    };
    let val = |ctx: &Ctx<'_>, k: usize| -> Result<f64, SpiceError> {
        let tok = t
            .get(k)
            .ok_or_else(|| err(l, format!("{name}: missing value")))?;
        ctx.value(tok)
            .ok_or_else(|| err(l, format!("{name}: cannot read value {tok}")))
    };
    let nodes = |a: usize, b: usize| t[a..b].to_vec();
    let one = |k: &str, v: f64| BTreeMap::from([(k.to_string(), v)]);
    let first = name.chars().next().unwrap().to_ascii_uppercase();
    let el: Element = match first {
        'R' => {
            need(4)?;
            (
                name.clone(),
                get("core.resistor")?,
                nodes(1, 3),
                one("R", val(ctx, 3)?),
            )
        }
        'C' => {
            need(4)?;
            (
                name.clone(),
                get("core.capacitor")?,
                nodes(1, 3),
                one("C", val(ctx, 3)?),
            )
        }
        'L' => {
            need(4)?;
            (
                name.clone(),
                get("core.inductor")?,
                nodes(1, 3),
                one("L", val(ctx, 3)?),
            )
        }
        'V' | 'I' => {
            need(3)?;
            let current = first == 'I';
            let mut params = BTreeMap::new();
            let mut def = get(if current { "core.idc" } else { "core.vdc" })?;
            let key = if current { "I" } else { "V" };
            let mut k = 3;
            while k < t.len() {
                let w = t[k].to_ascii_lowercase();
                match w.as_str() {
                    "dc" => {
                        params.insert(key.to_string(), val(ctx, k + 1)?);
                        k += 2;
                    }
                    "ac" => {
                        params.insert("AC".into(), val(ctx, k + 1)?);
                        k += 2;
                        if t.get(k).and_then(|x| ctx.value(x)).is_some() {
                            k += 1; // phase, ignored
                        }
                    }
                    "sin" if !current => {
                        def = get("core.vsine")?;
                        params.insert("Voff".into(), val(ctx, k + 1)?);
                        params.insert("Vamp".into(), val(ctx, k + 2)?);
                        params.insert("freq".into(), val(ctx, k + 3)?);
                        k += 4;
                        while t.get(k).and_then(|x| ctx.value(x)).is_some() {
                            k += 1;
                        }
                    }
                    "pulse" if !current => {
                        // PULSE(V1 V2 TD TR TF PW PER): a square wave between V1 and V2.
                        def = get("core.funcgen")?;
                        let v1 = val(ctx, k + 1)?;
                        let v2 = val(ctx, k + 2)?;
                        let pw = val(ctx, k + 6).unwrap_or(0.5e-3);
                        let per = val(ctx, k + 7).unwrap_or(1e-3);
                        params.insert("wave".into(), 1.0);
                        params.insert("Vlow".into(), v1);
                        params.insert("Vhigh".into(), v2);
                        params.insert("freq".into(), 1.0 / per);
                        params.insert("duty".into(), (pw / per).clamp(0.01, 0.99));
                        ctx.report.push(format!(
                            "{name}: PULSE delay and edge times ignored (ideal square wave)"
                        ));
                        k += 8;
                    }
                    _ => {
                        if let Some(v) = ctx.value(&t[k]) {
                            params.insert(key.to_string(), v);
                        } else {
                            ctx.report.push(format!("{name}: ignored {}", t[k]));
                        }
                        k += 1;
                    }
                }
            }
            (name.clone(), def, nodes(1, 3), params)
        }
        'E' | 'G' => {
            need(6)?;
            let (id, n) = if first == 'E' {
                ("core.vcvs", vec![t[1].clone(), t[2].clone()])
            } else {
                ("core.vccs", vec![t[2].clone(), t[1].clone()])
            };
            let mut ns = n;
            ns.push(t[3].clone());
            ns.push(t[4].clone());
            (name.clone(), get(id)?, ns, one("gain", val(ctx, 5)?))
        }
        'F' | 'H' => {
            need(5)?;
            let gain = val(ctx, 4)?;
            let ctrl = &t[3];
            let mut d = ComponentDef::new(
                &format!("spice.{}.{name}", first.to_ascii_lowercase()),
                &name,
                "Controlled sources",
                &first.to_string(),
                "box",
            );
            d.pins = vec![pin("P"), pin("N")];
            let e = ecs_expr::parse(&format!("{gain:e} * I({ctrl})"))
                .map_err(|e| err(l, e.to_string()))?;
            if first == 'F' {
                d.branches.push(BranchDef::Current {
                    name: None,
                    a: "P".into(),
                    b: "N".into(),
                    i: e,
                    q: None,
                });
            } else {
                d.branches.push(BranchDef::Voltage {
                    name: "H".into(),
                    a: "P".into(),
                    b: "N".into(),
                    v: e,
                    flux: None,
                });
            }
            (name.clone(), Arc::new(d), nodes(1, 3), BTreeMap::new())
        }
        'B' => {
            need(4)?;
            let rest: Vec<&String> = t[3..].iter().collect();
            let (kind, body) = match rest.as_slice() {
                [k, eq, b, ..] if *eq == "=" => (k.to_ascii_uppercase(), spice_expr(b)),
                _ => return Err(err(l, format!("{name}: expected V={{...}} or I={{...}}"))),
            };
            let mut expr = ecs_expr::parse(&body).map_err(|e| err(l, format!("{name}: {e}")))?;
            // Node voltages in the expression become extra pins of the source.
            let mut refs: Vec<String> = Vec::new();
            expr.visit(&mut |leaf| {
                if let ecs_expr::Expr::V(a, b) = leaf {
                    for n in std::iter::once(a).chain(b.iter()) {
                        if n != "0" && !refs.contains(n) {
                            refs.push(n.clone());
                        }
                    }
                }
            });
            let pin_of = |n: &str| format!("n_{}", n.replace(['.', '-'], "_"));
            let mut text = body.clone();
            for r in &refs {
                text = text
                    .replace(&format!("V({r}"), &format!("V({}", pin_of(r)))
                    .replace(&format!(", {r})"), &format!(", {})", pin_of(r)))
                    .replace(&format!(",{r})"), &format!(",{})", pin_of(r)));
            }
            expr = ecs_expr::parse(&text).map_err(|e| err(l, format!("{name}: {e}")))?;
            let mut d =
                ComponentDef::new(&format!("spice.b.{name}"), &name, "Behavioural", "B", "box");
            d.pins = vec![pin("P"), pin("N")];
            d.pins.extend(refs.iter().map(|r| pin(&pin_of(r))));
            if kind == "V" {
                d.branches.push(BranchDef::Voltage {
                    name: "B".into(),
                    a: "P".into(),
                    b: "N".into(),
                    v: expr,
                    flux: None,
                });
            } else {
                d.branches.push(BranchDef::Current {
                    name: None,
                    a: "P".into(),
                    b: "N".into(),
                    i: expr,
                    q: None,
                });
            }
            let mut ns = nodes(1, 3);
            ns.extend(refs);
            (name.clone(), Arc::new(d), ns, BTreeMap::new())
        }
        'D' => {
            need(4)?;
            let m = model(ctx, l, &t[3])?;
            (name.clone(), m, nodes(1, 3), BTreeMap::new())
        }
        'Q' => {
            need(5)?;
            // Q c b e [substrate] model
            let model_tok = if t.len() >= 6 && ctx.models.contains_key(&t[5].to_ascii_lowercase()) {
                5
            } else {
                4
            };
            if model_tok == 5 {
                ctx.report.push(format!("{name}: substrate node ignored"));
            }
            let m = model(ctx, l, &t[model_tok])?;
            (name.clone(), m, nodes(1, 4), BTreeMap::new())
        }
        'M' => {
            need(6)?;
            // M d g s b model [W= L=]
            let m = model(ctx, l, &t[5])?;
            let kv = pairs(&t[6..]);
            let w = kv
                .iter()
                .find(|(k, _)| k == "w")
                .and_then(|(_, v)| ctx.value(v));
            let len = kv
                .iter()
                .find(|(k, _)| k == "l")
                .and_then(|(_, v)| ctx.value(v));
            let mut params = BTreeMap::new();
            if let (Some(w), Some(len)) = (w, len) {
                let kp = m
                    .params
                    .iter()
                    .find(|p| p.name == "Kp")
                    .map_or(2e-5, |p| p.default);
                params.insert("Kp".to_string(), kp * w / len);
            }
            if t[4] != t[3] {
                ctx.report.push(format!(
                    "{name}: bulk node {} treated as tied to the source",
                    t[4]
                ));
            }
            (name.clone(), m, nodes(1, 4), params)
        }
        'J' => {
            need(5)?;
            let m = model(ctx, l, &t[4])?;
            (name.clone(), m, nodes(1, 4), BTreeMap::new())
        }
        'X' => {
            need(2)?;
            // X n1 n2 ... subname [PARAMS: ...]
            let stop = t
                .iter()
                .position(|x| x.eq_ignore_ascii_case("params:"))
                .unwrap_or(t.len());
            let sub_name = t[stop - 1].to_ascii_lowercase();
            let def = match subckts.get(&sub_name) {
                Some(d) => d.clone(),
                None => match raw_subckts.get(&sub_name) {
                    Some(body) => subckt(ctx, &sub_name, body, subckts, raw_subckts)?,
                    None => return Err(err(l, format!("{name}: unknown subcircuit {sub_name}"))),
                },
            };
            if stop < t.len() {
                ctx.report.push(format!(
                    "{name}: instance PARAMS: ignored; the subcircuit's defaults are used"
                ));
            }
            (name.clone(), def, nodes(1, stop - 1), BTreeMap::new())
        }
        'K' => {
            ctx.report.push(format!(
                "{name}: coupled inductors are not supported yet; ignored"
            ));
            return Ok(None);
        }
        _ => {
            ctx.report.push(format!(
                "{name}: element type {first} not supported; ignored"
            ));
            return Ok(None);
        }
    };
    Ok(Some(el))
}

fn model(ctx: &Ctx<'_>, l: &Line, name: &str) -> Result<Arc<ComponentDef>, SpiceError> {
    ctx.models
        .get(&name.to_ascii_lowercase())
        .cloned()
        .ok_or_else(|| err(l, format!("unknown model {name}")))
}

fn subckt(
    ctx: &mut Ctx<'_>,
    name: &str,
    body: &[Line],
    done: &HashMap<String, Arc<ComponentDef>>,
    raw: &HashMap<String, Vec<Line>>,
) -> Result<Arc<ComponentDef>, SpiceError> {
    let head = &body[0];
    let stop = head
        .tokens
        .iter()
        .position(|x| x.eq_ignore_ascii_case("params:"))
        .unwrap_or(head.tokens.len());
    let ports: Vec<String> = head.tokens[2..stop].to_vec();
    let saved = ctx.params.clone();
    for (k, v) in pairs(&head.tokens[stop.min(head.tokens.len())..]) {
        if let Some(x) = ctx.value(&v) {
            ctx.params.insert(k, x);
        }
    }
    let mut elements = Vec::new();
    for l in &body[1..] {
        let h = l.tokens[0].to_ascii_lowercase();
        if h == ".param" {
            for (k, v) in pairs(&l.tokens[1..]) {
                if let Some(x) = ctx.value(&v) {
                    ctx.params.insert(k, x);
                }
            }
            continue;
        }
        if h.starts_with('.') {
            continue;
        }
        if let Some((n, def, nodes, params)) = element(ctx, l, done, raw)? {
            if nodes.len() != def.pins.len() {
                return Err(err(
                    l,
                    format!("{n}: {} nodes for {} pins", nodes.len(), def.pins.len()),
                ));
            }
            elements.push(SubElement {
                name: n,
                def,
                nodes,
                params,
                exprs: Default::default(),
            });
        }
    }
    ctx.params = saved;
    let mut def = ComponentDef::new(
        &format!("spice.{name}"),
        &head.tokens[1],
        "Imported",
        "X",
        "",
    );
    def.pins = ports.iter().map(|p| pin(p)).collect();
    def.kind = DefKind::Subcircuit(Arc::new(Subckt { elements }));
    def.meta.author = "SPICE import".into();
    def.meta.description = format!("Subcircuit {} imported from SPICE.", head.tokens[1]);
    Ok(Arc::new(def))
}

/// Import SPICE text. `base` resolves `.include`; `loader` reads files.
pub fn import(
    text: &str,
    file: &str,
    base: &Path,
    loader: &dyn Fn(&Path) -> Option<String>,
) -> Result<Import, SpiceError> {
    let lib = ecs_lib::library();
    let mut ctx = Ctx {
        lib: &lib,
        params: HashMap::new(),
        models: HashMap::new(),
        report: Vec::new(),
        loader,
        base: base.to_path_buf(),
    };
    let mut out = Import::default();
    // The first line of a SPICE netlist is its title unless it is a comment,
    // a dot card, or reads like an element card.
    let first = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or_default()
        .trim()
        .to_string();
    let first_tokens = tokenize(&first);
    let looks_like_card = first_tokens.len() >= 3
        && first
            .chars()
            .next()
            .is_some_and(|c| "RCLVIEFGHBDQMJXKrclviefghbdqmjxk".contains(c));
    let title_line = !first.starts_with('*') && !first.starts_with('.') && !looks_like_card;
    out.title = first.trim_start_matches('*').trim().to_string();
    let mut lines = expand(text, file, &ctx, 0)?;
    if title_line && !lines.is_empty() && lines[0].file == file {
        lines.remove(0);
    }
    // Pass 1: params, models and raw subcircuit bodies.
    let mut raw: HashMap<String, Vec<Line>> = HashMap::new();
    let mut top: Vec<Line> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let l = &lines[i];
        let h = l.tokens[0].to_ascii_lowercase();
        match h.as_str() {
            ".subckt" => {
                let name = l
                    .tokens
                    .get(1)
                    .ok_or_else(|| err(l, ".subckt needs a name"))?
                    .to_ascii_lowercase();
                let mut body = vec![Line {
                    file: l.file.clone(),
                    number: l.number,
                    tokens: l.tokens.clone(),
                }];
                i += 1;
                let mut depth = 0;
                while i < lines.len() {
                    let h2 = lines[i].tokens[0].to_ascii_lowercase();
                    if h2 == ".subckt" {
                        depth += 1;
                    }
                    if h2 == ".ends" {
                        if depth == 0 {
                            break;
                        }
                        depth -= 1;
                    }
                    body.push(Line {
                        file: lines[i].file.clone(),
                        number: lines[i].number,
                        tokens: lines[i].tokens.clone(),
                    });
                    i += 1;
                }
                raw.insert(name, body);
            }
            ".param" => {
                for (k, v) in pairs(&l.tokens[1..]) {
                    let x = ctx
                        .value(&v)
                        .ok_or_else(|| err(l, format!("cannot evaluate {k}={v}")))?;
                    ctx.params.insert(k, x);
                }
            }
            ".model" => {
                let def = model_def(&mut ctx, l)?;
                ctx.models
                    .insert(l.tokens[1].to_ascii_lowercase(), def.clone());
                out.defs.push(def);
            }
            ".end" => break,
            _ => top.push(Line {
                file: l.file.clone(),
                number: l.number,
                tokens: l.tokens.clone(),
            }),
        }
        i += 1;
    }
    // Pass 2: subcircuits (recursively on demand), then top-level cards.
    let mut done: HashMap<String, Arc<ComponentDef>> = HashMap::new();
    let mut names: Vec<String> = raw.keys().cloned().collect();
    names.sort();
    for n in names {
        let def = subckt(&mut ctx, &n, &raw[&n], &done, &raw)?;
        done.insert(n, def.clone());
        out.defs.push(def);
    }
    let mut netlist = Netlist::new();
    for l in &top {
        let h = l.tokens[0].to_ascii_lowercase();
        match h.as_str() {
            ".op" => out.analyses.push(AnalysisCard::Op),
            ".tran" => {
                let v = |k: usize| l.tokens.get(k).and_then(|x| ctx.value(x));
                out.analyses.push(AnalysisCard::Tran {
                    step: v(1).ok_or_else(|| err(l, ".tran step"))?,
                    stop: v(2).ok_or_else(|| err(l, ".tran stop"))?,
                });
            }
            ".dc" => {
                let v = |k: usize| {
                    l.tokens
                        .get(k)
                        .and_then(|x| ctx.value(x))
                        .ok_or_else(|| err(l, ".dc needs source start stop step"))
                };
                out.analyses.push(AnalysisCard::Dc {
                    source: l.tokens.get(1).cloned().unwrap_or_default(),
                    start: v(2)?,
                    stop: v(3)?,
                    step: v(4)?,
                });
            }
            ".ac" => {
                let v = |k: usize| {
                    l.tokens
                        .get(k)
                        .and_then(|x| ctx.value(x))
                        .ok_or_else(|| err(l, ".ac needs dec n fstart fstop"))
                };
                if !l
                    .tokens
                    .get(1)
                    .is_some_and(|t| t.eq_ignore_ascii_case("dec"))
                {
                    out.report
                        .push("only .ac dec sweeps are supported; treated as dec".into());
                }
                out.analyses.push(AnalysisCard::Ac {
                    per_decade: v(2)? as usize,
                    start: v(3)?,
                    stop: v(4)?,
                });
            }
            h if h.starts_with('.') => out
                .report
                .push(format!("line {}: {} ignored", l.number, l.tokens[0])),
            _ => {
                if let Some((n, def, nodes, params)) = element(&mut ctx, l, &done, &raw)? {
                    if nodes.len() != def.pins.len() {
                        return Err(err(
                            l,
                            format!("{n}: {} nodes for {} pins", nodes.len(), def.pins.len()),
                        ));
                    }
                    let refs: Vec<&str> = nodes.iter().map(String::as_str).collect();
                    let pv: Vec<(&str, f64)> =
                        params.iter().map(|(k, v)| (k.as_str(), *v)).collect();
                    netlist.add(&n, def, &refs, &pv);
                }
            }
        }
    }
    out.netlist = netlist;
    out.report.extend(ctx.report);
    Ok(out)
}

/// Import from a file on disk (resolving includes beside it).
pub fn import_file(path: &Path) -> Result<Import, SpiceError> {
    let text = std::fs::read_to_string(path).map_err(|e| SpiceError {
        file: path.display().to_string(),
        line: 0,
        message: e.to_string(),
    })?;
    let base = path.parent().map(Path::to_path_buf).unwrap_or_default();
    import(&text, &path.display().to_string(), &base, &|p| {
        std::fs::read_to_string(p).ok()
    })
}

fn num(v: f64) -> String {
    format!("{v:e}")
}

/// Export a netlist as SPICE (for cross-checking in ngspice).
pub fn export(netlist: &Netlist, title: &str) -> String {
    let mut out = format!(
        "* {title}\n* exported by Electrocraft {}\n",
        env!("CARGO_PKG_VERSION")
    );
    let flat = ecs_sim::flatten(netlist).unwrap_or_else(|_| netlist.clone());
    let node = |i: usize| -> String {
        let n = &flat.nodes[i];
        if i == 0 {
            "0".into()
        } else {
            n.replace(' ', "_")
        }
    };
    let mut models = Vec::new();
    for inst in &flat.instances {
        let d = &inst.def;
        let p = |k: &str| {
            inst.params.get(k).copied().unwrap_or_else(|| {
                d.params
                    .iter()
                    .find(|x| x.name == k)
                    .map_or(0.0, |x| x.default)
            })
        };
        let nodes: Vec<String> = inst.pins.iter().map(|&i| node(i)).collect();
        let name = inst.name.replace('.', "_");
        let base = d.id.trim_start_matches("spice.");
        let card = match d.id.as_str() {
            "core.resistor" => format!("R{name} {} {} {}", nodes[0], nodes[1], num(p("R"))),
            "core.capacitor" => format!("C{name} {} {} {}", nodes[0], nodes[1], num(p("C"))),
            "core.inductor" => format!("L{name} {} {} {}", nodes[0], nodes[1], num(p("L"))),
            "core.vdc" => format!(
                "V{name} {} {} DC {} AC {}",
                nodes[0],
                nodes[1],
                num(p("V")),
                num(p("AC"))
            ),
            "core.battery" => format!("V{name} {} {} DC {}", nodes[0], nodes[1], num(p("V"))),
            "core.idc" => format!("I{name} {} {} DC {}", nodes[1], nodes[0], num(p("I"))),
            "core.vsine" => format!(
                "V{name} {} {} SIN({} {} {}) AC {}",
                nodes[0],
                nodes[1],
                num(p("Voff")),
                num(p("Vamp")),
                num(p("freq")),
                num(p("AC"))
            ),
            "core.funcgen" => {
                let per = 1.0 / p("freq");
                format!(
                    "V{name} {} {} PULSE({} {} 0 1n 1n {} {})",
                    nodes[0],
                    nodes[1],
                    num(p("Vlow")),
                    num(p("Vhigh")),
                    num(per * p("duty")),
                    num(per)
                )
            }
            "core.vcvs" => format!(
                "E{name} {} {} {} {} {}",
                nodes[0],
                nodes[1],
                nodes[2],
                nodes[3],
                num(p("gain"))
            ),
            "core.vccs" => format!(
                "G{name} {} {} {} {} {}",
                nodes[1],
                nodes[0],
                nodes[2],
                nodes[3],
                num(p("gain"))
            ),
            "core.switch" | "core.pushbutton" => {
                format!("R{name} {} {} {}", nodes[0], nodes[1], num(p("Roff")))
            }
            "core.voltmeter" | "core.scope" | "core.ground" => continue,
            "core.ammeter" => format!("V{name} {} {} DC 0", nodes[0], nodes[1]),
            _ if d.limits.iter().any(|_| true) && d.pins.len() == 2 => {
                models.push(format!(
                    ".model D_{name} D(IS={} N={} BV={} IBV={} CJO={})",
                    num(p("Is")),
                    num(p("N")),
                    num(p("BV")),
                    num(p("IBV")),
                    num(p("Cj"))
                ));
                format!("D{name} {} {} D_{name}", nodes[0], nodes[1])
            }
            "core.npn" | "core.pnp" => {
                let t = if d.id == "core.npn" { "NPN" } else { "PNP" };
                models.push(format!(
                    ".model Q_{name} {t}(IS={} NF={} BF={} BR={} VAF={} CJE={} CJC={})",
                    num(p("Is")),
                    num(p("Nf")),
                    num(p("Bf")),
                    num(p("Br")),
                    num(p("Vaf")),
                    num(p("Cje")),
                    num(p("Cjc"))
                ));
                format!("Q{name} {} {} {} Q_{name}", nodes[0], nodes[1], nodes[2])
            }
            "core.nmos" | "core.pmos" => {
                let pm = d.id == "core.pmos";
                models.push(format!(
                    ".model M_{name} {}(LEVEL=1 KP={} VTO={} LAMBDA={})",
                    if pm { "PMOS" } else { "NMOS" },
                    num(p("Kp")),
                    num(if pm { -p("Vto") } else { p("Vto") }),
                    num(p("lambda"))
                ));
                format!(
                    "M{name} {} {} {} {} M_{name}",
                    nodes[0], nodes[1], nodes[2], nodes[2]
                )
            }
            "core.njf" | "core.pjf" => {
                models.push(format!(
                    ".model J_{name} {}(BETA={} VTO={} LAMBDA={} IS={})",
                    if d.id == "core.njf" { "NJF" } else { "PJF" },
                    num(p("Beta")),
                    num(p("Vto")),
                    num(p("lambda")),
                    num(p("Is"))
                ));
                format!("J{name} {} {} {} J_{name}", nodes[0], nodes[1], nodes[2])
            }
            _ => format!("* {name}: {} ({base}) has no SPICE equivalent", d.meta.name),
        };
        out.push_str(&card);
        out.push('\n');
    }
    for m in models {
        out.push_str(&m);
        out.push('\n');
    }
    out.push_str(".op\n.end\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spice_suffixes() {
        assert_eq!(parse_value("1k"), Some(1e3));
        assert_eq!(parse_value("1M"), Some(1e-3));
        assert_eq!(parse_value("2meg"), Some(2e6));
        assert_eq!(parse_value("10uF"), Some(10e-6));
        assert_eq!(parse_value("1e-3"), Some(1e-3));
        assert_eq!(parse_value("4.7K"), Some(4.7e3));
    }

    #[test]
    fn tokenizer_handles_parens_equals_braces() {
        assert_eq!(
            tokenize(".model D1N4148 D(IS=2.52n RS=.568)"),
            vec![
                ".model", "D1N4148", "D", "IS", "=", "2.52n", "RS", "=", ".568"
            ]
        );
        assert_eq!(
            tokenize("B1 a 0 V={V(x)*2}"),
            vec!["B1", "a", "0", "V", "=", "{V(x)*2}"]
        );
    }
}
