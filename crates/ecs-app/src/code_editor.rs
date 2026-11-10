//! Code editor widget: syntax highlighting and code intelligence for the
//! component JavaScript, expression formulas (scope channels) and SPICE text.
//! Suggestions come from the language's keywords and built-ins, from what the
//! text itself declares (functions, variables, parameters, pins, states) and
//! from the caller (scope codes, symbol pins). Up/Down pick, Tab or Enter
//! accept, Escape closes; a line under the editor shows the signature of the
//! call around the cursor.

use crate::theme::{self, c};
use egui::text::{CCursor, CCursorRange, LayoutJob, TextFormat};
use egui::{Color32, Key, Modifiers, RichText, Sense, Ui, vec2};

/// Which language the text is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    /// A component file (spec 6).
    Js,
    /// The expression language (scope formulas, `e`-templates).
    Expr,
    /// SPICE models and subcircuits.
    Spice,
}

/// What a suggestion is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Keyword,
    Api,
    Method,
    Builtin,
    Function,
    Variable,
    Param,
    Pin,
    Code,
    Card,
}

/// A suggestion.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub label: String,
    pub kind: Kind,
    /// Signature or description.
    pub detail: String,
    /// Text inserted on accept (defaults to the label).
    pub insert: String,
}

impl Item {
    pub fn new(label: &str, kind: Kind, detail: &str) -> Self {
        Self {
            label: label.into(),
            kind,
            detail: detail.into(),
            insert: label.into(),
        }
    }
}

mod palette {
    use egui::Color32;
    pub const KEYWORD: Color32 = Color32::from_rgb(0xC5, 0x86, 0xC0);
    pub const API: Color32 = Color32::from_rgb(0x4E, 0xA1, 0xDC);
    pub const BUILTIN: Color32 = Color32::from_rgb(0x4E, 0xC9, 0xB0);
    pub const FUNCTION: Color32 = Color32::from_rgb(0xDC, 0xDC, 0xAA);
    pub const PARAM: Color32 = Color32::from_rgb(0x9C, 0xDC, 0xFE);
    pub const NUMBER: Color32 = Color32::from_rgb(0xB5, 0xCE, 0xA8);
    pub const STRING: Color32 = Color32::from_rgb(0xCE, 0x91, 0x78);
    pub const PIN: Color32 = Color32::from_rgb(0xF0, 0x8A, 0x4B);
    pub const CODE: Color32 = Color32::from_rgb(0x9B, 0xC5, 0xF3);
}

fn kind_color(k: Kind) -> Color32 {
    match k {
        Kind::Keyword => palette::KEYWORD,
        Kind::Api | Kind::Method => palette::API,
        Kind::Builtin => palette::BUILTIN,
        Kind::Function => palette::FUNCTION,
        Kind::Variable | Kind::Param => palette::PARAM,
        Kind::Pin => palette::PIN,
        Kind::Code => palette::CODE,
        Kind::Card => palette::KEYWORD,
    }
}

const JS_KEYWORDS: [&str; 24] = [
    "const",
    "let",
    "var",
    "function",
    "return",
    "if",
    "else",
    "true",
    "false",
    "export",
    "default",
    "new",
    "this",
    "typeof",
    "for",
    "while",
    "do",
    "switch",
    "case",
    "break",
    "continue",
    "null",
    "undefined",
    "of",
];

const JS_API: [(&str, &str); 10] = [
    (
        "component",
        "component({ id, name, pins, params, equations(c) {...} }) — describe the component",
    ),
    (
        "param",
        "param(default, unit?, description?, { live, min, max, values }?)",
    ),
    ("digital", "digital(family?) — a digital input pin"),
    ("digitalOut", "digitalOut(family?) — a digital output pin"),
    ("e", "e`...` — an equation in the expression language"),
    (
        "V",
        "V(\"A\") or V(\"A\", \"K\") — voltage of a pin or between two",
    ),
    ("I", "I(\"name\") — current of a named voltage branch"),
    ("vt", "thermal voltage kT/q (25.85 mV at 27 °C)"),
    ("Math", "Math.exp, Math.log, Math.tanh, Math.min, Math.PI…"),
    (
        "ctx",
        "hook context: ctx.level(pin), ctx.drive(pin, level, delay), ctx.state…",
    ),
];

const SPEC_KEYS: [(&str, &str); 15] = [
    ("id", "id: \"user.name\" — stable component id"),
    ("name", "name: display name"),
    ("category", "category: palette group"),
    (
        "symbol",
        "symbol: symbol id (ignored for components in a schematic)",
    ),
    ("prefix", "prefix: reference prefix, e.g. \"U\""),
    ("description", "description: shown in the inspector"),
    (
        "pins",
        "pins: [\"A\", \"B\"] or { D: digital(), Q: digitalOut() }",
    ),
    ("params", "params: { R: param(1e3, \"Ω\", \"Resistance\") }"),
    ("states", "states: { closed: 0 } — values changed by events"),
    ("controls", "controls: [{ kind: \"toggle\", state, label }]"),
    (
        "equations",
        "equations(c) { c.current(…); c.voltage(…); c.let(…); c.probe(…) }",
    ),
    ("onStart", "onStart(ctx) — once at reset"),
    ("onInputs", "onInputs(ctx) — when a digital input changes"),
    ("onTimer", "onTimer(ctx, tag) — when a timer fires"),
    (
        "onControl",
        "onControl(ctx, event) — a click or slider on the part",
    ),
];

