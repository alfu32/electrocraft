//! Netlist compiler and the MNA engine (spec 4.1 to 4.5).

use crate::linear::{DenseLu, LinearSolver};
use ecs_expr::{Leaf, Resolved, Sym, Tape, TapeBuilder, Workspace};
use ecs_model::{
    Action, Behavior, BranchDef, ComponentDef, ControlEvent, DefKind, EventCtx, Level, LimitDef,
    LogicFamily, PinKind,
};
use std::collections::BinaryHeap;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use thiserror::Error;

/// Boltzmann constant over electron charge, V/K.
const K_OVER_Q: f64 = 8.617_333_262e-5;

/// A flat netlist: named nodes and component instances.
#[derive(Debug, Clone, Default)]
pub struct Netlist {
    /// Node names. Index 0 is ground and is always named `0`.
    pub nodes: Vec<String>,
    pub instances: Vec<Instance>,
}

impl Netlist {
    /// A netlist containing only the ground node.
    pub fn new() -> Self {
        Self {
            nodes: vec!["0".into()],
            instances: Vec::new(),
        }
    }
    /// Index of a node, created on first use. `0`, `gnd` and `GND` are ground.
    pub fn node(&mut self, name: &str) -> usize {
        if matches!(name, "0" | "gnd" | "GND") {
            return 0;
        }
        if let Some(i) = self.nodes.iter().position(|n| n == name) {
            return i;
        }
        self.nodes.push(name.to_string());
        self.nodes.len() - 1
    }
    /// Add an instance with pins connected to the named nodes, in pin order.
    pub fn add(
        &mut self,
        name: &str,
        def: Arc<ComponentDef>,
        nodes: &[&str],
        params: &[(&str, f64)],
    ) {
        let pins = nodes.iter().map(|n| self.node(n)).collect();
        self.instances.push(Instance {
            name: name.into(),
            def,
            pins,
            params: params.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        });
    }
}

/// One component instance.
#[derive(Debug, Clone)]
pub struct Instance {
    /// Reference designator such as `R1`.
    pub name: String,
    pub def: Arc<ComponentDef>,
    /// Node index for each pin, in the definition's pin order.
    pub pins: Vec<usize>,
    /// Parameter overrides; missing ones take the definition default.
    pub params: BTreeMap<String, f64>,
}

/// Compile-time or solve-time failure.
#[derive(Debug, Clone, Error, PartialEq)]
pub enum SimError {
    #[error("{instance}: {message}")]
    Component { instance: String, message: String },
    #[error("voltage sources or inductors form a loop with no resistance: {0}")]
    VoltageLoop(String),
    #[error("the circuit matrix is singular; check for floating parts")]
    Singular,
    #[error("no convergence at t = {time:e} s near {nodes}")]
    NoConvergence { time: f64, nodes: String },
    #[error("{instance} produced a non-finite value")]
    NotFinite { instance: String },
}

/// Integration method (spec 4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    BackwardEuler,
    Trapezoidal,
}

/// Solver tolerances and limits (spec 4.8).
#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub reltol: f64,
    pub vntol: f64,
    pub abstol: f64,
    pub gmin: f64,
    pub max_iter: usize,
    /// Temperature in °C.
    pub temp: f64,
    /// θ of the θ-method used for "trapezoidal" steps: 0.5 is exact
    /// trapezoidal; larger values damp the ringing trapezoidal shows on
    /// stiff edges (a fast mode is multiplied by -(1-θ)/θ each step).
    pub theta: f64,
}

impl Options {
    /// Analysis-mode defaults.
    pub fn analysis() -> Self {
        Self {
            reltol: 1e-3,
            vntol: 1e-6,
            abstol: 1e-12,
            gmin: 1e-12,
            max_iter: 100,
            temp: 27.0,
            theta: 0.5,
        }
    }
    /// Interactive-mode defaults.
    pub fn interactive() -> Self {
        Self {
            reltol: 1e-2,
            max_iter: 20,
            theta: 0.6,
            ..Self::analysis()
        }
    }
}

impl Options {
    /// History weight `(1-θ)/θ` of the θ-method (1 for trapezoidal).
    pub fn damping(&self) -> f64 {
        (1.0 - self.theta) / self.theta
    }
}

impl Default for Options {
    fn default() -> Self {
        Self::analysis()
    }
}

/// Which DC operating point strategy succeeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpMethod {
    Newton,
    GminStepping,
    SourceStepping,
}

#[derive(Debug, Clone)]
enum CBranch {
    Current {
        a: Option<usize>,
        b: Option<usize>,
        out_i: usize,
        out_q: Option<usize>,
        /// Independent source: output does not depend on unknowns.
        source: bool,
        q_prev: f64,
        iq_prev: f64,
        /// Total current a→b at the last accepted point.
        current: f64,
    },
    Voltage {
        a: Option<usize>,
        b: Option<usize>,
        j: usize,
        out_v: usize,
        out_f: Option<usize>,
        source: bool,
        f_prev: f64,
        vf_prev: f64,
    },
}

#[derive(Debug, Clone)]
struct CLimit {
    a: Option<usize>,
    b: Option<usize>,
    vte: f64,
    vcrit: f64,
    last: Option<f64>,
}

/// A compiled instance.
#[derive(Debug, Clone)]
struct CInst {
    name: String,
    /// Global unknown index for each local unknown.
    map: Vec<usize>,
    tape: Tape,
    probe_tape: Tape,
    watch_tape: Tape,
    live: Vec<f64>,
    live_params: HashMap<String, usize>,
    live_states: HashMap<String, usize>,
    branches: Vec<CBranch>,
    limits: Vec<CLimit>,
    xl: Vec<f64>,
    trouble: bool,
}

/// A digital pin of an instance.
#[derive(Debug, Clone)]
struct DPin {
    name: String,
    /// Local unknown of the pin's node (None = ground).
    local: Option<usize>,
    fam: LogicFamily,
}

/// A digital output: its Thevenin driver ramps from `v0` to `v1` starting at `t0`.
#[derive(Debug, Clone)]
struct DOut {
    pin: DPin,
    slot_v0: usize,
    slot_v1: usize,
    slot_t0: usize,
    level: Level,
    /// Local unknowns of the rail pins when the family follows rails.
    rails: Option<(Option<usize>, Option<usize>)>,
}

/// Event-layer state of an instance with digital pins, watches or a behavior.
#[derive(Clone)]
struct DInst {
    k: usize,
    behavior: Option<Arc<dyn Behavior>>,
    inputs: Vec<DPin>,
    outputs: Vec<DOut>,
    watch_names: Vec<String>,
    /// Every pin with its local unknown, for `ctx.voltage`.
    pins: Vec<(String, Option<usize>)>,
    levels: BTreeMap<String, Level>,
    state: BTreeMap<String, f64>,
    params: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EvKind {
    Drive { out: usize, level: Level },
    Timer(u32),
}

/// A scheduled event; ordered by time then insertion (min-heap via Reverse).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Event {
    time: u64,
    seq: u64,
    d: usize,
    kind: EvKind,
}

impl Ord for Event {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        (o.time, o.seq).cmp(&(self.time, self.seq))
    }
}
impl PartialOrd for Event {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}

