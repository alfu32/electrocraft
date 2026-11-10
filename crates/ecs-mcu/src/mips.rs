//! PIC32 (MIPS32 M4K core): integer instruction set with branch delay
//! slots, HI/LO, CP0 Count (the core timer, SYSCLK/2), 32 KB RAM, and GPIO
//! registers with CLR/SET/INV aliases (PIC32MX1xx/2xx addresses).
//!
//! Assembler: GNU as syntax. `#` comments; labels `name:`; `.text`, `.data`,
//! `.bss`, `.word`, `.half`, `.byte`, `.ascii`, `.asciiz`, `.space`,
//! `.align`, `.equ`/`.set name, value`, `.set noreorder`/`reorder`;
//! `.globl`, `.ent`, `.end`, `.section`, `.type`, `.size` are ignored.
//! In the default `reorder` mode a NOP fills each branch delay slot.
//! Pseudo-instructions: li, la, move, not, neg, negu, b, beqz, bnez, blt,
//! bgt, ble, bge (and u variants), div/rem with three operands, immediate
//! forms of add/sub/and/or/xor/slt, loads and stores of a label.
//! Execution starts at `main` (or `_start`, or the first instruction);
//! returning from it stops the CPU.

use crate::asm::{self, AsmError, Ctx, Symbols};
use crate::{Cpu, Device, Port};

const TEXT_BASE: u32 = 0x9D00_0000;
const DATA_BASE: u32 = 0xA000_0000;
const RAM_SIZE: usize = 32 * 1024;
const HALT: u32 = 0xFFFF_FFF0;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Op {
    Add,
    Addu,
    Sub,
    Subu,
    And,
    Or,
    Xor,
    Nor,
    Slt,
    Sltu,
    Sll,
    Srl,
    Sra,
    Sllv,
    Srlv,
    Srav,
    Jr,
    Jalr,
    Mult,
    Multu,
    Div,
    Divu,
    Mfhi,
    Mflo,
    Mthi,
    Mtlo,
    Mul,
    Movn,
    Movz,
    Seb,
    Seh,
    Clz,
    Addi,
    Addiu,
    Andi,
    Ori,
    Xori,
    Slti,
    Sltiu,
    Lui,
    Lw,
    Lh,
    Lhu,
    Lb,
    Lbu,
    Sw,
    Sh,
    Sb,
    Beq,
    Bne,
    Blez,
    Bgtz,
    Bltz,
    Bgez,
    J,
    Jal,
    Mfc0,
    Mtc0,
    Nop,
    Ei,
    Di,
    Wait,
    Break,
}

#[derive(Debug, Clone, Copy)]
pub struct Ins {
    op: Op,
    rd: u8,
    rs: u8,
    rt: u8,
    /// Immediate, shift amount, or absolute branch/jump target.
    imm: i64,
}

#[derive(Debug, Clone)]
pub struct Program {
    text: Vec<Ins>,
    data: Vec<u8>,
    entry: u32,
}

pub struct Mips {
    p: Program,
    r: [u32; 32],
    hi: u32,
    lo: u32,
    pc: u32,
    npc: u32,
    ram: Vec<u8>,
    ports: Vec<Port>,
    ansel: [u32; 2],
    cycles: u64,
    count_offset: u32,
    compare: u32,
    halted: bool,
    fault: Option<String>,
}

impl Mips {
    pub fn new(dev: &Device, p: Program) -> Self {
        let mut ports = vec![Port::default(); dev.ports];
        for (_, port, bit) in &dev.pins {
            ports[*port].mask |= 1 << bit;
        }
        for p in &mut ports {
            p.tris = u32::MAX;
        }
        let mut ram = vec![0u8; RAM_SIZE];
        ram[..p.data.len().min(RAM_SIZE)].copy_from_slice(&p.data[..p.data.len().min(RAM_SIZE)]);
        let mut r = [0u32; 32];
        r[29] = 0x8000_0000 + RAM_SIZE as u32 - 16; // $sp
        r[31] = HALT; // $ra: returning from main stops
        let entry = p.entry;
        Self {
            p,
            r,
            hi: 0,
            lo: 0,
            pc: entry,
            npc: entry.wrapping_add(4),
            ram,
            ports,
            ansel: [u32::MAX; 2],
            cycles: 0,
            count_offset: 0,
            compare: 0,
            halted: false,
            fault: None,
        }
    }

    fn sfr(&mut self, phys: u32, write: Option<u32>) -> u32 {
        // GPIO: port n at 0x1F886000 + 0x100 n; registers ANSEL, TRIS, PORT, LAT, ODC.
        let off = phys.wrapping_sub(0x1F88_6000);
        let port = (off / 0x100) as usize;
        let reg = (off % 0x100) / 0x10;
        let alias = off % 0x10;
        if off < 0x200 && port < self.ports.len() {
            let cur = match reg {
                0 => self.ansel[port],
                1 => self.ports[port].tris,
                2 => self.ports[port].read(),
                3 => self.ports[port].lat,
                _ => 0,
            };
            if let Some(v) = write {
                // Writes to PORT go to the latch; CLR/SET/INV modify it.
                let base = if reg == 2 { self.ports[port].lat } else { cur };
                let new = match alias {
                    0x4 => base & !v,
                    0x8 => base | v,
                    0xC => base ^ v,
                    _ => v,
                };
                match reg {
                    0 => self.ansel[port] = new,
                    1 => self.ports[port].tris = new,
                    2 | 3 => self.ports[port].lat = new,
                    _ => {}
                }
            }
            return cur;
        }
        0
    }