const EQ_METHODS: [(&str, &str); 6] = [
    (
        "current",
        "c.current(a, b, i, q?) — current i (+ dq/dt) from a to b",
    ),
    (
        "voltage",
        "c.voltage(a, b, v, flux?, name?) — V(a) − V(b) = v (+ dflux/dt); adds I(name)",
    ),
    (
        "let",
        "c.let(name, equation) — name a value; later functions take it as a parameter",
    ),
    ("node", "c.node(name) — an internal node"),
    (
        "limit",
        "c.limit(\"pn\", () => V(\"A\", \"K\"), { vte, is }?) — junction limiting",
    ),
    (
        "probe",
        "c.probe(name, equation, unit?) — shown in the inspector and on the scope",
    ),
];

const CTX_MEMBERS: [(&str, &str); 9] = [
    ("time", "ctx.time — simulated time (s)"),
    ("state", "ctx.state.name — read or set a state"),
    ("param", "ctx.param.name — a parameter value"),
    ("level", "ctx.level(pin) — 0, 1 or \"X\""),
    ("rising", "ctx.rising(pin) — true on a rising edge"),
    ("falling", "ctx.falling(pin) — true on a falling edge"),
    ("voltage", "ctx.voltage(pin) — pin voltage"),
    (
        "drive",
        "ctx.drive(pin, level, delay?) — schedule an output level",
    ),
    ("timer", "ctx.timer(delay, tag?) — call onTimer later"),
];

const MATH: [(&str, &str); 18] = [
    ("exp", "Math.exp(x)"),
    ("log", "Math.log(x) — natural logarithm"),
    ("log10", "Math.log10(x)"),
    ("sqrt", "Math.sqrt(x)"),
    ("abs", "Math.abs(x)"),
    ("sin", "Math.sin(x)"),
    ("cos", "Math.cos(x)"),
    ("tan", "Math.tan(x)"),
    ("atan", "Math.atan(x)"),
    ("tanh", "Math.tanh(x)"),
    ("sinh", "Math.sinh(x)"),
    ("cosh", "Math.cosh(x)"),
    ("floor", "Math.floor(x)"),
    ("pow", "Math.pow(x, y) — same as x ** y"),
    ("min", "Math.min(a, b, …)"),
    ("max", "Math.max(a, b, …)"),
    ("PI", "Math.PI"),
    ("E", "Math.E"),
];

/// Functions and names of the expression language.
pub const EXPR_FUNCS: [(&str, &str); 26] = [
    ("exp", "exp(x)"),
    ("limexp", "limexp(x) — exp that continues linearly above 80"),
    ("ln", "ln(x) — natural logarithm"),
    ("log10", "log10(x)"),
    ("sqrt", "sqrt(x)"),
    ("safe_sqrt", "safe_sqrt(x) — sqrt(max(x, 0))"),
    ("sin", "sin(x)"),
    ("cos", "cos(x)"),
    ("tan", "tan(x)"),
    ("atan", "atan(x)"),
    ("tanh", "tanh(x)"),
    ("sinh", "sinh(x)"),
    ("cosh", "cosh(x)"),
    ("abs", "abs(x)"),
    ("floor", "floor(x)"),
    ("pow", "pow(x, y) — x ^ y"),
    ("min", "min(a, b)"),
    ("max", "max(a, b)"),
    ("safe_div", "safe_div(a, b) — a / b with |b| ≥ 1e-30"),
    ("select", "select(cond, a, b) — a when cond ≠ 0, else b"),
    ("smoothstep", "smoothstep(e0, e1, x) — smooth 0…1 step"),
    ("smooth_abs", "smooth_abs(x, eps?) — differentiable |x|"),
    ("t", "t — time (s)"),
    ("vt", "vt — thermal voltage"),
    ("temp", "temp — temperature (K)"),
    ("pi", "pi"),
];

const SPICE_CARDS: [(&str, &str); 14] = [
    (
        ".model",
        ".model NAME TYPE(PARAM=VALUE …) — D, NPN, PNP, NMOS, PMOS, NJF, PJF",
    ),
    (".subckt", ".subckt NAME port1 port2 … — start a subcircuit"),
    (".ends", ".ends — end the subcircuit"),
    (".param", ".param name=value"),
    (".include", ".include file"),
    (".lib", ".lib file section"),
    (".op", ".op — operating point"),
    (".dc", ".dc SOURCE start stop step"),
    (".tran", ".tran step stop"),
    (".ac", ".ac dec points fstart fstop"),
    (".step", ".step param NAME start stop step"),
    (".end", ".end"),
    (".options", ".options name=value"),
    (".print", ".print tran V(node)"),
];

const SPICE_TYPES: [&str; 9] = ["D", "NPN", "PNP", "NMOS", "PMOS", "NJF", "PJF", "SW", "R"];

