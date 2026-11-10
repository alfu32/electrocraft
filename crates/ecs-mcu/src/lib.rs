//! Microcontrollers (spec 7.2): PIC devices programmed in assembly.
//!
//! * 8-bit **PIC16** mid-range core (PIC12F675, PIC16F84A, PIC16F628A,
//!   PIC16F877A): all 35 instructions, banked data memory, 8-level stack,
//!   TMR0 with prescaler, INT/RB-change/TMR0 interrupts; MPASM-style
//!   assembler (`pic14`).
//! * 16-bit **PIC24** (PIC24FJ64GA002): W0-W15, the common instructions and
//!   addressing modes, TMR1; XC16-style assembler (`pic24`).
//! * 32-bit **PIC32** (PIC32MX250F128B): the MIPS32 integer instruction set
//!   with branch delay slots, the core timer, GPIO with SET/CLR/INV
//!   registers; GNU-style assembler (`mips`).
//!
//! The CPU runs on the digital event layer: a timer callback executes one
//! time slice of instructions; pin changes are scheduled at the instruction
//! time within the slice, inputs are read at slice boundaries.
//! Pins are bidirectional: TRIS selects input (driver off) or output.

pub mod asm;
pub mod mips;
pub mod pic14;
pub mod pic24;

use asm::AsmError;
use ecs_model::{
    Behavior, ComponentDef, EventCtx, KnownPart, Level, ParamDef, ParamKind, PinDef, PinKind,
    StateDef,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// A GPIO port: direction (1 = input), output latch, and input levels.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Port {
    pub tris: u32,
    pub lat: u32,
    pub input: u32,
    /// Pins that exist.
    pub mask: u32,
}

impl Port {
    /// What reading the port gives: input pins read the pin, outputs their latch.
    pub fn read(&self) -> u32 {
        ((self.input & self.tris) | (self.lat & !self.tris)) & self.mask
    }
}

/// A simulated CPU.
pub trait Cpu: Send {
    /// Execute one instruction (or take an interrupt); returns cycles used.
    fn step(&mut self) -> u32;
    fn ports(&mut self) -> &mut Vec<Port>;
    /// A stop condition (bad opcode, stack overflow...), if any.
    fn fault(&self) -> Option<String>;
    /// One-line register summary for the inspector.
    fn summary(&self) -> String;
}

/// Instruction set family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    Pic14,
    Pic24,
    Mips,
}

impl Arch {
    /// Language tag for the editor.
    pub fn language(self) -> &'static str {
        match self {
            Arch::Pic14 => "pic14",
            Arch::Pic24 => "pic24",
            Arch::Mips => "mips",
        }
    }
}

/// A device: pins, clock and memory sizes.
#[derive(Debug, Clone)]
pub struct Device {
    pub id: &'static str,
    pub part: &'static str,
    pub arch: Arch,
    /// Pins as (name, port index, bit); the symbol puts port A on the left.
    pub pins: Vec<(String, usize, u32)>,
    pub ports: usize,
    /// Default oscillator frequency (Hz).
    pub fosc: f64,
    /// Instruction cycles per oscillator cycle divisor (4, 2 or 1).
    pub osc_div: f64,
    pub cmos33: bool,
    pub desc: &'static str,
}

fn pins(
    port: usize,
    letter: &str,
    bits: std::ops::RangeInclusive<u32>,
) -> Vec<(String, usize, u32)> {
    bits.map(|b| (format!("R{letter}{b}"), port, b)).collect()
}

