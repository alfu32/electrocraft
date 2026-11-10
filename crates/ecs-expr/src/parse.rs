//! Recursive-descent parser for the expression language (spec section 5.2).

use crate::{BinOp, Expr, UnOp};
use std::fmt;

/// A parse error with a 1-based line and column inside the parsed text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.column, self.message)
    }
}

impl std::error::Error for ParseError {}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    Sym(&'static str),
    End,
}

struct Lexer<'a> {
    src: &'a str,
    toks: Vec<(Tok, usize)>,
}

const SYMBOLS: [&str; 19] = [
    "**", "<=", ">=", "==", "!=", "&&", "||", "+", "-", "*", "/", "^", "(", ")", ",", "<", ">",
    "?", ":",
];

impl<'a> Lexer<'a> {
    fn run(src: &'a str) -> Result<Vec<(Tok, usize)>, ParseError> {
        let mut lx = Lexer {
            src,
            toks: Vec::new(),
        };
        let bytes: Vec<(usize, char)> = src.char_indices().collect();
        let mut i = 0;
        'outer: while i < bytes.len() {
            let (pos, c) = bytes[i];
            if c.is_whitespace() {
                i += 1;
                continue;
            }
            if c.is_ascii_digit()
                || (c == '.' && bytes.get(i + 1).is_some_and(|b| b.1.is_ascii_digit()))
            {
                let start = pos;
                let mut j = i;
                while j < bytes.len() && (bytes[j].1.is_ascii_digit() || bytes[j].1 == '.') {
                    j += 1;
                }
                if j < bytes.len() && (bytes[j].1 == 'e' || bytes[j].1 == 'E') {
                    let mut k = j + 1;
                    if k < bytes.len() && (bytes[k].1 == '+' || bytes[k].1 == '-') {
                        k += 1;
                    }
                    if k < bytes.len() && bytes[k].1.is_ascii_digit() {
                        while k < bytes.len() && bytes[k].1.is_ascii_digit() {
                            k += 1;
                        }
                        j = k;
                    }
                }
                while j < bytes.len()
                    && (bytes[j].1.is_alphanumeric() || bytes[j].1 == 'µ' || bytes[j].1 == 'Ω')
                {
                    j += 1;
                }
                let end = bytes.get(j).map_or(src.len(), |b| b.0);
                let text = &src[start..end];
                let value = ecs_units::parse(text)
                    .map_err(|_| lx.error(start, format!("cannot read number \"{text}\"")))?;
                lx.toks.push((Tok::Num(value), start));
                i = j;
                continue;
            }
            if c.is_alphabetic() || c == '_' {
                let start = pos;
                let mut j = i;
                while j < bytes.len()
                    && (bytes[j].1.is_alphanumeric() || bytes[j].1 == '_' || bytes[j].1 == '.')
                {
                    j += 1;
                }
                let end = bytes.get(j).map_or(src.len(), |b| b.0);
                lx.toks
                    .push((Tok::Ident(src[start..end].to_string()), start));
                i = j;
                continue;
            }
            for s in SYMBOLS {
                if src[pos..].starts_with(s) {
                    lx.toks.push((Tok::Sym(s), pos));
                    i += s.chars().count();
                    continue 'outer;
                }
            }
            if c == '!' {
                lx.toks.push((Tok::Sym("!"), pos));
                i += 1;
                continue;
            }
            return Err(lx.error(pos, format!("unexpected character '{c}'")));
        }
        lx.toks.push((Tok::End, src.len()));
        Ok(lx.toks)
    }

    fn error(&self, offset: usize, message: String) -> ParseError {
        position(self.src, offset, message)
    }
}

fn position(src: &str, offset: usize, message: String) -> ParseError {
    let before = &src[..offset.min(src.len())];
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    ParseError {
        line,
        column,
        message,
    }
}

struct Parser<'a> {
    src: &'a str,
    toks: Vec<(Tok, usize)>,
    i: usize,
}

/// Parse an expression such as `Is * (limexp(V(A, K) / (N * vt)) - 1)`.
pub fn parse(src: &str) -> Result<Expr, ParseError> {
    let toks = Lexer::run(src)?;
    let mut p = Parser { src, toks, i: 0 };
    let e = p.ternary()?;
    if p.peek() != &Tok::End {
        return Err(p.err("unexpected text after the expression"));
    }
    Ok(e)
}