/// Everything the text declares.
pub fn declared(lang: Lang, text: &str) -> Vec<Item> {
    let mut out: Vec<Item> = Vec::new();
    let mut push = |item: Item| {
        if !out.iter().any(|o| o.label == item.label) {
            out.push(item);
        }
    };
    match lang {
        Lang::Js => {
            let toks = lex_js(text);
            let word = |k: usize| -> Option<&str> {
                toks.get(k)
                    .and_then(|t| (t.kind == Tk::Ident).then(|| &text[t.range.clone()]))
            };
            let punct = |k: usize, p: &str| {
                toks.get(k)
                    .is_some_and(|t| t.kind == Tk::Punct && &text[t.range.clone()] == p)
            };
            for k in 0..toks.len() {
                match word(k) {
                    Some("function") => {
                        if let Some(name) = word(k + 1) {
                            let sig = signature(text, &toks, k + 2);
                            let doc = doc_before(text, toks[k].range.start);
                            push(Item::new(
                                name,
                                Kind::Function,
                                &format!("{name}({sig}){doc}"),
                            ));
                        }
                    }
                    Some("let") if k >= 2 && punct(k - 1, ".") && punct(k + 1, "(") => {
                        if let Some(t) = toks.get(k + 2)
                            && t.kind == Tk::Str
                        {
                            let name =
                                text[t.range.clone()].trim_matches(|c| c == '"' || c == '\'');
                            push(Item::new(name, Kind::Variable, "c.let value"));
                        }
                    }
                    Some("node") if k >= 2 && punct(k - 1, ".") && punct(k + 1, "(") => {
                        if let Some(t) = toks.get(k + 2)
                            && t.kind == Tk::Str
                        {
                            let name =
                                text[t.range.clone()].trim_matches(|c| c == '"' || c == '\'');
                            push(Item::new(name, Kind::Pin, "internal node"));
                        }
                    }
                    Some("const" | "let" | "var") => {
                        if let Some(name) = word(k + 1)
                            && punct(k + 2, "=")
                        {
                            let is_fn = word(k + 3) == Some("function")
                                || (punct(k + 3, "(") && arrow_after(text, &toks, k + 3))
                                || (word(k + 3).is_some() && punct(k + 4, "=>"));
                            if is_fn {
                                let sig = if punct(k + 3, "(") {
                                    signature(text, &toks, k + 3)
                                } else {
                                    word(k + 3).unwrap_or("").to_string()
                                };
                                push(Item::new(name, Kind::Function, &format!("{name}({sig})")));
                            } else {
                                push(Item::new(name, Kind::Variable, "local constant"));
                            }
                        }
                    }
                    Some("param") if punct(k + 1, "(") && k >= 2 && punct(k - 1, ":") => {
                        if let Some(name) = word(k - 2) {
                            let args = signature(text, &toks, k + 1);
                            push(Item::new(
                                name,
                                Kind::Param,
                                &format!("parameter: param({args})"),
                            ));
                        }
                    }
                    Some("pins") if punct(k + 1, ":") => {
                        // ["A", "K"] or { A: digital(), ... }
                        let mut j = k + 2;
                        let open = toks.get(j).map(|t| &text[t.range.clone()]);
                        let close = match open {
                            Some("[") => "]",
                            Some("{") => "}",
                            _ => continue,
                        };
                        let mut depth = 0;
                        while j < toks.len() {
                            let s = &text[toks[j].range.clone()];
                            if toks[j].kind == Tk::Punct && (s == "[" || s == "{" || s == "(") {
                                depth += 1;
                            } else if toks[j].kind == Tk::Punct
                                && (s == "]" || s == "}" || s == ")")
                            {
                                depth -= 1;
                                if depth == 0 && s == close {
                                    break;
                                }
                            } else if depth == 1 {
                                if close == "]" && toks[j].kind == Tk::Str {
                                    push(Item::new(
                                        s.trim_matches(|c| c == '"' || c == '\''),
                                        Kind::Pin,
                                        "pin",
                                    ));
                                } else if close == "}"
                                    && (toks[j].kind == Tk::Ident || toks[j].kind == Tk::Str)
                                    && punct(j + 1, ":")
                                {
                                    push(Item::new(
                                        s.trim_matches(|c| c == '"' || c == '\''),
                                        Kind::Pin,
                                        "pin",
                                    ));
                                }
                            }
                            j += 1;
                        }
                    }
                    Some("states") if punct(k + 1, ":") && punct(k + 2, "{") => {
                        let mut j = k + 3;
                        while j < toks.len() && !punct(j, "}") {
                            if toks[j].kind == Tk::Ident && punct(j + 1, ":") {
                                push(Item::new(
                                    &text[toks[j].range.clone()],
                                    Kind::Variable,
                                    "state",
                                ));
                            }
                            j += 1;
                        }
                    }
                    _ => {}
                }
            }
        }
        Lang::Spice => {
            for line in text.lines() {
                let l = line.trim();
                let mut parts = l.split_whitespace();
                match parts.next().map(|w| w.to_ascii_lowercase()).as_deref() {
                    Some(".model") | Some(".subckt") => {
                        if let Some(name) = parts.next() {
                            let name = name.split('(').next().unwrap_or(name);
                            push(Item::new(name, Kind::Function, l));
                        }
                    }
                    _ => {}
                }
            }
        }
        Lang::Expr => {}
    }
    out
}

fn signature(text: &str, toks: &[Tok], open: usize) -> String {
    let Some(t) = toks.get(open) else {
        return String::new();
    };
    if &text[t.range.clone()] != "(" {
        return String::new();
    }
    let mut depth = 0;
    for (j, tk) in toks.iter().enumerate().skip(open) {
        let s = &text[tk.range.clone()];
        if tk.kind == Tk::Punct && (s == "(" || s == "{" || s == "[") {
            depth += 1;
        } else if tk.kind == Tk::Punct && (s == ")" || s == "}" || s == "]") {
            depth -= 1;
            if depth == 0 {
                let inner = &text[t.range.end..toks[j].range.start];
                return inner.split_whitespace().collect::<Vec<_>>().join(" ");
            }
        }
    }
    String::new()
}

fn arrow_after(text: &str, toks: &[Tok], open: usize) -> bool {
    let mut depth = 0;
    for (j, tk) in toks.iter().enumerate().skip(open) {
        let s = &text[tk.range.clone()];
        if s == "(" {
            depth += 1;
        } else if s == ")" {
            depth -= 1;
            if depth == 0 {
                return toks
                    .get(j + 1)
                    .is_some_and(|t| &text[t.range.clone()] == "=>");
            }
        }
    }
    false
}

