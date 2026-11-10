//! 16-bit PIC24 (PIC24FJ64GA002): W0-W15 (W15 the stack pointer), SR flags
//! N, OV, Z, C, data memory with the W registers mapped at 0x0000, RAM from
//! 0x0800, GPIO (TRIS/PORT/LAT), and Timer1 with prescaler and period match
//! setting T1IF (polled; interrupts are not modelled).
//!
//! Instructions (word and `.b` byte forms where the chip has them):
//! MOV (literal, register, file, indirect), ADD, ADDC, SUB, SUBB, SUBR, AND,
//! IOR, XOR (Wb,Ws,Wd · Wb,#lit,Wd · #lit,Wn · f · f,WREG), INC, INC2, DEC,
//! DEC2, COM, NEG, CLR, SETM, SL, LSR, ASR (by 1, by #lit or Wns), BSET,
//! BCLR, BTG, BTSC, BTSS, CP, CP0, BRA (with conditions), GOTO, CALL, RCALL,
//! RETURN, RETLW, REPEAT, PUSH, POP, MUL.UU/SS, SWAP, ZE, SE, NOP.
//! Addressing: Wn, [Wn], [Wn++], [Wn--], [++Wn], [--Wn], [Wn+#k].
//!
//! Assembler: XC16 syntax: `;` comments, `label:`, `#` literals (decimal by
//! default, `0x` hex), `.text`, `.bss`/`.data` with `.space`, `.equ`,
//! `.global`, `.include` and `.end` (ignored). Execution starts at `__reset`
//! (or `main`, or the first instruction).

use crate::asm::{self, AsmError, Ctx, Symbols};
use crate::{Cpu, Device, Port};

