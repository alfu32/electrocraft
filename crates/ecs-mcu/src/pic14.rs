//! 8-bit PIC mid-range core (14-bit instructions): PIC12F675, PIC16F84A,
//! PIC16F628A, PIC16F877A. Real instruction encodings, so computed gotos
//! (`ADDWF PCL,F`) and `DT` tables work as on the chip.
//!
//! Assembler: MPASM syntax. Labels start in column 1 (a trailing `:` is
//! optional); `;` comments; default radix **hex** as in MPASM (`radix dec`
//! changes it; write `D'10'`, `.10`, `0x0A`, `B'1010'`); directives `ORG`,
//! `EQU`, `SET`, `CBLOCK`/`ENDC`, `#define`, `DT`, `DW`, `BANKSEL`,
//! `PAGESEL`, `END`; `LIST`, `#include`, `__CONFIG`, `PROCESSOR`,
//! `ERRORLEVEL` are accepted and ignored (register names are built in).

use crate::asm::{self, AsmError, Ctx, Symbols};
use crate::{Cpu, Device, Port};

#[derive(Debug, Clone)]
pub struct Program {
    pub words: Vec<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Model {
    F675,
    F84A,
    F628A,
    F877A,
}

fn model(dev: &Device) -> Model {
    match dev.id {
        "mcu.pic12f675" => Model::F675,
        "mcu.pic16f84a" => Model::F84A,
        "mcu.pic16f628a" => Model::F628A,
        _ => Model::F877A,
    }
}

fn prog_size(m: Model) -> usize {
    match m {
        Model::F675 | Model::F84A => 1024,
        Model::F628A => 2048,
        Model::F877A => 8192,
    }
}

fn four_banks(m: Model) -> bool {
    matches!(m, Model::F628A | Model::F877A)
}

/// Port register address and TRIS address per port index.
fn port_regs(m: Model) -> Vec<(u16, u16)> {
    match m {
        Model::F675 => vec![(0x05, 0x85)],
        Model::F84A | Model::F628A => vec![(0x05, 0x85), (0x06, 0x86)],
        Model::F877A => vec![
            (0x05, 0x85),
            (0x06, 0x86),
            (0x07, 0x87),
            (0x08, 0x88),
            (0x09, 0x89),
        ],
    }
}

/// Canonical data address: banks folded, mirrors resolved.
fn canon(m: Model, a: u16) -> u16 {
    let banks = if four_banks(m) { 3 } else { 1 };
    let bank = (a >> 7) & banks;
    let low = a & 0x7F;
    let a = bank << 7 | low;
    if matches!(low, 0x00 | 0x02 | 0x03 | 0x04 | 0x0A | 0x0B) {
        return low;
    }
    match m {
        Model::F84A if bank == 1 && low >= 0x0C => low,
        Model::F675 if bank == 1 && (0x20..=0x5F).contains(&low) => low,
        Model::F628A | Model::F877A if low >= 0x70 => low,
        Model::F628A | Model::F877A if bank == 2 && matches!(low, 0x01 | 0x06) => low,
        Model::F628A | Model::F877A if bank == 3 && matches!(low, 0x01 | 0x06) => 0x80 | low,
        _ => a,
    }
}

const INDF: u16 = 0x00;
const TMR0: u16 = 0x01;
const PCL: u16 = 0x02;
const STATUS: u16 = 0x03;
const FSR: u16 = 0x04;
const PCLATH: u16 = 0x0A;
const INTCON: u16 = 0x0B;
const OPTION: u16 = 0x81;

const C: u8 = 0;
const DC: u8 = 1;
const Z: u8 = 2;

pub struct Pic14 {
    m: Model,
    prog: Vec<u16>,
    ram: Vec<u8>,
    w: u8,
    pc: u16,
    stack: [u16; 8],
    sp: usize,
    ports: Vec<Port>,
    regs: Vec<(u16, u16)>,
    prescale: u32,
    sleeping: bool,
    /// Input levels last seen, for INT and port-change interrupts.
    last_in: Vec<u32>,
    fault: Option<String>,
    cycles: u64,
}

impl Pic14 {
    pub fn new(dev: &Device, p: Program) -> Self {
        let m = model(dev);
        let mut prog = p.words;
        prog.resize(prog_size(m), 0);
        let mut ports = vec![Port::default(); dev.ports];
        for (name, port, bit) in &dev.pins {
            let _ = name;
            ports[*port].mask |= 1 << bit;
        }
        for p in &mut ports {
            p.tris = 0xFF;
        }
        let mut s = Self {
            m,
            prog,
            ram: vec![0; 512],
            w: 0,
            pc: 0,
            stack: [0; 8],
            sp: 0,
            last_in: vec![0; ports.len()],
            ports,
            regs: port_regs(m),
            prescale: 0,
            sleeping: false,
            fault: None,
            cycles: 0,
        };
        s.ram[STATUS as usize] = 0x18;
        s.ram[OPTION as usize] = 0xFF;
        s
    }