/// First line of a `/** ... */` comment right before `at`.
fn doc_before(text: &str, at: usize) -> String {
    let before = text[..at].trim_end();
    if !before.ends_with("*/") {
        return String::new();
    }
    let Some(start) = before.rfind("/**") else {
        return String::new();
    };
    let body: String = before[start + 3..before.len() - 2]
        .lines()
        .map(|l| l.trim().trim_start_matches('*').trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if body.is_empty() {
        String::new()
    } else {
        format!(" — {body}")
    }
}

/// The built-in vocabulary of a language.
pub fn builtins(lang: Lang) -> Vec<Item> {
    let mut v = Vec::new();
    match lang {
        Lang::Js => {
            v.extend(
                JS_KEYWORDS
                    .iter()
                    .map(|k| Item::new(k, Kind::Keyword, "keyword")),
            );
            v.extend(JS_API.iter().map(|(k, d)| Item::new(k, Kind::Api, d)));
            v.extend(SPEC_KEYS.iter().map(|(k, d)| Item::new(k, Kind::Api, d)));
            v.extend(
                EXPR_FUNCS
                    .iter()
                    .filter(|(k, _)| !matches!(*k, "t" | "temp" | "pi" | "vt"))
                    .map(|(k, d)| Item::new(k, Kind::Builtin, d)),
            );
        }
        Lang::Expr => {
            v.extend(
                EXPR_FUNCS
                    .iter()
                    .map(|(k, d)| Item::new(k, Kind::Builtin, d)),
            );
            v.push(Item::new("V", Kind::Api, "V(a) or V(a, b) — node voltage"));
            v.push(Item::new("I", Kind::Api, "I(branch) — branch current"));
        }
        Lang::Spice => {
            v.extend(SPICE_CARDS.iter().map(|(k, d)| Item::new(k, Kind::Card, d)));
            v.extend(
                SPICE_TYPES
                    .iter()
                    .map(|k| Item::new(k, Kind::Keyword, "model type")),
            );
        }
    }
    v
}

// ---------------------------------------------------------------- lexing

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tk {
    Ident,
    Num,
    Str,
    /// A template literal (backticks), possibly an `e`-template.
    Template,
    Comment,
    Punct,
    Space,
}

#[derive(Debug, Clone)]
struct Tok {
    kind: Tk,
    range: std::ops::Range<usize>,
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == '$'
}
fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// A forgiving JavaScript lexer (also used for the expression language and,
/// with `spice`, SPICE text); every byte belongs to one token.
fn lex(text: &str, lang: Lang) -> Vec<Tok> {
    let mut out = Vec::new();
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let end_of = |k: usize| chars.get(k).map_or(text.len(), |c| c.0);
    let mut k = 0;
    let mut line_start = true;
    while k < chars.len() {
        let (at, ch) = chars[k];
        let start = k;
        let kind;
        if ch.is_whitespace() {
            while k < chars.len() && chars[k].1.is_whitespace() {
                if chars[k].1 == '\n' {
                    line_start = true;
                }
                k += 1;
            }
            out.push(Tok {
                kind: Tk::Space,
                range: at..end_of(k),
            });
            continue;
        }
        let spice_comment = lang == Lang::Spice && ((line_start && ch == '*') || ch == ';');
        line_start = false;
        if spice_comment || (lang == Lang::Js && text[at..].starts_with("//")) {
            while k < chars.len() && chars[k].1 != '\n' {
                k += 1;
            }
            kind = Tk::Comment;
        } else if lang == Lang::Js && text[at..].starts_with("/*") {
            k += 2;
            while k < chars.len() && !text[chars[k].0..].starts_with("*/") {
                k += 1;
            }
            k = (k + 2).min(chars.len());
            kind = Tk::Comment;
        } else if lang == Lang::Js && (ch == '"' || ch == '\'') {
            k += 1;
            while k < chars.len() && chars[k].1 != ch && chars[k].1 != '\n' {
                if chars[k].1 == '\\' {
                    k += 1;
                }
                k += 1;
            }
            k = (k + 1).min(chars.len());
            kind = Tk::Str;
        } else if lang == Lang::Js && ch == '`' {
            k += 1;
            while k < chars.len() && chars[k].1 != '`' {
                if chars[k].1 == '\\' {
                    k += 1;
                }
                k += 1;
            }
            k = (k + 1).min(chars.len());
            kind = Tk::Template;
        } else if ch.is_ascii_digit()
            || (ch == '.' && chars.get(k + 1).is_some_and(|c| c.1.is_ascii_digit()))
        {
            while k < chars.len()
                && (chars[k].1.is_alphanumeric()
                    || chars[k].1 == '.'
                    || chars[k].1 == 'µ'
                    || chars[k].1 == 'Ω')
            {
                if matches!(chars[k].1, 'e' | 'E')
                    && chars.get(k + 1).is_some_and(|c| c.1 == '-' || c.1 == '+')
                {
                    k += 1;
                }
                k += 1;
            }
            kind = Tk::Num;
        } else if is_ident_start(ch) || (lang == Lang::Spice && ch == '.') {
            k += 1;
            while k < chars.len()
                && (is_ident(chars[k].1) || (lang != Lang::Js && chars[k].1 == '.'))
            {
                k += 1;
            }
            kind = Tk::Ident;
        } else {
            let two = text[at..].get(..2).unwrap_or("");
            let three = text[at..].get(..3).unwrap_or("");
            k += if ["===", "!==", "**="].contains(&three) {
                3
            } else if [
                "=>", "==", "!=", "<=", ">=", "&&", "||", "**", "+=", "-=", "*=", "/=",
            ]
            .contains(&two)
            {
                2
            } else {
                1
            };
            kind = Tk::Punct;
        }
        let _ = start;
        out.push(Tok {
            kind,
            range: at..end_of(k),
        });
    }
    out
}

fn lex_js(text: &str) -> Vec<Tok> {
    lex(text, Lang::Js)
        .into_iter()
        .filter(|t| t.kind != Tk::Space && t.kind != Tk::Comment)
        .collect()
}

/// Colour the text.
pub fn highlight(text: &str, lang: Lang, vocab: &[Item], font: egui::FontId) -> LayoutJob {
    let mut job = LayoutJob::default();
    let fmt = |color: Color32| TextFormat {
        font_id: font.clone(),
        color,
        ..Default::default()
    };
    let class = |w: &str| vocab.iter().find(|i| i.label == w).map(|i| i.kind);
    let toks = lex(text, lang);
    for (n, t) in toks.iter().enumerate() {
        let s = &text[t.range.clone()];
        let prev_word = toks[..n]
            .iter()
            .rev()
            .find(|p| p.kind != Tk::Space)
            .map(|p| &text[p.range.clone()]);
        let color = match t.kind {
            Tk::Space | Tk::Punct => c::TEXT,
            Tk::Comment => c::TEXT_FAINT,
            Tk::Num => palette::NUMBER,
            Tk::Str => {
                let inner = s.trim_matches(|c| c == '"' || c == '\'');
                if class(inner) == Some(Kind::Pin) {
                    palette::PIN
                } else {
                    palette::STRING
                }
            }
            Tk::Template if prev_word == Some("e") => {
                // An e-template: colour its content as the expression language.
                job.append("`", 0.0, fmt(palette::STRING));
                let inner = if s.len() > 1 && s.ends_with('`') {
                    &s[1..s.len() - 1]
                } else {
                    &s[1..]
                };
                let sub = highlight(inner, Lang::Expr, vocab, font.clone());
                for sec in &sub.sections {
                    job.append(
                        &inner[sec.byte_range.start.0..sec.byte_range.end.0],
                        0.0,
                        sec.format.clone(),
                    );
                }
                if s.len() > 1 && s.ends_with('`') {
                    job.append("`", 0.0, fmt(palette::STRING));
                }
                continue;
            }
            Tk::Template => palette::STRING,
            Tk::Ident => {
                let member = prev_word == Some(".");
                match lang {
                    Lang::Spice => {
                        if s.starts_with('.') {
                            kind_color(Kind::Card)
                        } else if SPICE_TYPES.iter().any(|k| k.eq_ignore_ascii_case(s)) {
                            palette::KEYWORD
                        } else if class(s) == Some(Kind::Function) {
                            palette::FUNCTION
                        } else {
                            c::TEXT
                        }
                    }
                    _ if member => {
                        if MATH.iter().any(|m| m.0 == s)
                            || EQ_METHODS.iter().any(|m| m.0 == s)
                            || CTX_MEMBERS.iter().any(|m| m.0 == s)
                        {
                            palette::API
                        } else {
                            c::TEXT
                        }
                    }
                    _ => match class(s) {
                        Some(k) => kind_color(k),
                        None if lang == Lang::Expr && s.contains('.') => palette::CODE,
                        None => c::TEXT,
                    },
                }
            }
        };
        job.append(s, 0.0, fmt(color));
    }
    job
}

// ---------------------------------------------------------- the widget

#[derive(Debug, Clone, Default)]
struct Popup {
    open: bool,
    sel: usize,
    /// Char range the accepted item replaces.
    range: (usize, usize),
    items: Vec<Item>,
    /// Do not reopen for this prefix start (after Escape).
    dismissed: Option<usize>,
}

/// Options for [`code_editor`].
pub struct Opts<'a> {
    pub rows: usize,
    pub single_line: bool,
    pub width: f32,
    pub hint: &'a str,
    /// Show the signature line under the editor.
    pub help: bool,
}

