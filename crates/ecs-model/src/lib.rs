//! Component definition contract (spec section 5).
//!
//! A component is data, not solver code: it declares pins, parameters and
//! branch equations as [`Expr`]s and the engine derives stamps, derivatives and
//! integration. Built-in Rust components, JavaScript components, SPICE imports
//! and subcircuits all produce the same [`ComponentDef`].

use ecs_expr::Expr;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

pub use ecs_expr;

/// Stable component identifier such as `core.resistor`.
pub type ComponentId = String;
/// Pin name, internal node name, or `"0"` for ground.
pub type NodeRef = String;

/// Descriptive metadata.
#[derive(Debug, Clone, Default)]
pub struct Meta {
    pub name: String,
    pub category: String,
    pub version: u32,
    pub author: String,
    pub description: String,
    /// Reference designator prefix: `R`, `C`, `Q`, `U`...
    pub prefix: String,
}

/// Electrical nature of a pin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinKind {
    Analog,
    /// Digital input of a logic family (phase P5).
    DigitalIn(String),
    /// Digital output of a logic family (phase P5).
    DigitalOut(String),
}

/// A pin of the component.
#[derive(Debug, Clone)]
pub struct PinDef {
    pub name: String,
    pub kind: PinKind,
}

/// Parameter value type.
#[derive(Debug, Clone, PartialEq)]
pub enum ParamKind {
    Real,
    Integer,
    Enum(Vec<String>),
    Bool,
}

/// A parameter with its unit and default.
#[derive(Debug, Clone)]
pub struct ParamDef {
    pub name: String,
    pub unit: String,
    pub default: f64,
    pub description: String,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub kind: ParamKind,
    /// Changeable while the simulation runs, without recompiling.
    pub live: bool,
}

impl ParamDef {
    /// A real parameter.
    pub fn real(name: &str, default: f64, unit: &str, description: &str) -> Self {
        Self {
            name: name.into(),
            unit: unit.into(),
            default,
            description: description.into(),
            min: None,
            max: None,
            kind: ParamKind::Real,
            live: false,
        }
    }
    /// Mark the parameter live.
    pub fn live(mut self) -> Self {
        self.live = true;
        self
    }
    /// Set the allowed range.
    pub fn range(mut self, min: f64, max: f64) -> Self {
        self.min = Some(min);
        self.max = Some(max);
        self
    }
}

/// A branch between two nodes.
#[derive(Debug, Clone)]
pub enum BranchDef {
    /// Current `i` (+ `dq/dt`) flowing from `a` to `b` through the component.
    Current {
        name: Option<String>,
        a: NodeRef,
        b: NodeRef,
        i: Expr,
        q: Option<Expr>,
    },
    /// `V(a) - V(b) = v + d(flux)/dt`. Adds a branch-current unknown `I(name)`,
    /// flowing from `a` to `b` through the component.
    Voltage {
        name: String,
        a: NodeRef,
        b: NodeRef,
        v: Expr,
        flux: Option<Expr>,
    },
}

impl BranchDef {
    /// The two nodes of the branch.
    pub fn nodes(&self) -> (&str, &str) {
        match self {
            BranchDef::Current { a, b, .. } | BranchDef::Voltage { a, b, .. } => (a, b),
        }
    }
    /// The branch name, if any.
    pub fn name(&self) -> Option<&str> {
        match self {
            BranchDef::Current { name, .. } => name.as_deref(),
            BranchDef::Voltage { name, .. } => Some(name),
        }
    }
}

/// A discrete state variable changed only by events and controls.
#[derive(Debug, Clone)]
pub struct StateDef {
    pub name: String,
    pub initial: f64,
}

/// Newton limiting on a controlling voltage.
#[derive(Debug, Clone)]
pub enum LimitDef {
    /// SPICE `pnjlim` on `V(a, b)` for a junction with emission voltage
    /// `vte = N * vt` and saturation current `is`.
    Pn {
        a: NodeRef,
        b: NodeRef,
        vte: Expr,
        is: Expr,
    },
}

/// A named quantity shown in the inspector and plottable.
#[derive(Debug, Clone)]
pub struct ProbeDef {
    pub name: String,
    pub expr: Expr,
    pub unit: String,
}

/// A user control on the canvas or in the inspector.
#[derive(Debug, Clone, PartialEq)]
pub enum ControlDef {
    /// Click flips a state between 0 and 1 (switch).
    Toggle { state: String, label: String },
    /// State is 1 while the pointer is held (push button).
    Momentary { state: String, label: String },
    /// A live parameter shown as a slider (potentiometer wiper).
    Slider { param: String, label: String },
}