impl Parser<'_> {
    fn peek(&self) -> &Tok {
        &self.toks[self.i].0
    }
    fn err(&self, message: &str) -> ParseError {
        position(self.src, self.toks[self.i].1, message.to_string())
    }
    fn eat(&mut self, s: &str) -> bool {
        if matches!(self.peek(), Tok::Sym(x) if *x == s) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, s: &str) -> Result<(), ParseError> {
        if self.eat(s) {
            Ok(())
        } else {
            Err(self.err(&format!("expected '{s}'")))
        }
    }

    fn ternary(&mut self) -> Result<Expr, ParseError> {
        let c = self.or()?;
        if self.eat("?") {
            let a = self.ternary()?;
            self.expect(":")?;
            let b = self.ternary()?;
            return Ok(Expr::Select(Box::new(c), Box::new(a), Box::new(b)));
        }
        Ok(c)
    }
    fn or(&mut self) -> Result<Expr, ParseError> {
        let mut a = self.and()?;
        while self.eat("||") {
            a = Expr::bin(BinOp::Or, a, self.and()?);
        }
        Ok(a)
    }
    fn and(&mut self) -> Result<Expr, ParseError> {
        let mut a = self.cmp()?;
        while self.eat("&&") {
            a = Expr::bin(BinOp::And, a, self.cmp()?);
        }
        Ok(a)
    }
    fn cmp(&mut self) -> Result<Expr, ParseError> {
        let a = self.add()?;
        for (s, op) in [
            ("<=", BinOp::Le),
            (">=", BinOp::Ge),
            ("==", BinOp::Eq),
            ("!=", BinOp::Ne),
            ("<", BinOp::Lt),
            (">", BinOp::Gt),
        ] {
            if self.eat(s) {
                return Ok(Expr::bin(op, a, self.add()?));
            }
        }
        Ok(a)
    }
    fn add(&mut self) -> Result<Expr, ParseError> {
        let mut a = self.mul()?;
        loop {
            if self.eat("+") {
                a = Expr::bin(BinOp::Add, a, self.mul()?);
            } else if self.eat("-") {
                a = Expr::bin(BinOp::Sub, a, self.mul()?);
            } else {
                return Ok(a);
            }
        }
    }
    fn mul(&mut self) -> Result<Expr, ParseError> {
        let mut a = self.unary()?;
        loop {
            if self.eat("*") {
                a = Expr::bin(BinOp::Mul, a, self.unary()?);
            } else if self.eat("/") {
                a = Expr::bin(BinOp::Div, a, self.unary()?);
            } else {
                return Ok(a);
            }
        }
    }
    fn unary(&mut self) -> Result<Expr, ParseError> {
        if self.eat("-") {
            return Ok(Expr::un(UnOp::Neg, self.unary()?));
        }
        if self.eat("+") {
            return self.unary();
        }
        if self.eat("!") {
            return Ok(Expr::un(UnOp::Not, self.unary()?));
        }
        self.power()
    }
    fn power(&mut self) -> Result<Expr, ParseError> {
        let base = self.primary()?;
        if self.eat("^") || self.eat("**") {
            // Right associative; binds tighter than unary minus on the left.
            let exp = self.unary()?;
            return Ok(Expr::bin(BinOp::Pow, base, exp));
        }
        Ok(base)
    }
    fn args(&mut self) -> Result<Vec<Expr>, ParseError> {
        let mut out = Vec::new();
        if self.eat(")") {
            return Ok(out);
        }
        loop {
            out.push(self.ternary()?);
            if self.eat(")") {
                return Ok(out);
            }
            self.expect(",")?;
        }
    }
    fn primary(&mut self) -> Result<Expr, ParseError> {
        let at = self.i;
        match self.peek().clone() {
            Tok::Num(v) => {
                self.i += 1;
                Ok(Expr::Num(v))
            }
            Tok::Sym("(") => {
                self.i += 1;
                let e = self.ternary()?;
                self.expect(")")?;
                Ok(e)
            }
            Tok::Ident(name) => {
                self.i += 1;
                if !self.eat("(") {
                    return Ok(match name.as_str() {
                        "pi" => Expr::Num(std::f64::consts::PI),
                        _ => Expr::Name(name),
                    });
                }
                let args = self.args()?;
                self.call(&name, args, at)
            }
            Tok::End => Err(self.err("unexpected end of expression")),
            Tok::Sym(s) => Err(self.err(&format!("unexpected '{s}'"))),
        }
    }

    fn call(&mut self, name: &str, mut args: Vec<Expr>, at: usize) -> Result<Expr, ParseError> {
        let fail = |p: &Self, msg: String| Err(position(p.src, p.toks[at].1, msg));
        let arity = |p: &Self, n: usize, args: &Vec<Expr>| -> Result<(), ParseError> {
            if args.len() == n {
                Ok(())
            } else {
                Err(position(
                    p.src,
                    p.toks[at].1,
                    format!(
                        "{name}() takes {n} argument{}",
                        if n == 1 { "" } else { "s" }
                    ),
                ))
            }
        };
        let node_name = |e: &Expr| match e {
            Expr::Name(n) => Some(n.clone()),
            Expr::Num(v) if *v == 0.0 => Some("0".to_string()),
            _ => None,
        };
        let un = match name {
            "exp" => Some(UnOp::Exp),
            "limexp" => Some(UnOp::Limexp),
            "ln" | "log" => Some(UnOp::Ln),
            "log10" => Some(UnOp::Log10),
            "sqrt" => Some(UnOp::Sqrt),
            "safe_sqrt" => Some(UnOp::SafeSqrt),
            "sin" => Some(UnOp::Sin),
            "cos" => Some(UnOp::Cos),
            "tan" => Some(UnOp::Tan),
            "atan" => Some(UnOp::Atan),
            "tanh" => Some(UnOp::Tanh),
            "sinh" => Some(UnOp::Sinh),
            "cosh" => Some(UnOp::Cosh),
            "abs" => Some(UnOp::Abs),
            "floor" => Some(UnOp::Floor),
            _ => None,
        };
        if let Some(op) = un {
            arity(self, 1, &args)?;
            return Ok(Expr::un(op, args.remove(0)));
        }
        let bin = match name {
            "pow" => Some(BinOp::Pow),
            "min" => Some(BinOp::Min),
            "max" => Some(BinOp::Max),
            "safe_div" => Some(BinOp::SafeDiv),
            _ => None,
        };
        if let Some(op) = bin {
            arity(self, 2, &args)?;
            let b = args.remove(1);
            return Ok(Expr::bin(op, args.remove(0), b));
        }
        match name {
            "V" => {
                if args.is_empty() || args.len() > 2 {
                    return fail(self, "V() takes one or two node names".into());
                }
                let a = node_name(&args[0]);
                let b = args.get(1).map(node_name);
                match (a, b) {
                    (Some(a), None) => Ok(Expr::V(a, None)),
                    (Some(a), Some(Some(b))) => Ok(Expr::V(a, Some(b))),
                    _ => fail(self, "V() arguments must be node names".into()),
                }
            }
            "I" => {
                arity(self, 1, &args)?;
                match node_name(&args[0]) {
                    Some(n) => Ok(Expr::I(n)),
                    None => fail(self, "I() argument must be a branch name".into()),
                }
            }
            "select" | "if" => {
                arity(self, 3, &args)?;
                let b = args.remove(2);
                let a = args.remove(1);
                Ok(Expr::Select(
                    Box::new(args.remove(0)),
                    Box::new(a),
                    Box::new(b),
                ))
            }
            "smoothstep" => {
                // t = clamp((x - e0) / (e1 - e0), 0, 1); t*t*(3 - 2t)
                arity(self, 3, &args)?;
                let x = args.remove(2);
                let e1 = args.remove(1);
                let e0 = args.remove(0);
                let t = Expr::bin(
                    BinOp::Min,
                    Expr::bin(BinOp::Max, (x - e0.clone()) / (e1 - e0), Expr::Num(0.0)),
                    Expr::Num(1.0),
                );
                Ok(t.clone() * t.clone() * (Expr::Num(3.0) - Expr::Num(2.0) * t))
            }
            "smooth_abs" => {
                // sqrt(x^2 + eps^2), eps defaults to 1e-6.
                if args.is_empty() || args.len() > 2 {
                    return fail(self, "smooth_abs() takes one or two arguments".into());
                }
                let eps = if args.len() == 2 {
                    args.remove(1)
                } else {
                    Expr::Num(1e-6)
                };
                let x = args.remove(0);
                Ok(Expr::un(UnOp::Sqrt, x.clone() * x + eps.clone() * eps))
            }
            _ => fail(self, format!("unknown function {name}()")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_diode_equation() {
        let e = parse("Is * (limexp(V(A, K) / (N * vt)) - 1)").unwrap();
        let mut names = Vec::new();
        e.visit(&mut |l| names.push(format!("{l:?}")));
        assert!(names.iter().any(|n| n.contains("V(\"A\", Some(\"K\"))")));
    }

    #[test]
    fn engineering_numbers_and_precedence() {
        let e = parse("2 * 4k7 + 3 ^ 2").unwrap();
        let v = crate::eval_with(&e, &|_| None).unwrap();
        assert_eq!(v, 2.0 * 4700.0 + 9.0);
        let e = parse("-2 ^ 2").unwrap();
        assert_eq!(crate::eval_with(&e, &|_| None).unwrap(), -4.0);
        let e = parse("1 < 2 ? 10 : 20").unwrap();
        assert_eq!(crate::eval_with(&e, &|_| None).unwrap(), 10.0);
    }

    #[test]
    fn errors_report_line_and_column() {
        let err = parse("1 +\n  foo(2)").unwrap_err();
        assert_eq!((err.line, err.column), (2, 3));
        let err = parse("V(1+2)").unwrap_err();
        assert_eq!(err.column, 1);
        let err = parse("(1 + 2").unwrap_err();
        assert!(err.message.contains("')'"));
    }
}