/// Replace subcircuit instances by their elements, recursively. Element
/// names and internal nodes are namespaced with the instance name.
pub fn flatten(netlist: &Netlist) -> Result<Netlist, SimError> {
    fn expand(
        out: &mut Netlist,
        prefix: &str,
        def: &ComponentDef,
        outer: &[usize],
        params: &BTreeMap<String, f64>,
        depth: usize,
    ) -> Result<(), SimError> {
        let DefKind::Subcircuit(body) = &def.kind else {
            unreachable!()
        };
        if depth > 32 {
            return Err(SimError::Component {
                instance: prefix.into(),
                message: "subcircuits nest too deeply".into(),
            });
        }
        // The subcircuit's parameter scope: its defaults overlaid by the instance.
        let mut scope = def.defaults();
        scope.extend(params.iter().map(|(k, v)| (k.clone(), *v)));
        for el in &body.elements {
            let mut el_params = el.params.clone();
            for (k, e) in &el.exprs {
                let v = ecs_expr::eval_with(e, &|n| scope.get(n).copied()).map_err(|err| {
                    SimError::Component {
                        instance: format!("{prefix}.{}", el.name),
                        message: format!("parameter {k}: {err}"),
                    }
                })?;
                el_params.insert(k.clone(), v);
            }
            let mut pins = Vec::new();
            for n in &el.nodes {
                let idx = if let Some(k) = def.pins.iter().position(|p| p.name == *n) {
                    outer[k]
                } else if matches!(n.as_str(), "0" | "gnd" | "GND") {
                    0
                } else {
                    out.node(&format!("{prefix}.{n}"))
                };
                pins.push(idx);
            }
            let name = format!("{prefix}.{}", el.name);
            if matches!(el.def.kind, DefKind::Subcircuit(_)) {
                expand(out, &name, &el.def, &pins, &el_params, depth + 1)?;
            } else {
                out.instances.push(Instance {
                    name,
                    def: el.def.clone(),
                    pins,
                    params: el_params,
                });
            }
        }
        Ok(())
    }
    if !netlist
        .instances
        .iter()
        .any(|i| matches!(i.def.kind, DefKind::Subcircuit(_)))
    {
        return Ok(netlist.clone());
    }
    let mut out = Netlist {
        nodes: netlist.nodes.clone(),
        instances: Vec::new(),
    };
    for inst in &netlist.instances {
        if matches!(inst.def.kind, DefKind::Subcircuit(_)) {
            if inst.pins.len() != inst.def.pins.len() {
                return Err(SimError::Component {
                    instance: inst.name.clone(),
                    message: format!(
                        "expects {} pins, got {}",
                        inst.def.pins.len(),
                        inst.pins.len()
                    ),
                });
            }
            expand(&mut out, &inst.name, &inst.def, &inst.pins, &inst.params, 0)?;
        } else {
            out.instances.push(inst.clone());
        }
    }
    Ok(out)
}

const LIVE_T: usize = 0;
const LIVE_TEMP: usize = 1;
const LIVE_VT: usize = 2;

/// A compiled circuit ready to solve.
pub struct Circuit {
    node_names: Vec<String>,
    /// Compiled node index for each netlist node index.
    netlist_index: Vec<usize>,
    n_nodes: usize,
    n: usize,
    unknown_names: Vec<String>,
    insts: Vec<CInst>,
    x: Vec<f64>,
    solver: Box<dyn LinearSolver>,
    rhs: Vec<f64>,
    ws: Workspace,
    time: f64,
    pub options: Options,
    linear: bool,
    /// Warnings found at compile time (floating nodes...).
    pub warnings: Vec<String>,
    after_breakpoint: bool,
    pub last_iterations: usize,
    /// Per row: sum of |branch current| (KCL rows) or |voltage| (branch rows),
    /// the scale of the residual convergence test.
    mag: Vec<f64>,
    /// Successful interactive steps since the last flagged trouble.
    clean_steps: u64,
    digital: Vec<DInst>,
    events: BinaryHeap<Event>,
    seq: u64,
    /// While settling the start state, drives apply at once.
    settling: bool,
}

fn pnjlim(vnew: f64, vold: f64, vt: f64, vcrit: f64) -> (f64, bool) {
    if vnew > vcrit && (vnew - vold).abs() > 2.0 * vt {
        if vold > 0.0 {
            let arg = 1.0 + (vnew - vold) / vt;
            if arg > 0.0 {
                (vold + vt * arg.ln(), true)
            } else {
                (vcrit, true)
            }
        } else {
            (vt * (vnew / vt).ln(), true)
        }
    } else {
        (vnew, false)
    }
}

struct UnionFind(Vec<usize>);
impl UnionFind {
    fn new(n: usize) -> Self {
        Self((0..n).collect())
    }
    fn find(&mut self, i: usize) -> usize {
        let mut r = i;
        while self.0[r] != r {
            r = self.0[r];
        }
        let mut i = i;
        while self.0[i] != r {
            let next = self.0[i];
            self.0[i] = r;
            i = next;
        }
        r
    }
    fn union(&mut self, a: usize, b: usize) -> bool {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra == rb {
            return false;
        }
        self.0[ra] = rb;
        true
    }
}

impl Circuit {
    /// Compile a netlist (spec 5.4 checks included).
    pub fn compile(netlist: &Netlist) -> Result<Circuit, SimError> {
        Self::compile_with(netlist, Options::default())
    }