/// A control event delivered to the engine.
#[derive(Debug, Clone, PartialEq)]
pub enum ControlEvent {
    SetState { state: String, value: f64 },
    SetParam { param: String, value: f64 },
}

/// A digital logic level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Level {
    Low,
    High,
    /// Unknown: between the input thresholds with no previous level.
    X,
}

impl Level {
    /// `true` → High, `false` → Low.
    pub fn from_bool(b: bool) -> Self {
        if b { Level::High } else { Level::Low }
    }
    /// High is true; Low and X are false.
    pub fn is_high(self) -> bool {
        self == Level::High
    }
    /// Logical not (X stays X).
    pub fn invert(self) -> Self {
        match self {
            Level::Low => Level::High,
            Level::High => Level::Low,
            Level::X => Level::X,
        }
    }
    /// 0, 1, or 0.5 for X.
    pub fn as_f64(self) -> f64 {
        match self {
            Level::Low => 0.0,
            Level::High => 1.0,
            Level::X => 0.5,
        }
    }
}

/// Electrical behaviour of digital pins (spec 4.7).
#[derive(Debug, Clone, PartialEq)]
pub struct LogicFamily {
    pub name: String,
    /// Input low threshold: below is Low.
    pub vil: f64,
    /// Input high threshold: above is High.
    pub vih: f64,
    pub vol: f64,
    pub voh: f64,
    /// Output resistance of the Thevenin driver.
    pub rout: f64,
    /// Input resistance to ground.
    pub rin: f64,
    /// Output transition time (rise and fall).
    pub tr: f64,
    /// When set, `voh`/`vol` follow these pins' voltages at each transition
    /// (positive rail pin, negative rail pin), as in a 555's output stage.
    pub rails: Option<(String, String)>,
}

impl LogicFamily {
    /// 5 V CMOS.
    pub fn cmos5() -> Self {
        Self {
            name: "cmos5".into(),
            vil: 1.5,
            vih: 3.5,
            vol: 0.0,
            voh: 5.0,
            rout: 50.0,
            rin: 1e9,
            tr: 10e-9,
            rails: None,
        }
    }
    /// 3.3 V CMOS.
    pub fn cmos33() -> Self {
        Self {
            name: "cmos33".into(),
            vil: 0.8,
            vih: 2.0,
            vol: 0.0,
            voh: 3.3,
            rout: 50.0,
            rin: 1e9,
            tr: 5e-9,
            rails: None,
        }
    }
    /// TTL (74LS-like levels).
    pub fn ttl() -> Self {
        Self {
            name: "ttl".into(),
            vil: 0.8,
            vih: 2.0,
            vol: 0.2,
            voh: 3.4,
            rout: 80.0,
            rin: 50e3,
            tr: 15e-9,
            rails: None,
        }
    }
    /// By name: `cmos5` (default), `cmos33`, `ttl`, or `rail:<P>:<N>`.
    pub fn by_name(name: &str) -> Self {
        if let Some(rest) = name.strip_prefix("rail:")
            && let Some((p, n)) = rest.split_once(':')
        {
            return Self {
                name: name.into(),
                rails: Some((p.into(), n.into())),
                rout: 10.0,
                ..Self::cmos5()
            };
        }
        match name {
            "cmos33" | "3v3" => Self::cmos33(),
            "ttl" => Self::ttl(),
            _ => Self::cmos5(),
        }
    }
    /// Family names selectable by the `family` parameter, in index order.
    pub const NAMES: [&'static str; 3] = ["cmos5", "cmos33", "ttl"];
}

/// What a behavior asks the engine to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Drive an output pin to a level after a delay (seconds).
    Drive {
        pin: String,
        level: Level,
        delay: f64,
    },
    /// Call `on_timer` with `tag` after a delay.
    Timer { delay: f64, tag: u32 },
}

/// Context a behavior hook runs in (spec 5.3).
pub struct EventCtx<'a> {
    pub time: f64,
    /// State variables declared by the definition (written back to the
    /// equations) and any private keys the behavior keeps.
    pub state: &'a mut BTreeMap<String, f64>,
    pub params: &'a BTreeMap<String, f64>,
    /// Current input levels by pin (and watch) name.
    pub levels: &'a BTreeMap<String, Level>,
    /// Levels before the change that triggered this call.
    pub previous: &'a BTreeMap<String, Level>,
    /// Pin voltages relative to ground.
    pub voltages: &'a BTreeMap<String, f64>,
    /// Requests collected during the call.
    pub actions: Vec<Action>,
}