impl Default for Opts<'_> {
    fn default() -> Self {
        Self {
            rows: 12,
            single_line: false,
            width: f32::INFINITY,
            hint: "",
            help: true,
        }
    }
}

fn char_to_byte(text: &str, ci: usize) -> usize {
    text.char_indices().nth(ci).map_or(text.len(), |(b, _)| b)
}

/// Completion context at a char cursor: replaced range and the candidates.
fn context(
    text: &str,
    cursor: usize,
    lang: Lang,
    vocab: &[Item],
) -> Option<((usize, usize), Vec<Item>, bool)> {
    let chars: Vec<char> = text.chars().collect();
    let cursor = cursor.min(chars.len());
    let word_char = |c: char| is_ident(c) || (lang != Lang::Js && c == '.');
    let mut start = cursor;
    while start > 0 && word_char(chars[start - 1]) {
        start -= 1;
    }
    let mut end = cursor;
    while end < chars.len() && word_char(chars[end]) {
        end += 1;
    }
    let prefix: String = chars[start..cursor].iter().collect();
    let line_start = chars[..cursor]
        .iter()
        .rposition(|c| *c == '\n')
        .map_or(0, |k| k + 1);
    let line: String = chars[line_start..cursor].iter().collect();
    let mut member = false;
    let candidates: Vec<Item> = if lang == Lang::Js {
        // Inside a string: pin and node names.
        let quotes = line.chars().filter(|c| *c == '"').count()
            + line.chars().filter(|c| *c == '\'').count();
        let backticks = text[..char_to_byte(text, cursor)].matches('`').count();
        if quotes % 2 == 1 {
            member = true;
            vocab
                .iter()
                .filter(|i| i.kind == Kind::Pin)
                .cloned()
                .collect()
        } else if backticks % 2 == 1 {
            // Inside a template: the expression language and declared names.
            let mut v = builtins(Lang::Expr);
            v.extend(
                vocab
                    .iter()
                    .filter(|i| matches!(i.kind, Kind::Param | Kind::Variable | Kind::Pin))
                    .cloned(),
            );
            v
        } else if start > 0 && chars[start - 1] == '.' {
            member = true;
            let mut o = start - 1;
            while o > 0 && is_ident(chars[o - 1]) {
                o -= 1;
            }
            let obj: String = chars[o..start - 1].iter().collect();
            let table: &[(&str, &str)] = match obj.as_str() {
                "Math" => &MATH,
                "ctx" => &CTX_MEMBERS,
                "c" => &EQ_METHODS,
                _ => &[],
            };
            table
                .iter()
                .map(|(k, d)| Item::new(k, Kind::Method, d))
                .collect()
        } else {
            vocab.to_vec()
        }
    } else {
        vocab.to_vec()
    };
    if prefix.is_empty() && !member {
        return None;
    }
    let lower = prefix.to_lowercase();
    let mut starts: Vec<Item> = Vec::new();
    let mut contains: Vec<Item> = Vec::new();
    for it in candidates {
        let l = it.label.to_lowercase();
        if it.label == prefix || starts.iter().chain(&contains).any(|o| o.label == it.label) {
            continue;
        }
        if l.starts_with(&lower) {
            starts.push(it);
        } else if lower.len() >= 2 && l.contains(&lower) {
            contains.push(it);
        }
    }
    starts.sort_by(|a, b| {
        (a.kind == Kind::Keyword)
            .cmp(&(b.kind == Kind::Keyword))
            .then(a.label.len().cmp(&b.label.len()))
    });
    starts.extend(contains);
    starts.truncate(40);
    if starts.is_empty() {
        return None;
    }
    Some(((start, end.max(cursor)), starts, member))
}

