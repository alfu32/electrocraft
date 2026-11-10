//! Plain JavaScript functions as equations (spec 6.1).
//!
//! A function passed where an expression is expected is not run: its source
//! is compiled into the expression language, so it is differentiated and
//! evaluated natively like an `e`-template. The accepted subset is what a
//! formula needs: `const`/`let` bindings and reassignment, `if`/`else` and
//! `return` (folded into `select`), the conditional operator, arithmetic,
//! comparisons, `&&`, `||`, `!`, `**`, `Math.*`, the expression functions
//! (`limexp`, `select`, `smoothstep`...), `V("A", "K")`, `I("L")`, and calls
//! to other functions declared at the top of the file, which are inlined.
//! The function's parameters name component variables (parameters, states,
//! `c.let` names, `vt`, `temp`, `t`), as identifiers or destructured
//! (`({ Is, N }) => ...`).

use std::collections::HashMap;

/// A compile error with a position in the component file.
#[derive(Debug, Clone, PartialEq)]
pub struct FnError {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    Num(f64),
    Str(String),
    Punct(&'static str),
    /// A template literal or regular expression: not part of the subset.
    Other,
}

#[derive(Debug, Clone)]
struct Token {
    tok: Tok,
    /// Byte offset in the text.
    at: usize,
}

const PUNCTS: [&str; 41] = [
    "===", "!==", "**=", "...", "=>", "==", "!=", "<=", ">=", "&&", "||", "**", "+=", "-=", "*=",
    "/=", "??", "?.", "++", "--", "(", ")", "{", "}", "[", "]", ",", ";", ".", ":", "?", "+", "-",
    "*", "/", "%", "<", ">", "=", "!", "&",
];

fn tokenize(src: &str) -> Vec<Token> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
        } else if src[i..].starts_with("//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if src[i..].starts_with("/*") {
            i = src[i + 2..].find("*/").map_or(b.len(), |k| i + 2 + k + 2);
        } else if c == b'"' || c == b'\'' {
            let start = i;
            i += 1;
            let mut s = String::new();
            while i < b.len() && b[i] != c && b[i] != b'\n' {
                if b[i] == b'\\' && i + 1 < b.len() {
                    i += 1;
                }
                let ch = src[i..].chars().next().unwrap_or(' ');
                s.push(ch);
                i += ch.len_utf8();
            }
            i += 1;
            out.push(Token {
                tok: Tok::Str(s),
                at: start,
            });
        } else if c == b'`' {
            // Template literal, with ${} nesting.
            let start = i;
            i += 1;
            let mut depth = 0;
            while i < b.len() {
                if b[i] == b'\\' {
                    i += 2;
                    continue;
                }
                if depth == 0 && b[i] == b'`' {
                    i += 1;
                    break;
                }
                if src[i..].starts_with("${") {
                    depth += 1;
                    i += 2;
                    continue;
                }
                if depth > 0 && b[i] == b'}' {
                    depth -= 1;
                }
                i += 1;
            }
            out.push(Token {
                tok: Tok::Other,
                at: start,
            });
        } else if c.is_ascii_digit() || (c == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit))
        {
            let start = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'.' || b[i] == b'_') {
                if (b[i] == b'e' || b[i] == b'E') && matches!(b.get(i + 1), Some(b'+' | b'-')) {
                    i += 1;
                }
                i += 1;
            }
            let text: String = src[start..i].chars().filter(|c| *c != '_').collect();
            let v = if let Some(h) = text.strip_prefix("0x") {
                i64::from_str_radix(h, 16)
                    .map(|v| v as f64)
                    .unwrap_or(f64::NAN)
            } else {
                text.parse().unwrap_or(f64::NAN)
            };
            out.push(Token {
                tok: Tok::Num(v),
                at: start,
            });
        } else if c.is_ascii_alphabetic() || c == b'_' || c == b'$' || c >= 0x80 {
            let start = i;
            while i < b.len()
                && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'$' || b[i] >= 0x80)
            {
                i += 1;
            }
            out.push(Token {
                tok: Tok::Ident(src[start..i].to_string()),
                at: start,
            });
        } else {
            match PUNCTS.iter().find(|p| src[i..].starts_with(**p)) {
                Some(p) => {
                    out.push(Token {
                        tok: Tok::Punct(p),
                        at: i,
                    });
                    i += p.len();
                }
                None => {
                    out.push(Token {
                        tok: Tok::Other,
                        at: i,
                    });
                    i += src[i..].chars().next().map_or(1, char::len_utf8);
                }
            }
        }
    }
    out
}