impl EventCtx<'_> {
    /// Level of an input pin or watch.
    pub fn level(&self, pin: &str) -> Level {
        self.levels.get(pin).copied().unwrap_or(Level::X)
    }
    /// Low (or X) → High since the last call.
    pub fn rising(&self, pin: &str) -> bool {
        self.level(pin) == Level::High
            && self.previous.get(pin).copied().unwrap_or(Level::X) != Level::High
    }
    /// High (or X) → Low since the last call.
    pub fn falling(&self, pin: &str) -> bool {
        self.level(pin) == Level::Low
            && self.previous.get(pin).copied().unwrap_or(Level::X) != Level::Low
    }
    /// Voltage of a pin.
    pub fn voltage(&self, pin: &str) -> f64 {
        self.voltages.get(pin).copied().unwrap_or(0.0)
    }
    /// A parameter value.
    pub fn param(&self, name: &str) -> f64 {
        self.params.get(name).copied().unwrap_or(0.0)
    }
    /// A state value (0 if unset).
    pub fn get(&self, name: &str) -> f64 {
        self.state.get(name).copied().unwrap_or(0.0)
    }
    /// Set a state value.
    pub fn set(&mut self, name: &str, value: f64) {
        self.state.insert(name.to_string(), value);
    }
    /// Drive an output after `delay` seconds.
    pub fn drive(&mut self, pin: &str, level: Level, delay: f64) {
        self.actions.push(Action::Drive {
            pin: pin.to_string(),
            level,
            delay: delay.max(0.0),
        });
    }
    /// Schedule `on_timer(tag)` after `delay` seconds.
    pub fn timer(&mut self, delay: f64, tag: u32) {
        self.actions.push(Action::Timer {
            delay: delay.max(0.0),
            tag,
        });
    }
}

/// Event-driven behavior: digital inputs, timers and user controls.
pub trait Behavior: Send + Sync {
    /// Called once after the operating point, before time starts. Defaults to
    /// [`Behavior::on_inputs`] so combinational parts settle.
    fn on_start(&self, ctx: &mut EventCtx<'_>) {
        self.on_inputs(ctx);
    }
    /// Digital inputs (or watches) changed.
    fn on_inputs(&self, _ctx: &mut EventCtx<'_>) {}
    /// A timer scheduled by the behavior fired.
    fn on_timer(&self, _ctx: &mut EventCtx<'_>, _tag: u32) {}
    /// A user control was operated.
    fn on_control(&self, ctx: &mut EventCtx<'_>, control: &ControlEvent) {
        if let ControlEvent::SetState { state, value } = control {
            ctx.state.insert(state.clone(), *value);
        }
    }
    /// The last error a hook raised (scripts), for the problems panel.
    fn diagnostics(&self) -> Option<String> {
        None
    }
}

/// One element of a subcircuit body.
#[derive(Debug, Clone)]
pub struct SubElement {
    /// Element name inside the subcircuit (`R1`, `Q3`, `X2`).
    pub name: String,
    pub def: Arc<ComponentDef>,
    /// Node per definition pin: a port name, `0`, or an internal node.
    pub nodes: Vec<String>,
    pub params: BTreeMap<String, f64>,
    /// Parameters given as expressions of the enclosing subcircuit's
    /// parameters (`{gain * 2}` in a nested schematic), evaluated when the
    /// subcircuit is flattened with the instance's values.
    pub exprs: BTreeMap<String, Expr>,
}

/// A subcircuit: ports (the definition's pins, in order) and elements.
#[derive(Debug, Clone, Default)]
pub struct Subckt {
    pub elements: Vec<SubElement>,
}

/// How the definition is realised.
#[derive(Debug, Clone)]
pub enum DefKind {
    Primitive,
    /// Ties its single pin to node 0.
    Ground,
    /// Flattened into its elements at compile time (spec 5.4).
    Subcircuit(Arc<Subckt>),
}

impl PartialEq for DefKind {
    fn eq(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (DefKind::Primitive, DefKind::Primitive)
                | (DefKind::Ground, DefKind::Ground)
                | (DefKind::Subcircuit(_), DefKind::Subcircuit(_))
        )
    }
}

/// A complete component definition. Immutable after loading.
#[derive(Clone)]
pub struct ComponentDef {
    pub id: ComponentId,
    pub meta: Meta,
    pub pins: Vec<PinDef>,
    pub params: Vec<ParamDef>,
    pub internal_nodes: Vec<NodeRef>,
    /// Named subexpressions (`c.let`), usable by later expressions.
    pub lets: Vec<(String, Expr)>,
    pub branches: Vec<BranchDef>,
    pub states: Vec<StateDef>,
    pub limits: Vec<LimitDef>,
    pub probes: Vec<ProbeDef>,
    pub controls: Vec<ControlDef>,
    /// Boolean conditions on voltages that act as extra digital inputs for the
    /// behavior (a 555's threshold and trigger comparators).
    pub watches: Vec<(String, Expr)>,
    pub behavior: Option<Arc<dyn Behavior>>,
    /// Symbol id in the schematic symbol library.
    pub symbol: String,
    pub kind: DefKind,
}

impl fmt::Debug for ComponentDef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ComponentDef")
            .field("id", &self.id)
            .field(
                "pins",
                &self.pins.iter().map(|p| &p.name).collect::<Vec<_>>(),
            )
            .field("branches", &self.branches.len())
            .finish()
    }
}