    /// Compile with explicit options.
    pub fn compile_with(source: &Netlist, options: Options) -> Result<Circuit, SimError> {
        let flat = flatten(source)?;
        let netlist = &flat;
        let mut node_names = netlist.nodes.clone();
        if node_names.is_empty() {
            node_names.push("0".into());
        }
        // Ground instances tie their pin's node to 0.
        let mut alias: Vec<usize> = (0..node_names.len()).collect();
        for inst in &netlist.instances {
            if inst.def.kind == DefKind::Ground
                && let Some(&p) = inst.pins.first()
            {
                alias[p] = 0;
            }
        }
        // Internal nodes.
        let mut internal: Vec<HashMap<String, usize>> = Vec::new();
        for inst in &netlist.instances {
            let mut m = HashMap::new();
            for n in &inst.def.internal_nodes {
                node_names.push(format!("{}.{}", inst.name, n));
                alias.push(node_names.len() - 1);
                m.insert(n.clone(), node_names.len() - 1);
            }
            internal.push(m);
        }
        // Renumber nodes: ground 0, then the rest compacted.
        let mut index = vec![usize::MAX; node_names.len()];
        let mut names = vec!["0".to_string()];
        index[0] = 0;
        for i in 1..node_names.len() {
            if alias[i] == 0 {
                index[i] = 0;
            } else {
                index[i] = names.len();
                names.push(node_names[i].clone());
            }
        }
        let n_nodes = names.len();
        let mut unknown_names: Vec<String> = names[1..].iter().map(|n| format!("V({n})")).collect();
        let mut next = n_nodes - 1;

        // Branch-current unknowns for every voltage branch, assigned up front so
        // an expression can read another element's current: I(Vx).
        let mut branch_unknowns: Vec<HashMap<String, usize>> = Vec::new();
        let mut first_vbranch: HashMap<String, usize> = HashMap::new();
        for inst in &netlist.instances {
            let mut m = HashMap::new();
            if inst.def.kind != DefKind::Ground {
                for b in &inst.def.branches {
                    if let BranchDef::Voltage { name, .. } = b {
                        m.insert(name.clone(), next);
                        first_vbranch.entry(inst.name.clone()).or_insert(next);
                        unknown_names.push(format!("I({}.{name})", inst.name));
                        next += 1;
                    }
                }
            }
            branch_unknowns.push(m);
        }
        let mut insts = Vec::new();
        let mut digital: Vec<DInst> = Vec::new();
        let mut warnings = Vec::new();
        let mut dc = UnionFind::new(n_nodes);
        let mut vloop = UnionFind::new(n_nodes);
        let vt = K_OVER_Q * (options.temp + 273.15);
        for (k, inst) in netlist.instances.iter().enumerate() {
            let def = &inst.def;
            if def.kind == DefKind::Ground {
                continue;
            }
            let err = |message: String| SimError::Component {
                instance: inst.name.clone(),
                message,
            };
            if inst.pins.len() != def.pins.len() {
                return Err(err(format!(
                    "expects {} pins, got {}",
                    def.pins.len(),
                    inst.pins.len()
                )));
            }
            // Node name -> global node index (0 = ground).
            let mut node_of: HashMap<&str, usize> = HashMap::new();
            for (p, &n) in def.pins.iter().zip(&inst.pins) {
                node_of.insert(p.name.as_str(), index[n]);
            }
            for (n, &g) in &internal[k] {
                node_of.insert(n.as_str(), index[g]);
            }
            node_of.insert("0", 0);
            let branch_unknown = &branch_unknowns[k];
            // Parameters first: digital families depend on them.
            let mut params = def.defaults();
            for (name, v) in &inst.params {
                if params.contains_key(name) {
                    params.insert(name.clone(), *v);
                }
            }
            // Engine-generated branches for digital pins (spec 4.7).
            let mut all_branches: Vec<BranchDef> = def.branches.clone();
            let mut dig_outs: Vec<(String, LogicFamily)> = Vec::new();
            let mut dig_ins: Vec<(String, LogicFamily)> = Vec::new();
            for pin in &def.pins {
                let fam = def.family(pin, &params);
                match &pin.kind {
                    PinKind::DigitalOut(_) => {
                        let p = &pin.name;
                        // Tri-state outputs scale the driver resistance by a state.
                        let oe = if def.states.iter().any(|st| st.name == format!("__oe_{p}")) {
                            format!(" * select(__oe_{p} > 0.5, 1, 1e8)")
                        } else {
                            String::new()
                        };
                        let src = format!(
                            "(V({p}) - (__v0_{p} + (__v1_{p} - __v0_{p}) * min(max((t - __t0_{p}) / {tr:e}, 0), 1))) / ({rout:e}{oe})",
                            tr = fam.tr,
                            rout = fam.rout
                        );
                        let i = ecs_expr::parse(&src)
                            .map_err(|e| err(format!("digital output {p}: {e}")))?;
                        all_branches.push(BranchDef::Current {
                            name: None,
                            a: p.clone(),
                            b: "0".into(),
                            i,
                            q: None,
                        });
                        dig_outs.push((p.clone(), fam));
                    }
                    PinKind::DigitalIn(_) => {
                        let p = &pin.name;
                        let i = ecs_expr::parse(&format!("V({p}) / {:e}", fam.rin))
                            .map_err(|e| err(e.to_string()))?;
                        all_branches.push(BranchDef::Current {
                            name: None,
                            a: p.clone(),
                            b: "0".into(),
                            i,
                            q: None,
                        });
                        dig_ins.push((p.clone(), fam));
                    }
                    PinKind::Analog => {}
                }
            }
            // Currents of other elements read with I(name): siblings first.
            let mut foreign: HashMap<String, usize> = HashMap::new();
            {
                let prefix = inst.name.rsplit_once('.').map(|(p, _)| p.to_string());
                let mut visit = |e: &ecs_expr::Expr| {
                    e.visit(&mut |leaf| {
                        if let ecs_expr::Expr::I(n) = leaf
                            && !branch_unknown.contains_key(n)
                        {
                            let sibling = prefix.as_ref().map(|p| format!("{p}.{n}"));
                            if let Some(&g) = sibling
                                .as_ref()
                                .and_then(|s| first_vbranch.get(s))
                                .or_else(|| first_vbranch.get(n))
                            {
                                foreign.insert(n.clone(), g);
                            }
                        }
                    })
                };
                for b in &all_branches {
                    match b {
                        BranchDef::Current { i, q, .. } => {
                            visit(i);
                            if let Some(q) = q {
                                visit(q);
                            }
                        }
                        BranchDef::Voltage { v, flux, .. } => {
                            visit(v);
                            if let Some(f) = flux {
                                visit(f);
                            }
                        }
                    }
                }
                for (_, e) in def.lets.iter().chain(def.watches.iter()) {
                    visit(e);
                }
                for p in &def.probes {
                    visit(&p.expr);
                }
            }
            // Local unknowns: referenced nodes (not ground), then branch currents.
            let mut map: Vec<usize> = Vec::new();
            let mut local_of_global: HashMap<usize, usize> = HashMap::new();
            let mut local = |global: usize, map: &mut Vec<usize>| -> usize {
                *local_of_global.entry(global).or_insert_with(|| {
                    map.push(global);
                    map.len() - 1
                })
            };
            let mut node_local: HashMap<String, Option<usize>> = HashMap::new();
            for (name, &g) in &node_of {
                let l = if g == 0 {
                    None
                } else {
                    Some(local(g - 1, &mut map))
                };
                node_local.insert(name.to_string(), l);
            }
            let mut branch_local: HashMap<String, usize> = HashMap::new();
            for (name, &g) in branch_unknown.iter().chain(foreign.iter()) {
                branch_local.insert(name.clone(), local(g, &mut map));
            }
            // Live slots: time, temperature, vt, live params, states, drivers.
            let mut live = vec![0.0, options.temp, vt];
            let mut live_params = HashMap::new();
            for p in def.params.iter().filter(|p| p.live) {
                live_params.insert(p.name.clone(), live.len());
                live.push(params[&p.name]);
            }
            let mut live_states = HashMap::new();
            for s in &def.states {
                live_states.insert(s.name.clone(), live.len());
                live.push(s.initial);
            }
            let mut drivers: HashMap<String, usize> = HashMap::new();
            for (p, fam) in &dig_outs {
                for (key, init) in [("__v0_", fam.vol), ("__v1_", fam.vol), ("__t0_", -1.0)] {
                    drivers.insert(format!("{key}{p}"), live.len());
                    live.push(init);
                }
            }
            let lets: HashMap<&str, &ecs_expr::Expr> =
                def.lets.iter().map(|(n, e)| (n.as_str(), e)).collect();
            let n_local = map.len();
            let mut resolve = |s: Sym<'_>| -> Result<Resolved, String> {
                let node = |n: &str| -> Result<Leaf, String> {
                    match node_local.get(n) {
                        Some(Some(l)) => Ok(Leaf::Unknown(*l)),
                        Some(None) => Ok(Leaf::Const(0.0)),
                        None => Err(format!("unknown node {n}")),
                    }
                };
                match s {
                    Sym::V(a, None) => Ok(Resolved::Leaf(node(a)?)),
                    Sym::V(a, Some(b)) => Ok(Resolved::Diff(node(a)?, node(b)?)),
                    Sym::I(b) => branch_local
                        .get(b)
                        .map(|&l| Resolved::Leaf(Leaf::Unknown(l)))
                        .ok_or_else(|| format!("unknown branch {b}")),
                    Sym::Name("t") => Ok(Resolved::Leaf(Leaf::Live(LIVE_T))),
                    Sym::Name("temp") => Ok(Resolved::Leaf(Leaf::Live(LIVE_TEMP))),
                    Sym::Name("vt") => Ok(Resolved::Leaf(Leaf::Live(LIVE_VT))),
                    Sym::Name(n) => {
                        if let Some(&slot) = drivers.get(n) {
                            Ok(Resolved::Leaf(Leaf::Live(slot)))
                        } else if let Some(&slot) = live_params.get(n) {
                            Ok(Resolved::Leaf(Leaf::Live(slot)))
                        } else if let Some(&slot) = live_states.get(n) {
                            Ok(Resolved::Leaf(Leaf::Live(slot)))
                        } else if let Some(v) = params.get(n) {
                            Ok(Resolved::Leaf(Leaf::Const(*v)))
                        } else if let Some(e) = lets.get(n) {
                            Ok(Resolved::Expr((*e).clone()))
                        } else {
                            Err(format!("unknown name {n}"))
                        }
                    }
                }
            };
            let mut tb = TapeBuilder::new(n_local);
            let mut outputs = Vec::new();
            let mut branches = Vec::new();
            for b in &all_branches {
                let (an, bn) = b.nodes();
                let a = *node_local
                    .get(an)
                    .ok_or_else(|| err(format!("unknown node {an}")))?;
                let bl = *node_local
                    .get(bn)
                    .ok_or_else(|| err(format!("unknown node {bn}")))?;
                let (ga, gb) = (node_of[an], node_of[bn]);
                match b {
                    BranchDef::Current { i, q, .. } => {
                        let id = tb.add_expr(i, &mut resolve).map_err(|e| err(e.0))?;
                        outputs.push(id);
                        let out_i = outputs.len() - 1;
                        let out_q = match q {
                            Some(q) => {
                                let id = tb.add_expr(q, &mut resolve).map_err(|e| err(e.0))?;
                                outputs.push(id);
                                Some(outputs.len() - 1)
                            }
                            None => None,
                        };
                        branches.push(CBranch::Current {
                            a,
                            b: bl,
                            out_i,
                            out_q,
                            source: false,
                            q_prev: 0.0,
                            iq_prev: 0.0,
                            current: 0.0,
                        });
                    }
                    BranchDef::Voltage { name, v, flux, .. } => {
                        let id = tb.add_expr(v, &mut resolve).map_err(|e| err(e.0))?;
                        outputs.push(id);
                        let out_v = outputs.len() - 1;
                        let out_f = match flux {
                            Some(f) => {
                                let id = tb.add_expr(f, &mut resolve).map_err(|e| err(e.0))?;
                                outputs.push(id);
                                Some(outputs.len() - 1)
                            }
                            None => None,
                        };
                        branches.push(CBranch::Voltage {
                            a,
                            b: bl,
                            j: branch_local[name],
                            out_v,
                            out_f,
                            source: false,
                            f_prev: 0.0,
                            vf_prev: 0.0,
                        });
                        // Voltage branches (sources, inductors, ammeters) must not form a loop.
                        if !vloop.union(ga, gb) {
                            return Err(SimError::VoltageLoop(inst.name.clone()));
                        }
                        dc.union(ga, gb);
                    }
                }
            }
            let tape = tb.finish(outputs);
            // Mark independent sources and DC paths.
            for (bi, b) in branches.iter_mut().enumerate() {
                let (an, bn) = all_branches[bi].nodes();
                match b {
                    CBranch::Current {
                        out_i,
                        out_q,
                        source,
                        ..
                    } => {
                        *source = tape.output_degree(*out_i) == 0 && out_q.is_none();
                        if tape.output_degree(*out_i) > 0 {
                            dc.union(node_of[an], node_of[bn]);
                        }
                    }
                    CBranch::Voltage {
                        out_v,
                        out_f,
                        source,
                        ..
                    } => {
                        *source = tape.output_degree(*out_v) == 0 && out_f.is_none();
                    }
                }
            }
            // Probes.
            let mut pb = TapeBuilder::new(n_local);
            let mut pouts = Vec::new();
            for p in &def.probes {
                pouts.push(pb.add_expr(&p.expr, &mut resolve).map_err(|e| err(e.0))?);
            }
            let probe_tape = pb.finish(pouts);
            let mut wb = TapeBuilder::new(n_local);
            let mut wouts = Vec::new();
            for (_, w) in &def.watches {
                wouts.push(wb.add_expr(w, &mut resolve).map_err(|e| err(e.0))?);
            }
            let watch_tape = wb.finish(wouts);
            // Limits.
            let mut limits = Vec::new();
            for l in &def.limits {
                let LimitDef::Pn { a, b, vte, is } = l;
                let lookup = |n: &str| match n {
                    "vt" => Some(vt),
                    "temp" => Some(options.temp),
                    _ => params.get(n).copied(),
                };
                let vte = ecs_expr::eval_with(vte, &lookup).map_err(|e| err(e.0))?;
                let is = ecs_expr::eval_with(is, &lookup).map_err(|e| err(e.0))?;
                let vcrit = vte * (vte / (std::f64::consts::SQRT_2 * is)).ln();
                limits.push(CLimit {
                    a: node_local[a.as_str()],
                    b: node_local[b.as_str()],
                    vte,
                    vcrit,
                    last: None,
                });
            }
            if def.behavior.is_some()
                || !dig_outs.is_empty()
                || !dig_ins.is_empty()
                || !def.watches.is_empty()
            {
                let dpin = |(p, fam): &(String, LogicFamily)| DPin {
                    name: p.clone(),
                    local: node_local.get(p).copied().flatten(),
                    fam: fam.clone(),
                };
                let outputs = dig_outs
                    .iter()
                    .map(|o| DOut {
                        pin: dpin(o),
                        slot_v0: drivers[&format!("__v0_{}", o.0)],
                        slot_v1: drivers[&format!("__v1_{}", o.0)],
                        slot_t0: drivers[&format!("__t0_{}", o.0)],
                        level: Level::Low,
                        rails: o.1.rails.as_ref().map(|(pp, nn)| {
                            (
                                node_local.get(pp).copied().flatten(),
                                node_local.get(nn).copied().flatten(),
                            )
                        }),
                    })
                    .collect();
                let state: BTreeMap<String, f64> = def
                    .states
                    .iter()
                    .map(|s| (s.name.clone(), s.initial))
                    .collect();
                digital.push(DInst {
                    k: insts.len(),
                    behavior: def.behavior.clone(),
                    inputs: dig_ins.iter().map(dpin).collect(),
                    outputs,
                    watch_names: def.watches.iter().map(|w| w.0.clone()).collect(),
                    pins: def
                        .pins
                        .iter()
                        .map(|p| (p.name.clone(), node_local.get(&p.name).copied().flatten()))
                        .collect(),
                    levels: BTreeMap::new(),
                    state,
                    params: params.clone(),
                });
            }
            insts.push(CInst {
                name: inst.name.clone(),
                xl: vec![0.0; map.len()],
                map,
                tape,
                probe_tape,
                watch_tape,
                live,
                live_params,
                live_states,
                branches,
                limits,
                trouble: false,
            });
        }
        for (i, name) in names.iter().enumerate().skip(1) {
            if dc.find(i) != dc.find(0) {
                warnings.push(format!("node {name} has no DC path to ground"));
            }
        }
        let n = next;
        let linear = insts
            .iter()
            .all(|c| c.tape.is_linear() && c.limits.is_empty());
        Ok(Circuit {
            netlist_index: index[..source.nodes.len().max(1)].to_vec(),
            node_names: names,
            n_nodes,
            n,
            unknown_names,
            insts,
            x: vec![0.0; n],
            solver: Box::new(DenseLu::new()),
            rhs: vec![0.0; n],
            ws: Workspace::default(),
            time: 0.0,
            options,
            linear,
            warnings,
            after_breakpoint: true,
            last_iterations: 0,
            mag: vec![0.0; n],
            clean_steps: 0,
            digital,
            events: BinaryHeap::new(),
            seq: 0,
            settling: false,
        })
    }

    /// Node names; index 0 is ground.
    pub fn node_names(&self) -> &[String] {
        &self.node_names
    }
    /// Number of unknowns.
    pub fn unknowns(&self) -> usize {
        self.n
    }
    /// Name of each unknown, `V(node)` or `I(inst.branch)`.
    pub fn unknown_names(&self) -> &[String] {
        &self.unknown_names
    }
    /// The solution vector.
    pub fn solution(&self) -> &[f64] {
        &self.x
    }
    /// Simulated time.
    pub fn time(&self) -> f64 {
        self.time
    }
    /// True when no component is nonlinear: Newton converges in one iteration.
    pub fn is_linear(&self) -> bool {
        self.linear
    }
    /// Number of numeric factorizations so far.
    pub fn factorizations(&self) -> u64 {
        self.solver.factorizations()
    }
    /// Voltage of node `i` (0 is ground).
    pub fn node_voltage(&self, i: usize) -> f64 {
        if i == 0 || i >= self.n_nodes {
            0.0
        } else {
            self.x[i - 1]
        }
    }
    /// Voltage of a node given its index in the source [`Netlist`].
    pub fn voltage_of_netlist_node(&self, i: usize) -> f64 {
        self.netlist_index
            .get(i)
            .map_or(0.0, |&k| self.node_voltage(k))
    }
    /// Voltage of a node by name.
    pub fn voltage(&self, name: &str) -> Option<f64> {
        if matches!(name, "0" | "gnd" | "GND") {
            return Some(0.0);
        }
        self.node_names
            .iter()
            .position(|n| n == name)
            .map(|i| self.node_voltage(i))
    }
    /// Index of an instance by name.
    pub fn instance_index(&self, name: &str) -> Option<usize> {
        self.insts.iter().position(|c| c.name == name)
    }
    /// Number of compiled (non-ground) instances.
    pub fn instance_count(&self) -> usize {
        self.insts.len()
    }
    /// Instance name.
    pub fn instance_name(&self, k: usize) -> &str {
        &self.insts[k].name
    }
    /// Whether the instance was flagged with convergence trouble.
    pub fn instance_trouble(&self, k: usize) -> bool {
        self.insts[k].trouble
    }
    /// Current a→b through each branch of instance `k`, at the last solution.
    pub fn branch_currents(&self, k: usize) -> Vec<f64> {
        let c = &self.insts[k];
        c.branches
            .iter()
            .map(|b| match b {
                CBranch::Current { current, .. } => *current,
                CBranch::Voltage { j, .. } => self.x[c.map[*j]],
            })
            .collect()
    }
    /// Evaluate the instance's probes at the current solution.
    pub fn probes(&mut self, k: usize) -> Vec<f64> {
        let c = &mut self.insts[k];
        for (l, &g) in c.map.iter().enumerate() {
            c.xl[l] = self.x[g];
        }
        c.live[LIVE_T] = self.time;
        c.probe_tape.eval(&c.xl, &c.live, &mut self.ws);
        (0..c.probe_tape.n_outputs())
            .map(|o| c.probe_tape.value(&self.ws, o))
            .collect()
    }
    /// Change a live parameter; inserts a breakpoint. Returns false if the
    /// parameter is not live (a recompile is needed).
    pub fn set_param(&mut self, k: usize, name: &str, value: f64) -> bool {
        let c = &mut self.insts[k];
        match c.live_params.get(name) {
            Some(&slot) => {
                c.live[slot] = value;
                self.after_breakpoint = true;
                true
            }
            None => false,
        }
    }
    /// Change a state variable; inserts a breakpoint.
    pub fn set_state(&mut self, k: usize, name: &str, value: f64) -> bool {
        let c = &mut self.insts[k];
        match c.live_states.get(name) {
            Some(&slot) => {
                c.live[slot] = value;
                self.after_breakpoint = true;
                true
            }
            None => false,
        }
    }
    /// Operate a user control. Instances with a behavior get `on_control`
    /// (which may drive outputs); others have the state set directly.
    pub fn control(&mut self, k: usize, name: &str, value: f64) -> bool {
        if let Some(di) = self
            .digital
            .iter()
            .position(|d| d.k == k && d.behavior.is_some())
        {
            let ev = ControlEvent::SetState {
                state: name.to_string(),
                value,
            };
            let prev = self.digital[di].levels.clone();
            self.run_hook(di, Hook::Control(ev), prev);
            self.after_breakpoint = true;
            true
        } else {
            self.set_state(k, name, value)
        }
    }

    /// Apply pending events at once and settle the digital parts (used while
    /// paused, so a toggle shows its effect immediately).
    pub fn settle(&mut self) -> Result<(), SimError> {
        self.settle_start_inner(false)
    }

    /// Every state variable of an instance.
    pub fn states(&self, k: usize) -> Vec<(String, f64)> {
        let c = &self.insts[k];
        c.live_states
            .iter()
            .map(|(n, &s)| (n.clone(), c.live[s]))
            .collect()
    }

    /// Read a state variable.
    pub fn state(&self, k: usize, name: &str) -> Option<f64> {
        let c = &self.insts[k];
        c.live_states.get(name).map(|&s| c.live[s])
    }

    /// Assemble the Newton system at `self.x`. `alpha` is the integration
    /// coefficient (0 for DC); `scale` scales independent sources.
    fn assemble(
        &mut self,
        alpha: f64,
        method: Option<Method>,
        gmin: f64,
        scale: f64,
    ) -> Result<bool, SimError> {
        let n = self.n;
        self.solver.reset(n);
        self.rhs.iter_mut().for_each(|v| *v = 0.0);
        self.mag.iter_mut().for_each(|v| *v = 0.0);
        let mut limited = false;
        for i in 0..self.n_nodes - 1 {
            self.solver.add(i, i, gmin);
        }
        let ws = &mut self.ws;
        for c in &mut self.insts {
            for (l, &g) in c.map.iter().enumerate() {
                c.xl[l] = self.x[g];
            }
            // Limiting: evaluate at limited controlling voltages. A node moved by
            // one limit stays put; later limits sharing it move their other node.
            let mut moved: u64 = 0;
            let bit = |l: usize| if l < 64 { 1u64 << l } else { 0 };
            for lim in &mut c.limits {
                let va = lim.a.map_or(0.0, |l| self.x[c.map[l]]);
                let vb = lim.b.map_or(0.0, |l| self.x[c.map[l]]);
                let vnew = va - vb;
                let vold = lim.last.unwrap_or(0.0);
                let (v, hit) = pnjlim(vnew, vold, lim.vte, lim.vcrit);
                limited |= hit;
                let a_moved = lim.a.is_some_and(|a| moved & bit(a) != 0);
                let b_moved = lim.b.is_some_and(|b| moved & bit(b) != 0);
                match (lim.a, lim.b) {
                    (Some(a), Some(b)) if a_moved => {
                        c.xl[b] = c.xl[a] - v;
                        moved |= bit(b);
                    }
                    (Some(a), Some(b)) if b_moved => {
                        c.xl[a] = c.xl[b] + v;
                        moved |= bit(a);
                    }
                    (Some(a), _) if hit => {
                        c.xl[a] = lim.b.map_or(0.0, |b| c.xl[b]) + v;
                        moved |= bit(a);
                    }
                    (None, Some(b)) if hit => {
                        c.xl[b] = -v;
                        moved |= bit(b);
                    }
                    _ => {}
                }
                lim.last = Some(v);
            }
            c.live[LIVE_T] = self.time;
            c.tape.eval(&c.xl, &c.live, ws);
            let tape = &c.tape;
            for b in &c.branches {
                match b {
                    CBranch::Current {
                        a,
                        b,
                        out_i,
                        out_q,
                        source,
                        q_prev,
                        iq_prev,
                        ..
                    } => {
                        let mut f = tape.value(ws, *out_i);
                        if *source {
                            f *= scale;
                        }
                        let gi = tape.grad(ws, *out_i);
                        let mut grad: [f64; 16] = [0.0; 16];
                        let nl = gi.len();
                        let mut gvec;
                        let g: &mut [f64] = if nl <= 16 {
                            grad[..nl].copy_from_slice(gi);
                            &mut grad[..nl]
                        } else {
                            gvec = gi.to_vec();
                            &mut gvec
                        };
                        if let (Some(oq), Some(m)) = (out_q, method) {
                            let q = tape.value(ws, *oq);
                            let gq = tape.grad(ws, *oq);
                            let hist = match m {
                                Method::BackwardEuler => -alpha * q_prev,
                                Method::Trapezoidal => {
                                    -alpha * q_prev - self.options.damping() * iq_prev
                                }
                            };
                            f += alpha * q + hist;
                            for (gk, gqk) in g.iter_mut().zip(gq) {
                                *gk += alpha * gqk;
                            }
                        }
                        if !f.is_finite() || g.iter().any(|v| !v.is_finite()) {
                            return Err(SimError::NotFinite {
                                instance: c.name.clone(),
                            });
                        }
                        let lin: f64 = g.iter().zip(&c.xl).map(|(gk, xk)| gk * xk).sum::<f64>() - f;
                        for node in [a, b].into_iter().flatten() {
                            self.mag[c.map[*node]] += f.abs();
                        }
                        if let Some(a) = a {
                            let row = c.map[*a];
                            for (k, gk) in g.iter().enumerate() {
                                if *gk != 0.0 {
                                    self.solver.add(row, c.map[k], *gk);
                                }
                            }
                            self.rhs[row] += lin;
                        }
                        if let Some(b) = b {
                            let row = c.map[*b];
                            for (k, gk) in g.iter().enumerate() {
                                if *gk != 0.0 {
                                    self.solver.add(row, c.map[k], -*gk);
                                }
                            }
                            self.rhs[row] -= lin;
                        }
                    }
                    CBranch::Voltage {
                        a,
                        b,
                        j,
                        out_v,
                        out_f,
                        source,
                        f_prev,
                        vf_prev,
                    } => {
                        let jg = c.map[*j];
                        if let Some(a) = a {
                            self.solver.add(c.map[*a], jg, 1.0);
                        }
                        if let Some(b) = b {
                            self.solver.add(c.map[*b], jg, -1.0);
                        }
                        // F = V(a) - V(b) - v - dflux/dt
                        let mut v = tape.value(ws, *out_v);
                        if *source {
                            v *= scale;
                        }
                        let va = a.map_or(0.0, |l| c.xl[l]);
                        let vb = b.map_or(0.0, |l| c.xl[l]);
                        let mut f = va - vb - v;
                        let gv = tape.grad(ws, *out_v);
                        let mut g: Vec<f64> = gv.iter().map(|x| -x).collect();
                        if let Some(a) = a {
                            g[*a] += 1.0;
                        }
                        if let Some(b) = b {
                            g[*b] -= 1.0;
                        }
                        if let Some(of) = out_f {
                            let fl = tape.value(ws, *of);
                            let gf = tape.grad(ws, *of);
                            if let Some(m) = method {
                                let hist = match m {
                                    Method::BackwardEuler => -alpha * f_prev,
                                    Method::Trapezoidal => {
                                        -alpha * f_prev - self.options.damping() * vf_prev
                                    }
                                };
                                f -= alpha * fl + hist;
                                for (gk, gfk) in g.iter_mut().zip(gf) {
                                    *gk -= alpha * gfk;
                                }
                            }
                        }
                        if !f.is_finite() || g.iter().any(|v| !v.is_finite()) {
                            return Err(SimError::NotFinite {
                                instance: c.name.clone(),
                            });
                        }
                        let lin: f64 = g.iter().zip(&c.xl).map(|(gk, xk)| gk * xk).sum::<f64>() - f;
                        self.mag[jg] += va.abs() + vb.abs() + v.abs();
                        for (k, gk) in g.iter().enumerate() {
                            if *gk != 0.0 {
                                self.solver.add(jg, c.map[k], *gk);
                            }
                        }
                        self.rhs[jg] += lin;
                    }
                }
            }
        }
        Ok(limited)
    }

    /// Newton-Raphson at the present time. Returns iterations used.
    fn newton(
        &mut self,
        alpha: f64,
        method: Option<Method>,
        gmin: f64,
        scale: f64,
        max_iter: usize,
    ) -> Result<usize, SimError> {
        let opts = self.options;
        for c in &mut self.insts {
            for l in &mut c.limits {
                let va = l.a.map_or(0.0, |k| self.x[c.map[k]]);
                let vb = l.b.map_or(0.0, |k| self.x[c.map[k]]);
                l.last = Some(va - vb);
            }
        }
        let mut mag_ref: Vec<f64> = Vec::new();
        // The previous update passed the voltage test only loosely: accept the
        // point once its own KCL residual balances too.
        let mut verify = false;
        for iter in 1..=max_iter {
            let limited = self.assemble(alpha, method, gmin, scale)?;
            if verify && !limited {
                let mut jx = vec![0.0; self.n];
                self.solver.mul(&self.x, &mut jx);
                let floor_i = opts.abstol.max(1e-9);
                let balanced = (0..self.n_nodes.saturating_sub(1)).all(|r| {
                    let f = (jx[r] - self.rhs[r]).abs();
                    f <= opts.reltol * mag_ref[r].max(self.mag[r]) + floor_i
                });
                if balanced {
                    return Ok(iter - 1);
                }
            }
            verify = false;
            if iter == 1 {
                // Tolerance scale from the starting (converged) point, so an
                // overshooting iterate cannot loosen its own test.
                mag_ref.clone_from(&self.mag);
            }
            // Residual test (SPICE checks device currents as well as voltage
            // changes): KCL within reltol of the currents meeting at each node.
            // It accepts states where the Jacobian is nearly singular along a
            // direction that carries no current, such as a regenerative edge.
            if iter > 1 && !limited && !self.linear {
                let mut jx = vec![0.0; self.n];
                self.solver.mul(&self.x, &mut jx);
                let ok = (0..self.n).all(|r| {
                    let f = (jx[r] - self.rhs[r]).abs();
                    let floor = if r < self.n_nodes - 1 {
                        opts.abstol.max(1e-9)
                    } else {
                        opts.vntol
                    };
                    f <= opts.reltol * mag_ref[r] + floor
                });
                if ok && iter > 2 {
                    return Ok(iter);
                }
            }
            if !self.solver.factor() {
                return Err(SimError::Singular);
            }
            let mut xn = self.rhs.clone();
            self.solver.solve(&mut xn);
            if xn.iter().any(|v| !v.is_finite()) {
                return Err(SimError::Singular);
            }
            let mut converged = !limited;
            let mut worst = (0.0, 0usize);
            #[allow(clippy::needless_range_loop)] // k indexes xn, x and the node split
            for k in 0..self.n {
                let tol = opts.reltol * xn[k].abs().max(self.x[k].abs())
                    + if k < self.n_nodes - 1 {
                        opts.vntol
                    } else {
                        opts.abstol
                    };
                let d = (xn[k] - self.x[k]).abs();
                if d > tol {
                    converged = false;
                }
                if d / tol > worst.0 {
                    worst = (d / tol, k);
                }
            }
            self.x = xn;
            if self.linear || (converged && self.n == 0) {
                return Ok(iter);
            }
            if converged && iter > 1 {
                // Quadratic convergence leaves the last update far inside the
                // tolerance. A step that only just fits can be a Newton walk
                // along an exponential (about one vt per step) far from the
                // solution, as SPICE's device-current check also guards.
                if worst.0 < 0.1 {
                    return Ok(iter);
                }
                verify = true;
            }
            if iter == max_iter {
                let name = self.unknown_names.get(worst.1).cloned().unwrap_or_default();
                return Err(SimError::NoConvergence {
                    time: self.time,
                    nodes: name,
                });
            }
        }
        Ok(max_iter)
    }

    fn commit(&mut self, alpha: f64, method: Option<Method>) {
        let damping = self.options.damping();
        let ws = &mut self.ws;
        for c in &mut self.insts {
            for (l, &g) in c.map.iter().enumerate() {
                c.xl[l] = self.x[g];
            }
            c.live[LIVE_T] = self.time;
            c.tape.eval(&c.xl, &c.live, ws);
            for b in &mut c.branches {
                match b {
                    CBranch::Current {
                        out_i,
                        out_q,
                        q_prev,
                        iq_prev,
                        current,
                        ..
                    } => {
                        let i = c.tape.value(ws, *out_i);
                        match (out_q, method) {
                            (Some(oq), Some(m)) => {
                                let q = c.tape.value(ws, *oq);
                                let iq = match m {
                                    Method::BackwardEuler => alpha * (q - *q_prev),
                                    Method::Trapezoidal => {
                                        alpha * (q - *q_prev) - damping * *iq_prev
                                    }
                                };
                                *q_prev = q;
                                *iq_prev = iq;
                                *current = i + iq;
                            }
                            (Some(oq), None) => {
                                *q_prev = c.tape.value(ws, *oq);
                                *iq_prev = 0.0;
                                *current = i;
                            }
                            _ => *current = i,
                        }
                    }
                    CBranch::Voltage {
                        out_f,
                        f_prev,
                        vf_prev,
                        ..
                    } => {
                        if let Some(of) = out_f {
                            let fl = c.tape.value(ws, *of);
                            *vf_prev = match method {
                                Some(Method::BackwardEuler) => alpha * (fl - *f_prev),
                                Some(Method::Trapezoidal) => {
                                    alpha * (fl - *f_prev) - damping * *vf_prev
                                }
                                None => 0.0,
                            };
                            *f_prev = fl;
                        }
                    }
                }
            }
        }
    }

    /// DC operating point with capacitors open and inductors shorted. Tries
    /// plain Newton, then gmin stepping, then source stepping (spec 4.3).
    pub fn dc_op(&mut self) -> Result<OpMethod, SimError> {
        let saved = self.x.clone();
        let gmin = self.options.gmin;
        let max = self.options.max_iter.max(50);
        let result = (|| {
            if self.newton(0.0, None, gmin, 1.0, max).is_ok() {
                return Ok(OpMethod::Newton);
            }
            self.x.iter_mut().for_each(|v| *v = 0.0);
            let mut ok = true;
            let mut g = 1e-3;
            while g > gmin {
                if self.newton(0.0, None, g, 1.0, max).is_err() {
                    ok = false;
                    break;
                }
                g /= 10.0;
            }
            if ok && self.newton(0.0, None, gmin, 1.0, max).is_ok() {
                return Ok(OpMethod::GminStepping);
            }
            self.x.iter_mut().for_each(|v| *v = 0.0);
            let steps = 20;
            for s in 1..=steps {
                let scale = s as f64 / steps as f64;
                self.newton(0.0, None, gmin, scale, max)?;
            }
            Ok(OpMethod::SourceStepping)
        })();
        match result {
            Ok(m) => {
                self.commit(0.0, None);
                self.after_breakpoint = true;
                Ok(m)
            }
            Err(e) => {
                self.x = saved;
                Err(e)
            }
        }
    }

    /// Reset time to zero and solve the operating point.
    pub fn reset(&mut self) -> Result<OpMethod, SimError> {
        self.time = 0.0;
        self.x.iter_mut().for_each(|v| *v = 0.0);
        for c in &mut self.insts {
            c.trouble = false;
            for l in &mut c.limits {
                l.last = None;
            }
        }
        self.events.clear();
        for d in &mut self.digital {
            d.levels.clear();
        }
        let method = self.dc_op()?;
        if !self.digital.is_empty() {
            self.settle_start()?;
        }
        Ok(method)
    }

    /// Advance by `h` with the given method. Backward Euler is used instead on
    /// the first step and after a breakpoint (spec 4.2).
    pub fn step(&mut self, h: f64, method: Method) -> Result<usize, SimError> {
        self.step_inner(h, method, false)
    }

    fn step_inner(
        &mut self,
        h: f64,
        method: Method,
        accept_failure: bool,
    ) -> Result<usize, SimError> {
        // Land exactly on the next digital event (spec 4.7).
        let mut h = h;
        if let Some(te) = self.next_event()
            && te > self.time + 1e-15
            && te < self.time + h
        {
            h = te - self.time;
        }
        let method = if self.after_breakpoint {
            Method::BackwardEuler
        } else {
            method
        };
        let alpha = match method {
            Method::BackwardEuler => 1.0 / h,
            Method::Trapezoidal => 1.0 / (self.options.theta * h),
        };
        let saved = self.x.clone();
        let t0 = self.time;
        self.time = t0 + h;
        match self.newton(
            alpha,
            Some(method),
            self.options.gmin,
            1.0,
            self.options.max_iter,
        ) {
            Ok(iters) => {
                self.commit(alpha, Some(method));
                self.after_breakpoint = false;
                self.last_iterations = iters;
                self.digital_after_step();
                Ok(iters)
            }
            Err(e) => {
                // Accept the last iterate only if it is finite and of the same
                // order as the state the step started from.
                let bound = 10.0 * saved.iter().fold(1.0f64, |m, v| m.max(v.abs())) + 10.0;
                let sane = self.x.iter().all(|v| v.is_finite() && v.abs() <= bound);
                if accept_failure && sane {
                    // Keep the last Newton iterate so the circuit keeps moving.
                    self.commit(alpha, Some(method));
                    self.after_breakpoint = true;
                    self.last_iterations = self.options.max_iter;
                    self.digital_after_step();
                } else {
                    self.x = saved;
                    self.time = t0;
                }
                Err(e)
            }
        }
    }

    /// Interactive step (spec 4.5): on a Newton failure the step is halved (down
    /// to h/2^20) and grows back after two successes. At the floor
    /// the step is accepted anyway, the trouble is flagged on the nonlinear
    /// components, and the simulation continues. Returns the time advanced.
    pub fn step_interactive(&mut self, h: f64) -> f64 {
        let start = self.time;
        // Regenerative edges (multivibrators, latches) need steps short enough
        // for junction charge to dominate, far below the nominal step.
        let floor = h / 1_048_576.0;
        let mut hh = h;
        let mut remaining = h;
        let mut failures = 0;
        let mut successes = 0;
        while remaining > h * 1e-9 {
            let take = hh.min(remaining);
            let at_floor = hh <= floor * 1.0001;
            let before = self.time;
            match self.step_inner(take, Method::Trapezoidal, at_floor) {
                Ok(_) => {
                    let take = self.time - before;
                    self.clean_steps += 1;
                    if self.clean_steps == 5_000 {
                        // Trouble markers fade once the circuit runs cleanly again.
                        self.insts.iter_mut().for_each(|c| c.trouble = false);
                    }
                    remaining -= take;
                    failures = 0;
                    successes += 1;
                    if successes >= 2 && hh < h {
                        hh = (hh * 2.0).min(h);
                        successes = 0;
                    }
                }
                Err(e) if at_floor => {
                    self.clean_steps = 0;
                    let flag = match &e {
                        SimError::NotFinite { instance } | SimError::Component { instance, .. } => {
                            Some(instance.clone())
                        }
                        _ => None,
                    };
                    for c in &mut self.insts {
                        let named = flag.as_ref().is_some_and(|f| *f == c.name);
                        if named || (flag.is_none() && !c.limits.is_empty()) {
                            c.trouble = true;
                        }
                    }
                    if self.time == before {
                        // Not even an iterate could be kept (non-finite): skip ahead.
                        self.time += take;
                    }
                    let take = self.time - before;
                    remaining -= take;
                    failures = 0;
                }
                Err(_) => {
                    failures += 1;
                    successes = 0;
                    hh = (hh / 2.0).max(floor);
                    self.after_breakpoint = true;
                    if failures > 24 {
                        hh = floor;
                    }
                }
            }
        }
        self.time - start
    }

    /// Fixed-step transient from the operating point. Records every unknown at
    /// every step: `(time, solution)`.
    pub fn transient(
        &mut self,
        stop: f64,
        h: f64,
        method: Method,
    ) -> Result<Vec<(f64, Vec<f64>)>, SimError> {
        self.reset()?;
        let mut out = vec![(0.0, self.x.clone())];
        let steps = (stop / h).round() as usize;
        for _ in 0..steps {
            self.step(h, method)?;
            out.push((self.time, self.x.clone()));
        }
        Ok(out)
    }
}