    fn load(&mut self, addr: u32, size: u32) -> Option<u32> {
        if !addr.is_multiple_of(size) {
            self.fault = Some(format!(
                "unaligned {size}-byte load at 0x{addr:08X} (pc 0x{:08X})",
                self.pc
            ));
            return None;
        }
        let phys = addr & 0x1FFF_FFFF;
        if (phys as usize) < RAM_SIZE {
            let a = phys as usize;
            let b = &self.ram[a..a + size as usize];
            return Some(match size {
                1 => b[0] as u32,
                2 => u16::from_le_bytes([b[0], b[1]]) as u32,
                _ => u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            });
        }
        if (0x1F80_0000..0x1F90_0000).contains(&phys) {
            let word = self.sfr(phys & !3, None);
            return Some(
                (word >> ((phys & 3) * 8))
                    & if size == 4 {
                        u32::MAX
                    } else {
                        (1 << (size * 8)) - 1
                    },
            );
        }
        self.fault = Some(format!(
            "load from unmapped address 0x{addr:08X} (pc 0x{:08X})",
            self.pc
        ));
        None
    }

    fn store(&mut self, addr: u32, size: u32, v: u32) {
        if !addr.is_multiple_of(size) {
            self.fault = Some(format!(
                "unaligned {size}-byte store at 0x{addr:08X} (pc 0x{:08X})",
                self.pc
            ));
            return;
        }
        let phys = addr & 0x1FFF_FFFF;
        if (phys as usize) < RAM_SIZE {
            let a = phys as usize;
            let bytes = v.to_le_bytes();
            self.ram[a..a + size as usize].copy_from_slice(&bytes[..size as usize]);
            return;
        }
        if (0x1F80_0000..0x1F90_0000).contains(&phys) {
            self.sfr(phys & !3, Some(v));
            return;
        }
        self.fault = Some(format!(
            "store to unmapped address 0x{addr:08X} (pc 0x{:08X})",
            self.pc
        ));
    }
}

impl Cpu for Mips {
    fn ports(&mut self) -> &mut Vec<Port> {
        &mut self.ports
    }
    fn fault(&self) -> Option<String> {
        self.fault.clone()
    }
    fn summary(&self) -> String {
        format!(
            "PC {:08X}  v0 {:08X}  a0 {:08X}  sp {:08X}  cycles {}",
            self.pc, self.r[2], self.r[4], self.r[29], self.cycles
        )
    }