    fn status(&self, bit: u8) -> bool {
        self.ram[STATUS as usize] >> bit & 1 == 1
    }
    fn set_status(&mut self, bit: u8, v: bool) {
        let s = &mut self.ram[STATUS as usize];
        if v {
            *s |= 1 << bit;
        } else {
            *s &= !(1 << bit);
        }
    }
    fn address(&self, f: u16) -> u16 {
        let st = self.ram[STATUS as usize] as u16;
        if f == INDF {
            let irp = (st >> 7) & 1;
            return canon(self.m, irp << 8 | self.ram[FSR as usize] as u16);
        }
        canon(self.m, ((st >> 5) & 3) << 7 | f)
    }
    fn read(&self, f: u16) -> u8 {
        if f == INDF && self.ram[FSR as usize] == 0 {
            return 0;
        }
        let a = self.address(f);
        for (k, (port, tris)) in self.regs.iter().enumerate() {
            if a == *port {
                return self.ports[k].read() as u8;
            }
            if a == *tris {
                return (self.ports[k].tris & self.ports[k].mask) as u8;
            }
        }
        if a == PCL {
            // The program counter already points at the next instruction.
            return self.pc as u8;
        }
        self.ram[a as usize]
    }
    /// Write; returns true when the write moved the program counter.
    fn write(&mut self, f: u16, v: u8) -> bool {
        if f == INDF && self.ram[FSR as usize] == 0 {
            return false;
        }
        let a = self.address(f);
        for (k, (port, tris)) in self.regs.clone().iter().enumerate() {
            if a == *port {
                self.ports[k].lat = v as u32;
                return false;
            }
            if a == *tris {
                self.ports[k].tris = v as u32 | !0xFF;
                return false;
            }
        }
        match a {
            PCL => {
                self.pc = ((self.ram[PCLATH as usize] as u16) << 8 | v as u16) & 0x1FFF;
                return true;
            }
            STATUS => {
                let keep = self.ram[STATUS as usize] & 0x18;
                self.ram[STATUS as usize] = (v & !0x18) | keep;
            }
            TMR0 => {
                self.ram[TMR0 as usize] = v;
                self.prescale = 0;
            }
            _ => self.ram[a as usize] = v,
        }
        false
    }
    fn push(&mut self, pc: u16) {
        self.stack[self.sp % 8] = pc;
        self.sp += 1;
        if self.sp > 8 {
            // The real stack wraps silently; a program doing it is broken.
            self.fault = Some(format!(
                "stack overflow at 0x{:03X} (more than 8 nested calls)",
                pc.wrapping_sub(1)
            ));
        }
    }
    fn pop(&mut self) -> u16 {
        if self.sp == 0 {
            self.fault = Some(format!(
                "stack underflow at 0x{:03X} (RETURN without CALL)",
                self.pc.wrapping_sub(1)
            ));
            return 0;
        }
        self.sp -= 1;
        self.stack[self.sp % 8]
    }
    fn timer0(&mut self, cycles: u32) {
        let opt = self.ram[OPTION as usize];
        if opt & 0x20 != 0 {
            return; // T0CS: counting the T0CKI pin, not modelled
        }
        let rate = if opt & 0x08 != 0 {
            1
        } else {
            2u32 << (opt & 7)
        };
        for _ in 0..cycles {
            self.prescale += 1;
            if self.prescale >= rate {
                self.prescale = 0;
                let t = &mut self.ram[TMR0 as usize];
                *t = t.wrapping_add(1);
                if *t == 0 {
                    self.ram[INTCON as usize] |= 0x04;
                }
            }
        }
    }
    fn edges(&mut self) {
        // INT pin: RB0 (GP2 on the PIC12F675); change interrupt: RB4-RB7.
        let (int_port, int_bit) = if self.m == Model::F675 {
            (0, 2)
        } else {
            (1, 0)
        };
        let rising = self.ram[OPTION as usize] & 0x40 != 0;
        for k in 0..self.ports.len() {
            let now = self.ports[k].input & self.ports[k].tris;
            let before = self.last_in[k];
            if k == int_port {
                let (b, a) = (before >> int_bit & 1, now >> int_bit & 1);
                if (rising && b == 0 && a == 1) || (!rising && b == 1 && a == 0) {
                    self.ram[INTCON as usize] |= 0x02;
                }
            }
            if k == 1 && self.m != Model::F675 && (before ^ now) & 0xF0 != 0 {
                self.ram[INTCON as usize] |= 0x01;
            }
            self.last_in[k] = now;
        }
    }
}

impl Cpu for Pic14 {
    fn ports(&mut self) -> &mut Vec<Port> {
        &mut self.ports
    }
    fn fault(&self) -> Option<String> {
        self.fault.clone()
    }
    fn summary(&self) -> String {
        format!(
            "PC {:03X}  W {:02X}  STATUS {:02X}  cycles {}",
            self.pc, self.w, self.ram[STATUS as usize], self.cycles
        )
    }