/// Which behavior hook to run.
enum Hook {
    Start,
    Inputs,
    Timer(u32),
    Control(ControlEvent),
}

impl Circuit {
    /// Time of the next scheduled event.
    pub fn next_event(&self) -> Option<f64> {
        self.events.peek().map(|e| f64::from_bits(e.time))
    }

    fn local_voltage(&self, k: usize, local: Option<usize>) -> f64 {
        local.map_or(0.0, |l| self.x[self.insts[k].map[l]])
    }

    /// Input levels (with hysteresis) and watch values of a digital instance.
    fn read_levels(&mut self, di: usize) -> BTreeMap<String, Level> {
        let d = &self.digital[di];
        let mut out = BTreeMap::new();
        for pin in &d.inputs {
            let v = self.local_voltage(d.k, pin.local);
            let prev = d.levels.get(&pin.name).copied().unwrap_or(Level::X);
            let level = if v >= pin.fam.vih {
                Level::High
            } else if v <= pin.fam.vil {
                Level::Low
            } else {
                prev
            };
            out.insert(pin.name.clone(), level);
        }
        if !d.watch_names.is_empty() {
            let k = d.k;
            let names = d.watch_names.clone();
            let c = &mut self.insts[k];
            for (l, &g) in c.map.iter().enumerate() {
                c.xl[l] = self.x[g];
            }
            c.live[LIVE_T] = self.time;
            c.watch_tape.eval(&c.xl, &c.live, &mut self.ws);
            for (w, name) in names.iter().enumerate() {
                out.insert(
                    name.clone(),
                    Level::from_bool(c.watch_tape.value(&self.ws, w) > 0.5),
                );
            }
        }
        out
    }