    fn step(&mut self) -> u32 {
        if self.halted || self.pc == HALT {
            self.halted = true;
            self.cycles += 1;
            return 1;
        }
        let k = self.pc.wrapping_sub(TEXT_BASE) / 4;
        let Some(ins) = self.p.text.get(k as usize).copied() else {
            self.fault = Some(format!("jump outside the program to 0x{:08X}", self.pc));
            return 1;
        };
        let (rs, rt) = (self.r[ins.rs as usize], self.r[ins.rt as usize]);
        let imm = ins.imm as i32 as u32;
        let mut next = self.npc.wrapping_add(4);
        let mut wr: Option<(u8, u32)> = None;
        let mut cyc = 1;
        use Op::*;
        match ins.op {
            Add | Addu => wr = Some((ins.rd, rs.wrapping_add(rt))),
            Sub | Subu => wr = Some((ins.rd, rs.wrapping_sub(rt))),
            And => wr = Some((ins.rd, rs & rt)),
            Or => wr = Some((ins.rd, rs | rt)),
            Xor => wr = Some((ins.rd, rs ^ rt)),
            Nor => wr = Some((ins.rd, !(rs | rt))),
            Slt => wr = Some((ins.rd, u32::from((rs as i32) < (rt as i32)))),
            Sltu => wr = Some((ins.rd, u32::from(rs < rt))),
            Sll => wr = Some((ins.rd, rt << (ins.imm & 31))),
            Srl => wr = Some((ins.rd, rt >> (ins.imm & 31))),
            Sra => wr = Some((ins.rd, ((rt as i32) >> (ins.imm & 31)) as u32)),
            Sllv => wr = Some((ins.rd, rt << (rs & 31))),
            Srlv => wr = Some((ins.rd, rt >> (rs & 31))),
            Srav => wr = Some((ins.rd, ((rt as i32) >> (rs & 31)) as u32)),
            Jr => next = rs,
            Jalr => {
                wr = Some((ins.rd, self.npc.wrapping_add(4)));
                next = rs;
            }
            Mult => {
                let p = (rs as i32 as i64) * (rt as i32 as i64);
                self.hi = (p >> 32) as u32;
                self.lo = p as u32;
            }
            Multu => {
                let p = rs as u64 * rt as u64;
                self.hi = (p >> 32) as u32;
                self.lo = p as u32;
            }
            Div => {
                if rt != 0 {
                    self.lo = (rs as i32).wrapping_div(rt as i32) as u32;
                    self.hi = (rs as i32).wrapping_rem(rt as i32) as u32;
                }
                cyc = 35;
            }
            Divu => {
                if let (Some(q), Some(r)) = (rs.checked_div(rt), rs.checked_rem(rt)) {
                    self.lo = q;
                    self.hi = r;
                }
                cyc = 35;
            }
            Mfhi => wr = Some((ins.rd, self.hi)),
            Mflo => wr = Some((ins.rd, self.lo)),
            Mthi => self.hi = rs,
            Mtlo => self.lo = rs,
            Mul => wr = Some((ins.rd, (rs as i32).wrapping_mul(rt as i32) as u32)),
            Movn => {
                if rt != 0 {
                    wr = Some((ins.rd, rs));
                }
            }
            Movz => {
                if rt == 0 {
                    wr = Some((ins.rd, rs));
                }
            }
            Seb => wr = Some((ins.rd, rt as u8 as i8 as i32 as u32)),
            Seh => wr = Some((ins.rd, rt as u16 as i16 as i32 as u32)),
            Clz => wr = Some((ins.rd, rs.leading_zeros())),
            Addi | Addiu => wr = Some((ins.rt, rs.wrapping_add(imm))),
            Andi => wr = Some((ins.rt, rs & (imm & 0xFFFF))),
            Ori => wr = Some((ins.rt, rs | (imm & 0xFFFF))),
            Xori => wr = Some((ins.rt, rs ^ (imm & 0xFFFF))),
            Slti => wr = Some((ins.rt, u32::from((rs as i32) < (imm as i32)))),
            Sltiu => wr = Some((ins.rt, u32::from(rs < imm))),
            Lui => wr = Some((ins.rt, (imm & 0xFFFF) << 16)),
            Lw | Lh | Lhu | Lb | Lbu => {
                let addr = rs.wrapping_add(imm);
                let size = match ins.op {
                    Lw => 4,
                    Lh | Lhu => 2,
                    _ => 1,
                };
                if let Some(v) = self.load(addr, size) {
                    let v = match ins.op {
                        Lh => v as u16 as i16 as i32 as u32,
                        Lb => v as u8 as i8 as i32 as u32,
                        _ => v,
                    };
                    wr = Some((ins.rt, v));
                }
            }
            Sw | Sh | Sb => {
                let size = match ins.op {
                    Sw => 4,
                    Sh => 2,
                    _ => 1,
                };
                self.store(rs.wrapping_add(imm), size, rt);
            }
            Beq | Bne | Blez | Bgtz | Bltz | Bgez => {
                let take = match ins.op {
                    Beq => rs == rt,
                    Bne => rs != rt,
                    Blez => (rs as i32) <= 0,
                    Bgtz => (rs as i32) > 0,
                    Bltz => (rs as i32) < 0,
                    _ => (rs as i32) >= 0,
                };
                if take {
                    next = ins.imm as u32;
                }
            }
            J => next = ins.imm as u32,
            Jal => {
                wr = Some((31, self.npc.wrapping_add(4)));
                next = ins.imm as u32;
            }
            Mfc0 => {
                let count = (self.cycles / 2) as u32 + self.count_offset;
                wr = Some((
                    ins.rt,
                    match ins.rd {
                        9 => count,
                        11 => self.compare,
                        15 => 0x0001_8765, // PRId of an M4K
                        _ => 0,
                    },
                ));
            }
            Mtc0 => match ins.rd {
                9 => self.count_offset = rt.wrapping_sub((self.cycles / 2) as u32),
                11 => self.compare = rt,
                _ => {}
            },
            Nop | Ei | Di => {}
            Wait => {}
            Break => self.fault = Some(format!("break at 0x{:08X}", self.pc)),
        }
        if let Some((reg, v)) = wr
            && reg != 0
        {
            self.r[reg as usize] = v;
        }
        self.pc = self.npc;
        self.npc = next;
        self.cycles += cyc as u64;
        cyc
    }
}

// ------------------------------------------------------------ assembler

const REGS: [&str; 32] = [
    "zero", "at", "v0", "v1", "a0", "a1", "a2", "a3", "t0", "t1", "t2", "t3", "t4", "t5", "t6",
    "t7", "s0", "s1", "s2", "s3", "s4", "s5", "s6", "s7", "t8", "t9", "k0", "k1", "gp", "sp", "fp",
    "ra",
];

fn reg(s: &str) -> Option<u8> {
    let r = s.trim().strip_prefix('$')?;
    if let Ok(n) = r.parse::<u8>() {
        return (n < 32).then_some(n);
    }
    if r == "s8" {
        return Some(30);
    }
    REGS.iter().position(|x| *x == r).map(|k| k as u8)
}