    fn step(&mut self) -> u32 {
        self.edges();
        let intcon = self.ram[INTCON as usize];
        let pending = (intcon & 0x20 != 0 && intcon & 0x04 != 0)
            || (intcon & 0x10 != 0 && intcon & 0x02 != 0)
            || (intcon & 0x08 != 0 && intcon & 0x01 != 0);
        if pending {
            self.sleeping = false;
        }
        if pending && intcon & 0x80 != 0 {
            let pc = self.pc;
            self.push(pc);
            self.pc = 4;
            self.ram[INTCON as usize] &= !0x80;
            self.timer0(2);
            self.cycles += 2;
            return 2;
        }
        if self.sleeping {
            self.cycles += 1;
            return 1;
        }
        let op = self.prog[self.pc as usize % self.prog.len()];
        self.pc = (self.pc + 1) & 0x1FFF;
        let mut cyc = 1;
        let f = op & 0x7F;
        let d = op & 0x80 != 0;
        let k = (op & 0xFF) as u8;
        let b = ((op >> 7) & 7) as u8;
        let store = |s: &mut Self, v: u8| -> bool {
            if d {
                s.write(f, v)
            } else {
                s.w = v;
                false
            }
        };
        let mut jumped = false;
        match op >> 12 {
            0b00 => match (op >> 8) & 0xF {
                0x0 => {
                    if op & 0x80 != 0 {
                        jumped = self.write(f, self.w); // MOVWF
                    } else {
                        match op {
                            0x0008 => {
                                self.pc = self.pop();
                                cyc = 2;
                            }
                            0x0009 => {
                                self.pc = self.pop();
                                self.ram[INTCON as usize] |= 0x80;
                                cyc = 2;
                            }
                            0x0063 => {
                                self.sleeping = true;
                                self.set_status(3, false);
                            }
                            0x0064 => self.ram[STATUS as usize] |= 0x18,
                            _ => {} // NOP
                        }
                    }
                }
                0x1 => {
                    if d {
                        jumped = self.write(f, 0); // CLRF
                    } else {
                        self.w = 0; // CLRW
                    }
                    self.set_status(Z, true);
                }
                0x2 => {
                    let (x, w) = (self.read(f), self.w);
                    let r = x.wrapping_sub(w);
                    self.set_status(C, x >= w);
                    self.set_status(DC, (x & 0xF) >= (w & 0xF));
                    self.set_status(Z, r == 0);
                    jumped = store(self, r);
                }
                0x3 => {
                    let r = self.read(f).wrapping_sub(1);
                    self.set_status(Z, r == 0);
                    jumped = store(self, r);
                }
                0x4 => {
                    let r = self.read(f) | self.w;
                    self.set_status(Z, r == 0);
                    jumped = store(self, r);
                }
                0x5 => {
                    let r = self.read(f) & self.w;
                    self.set_status(Z, r == 0);
                    jumped = store(self, r);
                }
                0x6 => {
                    let r = self.read(f) ^ self.w;
                    self.set_status(Z, r == 0);
                    jumped = store(self, r);
                }
                0x7 => {
                    let (x, w) = (self.read(f), self.w);
                    let r = x as u16 + w as u16;
                    self.set_status(C, r > 0xFF);
                    self.set_status(DC, (x & 0xF) + (w & 0xF) > 0xF);
                    self.set_status(Z, r & 0xFF == 0);
                    jumped = store(self, r as u8);
                }
                0x8 => {
                    let r = self.read(f);
                    self.set_status(Z, r == 0);
                    jumped = store(self, r);
                }
                0x9 => {
                    let r = !self.read(f);
                    self.set_status(Z, r == 0);
                    jumped = store(self, r);
                }
                0xA => {
                    let r = self.read(f).wrapping_add(1);
                    self.set_status(Z, r == 0);
                    jumped = store(self, r);
                }
                0xB | 0xF => {
                    let x = self.read(f);
                    let r = if (op >> 8) & 0xF == 0xB {
                        x.wrapping_sub(1)
                    } else {
                        x.wrapping_add(1)
                    };
                    jumped = store(self, r);
                    if r == 0 && !jumped {
                        self.pc = (self.pc + 1) & 0x1FFF;
                        cyc = 2;
                    }
                }
                0xC => {
                    let x = self.read(f);
                    let c = self.status(C);
                    self.set_status(C, x & 1 == 1);
                    jumped = store(self, x >> 1 | (u8::from(c) << 7));
                }
                0xD => {
                    let x = self.read(f);
                    let c = self.status(C);
                    self.set_status(C, x & 0x80 != 0);
                    jumped = store(self, x << 1 | u8::from(c));
                }
                0xE => {
                    let x = self.read(f);
                    jumped = store(self, x.rotate_left(4));
                }
                _ => unreachable!(),
            },
            0b01 => {
                let x = self.read(f);
                match (op >> 10) & 3 {
                    0 => jumped = self.write(f, x & !(1 << b)),
                    1 => jumped = self.write(f, x | (1 << b)),
                    2 => {
                        if x >> b & 1 == 0 {
                            self.pc = (self.pc + 1) & 0x1FFF;
                            cyc = 2;
                        }
                    }
                    _ => {
                        if x >> b & 1 == 1 {
                            self.pc = (self.pc + 1) & 0x1FFF;
                            cyc = 2;
                        }
                    }
                }
            }
            0b10 => {
                let target = ((self.ram[PCLATH as usize] as u16 & 0x18) << 8) | (op & 0x7FF);
                if op & 0x800 == 0 {
                    let pc = self.pc;
                    self.push(pc);
                }
                self.pc = target;
                cyc = 2;
            }
            _ => {
                let w = self.w;
                match (op >> 8) & 0xF {
                    0x0..=0x3 => self.w = k,
                    0x4..=0x7 => {
                        self.w = k;
                        self.pc = self.pop();
                        cyc = 2;
                    }
                    0x8 => {
                        self.w = w | k;
                        self.set_status(Z, self.w == 0);
                    }
                    0x9 => {
                        self.w = w & k;
                        self.set_status(Z, self.w == 0);
                    }
                    0xA => {
                        self.w = w ^ k;
                        self.set_status(Z, self.w == 0);
                    }
                    0xC | 0xD => {
                        let r = k.wrapping_sub(w);
                        self.set_status(C, k >= w);
                        self.set_status(DC, (k & 0xF) >= (w & 0xF));
                        self.set_status(Z, r == 0);
                        self.w = r;
                    }
                    0xE | 0xF => {
                        let r = k as u16 + w as u16;
                        self.set_status(C, r > 0xFF);
                        self.set_status(DC, (k & 0xF) + (w & 0xF) > 0xF);
                        self.set_status(Z, r & 0xFF == 0);
                        self.w = r as u8;
                    }
                    _ => {
                        self.fault = Some(format!(
                            "invalid instruction 0x{op:04X} at 0x{:03X}",
                            self.pc.wrapping_sub(1)
                        ));
                    }
                }
            }
        }
        if jumped {
            cyc = 2;
        }
        self.timer0(cyc);
        self.cycles += cyc as u64;
        cyc
    }
}

// ------------------------------------------------------------ assembler

#[derive(Clone, Copy)]
enum Form {
    /// f, d
    Fd(u16),
    /// f
    F(u16),
    /// f, b
    Fb(u16),
    /// k (8-bit)
    K(u16),
    /// k (11-bit address)
    A(u16),
    /// no operands
    N(u16),
}

fn instruction(m: &str) -> Option<Form> {
    use Form::*;
    Some(match m {
        "ADDWF" => Fd(0x0700),
        "ANDWF" => Fd(0x0500),
        "CLRF" => F(0x0180),
        "CLRW" => N(0x0100),
        "COMF" => Fd(0x0900),
        "DECF" => Fd(0x0300),
        "DECFSZ" => Fd(0x0B00),
        "INCF" => Fd(0x0A00),
        "INCFSZ" => Fd(0x0F00),
        "IORWF" => Fd(0x0400),
        "MOVF" => Fd(0x0800),
        "MOVWF" => F(0x0080),
        "NOP" => N(0x0000),
        "RLF" => Fd(0x0D00),
        "RRF" => Fd(0x0C00),
        "SUBWF" => Fd(0x0200),
        "SWAPF" => Fd(0x0E00),
        "XORWF" => Fd(0x0600),
        "BCF" => Fb(0x1000),
        "BSF" => Fb(0x1400),
        "BTFSC" => Fb(0x1800),
        "BTFSS" => Fb(0x1C00),
        "ADDLW" => K(0x3E00),
        "ANDLW" => K(0x3900),
        "CALL" => A(0x2000),
        "CLRWDT" => N(0x0064),
        "GOTO" => A(0x2800),
        "IORLW" => K(0x3800),
        "MOVLW" => K(0x3000),
        "RETFIE" => N(0x0009),
        "RETLW" => K(0x3400),
        "RETURN" => N(0x0008),
        "SLEEP" => N(0x0063),
        "SUBLW" => K(0x3C00),
        "XORLW" => K(0x3A00),
        _ => return None,
    })
}

const DOCS: [(&str, &str); 35] = [
    ("ADDWF", "ADDWF f,d — W + f → d (C, DC, Z)"),
    ("ANDWF", "ANDWF f,d — W AND f → d (Z)"),
    ("CLRF", "CLRF f — 0 → f (Z)"),
    ("CLRW", "CLRW — 0 → W (Z)"),
    ("COMF", "COMF f,d — NOT f → d (Z)"),
    ("DECF", "DECF f,d — f − 1 → d (Z)"),
    ("DECFSZ", "DECFSZ f,d — f − 1 → d, skip if 0"),
    ("INCF", "INCF f,d — f + 1 → d (Z)"),
    ("INCFSZ", "INCFSZ f,d — f + 1 → d, skip if 0"),
    ("IORWF", "IORWF f,d — W OR f → d (Z)"),
    ("MOVF", "MOVF f,d — f → d (Z)"),
    ("MOVWF", "MOVWF f — W → f"),
    ("NOP", "NOP — no operation"),
    ("RLF", "RLF f,d — rotate left through carry → d (C)"),
    ("RRF", "RRF f,d — rotate right through carry → d (C)"),
    ("SUBWF", "SUBWF f,d — f − W → d (C = no borrow, DC, Z)"),
    ("SWAPF", "SWAPF f,d — swap nibbles of f → d"),
    ("XORWF", "XORWF f,d — W XOR f → d (Z)"),
    ("BCF", "BCF f,b — clear bit b of f"),
    ("BSF", "BSF f,b — set bit b of f"),
    ("BTFSC", "BTFSC f,b — skip next if bit b of f is 0"),
    ("BTFSS", "BTFSS f,b — skip next if bit b of f is 1"),
    ("ADDLW", "ADDLW k — W + k → W (C, DC, Z)"),
    ("ANDLW", "ANDLW k — W AND k → W (Z)"),
    ("CALL", "CALL k — call subroutine (8-level stack)"),
    ("CLRWDT", "CLRWDT — clear the watchdog"),
    ("GOTO", "GOTO k — jump"),
    ("IORLW", "IORLW k — W OR k → W (Z)"),
    ("MOVLW", "MOVLW k — k → W"),
    ("RETFIE", "RETFIE — return from interrupt, GIE = 1"),
    ("RETLW", "RETLW k — return with k in W (tables)"),
    ("RETURN", "RETURN — return from subroutine"),
    ("SLEEP", "SLEEP — sleep until an interrupt flag is set"),
    ("SUBLW", "SUBLW k — k − W → W (C = no borrow, DC, Z)"),
    ("XORLW", "XORLW k — W XOR k → W (Z)"),
];

const DIRECTIVES: [(&str, &str); 18] = [
    ("ORG", "ORG addr — place following code at addr"),
    ("EQU", "name EQU value — define a constant"),
    ("SET", "name SET value — define a redefinable constant"),
    (
        "CBLOCK",
        "CBLOCK addr — name a block of RAM bytes until ENDC (name:n reserves n)",
    ),
    ("ENDC", "ENDC — end of CBLOCK"),
    ("#DEFINE", "#define NAME text — text substitution"),
    ("DT", "DT a, b, \"text\" — RETLW table"),
    ("DW", "DW w, … — raw program words"),
    (
        "BANKSEL",
        "BANKSEL f — select the bank of register f (RP0/RP1)",
    ),
    (
        "PAGESEL",
        "PAGESEL label — select the program page of label (PCLATH)",
    ),
    ("END", "END — end of source"),
    (
        "RADIX",
        "RADIX dec|hex — default number base (MPASM default is hex)",
    ),
    ("LIST", "LIST p=… — accepted, ignored"),
    (
        "#INCLUDE",
        "#include <p16f84a.inc> — accepted; register names are built in",
    ),
    ("__CONFIG", "__CONFIG value — accepted, ignored"),
    ("PROCESSOR", "PROCESSOR name — accepted, ignored"),
    ("ERRORLEVEL", "ERRORLEVEL -302 — accepted, ignored"),
    ("BANKISEL", "BANKISEL f — select IRP for indirect access"),
];

/// Register and bit names of the include file.
fn builtin_symbols(m: Model) -> Symbols {
    let mut s = Symbols::default();
    for (n, v) in [
        ("W", 0),
        ("F", 1),
        ("INDF", 0),
        ("TMR0", 1),
        ("PCL", 2),
        ("STATUS", 3),
        ("FSR", 4),
        ("PCLATH", 0x0A),
        ("INTCON", 0x0B),
        ("OPTION_REG", 0x81),
        ("C", 0),
        ("DC", 1),
        ("Z", 2),
        ("NOT_PD", 3),
        ("NOT_TO", 4),
        ("RP0", 5),
        ("RP1", 6),
        ("IRP", 7),
        ("GIE", 7),
        ("PEIE", 6),
        ("T0IE", 5),
        ("TMR0IE", 5),
        ("INTE", 4),
        ("INT0IE", 4),
        ("RBIE", 3),
        ("GPIE", 3),
        ("T0IF", 2),
        ("TMR0IF", 2),
        ("INTF", 1),
        ("INT0IF", 1),
        ("RBIF", 0),
        ("GPIF", 0),
        ("NOT_RBPU", 7),
        ("NOT_GPPU", 7),
        ("INTEDG", 6),
        ("T0CS", 5),
        ("T0SE", 4),
        ("PSA", 3),
        ("PS2", 2),
        ("PS1", 1),
        ("PS0", 0),
    ] {
        s.set(n, v);
    }
    let ports: &[(&str, u16, u16, u32)] = match m {
        Model::F675 => &[("GPIO", 0x05, 0x85, 6)],
        Model::F84A => &[("PORTA", 0x05, 0x85, 5), ("PORTB", 0x06, 0x86, 8)],
        Model::F628A => &[("PORTA", 0x05, 0x85, 8), ("PORTB", 0x06, 0x86, 8)],
        Model::F877A => &[
            ("PORTA", 0x05, 0x85, 6),
            ("PORTB", 0x06, 0x86, 8),
            ("PORTC", 0x07, 0x87, 8),
            ("PORTD", 0x08, 0x88, 8),
            ("PORTE", 0x09, 0x89, 3),
        ],
    };
    for (name, p, t, bits) in ports {
        s.set(name, *p as i64);
        let tris = if *name == "GPIO" {
            "TRISIO".to_string()
        } else {
            format!("TRIS{}", &name[4..])
        };
        s.set(&tris, *t as i64);
        for b in 0..*bits {
            let bit = if *name == "GPIO" {
                format!("GP{b}")
            } else {
                format!("R{}{b}", &name[4..])
            };
            s.set(&bit, b as i64);
            if *name == "GPIO" {
                s.set(&format!("TRISIO{b}"), b as i64);
            } else {
                s.set(&format!("TRIS{}{b}", &name[4..]), b as i64);
            }
        }
    }
    match m {
        Model::F675 => {
            s.set("CMCON", 0x19);
            s.set("ANSEL", 0x9F);
            s.set("OSCCAL", 0x90);
        }
        Model::F628A => s.set("CMCON", 0x1F),
        Model::F877A => {
            s.set("ADCON1", 0x9F);
            s.set("CMCON", 0x9C);
        }
        Model::F84A => {}
    }
    s
}

fn ctx(here: i64, radix: u32, syms: &Symbols, lenient: bool) -> Ctx<'_> {
    Ctx {
        syms,
        radix,
        here,
        lenient,
    }
}

