//! Pieces shared by the assemblers: errors, operand splitting and a constant
//! expression evaluator with the number formats of MPASM and GNU as.

use std::collections::HashMap;

/// An assembly error at a source line (1-based).
#[derive(Debug, Clone, PartialEq)]
pub struct AsmError {
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for AsmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

/// Symbols, case-insensitive.
#[derive(Debug, Clone, Default)]
pub struct Symbols {
    map: HashMap<String, i64>,
}

impl Symbols {
    pub fn set(&mut self, name: &str, v: i64) {
        self.map.insert(name.to_ascii_uppercase(), v);
    }
    pub fn get(&self, name: &str) -> Option<i64> {
        self.map.get(&name.to_ascii_uppercase()).copied()
    }
    pub fn contains(&self, name: &str) -> bool {
        self.map.contains_key(&name.to_ascii_uppercase())
    }
    pub fn names(&self) -> impl Iterator<Item = (&String, &i64)> {
        self.map.iter()
    }
}

/// Remove a comment that starts with any of `marks` outside quotes.
pub fn strip_comment<'a>(line: &'a str, marks: &[&str]) -> &'a str {
    let b = line.as_bytes();
    let mut quote: Option<u8> = None;
    let mut k = 0;
    while k < b.len() {
        let c = b[k];
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == b'"' || (c == b'\'' && b.get(k + 2) == Some(&b'\'')) => quote = Some(c),
            None => {
                if marks.iter().any(|m| line[k..].starts_with(m)) {
                    return &line[..k];
                }
            }
        }
        k += 1;
    }
    line
}

