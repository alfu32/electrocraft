//! JavaScript components (spec section 6, phase P6), run on QuickJS.
//!
//! A script runs once at load time to describe a [`ComponentDef`]: pins,
//! parameters, and equations written with the `e` tagged template, which
//! Rust parses with [`ecs_expr::parse`] (one expression language everywhere).
//! After that, only behavior hooks run, on events, never inside the Newton
//! loop. Each thread that calls a hook owns its own QuickJS runtime.
//!
//! Sandbox: the ECMAScript standard library and the component API only. No
//! file system, network, timers or module imports; `eval` and the `Function`
//! constructors are removed; memory is capped and each call has a time budget.

use ecs_model::{
    Action, Behavior, BranchDef, ComponentDef, ControlDef, ControlEvent, EventCtx, Level, LimitDef,
    Meta, ParamDef, ParamKind, PinDef, PinKind, ProbeDef, StateDef,
};
use rquickjs::{Context, Function, Runtime};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// TypeScript declarations for editors (shipped beside user components).
pub const DTS: &str = include_str!("electrocraft.d.ts");

/// Memory cap per QuickJS runtime.
pub const MEMORY_LIMIT: usize = 64 << 20;
/// Time budget for loading a script and for each hook call.
pub const TIME_BUDGET: Duration = Duration::from_millis(250);

/// A load or run error with its location in the script.
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptError {
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}: {}",
            self.file, self.line, self.column, self.message
        )
    }
}

impl std::error::Error for ScriptError {}

const PRELUDE: &str = include_str!("prelude.js");

static EPOCH: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

fn now_ns() -> u64 {
    EPOCH.get_or_init(Instant::now).elapsed().as_nanos() as u64
}

/// One sandboxed runtime with a script loaded.
struct Engine {
    _rt: Runtime,
    ctx: Context,
    deadline: Arc<AtomicU64>,
}

fn rewrite_module(source: &str) -> String {
    // Files are written as ES modules; the default export becomes a global.
    source.replacen("export default", "globalThis.__component =", 1)
}

fn exception_text(ctx: &rquickjs::Ctx<'_>, e: rquickjs::Error, file: &str) -> ScriptError {
    let (mut message, mut line, mut column) = (e.to_string(), 0, 0);
    if matches!(e, rquickjs::Error::Exception) {
        let ex = ctx.catch();
        if let Some(obj) = ex.as_object() {
            let msg: Option<String> = obj.get("message").ok();
            let stack: Option<String> = obj.get("stack").ok();
            message = msg.unwrap_or_else(|| "exception".into());
            if let Some(stack) = stack {
                // "    at f (file.js:12:5)" — take the first frame in this file.
                for frame in stack.lines() {
                    if let Some(rest) = frame.split(&format!("{file}:")).nth(1) {
                        let nums: Vec<usize> = rest
                            .trim_end_matches(')')
                            .split(':')
                            .filter_map(|n| n.parse().ok())
                            .collect();
                        line = nums.first().copied().unwrap_or(0);
                        column = nums.get(1).copied().unwrap_or(0);
                        break;
                    }
                }
            }
        } else if let Some(s) = ex.as_string().and_then(|s| s.to_string().ok()) {
            message = s;
        }
    }
    if message.contains("interrupted") {
        message = format!(
            "stopped: exceeded the {} ms time budget",
            TIME_BUDGET.as_millis()
        );
    }
    ScriptError {
        file: file.into(),
        line,
        column,
        message,
    }
}

impl Engine {
    fn new(source: &str, file: &str) -> Result<Self, ScriptError> {
        let err = |m: String| ScriptError {
            file: file.into(),
            line: 0,
            column: 0,
            message: m,
        };
        let rt = Runtime::new().map_err(|e| err(e.to_string()))?;
        rt.set_memory_limit(MEMORY_LIMIT);
        rt.set_max_stack_size(1 << 20);
        let deadline = Arc::new(AtomicU64::new(u64::MAX));
        let d2 = deadline.clone();
        rt.set_interrupt_handler(Some(Box::new(move || {
            now_ns() > d2.load(Ordering::Relaxed)
        })));
        let ctx = Context::full(&rt).map_err(|e| err(e.to_string()))?;
        let engine = Self {
            _rt: rt,
            ctx,
            deadline,
        };
        engine.arm();
        engine.ctx.with(|ctx| {
            let mut opts = rquickjs::context::EvalOptions::default();
            opts.global = true;
            opts.filename = Some("electrocraft-prelude.js".into());
            ctx.eval_with_options::<(), _>(PRELUDE, opts)
                .map_err(|e| exception_text(&ctx, e, "electrocraft-prelude.js"))?;
            let mut opts = rquickjs::context::EvalOptions::default();
            opts.global = true;
            opts.strict = true;
            opts.filename = Some(file.into());
            ctx.eval_with_options::<(), _>(rewrite_module(source), opts)
                .map_err(|e| exception_text(&ctx, e, file))
        })?;
        engine.disarm();
        Ok(engine)
    }

