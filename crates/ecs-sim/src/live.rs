//! Interactive simulation thread (spec 3 "Threads and messages" and 4.5).
//!
//! The UI sends [`Command`]s over a channel. After each frame's worth of steps
//! the engine publishes a [`Snapshot`] through a lock-free triple buffer and
//! streams probed signals through a single-producer, single-consumer ring.

use crate::circuit::{Circuit, Netlist, Options};
use crossbeam_channel::{Receiver, Sender, unbounded};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Maximum raw signals per scope sample (scope formulas combine them).
pub const MAX_CHANNELS: usize = 64;

/// What a scope channel measures.
#[derive(Debug, Clone, PartialEq)]
pub enum ProbeTarget {
    /// Voltage of a node (by netlist node index, 0 = ground).
    Node(usize),
    /// Voltage difference between two nodes.
    Diff(usize, usize),
    /// Current through branch `branch` of instance `inst` (by name).
    Current { inst: String, branch: usize },
    /// Probe `index` of instance `inst` (definition order).
    Probe { inst: String, index: usize },
}

/// Messages to the simulation thread.
#[derive(Debug, Clone)]
pub enum Command {
    /// Replace the circuit (topology edit). `generation` tags snapshots.
    Rebuild {
        netlist: Netlist,
        generation: u64,
    },
    /// Change a live parameter of an instance.
    SetParam {
        inst: String,
        name: String,
        value: f64,
    },
    /// Change a state variable (switch position, button).
    SetState {
        inst: String,
        name: String,
        value: f64,
    },
    Run,
    Pause,
    /// Advance one nominal step while paused.
    Step,
    /// Simulated seconds per real second.
    SetSpeed(f64),
    /// Nominal time step in seconds.
    SetTimeStep(f64),
    /// Recompute the operating point and restart at t = 0.
    Reset,
    /// Replace the probed scope channels.
    SetProbes(Vec<ProbeTarget>),
    Shutdown,
}

/// State published after each frame.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub generation: u64,
    pub time: f64,
    pub running: bool,
    /// Voltage per netlist node index.
    pub node_voltages: Vec<f64>,
    /// Per instance (by netlist order, ground instances skipped): name and
    /// branch currents.
    pub currents: Vec<(String, Vec<f64>)>,
    /// Per instance: probe values in definition order.
    pub probes: Vec<Vec<f64>>,
    /// Per instance: convergence trouble flag.
    pub trouble: Vec<bool>,
    /// Requested simulated seconds per real second.
    pub speed: f64,
    /// Achieved simulated seconds per real second.
    pub actual_speed: f64,
    pub time_step: f64,
    pub newton_iterations: usize,
    pub linear: bool,
    pub error: Option<String>,
    pub warnings: Vec<String>,
}

/// One scope sample.
#[derive(Debug, Clone, Copy)]
pub struct ScopeSample {
    pub time: f64,
    pub values: [f64; MAX_CHANNELS],
    /// Probe-list generation the sample belongs to.
    pub probes: u32,
}

impl Default for ScopeSample {
    fn default() -> Self {
        Self {
            time: 0.0,
            values: [0.0; MAX_CHANNELS],
            probes: 0,
        }
    }
}

/// UI-side handle to the simulation thread.
pub struct SimHandle {
    tx: Sender<Command>,
    snapshot: triple_buffer::Output<Snapshot>,
    samples: rtrb::Consumer<ScopeSample>,
    thread: Option<JoinHandle<()>>,
}

impl SimHandle {
    /// Start the simulation thread.
    pub fn spawn() -> Self {
        let (tx, rx) = unbounded();
        let (input, output) = triple_buffer::triple_buffer(&Snapshot::default());
        let (producer, consumer) = rtrb::RingBuffer::new(1 << 13);
        let thread = std::thread::Builder::new()
            .name("electrocraft-sim".into())
            .spawn(move || Engine::new(rx, input, producer).run())
            .expect("start simulation thread");
        Self {
            tx,
            snapshot: output,
            samples: consumer,
            thread: Some(thread),
        }
    }

    /// Send a command; ignored if the thread has stopped.
    pub fn send(&self, c: Command) {
        let _ = self.tx.send(c);
    }