/// Split operands at commas outside quotes, parentheses and brackets.
pub fn split_operands(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut cur = String::new();
    for c in s.chars() {
        if let Some(q) = quote {
            cur.push(c);
            if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' => {
                quote = Some('"');
                cur.push(c);
            }
            '(' | '[' => {
                depth += 1;
                cur.push(c);
            }
            ')' | ']' => {
                depth -= 1;
                cur.push(c);
            }
            ',' if depth == 0 => {
                out.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() || !out.is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

/// Expression context: symbols, the default radix and `$` (current address).
pub struct Ctx<'a> {
    pub syms: &'a Symbols,
    pub radix: u32,
    pub here: i64,
    /// In pass one, unknown symbols evaluate to 0 instead of failing.
    pub lenient: bool,
}

/// Evaluate a constant expression.
pub fn eval(src: &str, cx: &Ctx<'_>) -> Result<i64, String> {
    let toks = lex(src, cx.radix)?;
    let mut p = P { t: &toks, k: 0, cx };
    let v = p.or()?;
    if p.k != toks.len() {
        return Err(format!("unexpected `{}` in `{src}`", tok_text(&toks[p.k])));
    }
    Ok(v)
}

#[derive(Debug, Clone, PartialEq)]
enum T {
    Num(i64),
    Id(String),
    Op(&'static str),
}

fn tok_text(t: &T) -> String {
    match t {
        T::Num(n) => n.to_string(),
        T::Id(s) => s.clone(),
        T::Op(o) => o.to_string(),
    }
}

fn parse_digits(s: &str, radix: u32) -> Result<i64, String> {
    let clean: String = s.chars().filter(|c| *c != '_').collect();
    i64::from_str_radix(&clean, radix).map_err(|_| format!("bad number `{s}`"))
}

fn lex(src: &str, radix: u32) -> Result<Vec<T>, String> {
    let c: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut k = 0;
    while k < c.len() {
        let ch = c[k];
        if ch.is_whitespace() {
            k += 1;
            continue;
        }
        // Quoted forms: H'1F' D'10' B'1010' O'17' A'c' and 'c'.
        if k + 1 < c.len() && c[k + 1] == '\'' && "HDBOAhdboa".contains(ch) {
            let end = c[k + 2..]
                .iter()
                .position(|x| *x == '\'')
                .ok_or("unterminated quote")?
                + k
                + 2;
            let body: String = c[k + 2..end].iter().collect();
            let v = match ch.to_ascii_uppercase() {
                'H' => parse_digits(&body, 16)?,
                'D' => parse_digits(&body, 10)?,
                'B' => parse_digits(&body, 2)?,
                'O' => parse_digits(&body, 8)?,
                _ => body.chars().next().map_or(0, |x| x as i64),
            };
            out.push(T::Num(v));
            k = end + 1;
            continue;
        }
        if ch == '\'' && k + 2 < c.len() && c[k + 2] == '\'' {
            out.push(T::Num(c[k + 1] as i64));
            k += 3;
            continue;
        }
        if ch == '.' && k + 1 < c.len() && c[k + 1].is_ascii_digit() {
            let s = k + 1;
            k += 1;
            while k < c.len() && c[k].is_ascii_digit() {
                k += 1;
            }
            out.push(T::Num(parse_digits(
                &c[s..k].iter().collect::<String>(),
                10,
            )?));
            continue;
        }
        if ch.is_ascii_digit() {
            let s = k;
            while k < c.len() && (c[k].is_ascii_alphanumeric() || c[k] == '_') {
                k += 1;
            }
            let w: String = c[s..k].iter().collect();
            let lw = w.to_ascii_lowercase();
            let v = if let Some(h) = lw.strip_prefix("0x") {
                parse_digits(h, 16)?
            } else if let Some(b) = lw.strip_prefix("0b") {
                parse_digits(b, 2)?
            } else if let Some(h) = lw.strip_suffix('h') {
                parse_digits(h, 16)?
            } else if radix == 16 {
                parse_digits(&lw, 16)?
            } else {
                parse_digits(&lw, radix)?
            };
            out.push(T::Num(v));
            continue;
        }
        if ch.is_alphabetic()
            || ch == '_'
            || ch == '.'
            || (ch == '%' && c.get(k + 1).is_some_and(|x| x.is_alphabetic()))
        {
            let s = k;
            k += 1;
            while k < c.len() && (c[k].is_alphanumeric() || c[k] == '_' || c[k] == '.') {
                k += 1;
            }
            out.push(T::Id(c[s..k].iter().collect()));
            continue;
        }
        if ch == '$' {
            out.push(T::Id("$".into()));
            k += 1;
            continue;
        }
        let two: String = c[k..(k + 2).min(c.len())].iter().collect();
        let op = ["<<", ">>", "&&", "||"].into_iter().find(|o| *o == two);
        if let Some(o) = op {
            out.push(T::Op(o));
            k += 2;
            continue;
        }
        let one = match ch {
            '+' => "+",
            '-' => "-",
            '*' => "*",
            '/' => "/",
            '%' => "%",
            '&' => "&",
            '|' => "|",
            '^' => "^",
            '~' => "~",
            '!' => "!",
            '(' => "(",
            ')' => ")",
            _ => return Err(format!("unexpected `{ch}` in `{src}`")),
        };
        out.push(T::Op(one));
        k += 1;
    }
    Ok(out)
}

struct P<'a> {
    t: &'a [T],
    k: usize,
    cx: &'a Ctx<'a>,
}

impl P<'_> {
    fn op(&mut self, o: &str) -> bool {
        if self.t.get(self.k)
            == Some(&T::Op(match o {
                "+" => "+",
                "-" => "-",
                "*" => "*",
                "/" => "/",
                "%" => "%",
                "&" => "&",
                "|" => "|",
                "^" => "^",
                "<<" => "<<",
                ">>" => ">>",
                "(" => "(",
                ")" => ")",
                "~" => "~",
                "!" => "!",
                _ => return false,
            }))
        {
            self.k += 1;
            true
        } else {
            false
        }
    }
    fn or(&mut self) -> Result<i64, String> {
        let mut v = self.xor()?;
        while self.op("|") {
            v |= self.xor()?;
        }
        Ok(v)
    }
    fn xor(&mut self) -> Result<i64, String> {
        let mut v = self.and()?;
        while self.op("^") {
            v ^= self.and()?;
        }
        Ok(v)
    }
    fn and(&mut self) -> Result<i64, String> {
        let mut v = self.shift()?;
        while self.op("&") {
            v &= self.shift()?;
        }
        Ok(v)
    }
    fn shift(&mut self) -> Result<i64, String> {
        let mut v = self.add()?;
        loop {
            if self.op("<<") {
                v = v.wrapping_shl(self.add()? as u32);
            } else if self.op(">>") {
                v = v.wrapping_shr(self.add()? as u32);
            } else {
                return Ok(v);
            }
        }
    }
    fn add(&mut self) -> Result<i64, String> {
        let mut v = self.mul()?;
        loop {
            if self.op("+") {
                v = v.wrapping_add(self.mul()?);
            } else if self.op("-") {
                v = v.wrapping_sub(self.mul()?);
            } else {
                return Ok(v);
            }
        }
    }
    fn mul(&mut self) -> Result<i64, String> {
        let mut v = self.unary()?;
        loop {
            if self.op("*") {
                v = v.wrapping_mul(self.unary()?);
            } else if self.op("/") {
                let d = self.unary()?;
                v = if d == 0 {
                    return Err("division by zero".into());
                } else {
                    v / d
                };
            } else if self.op("%") {
                let d = self.unary()?;
                v = if d == 0 {
                    return Err("division by zero".into());
                } else {
                    v % d
                };
            } else {
                return Ok(v);
            }
        }
    }
    fn unary(&mut self) -> Result<i64, String> {
        if self.op("-") {
            return Ok(-self.unary()?);
        }
        if self.op("+") {
            return self.unary();
        }
        if self.op("~") {
            return Ok(!self.unary()?);
        }
        if self.op("!") {
            return Ok(i64::from(self.unary()? == 0));
        }
        self.atom()
    }
    fn atom(&mut self) -> Result<i64, String> {
        match self.t.get(self.k).cloned() {
            Some(T::Num(n)) => {
                self.k += 1;
                Ok(n)
            }
            Some(T::Op("(")) => {
                self.k += 1;
                let v = self.or()?;
                if !self.op(")") {
                    return Err("missing `)`".into());
                }
                Ok(v)
            }
            Some(T::Id(name)) => {
                self.k += 1;
                let up = name.to_ascii_uppercase();
                let func = matches!(up.as_str(), "HIGH" | "LOW" | "UPPER" | "%HI" | "%LO");
                if func && self.t.get(self.k) == Some(&T::Op("(")) {
                    self.k += 1;
                    let x = self.or()?;
                    if !self.op(")") {
                        return Err("missing `)`".into());
                    }
                    return Ok(match up.as_str() {
                        "HIGH" => (x >> 8) & 0xFF,
                        "LOW" => x & 0xFF,
                        "UPPER" => (x >> 16) & 0xFF,
                        "%HI" => ((x + 0x8000) >> 16) & 0xFFFF,
                        _ => {
                            let lo = x & 0xFFFF;
                            if lo >= 0x8000 { lo - 0x10000 } else { lo }
                        }
                    });
                }
                if name == "$" {
                    return Ok(self.cx.here);
                }
                match self.cx.syms.get(&name) {
                    Some(v) => Ok(v),
                    None if self.cx.lenient => Ok(0),
                    None => Err(format!("unknown symbol `{name}`")),
                }
            }
            Some(t) => Err(format!("unexpected `{}`", tok_text(&t))),
            None => Err("missing value".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_formats_and_operators() {
        let mut s = Symbols::default();
        s.set("PORTB", 6);
        let cx = |radix| Ctx {
            syms: &s,
            radix,
            here: 0x10,
            lenient: false,
        };
        for (src, radix, want) in [
            ("10", 16, 16),
            ("10", 10, 10),
            ("D'10'", 16, 10),
            (".10", 16, 10),
            ("H'1F'", 10, 31),
            ("0FFh", 10, 255),
            ("B'1010'", 16, 10),
            ("0b1010 | 1", 10, 11),
            ("'A'", 10, 65),
            ("portb + 1", 16, 7),
            ("HIGH(0x1234) + LOW(0x1234)", 10, 0x12 + 0x34),
            ("$ + 2", 10, 0x12),
            ("(1 << 4) - 1", 10, 15),
            ("%hi(0xBF886138)", 10, 0xBF88),
            ("%lo(0xBF886138)", 10, 0x6138),
            ("%hi(0xBF88A000)", 10, 0xBF89),
            ("%lo(0xBF88A000)", 10, -0x6000),
            ("7 % 3", 10, 1),
        ] {
            assert_eq!(eval(src, &cx(radix)), Ok(want), "{src}");
        }
        assert!(eval("nope", &cx(10)).is_err());
        assert_eq!(
            split_operands("a, (b, c), \"x,y\""),
            vec!["a", "(b, c)", "\"x,y\""]
        );
        assert_eq!(strip_comment("movlw 'A' ; load", &[";"]), "movlw 'A' ");
    }
}
