//! Hash-consed expression graph compiled to a flat instruction tape with
//! forward-mode automatic differentiation.

use crate::{BinOp, Expr, UnOp};
use std::collections::HashMap;
use std::fmt;

/// Index of a node in a [`TapeBuilder`].
pub type NodeId = u32;

/// What a symbol stands for once a component is being compiled.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Leaf {
    /// A value fixed for the life of the tape (non-live parameters are folded here).
    Const(f64),
    /// A local unknown (node voltage or branch current). Derivatives are taken
    /// with respect to these, in index order.
    Unknown(usize),
    /// A value that changes between evaluations but is not differentiated:
    /// time, temperature, live parameters, state variables.
    Live(usize),
}

/// A symbol reference handed to the resolver.
#[derive(Debug, Clone, Copy)]
pub enum Sym<'a> {
    Name(&'a str),
    V(&'a str, Option<&'a str>),
    I(&'a str),
}

/// A resolver's answer for a symbol.
#[derive(Debug, Clone)]
pub enum Resolved {
    Leaf(Leaf),
    /// `a - b`, used for voltage differences.
    Diff(Leaf, Leaf),
    /// Another expression compiled in place (a `let` name).
    Expr(Expr),
}

/// Compilation failure, usually an unknown name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileError(pub String);

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for CompileError {}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Op {
    Const(f64),
    Unknown(u32),
    Live(u32),
    Un(UnOp, NodeId),
    Bin(BinOp, NodeId, NodeId),
    Select(NodeId, NodeId, NodeId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Key {
    Const(u64),
    Unknown(u32),
    Live(u32),
    Un(UnOp, NodeId),
    Bin(BinOp, NodeId, NodeId),
    Select(NodeId, NodeId, NodeId),
}

fn key(op: &Op) -> Key {
    match *op {
        // Normalise -0.0 so it shares a node with 0.0.
        Op::Const(v) => Key::Const(if v == 0.0 { 0 } else { v.to_bits() }),
        Op::Unknown(i) => Key::Unknown(i),
        Op::Live(i) => Key::Live(i),
        Op::Un(o, a) => Key::Un(o, a),
        Op::Bin(o, a, b) => Key::Bin(o, a, b),
        Op::Select(c, a, b) => Key::Select(c, a, b),
    }
}

const LIMEXP_KNEE: f64 = 80.0;

fn un_value(op: UnOp, a: f64) -> f64 {
    match op {
        UnOp::Neg => -a,
        UnOp::Exp => a.exp(),
        UnOp::Limexp => {
            if a < LIMEXP_KNEE {
                a.exp()
            } else {
                LIMEXP_KNEE.exp() * (1.0 + a - LIMEXP_KNEE)
            }
        }
        UnOp::Ln => a.ln(),
        UnOp::Log10 => a.log10(),
        UnOp::Sqrt => a.sqrt(),
        UnOp::SafeSqrt => a.max(0.0).sqrt(),
        UnOp::Sin => a.sin(),
        UnOp::Cos => a.cos(),
        UnOp::Tan => a.tan(),
        UnOp::Atan => a.atan(),
        UnOp::Tanh => a.tanh(),
        UnOp::Sinh => a.sinh(),
        UnOp::Cosh => a.cosh(),
        UnOp::Abs => a.abs(),
        UnOp::Floor => a.floor(),
        UnOp::Not => f64::from(a == 0.0),
    }
}

/// d(op(a))/da evaluated at `a`, given the op's value `v`.
fn un_slope(op: UnOp, a: f64, v: f64) -> f64 {
    match op {
        UnOp::Neg => -1.0,
        UnOp::Exp => v,
        UnOp::Limexp => {
            if a < LIMEXP_KNEE {
                v
            } else {
                LIMEXP_KNEE.exp()
            }
        }
        UnOp::Ln => 1.0 / a,
        UnOp::Log10 => 1.0 / (a * std::f64::consts::LN_10),
        UnOp::Sqrt => 0.5 / v,
        UnOp::SafeSqrt => {
            if a > 0.0 {
                0.5 / v
            } else {
                0.0
            }
        }
        UnOp::Sin => a.cos(),
        UnOp::Cos => -a.sin(),
        UnOp::Tan => 1.0 + v * v,
        UnOp::Atan => 1.0 / (1.0 + a * a),
        UnOp::Tanh => 1.0 - v * v,
        UnOp::Sinh => a.cosh(),
        UnOp::Cosh => a.sinh(),
        UnOp::Abs => {
            if a >= 0.0 {
                1.0
            } else {
                -1.0
            }
        }
        UnOp::Floor | UnOp::Not => 0.0,
    }
}

fn safe_den(b: f64) -> f64 {
    if b.abs() < 1e-30 {
        if b < 0.0 { -1e-30 } else { 1e-30 }
    } else {
        b
    }
}

fn bin_value(op: BinOp, a: f64, b: f64) -> f64 {
    match op {
        BinOp::Add => a + b,
        BinOp::Sub => a - b,
        BinOp::Mul => a * b,
        BinOp::Div => a / b,
        BinOp::Pow => a.powf(b),
        BinOp::Min => a.min(b),
        BinOp::Max => a.max(b),
        BinOp::SafeDiv => a / safe_den(b),
        BinOp::Lt => f64::from(a < b),
        BinOp::Le => f64::from(a <= b),
        BinOp::Gt => f64::from(a > b),
        BinOp::Ge => f64::from(a >= b),
        BinOp::Eq => f64::from(a == b),
        BinOp::Ne => f64::from(a != b),
        BinOp::And => f64::from(a != 0.0 && b != 0.0),
        BinOp::Or => f64::from(a != 0.0 || b != 0.0),
    }
}

/// (d/da, d/db) of a binary op at (a, b) with value v.
fn bin_slopes(op: BinOp, a: f64, b: f64, v: f64) -> (f64, f64) {
    match op {
        BinOp::Add => (1.0, 1.0),
        BinOp::Sub => (1.0, -1.0),
        BinOp::Mul => (b, a),
        BinOp::Div => (1.0 / b, -a / (b * b)),
        BinOp::SafeDiv => {
            let d = safe_den(b);
            (1.0 / d, if b.abs() < 1e-30 { 0.0 } else { -a / (d * d) })
        }
        BinOp::Pow => {
            let da = if b == 0.0 { 0.0 } else { b * a.powf(b - 1.0) };
            let db = if a > 0.0 { v * a.ln() } else { 0.0 };
            (da, db)
        }
        BinOp::Min => {
            if a <= b {
                (1.0, 0.0)
            } else {
                (0.0, 1.0)
            }
        }
        BinOp::Max => {
            if a >= b {
                (1.0, 0.0)
            } else {
                (0.0, 1.0)
            }
        }
        _ => (0.0, 0.0),
    }
}

/// Builds a hash-consed expression graph. Identical subexpressions share one
/// node (CSE) and operations on constants fold immediately.
#[derive(Debug, Clone)]
pub struct TapeBuilder {
    n_unknowns: usize,
    ops: Vec<Op>,
    map: HashMap<Key, NodeId>,
}

impl TapeBuilder {
    /// A builder whose tapes differentiate with respect to `n_unknowns` locals.
    pub fn new(n_unknowns: usize) -> Self {
        Self {
            n_unknowns,
            ops: Vec::new(),
            map: HashMap::new(),
        }
    }

    fn intern(&mut self, op: Op) -> NodeId {
        let k = key(&op);
        if let Some(&id) = self.map.get(&k) {
            return id;
        }
        let id = self.ops.len() as NodeId;
        self.ops.push(op);
        self.map.insert(k, id);
        id
    }

    fn constant(&self, id: NodeId) -> Option<f64> {
        match self.ops[id as usize] {
            Op::Const(v) => Some(v),
            _ => None,
        }
    }

    /// A constant node.
    pub fn konst(&mut self, v: f64) -> NodeId {
        self.intern(Op::Const(v))
    }

    /// A leaf node.
    pub fn leaf(&mut self, leaf: Leaf) -> NodeId {
        match leaf {
            Leaf::Const(v) => self.konst(v),
            Leaf::Unknown(i) => {
                assert!(i < self.n_unknowns, "unknown {i} out of range");
                self.intern(Op::Unknown(i as u32))
            }
            Leaf::Live(i) => self.intern(Op::Live(i as u32)),
        }
    }

    /// Apply a unary operation, folding constants.
    pub fn un(&mut self, op: UnOp, a: NodeId) -> NodeId {
        if let Some(x) = self.constant(a) {
            return self.konst(un_value(op, x));
        }
        if op == UnOp::Neg
            && let Op::Un(UnOp::Neg, inner) = self.ops[a as usize]
        {
            return inner;
        }
        self.intern(Op::Un(op, a))
    }

    /// Apply a binary operation, folding constants and simple identities.
    // Explicit `==` guards read better than float literal patterns here.
    #[allow(clippy::redundant_guards)]
    pub fn bin(&mut self, op: BinOp, a: NodeId, b: NodeId) -> NodeId {
        let (ca, cb) = (self.constant(a), self.constant(b));
        if let (Some(x), Some(y)) = (ca, cb) {
            return self.konst(bin_value(op, x, y));
        }
        match (op, ca, cb) {
            (BinOp::Add, Some(z), _) if z == 0.0 => return b,
            (BinOp::Add | BinOp::Sub, _, Some(z)) if z == 0.0 => return a,
            (BinOp::Sub, Some(z), _) if z == 0.0 => return self.un(UnOp::Neg, b),
            (BinOp::Mul, Some(o), _) if o == 1.0 => return b,
            (BinOp::Mul | BinOp::Div, _, Some(o)) if o == 1.0 => return a,
            (BinOp::Mul, Some(z), _) | (BinOp::Mul, _, Some(z)) if z == 0.0 => {
                return self.konst(0.0);
            }
            (BinOp::Mul, Some(m), _) if m == -1.0 => return self.un(UnOp::Neg, b),
            (BinOp::Mul, _, Some(m)) if m == -1.0 => return self.un(UnOp::Neg, a),
            (BinOp::Pow, _, Some(o)) if o == 1.0 => return a,
            (BinOp::Pow, _, Some(z)) if z == 0.0 => return self.konst(1.0),
            (BinOp::Pow, _, Some(two)) if two == 2.0 => return self.bin(BinOp::Mul, a, a),
            _ => {}
        }
        if op == BinOp::Sub && a == b {
            return self.konst(0.0);
        }
        // Commutative operations get a canonical operand order for better CSE.
        let (a, b) = if matches!(op, BinOp::Add | BinOp::Mul | BinOp::Min | BinOp::Max) && a > b {
            (b, a)
        } else {
            (a, b)
        };
        self.intern(Op::Bin(op, a, b))
    }

    /// `select(c, a, b)`, folding a constant condition.
    pub fn select(&mut self, c: NodeId, a: NodeId, b: NodeId) -> NodeId {
        if let Some(x) = self.constant(c) {
            return if x != 0.0 { a } else { b };
        }
        if a == b {
            return a;
        }
        self.intern(Op::Select(c, a, b))
    }

    /// Compile an expression, asking `resolve` what each symbol means.
    pub fn add_expr(
        &mut self,
        e: &Expr,
        resolve: &mut dyn FnMut(Sym<'_>) -> Result<Resolved, String>,
    ) -> Result<NodeId, CompileError> {
        self.add_expr_depth(e, resolve, 0)
    }

    fn add_expr_depth(
        &mut self,
        e: &Expr,
        resolve: &mut dyn FnMut(Sym<'_>) -> Result<Resolved, String>,
        depth: usize,
    ) -> Result<NodeId, CompileError> {
        if depth > 64 {
            return Err(CompileError(
                "expression names refer to each other in a loop".into(),
            ));
        }
        let sym = match e {
            Expr::Num(v) => return Ok(self.konst(*v)),
            Expr::Name(n) => Sym::Name(n),
            Expr::V(a, b) => Sym::V(a, b.as_deref()),
            Expr::I(n) => Sym::I(n),
            Expr::Un(op, a) => {
                let a = self.add_expr_depth(a, resolve, depth)?;
                return Ok(self.un(*op, a));
            }
            Expr::Bin(op, a, b) => {
                let a = self.add_expr_depth(a, resolve, depth)?;
                let b = self.add_expr_depth(b, resolve, depth)?;
                return Ok(self.bin(*op, a, b));
            }
            Expr::Select(c, a, b) => {
                let c = self.add_expr_depth(c, resolve, depth)?;
                let a = self.add_expr_depth(a, resolve, depth)?;
                let b = self.add_expr_depth(b, resolve, depth)?;
                return Ok(self.select(c, a, b));
            }
        };
        match resolve(sym).map_err(CompileError)? {
            Resolved::Leaf(l) => Ok(self.leaf(l)),
            Resolved::Diff(a, b) => {
                let a = self.leaf(a);
                let b = self.leaf(b);
                Ok(self.bin(BinOp::Sub, a, b))
            }
            Resolved::Expr(inner) => self.add_expr_depth(&inner, resolve, depth + 1),
        }
    }

    /// Finish with the given outputs. Nodes no output depends on are dropped.
    pub fn finish(self, outputs: Vec<NodeId>) -> Tape {
        let mut live = vec![false; self.ops.len()];
        for &o in &outputs {
            live[o as usize] = true;
        }
        for i in (0..self.ops.len()).rev() {
            if !live[i] {
                continue;
            }
            match self.ops[i] {
                Op::Un(_, a) => live[a as usize] = true,
                Op::Bin(_, a, b) => {
                    live[a as usize] = true;
                    live[b as usize] = true;
                }
                Op::Select(c, a, b) => {
                    live[c as usize] = true;
                    live[a as usize] = true;
                    live[b as usize] = true;
                }
                _ => {}
            }
        }
        let mut remap = vec![u32::MAX; self.ops.len()];
        let mut ops = Vec::new();
        for (i, op) in self.ops.iter().enumerate() {
            if !live[i] {
                continue;
            }
            let r = |x: NodeId| remap[x as usize];
            let op = match *op {
                Op::Un(o, a) => Op::Un(o, r(a)),
                Op::Bin(o, a, b) => Op::Bin(o, r(a), r(b)),
                Op::Select(c, a, b) => Op::Select(r(c), r(a), r(b)),
                other => other,
            };
            remap[i] = ops.len() as u32;
            ops.push(op);
        }
        let outputs: Vec<u32> = outputs.iter().map(|&o| remap[o as usize]).collect();
        // Degree: 0 = independent of unknowns, 1 = affine in them, 2 = nonlinear.
        let mut degree = vec![0u8; ops.len()];
        for (i, op) in ops.iter().enumerate() {
            let d = |x: NodeId| degree[x as usize];
            degree[i] = match *op {
                Op::Const(_) | Op::Live(_) => 0,
                Op::Unknown(_) => 1,
                Op::Un(UnOp::Neg, a) => d(a),
                Op::Un(_, a) => {
                    if d(a) == 0 {
                        0
                    } else {
                        2
                    }
                }
                Op::Bin(BinOp::Add | BinOp::Sub, a, b) => d(a).max(d(b)),
                Op::Bin(BinOp::Mul, a, b) => {
                    if d(a) == 0 || d(b) == 0 {
                        d(a).max(d(b))
                    } else {
                        2
                    }
                }
                Op::Bin(BinOp::Div | BinOp::SafeDiv, a, b) => {
                    if d(b) == 0 {
                        d(a)
                    } else {
                        2
                    }
                }
                Op::Bin(_, a, b) => {
                    if d(a) == 0 && d(b) == 0 {
                        0
                    } else {
                        2
                    }
                }
                Op::Select(c, a, b) => {
                    if d(c) == 0 {
                        d(a).max(d(b))
                    } else {
                        2
                    }
                }
            };
        }
        let linear = outputs.iter().all(|&o| degree[o as usize] <= 1);
        let out_degree = outputs.iter().map(|&o| degree[o as usize]).collect();
        let n_live = ops
            .iter()
            .filter_map(|op| match op {
                Op::Live(i) => Some(*i as usize + 1),
                _ => None,
            })
            .max()
            .unwrap_or(0);
        let depends: Vec<bool> = degree.iter().map(|&d| d > 0).collect();
        Tape {
            n_unknowns: self.n_unknowns,
            n_live,
            ops,
            outputs,
            depends,
            linear,
            out_degree,
        }
    }
}

/// A compiled, immutable instruction tape. Evaluating it computes every output
/// and its partial derivatives with respect to every local unknown in one pass.
#[derive(Debug, Clone)]
pub struct Tape {
    n_unknowns: usize,
    n_live: usize,
    ops: Vec<Op>,
    outputs: Vec<u32>,
    depends: Vec<bool>,
    linear: bool,
    out_degree: Vec<u8>,
}

/// Scratch space reused across evaluations of a tape.
#[derive(Debug, Clone, Default)]
pub struct Workspace {
    val: Vec<f64>,
    grad: Vec<f64>,
}

impl Tape {
    /// Number of local unknowns derivatives are taken against.
    pub fn n_unknowns(&self) -> usize {
        self.n_unknowns
    }
    /// Number of live inputs the tape reads.
    pub fn n_live(&self) -> usize {
        self.n_live
    }
    /// Number of outputs.
    pub fn n_outputs(&self) -> usize {
        self.outputs.len()
    }
    /// Number of instructions.
    pub fn len(&self) -> usize {
        self.ops.len()
    }
    /// Whether the tape has no instructions.
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }
    /// True when every output is affine in the unknowns, so its Jacobian is
    /// constant while live inputs stay unchanged.
    pub fn is_linear(&self) -> bool {
        self.linear
    }

    /// Degree of output `k` in the unknowns: 0 = independent of them,
    /// 1 = affine, 2 = nonlinear.
    pub fn output_degree(&self, k: usize) -> u8 {
        self.out_degree[k]
    }

    /// Evaluate values and derivatives.
    pub fn eval(&self, x: &[f64], live: &[f64], ws: &mut Workspace) {
        let n = self.n_unknowns;
        let len = self.ops.len();
        ws.val.clear();
        ws.val.resize(len, 0.0);
        ws.grad.clear();
        ws.grad.resize(len * n, 0.0);
        for i in 0..len {
            let (val, rest) = ws.val.split_at_mut(i);
            let (grad_before, grad_rest) = ws.grad.split_at_mut(i * n);
            let g = &mut grad_rest[..n];
            let v = match self.ops[i] {
                Op::Const(c) => c,
                Op::Unknown(k) => {
                    g[k as usize] = 1.0;
                    x[k as usize]
                }
                Op::Live(k) => live.get(k as usize).copied().unwrap_or(0.0),
                Op::Un(op, a) => {
                    let av = val[a as usize];
                    let v = un_value(op, av);
                    if self.depends[i] {
                        let s = un_slope(op, av, v);
                        let ga = &grad_before[a as usize * n..a as usize * n + n];
                        for (gk, gak) in g.iter_mut().zip(ga) {
                            *gk = s * gak;
                        }
                    }
                    v
                }
                Op::Bin(op, a, b) => {
                    let (av, bv) = (val[a as usize], val[b as usize]);
                    let v = bin_value(op, av, bv);
                    if self.depends[i] {
                        let (sa, sb) = bin_slopes(op, av, bv, v);
                        let ga = &grad_before[a as usize * n..a as usize * n + n];
                        let gb = &grad_before[b as usize * n..b as usize * n + n];
                        for k in 0..n {
                            let mut d = 0.0;
                            if sa != 0.0 {
                                d += sa * ga[k];
                            }
                            if sb != 0.0 {
                                d += sb * gb[k];
                            }
                            g[k] = d;
                        }
                    }
                    v
                }
                Op::Select(c, a, b) => {
                    let pick = if val[c as usize] != 0.0 { a } else { b };
                    if self.depends[i] {
                        g.copy_from_slice(&grad_before[pick as usize * n..pick as usize * n + n]);
                    }
                    val[pick as usize]
                }
            };
            rest[0] = v;
        }
    }

    /// Value of output `k` after [`Tape::eval`].
    pub fn value(&self, ws: &Workspace, k: usize) -> f64 {
        ws.val[self.outputs[k] as usize]
    }

    /// Partial derivatives of output `k` after [`Tape::eval`].
    pub fn grad<'w>(&self, ws: &'w Workspace, k: usize) -> &'w [f64] {
        let n = self.n_unknowns;
        let o = self.outputs[k] as usize;
        &ws.grad[o * n..o * n + n]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    fn compile(src: &str, names: &[&str]) -> Tape {
        let e = parse(src).unwrap();
        let mut b = TapeBuilder::new(names.len());
        let id = b
            .add_expr(&e, &mut |s| match s {
                Sym::Name(n) => names
                    .iter()
                    .position(|m| *m == n)
                    .map(|i| Resolved::Leaf(Leaf::Unknown(i)))
                    .ok_or_else(|| format!("unknown {n}")),
                _ => Err("no voltages".into()),
            })
            .unwrap();
        b.finish(vec![id])
    }

    #[test]
    fn folds_constants_and_shares_subexpressions() {
        let t = compile("(2 * 3 + x) * (6 + x) + 0 * y", &["x", "y"]);
        // x, 6, 6+x, mul: four instructions after folding and CSE.
        assert_eq!(t.len(), 4);
        let mut ws = Workspace::default();
        t.eval(&[2.0, 5.0], &[], &mut ws);
        assert_eq!(t.value(&ws, 0), 64.0);
        assert_eq!(t.grad(&ws, 0), &[16.0, 0.0]);
    }

    #[test]
    fn linearity_detection() {
        assert!(compile("3 * x - y / 2", &["x", "y"]).is_linear());
        assert!(!compile("x * y", &["x", "y"]).is_linear());
        assert!(!compile("exp(x)", &["x"]).is_linear());
    }

    #[test]
    fn limexp_continues_linearly() {
        let t = compile("limexp(x)", &["x"]);
        let mut ws = Workspace::default();
        t.eval(&[100.0], &[], &mut ws);
        let k = 80f64.exp();
        assert!((t.value(&ws, 0) - k * 21.0).abs() / (k * 21.0) < 1e-12);
        assert!((t.grad(&ws, 0)[0] - k).abs() / k < 1e-12);
    }
}