const TEXT_BASE: u32 = 0x0200;
const RAM_START: u16 = 0x0800;
const RAM_END: u16 = 0x2800;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    Direct,
    Plain,
    PostInc,
    PostDec,
    PreInc,
    PreDec,
    Offset(i16),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Opnd {
    None,
    W(u8, Mode),
    Lit(i64),
    File(u16),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Op {
    Mov,
    Add,
    Addc,
    Sub,
    Subb,
    Subr,
    And,
    Ior,
    Xor,
    Inc,
    Inc2,
    Dec,
    Dec2,
    Com,
    Neg,
    Clr,
    Setm,
    Sl,
    Lsr,
    Asr,
    Bset,
    Bclr,
    Btg,
    Btsc,
    Btss,
    Cp,
    Cp0,
    Bra,
    Goto,
    Call,
    Rcall,
    Return,
    Retlw,
    Repeat,
    Push,
    Pop,
    MulUu,
    MulSs,
    Swap,
    Ze,
    Se,
    Nop,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Cond {
    Always,
    Z,
    Nz,
    C,
    Nc,
    N,
    Nn,
    Ov,
    Nov,
    Ge,
    Lt,
    Gt,
    Le,
    Gtu,
    Leu,
}

#[derive(Debug, Clone, Copy)]
struct Ins {
    op: Op,
    byte: bool,
    a: Opnd,
    b: Opnd,
    c: Opnd,
    cond: Cond,
    /// Branch target (program address) or literal.
    target: u32,
    words: u32,
}

#[derive(Debug, Clone)]
pub struct Program {
    /// Instruction per program word address / 2 (None for the second word).
    slots: Vec<Option<Ins>>,
    data: Vec<(u16, u8)>,
    entry: u32,
    stack: u16,
}

pub struct Pic24 {
    p: Program,
    w: [u16; 16],
    sr: u16,
    mem: Vec<u8>,
    pc: u32,
    ports: Vec<Port>,
    repeat: u32,
    t1_pre: u32,
    cycles: u64,
    fault: Option<String>,
}

const SR_C: u16 = 1;
const SR_Z: u16 = 2;
const SR_OV: u16 = 4;
const SR_N: u16 = 8;

// SFR addresses (PIC24FJ64GA002).
const SR: u16 = 0x0042;
const TMR1: u16 = 0x0100;
const PR1: u16 = 0x0102;
const T1CON: u16 = 0x0104;
const IFS0: u16 = 0x0084;
const PORT_BASE: [u16; 2] = [0x02C0, 0x02C8];

impl Pic24 {
    pub fn new(dev: &Device, p: Program) -> Self {
        let mut ports = vec![Port::default(); dev.ports];
        for (_, port, bit) in &dev.pins {
            ports[*port].mask |= 1 << bit;
        }
        for p in &mut ports {
            p.tris = 0xFFFF;
        }
        let mut mem = vec![0u8; 0x10000];
        for (a, b) in &p.data {
            mem[*a as usize] = *b;
        }
        mem[PR1 as usize] = 0xFF;
        mem[PR1 as usize + 1] = 0xFF;
        let mut w = [0u16; 16];
        w[15] = p.stack;
        let entry = p.entry;
        Self {
            p,
            w,
            sr: 0,
            mem,
            pc: entry,
            ports,
            repeat: 0,
            t1_pre: 0,
            cycles: 0,
            fault: None,
        }
    }

    fn rd16(&self, a: u16) -> u16 {
        let a = a & !1;
        if a < 0x20 {
            return self.w[(a / 2) as usize];
        }
        if a == SR {
            return self.sr;
        }
        for (k, base) in PORT_BASE.iter().enumerate() {
            if k < self.ports.len() && (*base..*base + 6).contains(&a) {
                let p = self.ports[k];
                return match a - base {
                    0 => p.tris as u16,
                    2 => p.read() as u16,
                    _ => p.lat as u16,
                };
            }
        }
        u16::from_le_bytes([self.mem[a as usize], self.mem[a as usize + 1]])
    }

    fn wr16(&mut self, a: u16, v: u16) {
        let a = a & !1;
        if a < 0x20 {
            self.w[(a / 2) as usize] = v;
            return;
        }
        if a == SR {
            self.sr = v;
            return;
        }
        for (k, base) in PORT_BASE.iter().enumerate() {
            if k < self.ports.len() && (*base..*base + 6).contains(&a) {
                match a - base {
                    0 => self.ports[k].tris = v as u32,
                    _ => self.ports[k].lat = v as u32,
                }
                return;
            }
        }
        if a == TMR1 {
            self.t1_pre = 0;
        }
        self.mem[a as usize..a as usize + 2].copy_from_slice(&v.to_le_bytes());
    }

    fn rd(&self, a: u16, byte: bool) -> u16 {
        let w = self.rd16(a);
        if byte {
            if a & 1 == 1 { w >> 8 } else { w & 0xFF }
        } else {
            w
        }
    }

    fn wr(&mut self, a: u16, v: u16, byte: bool) {
        if byte {
            let w = self.rd16(a);
            let nw = if a & 1 == 1 {
                (w & 0x00FF) | (v & 0xFF) << 8
            } else {
                (w & 0xFF00) | (v & 0xFF)
            };
            self.wr16(a, nw);
        } else {
            if a & 1 == 1 {
                self.fault = Some(format!(
                    "word access to odd address 0x{a:04X} (pc 0x{:04X})",
                    self.pc
                ));
                return;
            }
            self.wr16(a, v);
        }
    }

    /// Resolve an operand to a location (applying pre/post modification).
    fn loc(&mut self, o: Opnd, byte: bool) -> Loc {
        let step = if byte { 1 } else { 2 };
        match o {
            Opnd::W(n, Mode::Direct) => Loc::W(n),
            Opnd::W(n, m) => {
                let r = n as usize;
                let a = match m {
                    Mode::Plain | Mode::Direct => self.w[r],
                    Mode::PostInc => {
                        let a = self.w[r];
                        self.w[r] = a.wrapping_add(step);
                        a
                    }
                    Mode::PostDec => {
                        let a = self.w[r];
                        self.w[r] = a.wrapping_sub(step);
                        a
                    }
                    Mode::PreInc => {
                        self.w[r] = self.w[r].wrapping_add(step);
                        self.w[r]
                    }
                    Mode::PreDec => {
                        self.w[r] = self.w[r].wrapping_sub(step);
                        self.w[r]
                    }
                    Mode::Offset(k) => self.w[r].wrapping_add(k as u16),
                };
                Loc::Mem(a)
            }
            Opnd::File(a) => Loc::Mem(a),
            Opnd::Lit(v) => Loc::Lit(v as u16),
            Opnd::None => Loc::Lit(0),
        }
    }

    fn get(&self, l: Loc, byte: bool) -> u16 {
        match l {
            Loc::W(n) => {
                let v = self.w[n as usize];
                if byte { v & 0xFF } else { v }
            }
            Loc::Mem(a) => self.rd(a, byte),
            Loc::Lit(v) => {
                if byte {
                    v & 0xFF
                } else {
                    v
                }
            }
        }
    }

    fn put(&mut self, l: Loc, v: u16, byte: bool) {
        match l {
            Loc::W(n) => {
                let r = &mut self.w[n as usize];
                *r = if byte { (*r & 0xFF00) | (v & 0xFF) } else { v };
            }
            Loc::Mem(a) => self.wr(a, v, byte),
            Loc::Lit(_) => {}
        }
    }

    fn flag(&mut self, bit: u16, v: bool) {
        if v {
            self.sr |= bit;
        } else {
            self.sr &= !bit;
        }
    }

    fn nz(&mut self, r: u32, byte: bool) {
        let (mask, top) = if byte { (0xFF, 0x80) } else { (0xFFFF, 0x8000) };
        self.flag(SR_Z, r & mask == 0);
        self.flag(SR_N, r & top != 0);
    }

    /// a + b + cin with all flags.
    fn add_flags(&mut self, a: u16, b: u16, cin: u16, byte: bool) -> u16 {
        let (mask, top) = if byte {
            (0xFFu32, 0x80u32)
        } else {
            (0xFFFF, 0x8000)
        };
        let (a, b) = (a as u32 & mask, b as u32 & mask);
        let r = a + b + cin as u32;
        self.flag(SR_C, r > mask);
        self.flag(SR_OV, (!(a ^ b) & (a ^ r) & top) != 0);
        self.nz(r, byte);
        (r & mask) as u16
    }

    fn timer1(&mut self, cycles: u32) {
        let con = self.rd16(T1CON);
        if con & 0x8000 == 0 {
            return;
        }
        let rate = [1, 8, 64, 256][((con >> 4) & 3) as usize];
        for _ in 0..cycles {
            self.t1_pre += 1;
            if self.t1_pre < rate {
                continue;
            }
            self.t1_pre = 0;
            let t = self.rd16(TMR1);
            let pr = self.rd16(PR1);
            if t == pr {
                self.mem[TMR1 as usize..TMR1 as usize + 2].copy_from_slice(&0u16.to_le_bytes());
                let f = self.rd16(IFS0) | 0x0008;
                self.mem[IFS0 as usize..IFS0 as usize + 2].copy_from_slice(&f.to_le_bytes());
            } else {
                self.mem[TMR1 as usize..TMR1 as usize + 2]
                    .copy_from_slice(&t.wrapping_add(1).to_le_bytes());
            }
        }
    }

    fn cond(&self, c: Cond) -> bool {
        let (n, ov, z, cf) = (
            self.sr & SR_N != 0,
            self.sr & SR_OV != 0,
            self.sr & SR_Z != 0,
            self.sr & SR_C != 0,
        );
        match c {
            Cond::Always => true,
            Cond::Z => z,
            Cond::Nz => !z,
            Cond::C => cf,
            Cond::Nc => !cf,
            Cond::N => n,
            Cond::Nn => !n,
            Cond::Ov => ov,
            Cond::Nov => !ov,
            Cond::Ge => n == ov,
            Cond::Lt => n != ov,
            Cond::Gt => !z && n == ov,
            Cond::Le => z || n != ov,
            Cond::Gtu => cf && !z,
            Cond::Leu => !cf || z,
        }
    }

    fn push16(&mut self, v: u16) {
        let sp = self.w[15];
        self.wr16(sp, v);
        self.w[15] = sp.wrapping_add(2);
        if self.w[15] >= RAM_END {
            self.fault = Some("stack overflow".into());
        }
    }
    fn pop16(&mut self) -> u16 {
        if self.w[15] <= self.p.stack {
            self.fault = Some(format!(
                "stack underflow at 0x{:04X} (RETURN without CALL?)",
                self.pc
            ));
            return 0;
        }
        self.w[15] = self.w[15].wrapping_sub(2);
        self.rd16(self.w[15])
    }
}

#[derive(Clone, Copy)]
enum Loc {
    W(u8),
    Mem(u16),
    Lit(u16),
}

impl Cpu for Pic24 {
    fn ports(&mut self) -> &mut Vec<Port> {
        &mut self.ports
    }
    fn fault(&self) -> Option<String> {
        self.fault.clone()
    }
    fn summary(&self) -> String {
        format!(
            "PC {:06X}  W0 {:04X}  W1 {:04X}  SR {:04X}  cycles {}",
            self.pc, self.w[0], self.w[1], self.sr, self.cycles
        )
    }

    fn step(&mut self) -> u32 {
        let slot = ((self.pc - TEXT_BASE) / 2) as usize;
        let Some(Some(ins)) = self.p.slots.get(slot).copied() else {
            self.fault = Some(format!("no instruction at 0x{:06X}", self.pc));
            return 1;
        };
        let mut next = self.pc + 2 * ins.words;
        let mut cyc = 1;
        let byte = ins.byte;
        let mask: u32 = if byte { 0xFF } else { 0xFFFF };
        use Op::*;
        match ins.op {
            Mov => {
                let s = self.loc(ins.a, byte);
                let v = self.get(s, byte);
                if ins.b == Opnd::None {
                    // MOV f: flags only (f to itself).
                    self.nz(v as u32, byte);
                } else {
                    let d = self.loc(ins.b, byte);
                    self.put(d, v, byte);
                    if matches!(ins.a, Opnd::File(_)) && matches!(ins.b, Opnd::W(_, Mode::Direct)) {
                        self.nz(v as u32, byte);
                    }
                }
            }
            Add | Addc | Sub | Subb | Subr | And | Ior | Xor => {
                // Forms: (Wb, Ws|#lit, Wd); (#lit, Wn); (f) or (f, WREG).
                let (x, y, dest) = match (ins.a, ins.b, ins.c) {
                    (Opnd::Lit(k), Opnd::W(n, _), Opnd::None) => {
                        let v = self.w[n as usize];
                        (if byte { v & 0xFF } else { v }, k as u16, Loc::W(n))
                    }
                    (Opnd::File(f), b, Opnd::None) => {
                        let fv = self.rd(f, byte);
                        let w0 = self.get(Loc::W(0), byte);
                        (
                            fv,
                            w0,
                            if b == Opnd::None {
                                Loc::Mem(f)
                            } else {
                                Loc::W(0)
                            },
                        )
                    }
                    (a, b, c) => {
                        let la = self.loc(a, byte);
                        let xa = self.get(la, byte);
                        let lb = self.loc(b, byte);
                        let yb = self.get(lb, byte);
                        let lc = self.loc(c, byte);
                        (xa, yb, lc)
                    }
                };
                let c_in = u16::from(self.sr & SR_C != 0);
                let r = match ins.op {
                    Add => self.add_flags(x, y, 0, byte),
                    Addc => self.add_flags(x, y, c_in, byte),
                    Sub => self.add_flags(x, !y, 1, byte),
                    Subb => self.add_flags(x, !y, c_in, byte),
                    Subr => self.add_flags(y, !x, 1, byte),
                    And => {
                        let r = (x & y) as u32;
                        self.nz(r, byte);
                        r as u16
                    }
                    Ior => {
                        let r = (x | y) as u32;
                        self.nz(r, byte);
                        r as u16
                    }
                    _ => {
                        let r = (x ^ y) as u32;
                        self.nz(r, byte);
                        r as u16
                    }
                };
                self.put(dest, r, byte);
            }
            Inc | Inc2 | Dec | Dec2 | Com | Neg | Sl | Lsr | Asr | Swap | Ze | Se => {
                let (src, dst) = match (ins.a, ins.b) {
                    (Opnd::File(f), Opnd::None) => (Loc::Mem(f), Loc::Mem(f)),
                    (Opnd::File(f), _) => (Loc::Mem(f), Loc::W(0)),
                    (a, Opnd::None) => {
                        let l = self.loc(a, byte);
                        (l, l)
                    }
                    (a, b) if matches!(ins.op, Sl | Lsr | Asr) && ins.c != Opnd::None => {
                        // Wb, #lit4 | Wns, Wnd
                        let l = self.loc(a, false);
                        let x = self.get(l, false) as u32;
                        let sh = match b {
                            Opnd::Lit(k) => k as u32 & 15,
                            other => {
                                let lb = self.loc(other, false);
                                self.get(lb, false) as u32 & 15
                            }
                        };
                        let r = match ins.op {
                            Sl => (x << sh) & 0xFFFF,
                            Lsr => x >> sh,
                            _ => ((x as u16 as i16) >> sh) as u16 as u32,
                        };
                        self.nz(r, false);
                        let lc = self.loc(ins.c, false);
                        self.put(lc, r as u16, false);
                        self.pc = next;
                        self.timer1(1);
                        self.cycles += 1;
                        return 1;
                    }
                    (a, b) => {
                        let la = self.loc(a, byte);
                        let lb = self.loc(b, byte);
                        (la, lb)
                    }
                };
                let x = self.get(src, byte);
                let r = match ins.op {
                    Inc => self.add_flags(x, 1, 0, byte),
                    Inc2 => self.add_flags(x, 2, 0, byte),
                    Dec => self.add_flags(x, !1, 1, byte),
                    Dec2 => self.add_flags(x, !2, 1, byte),
                    Neg => self.add_flags(0, !x, 1, byte),
                    Com => {
                        let r = !x as u32 & mask;
                        self.nz(r, byte);
                        r as u16
                    }
                    Sl => {
                        let r = (x as u32) << 1;
                        self.flag(SR_C, r & (mask + 1) != 0);
                        self.nz(r, byte);
                        (r & mask) as u16
                    }
                    Lsr => {
                        self.flag(SR_C, x & 1 == 1);
                        let r = (x as u32 & mask) >> 1;
                        self.nz(r, byte);
                        r as u16
                    }
                    Asr => {
                        self.flag(SR_C, x & 1 == 1);
                        let r = if byte {
                            ((x as u8 as i8) >> 1) as u8 as u32
                        } else {
                            ((x as i16) >> 1) as u16 as u32
                        };
                        self.nz(r, byte);
                        r as u16
                    }
                    Swap => {
                        if byte {
                            (x << 4 | x >> 4) & 0xFF
                        } else {
                            x.rotate_left(8)
                        }
                    }
                    Ze => x & 0xFF,
                    _ => x as u8 as i8 as i16 as u16,
                };
                self.put(dst, r, byte);
            }
            Clr | Setm => {
                let l = match ins.a {
                    Opnd::File(f) if ins.b != Opnd::None => {
                        let _ = f;
                        Loc::W(0)
                    }
                    a => self.loc(a, byte),
                };
                self.put(l, if ins.op == Clr { 0 } else { 0xFFFF }, byte);
                if ins.op == Clr {
                    self.flag(SR_Z, true);
                }
            }
            Bset | Bclr | Btg | Btsc | Btss => {
                let bit = ins.target as u16;
                let wide = !byte;
                let l = self.loc(ins.a, !wide);
                let x = self.get(l, !wide);
                match ins.op {
                    Bset => self.put(l, x | (1 << bit), !wide),
                    Bclr => self.put(l, x & !(1 << bit), !wide),
                    Btg => self.put(l, x ^ (1 << bit), !wide),
                    _ => {
                        let set = x >> bit & 1 == 1;
                        if (ins.op == Btss) == set {
                            // Skip the next instruction (one or two words).
                            let after = ((next - TEXT_BASE) / 2) as usize;
                            let w = self
                                .p
                                .slots
                                .get(after)
                                .and_then(|s| s.map(|i| i.words))
                                .unwrap_or(1);
                            next += 2 * w;
                            cyc = 1 + w;
                        }
                    }
                }
            }
            Cp | Cp0 => {
                let la = self.loc(ins.a, byte);
                let x = self.get(la, byte);
                let y = match (ins.op, ins.b) {
                    (Cp0, _) => 0,
                    (_, Opnd::None) => self.get(Loc::W(0), byte),
                    (_, b) => {
                        let lb = self.loc(b, byte);
                        self.get(lb, byte)
                    }
                };
                self.add_flags(x, !y, 1, byte);
            }
            Bra => {
                if self.cond(ins.cond) {
                    next = ins.target;
                    cyc = 2;
                }
            }
            Goto => {
                next = ins.target;
                cyc = 2;
            }
            Call | Rcall => {
                let ret = next;
                self.push16(ret as u16);
                self.push16((ret >> 16) as u16);
                next = ins.target;
                cyc = 2;
            }
            Return | Retlw => {
                if ins.op == Retlw
                    && let Opnd::W(n, _) = ins.b
                {
                    self.w[n as usize] = if byte {
                        (self.w[n as usize] & 0xFF00) | (ins.target as u16 & 0xFF)
                    } else {
                        ins.target as u16
                    };
                }
                let hi = self.pop16() as u32;
                let lo = self.pop16() as u32;
                next = hi << 16 | lo;
                cyc = 3;
            }
            Repeat => {
                self.pc = next;
                self.repeat = ins.target + 1;
                self.timer1(1);
                self.cycles += 1;
                return 1;
            }
            Push => {
                let l = self.loc(ins.a, false);
                let v = self.get(l, false);
                self.push16(v);
            }
            Pop => {
                let v = self.pop16();
                let l = self.loc(ins.a, false);
                self.put(l, v, false);
            }
            MulUu | MulSs => {
                let (lb, ls) = (self.loc(ins.a, false), self.loc(ins.b, false));
                let (x, y) = (self.get(lb, false), self.get(ls, false));
                let p = if ins.op == MulUu {
                    x as u32 * y as u32
                } else {
                    ((x as i16 as i32) * (y as i16 as i32)) as u32
                };
                if let Opnd::W(n, _) = ins.c {
                    self.w[n as usize & 14] = p as u16;
                    self.w[(n as usize & 14) + 1] = (p >> 16) as u16;
                }
            }
            Nop => {}
        }
        if self.repeat > 0 {
            self.repeat -= 1;
            if self.repeat > 0 {
                next = self.pc; // execute again
            }
        }
        self.pc = next;
        self.timer1(cyc);
        self.cycles += cyc as u64;
        cyc
    }
}

// ------------------------------------------------------------ assembler

fn wreg(s: &str) -> Option<u8> {
    let l = s.trim().to_ascii_lowercase();
    if l == "wreg" {
        return Some(0);
    }
    let n = l.strip_prefix('w')?.parse::<u8>().ok()?;
    (n < 16).then_some(n)
}

fn opnd(s: &str, cx: &Ctx<'_>, errs: &mut Vec<String>) -> Opnd {
    let t = s.trim();
    if let Some(lit) = t.strip_prefix('#') {
        return Opnd::Lit(asm::eval(lit, cx).unwrap_or_else(|e| {
            errs.push(e);
            0
        }));
    }
    if let Some(n) = wreg(t) {
        return Opnd::W(n, Mode::Direct);
    }
    if let Some(inner) = t.strip_prefix('[').and_then(|x| x.strip_suffix(']')) {
        let i = inner.trim();
        let (body, mode) = if let Some(r) = i.strip_suffix("++") {
            (r, Mode::PostInc)
        } else if let Some(r) = i.strip_suffix("--") {
            (r, Mode::PostDec)
        } else if let Some(r) = i.strip_prefix("++") {
            (r, Mode::PreInc)
        } else if let Some(r) = i.strip_prefix("--") {
            (r, Mode::PreDec)
        } else if let Some((r, k)) = i.split_once('+') {
            let k = asm::eval(k.trim().trim_start_matches('#'), cx).unwrap_or_else(|e| {
                errs.push(e);
                0
            });
            (r, Mode::Offset(k as i16))
        } else if let Some((r, k)) = i.split_once('-') {
            let k = asm::eval(k.trim().trim_start_matches('#'), cx).unwrap_or_else(|e| {
                errs.push(e);
                0
            });
            (r, Mode::Offset(-(k as i16)))
        } else {
            (i, Mode::Plain)
        };
        return match wreg(body) {
            Some(n) => Opnd::W(n, mode),
            None => {
                errs.push(format!("bad indirect operand `{t}`"));
                Opnd::None
            }
        };
    }
    match asm::eval(t, cx) {
        Ok(v) => Opnd::File(v as u16),
        Err(e) => {
            errs.push(e);
            Opnd::None
        }
    }
}

fn sfr_symbols() -> Symbols {
    let mut s = Symbols::default();
    for (n, v) in [
        ("WREG", 0),
        ("SR", 0x42),
        ("TMR1", 0x100),
        ("PR1", 0x102),
        ("T1CON", 0x104),
        ("IFS0", 0x84),
        ("TRISA", 0x2C0),
        ("PORTA", 0x2C2),
        ("LATA", 0x2C4),
        ("ODCA", 0x2C6),
        ("TRISB", 0x2C8),
        ("PORTB", 0x2CA),
        ("LATB", 0x2CC),
        ("ODCB", 0x2CE),
        ("AD1PCFG", 0x32C),
        ("T1IF", 3),
        ("TON", 15),
        ("TCKPS0", 4),
        ("TCKPS1", 5),
    ] {
        s.set(n, v);
    }
    for k in 0..16 {
        s.set(&format!("WREG{k}"), 2 * k);
    }
    s
}

const DOCS: [(&str, &str); 41] = [
    (
        "mov",
        "mov #lit16, Wn · mov Ws, Wd · mov f, Wn · mov Wn, f · mov.b",
    ),
    (
        "add",
        "add Wb, Ws, Wd · add Wb, #lit5, Wd · add #lit10, Wn · add f [, WREG]",
    ),
    ("addc", "addc — add with carry (forms of add)"),
    (
        "sub",
        "sub Wb, Ws, Wd (Wb − Ws) · sub #lit10, Wn (Wn − lit) · sub f (f − W0)",
    ),
    ("subb", "subb — subtract with borrow"),
    ("subr", "subr Wb, Ws, Wd — Ws − Wb"),
    ("and", "and Wb, Ws, Wd · and #lit10, Wn · and f"),
    ("ior", "ior — inclusive OR (forms of and)"),
    ("xor", "xor — exclusive OR (forms of and)"),
    ("inc", "inc Ws, Wd · inc f [, WREG]"),
    ("inc2", "inc2 — add 2"),
    ("dec", "dec Ws, Wd · dec f [, WREG]"),
    ("dec2", "dec2 — subtract 2"),
    ("com", "com Ws, Wd — complement"),
    ("neg", "neg Ws, Wd — negate"),
    ("clr", "clr Wd · clr f"),
    ("setm", "setm Wd · setm f — all ones"),
    ("sl", "sl Ws, Wd · sl Wb, #lit4, Wnd · sl Wb, Wns, Wnd"),
    ("lsr", "lsr — logical shift right (forms of sl)"),
    ("asr", "asr — arithmetic shift right (forms of sl)"),
    ("bset", "bset f, #bit · bset Ws, #bit"),
    ("bclr", "bclr f, #bit · bclr Ws, #bit"),
    ("btg", "btg f, #bit — toggle a bit"),
    ("btsc", "btsc f, #bit — skip if clear"),
    ("btss", "btss f, #bit — skip if set"),
    (
        "cp",
        "cp Wb, Ws · cp Wb, #lit5 · cp f — compare (flags of Wb − Ws)",
    ),
    ("cp0", "cp0 Ws · cp0 f — compare with zero"),
    (
        "bra",
        "bra label · bra cond, label (z nz c nc n nn ov nov ge lt gt le gtu leu)",
    ),
    ("goto", "goto label"),
    ("call", "call label"),
    ("rcall", "rcall label"),
    ("return", "return"),
    ("retlw", "retlw #lit10, Wn — return with a literal"),
    (
        "repeat",
        "repeat #lit — run the next instruction lit + 1 times",
    ),
    ("push", "push Ws · push f"),
    ("pop", "pop Wd · pop f"),
    (
        "mul.uu",
        "mul.uu Wb, Ws, Wnd — unsigned 32-bit product in Wnd:Wnd+1",
    ),
    ("mul.ss", "mul.ss Wb, Ws, Wnd — signed product"),
    ("swap", "swap Wn — swap bytes"),
    ("ze", "ze Ws, Wnd — zero-extend byte"),
    ("nop", "nop"),
];

/// Assemble XC16-style source.
pub fn assemble(_dev: &Device, src: &str) -> Result<Program, Vec<AsmError>> {
    struct St {
        line: usize,
        op: String,
        args: Vec<String>,
        labels: Vec<String>,
    }
    let mut stmts: Vec<St> = Vec::new();
    let mut pending: Vec<String> = Vec::new();
    for (k, raw) in src.lines().enumerate() {
        let mut rest = asm::strip_comment(raw, &[";"]).trim();
        while let Some(colon) = rest.find(':') {
            let name = rest[..colon].trim();
            if name.is_empty()
                || !name
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
            {
                break;
            }
            pending.push(name.to_string());
            rest = rest[colon + 1..].trim();
        }
        if rest.is_empty() {
            continue;
        }
        let mut it = rest.splitn(2, char::is_whitespace);
        let op = it.next().unwrap_or("").to_ascii_lowercase();
        let args = asm::split_operands(it.next().unwrap_or(""));
        stmts.push(St {
            line: k + 1,
            op,
            args,
            labels: std::mem::take(&mut pending),
        });
    }
    let mut syms = sfr_symbols();
    let mut errors = Vec::new();
    let mut slots: Vec<Option<Ins>> = Vec::new();
    let mut data: Vec<(u16, u8)> = Vec::new();
    let mut ram_top = RAM_START;
    for pass in 0..2 {
        let mut pc = TEXT_BASE;
        let mut dp = RAM_START;
        let mut in_data = false;
        slots.clear();
        data.clear();
        for st in &stmts {
            for l in &st.labels {
                if pass == 0 && syms.contains(l) {
                    errors.push(AsmError {
                        line: st.line,
                        message: format!("label `{l}` defined twice"),
                    });
                }
                syms.set(l, if in_data { dp as i64 } else { pc as i64 });
            }
            let cx = Ctx {
                syms: &syms,
                radix: 10,
                here: pc as i64,
                lenient: pass == 0,
            };
            let mut errs: Vec<String> = Vec::new();
            let (base, byte) = match st.op.split_once('.') {
                Some((b, "b")) => (b.to_string(), true),
                Some((b, "w")) => (b.to_string(), false),
                _ => (st.op.clone(), false),
            };
            if st.op.starts_with('.') {
                match st.op.as_str() {
                    ".text" => in_data = false,
                    ".bss" | ".data" | ".nbss" | ".ndata" => in_data = true,
                    ".section" => {
                        in_data = st
                            .args
                            .first()
                            .is_some_and(|a| a.contains("bss") || a.contains("data"))
                    }
                    ".equ" | ".set" if st.args.len() == 2 => {
                        let v = asm::eval(&st.args[1], &cx).unwrap_or_else(|e| {
                            errs.push(e);
                            0
                        });
                        syms.set(&st.args[0], v);
                    }
                    ".space" | ".skip" => {
                        let n = st
                            .args
                            .first()
                            .map_or(0, |a| asm::eval(a, &cx).unwrap_or(0))
                            as u16;
                        dp = dp.wrapping_add(n);
                    }
                    ".word" | ".byte" => {
                        let size = if st.op == ".word" { 2 } else { 1 };
                        for a in &st.args {
                            let v = asm::eval(a, &cx).unwrap_or_else(|e| {
                                errs.push(e);
                                0
                            });
                            for (j, b) in (v as u16).to_le_bytes()[..size].iter().enumerate() {
                                data.push((dp + j as u16, *b));
                            }
                            dp += size as u16;
                        }
                    }
                    ".align" if dp % 2 == 1 => dp += 1,
                    _ => {}
                }
                for e in errs {
                    if pass == 1 {
                        errors.push(AsmError {
                            line: st.line,
                            message: e,
                        });
                    }
                }
                continue;
            }
            let a = |k: usize, errs: &mut Vec<String>| match st.args.get(k) {
                Some(s) => opnd(s, &cx, errs),
                None => Opnd::None,
            };
            let mut ins = Ins {
                op: Op::Nop,
                byte,
                a: Opnd::None,
                b: Opnd::None,
                c: Opnd::None,
                cond: Cond::Always,
                target: 0,
                words: 1,
            };
            let target = |s: &str, errs: &mut Vec<String>| {
                asm::eval(s, &cx).unwrap_or_else(|e| {
                    errs.push(e);
                    0
                }) as u32
            };
            use Op::*;
            let op = match base.as_str() {
                "mov" => Some(Mov),
                "add" => Some(Add),
                "addc" => Some(Addc),
                "sub" => Some(Sub),
                "subb" => Some(Subb),
                "subr" => Some(Subr),
                "and" => Some(And),
                "ior" => Some(Ior),
                "xor" => Some(Xor),
                "inc" => Some(Inc),
                "inc2" => Some(Inc2),
                "dec" => Some(Dec),
                "dec2" => Some(Dec2),
                "com" => Some(Com),
                "neg" => Some(Neg),
                "clr" => Some(Clr),
                "setm" => Some(Setm),
                "sl" => Some(Sl),
                "lsr" => Some(Lsr),
                "asr" => Some(Asr),
                "bset" => Some(Bset),
                "bclr" => Some(Bclr),
                "btg" => Some(Btg),
                "btsc" => Some(Btsc),
                "btss" => Some(Btss),
                "cp" => Some(Cp),
                "cp0" => Some(Cp0),
                "bra" => Some(Bra),
                "goto" => Some(Goto),
                "call" => Some(Call),
                "rcall" => Some(Rcall),
                "return" => Some(Return),
                "retlw" => Some(Retlw),
                "repeat" => Some(Repeat),
                "push" => Some(Push),
                "pop" => Some(Pop),
                "mul.uu" => Some(MulUu),
                "mul.ss" => Some(MulSs),
                "swap" => Some(Swap),
                "ze" => Some(Ze),
                "se" => Some(Se),
                "nop" => Some(Nop),
                _ => None,
            };
            let Some(o) = op else {
                if pass == 1 {
                    errors.push(AsmError {
                        line: st.line,
                        message: format!("unknown or unsupported instruction `{}`", st.op),
                    });
                }
                slots.push(None);
                pc += 2;
                continue;
            };
            ins.op = o;
            if matches!(o, MulUu | MulSs) {
                ins.byte = false;
            }
            match o {
                Bra => {
                    if st.args.len() == 2 {
                        ins.cond = match st.args[0].to_ascii_lowercase().as_str() {
                            "z" => Cond::Z,
                            "nz" => Cond::Nz,
                            "c" | "geu" => Cond::C,
                            "nc" | "ltu" => Cond::Nc,
                            "n" => Cond::N,
                            "nn" => Cond::Nn,
                            "ov" => Cond::Ov,
                            "nov" => Cond::Nov,
                            "ge" => Cond::Ge,
                            "lt" => Cond::Lt,
                            "gt" => Cond::Gt,
                            "le" => Cond::Le,
                            "gtu" => Cond::Gtu,
                            "leu" => Cond::Leu,
                            other => {
                                errs.push(format!("unknown condition `{other}`"));
                                Cond::Always
                            }
                        };
                        ins.target = target(&st.args[1], &mut errs);
                    } else {
                        ins.target = target(st.args.first().map_or("", |s| s.as_str()), &mut errs);
                    }
                }
                Goto | Call => {
                    ins.target = target(st.args.first().map_or("", |s| s.as_str()), &mut errs);
                    ins.words = 2;
                }
                Rcall => ins.target = target(st.args.first().map_or("", |s| s.as_str()), &mut errs),
                Return | Nop => {}
                Retlw | Repeat => {
                    match a(0, &mut errs) {
                        Opnd::Lit(k) => ins.target = k as u32,
                        _ => errs.push(format!("{} needs a #literal", st.op)),
                    }
                    ins.b = a(1, &mut errs);
                }
                Bset | Bclr | Btg | Btsc | Btss => {
                    ins.a = a(0, &mut errs);
                    match a(1, &mut errs) {
                        Opnd::Lit(k) if (0..16).contains(&k) => ins.target = k as u32,
                        _ => errs.push("bit must be #0 to #15".into()),
                    }
                    // A file bit above 7 in byte mode means the odd byte.
                    if !byte && let Opnd::File(_) = ins.a {
                        ins.byte = false;
                    }
                }
                _ => {
                    ins.a = a(0, &mut errs);
                    ins.b = a(1, &mut errs);
                    ins.c = a(2, &mut errs);
                    if o == Mov && matches!((ins.a, ins.b), (Opnd::Lit(_), Opnd::File(_))) {
                        errs.push(
                            "mov #lit, f is not an instruction: load a W register first".into(),
                        );
                    }
                    if matches!(o, Mov) && ins.a == Opnd::None {
                        errs.push("mov needs operands".into());
                    }
                }
            }
            for e in errs {
                if pass == 1 {
                    errors.push(AsmError {
                        line: st.line,
                        message: e,
                    });
                }
            }
            slots.push(Some(ins));
            if ins.words == 2 {
                slots.push(None);
            }
            pc += 2 * ins.words;
        }
        ram_top = dp;
    }
    if slots.is_empty() {
        errors.push(AsmError {
            line: 1,
            message: "no instructions".into(),
        });
    }
    if !errors.is_empty() {
        errors.sort_by_key(|e| e.line);
        return Err(errors);
    }
    let entry = ["__reset", "main", "_main"]
        .iter()
        .find_map(|n| syms.get(n))
        .map_or(TEXT_BASE, |v| v as u32);
    Ok(Program {
        slots,
        data,
        entry,
        stack: (ram_top + 1) & !1,
    })
}

/// Editor vocabulary.
pub fn vocabulary(_dev: &Device) -> Vec<(String, String, bool)> {
    let mut v: Vec<(String, String, bool)> = DOCS
        .iter()
        .map(|(k, d)| (k.to_string(), d.to_string(), true))
        .collect();
    for d in [
        ".text", ".bss", ".data", ".space", ".word", ".equ", ".global", ".include", ".end",
    ] {
        v.push((d.to_string(), format!("{d} — directive"), true));
    }
    for k in 0..16 {
        v.push((
            format!("w{k}"),
            if k == 15 {
                "W15: stack pointer".into()
            } else {
                "working register".into()
            },
            false,
        ));
    }
    let syms = sfr_symbols();
    let mut names: Vec<(&String, &i64)> = syms.names().collect();
    names.sort();
    for (n, val) in names {
        v.push((n.clone(), format!("{n} = 0x{val:04X}"), false));
    }
    v
}

/// A blink program.
pub fn template(dev: &Device) -> String {
    format!(
        "; {part}: blink RB0 every 250 ms (FRC 8 MHz: Fcy 4 MHz) while RB1 is low.
; XC16 syntax: #literal (decimal unless 0x), Wn registers, [Wn] indirect.
        .include \"p24fj64ga002.inc\"
        .global __reset
        .text
__reset:
        bclr    TRISB, #0           ; RB0 output
        ; Timer1: Fcy/256 = 15625 Hz; period 3906 = 250 ms
        mov     #3906, w0
        mov     w0, PR1
        mov     #0x8030, w0         ; TON, prescaler 1:256
        mov     w0, T1CON
loop:
        btsc    PORTB, #1           ; RB1 high: hold
        bra     loop
wait:
        btss    IFS0, #T1IF         ; period elapsed?
        bra     wait
        bclr    IFS0, #T1IF
        btg     LATB, #0            ; toggle RB0
        bra     loop
        .end
",
        part = dev.part
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device;

    fn cpu(src: &str) -> Pic24 {
        let d = device("mcu.pic24fj64ga002").unwrap();
        Pic24::new(&d, assemble(&d, src).unwrap_or_else(|e| panic!("{e:?}")))
    }

    #[test]
    fn arithmetic_addressing_and_calls() {
        let mut c = cpu("
        .bss
buf:    .space 8
        .text
__reset:
        mov #buf, w1
        mov #7, w2
        mov w2, [w1++]          ; buf[0] = 7
        mov #5, w3
        mov w3, [w1++]          ; buf[1] = 5
        mov #buf, w1
        mov [w1++], w4          ; 7
        sub w4, [w1], w5        ; 7 - 5 = 2
        add w5, #3, w6          ; 5
        add #10, w6             ; 15
        rcall double            ; w6 = 30
        mov #100, w0
        cp w0, #50
        bra gt, big
        mov #1, w7
big:    repeat #4
        inc w8, w8              ; 5 times
        mul.uu w6, w6, w10      ; 900
        bra $
double: add w6, w6, w6
        return
        ");
        for _ in 0..40 {
            c.step();
            assert!(c.fault().is_none(), "{:?}", c.fault());
        }
        assert_eq!(c.w[4], 7);
        assert_eq!(c.w[5], 2);
        assert_eq!(c.w[6], 30);
        assert_eq!(c.w[7], 0);
        assert_eq!(c.w[8], 5);
        assert_eq!(c.w[10], 900);
    }

    #[test]
    fn ports_bits_and_timer() {
        let mut c = cpu("
__reset:
        bclr TRISB, #3
        bset LATB, #3
        mov #9, w0
        mov w0, PR1
        mov #0x8000, w0
        mov w0, T1CON
wait:   btss IFS0, #3
        bra wait
        btg LATB, #3
        bra $
        ");
        for _ in 0..40 {
            c.step();
        }
        assert_eq!(c.ports[1].tris & (1 << 3), 0);
        assert_eq!(
            c.ports[1].lat & (1 << 3),
            0,
            "toggled back after the timer period"
        );
    }

    #[test]
    fn errors_and_template() {
        let d = device("mcu.pic24fj64ga002").unwrap();
        let e = assemble(
            &d,
            "__reset:\n  mov #1, LATB\n  frob w0\n  bra zz, __reset\n",
        )
        .unwrap_err();
        assert_eq!(
            e.iter().map(|x| x.line).collect::<Vec<_>>(),
            vec![2, 3, 4],
            "{e:?}"
        );
        assemble(&d, &template(&d)).unwrap_or_else(|e| panic!("{e:?}"));
    }
}