/// The call around the cursor: the name before the innermost open `(`.
fn enclosing_call(text: &str, cursor: usize) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut depth = 0i32;
    let mut k = cursor.min(chars.len());
    while k > 0 {
        k -= 1;
        match chars[k] {
            ')' => depth += 1,
            '(' if depth == 0 => {
                let end = k;
                let mut s = k;
                while s > 0 && (is_ident(chars[s - 1]) || chars[s - 1] == '.') {
                    s -= 1;
                }
                let name: String = chars[s..end].iter().collect();
                return (!name.is_empty()).then_some(name);
            }
            '(' => depth -= 1,
            '\n' if depth == 0 && k + 1 < cursor && chars.get(k.wrapping_sub(1)) == Some(&'\n') => {
                return None;
            }
            _ => {}
        }
    }
    None
}

fn word_at(text: &str, cursor: usize, lang: Lang) -> String {
    let chars: Vec<char> = text.chars().collect();
    let wc = |c: char| is_ident(c) || (lang != Lang::Js && c == '.');
    let mut s = cursor.min(chars.len());
    while s > 0 && wc(chars[s - 1]) {
        s -= 1;
    }
    let mut e = cursor.min(chars.len());
    while e < chars.len() && wc(chars[e]) {
        e += 1;
    }
    chars[s..e].iter().collect()
}

fn lookup(name: &str, vocab: &[Item]) -> Option<Item> {
    let short = name.rsplit('.').next().unwrap_or(name);
    if let Some(m) = name.strip_prefix("Math.") {
        return MATH
            .iter()
            .find(|x| x.0 == m)
            .map(|(k, d)| Item::new(k, Kind::Method, d));
    }
    if name.contains('.') && !vocab.iter().any(|i| i.label == name) {
        let all = EQ_METHODS.iter().chain(CTX_MEMBERS.iter());
        if let Some((k, d)) = all.clone().find(|x| x.0 == short) {
            return Some(Item::new(k, Kind::Method, d));
        }
    }
    vocab.iter().find(|i| i.label == name).cloned()
}