/// Every device.
pub fn devices() -> Vec<Device> {
    vec![
        Device {
            id: "mcu.pic12f675",
            part: "PIC12F675",
            arch: Arch::Pic14,
            pins: (0..=5).map(|b| (format!("GP{b}"), 0, b)).collect(),
            ports: 1,
            fosc: 4e6,
            osc_div: 4.0,
            cmos33: false,
            desc: "8-pin 8-bit PIC (1K words, 64 bytes RAM), GPIO GP0-GP5 (GP3 input only), INT on GP2. Analog functions are not modelled: pins are digital.",
        },
        Device {
            id: "mcu.pic16f84a",
            part: "PIC16F84A",
            arch: Arch::Pic14,
            pins: [pins(0, "A", 0..=4), pins(1, "B", 0..=7)].concat(),
            ports: 2,
            fosc: 4e6,
            osc_div: 4.0,
            cmos33: false,
            desc: "18-pin 8-bit PIC (1K words, 68 bytes RAM), PORTA RA0-RA4, PORTB RB0-RB7, INT on RB0, change interrupt on RB4-RB7.",
        },
        Device {
            id: "mcu.pic16f628a",
            part: "PIC16F628A",
            arch: Arch::Pic14,
            pins: [pins(0, "A", 0..=7), pins(1, "B", 0..=7)].concat(),
            ports: 2,
            fosc: 4e6,
            osc_div: 4.0,
            cmos33: false,
            desc: "18-pin 8-bit PIC (2K words, 224 bytes RAM), PORTA RA0-RA7, PORTB RB0-RB7. Comparators, UART and CCP are not modelled.",
        },
        Device {
            id: "mcu.pic16f877a",
            part: "PIC16F877A",
            arch: Arch::Pic14,
            pins: [
                pins(0, "A", 0..=5),
                pins(1, "B", 0..=7),
                pins(2, "C", 0..=7),
                pins(3, "D", 0..=7),
                pins(4, "E", 0..=2),
            ]
            .concat(),
            ports: 5,
            fosc: 4e6,
            osc_div: 4.0,
            cmos33: false,
            desc: "40-pin 8-bit PIC (8K words, 368 bytes RAM), PORTA-PORTE. ADC, UART, SPI and CCP are not modelled.",
        },
        Device {
            id: "mcu.pic24fj64ga002",
            part: "PIC24FJ64GA002",
            arch: Arch::Pic24,
            pins: [pins(0, "A", 0..=4), pins(1, "B", 0..=15)].concat(),
            ports: 2,
            fosc: 8e6,
            osc_div: 2.0,
            cmos33: true,
            desc: "28-pin 16-bit PIC24 (3.3 V, FRC 8 MHz: 4 MIPS), PORTA RA0-RA4, PORTB RB0-RB15, TMR1. A subset of the instruction set (see the editor's list).",
        },
        Device {
            id: "mcu.pic32mx250f128b",
            part: "PIC32MX250F128B",
            arch: Arch::Mips,
            pins: [pins(0, "A", 0..=4), pins(1, "B", 0..=15)].concat(),
            ports: 2,
            fosc: 40e6,
            osc_div: 1.0,
            cmos33: true,
            desc: "28-pin 32-bit PIC32 (MIPS32 M4K, 3.3 V, 40 MHz), PORTA RA0-RA4, PORTB RB0-RB15 with SET/CLR/INV registers, core timer (CP0 Count).",
        },
    ]
}

/// A device by definition id.
pub fn device(id: &str) -> Option<Device> {
    devices().into_iter().find(|d| d.id == id)
}

/// An assembled program.
#[derive(Debug, Clone)]
pub enum Image {
    Pic14(pic14::Program),
    Pic24(pic24::Program),
    Mips(mips::Program),
}

/// Assemble source for a device.
pub fn assemble(dev: &Device, source: &str) -> Result<Image, Vec<AsmError>> {
    match dev.arch {
        Arch::Pic14 => pic14::assemble(dev, source).map(Image::Pic14),
        Arch::Pic24 => pic24::assemble(dev, source).map(Image::Pic24),
        Arch::Mips => mips::assemble(dev, source).map(Image::Mips),
    }
}