    fn arm(&self) {
        self.deadline
            .store(now_ns() + TIME_BUDGET.as_nanos() as u64, Ordering::Relaxed);
    }
    fn disarm(&self) {
        self.deadline.store(u64::MAX, Ordering::Relaxed);
    }

    fn call(
        &self,
        func: &str,
        args: (String, String, u32),
        file: &str,
    ) -> Result<String, ScriptError> {
        self.arm();
        let r = self.ctx.with(|ctx| {
            let f: Function = ctx
                .globals()
                .get(func)
                .map_err(|e| exception_text(&ctx, e, file))?;
            f.call::<_, String>(args)
                .map_err(|e| exception_text(&ctx, e, file))
        });
        self.disarm();
        r
    }
}

fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::Bool(b) => Some(f64::from(u8::from(*b))),
        Value::String(s) if s == "X" => Some(0.5),
        _ => None,
    }
}

fn s(v: &Value, k: &str) -> String {
    v.get(k)
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string()
}

/// Parse an `e` template's text, reporting the template's location.
fn expr(v: &Value, file: &str) -> Result<ecs_expr::Expr, ScriptError> {
    let src = v.get("src").and_then(|x| x.as_str()).unwrap_or_default();
    let loc = v.get("loc").and_then(|x| x.as_str()).unwrap_or_default();
    ecs_expr::parse(src).map_err(|e| {
        let nums: Vec<usize> = loc.split(':').filter_map(|n| n.parse().ok()).collect();
        ScriptError {
            file: file.into(),
            line: nums.first().copied().unwrap_or(0) + e.line.saturating_sub(1),
            column: nums.get(1).copied().unwrap_or(0),
            message: format!(
                "in e`{src}`: {} (column {} of the template)",
                e.message, e.column
            ),
        }
    })
}