/// A code editor with highlighting and suggestions. `extra` adds caller
/// knowledge (pins, scope codes). Returns the text edit's response.
pub fn code_editor(
    ui: &mut Ui,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    text: &mut String,
    lang: Lang,
    extra: &[Item],
    opts: Opts<'_>,
) -> egui::Response {
    let id = ui.make_persistent_id(id_salt);
    let te_id = id.with("text");
    let mut vocab = builtins(lang);
    for it in declared(lang, text)
        .into_iter()
        .chain(extra.iter().cloned())
    {
        if let Some(slot) = vocab.iter_mut().find(|v| v.label == it.label) {
            *slot = it;
        } else {
            vocab.push(it);
        }
    }
    let mut pop: Popup = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    let focused = ui.memory(|m| m.has_focus(te_id));
    let mut accept: Option<usize> = None;
    if pop.open && focused && !pop.items.is_empty() {
        ui.input_mut(|i| {
            if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                pop.sel = (pop.sel + 1) % pop.items.len();
            }
            if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                pop.sel = (pop.sel + pop.items.len() - 1) % pop.items.len();
            }
            if i.consume_key(Modifiers::NONE, Key::Tab)
                || i.consume_key(Modifiers::NONE, Key::Enter)
            {
                accept = Some(pop.sel);
            }
            if i.consume_key(Modifiers::NONE, Key::Escape) {
                pop.open = false;
                pop.dismissed = Some(pop.range.0);
            }
        });
    }
    let apply = |text: &mut String, pop: &mut Popup, k: usize, ui: &Ui| {
        let Some(item) = pop.items.get(k).cloned() else {
            return;
        };
        let (a, b) = (
            char_to_byte(text, pop.range.0),
            char_to_byte(text, pop.range.1),
        );
        text.replace_range(a..b, &item.insert);
        let caret = pop.range.0 + item.insert.chars().count();
        let mut state = egui::TextEdit::load_state(ui.ctx(), te_id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(CCursorRange::one(CCursor::new(caret))));
        state.store(ui.ctx(), te_id);
        pop.open = false;
        pop.items.clear();
    };
    if let Some(k) = accept {
        apply(text, &mut pop, k, ui);
    }
    // Enter that reaches the editor: keep the indentation (one more level after `{`).
    let newline = !opts.single_line
        && focused
        && accept.is_none()
        && ui.input(|i| i.key_pressed(Key::Enter) && i.modifiers.is_none());
    let font = theme::mono(12.5);
    let vocab_ref = &vocab;
    let font2 = font.clone();
    let mut layouter = move |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
        let mut job = highlight(buf.as_str(), lang, vocab_ref, font2.clone());
        job.wrap.max_width = wrap;
        ui.ctx().fonts_mut(|f| f.layout_job(job))
    };
    let edit = if opts.single_line {
        egui::TextEdit::singleline(text)
    } else {
        egui::TextEdit::multiline(text)
            .code_editor()
            .desired_rows(opts.rows)
    };
    let out = edit
        .id(te_id)
        .font(font)
        .desired_width(opts.width)
        .hint_text(opts.hint)
        .layouter(&mut layouter)
        .show(ui);
    let mut cursor = out.cursor_range.map(|r| r.primary.index.0);
    if newline
        && out.response.changed()
        && let Some(cur) = cursor
    {
        let at = char_to_byte(text, cur);
        if text[..at].ends_with('\n') {
            let before = &text[..at - 1];
            let line = &before[before.rfind('\n').map_or(0, |k| k + 1)..];
            let mut indent: String = line
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect();
            if line.trim_end().ends_with(['{', '(', '[']) {
                indent.push_str("  ");
            }
            if !indent.is_empty() {
                text.insert_str(at, &indent);
                let caret = cur + indent.chars().count();
                let mut state = egui::TextEdit::load_state(ui.ctx(), te_id).unwrap_or_default();
                state
                    .cursor
                    .set_char_range(Some(CCursorRange::one(CCursor::new(caret))));
                state.store(ui.ctx(), te_id);
                cursor = Some(caret);
            }
        }
    }
    if let Some(cur) = cursor {
        let changed = out.response.changed();
        match context(text, cur, lang, &vocab) {
            Some((range, items, member)) => {
                if pop.dismissed != Some(range.0)
                    && (changed || (pop.open && range.0 == pop.range.0))
                {
                    if !pop.open || pop.range.0 != range.0 || items != pop.items {
                        pop.sel = 0;
                    }
                    pop.open = changed || pop.open || member;
                    pop.range = range;
                    pop.items = items;
                } else if pop.range.0 != range.0 {
                    pop.open = false;
                }
            }
            None => {
                pop.open = false;
                pop.dismissed = None;
            }
        }
        if !out.response.has_focus() && !focused {
            pop.open = false;
        }
        // Suggestions popup under the cursor.
        if pop.open && !pop.items.is_empty() {
            let caret = out.galley.pos_from_cursor(CCursor::new(cur));
            let at = out.galley_pos + caret.left_bottom().to_vec2() + vec2(0.0, 2.0);
            let mut clicked = None;
            egui::Area::new(id.with("popup"))
                .order(egui::Order::Foreground)
                .fixed_pos(at)
                .show(ui.ctx(), |ui| {
                    theme::card().show(ui, |ui| {
                        ui.set_max_width(460.0);
                        ui.spacing_mut().item_spacing.y = 1.0;
                        egui::ScrollArea::vertical()
                            .max_height(220.0)
                            .show(ui, |ui| {
                                for (k, it) in pop.items.iter().enumerate() {
                                    let sel = k == pop.sel;
                                    let r = ui.horizontal(|ui| {
                                        ui.label(
                                            RichText::new(kind_tag(it.kind))
                                                .font(theme::mono(10.0))
                                                .color(kind_color(it.kind)),
                                        );
                                        ui.label(
                                            RichText::new(&it.label).font(theme::mono(12.0)).color(
                                                if sel { c::TEXT } else { kind_color(it.kind) },
                                            ),
                                        );
                                    });
                                    let rr = r.response.rect.expand2(vec2(4.0, 1.0));
                                    if sel {
                                        ui.painter().rect_filled(
                                            rr,
                                            3.0,
                                            c::ACTIVE_TAB_BG.gamma_multiply(0.6),
                                        );
                                        r.response.scroll_to_me(None);
                                    }
                                    if ui
                                        .interact(rr, id.with(("item", k)), Sense::click())
                                        .clicked()
                                    {
                                        clicked = Some(k);
                                    }
                                }
                            });
                        if let Some(it) = pop.items.get(pop.sel) {
                            ui.separator();
                            ui.label(RichText::new(&it.detail).color(c::TEXT_DIM).small());
                        }
                    });
                });
            if let Some(k) = clicked {
                apply(text, &mut pop, k, ui);
                ui.memory_mut(|m| m.request_focus(te_id));
            }
        }
        // Signature help.
        if opts.help && !opts.single_line {
            let word = word_at(text, cur, lang);
            let info = enclosing_call(text, cur)
                .and_then(|n| lookup(&n, &vocab))
                .or_else(|| lookup(&word, &vocab));
            let line = info.map(|i| i.detail).unwrap_or_else(|| match lang {
                Lang::Js => "Ctrl+Space: suggestions · Tab/Enter: accept · Esc: close".into(),
                _ => String::new(),
            });
            ui.label(
                RichText::new(line)
                    .font(theme::mono(11.0))
                    .color(c::TEXT_DIM),
            );
        }
    } else if opts.help && !opts.single_line {
        ui.label(RichText::new(" ").font(theme::mono(11.0)));
    }
    // Ctrl+Space opens suggestions for the word at the cursor.
    if focused && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Space)) {
        pop.dismissed = None;
        if let Some(cur) = cursor
            && let Some((range, items, _)) = context(text, cur, lang, &vocab)
                .or_else(|| Some(((cur, cur), vocab.clone(), false)))
        {
            pop.open = true;
            pop.sel = 0;
            pop.range = range;
            pop.items = items.into_iter().take(60).collect();
        }
    }
    ui.data_mut(|d| d.insert_temp(id, pop));
    out.response.response
}