const DOCS: [(&str, &str); 70] = [
    ("add", "add rd, rs, rt — rd = rs + rt"),
    ("addu", "addu rd, rs, rt — rd = rs + rt (no trap)"),
    ("sub", "sub rd, rs, rt"),
    ("subu", "subu rd, rs, rt"),
    ("and", "and rd, rs, rt"),
    ("or", "or rd, rs, rt"),
    ("xor", "xor rd, rs, rt"),
    ("nor", "nor rd, rs, rt"),
    ("slt", "slt rd, rs, rt — rd = rs < rt (signed)"),
    ("sltu", "sltu rd, rs, rt — unsigned"),
    ("sll", "sll rd, rt, sa — shift left"),
    ("srl", "srl rd, rt, sa — logical right"),
    ("sra", "sra rd, rt, sa — arithmetic right"),
    ("sllv", "sllv rd, rt, rs"),
    ("srlv", "srlv rd, rt, rs"),
    ("srav", "srav rd, rt, rs"),
    ("jr", "jr rs — jump to register ($ra returns)"),
    ("jalr", "jalr rs — call register"),
    ("mult", "mult rs, rt — HI:LO = rs × rt"),
    ("multu", "multu rs, rt"),
    (
        "div",
        "div rs, rt — LO = quotient, HI = remainder (or div rd, rs, rt)",
    ),
    ("divu", "divu rs, rt"),
    ("rem", "rem rd, rs, rt — remainder (pseudo)"),
    ("mfhi", "mfhi rd"),
    ("mflo", "mflo rd"),
    ("mthi", "mthi rs"),
    ("mtlo", "mtlo rs"),
    ("mul", "mul rd, rs, rt — low 32 bits of the product"),
    ("movn", "movn rd, rs, rt — rd = rs if rt ≠ 0"),
    ("movz", "movz rd, rs, rt — rd = rs if rt = 0"),
    ("seb", "seb rd, rt — sign-extend byte"),
    ("seh", "seh rd, rt — sign-extend half"),
    ("clz", "clz rd, rs — count leading zeros"),
    ("addi", "addi rt, rs, imm"),
    ("addiu", "addiu rt, rs, imm"),
    ("andi", "andi rt, rs, imm16"),
    ("ori", "ori rt, rs, imm16"),
    ("xori", "xori rt, rs, imm16"),
    ("slti", "slti rt, rs, imm"),
    ("sltiu", "sltiu rt, rs, imm"),
    ("lui", "lui rt, imm16 — upper half"),
    ("lw", "lw rt, off(rs) — load word"),
    ("lh", "lh rt, off(rs)"),
    ("lhu", "lhu rt, off(rs)"),
    ("lb", "lb rt, off(rs)"),
    ("lbu", "lbu rt, off(rs)"),
    ("sw", "sw rt, off(rs) — store word"),
    ("sh", "sh rt, off(rs)"),
    ("sb", "sb rt, off(rs)"),
    ("beq", "beq rs, rt, label"),
    ("bne", "bne rs, rt, label"),
    ("blez", "blez rs, label"),
    ("bgtz", "bgtz rs, label"),
    ("bltz", "bltz rs, label"),
    ("bgez", "bgez rs, label"),
    ("j", "j label"),
    ("jal", "jal label — call"),
    ("mfc0", "mfc0 rt, $9 — read the core timer (SYSCLK/2)"),
    ("mtc0", "mtc0 rt, $9 — set the core timer"),
    ("nop", "nop"),
    ("ei", "ei — enable interrupts (no effect)"),
    ("di", "di — disable interrupts"),
    ("li", "li rt, value — load a constant (pseudo)"),
    ("la", "la rt, label — load an address (pseudo)"),
    ("move", "move rd, rs (pseudo)"),
    ("b", "b label — branch always (pseudo)"),
    ("beqz", "beqz rs, label (pseudo)"),
    ("bnez", "bnez rs, label (pseudo)"),
    (
        "blt",
        "blt rs, rt, label (pseudo; also bgt, ble, bge, bltu…)",
    ),
    ("not", "not rd, rs (pseudo)"),
];

fn sfr_symbols() -> Symbols {
    let mut s = Symbols::default();
    for (k, port) in ["A", "B"].iter().enumerate() {
        let base = 0xBF88_6000i64 + 0x100 * k as i64;
        for (j, reg) in ["ANSEL", "TRIS", "PORT", "LAT", "ODC"].iter().enumerate() {
            let a = base + 0x10 * j as i64;
            s.set(&format!("{reg}{port}"), a);
            s.set(&format!("{reg}{port}CLR"), a + 4);
            s.set(&format!("{reg}{port}SET"), a + 8);
            s.set(&format!("{reg}{port}INV"), a + 12);
        }
    }
    s
}

struct Stmt {
    line: usize,
    op: String,
    args: Vec<String>,
}

fn statements(src: &str) -> (Vec<(usize, String)>, Vec<Stmt>, Vec<AsmError>) {
    // Returns labels (by statement index), statements, errors.
    let mut labels = Vec::new();
    let mut out = Vec::new();
    let errors = Vec::new();
    for (k, raw) in src.lines().enumerate() {
        let text = asm::strip_comment(raw, &["#", "//"]);
        for part in text.split(';') {
            let mut rest = part.trim();
            // Labels.
            while let Some(colon) = rest.find(':') {
                let name = rest[..colon].trim();
                if name.is_empty()
                    || !name
                        .chars()
                        .all(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == '$')
                    || name.starts_with('$')
                {
                    break;
                }
                labels.push((out.len(), name.to_string()));
                rest = rest[colon + 1..].trim();
            }
            if rest.is_empty() {
                continue;
            }
            let mut it = rest.splitn(2, char::is_whitespace);
            let op = it.next().unwrap_or("").to_ascii_lowercase();
            let args = asm::split_operands(it.next().unwrap_or(""));
            out.push(Stmt {
                line: k + 1,
                op,
                args,
            });
        }
    }
    (labels, out, errors)
}

fn is_branch(op: &str) -> bool {
    matches!(
        op,
        "beq"
            | "bne"
            | "blez"
            | "bgtz"
            | "bltz"
            | "bgez"
            | "j"
            | "jal"
            | "jr"
            | "jalr"
            | "b"
            | "beqz"
            | "bnez"
            | "blt"
            | "bgt"
            | "ble"
            | "bge"
            | "bltu"
            | "bgtu"
            | "bleu"
            | "bgeu"
    )
}