/// Load a JavaScript component file into a definition.
pub fn load_component(source: &str, file: &str) -> Result<ComponentDef, ScriptError> {
    let engine = Engine::new(source, file)?;
    let json = engine.call(
        "__describeComponent",
        (String::new(), String::new(), 0),
        file,
    )?;
    let d: Value = serde_json::from_str(&json).map_err(|e| ScriptError {
        file: file.into(),
        line: 0,
        column: 0,
        message: e.to_string(),
    })?;
    let fail = |m: String| ScriptError {
        file: file.into(),
        line: 0,
        column: 0,
        message: m,
    };
    let id = s(&d, "id");
    if id.is_empty() {
        return Err(fail("component({...}) needs an id".into()));
    }
    let mut def = ComponentDef::new(
        &id,
        &s(&d, "name"),
        &s(&d, "category"),
        &s(&d, "prefix"),
        &s(&d, "symbol"),
    );
    def.meta = Meta {
        name: s(&d, "name"),
        category: s(&d, "category"),
        version: d.get("version").and_then(|v| v.as_u64()).unwrap_or(1) as u32,
        author: s(&d, "author"),
        description: s(&d, "description"),
        prefix: s(&d, "prefix"),
    };
    for p in d["pins"].as_array().into_iter().flatten() {
        let kind = match p["kind"].as_str() {
            Some("in") => PinKind::DigitalIn(s(p, "family")),
            Some("out") => PinKind::DigitalOut(s(p, "family")),
            _ => PinKind::Analog,
        };
        def.pins.push(PinDef {
            name: s(p, "name"),
            kind,
        });
    }
    for p in d["params"].as_array().into_iter().flatten() {
        let mut pd = ParamDef::real(
            &s(p, "name"),
            num(&p["default"]).unwrap_or(0.0),
            &s(p, "unit"),
            &s(p, "description"),
        );
        pd.live = p["live"].as_bool().unwrap_or(false);
        pd.min = num(&p["min"]);
        pd.max = num(&p["max"]);
        if let Some(vals) = p["values"].as_array() {
            pd.kind = ParamKind::Enum(
                vals.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect(),
            );
        }
        def.params.push(pd);
    }
    for st in d["states"].as_array().into_iter().flatten() {
        def.states.push(StateDef {
            name: s(st, "name"),
            initial: num(&st["initial"]).unwrap_or(0.0),
        });
    }
    for n in d["nodes"].as_array().into_iter().flatten() {
        def.internal_nodes
            .push(n.as_str().unwrap_or_default().to_string());
    }
    for l in d["lets"].as_array().into_iter().flatten() {
        def.lets.push((s(l, "name"), expr(&l["expr"], file)?));
    }
    for b in d["branches"].as_array().into_iter().flatten() {
        let opt = |k: &str| -> Result<Option<ecs_expr::Expr>, ScriptError> {
            if b[k].is_null() {
                Ok(None)
            } else {
                expr(&b[k], file).map(Some)
            }
        };
        def.branches.push(if b["kind"] == "voltage" {
            BranchDef::Voltage {
                name: s(b, "name"),
                a: s(b, "a"),
                b: s(b, "b"),
                v: expr(&b["v"], file)?,
                flux: opt("flux")?,
            }
        } else {
            BranchDef::Current {
                name: None,
                a: s(b, "a"),
                b: s(b, "b"),
                i: expr(&b["i"], file)?,
                q: opt("q")?,
            }
        });
    }
    for l in d["limits"].as_array().into_iter().flatten() {
        let ctl = expr(&l["expr"], file)?;
        let ecs_expr::Expr::V(a, Some(b)) = ctl else {
            return Err(fail(
                "c.limit(\"pn\", e`V(a, b)`): the expression must be a voltage difference".into(),
            ));
        };
        let has = |n: &str| def.params.iter().any(|p| p.name == n);
        let vte = if l["vte"].is_null() {
            ecs_expr::parse(if has("N") { "N * vt" } else { "vt" }).unwrap()
        } else {
            expr(&l["vte"], file)?
        };
        let is = if l["is"].is_null() {
            ecs_expr::parse(if has("Is") { "Is" } else { "1e-14" }).unwrap()
        } else {
            expr(&l["is"], file)?
        };
        def.limits.push(LimitDef::Pn { a, b, vte, is });
    }
    for p in d["probes"].as_array().into_iter().flatten() {
        def.probes.push(ProbeDef {
            name: s(p, "name"),
            expr: expr(&p["expr"], file)?,
            unit: s(p, "unit"),
        });
    }
    for c in d["controls"].as_array().into_iter().flatten() {
        let label = s(c, "label");
        match c["kind"].as_str() {
            Some("toggle") => def.controls.push(ControlDef::Toggle {
                state: s(c, "state"),
                label,
            }),
            Some("momentary") => def.controls.push(ControlDef::Momentary {
                state: s(c, "state"),
                label,
            }),
            Some("slider") => def.controls.push(ControlDef::Slider {
                param: s(c, "param"),
                label,
            }),
            _ => {}
        }
    }
    let hooks = &d["hooks"];
    if ["inputs", "timer", "control", "start"]
        .iter()
        .any(|h| hooks[h].as_bool() == Some(true))
    {
        def.behavior = Some(Arc::new(JsBehavior {
            key: format!("{file}#{:x}", fxhash(source)),
            file: file.into(),
            source: source.into(),
            errors: Default::default(),
        }));
    }
    Ok(def)
}

fn fxhash(s: &str) -> u64 {
    s.bytes().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100000001b3)
    })
}

thread_local! {
    static ENGINES: RefCell<HashMap<String, Result<Engine, ScriptError>>> = RefCell::new(HashMap::new());
}

/// Behavior hooks implemented in JavaScript. Each thread lazily builds its
/// own runtime for the script; values never cross threads.
pub struct JsBehavior {
    key: String,
    file: String,
    source: String,
    /// Last hook error, shown in the problems panel.
    pub errors: std::sync::Mutex<Option<ScriptError>>,
}

fn level_json(l: Level) -> Value {
    match l {
        Level::Low => Value::from(0),
        Level::High => Value::from(1),
        Level::X => Value::from("X"),
    }
}