struct Line {
    no: usize,
    label: Option<String>,
    op: String,
    args: String,
}

fn parse_lines(src: &str) -> Vec<Line> {
    let mut defines: Vec<(String, String)> = Vec::new();
    let mut out = Vec::new();
    for (k, raw) in src.lines().enumerate() {
        let text = asm::strip_comment(raw, &[";"]);
        if text.trim().is_empty() {
            continue;
        }
        let trimmed = text.trim();
        if trimmed.to_ascii_uppercase().starts_with("#DEFINE") {
            let rest = trimmed[7..].trim();
            let mut it = rest.splitn(2, char::is_whitespace);
            if let Some(name) = it.next() {
                defines.push((name.to_string(), it.next().unwrap_or("").trim().to_string()));
            }
            continue;
        }
        // Text substitution of #define names (whole words).
        let mut line = text.to_string();
        for (name, val) in &defines {
            line = replace_word(&line, name, val);
        }
        let starts_col0 = !line.starts_with(char::is_whitespace);
        let mut words = line.split_whitespace();
        let first = words.next().unwrap_or("").to_string();
        let (label, op, args);
        let is_keyword = |w: &str| {
            let u = w.to_ascii_uppercase();
            instruction(&u).is_some() || DIRECTIVES.iter().any(|d| d.0 == u) || u == "INCLUDE"
        };
        if first.ends_with(':') || (starts_col0 && !is_keyword(&first)) {
            label = Some(first.trim_end_matches(':').to_string());
            let rest = line.trim_start()[first.len()..].trim_start().to_string();
            let mut w2 = rest.splitn(2, char::is_whitespace);
            op = w2.next().unwrap_or("").to_string();
            args = w2.next().unwrap_or("").trim().to_string();
        } else {
            label = None;
            op = first.clone();
            args = line.trim_start()[first.len()..].trim().to_string();
        }
        out.push(Line {
            no: k + 1,
            label,
            op: op.to_ascii_uppercase(),
            args,
        });
    }
    out
}