    /// The newest snapshot.
    pub fn snapshot(&mut self) -> &Snapshot {
        self.snapshot.read()
    }

    /// Drain every scope sample produced since the last call.
    pub fn drain_samples(&mut self, out: &mut Vec<ScopeSample>) {
        while let Ok(s) = self.samples.pop() {
            out.push(s);
        }
    }
}

impl Drop for SimHandle {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Shutdown);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

struct Engine {
    rx: Receiver<Command>,
    out: triple_buffer::Input<Snapshot>,
    samples: rtrb::Producer<ScopeSample>,
    circuit: Option<Circuit>,
    netlist: Option<Netlist>,
    generation: u64,
    running: bool,
    speed: f64,
    h: f64,
    probes: Vec<ProbeTarget>,
    probe_gen: u32,
    error: Option<String>,
    actual: f64,
}

const FRAME: Duration = Duration::from_millis(16);

impl Engine {
    fn new(
        rx: Receiver<Command>,
        out: triple_buffer::Input<Snapshot>,
        samples: rtrb::Producer<ScopeSample>,
    ) -> Self {
        Self {
            rx,
            out,
            samples,
            circuit: None,
            netlist: None,
            generation: 0,
            running: false,
            speed: 1e-3,
            h: 1e-6,
            probes: Vec::new(),
            probe_gen: 0,
            error: None,
            actual: 0.0,
        }
    }

    fn rebuild(&mut self) {
        self.error = None;
        let Some(netlist) = &self.netlist else {
            self.circuit = None;
            return;
        };
        match Circuit::compile_with(netlist, Options::interactive()) {
            Ok(mut c) => {
                if let Err(e) = c.reset() {
                    self.error = Some(format!("operating point: {e}"));
                }
                self.circuit = Some(c);
            }
            Err(e) => {
                self.error = Some(e.to_string());
                self.circuit = None;
            }
        }
    }

    fn handle(&mut self, c: Command) -> bool {
        match c {
            Command::Rebuild {
                netlist,
                generation,
            } => {
                // Keep switch positions and live values across a rebuild.
                let old = self.circuit.take();
                self.netlist = Some(netlist);
                self.generation = generation;
                self.rebuild();
                if let (Some(old), Some(new)) = (old, self.circuit.as_mut()) {
                    let mut changed = false;
                    for k in 0..new.instance_count() {
                        if let Some(ok) = old.instance_index(new.instance_name(k)) {
                            for (s, v) in old.states(ok) {
                                if new.state(k, &s).is_some_and(|cur| cur != v) {
                                    changed |= new.control(k, &s, v);
                                }
                            }
                        }
                    }
                    if changed {
                        let _ = new.dc_op();
                        let _ = new.settle();
                    }
                }
            }
            Command::SetParam { inst, name, value } => {
                let mut recompile = false;
                if let Some(c) = &mut self.circuit
                    && let Some(k) = c.instance_index(&inst)
                    && !c.set_param(k, &name, value)
                {
                    recompile = true;
                }
                if let Some(n) = &mut self.netlist {
                    for i in n.instances.iter_mut().filter(|i| i.name == inst) {
                        i.params.insert(name.clone(), value);
                    }
                }
                if recompile || self.circuit.is_none() {
                    self.rebuild();
                }
            }
            Command::SetState { inst, name, value } => {
                if let Some(c) = &mut self.circuit
                    && let Some(k) = c.instance_index(&inst)
                {
                    c.control(k, &name, value);
                    if !self.running {
                        // Show the effect immediately even while paused.
                        let _ = c.settle();
                        let _ = c.dc_op();
                    }
                }
            }
            Command::Run => self.running = true,
            Command::Pause => self.running = false,
            Command::Step => {
                let h = self.h;
                if let Some(c) = &mut self.circuit {
                    c.step_interactive(h);
                    self.sample();
                }
            }
            Command::SetSpeed(s) => self.speed = s.clamp(1e-9, 10.0),
            Command::SetTimeStep(h) => self.h = h.clamp(1e-12, 1.0),
            Command::Reset => {
                self.rebuild();
            }
            Command::SetProbes(p) => {
                self.probes = p;
                self.probes.truncate(MAX_CHANNELS);
                self.probe_gen = self.probe_gen.wrapping_add(1);
            }
            Command::Shutdown => return false,
        }
        true
    }

