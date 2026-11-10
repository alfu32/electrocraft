//! Expression language shared by components, scripts and the inspector.
//!
//! An [`Expr`] is a syntax tree built by [`parse`] (or by hand). Expressions are
//! never interpreted as strings at run time: a [`TapeBuilder`] compiles them into a
//! flat [`Tape`] with constant folding, common subexpression elimination
//! (hash-consing) and forward-mode automatic differentiation with respect to the
//! component's local unknowns.

mod parse;
mod tape;

pub use parse::{ParseError, parse};
pub use tape::{CompileError, Leaf, NodeId, Resolved, Sym, Tape, TapeBuilder, Workspace};

/// Unary operations and one-argument functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnOp {
    Neg,
    Exp,
    /// Exponential that continues linearly beyond 80, so junctions cannot overflow.
    Limexp,
    Ln,
    Log10,
    Sqrt,
    /// `sqrt(max(x, 0))`.
    SafeSqrt,
    Sin,
    Cos,
    Tan,
    Atan,
    Tanh,
    Sinh,
    Cosh,
    Abs,
    Floor,
    /// Logical not: 1 when the argument is 0, else 0.
    Not,
}

/// Binary operations and two-argument functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    Min,
    Max,
    /// Division that replaces a denominator smaller than 1e-30 in magnitude.
    SafeDiv,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    And,
    Or,
}

/// Expression syntax tree.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// A number.
    Num(f64),
    /// A bare name: parameter, state variable, `let` name, `t`, `temp` or `vt`.
    Name(String),
    /// `V(a)` or `V(a, b)`: node voltage or voltage difference.
    V(String, Option<String>),
    /// `I(branch)`: a branch current unknown.
    I(String),
    Un(UnOp, Box<Expr>),
    Bin(BinOp, Box<Expr>, Box<Expr>),
    /// `select(cond, a, b)`: `a` when `cond` is non-zero, else `b`.
    Select(Box<Expr>, Box<Expr>, Box<Expr>),
}

impl Expr {
    /// A number literal.
    pub fn num(v: f64) -> Self {
        Expr::Num(v)
    }
    /// A bare name.
    pub fn name(n: &str) -> Self {
        Expr::Name(n.to_string())
    }
    /// `V(a)` or `V(a, b)`.
    pub fn v(a: &str, b: Option<&str>) -> Self {
        Expr::V(a.to_string(), b.map(str::to_string))
    }
    /// Apply a unary operation.
    pub fn un(op: UnOp, a: Expr) -> Self {
        Expr::Un(op, Box::new(a))
    }
    /// Apply a binary operation.
    pub fn bin(op: BinOp, a: Expr, b: Expr) -> Self {
        Expr::Bin(op, Box::new(a), Box::new(b))
    }

    /// Visit every leaf name, voltage and current reference.
    pub fn visit(&self, f: &mut dyn FnMut(&Expr)) {
        match self {
            Expr::Num(_) | Expr::Name(_) | Expr::V(..) | Expr::I(_) => f(self),
            Expr::Un(_, a) => a.visit(f),
            Expr::Bin(_, a, b) => {
                a.visit(f);
                b.visit(f);
            }
            Expr::Select(c, a, b) => {
                c.visit(f);
                a.visit(f);
                b.visit(f);
            }
        }
    }
}

impl std::ops::Add for Expr {
    type Output = Expr;
    fn add(self, rhs: Expr) -> Expr {
        Expr::bin(BinOp::Add, self, rhs)
    }
}
impl std::ops::Sub for Expr {
    type Output = Expr;
    fn sub(self, rhs: Expr) -> Expr {
        Expr::bin(BinOp::Sub, self, rhs)
    }
}
impl std::ops::Mul for Expr {
    type Output = Expr;
    fn mul(self, rhs: Expr) -> Expr {
        Expr::bin(BinOp::Mul, self, rhs)
    }
}
impl std::ops::Div for Expr {
    type Output = Expr;
    fn div(self, rhs: Expr) -> Expr {
        Expr::bin(BinOp::Div, self, rhs)
    }
}
impl std::ops::Neg for Expr {
    type Output = Expr;
    fn neg(self) -> Expr {
        Expr::un(UnOp::Neg, self)
    }
}