fn replace_word(s: &str, name: &str, val: &str) -> String {
    let mut out = String::new();
    let b: Vec<char> = s.chars().collect();
    let n: Vec<char> = name.chars().collect();
    let mut k = 0;
    while k < b.len() {
        let boundary_before = k == 0 || !(b[k - 1].is_alphanumeric() || b[k - 1] == '_');
        if boundary_before && b[k..].starts_with(&n) {
            let after = k + n.len();
            if after >= b.len() || !(b[after].is_alphanumeric() || b[after] == '_') {
                out.push_str(val);
                k = after;
                continue;
            }
        }
        out.push(b[k]);
        k += 1;
    }
    out
}

/// Assemble MPASM-style source.
pub fn assemble(dev: &Device, src: &str) -> Result<Program, Vec<AsmError>> {
    let m = model(dev);
    let lines = parse_lines(src);
    let mut syms = builtin_symbols(m);
    let size = prog_size(m);
    let mut errors = Vec::new();
    let mut words = vec![0u16; size];
    let mut used = vec![false; size];
    for pass in 0..2 {
        let mut pc: i64 = 0;
        let mut radix = 16;
        let mut cblock: Option<i64> = None;
        for l in &lines {
            let lenient = pass == 0;
            let err = |errors: &mut Vec<AsmError>, msg: String| {
                if pass == 1 {
                    errors.push(AsmError {
                        line: l.no,
                        message: msg,
                    });
                }
            };
            // Inside CBLOCK: each line names bytes.
            if let Some(addr) = cblock.as_mut() {
                if l.op == "ENDC"
                    || l.label
                        .as_deref()
                        .is_some_and(|x| x.eq_ignore_ascii_case("ENDC"))
                {
                    cblock = None;
                    continue;
                }
                let all = format!(
                    "{} {} {}",
                    l.label.clone().unwrap_or_default(),
                    l.op,
                    l.args
                );
                for item in all.split([',', ' ']).filter(|x| !x.trim().is_empty()) {
                    let (name, n) = match item.split_once(':') {
                        Some((a, b)) => (
                            a.trim(),
                            asm::eval(b, &ctx(pc, radix, &syms, lenient)).unwrap_or(1),
                        ),
                        None => (item.trim(), 1),
                    };
                    syms.set(name, *addr);
                    *addr += n;
                }
                continue;
            }
            match l.op.as_str() {
                "EQU" | "SET" => {
                    let Some(name) = &l.label else {
                        err(&mut errors, format!("{} needs a name in column 1", l.op));
                        continue;
                    };
                    match asm::eval(&l.args, &ctx(pc, radix, &syms, lenient)) {
                        Ok(v) => syms.set(name, v),
                        Err(e) => err(&mut errors, e),
                    }
                    continue;
                }
                _ => {}
            }
            if let Some(label) = &l.label {
                if pass == 0
                    && syms.contains(label)
                    && instruction(&label.to_ascii_uppercase()).is_none()
                {
                    // Redefinition is reported in pass two.
                }
                syms.set(label, pc);
            }
            let ops = asm::split_operands(&l.args);
            let mut emit = |pc: &mut i64, w: u16, errors: &mut Vec<AsmError>| {
                let a = *pc as usize;
                if a >= size {
                    err(
                        errors,
                        format!("program memory full (0x{a:X} beyond {size} words)"),
                    );
                } else {
                    if pass == 1 {
                        if used[a] {
                            err(
                                errors,
                                format!("address 0x{a:03X} is used twice (check ORG)"),
                            );
                        }
                        used[a] = true;
                        words[a] = w & 0x3FFF;
                    }
                }
                *pc += 1;
            };
            let op = l.op.as_str();
            match op {
                "" => {}
                "ORG" => match asm::eval(&l.args, &ctx(pc, radix, &syms, lenient)) {
                    Ok(v) => pc = v,
                    Err(e) => err(&mut errors, e),
                },
                "END" => break,
                "RADIX" => {
                    radix = match l.args.trim().to_ascii_lowercase().as_str() {
                        "dec" | "decimal" => 10,
                        "hex" | "hexadecimal" => 16,
                        "oct" | "octal" => 8,
                        other => {
                            err(&mut errors, format!("unknown radix `{other}`"));
                            radix
                        }
                    }
                }
                "LIST" => {
                    for o in &ops {
                        if let Some(r) = o.to_ascii_lowercase().strip_prefix("r=") {
                            radix = if r.starts_with("dec") {
                                10
                            } else if r.starts_with("oct") {
                                8
                            } else {
                                16
                            };
                        }
                    }
                }
                "#INCLUDE" | "INCLUDE" | "__CONFIG" | "__IDLOCS" | "PROCESSOR" | "ERRORLEVEL"
                | "__MAXRAM" | "__BADRAM" | "CONFIG" => {}
                "CBLOCK" => match asm::eval(&l.args, &ctx(pc, radix, &syms, lenient)) {
                    Ok(v) => cblock = Some(v),
                    Err(e) => err(&mut errors, e),
                },
                "ENDC" => {}
                "DT" | "DW" => {
                    for o in &ops {
                        if let Some(text) = o.strip_prefix('"').and_then(|x| x.strip_suffix('"')) {
                            for ch in text.chars() {
                                let w = if op == "DT" {
                                    0x3400 | (ch as u16 & 0xFF)
                                } else {
                                    ch as u16
                                };
                                emit(&mut pc, w, &mut errors);
                            }
                        } else {
                            match asm::eval(o, &ctx(pc, radix, &syms, lenient)) {
                                Ok(v) => emit(
                                    &mut pc,
                                    if op == "DT" {
                                        0x3400 | (v as u16 & 0xFF)
                                    } else {
                                        v as u16
                                    },
                                    &mut errors,
                                ),
                                Err(e) => {
                                    err(&mut errors, e);
                                    pc += 1;
                                }
                            }
                        }
                    }
                }
                "BANKSEL" | "BANKISEL" => {
                    let a =
                        asm::eval(&l.args, &ctx(pc, radix, &syms, lenient)).unwrap_or_else(|e| {
                            err(&mut errors, e);
                            0
                        });
                    if op == "BANKISEL" {
                        emit(
                            &mut pc,
                            if a & 0x100 != 0 {
                                0x1780 | 0x03
                            } else {
                                0x1380 | 0x03
                            },
                            &mut errors,
                        );
                    } else {
                        // BCF/BSF STATUS,RP0 (and RP1 on four-bank parts).
                        emit(
                            &mut pc,
                            if a & 0x80 != 0 { 0x1683 } else { 0x1283 },
                            &mut errors,
                        );
                        if four_banks(m) {
                            emit(
                                &mut pc,
                                if a & 0x100 != 0 { 0x1703 } else { 0x1303 },
                                &mut errors,
                            );
                        }
                    }
                }
                "PAGESEL" => {
                    if size > 2048 {
                        let a = asm::eval(&l.args, &ctx(pc, radix, &syms, lenient)).unwrap_or(0);
                        emit(
                            &mut pc,
                            if a & 0x800 != 0 { 0x158A } else { 0x118A },
                            &mut errors,
                        );
                        emit(
                            &mut pc,
                            if a & 0x1000 != 0 { 0x160A } else { 0x120A },
                            &mut errors,
                        );
                    }
                }
                _ => {
                    let Some(form) = instruction(op) else {
                        err(&mut errors, format!("unknown instruction `{}`", l.op));
                        pc += 1;
                        continue;
                    };
                    let c = ctx(pc, radix, &syms, lenient);
                    let val = |k: usize, errors: &mut Vec<AsmError>| -> i64 {
                        match ops.get(k) {
                            Some(o) => asm::eval(o, &c).unwrap_or_else(|e| {
                                err(errors, e);
                                0
                            }),
                            None => {
                                err(errors, format!("{op} needs more operands"));
                                0
                            }
                        }
                    };
                    let word = match form {
                        Form::N(base) => base,
                        Form::F(base) => base | (val(0, &mut errors) as u16 & 0x7F),
                        Form::Fd(base) => {
                            let f = val(0, &mut errors) as u16 & 0x7F;
                            let d = if ops.len() > 1 {
                                val(1, &mut errors)
                            } else {
                                1
                            };
                            if d != 0 && d != 1 {
                                err(&mut errors, "destination must be W (0) or F (1)".into());
                            }
                            base | f | ((d as u16 & 1) << 7)
                        }
                        Form::Fb(base) => {
                            let f = val(0, &mut errors) as u16 & 0x7F;
                            let b = val(1, &mut errors);
                            if !(0..8).contains(&b) {
                                err(&mut errors, format!("bit {b} is not 0-7"));
                            }
                            base | f | ((b as u16 & 7) << 7)
                        }
                        Form::K(base) => {
                            let k = val(0, &mut errors);
                            if !(-128..=255).contains(&k) {
                                err(&mut errors, format!("literal {k} does not fit in 8 bits"));
                            }
                            base | (k as u16 & 0xFF)
                        }
                        Form::A(base) => {
                            let a = val(0, &mut errors);
                            if pass == 1 && (a < 0 || a as usize >= size) {
                                err(
                                    &mut errors,
                                    format!("address {a} is outside program memory"),
                                );
                            }
                            base | (a as u16 & 0x7FF)
                        }
                    };
                    emit(&mut pc, word, &mut errors);
                }
            }
        }
    }
    if errors.is_empty() {
        Ok(Program { words })
    } else {
        Err(errors)
    }
}