fn new_cpu(dev: &Device, image: &Image) -> Box<dyn Cpu> {
    match image {
        Image::Pic14(p) => Box::new(pic14::Pic14::new(dev, p.clone())),
        Image::Pic24(p) => Box::new(pic24::Pic24::new(dev, p.clone())),
        Image::Mips(p) => Box::new(mips::Mips::new(dev, p.clone())),
    }
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

struct Running {
    cpu: Box<dyn Cpu>,
    /// What the pins were last driven to, per port: (tris, lat).
    driven: Vec<(u32, u32)>,
    /// Cycles run beyond the last slice.
    debt: f64,
}

/// The behavior of a programmed device.
pub struct Mcu {
    dev: Device,
    image: Option<Image>,
    running: Mutex<HashMap<u64, Running>>,
    fault: Mutex<Option<String>>,
}

const TIMER: u32 = 1;

impl Mcu {
    fn pin_name(&self, port: usize, bit: u32) -> Option<&str> {
        self.dev
            .pins
            .iter()
            .find(|p| p.1 == port && p.2 == bit)
            .map(|p| p.0.as_str())
    }

    fn sync_inputs(&self, c: &EventCtx<'_>, r: &mut Running) {
        let ports = r.cpu.ports();
        for (name, port, bit) in &self.dev.pins {
            if let Some(p) = ports.get_mut(*port) {
                let high = c.level(&format!("{name}_in")).is_high();
                if high {
                    p.input |= 1 << bit;
                } else {
                    p.input &= !(1 << bit);
                }
            }
        }
    }

    /// Drive pins whose direction or latch changed, `delay` seconds from now.
    fn drive(&self, c: &mut EventCtx<'_>, r: &mut Running, delay: f64) {
        let ports: Vec<Port> = r.cpu.ports().clone();
        for (k, p) in ports.iter().enumerate() {
            let (tris0, lat0) = r.driven.get(k).copied().unwrap_or((u32::MAX, 0));
            if (tris0, lat0) == (p.tris, p.lat) {
                continue;
            }
            for bit in 0..32 {
                let m = 1 << bit;
                if p.mask & m == 0 || ((tris0 ^ p.tris) | (lat0 ^ p.lat)) & m == 0 {
                    continue;
                }
                let Some(name) = self.pin_name(k, bit) else {
                    continue;
                };
                let name = name.to_string();
                let output = p.tris & m == 0;
                c.set(&format!("__oe_{name}"), if output { 1.0 } else { 0.0 });
                c.drive(&name, Level::from_bool(p.lat & m != 0), delay);
            }
            if let Some(d) = r.driven.get_mut(k) {
                *d = (p.tris, p.lat);
            }
        }
    }

    fn fcy(&self, c: &EventCtx<'_>) -> f64 {
        (c.param("fosc") / self.dev.osc_div).max(1.0)
    }
}

impl Behavior for Mcu {
    fn on_start(&self, c: &mut EventCtx<'_>) {
        let Some(image) = &self.image else {
            // No program: every pin stays an input.
            return;
        };
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        c.set("__cpu", id as f64);
        let mut r = Running {
            cpu: new_cpu(&self.dev, image),
            driven: vec![(u32::MAX, 0); self.dev.ports],
            debt: 0.0,
        };
        self.sync_inputs(c, &mut r);
        self.drive(c, &mut r, 0.0);
        if let Ok(mut m) = self.running.lock() {
            // Keep the table small: a reset starts a fresh CPU.
            if m.len() > 64 {
                m.clear();
            }
            m.insert(id, r);
        }
        let q = c.param("slice");
        c.timer(q, TIMER);
    }

    fn on_inputs(&self, c: &mut EventCtx<'_>) {
        let id = c.get("__cpu") as u64;
        if let Ok(mut m) = self.running.lock()
            && let Some(r) = m.get_mut(&id)
        {
            self.sync_inputs(c, r);
        }
    }

    fn on_timer(&self, c: &mut EventCtx<'_>, tag: u32) {
        if tag != TIMER {
            return;
        }
        let id = c.get("__cpu") as u64;
        let fcy = self.fcy(c);
        let slice = c.param("slice").max(1e-7);
        let Ok(mut m) = self.running.lock() else {
            return;
        };
        let Some(r) = m.get_mut(&id) else { return };
        let budget = slice * fcy;
        let mut used = r.debt;
        while used < budget {
            if let Some(f) = r.cpu.fault() {
                if let Ok(mut g) = self.fault.lock() {
                    *g = Some(f);
                }
                used = budget;
                break;
            }
            used += r.cpu.step() as f64;
            self.drive(c, r, (used / fcy).min(slice));
        }
        r.debt = used - budget;
        drop(m);
        c.timer(slice, TIMER);
    }

    fn diagnostics(&self) -> Option<String> {
        self.fault.lock().ok().and_then(|f| f.clone())
    }
}

fn base_def(dev: &Device) -> ComponentDef {
    let mut d = ComponentDef::new(
        dev.id,
        dev.part,
        "Microcontrollers",
        "U",
        &format!("ic:{}", dev.id),
    );
    let mut fam = ParamDef::real(
        "family",
        if dev.cmos33 { 1.0 } else { 0.0 },
        "",
        "I/O logic levels",
    );
    fam.kind = ParamKind::Enum(vec!["5 V CMOS".into(), "3.3 V CMOS".into(), "TTL".into()]);
    d.params.push(fam);
    d.params
        .push(ParamDef::real("fosc", dev.fosc, "Hz", "Oscillator frequency").range(1e3, 200e6));
    d.params.push(
        ParamDef::real(
            "slice",
            20e-6,
            "s",
            "Time slice: inputs are read between slices",
        )
        .range(1e-7, 1e-2),
    );
    let vth = if dev.cmos33 { 1.65 } else { 2.5 };
    for (name, _, _) in &dev.pins {
        d.pins.push(PinDef {
            name: name.clone(),
            kind: PinKind::DigitalOut(String::new()),
        });
        d.states.push(StateDef {
            name: format!("__oe_{name}"),
            initial: 0.0,
        });
        d.watches.push((
            format!("{name}_in"),
            ecs_expr::parse(&format!("V({name}) > {vth}")).expect("watch"),
        ));
    }
    d.meta.description = format!(
        "{} Program it with Edit Program… in the inspector.",
        dev.desc
    );
    d
}

/// Every device definition, without a program (all pins inputs).
pub fn definitions() -> Vec<ComponentDef> {
    devices()
        .iter()
        .map(|dev| {
            let mut d = base_def(dev);
            d.behavior = Some(Arc::new(Mcu {
                dev: dev.clone(),
                image: None,
                running: Default::default(),
                fault: Default::default(),
            }));
            d
        })
        .collect()
}

/// A definition running `source` (assembled). Errors leave the device idle.
pub fn with_program(def: &ComponentDef, source: &str) -> (ComponentDef, Vec<AsmError>) {
    let Some(dev) = device(&def.id) else {
        return (
            def.clone(),
            vec![AsmError {
                line: 0,
                message: format!("{} is not a microcontroller", def.id),
            }],
        );
    };
    let (image, errors) = if source.trim().is_empty() {
        (None, Vec::new())
    } else {
        match assemble(&dev, source) {
            Ok(i) => (Some(i), Vec::new()),
            Err(e) => (None, e),
        }
    };
    let mut d = def.clone();
    d.behavior = Some(Arc::new(Mcu {
        dev,
        image,
        running: Default::default(),
        fault: Default::default(),
    }));
    (d, errors)
}

/// Symbol layouts: port A (and C/E) on the left, the rest on the right.
pub fn symbols() -> Vec<(String, String, Vec<String>, Vec<String>)> {
    devices()
        .iter()
        .map(|d| {
            let half = d.pins.len().div_ceil(2);
            let (l, r): (Vec<_>, Vec<_>) = if d.ports == 1 {
                let n: Vec<String> = d.pins.iter().map(|p| p.0.clone()).collect();
                (n[..half].to_vec(), n[half..].to_vec())
            } else {
                let left: Vec<String> = d
                    .pins
                    .iter()
                    .filter(|p| p.1 % 2 == 0)
                    .map(|p| p.0.clone())
                    .collect();
                let right: Vec<String> = d
                    .pins
                    .iter()
                    .filter(|p| p.1 % 2 == 1)
                    .map(|p| p.0.clone())
                    .collect();
                (left, right)
            };
            let title = d
                .part
                .trim_start_matches("PIC")
                .chars()
                .take(10)
                .collect::<String>();
            (format!("ic:{}", d.id), title, l, r)
        })
        .collect()
}

/// Part numbers: one per device (and common variants).
pub fn part_numbers() -> Vec<KnownPart> {
    let mut v = Vec::new();
    for d in devices() {
        v.push(KnownPart {
            number: d.part.into(),
            def: d.id.into(),
            params: Vec::new(),
            note: d.desc.into(),
            default: true,
        });
    }
    for (number, id) in [
        ("PIC16F84", "mcu.pic16f84a"),
        ("PIC16F648A", "mcu.pic16f628a"),
        ("PIC16F876A", "mcu.pic16f877a"),
        ("PIC12F629", "mcu.pic12f675"),
        ("PIC24FJ32GA002", "mcu.pic24fj64ga002"),
        ("PIC32MX170F256B", "mcu.pic32mx250f128b"),
    ] {
        v.push(KnownPart {
            number: number.into(),
            def: id.into(),
            params: Vec::new(),
            note: "Modelled with the same core and pins".into(),
            default: false,
        });
    }
    v
}

/// A starting program for a device: blinks a pin and copies an input.
pub fn template(id: &str) -> String {
    let Some(d) = device(id) else {
        return String::new();
    };
    match d.arch {
        Arch::Pic14 => pic14::template(&d),
        Arch::Pic24 => pic24::template(&d),
        Arch::Mips => mips::template(&d),
    }
}

/// Editor vocabulary: (word, detail, is_instruction).
pub fn vocabulary(id: &str) -> Vec<(String, String, bool)> {
    let Some(d) = device(id) else {
        return Vec::new();
    };
    match d.arch {
        Arch::Pic14 => pic14::vocabulary(&d),
        Arch::Pic24 => pic24::vocabulary(&d),
        Arch::Mips => mips::vocabulary(&d),
    }
}

/// Whether a definition is a microcontroller.
pub fn is_mcu(id: &str) -> bool {
    id.starts_with("mcu.")
}