impl ComponentDef {
    /// An empty primitive definition to fill in.
    pub fn new(id: &str, name: &str, category: &str, prefix: &str, symbol: &str) -> Self {
        Self {
            id: id.into(),
            meta: Meta {
                name: name.into(),
                category: category.into(),
                version: 1,
                author: "Electrocraft".into(),
                description: String::new(),
                prefix: prefix.into(),
            },
            pins: Vec::new(),
            params: Vec::new(),
            internal_nodes: Vec::new(),
            lets: Vec::new(),
            branches: Vec::new(),
            states: Vec::new(),
            limits: Vec::new(),
            probes: Vec::new(),
            controls: Vec::new(),
            watches: Vec::new(),
            behavior: None,
            symbol: symbol.into(),
            kind: DefKind::Primitive,
        }
    }

    /// Index of a parameter by name.
    pub fn param_index(&self, name: &str) -> Option<usize> {
        self.params.iter().position(|p| p.name == name)
    }

    /// Default parameter values by name.
    pub fn defaults(&self) -> BTreeMap<String, f64> {
        self.params
            .iter()
            .map(|p| (p.name.clone(), p.default))
            .collect()
    }

    /// The logic family of a digital pin: the pin's own, else the `family`
    /// parameter (index into [`LogicFamily::NAMES`]), else 5 V CMOS.
    pub fn family(&self, pin: &PinDef, params: &BTreeMap<String, f64>) -> LogicFamily {
        let own = match &pin.kind {
            PinKind::DigitalIn(f) | PinKind::DigitalOut(f) => f.as_str(),
            PinKind::Analog => "",
        };
        if !own.is_empty() {
            return LogicFamily::by_name(own);
        }
        let k = params.get("family").copied().unwrap_or(0.0) as usize;
        LogicFamily::by_name(LogicFamily::NAMES.get(k).copied().unwrap_or("cmos5"))
    }

    /// Whether this definition stamps nothing nonlinear (cheap structural hint;
    /// the authoritative answer comes from the compiled tapes).
    pub fn has_limits(&self) -> bool {
        !self.limits.is_empty()
    }
}

/// A real part number mapped to a definition and the parameter values it
/// sets (spec 7): `2N2222A` → `core.npn` with its model, `74HC00` → the chip.
#[derive(Debug, Clone, PartialEq)]
pub struct KnownPart {
    pub number: String,
    /// Definition id it uses.
    pub def: String,
    /// Parameter values it sets.
    pub params: Vec<(String, f64)>,
    pub note: String,
    /// The number new parts of this definition get.
    pub default: bool,
}

/// A set of definitions by id.
#[derive(Debug, Clone, Default)]
pub struct Library {
    defs: BTreeMap<ComponentId, Arc<ComponentDef>>,
}

impl Library {
    /// An empty library.
    pub fn new() -> Self {
        Self::default()
    }
    /// Add or replace a definition.
    pub fn insert(&mut self, def: ComponentDef) {
        self.defs.insert(def.id.clone(), Arc::new(def));
    }
    /// Add or replace a shared definition.
    pub fn insert_arc(&mut self, def: Arc<ComponentDef>) {
        self.defs.insert(def.id.clone(), def);
    }
    /// Remove one definition.
    pub fn remove(&mut self, id: &str) {
        self.defs.remove(id);
    }
    /// Remove definitions whose id starts with `prefix` (package reload).
    pub fn remove_prefix(&mut self, prefix: &str) {
        self.defs.retain(|k, _| !k.starts_with(prefix));
    }
    /// Look up a definition.
    pub fn get(&self, id: &str) -> Option<&Arc<ComponentDef>> {
        self.defs.get(id)
    }
    /// All definitions in id order.
    pub fn iter(&self) -> impl Iterator<Item = &Arc<ComponentDef>> {
        self.defs.values()
    }
    /// Number of definitions.
    pub fn len(&self) -> usize {
        self.defs.len()
    }
    /// Whether the library is empty.
    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }
}