    /// Run a behavior hook, write declared states back to the equations and
    /// schedule the requested drives and timers.
    fn run_hook(&mut self, di: usize, hook: Hook, previous: BTreeMap<String, Level>) {
        let Some(behavior) = self.digital[di].behavior.clone() else {
            return;
        };
        let k = self.digital[di].k;
        // Sync declared states from their live slots.
        let mut state = self.digital[di].state.clone();
        for (name, &slot) in &self.insts[k].live_states {
            state.insert(name.clone(), self.insts[k].live[slot]);
        }
        let voltages: BTreeMap<String, f64> = self.digital[di]
            .pins
            .iter()
            .map(|(n, l)| (n.clone(), self.local_voltage(k, *l)))
            .collect();
        let levels = self.digital[di].levels.clone();
        let params = self.digital[di].params.clone();
        let mut ctx = EventCtx {
            time: self.time,
            state: &mut state,
            params: &params,
            levels: &levels,
            previous: &previous,
            voltages: &voltages,
            actions: Vec::new(),
        };
        match hook {
            Hook::Start => behavior.on_start(&mut ctx),
            Hook::Inputs => behavior.on_inputs(&mut ctx),
            Hook::Timer(tag) => behavior.on_timer(&mut ctx, tag),
            Hook::Control(ev) => behavior.on_control(&mut ctx, &ev),
        }
        let actions = std::mem::take(&mut ctx.actions);
        drop(ctx);
        // Declared states feed the equations: a change is a breakpoint.
        let slots: Vec<(String, usize)> = self.insts[k]
            .live_states
            .iter()
            .map(|(n, s)| (n.clone(), *s))
            .collect();
        for (name, slot) in slots {
            if let Some(&v) = state.get(&name)
                && self.insts[k].live[slot] != v
            {
                self.insts[k].live[slot] = v;
                self.after_breakpoint = true;
            }
        }
        self.digital[di].state = state;
        for a in actions {
            match a {
                Action::Drive { pin, level, delay } => {
                    let Some(out) = self.digital[di]
                        .outputs
                        .iter()
                        .position(|o| o.pin.name == pin)
                    else {
                        continue;
                    };
                    if self.settling {
                        self.apply_drive(di, out, level, true);
                    } else {
                        self.schedule(self.time + delay, di, EvKind::Drive { out, level });
                    }
                }
                Action::Timer { delay, tag } => {
                    self.schedule(self.time + delay.max(1e-12), di, EvKind::Timer(tag))
                }
            }
        }
    }