#[derive(Debug, Clone)]
enum Ex {
    Num(f64),
    Str(String),
    Name(String, usize),
    Un(&'static str, Box<Ex>),
    Bin(&'static str, Box<Ex>, Box<Ex>),
    Cond(Box<Ex>, Box<Ex>, Box<Ex>),
    /// Callee (a name or `Math.x`), arguments, position.
    Call(String, Vec<Ex>, usize),
}

#[derive(Debug, Clone)]
enum St {
    Bind(String, Ex),
    Assign(String, &'static str, Ex, usize),
    Return(Ex),
    If(Ex, Vec<St>, Vec<St>),
}

#[derive(Debug, Clone)]
struct Func {
    params: Vec<String>,
    body: Vec<St>,
}

struct Parser<'a> {
    toks: &'a [Token],
    k: usize,
    /// Offset of the first token in the file, and the file text, for positions.
    base: usize,
    file: &'a str,
}

type PResult<T> = Result<T, FnError>;

fn position(file: &str, at: usize) -> (usize, usize) {
    let before = &file[..at.min(file.len())];
    let line = before.matches('\n').count() + 1;
    let column = before.len() - before.rfind('\n').map_or(0, |k| k + 1) + 1;
    (line, column)
}

impl Parser<'_> {
    fn err<T>(&self, at: usize, message: impl Into<String>) -> PResult<T> {
        let (line, column) = position(self.file, self.base + at);
        Err(FnError {
            line,
            column,
            message: message.into(),
        })
    }
    fn here(&self) -> usize {
        self.toks
            .get(self.k)
            .map_or_else(|| self.toks.last().map_or(0, |t| t.at), |t| t.at)
    }
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.k).map(|t| &t.tok)
    }
    fn is(&self, p: &str) -> bool {
        matches!(self.peek(), Some(Tok::Punct(q)) if *q == p)
    }
    fn is_word(&self, w: &str) -> bool {
        matches!(self.peek(), Some(Tok::Ident(q)) if q == w)
    }
    fn eat(&mut self, p: &str) -> bool {
        if self.is(p) {
            self.k += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, p: &str) -> PResult<()> {
        if self.eat(p) {
            Ok(())
        } else {
            let at = self.here();
            self.err(at, format!("expected `{p}`"))
        }
    }
    fn ident(&mut self) -> PResult<String> {
        match self.peek() {
            Some(Tok::Ident(n)) => {
                let n = n.clone();
                self.k += 1;
                Ok(n)
            }
            _ => {
                let at = self.here();
                self.err(at, "expected a name")
            }
        }
    }

    /// `function f(a, b) {...}`, `(a, b) => ...`, `a => ...`, `f(a) {...}`.
    fn function(&mut self) -> PResult<Func> {
        if self.is_word("async") {
            let at = self.here();
            return self.err(at, "equation functions cannot be async");
        }
        if self.is_word("function") {
            self.k += 1;
            if self.eat("*") {
                let at = self.here();
                return self.err(at, "equation functions cannot be generators");
            }
        }
        let params = if let Some(Tok::Ident(_)) = self.peek() {
            let n = self.ident()?;
            if self.is("=>") {
                vec![n]
            } else {
                self.params()?
            }
        } else {
            self.params()?
        };
        let body = if self.eat("=>") {
            if self.is("{") {
                self.block()?
            } else {
                vec![St::Return(self.expr()?)]
            }
        } else {
            self.block()?
        };
        Ok(Func { params, body })
    }

    fn params(&mut self) -> PResult<Vec<String>> {
        self.expect("(")?;
        let mut out = Vec::new();
        while !self.eat(")") {
            if self.eat("{") {
                while !self.eat("}") {
                    let key = self.ident()?;
                    let name = if self.eat(":") {
                        self.ident()?
                    } else {
                        key.clone()
                    };
                    if name != key {
                        let at = self.here();
                        return self.err(
                            at,
                            format!("rename `{key}: {name}` is not supported; use {key}"),
                        );
                    }
                    if self.eat("=") {
                        self.expr()?;
                    }
                    out.push(name);
                    self.eat(",");
                }
            } else {
                out.push(self.ident()?);
                if self.eat("=") {
                    self.expr()?;
                }
            }
            if !self.eat(",") {
                self.expect(")")?;
                break;
            }
        }
        Ok(out)
    }

    fn block(&mut self) -> PResult<Vec<St>> {
        self.expect("{")?;
        let mut out = Vec::new();
        while !self.eat("}") {
            if self.k >= self.toks.len() {
                let at = self.here();
                return self.err(at, "missing `}`");
            }
            out.extend(self.statement()?);
        }
        Ok(out)
    }

    fn statement(&mut self) -> PResult<Vec<St>> {
        let at = self.here();
        if self.eat(";") {
            return Ok(Vec::new());
        }
        if self.is("{") {
            return self.block();
        }
        match self.peek() {
            Some(Tok::Ident(w)) if matches!(w.as_str(), "const" | "let" | "var") => {
                self.k += 1;
                let mut out = Vec::new();
                loop {
                    let name = self.ident()?;
                    self.expect("=")?;
                    out.push(St::Bind(name, self.expr()?));
                    if !self.eat(",") {
                        break;
                    }
                }
                self.eat(";");
                Ok(out)
            }
            Some(Tok::Ident(w)) if w == "return" => {
                self.k += 1;
                let e = self.expr()?;
                self.eat(";");
                Ok(vec![St::Return(e)])
            }
            Some(Tok::Ident(w)) if w == "if" => {
                self.k += 1;
                self.expect("(")?;
                let c = self.expr()?;
                self.expect(")")?;
                let then = self.statement()?;
                let other = if self.is_word("else") {
                    self.k += 1;
                    self.statement()?
                } else {
                    Vec::new()
                };
                Ok(vec![St::If(c, then, other)])
            }
            Some(Tok::Ident(w))
                if matches!(
                    w.as_str(),
                    "for" | "while" | "do" | "switch" | "try" | "throw" | "function" | "class"
                ) =>
            {
                let w = w.clone();
                self.err(at, format!("`{w}` is not available in equation functions (use const, if/else, return and helper functions)"))
            }
            Some(Tok::Ident(_))
                if matches!(
                    self.toks.get(self.k + 1).map(|t| &t.tok),
                    Some(Tok::Punct("=" | "+=" | "-=" | "*=" | "/=" | "**="))
                ) =>
            {
                let name = self.ident()?;
                let Some(Tok::Punct(op)) = self.peek().cloned() else {
                    unreachable!()
                };
                self.k += 1;
                let e = self.expr()?;
                self.eat(";");
                Ok(vec![St::Assign(name, op, e, at)])
            }
            _ => self.err(at, "expected const, let, if, return or an assignment"),
        }
    }

    fn expr(&mut self) -> PResult<Ex> {
        let c = self.binary(0)?;
        if self.eat("?") {
            let a = self.expr()?;
            self.expect(":")?;
            let b = self.expr()?;
            return Ok(Ex::Cond(Box::new(c), Box::new(a), Box::new(b)));
        }
        Ok(c)
    }

    fn binary(&mut self, min: u8) -> PResult<Ex> {
        let mut lhs = self.unary()?;
        while let Some(Tok::Punct(op)) = self.peek().cloned() {
            let prec = match op {
                "||" | "??" => 1,
                "&&" => 2,
                "==" | "===" | "!=" | "!==" => 3,
                "<" | "<=" | ">" | ">=" => 4,
                "+" | "-" => 5,
                "*" | "/" | "%" => 6,
                "**" => 7,
                _ => break,
            };
            if prec < min.max(1) {
                break;
            }
            self.k += 1;
            // `**` is right-associative.
            let rhs = if op == "**" {
                self.binary(prec)?
            } else {
                self.binary(prec + 1)?
            };
            lhs = Ex::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> PResult<Ex> {
        for op in ["-", "+", "!"] {
            if self.eat(op) {
                let a = self.unary()?;
                return Ok(Ex::Un(op, Box::new(a)));
            }
        }
        self.postfix()
    }

    fn postfix(&mut self) -> PResult<Ex> {
        let at = self.here();
        let mut e = match self.peek().cloned() {
            Some(Tok::Num(v)) => {
                self.k += 1;
                Ex::Num(v)
            }
            Some(Tok::Str(s)) => {
                self.k += 1;
                Ex::Str(s)
            }
            Some(Tok::Ident(n)) => {
                self.k += 1;
                match n.as_str() {
                    "true" => Ex::Num(1.0),
                    "false" => Ex::Num(0.0),
                    _ => Ex::Name(n, at),
                }
            }
            Some(Tok::Punct("(")) => {
                self.k += 1;
                let e = self.expr()?;
                self.expect(")")?;
                e
            }
            Some(Tok::Other) => return self.err(
                at,
                "template literals belong in e`...`; inside a function write the formula directly",
            ),
            _ => return self.err(at, "expected a value"),
        };
        loop {
            if self.eat(".") {
                let member = self.ident()?;
                e = match e {
                    Ex::Name(obj, p) => Ex::Name(format!("{obj}.{member}"), p),
                    _ => return self.err(at, "only Math.name members are available"),
                };
            } else if self.is("(") {
                self.k += 1;
                let mut args = Vec::new();
                while !self.eat(")") {
                    args.push(self.expr()?);
                    if !self.eat(",") {
                        self.expect(")")?;
                        break;
                    }
                }
                e = match e {
                    Ex::Name(callee, p) => Ex::Call(callee, args, p),
                    _ => return self.err(at, "only named functions can be called"),
                };
            } else if self.is("[") {
                return self.err(at, "indexing is not available in equation functions");
            } else {
                break;
            }
        }
        Ok(e)
    }
}

/// Top-level `function name(...)` declarations and `const name = (...) =>`
/// arrow functions of a component file: token index where each starts.
fn helper_starts(toks: &[Token]) -> HashMap<String, usize> {
    let mut out = HashMap::new();
    let mut depth = 0i32;
    let mut k = 0;
    while k < toks.len() {
        match &toks[k].tok {
            Tok::Punct("{" | "(" | "[") => depth += 1,
            Tok::Punct("}" | ")" | "]") => depth -= 1,
            Tok::Ident(w) if depth == 0 && w == "function" => {
                if let Some(Tok::Ident(name)) = toks.get(k + 1).map(|t| &t.tok) {
                    out.insert(name.clone(), k);
                }
            }
            Tok::Ident(w) if depth == 0 && matches!(w.as_str(), "const" | "let" | "var") => {
                if let (Some(Tok::Ident(name)), Some(Tok::Punct("="))) = (
                    toks.get(k + 1).map(|t| &t.tok),
                    toks.get(k + 2).map(|t| &t.tok),
                ) {
                    // `const f = function (...)`, `const f = (...) =>`, `const f = x =>`.
                    let is_fn = match toks.get(k + 3).map(|t| &t.tok) {
                        Some(Tok::Ident(w)) if w == "function" => true,
                        Some(Tok::Ident(_)) => {
                            matches!(toks.get(k + 4).map(|t| &t.tok), Some(Tok::Punct("=>")))
                        }
                        Some(Tok::Punct("(")) => {
                            // Find the matching ")" and check for "=>".
                            let mut d = 0;
                            let mut j = k + 3;
                            while j < toks.len() {
                                match toks[j].tok {
                                    Tok::Punct("(") => d += 1,
                                    Tok::Punct(")") => {
                                        d -= 1;
                                        if d == 0 {
                                            break;
                                        }
                                    }
                                    _ => {}
                                }
                                j += 1;
                            }
                            matches!(toks.get(j + 1).map(|t| &t.tok), Some(Tok::Punct("=>")))
                        }
                        _ => false,
                    };
                    if is_fn {
                        out.insert(name.clone(), k + 3);
                    }
                }
            }
            _ => {}
        }
        k += 1;
    }
    out
}

/// Expression functions passed through by name.
const EXPR_FUNCS: [&str; 22] = [
    "exp",
    "limexp",
    "ln",
    "log",
    "log10",
    "sqrt",
    "safe_sqrt",
    "sin",
    "cos",
    "tan",
    "atan",
    "tanh",
    "sinh",
    "cosh",
    "abs",
    "floor",
    "pow",
    "min",
    "max",
    "safe_div",
    "smoothstep",
    "smooth_abs",
];

fn math(name: &str) -> Option<&'static str> {
    Some(match name {
        "exp" => "exp",
        "log" => "ln",
        "log10" => "log10",
        "sqrt" => "sqrt",
        "abs" => "abs",
        "sin" => "sin",
        "cos" => "cos",
        "tan" => "tan",
        "atan" => "atan",
        "tanh" => "tanh",
        "sinh" => "sinh",
        "cosh" => "cosh",
        "floor" => "floor",
        "pow" => "pow",
        "min" => "min",
        "max" => "max",
        _ => return None,
    })
}

struct Compiler<'a> {
    toks: &'a [Token],
    file: &'a str,
    helpers: HashMap<String, usize>,
    parsed: HashMap<String, Func>,
}