/// Editor vocabulary.
pub fn vocabulary(dev: &Device) -> Vec<(String, String, bool)> {
    let mut v: Vec<(String, String, bool)> = DOCS
        .iter()
        .map(|(k, d)| (k.to_string(), d.to_string(), true))
        .collect();
    v.extend(
        DIRECTIVES
            .iter()
            .map(|(k, d)| (k.to_string(), d.to_string(), true)),
    );
    let syms = builtin_symbols(model(dev));
    let mut names: Vec<(&String, &i64)> = syms.names().collect();
    names.sort();
    for (n, val) in names {
        v.push((n.clone(), format!("{n} = 0x{val:02X}"), false));
    }
    v
}

/// A blink program.
pub fn template(dev: &Device) -> String {
    let m = model(dev);
    let (port, tris, out, inp) = match m {
        Model::F675 => ("GPIO", "TRISIO", "GP0", "GP3"),
        _ => ("PORTB", "TRISB", "RB0", "RB1"),
    };
    let off = if m == Model::F675 {
        "        movlw   0x07\n        movwf   CMCON           ; comparator off\n"
    } else if m == Model::F628A {
        "        movlw   0x07\n        movwf   CMCON           ; comparators off\n"
    } else {
        ""
    };
    format!(
        "; {part}: blink {out} every 250 ms (4 MHz clock) while {inp} is low.
; MPASM syntax. Default radix is hex: write D'10' or .10 for decimal.
        list    p={lower}
        radix   dec

        cblock  0x20
        d1, d2, d3              ; delay counters
        endc

        org     0
        goto    start
        org     4
        retfie                  ; no interrupts used

start:
{off}        banksel {tris}
        movlw   B'11111110'     ; {out} output, the rest inputs
        movwf   {tris}
        banksel {port}
        clrf    {port}

loop:   btfsc   {port}, {inp}     ; {inp} high: hold
        goto    loop
        movlw   1 << {out}
        xorwf   {port}, F       ; toggle {out}
        call    delay250
        goto    loop

; 250 ms at 1 MIPS: 250 x 1 ms
delay250:
        movlw   250
        movwf   d1
ms:     movlw   199             ; 199 x 5 + 5 cycles = 1 ms
        movwf   d2
inner:  nop
        nop
        decfsz  d2, F
        goto    inner
        decfsz  d1, F
        goto    ms
        return

        end
",
        part = dev.part,
        lower = dev.part.to_ascii_lowercase().trim_start_matches("pic"),
    )
    .replace(
        "d1, d2, d3              ; delay counters",
        "d1, d2                  ; delay counters",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cpu, device};

    fn run(src: &str, dev: &str, steps: usize) -> Pic14 {
        let d = device(dev).unwrap();
        let p = assemble(&d, src).unwrap_or_else(|e| panic!("{e:?}"));
        let mut cpu = Pic14::new(&d, p);
        for _ in 0..steps {
            cpu.step();
            assert!(cpu.fault().is_none(), "{:?}", cpu.fault());
        }
        cpu
    }

    #[test]
    fn arithmetic_flags_and_tables() {
        let src = "
        radix dec
        cblock 0x20
        a, b, r
        endc
        org 0
        movlw 200
        movwf a
        movlw 100
        addwf a, W      ; 300: W = 44, C = 1
        movwf r
        movlw 5
        call table      ; W = 'C' (index 2 -> ... )
        movwf b
        movlw 10
        sublw 3         ; 3 - 10 = -7 = 249, C = 0 (borrow)
        goto $
table:  addwf PCL, F
        dt \"ABCDEFGH\"
        ";
        let cpu = run(src, "mcu.pic16f84a", 40);
        assert_eq!(cpu.ram[0x22], 44);
        assert_eq!(cpu.ram[0x21], b'F');
        assert_eq!(cpu.w, 249);
        assert!(!cpu.status(C));
    }

    #[test]
    fn banks_ports_and_bit_tests() {
        let src = "
        org 0
        bsf STATUS, RP0
        movlw b'00001111'
        movwf TRISB         ; RB4-RB7 outputs
        bcf STATUS, RP0
        movlw 0xA0
        movwf PORTB
        btfsc PORTB, RB0    ; RB0 is an input reading high: no skip
        bsf PORTB, RB4
        goto $
        ";
        let d = device("mcu.pic16f84a").unwrap();
        let mut cpu = Pic14::new(&d, assemble(&d, src).unwrap());
        cpu.ports()[1].input = 0x01;
        for _ in 0..12 {
            cpu.step();
        }
        assert_eq!(cpu.ports[1].tris & 0xFF, 0x0F);
        assert_eq!(cpu.ports[1].lat & 0xF0, 0xB0);
        // GPR mirrored between banks on the 16F84A.
        assert_eq!(canon(Model::F84A, 0x8C), 0x0C);
        assert_eq!(canon(Model::F628A, 0xF5), 0x75);
        assert_eq!(canon(Model::F628A, 0x186), 0x86);
    }

    #[test]
    fn timer0_interrupt() {
        let src = "
        radix dec
        cblock 0x20
        count
        endc
        org 0
        goto start
        org 4
        incf count, F
        bcf INTCON, T0IF
        retfie
start:  bsf STATUS, RP0
        movlw b'11011000'   ; TMR0 from Fcy, no prescaler
        movwf OPTION_REG
        bcf STATUS, RP0
        bsf INTCON, T0IE
        bsf INTCON, GIE
        goto $
        ";
        let cpu = run(src, "mcu.pic16f628a", 2000);
        // One overflow per 256 instruction cycles (setup takes a few).
        let n = cpu.ram[0x20] as i64;
        let want = cpu.cycles as i64 / 256;
        assert!(
            (n - want).abs() <= 1,
            "{n} interrupts in {} cycles",
            cpu.cycles
        );
    }

    #[test]
    fn errors_name_lines() {
        let d = device("mcu.pic16f84a").unwrap();
        let e = assemble(
            &d,
            "  movlw 1\n  frobnicate 2\n  goto nowhere\n  movlw 300\n",
        )
        .unwrap_err();
        let lines: Vec<usize> = e.iter().map(|x| x.line).collect();
        assert_eq!(lines, vec![2, 3, 4], "{e:?}");
    }

    #[test]
    fn templates_assemble() {
        for id in [
            "mcu.pic12f675",
            "mcu.pic16f84a",
            "mcu.pic16f628a",
            "mcu.pic16f877a",
        ] {
            let d = device(id).unwrap();
            assemble(&d, &template(&d)).unwrap_or_else(|e| panic!("{id}: {e:?}"));
        }
    }
}