    fn sample(&mut self) {
        if let Some(c) = &mut self.circuit {
            push_sample(c, &self.probes, self.probe_gen, &mut self.samples);
        }
    }

    fn publish(&mut self) {
        let snap = self.out.input_buffer_mut();
        snap.generation = self.generation;
        snap.running = self.running;
        snap.speed = self.speed;
        snap.actual_speed = self.actual;
        snap.time_step = self.h;
        snap.error = self.error.clone();
        match &mut self.circuit {
            Some(c) => {
                snap.time = c.time();
                let count = self.netlist.as_ref().map_or(0, |n| n.nodes.len());
                snap.node_voltages = (0..count).map(|i| c.voltage_of_netlist_node(i)).collect();
                snap.currents = (0..c.instance_count())
                    .map(|k| (c.instance_name(k).to_string(), c.branch_currents(k)))
                    .collect();
                snap.probes = (0..c.instance_count()).map(|k| c.probes(k)).collect();
                snap.trouble = (0..c.instance_count())
                    .map(|k| c.instance_trouble(k))
                    .collect();
                snap.newton_iterations = c.last_iterations;
                snap.linear = c.is_linear();
                snap.warnings.clone_from(&c.warnings);
            }
            None => {
                snap.time = 0.0;
                snap.node_voltages.clear();
                snap.currents.clear();
                snap.probes.clear();
                snap.trouble.clear();
                snap.warnings.clear();
            }
        }
        self.out.publish();
    }

    fn run(mut self) {
        let mut last = Instant::now();
        loop {
            // While paused, block for the next command; while running, drain.
            if !self.running {
                self.publish();
                match self.rx.recv() {
                    Ok(c) => {
                        if !self.handle(c) {
                            return;
                        }
                    }
                    Err(_) => return,
                }
                last = Instant::now();
            }
            while let Ok(c) = self.rx.try_recv() {
                if !self.handle(c) {
                    return;
                }
            }
            if !self.running {
                continue;
            }
            let frame_start = Instant::now();
            let real = frame_start.duration_since(last).as_secs_f64().min(0.1);
            last = frame_start;
            let target = self.speed * real;
            let budget = FRAME.mul_f64(0.6);
            let mut advanced = 0.0;
            if let Some(c) = &mut self.circuit {
                let h = self.h.min(target.max(self.h * 1e-3));
                let steps = (target / h).ceil().max(1.0) as usize;
                // Decimate scope samples to a few hundred per frame.
                let every = (steps / 400).max(1);
                for i in 0..steps {
                    advanced += c.step_interactive(h);
                    if i % every == 0 {
                        push_sample(c, &self.probes, self.probe_gen, &mut self.samples);
                    }
                    if frame_start.elapsed() > budget {
                        break;
                    }
                }
            }
            let real_frame = frame_start.elapsed().as_secs_f64().max(real).max(1e-6);
            self.actual =
                0.9 * self.actual + 0.1 * (advanced / real_frame.max(FRAME.as_secs_f64()));
            self.publish();
            let spent = frame_start.elapsed();
            if spent < FRAME {
                std::thread::sleep(FRAME - spent);
            }
        }
    }
}

fn push_sample(
    c: &mut Circuit,
    probes: &[ProbeTarget],
    generation: u32,
    out: &mut rtrb::Producer<ScopeSample>,
) {
    if probes.is_empty() {
        return;
    }
    let mut s = ScopeSample {
        time: c.time(),
        probes: generation,
        ..Default::default()
    };
    for (i, p) in probes.iter().enumerate() {
        s.values[i] = match p {
            ProbeTarget::Node(n) => c.voltage_of_netlist_node(*n),
            ProbeTarget::Diff(a, b) => {
                c.voltage_of_netlist_node(*a) - c.voltage_of_netlist_node(*b)
            }
            ProbeTarget::Current { inst, branch } => c
                .instance_index(inst)
                .and_then(|k| c.branch_currents(k).get(*branch).copied())
                .unwrap_or(0.0),
            ProbeTarget::Probe { inst, index } => match c.instance_index(inst) {
                Some(k) => c.probes(k).get(*index).copied().unwrap_or(0.0),
                None => 0.0,
            },
        };
    }
    let _ = out.push(s);
}