/// Values in scope: expression text for locals and arguments.
type Env = HashMap<String, String>;

impl Compiler<'_> {
    fn err<T>(&self, at: usize, message: impl Into<String>) -> Result<T, FnError> {
        let (line, column) = position(self.file, at);
        Err(FnError {
            line,
            column,
            message: message.into(),
        })
    }

    fn helper(&mut self, name: &str) -> Option<Result<Func, FnError>> {
        if let Some(f) = self.parsed.get(name) {
            return Some(Ok(f.clone()));
        }
        let start = *self.helpers.get(name)?;
        let mut p = Parser {
            toks: self.toks,
            k: start,
            base: 0,
            file: self.file,
        };
        // Skip `function name` so `function()` parsing sees the parameters.
        if matches!(&self.toks[start].tok, Tok::Ident(w) if w == "function") {
            p.k += 2;
        }
        let f = p.function();
        if let Ok(f) = &f {
            self.parsed.insert(name.to_string(), f.clone());
        }
        Some(f)
    }

    fn body(&mut self, stmts: &[St], env: &Env, depth: usize) -> Result<Option<String>, FnError> {
        let mut env = env.clone();
        for (k, st) in stmts.iter().enumerate() {
            match st {
                St::Bind(name, e) => {
                    let v = self.expr(e, &env, depth)?;
                    env.insert(name.clone(), v);
                }
                St::Assign(name, op, e, at) => {
                    let Some(old) = env.get(name).cloned() else {
                        return self.err(
                            *at,
                            format!("{name} is not a local variable; declare it with let"),
                        );
                    };
                    let v = self.expr(e, &env, depth)?;
                    let new = match *op {
                        "=" => v,
                        "+=" => format!("({old} + {v})"),
                        "-=" => format!("({old} - {v})"),
                        "*=" => format!("({old} * {v})"),
                        "/=" => format!("({old} / {v})"),
                        _ => format!("({old} ^ {v})"),
                    };
                    env.insert(name.clone(), new);
                }
                St::Return(e) => return self.expr(e, &env, depth).map(Some),
                St::If(c, then, other) => {
                    // Each branch continues with the statements after the if.
                    let rest = &stmts[k + 1..];
                    let cond = self.expr(c, &env, depth)?;
                    let a: Vec<St> = then.iter().chain(rest).cloned().collect();
                    let b: Vec<St> = other.iter().chain(rest).cloned().collect();
                    let a = self.body(&a, &env, depth)?;
                    let b = self.body(&b, &env, depth)?;
                    return Ok(match (a, b) {
                        (Some(a), Some(b)) => Some(format!("select({cond}, {a}, {b})")),
                        _ => None,
                    });
                }
            }
        }
        Ok(None)
    }

    fn expr(&mut self, e: &Ex, env: &Env, depth: usize) -> Result<String, FnError> {
        Ok(match e {
            Ex::Num(v) => {
                if !v.is_finite() {
                    return Err(FnError {
                        line: 0,
                        column: 0,
                        message: "numbers must be finite".into(),
                    });
                }
                if *v < 0.0 {
                    format!("({v:e})")
                } else {
                    format!("{v:e}")
                }
            }
            Ex::Str(s) => {
                return Err(FnError {
                    line: 0,
                    column: 0,
                    message: format!(
                        "text \"{s}\" can only be a pin name in V(...) or a branch in I(...)"
                    ),
                });
            }
            Ex::Name(n, at) => match n.as_str() {
                _ if env.contains_key(n) => env[n].clone(),
                "Math.PI" => "pi".into(),
                "Math.E" => format!("{:e}", std::f64::consts::E),
                "Infinity" | "NaN" | "undefined" | "null" => {
                    return self.err(*at, format!("{n} cannot be part of an equation"));
                }
                _ if n.contains('.') => {
                    return self.err(
                        *at,
                        format!("unknown {n}; only Math.PI and Math.E are available"),
                    );
                }
                _ => n.clone(),
            },
            Ex::Un(op, a) => {
                let a = self.expr(a, env, depth)?;
                match *op {
                    "-" => format!("(-{a})"),
                    "!" => format!("(!{a})"),
                    _ => a,
                }
            }
            Ex::Bin(op, a, b) => {
                let a = self.expr(a, env, depth)?;
                let b = self.expr(b, env, depth)?;
                let sym = match *op {
                    "**" => "^",
                    "===" | "==" => "==",
                    "!==" | "!=" => "!=",
                    "??" => {
                        return Err(FnError {
                            line: 0,
                            column: 0,
                            message: "?? is not available in equations".into(),
                        });
                    }
                    "%" => return Ok(format!("({a} - {b} * floor({a} / {b}))")),
                    s => s,
                };
                format!("({a} {sym} {b})")
            }
            Ex::Cond(c, a, b) => format!(
                "select({}, {}, {})",
                self.expr(c, env, depth)?,
                self.expr(a, env, depth)?,
                self.expr(b, env, depth)?
            ),
            Ex::Call(callee, args, at) => self.call(callee, args, *at, env, depth)?,
        })
    }

    fn call(
        &mut self,
        callee: &str,
        args: &[Ex],
        at: usize,
        env: &Env,
        depth: usize,
    ) -> Result<String, FnError> {
        let strings = |k: usize| -> Option<&str> {
            match args.get(k) {
                Some(Ex::Str(s)) => Some(s.as_str()),
                _ => None,
            }
        };
        match callee {
            "V" => {
                return match (args.len(), strings(0), strings(1)) {
                    (1, Some(a), _) => Ok(format!("V({a})")),
                    (2, Some(a), Some(b)) => Ok(format!("V({a}, {b})")),
                    _ => self.err(
                        at,
                        "write V(\"A\") or V(\"A\", \"K\") with pin or node names in quotes",
                    ),
                };
            }
            "I" => {
                return match (args.len(), strings(0)) {
                    (1, Some(a)) => Ok(format!("I({a})")),
                    _ => self.err(at, "write I(\"name\") with the branch name in quotes"),
                };
            }
            _ => {}
        }
        let mut vals = Vec::new();
        for a in args {
            vals.push(self.expr(a, env, depth).map_err(|e| {
                if e.line == 0 {
                    FnError {
                        line: position(self.file, at).0,
                        column: position(self.file, at).1,
                        ..e
                    }
                } else {
                    e
                }
            })?);
        }
        if let Some(m) = callee.strip_prefix("Math.") {
            let Some(f) = math(m) else {
                return self.err(at, format!("Math.{m} is not available in equations"));
            };
            return Ok(match (f, vals.len()) {
                ("min" | "max", n) if n >= 2 => vals
                    .into_iter()
                    .reduce(|a, b| format!("{f}({a}, {b})"))
                    .unwrap_or_default(),
                (_, _) => format!("{f}({})", vals.join(", ")),
            });
        }
        if callee == "select" {
            return Ok(format!("select({})", vals.join(", ")));
        }
        if EXPR_FUNCS.contains(&callee) {
            return Ok(match (callee, vals.len()) {
                ("min" | "max", n) if n > 2 => vals
                    .into_iter()
                    .reduce(|a, b| format!("{callee}({a}, {b})"))
                    .unwrap_or_default(),
                _ => format!("{callee}({})", vals.join(", ")),
            });
        }
        // A helper declared in the file: inline it.
        match self.helper(callee) {
            Some(Ok(f)) => {
                if depth > 32 {
                    return self.err(at, format!("{callee} calls itself; recursion is not available in equations"));
                }
                if f.params.len() != vals.len() {
                    return self.err(at, format!("{callee} takes {} arguments, got {}", f.params.len(), vals.len()));
                }
                let inner: Env = f.params.iter().cloned().zip(vals).collect();
                match self.body(&f.body, &inner, depth + 1)? {
                    Some(v) => Ok(v),
                    None => self.err(at, format!("{callee} must return a value on every path")),
                }
            }
            Some(Err(e)) => Err(FnError { message: format!("in {callee}: {}", e.message), ..e }),
            None => self.err(at, format!("unknown function {callee}; declare it at the top of the file, or use Math.* and the expression functions")),
        }
    }
}