    fn schedule(&mut self, time: f64, d: usize, kind: EvKind) {
        self.seq += 1;
        self.events.push(Event {
            time: time.max(0.0).to_bits(),
            seq: self.seq,
            d,
            kind,
        });
    }

    /// Start a transition on an output. `instant` skips the ramp.
    fn apply_drive(&mut self, di: usize, out: usize, level: Level, instant: bool) {
        let d = &self.digital[di];
        let o = &d.outputs[out];
        if o.level == level && !instant {
            return;
        }
        let k = d.k;
        let c = &self.insts[k];
        let (v0, v1, t0) = (c.live[o.slot_v0], c.live[o.slot_v1], c.live[o.slot_t0]);
        let tr = o.pin.fam.tr;
        let now = v0 + (v1 - v0) * ((self.time - t0) / tr).clamp(0.0, 1.0);
        let (vol, voh) = match o.rails {
            // CMOS-style (7555) output stage: nearly rail to rail.
            Some((p, n)) => (
                self.local_voltage(k, n) + 0.05,
                self.local_voltage(k, p) - 0.2,
            ),
            None => (o.pin.fam.vol, o.pin.fam.voh),
        };
        let target = match level {
            Level::High => voh,
            Level::Low => vol,
            Level::X => 0.5 * (vol + voh),
        };
        let (s0, s1, st) = (o.slot_v0, o.slot_v1, o.slot_t0);
        let time = self.time;
        let c = &mut self.insts[k];
        if instant {
            c.live[s0] = target;
            c.live[s1] = target;
            c.live[st] = -1.0;
        } else {
            c.live[s0] = now;
            c.live[s1] = target;
            c.live[st] = time;
        }
        self.digital[di].outputs[out].level = level;
        self.after_breakpoint = true;
    }