/// Assemble GNU-style MIPS source.
pub fn assemble(_dev: &Device, src: &str) -> Result<Program, Vec<AsmError>> {
    let (labels, stmts, mut errors) = statements(src);
    let mut syms = sfr_symbols();
    // Sizes from pass one (pseudo-instructions whose length depends on values).
    let mut li_long: Vec<bool> = vec![true; stmts.len()];
    let mut text: Vec<Ins> = Vec::new();
    let mut data: Vec<u8> = Vec::new();
    for pass in 0..2 {
        let mut tpc = TEXT_BASE;
        let mut dpc = DATA_BASE;
        let mut in_data = false;
        let mut reorder = true;
        text.clear();
        data.clear();
        let mut li = 0;
        for (si, st) in stmts.iter().enumerate() {
            for (_, name) in labels.iter().filter(|(k, _)| *k == si) {
                if pass == 0 && syms.contains(name) {
                    errors.push(AsmError {
                        line: st.line,
                        message: format!("label `{name}` defined twice"),
                    });
                }
                syms.set(name, if in_data { dpc as i64 } else { tpc as i64 });
            }
            let cx = Ctx {
                syms: &syms,
                radix: 10,
                here: if in_data { dpc } else { tpc } as i64,
                lenient: pass == 0,
            };
            let mut errs: Vec<String> = Vec::new();
            let val = |s: &str, errs: &mut Vec<String>| -> i64 {
                asm::eval(s, &cx).unwrap_or_else(|e| {
                    errs.push(e);
                    0
                })
            };
            let r = |k: usize, errs: &mut Vec<String>| -> u8 {
                match st.args.get(k).and_then(|a| reg(a)) {
                    Some(r) => r,
                    None => {
                        errs.push(format!(
                            "{}: operand {} must be a register like $t0",
                            st.op,
                            k + 1
                        ));
                        0
                    }
                }
            };
            let mut emit: Vec<Ins> = Vec::new();
            let ins = |op: Op, rd: u8, rs: u8, rt: u8, imm: i64| Ins {
                op,
                rd,
                rs,
                rt,
                imm,
            };
            let op = st.op.as_str();
            // Directives.
            if op.starts_with('.') {
                match op {
                    ".text" => in_data = false,
                    ".data" | ".bss" | ".rodata" | ".sdata" => in_data = true,
                    ".section" => {
                        in_data = st
                            .args
                            .first()
                            .is_some_and(|a| a.contains("data") || a.contains("bss"))
                    }
                    ".set" if st.args.len() == 1 => match st.args[0].as_str() {
                        "noreorder" => reorder = false,
                        "reorder" => reorder = true,
                        _ => {}
                    },
                    ".set" | ".equ" | ".equiv" => {
                        if st.args.len() == 2 {
                            let v = val(&st.args[1], &mut errs);
                            syms.set(&st.args[0], v);
                        }
                    }
                    ".word" | ".half" | ".hword" | ".short" | ".byte" => {
                        let size = match op {
                            ".word" => 4,
                            ".byte" => 1,
                            _ => 2,
                        };
                        for a in &st.args {
                            let v = val(a, &mut errs) as u32;
                            if in_data {
                                data.extend_from_slice(&v.to_le_bytes()[..size]);
                                dpc += size as u32;
                            } else {
                                errs.push("data in .text: use .data".into());
                            }
                        }
                    }
                    ".ascii" | ".asciiz" | ".string" => {
                        for a in &st.args {
                            let s = a
                                .trim()
                                .trim_matches('"')
                                .replace("\\n", "\n")
                                .replace("\\0", "\0");
                            data.extend_from_slice(s.as_bytes());
                            dpc += s.len() as u32;
                            if op != ".ascii" {
                                data.push(0);
                                dpc += 1;
                            }
                        }
                    }
                    ".space" | ".skip" | ".zero" => {
                        let n = st.args.first().map_or(0, |a| val(a, &mut errs)) as usize;
                        data.extend(std::iter::repeat_n(0, n));
                        dpc += n as u32;
                    }
                    ".align" => {
                        let n = 1u32 << st.args.first().map_or(2, |a| val(a, &mut errs));
                        while in_data && !dpc.is_multiple_of(n) {
                            data.push(0);
                            dpc += 1;
                        }
                    }
                    _ => {} // .globl .ent .end .type .size .frame .mask ...
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
            if in_data {
                errors.push(AsmError {
                    line: st.line,
                    message: "instructions belong in .text".into(),
                });
                continue;
            }
            let a = |k: usize| st.args.get(k).map(String::as_str).unwrap_or("");
            let is_reg = |k: usize| reg(a(k)).is_some();
            // Memory operand: off(reg), (reg), or a label.
            let mem = |k: usize, errs: &mut Vec<String>| -> (Option<u8>, i64) {
                let s = a(k);
                if let Some(open) = s.rfind('(') {
                    let base = reg(&s[open + 1..s.len() - 1]);
                    if base.is_none() {
                        errs.push(format!("bad address `{s}`"));
                    }
                    let off = if s[..open].trim().is_empty() {
                        0
                    } else {
                        val(&s[..open], errs)
                    };
                    (base, off)
                } else {
                    (None, val(s, errs))
                }
            };
            let target = |k: usize, errs: &mut Vec<String>| val(a(k), errs);
            use Op::*;
            let simple3 = |o: Op| -> Option<Op> {
                Some(match o {
                    Add => Addi,
                    Addu => Addiu,
                    And => Andi,
                    Or => Ori,
                    Xor => Xori,
                    Slt => Slti,
                    Sltu => Sltiu,
                    _ => return None,
                })
            };
            let r3 = |name: &str| -> Option<Op> {
                Some(match name {
                    "add" => Add,
                    "addu" => Addu,
                    "sub" => Sub,
                    "subu" => Subu,
                    "and" => And,
                    "or" => Or,
                    "xor" => Xor,
                    "nor" => Nor,
                    "slt" => Slt,
                    "sltu" => Sltu,
                    "mul" => Mul,
                    "movn" => Movn,
                    "movz" => Movz,
                    _ => return None,
                })
            };
            match op {
                _ if r3(op).is_some() => {
                    let o = r3(op).unwrap();
                    let rd = r(0, &mut errs);
                    let rs = r(1, &mut errs);
                    if is_reg(2) {
                        emit.push(ins(o, rd, rs, r(2, &mut errs), 0));
                    } else if matches!(o, Sub | Subu) {
                        emit.push(ins(
                            if o == Sub { Addi } else { Addiu },
                            0,
                            rs,
                            rd,
                            -val(a(2), &mut errs),
                        ));
                    } else if let Some(io) = simple3(o) {
                        emit.push(ins(io, 0, rs, rd, val(a(2), &mut errs)));
                    } else {
                        errs.push(format!("{op} needs three registers"));
                    }
                }
                "sll" | "srl" | "sra" => {
                    let (rd, rt) = (r(0, &mut errs), r(1, &mut errs));
                    if is_reg(2) {
                        let o = match op {
                            "sll" => Sllv,
                            "srl" => Srlv,
                            _ => Srav,
                        };
                        emit.push(ins(o, rd, r(2, &mut errs), rt, 0));
                    } else {
                        let sa = val(a(2), &mut errs);
                        if !(0..32).contains(&sa) {
                            errs.push(format!("shift {sa} is not 0-31"));
                        }
                        let o = match op {
                            "sll" => Sll,
                            "srl" => Srl,
                            _ => Sra,
                        };
                        emit.push(ins(o, rd, 0, rt, sa));
                    }
                }
                "sllv" | "srlv" | "srav" => {
                    let o = match op {
                        "sllv" => Sllv,
                        "srlv" => Srlv,
                        _ => Srav,
                    };
                    emit.push(ins(o, r(0, &mut errs), r(2, &mut errs), r(1, &mut errs), 0));
                }
                "addi" | "addiu" | "andi" | "ori" | "xori" | "slti" | "sltiu" => {
                    let o = match op {
                        "addi" => Addi,
                        "addiu" => Addiu,
                        "andi" => Andi,
                        "ori" => Ori,
                        "xori" => Xori,
                        "slti" => Slti,
                        _ => Sltiu,
                    };
                    let v = val(a(2), &mut errs);
                    let unsigned = matches!(o, Andi | Ori | Xori);
                    if pass == 1
                        && ((unsigned && !(0..=0xFFFF).contains(&v))
                            || (!unsigned && !(-0x8000..=0x7FFF).contains(&v)))
                    {
                        errs.push(format!("immediate {v} does not fit in 16 bits (use li)"));
                    }
                    emit.push(ins(o, 0, r(1, &mut errs), r(0, &mut errs), v));
                }
                "lui" => emit.push(ins(Lui, 0, 0, r(0, &mut errs), val(a(1), &mut errs))),
                "lw" | "lh" | "lhu" | "lb" | "lbu" | "sw" | "sh" | "sb" => {
                    let o = match op {
                        "lw" => Lw,
                        "lh" => Lh,
                        "lhu" => Lhu,
                        "lb" => Lb,
                        "lbu" => Lbu,
                        "sw" => Sw,
                        "sh" => Sh,
                        _ => Sb,
                    };
                    let rt = r(0, &mut errs);
                    match mem(1, &mut errs) {
                        (Some(base), off) => emit.push(ins(o, 0, base, rt, off)),
                        (None, addr) => {
                            // Label: lui $at, %hi; op rt, %lo($at).
                            let lo = {
                                let l = addr & 0xFFFF;
                                if l >= 0x8000 { l - 0x10000 } else { l }
                            };
                            emit.push(ins(Lui, 0, 0, 1, ((addr - lo) >> 16) & 0xFFFF));
                            emit.push(ins(o, 0, 1, rt, lo));
                        }
                    }
                }
                "mult" | "multu" => emit.push(ins(
                    if op == "mult" { Mult } else { Multu },
                    0,
                    r(0, &mut errs),
                    r(1, &mut errs),
                    0,
                )),
                "div" | "divu" | "rem" | "remu" => {
                    let o = if op.ends_with('u') { Divu } else { Div };
                    if st.args.len() == 3 {
                        let (rd, rs, rt) = (r(0, &mut errs), r(1, &mut errs), r(2, &mut errs));
                        emit.push(ins(o, 0, rs, rt, 0));
                        emit.push(ins(
                            if op.starts_with("rem") { Mfhi } else { Mflo },
                            rd,
                            0,
                            0,
                            0,
                        ));
                    } else {
                        emit.push(ins(o, 0, r(0, &mut errs), r(1, &mut errs), 0));
                    }
                }
                "mfhi" | "mflo" => emit.push(ins(
                    if op == "mfhi" { Mfhi } else { Mflo },
                    r(0, &mut errs),
                    0,
                    0,
                    0,
                )),
                "mthi" | "mtlo" => emit.push(ins(
                    if op == "mthi" { Mthi } else { Mtlo },
                    0,
                    r(0, &mut errs),
                    0,
                    0,
                )),
                "seb" | "seh" => emit.push(ins(
                    if op == "seb" { Seb } else { Seh },
                    r(0, &mut errs),
                    0,
                    r(1, &mut errs),
                    0,
                )),
                "clz" => emit.push(ins(Clz, r(0, &mut errs), r(1, &mut errs), 0, 0)),
                "jr" => emit.push(ins(Jr, 0, r(0, &mut errs), 0, 0)),
                "jalr" => {
                    if st.args.len() == 2 {
                        emit.push(ins(Jalr, r(0, &mut errs), r(1, &mut errs), 0, 0));
                    } else {
                        emit.push(ins(Jalr, 31, r(0, &mut errs), 0, 0));
                    }
                }
                "j" | "jal" => emit.push(ins(
                    if op == "j" { J } else { Jal },
                    0,
                    0,
                    0,
                    target(0, &mut errs),
                )),
                "b" => emit.push(ins(Beq, 0, 0, 0, target(0, &mut errs))),
                "beq" | "bne" => {
                    let (rs, t) = (r(0, &mut errs), target(2, &mut errs));
                    let o = if op == "beq" { Beq } else { Bne };
                    if is_reg(1) {
                        emit.push(ins(o, 0, rs, r(1, &mut errs), t));
                    } else {
                        // beq rs, imm, label: li $at, imm first.
                        emit.push(ins(Addiu, 0, 0, 1, val(a(1), &mut errs)));
                        emit.push(ins(o, 0, rs, 1, t));
                    }
                }
                "beqz" | "bnez" => emit.push(ins(
                    if op == "beqz" { Beq } else { Bne },
                    0,
                    r(0, &mut errs),
                    0,
                    target(1, &mut errs),
                )),
                "blez" | "bgtz" | "bltz" | "bgez" => {
                    let o = match op {
                        "blez" => Blez,
                        "bgtz" => Bgtz,
                        "bltz" => Bltz,
                        _ => Bgez,
                    };
                    emit.push(ins(o, 0, r(0, &mut errs), 0, target(1, &mut errs)));
                }
                "blt" | "bgt" | "ble" | "bge" | "bltu" | "bgtu" | "bleu" | "bgeu" => {
                    let (x, y) = (r(0, &mut errs), r(1, &mut errs));
                    let t = target(2, &mut errs);
                    let u = op.ends_with('u');
                    let cmp = if u { Sltu } else { Slt };
                    let base = op.trim_end_matches('u');
                    // blt: x<y; bgt: y<x; bge: !(x<y); ble: !(y<x).
                    let (p, q, taken_if_set) = match base {
                        "blt" => (x, y, true),
                        "bgt" => (y, x, true),
                        "bge" => (x, y, false),
                        _ => (y, x, false),
                    };
                    emit.push(ins(cmp, 1, p, q, 0));
                    emit.push(ins(if taken_if_set { Bne } else { Beq }, 0, 1, 0, t));
                }
                "li" => {
                    let rt = r(0, &mut errs);
                    let v = val(a(1), &mut errs);
                    if pass == 0 {
                        // Known now and small: one instruction.
                        let strict = Ctx {
                            syms: &syms,
                            radix: 10,
                            here: 0,
                            lenient: false,
                        };
                        li_long[si] = match asm::eval(a(1), &strict) {
                            Ok(v) => !((-0x8000..=0xFFFF).contains(&v)),
                            Err(_) => true,
                        };
                    }
                    if li_long[si] {
                        emit.push(ins(Lui, 0, 0, rt, (v >> 16) & 0xFFFF));
                        emit.push(ins(Ori, 0, rt, rt, v & 0xFFFF));
                    } else if v < 0 {
                        emit.push(ins(Addiu, 0, 0, rt, v));
                    } else {
                        emit.push(ins(Ori, 0, 0, rt, v));
                    }
                    li += 1;
                }
                "la" => {
                    let rt = r(0, &mut errs);
                    let v = val(a(1), &mut errs);
                    emit.push(ins(Lui, 0, 0, rt, (v >> 16) & 0xFFFF));
                    emit.push(ins(Ori, 0, rt, rt, v & 0xFFFF));
                }
                "move" => emit.push(ins(Addu, r(0, &mut errs), r(1, &mut errs), 0, 0)),
                "not" => emit.push(ins(Nor, r(0, &mut errs), r(1, &mut errs), 0, 0)),
                "neg" | "negu" => emit.push(ins(Subu, r(0, &mut errs), 0, r(1, &mut errs), 0)),
                "mfc0" | "mtc0" => {
                    let rt = r(0, &mut errs);
                    let rd = reg(a(1)).unwrap_or_else(|| {
                        errs.push("mfc0/mtc0 need a CP0 register like $9".into());
                        0
                    });
                    emit.push(ins(if op == "mfc0" { Mfc0 } else { Mtc0 }, rd, 0, rt, 0));
                }
                "nop" | "ehb" | "ssnop" => emit.push(ins(Nop, 0, 0, 0, 0)),
                "ei" => emit.push(ins(Ei, 0, 0, 0, 0)),
                "di" => emit.push(ins(Di, 0, 0, 0, 0)),
                "wait" => emit.push(ins(Wait, 0, 0, 0, 0)),
                "break" | "sdbbp" | "syscall" => emit.push(ins(Break, 0, 0, 0, 0)),
                _ => errs.push(format!("unknown instruction `{op}`")),
            }
            if reorder && is_branch(op) {
                emit.push(ins(Nop, 0, 0, 0, 0));
            }
            for e in errs {
                if pass == 1 {
                    errors.push(AsmError {
                        line: st.line,
                        message: e,
                    });
                }
            }
            for i in emit {
                text.push(i);
                tpc += 4;
            }
        }
        let _ = li;
    }
    if text.is_empty() {
        errors.push(AsmError {
            line: 1,
            message: "no instructions".into(),
        });
    }
    // Branch targets must be in the program.
    let end = TEXT_BASE + 4 * text.len() as u32;
    for i in &text {
        if matches!(
            i.op,
            Op::Beq | Op::Bne | Op::Blez | Op::Bgtz | Op::Bltz | Op::Bgez | Op::J | Op::Jal
        ) && !(TEXT_BASE..end).contains(&(i.imm as u32))
        {
            errors.push(AsmError {
                line: 0,
                message: format!("branch to 0x{:08X} is outside the program", i.imm as u32),
            });
            break;
        }
    }
    if !errors.is_empty() {
        errors.sort_by_key(|e| e.line);
        return Err(errors);
    }
    let entry = ["main", "_start", "__reset"]
        .iter()
        .find_map(|n| syms.get(n))
        .map_or(TEXT_BASE, |v| v as u32);
    Ok(Program { text, data, entry })
}

/// Editor vocabulary.
pub fn vocabulary(_dev: &Device) -> Vec<(String, String, bool)> {
    let mut v: Vec<(String, String, bool)> = DOCS
        .iter()
        .map(|(k, d)| (k.to_string(), d.to_string(), true))
        .collect();
    for d in [
        ".text", ".data", ".word", ".byte", ".half", ".ascii", ".asciiz", ".space", ".align",
        ".equ", ".set", ".globl",
    ] {
        v.push((d.to_string(), format!("{d} — directive"), true));
    }
    for r in REGS {
        v.push((format!("${r}"), "register".into(), false));
    }
    let syms = sfr_symbols();
    let mut names: Vec<(&String, &i64)> = syms.names().collect();
    names.sort();
    for (n, val) in names {
        v.push((n.clone(), format!("{n} = 0x{val:08X}"), false));
    }
    v
}

/// A blink program.
pub fn template(dev: &Device) -> String {
    format!(
        "# {part}: blink RB0 every 250 ms (40 MHz SYSCLK) while RB1 is low.
# GNU as syntax; a NOP fills each branch delay slot (.set reorder).
        .text
        .globl  main
main:
        li      $t0, 1              # RB0
        la      $t1, TRISBCLR
        sw      $t0, 0($t1)         # RB0 output
        la      $t2, LATBINV
        la      $t3, PORTB
loop:
        lw      $t4, 0($t3)
        andi    $t4, $t4, 2         # RB1
        bnez    $t4, loop           # RB1 high: hold
        sw      $t0, 0($t2)         # toggle RB0
        jal     delay250
        b       loop

# 250 ms on the core timer (SYSCLK/2 = 20 MHz: 5,000,000 counts)
delay250:
        mfc0    $t5, $9
        li      $t6, 5000000
wait:   mfc0    $t7, $9
        subu    $t7, $t7, $t5
        sltu    $t7, $t7, $t6
        bnez    $t7, wait
        jr      $ra
",
        part = dev.part
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device;

    fn cpu(src: &str) -> Mips {
        let d = device("mcu.pic32mx250f128b").unwrap();
        let p = assemble(&d, src).unwrap_or_else(|e| panic!("{e:?}"));
        Mips::new(&d, p)
    }

    #[test]
    fn arithmetic_memory_and_delay_slots() {
        let mut c = cpu("
        .data
nums:   .word 3, 4, 5
        .text
main:   move $s7, $ra           # jal below overwrites $ra
        la $t0, nums
        lw $t1, 0($t0)
        lw $t2, 4($t0)
        mul $t3, $t1, $t2       # 12
        li $t4, 0x12345678
        li $t5, -2
        sra $t6, $t5, 1         # -1
        div $t7, $t3, $t1       # 4
        sw $t3, 8($t0)
        lw $s0, nums+8
        blt $t1, $t2, less
        li $s1, 99
less:   jal f
        addiu $s2, $s2, 1       # runs after f returns
        jr $s7
f:      li $s2, 10
        jr $ra
        ");
        for _ in 0..80 {
            c.step();
            assert!(c.fault().is_none(), "{:?}", c.fault());
        }
        assert_eq!(c.r[11], 12);
        assert_eq!(c.r[12], 0x1234_5678);
        assert_eq!(c.r[14], u32::MAX);
        assert_eq!(c.r[15], 4);
        assert_eq!(c.r[16], 12);
        assert_eq!(c.r[17], 0);
        assert_eq!(c.r[18], 11);
        assert!(c.halted);
    }

    #[test]
    fn noreorder_executes_the_delay_slot() {
        let mut c = cpu("
        .set noreorder
main:   li $t0, 0
        b skip
        addiu $t0, $t0, 1       # delay slot: runs
        addiu $t0, $t0, 100     # skipped
skip:   nop
        jr $ra
        nop
        ");
        for _ in 0..10 {
            c.step();
        }
        assert_eq!(c.r[8], 1);
    }

    #[test]
    fn gpio_set_clear_invert() {
        let mut c = cpu("
main:   li $t0, 0x21
        la $t1, TRISBCLR
        sw $t0, 0($t1)
        la $t1, LATBSET
        sw $t0, 0($t1)
        li $t0, 1
        la $t1, LATBINV
        sw $t0, 0($t1)
        jr $ra
        ");
        for _ in 0..30 {
            c.step();
        }
        let p = c.ports[1];
        assert_eq!(p.tris & 0xFFFF, 0xFFDE);
        assert_eq!(p.lat & 0xFFFF, 0x20);
    }

    #[test]
    fn errors_and_template() {
        let d = device("mcu.pic32mx250f128b").unwrap();
        let e = assemble(
            &d,
            "main: addu $t0, $t1\n frob $t0\n addi $t0, $t0, 70000\n",
        )
        .unwrap_err();
        assert_eq!(
            e.iter().map(|x| x.line).collect::<Vec<_>>(),
            vec![1, 2, 3],
            "{e:?}"
        );
        assemble(&d, &template(&d)).unwrap_or_else(|e| panic!("{e:?}"));
    }
}