/// Compile a function's source (from `Function.prototype.toString`) into
/// expression-language text. `file` is the whole component file, for helper
/// functions and positions.
pub fn compile(fn_src: &str, file: &str) -> Result<String, FnError> {
    let offset = file.find(fn_src).unwrap_or(0);
    let toks = tokenize(fn_src);
    let mut p = Parser {
        toks: &toks,
        k: 0,
        base: offset,
        file,
    };
    let f = p.function()?;
    let file_toks = tokenize(file);
    let mut c = Compiler {
        toks: &file_toks,
        file,
        helpers: helper_starts(&file_toks),
        parsed: HashMap::new(),
    };
    // Parameters name component variables.
    let env: Env = f.params.iter().map(|n| (n.clone(), n.clone())).collect();
    match c.body(&f.body, &env, 0) {
        Ok(Some(v)) => Ok(v),
        Ok(None) => {
            let (line, column) = position(file, offset);
            Err(FnError {
                line,
                column,
                message: "the function must return a value on every path".into(),
            })
        }
        Err(e) if e.line == 0 => {
            let (line, column) = position(file, offset);
            Err(FnError { line, column, ..e })
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(src: &str, file: &str, vars: &[(&str, f64)]) -> f64 {
        let text = compile(src, file).unwrap_or_else(|e| panic!("{e:?}"));
        let e = ecs_expr::parse(&text).unwrap_or_else(|e| panic!("{text}: {e}"));
        let vars: HashMap<String, f64> = vars.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        ecs_expr::eval_with(&e, &|n| vars.get(n).copied())
            .unwrap_or_else(|e| panic!("{text}: {e:?}"))
    }

    #[test]
    fn arrow_with_destructured_parameters() {
        let f = "({ R, k }) => V(\"A\", \"B\") / R * k";
        assert_eq!(compile(f, f).unwrap().replace(' ', ""), "((V(A,B)/R)*k)");
        assert_eq!(
            eval("(a, b) => a ** 2 + b", "", &[("a", 3.0), ("b", 1.0)]),
            10.0
        );
        assert_eq!(eval("x => -(x ** 2)", "", &[("x", 3.0)]), -9.0);
        assert_eq!(eval("x => 2 ** 3 ** 2", "", &[("x", 0.0)]), 512.0);
    }

    #[test]
    fn statements_fold_into_select() {
        let f = "function (x, lim) {
            const a = Math.abs(x);
            let y = x;
            if (a > lim) {
                y = lim * (x > 0 ? 1 : -1);
            } else if (a < 1e-3) return 0;
            y *= 2;
            return y;
        }";
        assert_eq!(eval(f, f, &[("x", 5.0), ("lim", 2.0)]), 4.0);
        assert_eq!(eval(f, f, &[("x", -5.0), ("lim", 2.0)]), -4.0);
        assert_eq!(eval(f, f, &[("x", 1.5), ("lim", 2.0)]), 3.0);
        assert_eq!(eval(f, f, &[("x", 1e-4), ("lim", 2.0)]), 0.0);
    }

    #[test]
    fn helpers_are_inlined_and_math_maps() {
        let file = "// helpers\nfunction sq(v) { return v * v; }\nconst hyp = (a, b) => Math.sqrt(sq(a) + sq(b));\nexport default component({ equations(c) { c.current('A','B', ({ x, y }) => hyp(x, y) + Math.max(1, 2, x) + Math.PI * 0); } });";
        let f = "({ x, y }) => hyp(x, y) + Math.max(1, 2, x) + Math.PI * 0";
        assert_eq!(eval(f, file, &[("x", 3.0), ("y", 4.0)]), 8.0);
        assert_eq!(eval("x => x % 3", "", &[("x", 7.0)]), 1.0);
        assert_eq!(eval("x => limexp(x) - exp(x)", "", &[("x", 1.0)]), 0.0);
    }

    #[test]
    fn errors_point_at_the_line() {
        let file = "function bad(x) {\n  for (;;) {}\n}\nconst f = (x) => bad(x);\n";
        let e = compile("(x) => bad(x)", file).unwrap_err();
        assert_eq!(e.line, 2, "{e:?}");
        assert!(e.message.contains("for"), "{e:?}");
        let e = compile("(x) => nope(x)", "\n\nconst g = (x) => nope(x);").unwrap_err();
        assert_eq!(e.line, 3, "{e:?}");
        assert!(e.message.contains("unknown function nope"), "{e:?}");
        let e = compile("(x) => { if (x > 0) return 1; }", "").unwrap_err();
        assert!(e.message.contains("every path"), "{e:?}");
        let e = compile("(x) => V(A, K)", "").unwrap_err();
        assert!(e.message.contains("quotes"), "{e:?}");
    }
}