/// Evaluate an expression whose leaves are all names, using `lookup`.
/// Voltages and currents evaluate through `lookup` too, as `V(a)`, `V(a,b)`, `I(x)`.
pub fn eval_with(e: &Expr, lookup: &dyn Fn(&str) -> Option<f64>) -> Result<f64, CompileError> {
    let mut b = TapeBuilder::new(0);
    let id = b.add_expr(e, &mut |s| {
        let key = match s {
            Sym::Name(n) => n.to_string(),
            Sym::V(a, None) => format!("V({a})"),
            Sym::V(a, Some(c)) => format!("V({a},{c})"),
            Sym::I(n) => format!("I({n})"),
        };
        lookup(&key)
            .map(|v| Resolved::Leaf(Leaf::Const(v)))
            .ok_or_else(|| format!("unknown name {key}"))
    })?;
    let tape = b.finish(vec![id]);
    let mut ws = Workspace::default();
    tape.eval(&[], &[], &mut ws);
    Ok(tape.value(&ws, 0))
}

impl UnOp {
    /// Function name in the expression language (`-` for negation, `!` for not).
    pub fn name(self) -> &'static str {
        match self {
            UnOp::Neg => "-",
            UnOp::Exp => "exp",
            UnOp::Limexp => "limexp",
            UnOp::Ln => "ln",
            UnOp::Log10 => "log10",
            UnOp::Sqrt => "sqrt",
            UnOp::SafeSqrt => "safe_sqrt",
            UnOp::Sin => "sin",
            UnOp::Cos => "cos",
            UnOp::Tan => "tan",
            UnOp::Atan => "atan",
            UnOp::Tanh => "tanh",
            UnOp::Sinh => "sinh",
            UnOp::Cosh => "cosh",
            UnOp::Abs => "abs",
            UnOp::Floor => "floor",
            UnOp::Not => "!",
        }
    }
}

/// Prints source text that [`parse`] reads back to the same tree.
impl std::fmt::Display for Expr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Expr::Num(v) => {
                if *v < 0.0 {
                    write!(f, "(-{})", -v)
                } else {
                    write!(f, "{v}")
                }
            }
            Expr::Name(n) => f.write_str(n),
            Expr::V(a, None) => write!(f, "V({a})"),
            Expr::V(a, Some(b)) => write!(f, "V({a}, {b})"),
            Expr::I(n) => write!(f, "I({n})"),
            Expr::Un(UnOp::Neg, a) => write!(f, "(-{a})"),
            Expr::Un(UnOp::Not, a) => write!(f, "(!{a})"),
            Expr::Un(op, a) => write!(f, "{}({a})", op.name()),
            Expr::Bin(op, a, b) => {
                let sym = match op {
                    BinOp::Add => "+",
                    BinOp::Sub => "-",
                    BinOp::Mul => "*",
                    BinOp::Div => "/",
                    BinOp::Pow => "^",
                    BinOp::Lt => "<",
                    BinOp::Le => "<=",
                    BinOp::Gt => ">",
                    BinOp::Ge => ">=",
                    BinOp::Eq => "==",
                    BinOp::Ne => "!=",
                    BinOp::And => "&&",
                    BinOp::Or => "||",
                    BinOp::Min => return write!(f, "min({a}, {b})"),
                    BinOp::Max => return write!(f, "max({a}, {b})"),
                    BinOp::SafeDiv => return write!(f, "safe_div({a}, {b})"),
                };
                write!(f, "({a} {sym} {b})")
            }
            Expr::Select(c, a, b) => write!(f, "select({c}, {a}, {b})"),
        }
    }
}

#[cfg(test)]
mod print_tests {
    #[test]
    fn printing_round_trips() {
        for src in [
            "Is * (limexp(V(A, K) / (N * vt)) - 1)",
            "select(closed > 0.5, 1 / Ron, 1 / Roff) * V(A, B)",
            "-x ^ 2 + min(a, b) - max(1e-12, I(L)) * 2.5e6",
            "!(a < b) || c >= 3 && d != -4",
            "smoothstep(0, 1, x)",
        ] {
            let e = crate::parse(src).unwrap();
            let again = crate::parse(&e.to_string()).unwrap_or_else(|err| panic!("{e}: {err}"));
            assert_eq!(e, again, "{src} -> {e}");
        }
    }
}