    /// Apply due events, then re-read inputs and run `on_inputs` where they
    /// changed. Zero-delay chains are followed a bounded number of times.
    fn digital_after_step(&mut self) {
        if self.digital.is_empty() {
            return;
        }
        for _ in 0..64 {
            let mut busy = false;
            while let Some(e) = self.events.peek().copied() {
                if f64::from_bits(e.time) > self.time + 1e-15 {
                    break;
                }
                self.events.pop();
                busy = true;
                match e.kind {
                    EvKind::Drive { out, level } => self.apply_drive(e.d, out, level, false),
                    EvKind::Timer(tag) => {
                        let prev = self.digital[e.d].levels.clone();
                        self.run_hook(e.d, Hook::Timer(tag), prev);
                    }
                }
            }
            for di in 0..self.digital.len() {
                let now = self.read_levels(di);
                if now != self.digital[di].levels {
                    let prev = std::mem::replace(&mut self.digital[di].levels, now);
                    busy = true;
                    self.run_hook(di, Hook::Inputs, prev);
                }
            }
            let due_now = self.next_event().is_some_and(|t| t <= self.time + 1e-15);
            if !busy || !due_now {
                break;
            }
        }
    }

    fn settle_start(&mut self) -> Result<(), SimError> {
        self.settle_start_inner(true)
    }