fn kind_tag(k: Kind) -> &'static str {
    match k {
        Kind::Keyword => "kw ",
        Kind::Api => "api",
        Kind::Method => "mth",
        Kind::Builtin => "fn ",
        Kind::Function => "def",
        Kind::Variable => "var",
        Kind::Param => "par",
        Kind::Pin => "pin",
        Kind::Code => "cod",
        Kind::Card => "crd",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declarations_are_found() {
        let src = crate::components::template_js("local.x", "X", &["A".into(), "B".into()]);
        let d = declared(Lang::Js, &src);
        let get = |n: &str| d.iter().find(|i| i.label == n).map(|i| i.kind);
        assert_eq!(get("conductance"), Some(Kind::Function));
        assert_eq!(get("R"), Some(Kind::Param));
        assert_eq!(get("Imax"), Some(Kind::Param));
        assert_eq!(get("A"), Some(Kind::Pin));
        assert_eq!(get("Ir"), Some(Kind::Variable));
        let f = d.iter().find(|i| i.label == "conductance").unwrap();
        assert!(
            f.detail.contains("conductance(v, R, Imax)") && f.detail.contains("soft current limit"),
            "{}",
            f.detail
        );
        let digital = "export default component({ pins: { D: digital(), Q: digitalOut(\"ttl\") }, states: { q: 0 } });";
        let d = declared(Lang::Js, digital);
        assert!(
            d.iter().any(|i| i.label == "Q" && i.kind == Kind::Pin),
            "{d:?}"
        );
        assert!(
            d.iter().any(|i| i.label == "q" && i.kind == Kind::Variable),
            "{d:?}"
        );
    }

    #[test]
    fn completion_contexts() {
        let src = "function shockley(vd) { return vd; }\nexport default component({ pins: [\"A\", \"K\"], params: { Is: param(1) },\n equations(c) { c.cu";
        let mut vocab = builtins(Lang::Js);
        vocab.extend(declared(Lang::Js, src));
        let n = src.chars().count();
        let (range, items, member) = context(src, n, Lang::Js, &vocab).unwrap();
        assert!(member);
        assert_eq!(items[0].label, "current");
        assert_eq!(range.1 - range.0, 2);
        let s2 = format!("{src}rrent(\"A\", \"K\", () => sho");
        let (_, items, _) = context(&s2, s2.chars().count(), Lang::Js, &vocab).unwrap();
        assert_eq!(items[0].label, "shockley");
        let s3 = format!("{src}rrent(\"");
        let (_, items, _) = context(&s3, s3.chars().count(), Lang::Js, &vocab).unwrap();
        assert!(
            items.iter().all(|i| i.kind == Kind::Pin) && items.len() == 2,
            "{items:?}"
        );
        let s4 = format!("{src}rrent(\"A\", \"K\", () => Math.ta");
        let (_, items, _) = context(&s4, s4.chars().count(), Lang::Js, &vocab).unwrap();
        assert_eq!(
            items.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(),
            ["tan", "tanh", "atan"]
        );
        assert_eq!(
            enclosing_call(&s2, s2.chars().count()).as_deref(),
            Some("c.current")
        );
        // Expression language: codes with dots.
        let codes = vec![
            Item::new("R1.A.V", Kind::Code, "voltage"),
            Item::new("R1.I", Kind::Code, "current"),
        ];
        let mut v = builtins(Lang::Expr);
        v.extend(codes);
        let (range, items, _) = context("out - R1.", 9, Lang::Expr, &v).unwrap();
        assert_eq!(range, (6, 9));
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn highlighting_covers_the_text() {
        let src = crate::components::template_js("local.x", "X", &["A".into(), "B".into()])
            + "\nconst q = e`V(A, B) * limexp(x)`; /* c */ 1e-3";
        let mut vocab = builtins(Lang::Js);
        vocab.extend(declared(Lang::Js, &src));
        let job = highlight(&src, Lang::Js, &vocab, egui::FontId::monospace(12.0));
        assert_eq!(job.text, src);
        let colour_of = |needle: &str| {
            let at = src.find(needle).unwrap();
            job.sections
                .iter()
                .find(|s| s.byte_range.start.0 <= at && at < s.byte_range.end.0)
                .unwrap()
                .format
                .color
        };
        assert_eq!(colour_of("function conductance"), palette::KEYWORD);
        assert_eq!(colour_of("conductance("), palette::FUNCTION);
        assert_eq!(colour_of("limexp(x)"), palette::BUILTIN);
        assert_eq!(colour_of("1e-3"), palette::NUMBER);
        for (lang, text) in [
            (
                Lang::Spice,
                "* comment\n.model D1N4148 D(IS=2.52n N=1.75)\n.subckt X a b\nR1 a b 1k ; tail\n.ends",
            ),
            (Lang::Expr, "R1.I * (in - out) + 2.5e-3"),
        ] {
            let job = highlight(text, lang, &builtins(lang), egui::FontId::monospace(12.0));
            assert_eq!(job.text, text);
        }
    }
}