impl JsBehavior {
    fn run(&self, which: &str, ctx: &mut EventCtx<'_>, tag: u32, event: Option<&ControlEvent>) {
        let payload = serde_json::json!({
            "time": ctx.time,
            "state": ctx.state.iter().map(|(k, v)| (k.clone(), Value::from(*v))).collect::<serde_json::Map<_, _>>(),
            "params": ctx.params.iter().map(|(k, v)| (k.clone(), Value::from(*v))).collect::<serde_json::Map<_, _>>(),
            "levels": ctx.levels.iter().map(|(k, v)| (k.clone(), level_json(*v))).collect::<serde_json::Map<_, _>>(),
            "previous": ctx.previous.iter().map(|(k, v)| (k.clone(), level_json(*v))).collect::<serde_json::Map<_, _>>(),
            "voltages": ctx.voltages.iter().map(|(k, v)| (k.clone(), Value::from(*v))).collect::<serde_json::Map<_, _>>(),
            "event": match event {
                Some(ControlEvent::SetState { state, value }) => serde_json::json!({"kind": "state", "name": state, "value": value}),
                Some(ControlEvent::SetParam { param, value }) => serde_json::json!({"kind": "param", "name": param, "value": value}),
                None => Value::Null,
            },
        });
        let result = ENGINES.with(|engines| {
            let mut engines = engines.borrow_mut();
            let engine = engines
                .entry(self.key.clone())
                .or_insert_with(|| Engine::new(&self.source, &self.file));
            match engine {
                Ok(engine) => engine.call(
                    "__runHook",
                    (which.to_string(), payload.to_string(), tag),
                    &self.file,
                ),
                Err(e) => Err(e.clone()),
            }
        });
        match result.and_then(|json| {
            serde_json::from_str::<Value>(&json).map_err(|e| ScriptError {
                file: self.file.clone(),
                line: 0,
                column: 0,
                message: e.to_string(),
            })
        }) {
            Ok(v) => {
                if let Some(state) = v["state"].as_object() {
                    for (k, x) in state {
                        if let Some(n) = num(x) {
                            ctx.state.insert(k.clone(), n);
                        }
                    }
                }
                for a in v["actions"].as_array().into_iter().flatten() {
                    let delay = num(&a["delay"]).unwrap_or(0.0);
                    if let Some(tag) = a["timer"].as_u64() {
                        ctx.actions.push(Action::Timer {
                            delay,
                            tag: tag as u32,
                        });
                    } else {
                        let level = match &a["level"] {
                            Value::String(s) if s == "X" => Level::X,
                            x => Level::from_bool(num(x).unwrap_or(0.0) > 0.5),
                        };
                        ctx.actions.push(Action::Drive {
                            pin: s(a, "pin"),
                            level,
                            delay,
                        });
                    }
                }
            }
            Err(e) => {
                if let Ok(mut slot) = self.errors.lock() {
                    *slot = Some(e);
                }
            }
        }
    }
}

impl Behavior for JsBehavior {
    fn on_start(&self, ctx: &mut EventCtx<'_>) {
        self.run("start", ctx, 0, None)
    }
    fn on_inputs(&self, ctx: &mut EventCtx<'_>) {
        self.run("inputs", ctx, 0, None)
    }
    fn on_timer(&self, ctx: &mut EventCtx<'_>, tag: u32) {
        self.run("timer", ctx, tag, None)
    }
    fn on_control(&self, ctx: &mut EventCtx<'_>, control: &ControlEvent) {
        self.run("control", ctx, 0, Some(control))
    }
    fn diagnostics(&self) -> Option<String> {
        self.errors
            .lock()
            .ok()
            .and_then(|e| e.as_ref().map(|e| e.to_string()))
    }
}

/// The reference components shipped as JavaScript (spec 6.2).
pub const REFERENCE: [(&str, &str); 2] = [
    (
        "simple_diode.js",
        include_str!("../components/simple_diode.js"),
    ),
    ("dff.js", include_str!("../components/dff.js")),
];

/// Load every `.js` file in a folder. Returns definitions and per-file errors.
pub fn load_folder(dir: &std::path::Path) -> (Vec<ComponentDef>, Vec<ScriptError>) {
    let mut defs = Vec::new();
    let mut errors = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (defs, errors);
    };
    let mut paths: Vec<_> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "js"))
        .collect();
    paths.sort();
    for p in paths {
        let file = p.display().to_string();
        match std::fs::read_to_string(&p) {
            Ok(src) => match load_component(&src, &file) {
                Ok(d) => defs.push(d),
                Err(e) => errors.push(e),
            },
            Err(e) => errors.push(ScriptError {
                file,
                line: 0,
                column: 0,
                message: e.to_string(),
            }),
        }
    }
    (defs, errors)
}

/// Last-modified stamp of every `.js` file in a folder (for hot reload).
pub fn folder_stamp(dir: &std::path::Path) -> BTreeMap<String, std::time::SystemTime> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "js"))
        .filter_map(|e| {
            Some((
                e.path().display().to_string(),
                e.metadata().ok()?.modified().ok()?,
            ))
        })
        .collect()
}