    /// Settle digital outputs at the present time: drives apply instantly and
    /// the operating point is re-solved until nothing changes.
    fn settle_start_inner(&mut self, start: bool) -> Result<(), SimError> {
        self.settling = true;
        let result = (|| {
            // Pending drives due now apply instantly.
            let mut keep = Vec::new();
            while let Some(e) = self.events.pop() {
                match e.kind {
                    EvKind::Drive { out, level }
                        if f64::from_bits(e.time) <= self.time + 1e-6 || !start =>
                    {
                        self.apply_drive(e.d, out, level, true)
                    }
                    _ => keep.push(e),
                }
            }
            self.events.extend(keep);
            for round in 0..20 {
                let mut changed = false;
                for di in 0..self.digital.len() {
                    let now = self.read_levels(di);
                    let first = start && round == 0;
                    if first || now != self.digital[di].levels {
                        // Settling re-evaluates levels but reports no edges:
                        // power-up is not a clock edge.
                        self.digital[di].levels = now.clone();
                        let prev = now;
                        let outs: Vec<Level> =
                            self.digital[di].outputs.iter().map(|o| o.level).collect();
                        self.run_hook(di, if first { Hook::Start } else { Hook::Inputs }, prev);
                        let after: Vec<Level> =
                            self.digital[di].outputs.iter().map(|o| o.level).collect();
                        changed |= outs != after || first;
                    }
                }
                if !changed {
                    break;
                }
                self.dc_op()?;
            }
            Ok(())
        })();
        self.settling = false;
        self.after_breakpoint = true;
        result
    }
}

#[path = "analysis.rs"]
pub mod analysis;
