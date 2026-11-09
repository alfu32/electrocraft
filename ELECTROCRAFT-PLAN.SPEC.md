# Circuit Simulator — Specification & Plan

Nov 9, 2026 · @Asdfghjkar

## 1. Overview & goals

A native Rust desktop app for drawing electronic schematics and simulating them live, in the spirit of iCircuit: you place parts, wire them, and see voltages, currents and waveforms update while you flip switches and turn knobs. On demand, the same circuit can be run through a slower, SPICE-grade analysis for accurate results.

**Goals**

- **Live interactive simulation:** the circuit runs continuously at a real-time or scaled pace. Edits and control changes (switches, potentiometers, sources) take effect immediately.
- **SPICE-grade analysis as a command:** DC operating point, transient, AC (frequency response) and parameter sweeps, run on request with tight tolerances. The results open in a plot window.
- **Programmable components:** every component, built-in or user-made, is defined through one contract. Users write their own components in JavaScript.
- **Model import:** components can be created from SPICE `.model` and `.subckt` files. A separate companion tool turns datasheet data into component definitions.
- **Mixed-signal:** analog and digital parts in the same schematic.

**Non-goals for v1**

- PCB layout, footprints, routing.
- RF and electromagnetic field simulation, transmission lines beyond a lossless ideal model.
- Thermal simulation (device temperature is a fixed parameter).
- Collaborative or cloud editing.

## 2. Decision log

The choices below are settled. The coding agent should treat them as constraints, not suggestions.

| # | Decision | Choice | Reason |
| --- | --- | --- | --- |
| D1 | Language | Rust (stable, 2021 edition or later) | Owner's choice. Performance and safety for the solver. |
| D2 | UI shell | egui (via eframe) for all panels, menus, dialogs, inspectors | Proven in Meshcraft. |
| D3 | Schematic canvas rendering | Raylib 6 rendering off-screen into a render texture. The pixels are copied each frame into an egui texture and shown as an image. | Same pattern as Meshcraft. Raylib has been artifact-free on the owner's machine. |
| D4 | GPU API | No WGPU anywhere, including egui's backend: use eframe's glow (OpenGL) backend | WGPU apps showed rendering artifacts on the owner's machine. |
| D5 | Simulation method | Modified Nodal Analysis (MNA), time-stepped, with companion models and Newton-Raphson. Digital parts run on an event layer coupled to the analog solver. | Industry standard (SPICE, iCircuit, Falstad). Handles both live and accurate modes. |
| D6 | Two simulation modes | **Interactive** (continuous, real-time, relaxed tolerances) and **Analysis** (commands: DC OP, transient, AC, sweeps; SPICE tolerances) | Owner wants both. Both use the same engine and component models with different settings. |
| D7 | Component contract | Components declare physics (pins, parameters, branch currents and charges as expressions of voltages). The engine derives stamps, derivatives and integration. | Users never touch solver internals. The solver can evolve without rewriting components. |
| D8 | User component language | JavaScript (ES2020) on the QuickJS engine, via the `rquickjs` crate | Owner's choice. Widely known, so users can write components without learning a new language. Scripts only build expressions; the hot loop runs native code, so script speed does not limit simulation speed. |
| D9 | Model sources | SPICE `.model` / `.subckt` import in the main app; datasheet-to-model conversion in a separate companion tool | Manufacturers publish SPICE models. Datasheets give parameters for known model families, not programs. |

## 3. Architecture

The simulator is a headless library with no UI dependency. The app talks to it only through messages. That keeps the engine testable from the command line and lets the datasheet tool reuse it.

&#91;embedded content: runtime architecture · UI thread, simulation thread, analysis workers\]

The UI thread sends edits and parameter changes to the simulation thread and reads back state snapshots and scope samples. Analysis commands run on worker threads, each with its own copy of the compiled circuit.

**Workspace crates**

| Crate | Kind | Responsibility | Depends on |
| --- | --- | --- | --- |
| `ecs-units` | lib | SI units, engineering notation parsing and formatting (`4k7`, `100n`, `2.2M`) | none |
| `ecs-expr` | lib | Expression graph, simplification, forward-mode autodiff, compilation to a flat instruction tape | ecs-units |
| `ecs-model` | lib | Component definition contract: pins, parameters, internal nodes, branches, behavioral and digital hooks | ecs-expr |
| `ecs-sim` | lib | Netlist compiler, MNA assembly, sparse LU, Newton-Raphson, integration, timestep control, event layer, all analyses | ecs-model |
| `ecs-lib` | lib | Built-in components, written in Rust against the same contract | ecs-model |
| `ecs-script` | lib | JavaScript host (QuickJS via `rquickjs`): loads `.js` component files into `ecs-model` definitions and parses `e` expression templates through `ecs-expr` | ecs-model |
| `ecs-spice` | lib | SPICE netlist, `.model` and `.subckt` parser and mapper | ecs-model, ecs-lib |
| `ecs-schematic` | lib | Schematic document: symbols, placement, wires, junctions, labels, undo/redo, netlist extraction | ecs-model |
| `ecs-render` | lib | Raylib off-screen schematic renderer, hit testing, live overlays | ecs-schematic |
| `ecs-app` | bin | eframe application: shell, editor tools, scopes, plots, settings | all of the above |
| `ecs-cli` | bin | Headless runner: netlist in, CSV or JSON results out. Used for tests and benchmarks. | ecs-sim, ecs-lib, ecs-script, ecs-spice |
| `datasheet-tool` | bin | Companion app, later phase (section 8) | ecs-model, ecs-sim |

`ecs` is a placeholder prefix until the product has a name.

**Threads and messages**

- **UI thread** owns the egui context, the Raylib context and the schematic document. The Raylib integration follows Meshcraft: render the schematic into a `RenderTexture2D`, read the pixels back and update an egui texture each frame the canvas is dirty.
- **Simulation thread** owns the compiled circuit and the interactive engine. It receives commands over a `crossbeam-channel`: `Rebuild(netlist)`, `SetParam`, `SetControl` (switch, potentiometer), `Run`, `Pause`, `SetSpeed`, `Reset`.
- **State out:** after each frame's worth of steps, the engine publishes a snapshot (node voltages, branch currents, digital states) through a lock-free triple buffer. The UI reads the newest one; old ones are dropped.
- **Samples out:** probed signals stream through a single-producer, single-consumer ring buffer per scope so no sample is lost between frames.
- **Analysis workers** come from a small pool. Each job gets a clone of the compiled circuit, reports progress, can be cancelled, and returns a result set (named vectors over time or frequency).
- **Topology edits** (adding a part, moving a wire) trigger a full recompile on the simulation thread. Parameter edits update values in place without recompiling.

## 4. Simulation engine

One engine serves both modes. The circuit is a system of nonlinear differential equations, solved one time step at a time with Newton-Raphson. The modes differ only in tolerances, step control and pacing (4.8).

### 4.1 Formulation

The unknown vector x holds every node voltage (ground is node 0 and excluded) plus one current for each branch that needs one: voltage sources, inductors, controlled voltage sources, and any component branch declared as a voltage-defined branch.

Every component contributes to a residual in charge-oriented form: a resistive part i(x, t) and a reactive part q(x), its stored charge or flux. Kirchhoff's current law at every node, plus each branch equation, gives:

```latex
F(x) = i(x, t) + \frac{d}{dt} q(x) = 0
```

Newton-Raphson solves each step by repeatedly solving the linear system below, where α comes from the integration method (4.2):

```latex
\left( \frac{\partial i}{\partial x} + \alpha \frac{\partial q}{\partial x} \right) \Delta x = -F(x)
```

The engine computes i, q and both Jacobians from each component's expressions using automatic differentiation (section 5). Components never write matrix stamps by hand. The charge form conserves charge exactly, which matters for switched-capacitor and nonlinear capacitance models.

### 4.2 Time integration

- **Backward Euler:** first step, the step after any breakpoint (switch flip, source edge, digital transition), and as a fallback after failed steps. Very stable, damps ringing.
- **Trapezoidal:** default otherwise. Second order, accurate for oscillators and filters.
- **Gear-2 (BDF2):** optional in Analysis mode, for circuits where trapezoidal produces numerical ringing.

The integrator keeps the history of q per component. A component never sees the integration method.

### 4.3 Nonlinear solving

- **Convergence test:** each node voltage must satisfy |Δv| < reltol·|v| + vntol, and each branch current |Δi| < reltol·|i| + abstol. Defaults: reltol 1e-3, vntol 1 µV, abstol 1 pA.
- **Limiting:** components can declare limiting on a controlling voltage (the SPICE `pnjlim` style for junctions, `fetlim` for MOS gates). The engine applies it between iterations to stop exponentials from overflowing.
- **Gmin:** a tiny conductance (default 1e-12 S) across every nonlinear junction and from every node to ground, so no node floats.
- **DC operating point fallbacks:** plain Newton first; then gmin stepping (start at 1e-3 S, shrink to the default); then source stepping (ramp all independent sources from 0 to 100%). Report which method succeeded.

### 4.4 Linear solver

- The matrix sparsity pattern depends only on topology. The compiler computes it, chooses a fill-reducing ordering and does the symbolic factorization once per topology. Each Newton iteration only refactors numerically.
- Use `faer` for sparse LU. Hide it behind a `LinearSolver` trait so a dense LU (faster for small circuits, under about 50 unknowns) can be swapped in. The agent benchmarks both and picks the switch-over size.
- **Linear fast path:** if a circuit has no nonlinear components and the step size is fixed, the matrix never changes. Factor once and reuse the factors for every step until a switch or parameter change. Most beginner circuits hit this path, so live mode is very fast for them.
- Analysis mode AC uses the same pattern with complex numbers (`faer` supports complex LU).

### 4.5 Interactive mode

The goal is a responsive, believable picture, not the last digit of accuracy.

- **Pacing:** the user sets a simulation speed (simulated time per real second) and a nominal time step. Each UI frame, the engine advances simulated time by speed × frame time, within a CPU budget (default 60% of a frame). If it cannot keep up, it slows down and the status bar shows the actual rate.
- **Step control:** fixed nominal step, reduced automatically around breakpoints and when Newton fails (halve up to 8 times, then grow back). No truncation-error control.
- **Failure is never fatal:** after the step floor is reached, the engine accepts the step, flags the circuit with a "convergence trouble" marker on the offending component, and keeps running.
- **Live controls:** switches, potentiometers, and source values change between steps and insert a breakpoint.
- **Reset:** recompute the DC operating point and restart time at zero.

### 4.6 Analysis mode commands

Analysis commands are invoked from a menu or the command palette. Each runs on a worker and opens its result in a plot window.

| Command | SPICE equivalent | What it does | Phase |
| --- | --- | --- | --- |
| DC operating point | `.op` | Solve with capacitors open and inductors shorted. Annotate every node and part on the schematic. | v1 |
| DC sweep | `.dc` | Sweep one or two sources or parameters, solve the operating point at each value | v1 |
| Transient | `.tran` | Variable-step transient with truncation-error control, breakpoints, start from OP or user initial conditions | v1 |
| AC small-signal | `.ac` | Linearize at the operating point, sweep frequency (linear, per decade, per octave). Bode plot. | v1 |
| Parameter step | `.step` | Repeat any analysis over a list or range of parameter values, in parallel | v1 |
| FFT | none | Spectrum of any transient result, computed in the plot window | v1 |
| Noise | `.noise` | Output and input-referred noise density | later |
| Monte Carlo and temperature | `.mc`, `.temp` | Tolerance and temperature spreads | later |

AC analysis needs nothing extra from components: the small-signal model is the Jacobians ∂i/∂x and ∂q/∂x at the operating point, which autodiff already provides.

### 4.7 Digital event layer

- Each digital pin belongs to a **logic family** (e.g. 5 V CMOS, 3.3 V CMOS, TTL) that defines input thresholds, output high and low voltages, output resistance, and rise and fall times.
- **Inputs** read the analog node voltage and convert it to 0, 1 or X (between thresholds), with optional hysteresis.
- **Outputs** drive the analog node through a Thevenin equivalent (voltage and resistance). A change ramps over the family's rise or fall time rather than jumping, which keeps Newton stable.
- **Events:** a digital component reacts to input changes by scheduling output changes after its propagation delay on a time-ordered queue. The analog stepper always lands exactly on queued event times.
- **Later optimization:** nets with only digital pins can be resolved purely in the event domain without analog nodes.

### 4.8 Mode settings

| Setting | Interactive | Analysis |
| --- | --- | --- |
| reltol | 1e-2 | 1e-3 (user-settable) |
| Max Newton iterations per step | 20 | 100 |
| Step control | Fixed nominal, halving on failure | Truncation-error control, min and max step |
| Integration | Trapezoidal, Euler at breakpoints | User choice: trapezoidal or Gear-2 |
| On non-convergence | Accept step, flag component, continue | Abort with a diagnostic naming the nodes that failed |
| Pacing | Real time × speed factor | As fast as possible |
| Output | Snapshots and probed signals only | Every node and branch current, every accepted step |

## 5. Component model contract

A component is data, not solver code: it declares its pins, parameters and branch equations as expressions, and the engine does everything else. Built-in Rust components, JavaScript components, SPICE imports and subcircuits all produce the same `ComponentDef`.

### 5.1 What a definition contains

| Part | Contents | Example |
| --- | --- | --- |
| Identity | Stable id, display name, category, version, author | `core.diode`, "Diode", Semiconductors |
| Pins | Name, kind: analog, or digital with a logic family | `A`, `K` |
| Parameters | Name, unit, default, range, type (real, integer, enum, bool, file), and a **live** flag for values changeable while running | `Is = 1e-14 A`, `N = 1`, `Rs = 0 Ω` |
| Internal nodes | Extra nodes private to the component | Node between Rs and the junction |
| Current branches | Between two nodes: current i = f(…) and charge q = g(…) | Junction current and depletion charge |
| Voltage branches | Between two nodes: voltage v = f(…). Adds a branch-current unknown. | Voltage source, inductor (as flux) |
| State variables | Discrete values changed only by events, read by expressions | Switch position, flip-flop Q |
| Behavior hooks | Callbacks on input change, timers and user controls. They may change state and schedule events. | Clock edge, button press |
| Breakpoints | Times where the component's sources are discontinuous | Edges of a pulse source |
| Limiting | Which controlling voltages use junction or FET limiting | `pn` limiting on V(A, K) |
| Probes | Named quantities shown in the inspector and plottable | Power dissipated, junction current |
| Symbol | Reference to a symbol file (drawing and pin positions), separate from the electrical model | `symbols/diode.sym` |

### 5.2 Expressions

Expressions are built as a graph, never as strings evaluated at run time. Leaves and operations:

- **Leaves:** constants, parameters, node voltages V(n) or V(a, b), branch currents I(branch), time t, temperature, state variables.
- **Arithmetic:** + − × ÷, pow, neg, abs, min, max.
- **Functions:** exp, ln, log10, sqrt, sin, cos, tan, atan, tanh, sinh, cosh.
- **Safe variants:** `limexp(x)`, exponential that continues linearly beyond x = 80 so junctions cannot overflow; `safe_div`, `safe_sqrt`.
- **Conditionals:** `select(cond, a, b)` with comparisons. Conditionals on voltages are allowed but create kinks; the library provides `smoothstep` and `smooth_abs` and documentation recommends them.

**Compilation pipeline** (in `ecs-expr`): constant folding (non-live parameters are folded at compile time), common subexpression elimination, then forward-mode automatic differentiation with respect to every unknown the expression reads. The result is a flat instruction tape per component that computes values and all partial derivatives in one pass. A tape interpreter in Rust runs it in the hot loop. A JIT (for example Cranelift) is out of scope but the tape format must not rule it out.

### 5.3 Rust shape of the contract

```rust
pub struct ComponentDef {
    pub id: ComponentId,
    pub meta: Meta,                       // name, category, version, docs
    pub pins: Vec<PinDef>,
    pub params: Vec<ParamDef>,
    pub internal_nodes: Vec<NodeName>,
    pub branches: Vec<BranchDef>,         // current or voltage branches
    pub states: Vec<StateDef>,
    pub limits: Vec<LimitDef>,
    pub probes: Vec<ProbeDef>,
    pub behavior: Option<Box<dyn Behavior>>, // events, timers, controls
    pub breakpoints: Option<Expr>,        // or a callback; see 5.4
    pub symbol: SymbolRef,
    pub kind: DefKind,                    // Primitive or Subcircuit(netlist)
}

pub enum BranchDef {
    Current { a: NodeRef, b: NodeRef, i: Expr, q: Option<Expr> },
    Voltage { a: NodeRef, b: NodeRef, v: Expr, flux: Option<Expr> },
}

pub trait Behavior: Send {
    fn on_inputs(&mut self, ctx: &mut EventCtx);   // digital inputs changed
    fn on_timer(&mut self, ctx: &mut EventCtx, tag: u32);
    fn on_control(&mut self, ctx: &mut EventCtx, control: &ControlEvent);
}
```

`EventCtx` lets a behavior read digital input levels, node voltages and state; write state; schedule digital outputs and timers. It cannot add nodes or change expressions. Structural changes require a recompile.

### 5.4 Rules the engine enforces

- Every expression must be finite for any input. The engine checks for NaN and infinity after each evaluation and reports the component by name.
- Voltage branches in a loop with no resistance (two ideal sources in parallel) and nodes with no DC path to ground are detected at compile time and reported, not left to the solver.
- Subcircuits flatten at compile time. Their internal nodes are namespaced (`X1.Q3.b`) so probes and error messages stay readable.
- A `ComponentDef` is immutable after loading. Live parameters are the only values that change during a run.

## 6. Scripting with JavaScript

Users write components as JavaScript (ES2020) files, run by the QuickJS engine through the `rquickjs` crate. A script runs once at load time to build a `ComponentDef`. After that, only its behavior hooks run, and only on events. JavaScript never runs inside the Newton loop, so script speed does not limit simulation speed.

**Why QuickJS.** It is small (under 1 MB), embeds cleanly, and supports memory limits and an interrupt handler for runaway code. V8 (through `deno_core`) is far larger and harder to build. Boa, a pure-Rust engine, can be reconsidered later; the scripting front end is thin, so swapping engines does not touch the simulator.

**Equations use a tagged template.** JavaScript has no operator overloading, so `a * b` cannot build an expression graph. Equations are written as `` e`Is * (limexp(V(A, K) / (N * vt)) - 1)` ``. The `e` tag parses the text with the same expression parser (`ecs-expr`) used for arbitrary-expression sources in the inspector, so there is one expression language everywhere. `${}` inserts an expression object or a number. Parse errors report file, line and column.

**TypeScript support.** The app ships a `.d.ts` file describing the API, so editors such as VS Code autocomplete and type-check component files. Users may write TypeScript and compile it themselves; v1 loads only JavaScript.

### 6.1 Script API

- `export default component({ ... })` returns a definition. Fields mirror section 5.1. Files are ES modules; `import` resolves only inside the component's own package.
- `param(default, unit, description, opts)` declares a parameter. `opts` holds range, enum values and `live: true`.
- Pins: a plain name is analog; `digital(family)` and `digitalOut(family)` declare digital pins.
- Inside `equations(c) { ... }`:
  - In `e` templates, names resolve to parameters, `V(a)`, `V(a, b)`, `I(branch)`, `t`, `temp`, `vt` (thermal voltage at the simulation temperature), state variables, names declared with `c.let`, and the functions of 5.2.
  - `c.current(a, b, i, q)` and `c.voltage(a, b, v, flux)` add branches; `c.node(name)` adds an internal node.
  - `c.let(name, expr)` names a subexpression for reuse in later templates; `c.limit("pn" | "fet", expr)`; `c.probe(name, expr, unit)`.
- Hooks: `onInputs(ctx)`, `onTimer(ctx, tag)`, `onControl(ctx, event)`. `ctx` offers `level(pin)` (0, 1 or "X"), `rising(pin)`, `falling(pin)`, `voltage(pin)`, `state`, `param`, `drive(pin, level, delay)`, `timer(delay, tag)`.

### 6.2 Examples

A diode with junction capacitance:

```javascript
export default component({
  id: "user.simple_diode",
  name: "Simple diode",
  category: "Semiconductors",
  symbol: "diode",
  pins: ["A", "K"],
  params: {
    Is: param(1e-14, "A", "Saturation current"),
    N:  param(1.0,   "",  "Emission coefficient"),
    Cj: param(2e-12, "F", "Junction capacitance"),
  },
  equations(c) {
    c.let("Id", e`Is * (limexp(V(A, K) / (N * vt)) - 1)`);
    c.current("A", "K", e`Id`, e`Cj * V(A, K)`);
    c.limit("pn", e`V(A, K)`);
    c.probe("P", e`V(A, K) * Id`, "W");
  },
});
```

A D flip-flop, purely behavioral:

```javascript
export default component({
  id: "user.dff",
  name: "D flip-flop",
  category: "Digital",
  symbol: "dff",
  pins: { D: digital(), CLK: digital(), Q: digitalOut(), QN: digitalOut() },
  params: { tpd: param(10e-9, "s", "Propagation delay") },
  states: { q: 0 },
  onInputs(ctx) {
    if (ctx.rising("CLK")) {
      const d = ctx.level("D");
      ctx.state.q = d;
      ctx.drive("Q", d, ctx.param.tpd);
      ctx.drive("QN", d === "X" ? "X" : 1 - d, ctx.param.tpd);
    }
  },
});
```

### 6.3 Sandbox and lifecycle

- **Available:** the ECMAScript standard library and the component API only. No file system, network, process, `setTimeout`, or Node or browser APIs. `eval` and the `Function` constructor are disabled.
- **Limits:** a memory cap per QuickJS runtime and an interrupt handler that enforces a time budget per hook call. A hook that exceeds its budget is stopped, and the component is flagged with the error.
- **Threads:** each simulation thread and each analysis worker owns its own QuickJS runtime. JavaScript values never cross threads.
- **Hot reload:** the app watches user component folders. Saving a `.js` file reloads the definition and recompiles open circuits that use it. Load errors appear in the problems panel with file and line.
- **Built-ins in JavaScript:** a few built-in components are also shipped as JavaScript (for example the flip-flop above) and serve as reference examples for users.

## 7. Built-in component library (v1)

v1 ships about 60 components, enough for typical hobby and education circuits. Each is defined against section 5 and has a test circuit with a known answer (section 11).

| Category | Components | Model notes |
| --- | --- | --- |
| Passive | Resistor, capacitor, inductor, potentiometer, transformer, fuse | Capacitor and inductor with optional series resistance. Potentiometer wiper is a live parameter. Transformer as coupled inductors with coupling k. Fuse opens above its I²t rating. |
| Sources | Ground, DC voltage, DC current, battery, sine, pulse, piecewise linear, function generator, rail label | Battery has internal resistance. Pulse and PWL sources declare breakpoints. Function generator: sine, square, triangle, sawtooth. |
| Controlled and behavioral sources | VCVS, VCCS, CCVS, CCCS, arbitrary-expression voltage and current source | The arbitrary source takes a user expression in the 5.2 language, typed in the inspector |
| Switches and inputs | SPST, SPDT, push button, relay, voltage-controlled switch | Relay: coil inductance and resistance, pull-in and drop-out currents, contact bounce off by default |
| Diodes | Diode, Zener, LED, Schottky | SPICE level-1 diode with Rs, Cj, breakdown. LED glows on the canvas in proportion to its current. |
| Transistors | NPN, PNP, N-MOSFET, P-MOSFET, N-JFET, P-JFET | BJT: Gummel-Poon subset (Is, Bf, Br, Vaf, Rb, Rc, Re, Cje, Cjc, Tf). MOSFET: level 1 (Shichman-Hodges) with Cgs, Cgd, body diode. JFET: SPICE JFET model. |
| Analog ICs | Ideal op-amp, op-amp macromodel, comparator, 555 timer, linear regulator | Macromodel: gain-bandwidth, slew rate, output swing to rails, current limit. 555 and regulator are behavioral. |
| Digital | AND, OR, NAND, NOR, XOR, XNOR, NOT, buffer, tri-state buffer, D, JK and T flip-flops, counter, shift register, multiplexer, BCD to 7-segment decoder | Families: 5 V CMOS, 3.3 V CMOS, TTL |
| Digital I/O | Logic toggle, clock, logic probe | Logic toggle is clickable while running |
| Indicators | Lamp, 7-segment display, LED bar | Lamp is a temperature-dependent resistor; brightness drawn on the canvas |
| Instruments | Voltmeter, ammeter, probe, scope channel | Ideal: infinite or zero impedance, no effect on the circuit |

Later: MOSFET level 3 and BSIM, motors, speakers with audio output, microcontroller blocks, optocouplers, SCR and TRIAC.

## 8. Model import

There are two routes from a real part to a component. SPICE import is in the main app and comes first, because most manufacturers publish SPICE models. Datasheet conversion is a separate companion tool built later.

### 8.1 SPICE import (`ecs-spice`)

- **Input:** `.lib`, `.mod`, `.sub`, `.cir` files, or text pasted into a dialog.
- **Supported cards:** `.model` for D, NPN, PNP, NMOS and PMOS (level 1), NJF and PJF; `.subckt` built from R, C, L, V, I, E, F, G, H, D, Q, M, J, X and behavioral B sources (the expression subset that maps to 5.2); `.param`, `.include`, `.lib`; SPICE engineering suffixes (`meg`, `u`, `n`, `p`).
- **Mapping:** each `.model` becomes a built-in component with its parameters filled in. Each `.subckt` becomes a subcircuit component. Models for unsupported levels map to the nearest supported level with a warning listing ignored parameters.
- **Symbols:** the import dialog lets the user pick a standard symbol and map pins to subcircuit ports. Without a choice, a rectangle symbol with labeled pins is generated.
- **Output:** a component package (section 10) saved to the user library, plus an import report listing anything ignored or approximated.
- **Dialects:** plain SPICE3 and the common LTspice and PSpice extensions. Encrypted vendor models are not supported.

### 8.2 Datasheet tool (`datasheet-tool`, later phase)

A datasheet gives numbers and curves, not equations. The tool fits parameters of a known model family to those numbers and writes a JavaScript component.

1. Pick a model family: diode, Zener, LED, BJT, MOSFET, op-amp macromodel, regulator.
2. Enter datasheet points in a form for that family. Examples: forward voltage at given currents; hFE versus collector current; Ciss, Coss and Crss; gain-bandwidth and slew rate.
3. Optionally digitize curves: load a screenshot of a datasheet graph, calibrate the two axes (linear or log), click points along a curve.
4. Fit: Levenberg-Marquardt least squares, evaluating the model through `ecs-sim` test benches (for example a swept current source into the diode).
5. Review: overlay plots of datasheet points against the fitted model, with error per point.
6. Export a JavaScript component file and a fit report into a component package.

## 9. Editor & UI

egui owns every widget, panel and input event. Raylib only draws the schematic into an off-screen texture that egui shows as an image. Editing logic lives in `ecs-schematic` and `ecs-app`, never in the renderer.

### 9.1 Window layout

- **Top:** menu bar, toolbar (select, wire, place, probe; run, pause, reset; speed slider), command palette (Ctrl+K).
- **Left:** component palette: searchable, grouped by category, favorites, user and imported packages.
- **Center:** schematic canvas.
- **Right:** inspector for the selection: parameters with units, live controls (slider for a potentiometer, toggle for a switch), probe values.
- **Bottom:** scopes, problems panel (load errors, compile errors, convergence warnings), analysis log.
- **Floating or docked:** analysis plot windows. Use a dock layout crate such as `egui_dock`.
- **Status bar:** simulated time, requested and actual speed, step size, solver state.

### 9.2 Canvas integration

- Raylib renders the schematic into a `RenderTexture2D` sized to the canvas in physical pixels (respecting DPI scale). The pixels are copied into an egui texture, the same path as Meshcraft.
- Re-render only when the canvas is dirty: an edit, a pan or zoom, a hover change, or new simulation state while running.
- egui forwards pointer and keyboard input over the canvas rect. The app converts it to schematic coordinates and feeds it to the active tool's state machine.
- **Performance budget:** a 2560×1440 canvas is about 15 MB per read-back. Measure the copy cost early (phase 1). If it is too slow at high resolution while the simulation animates, see open question Q2.

### 9.3 Editing

- Grid with snapping; place, move, rotate (R), mirror (M), delete.
- Wire tool with orthogonal routing; junctions created automatically where a wire ends on another wire; wires stretch when parts are dragged.
- Box and click selection, copy and paste, duplicate.
- Net labels and power symbols that connect by name; text annotations.
- Undo and redo for every edit (command pattern in `ecs-schematic`).
- Zoom with the wheel, pan with the middle button or space+drag, fit to view.
- Symbols are vector files (lines, arcs, circles, polygons, text, pin points), drawn by Raylib. v1 ships ANSI-style symbols. IEC style is a later option.

### 9.4 Live visualization (the iCircuit feel)

- **Wire color by voltage:** a diverging color scale, positive green through grey at 0 V to negative red, auto-ranged to the circuit's supply rails.
- **Current dots** move along wires and through parts, with speed proportional to current on a log scale, so microamps and amps are both visible.
- **Part effects:** LEDs and lamps glow, 7-segment displays light, switches show their state, relays click over.
- **Hover:** tooltip with the node voltage, or the part's current, voltage and power.
- **Problem markers:** a badge on any part flagged by the solver.
- After a DC operating point analysis, node voltages and part currents are annotated on the schematic until the next edit.

### 9.5 Scopes and plots

- **Scopes (live):** click a node or part with the probe tool to add a channel. Each scope has time per division, per-channel scale and offset, auto-scale, edge trigger with level, XY mode, and two cursors with delta readout.
- **Plot windows (analysis):** built with `egui_plot`. Multiple traces, linear and log axes, Bode layout (magnitude and phase), cursors, FFT of a transient trace, export to CSV and PNG.
- A parameter-step result shows one trace per step, colored along a scale with a legend.

## 10. File formats

All native files are pretty-printed JSON (serde) with a `format_version` field and stable key order, so they diff cleanly in git and other tools can read them. Loaders migrate older versions forward.

| File | Extension | Contents |
| --- | --- | --- |
| Schematic | `.ecs` | Metadata; component instances (unique id, definition id and version, package, position, rotation, mirror, parameter overrides, reference label like R1); wires as polylines; junctions; net labels; text; probes and scope setup; simulation settings; saved analysis presets |
| Symbol | `.sym.json` | Vector primitives, pin points with names and directions, label anchors, bounding box |
| Component package | folder, or `.ecspkg` (zip) | `package.json` manifest (id, name, version, author, license, list of components); `.js` component files; `symbols/`; `models/` with the original SPICE files; `docs/` |
| Analysis result | `.ecsr` | Analysis command and settings, then named vectors (time or frequency axis plus signals). Binary payload with a JSON header, for size. |

**Exports:** SPICE netlist (to cross-check a circuit in ngspice), CSV of any plot or scope, PNG of the canvas. SVG export of the schematic comes later.

**Compatibility rule:** a schematic stores the version of each component definition it used. If a newer version is installed, the app offers to update, and keeps the old behavior until the user accepts.

## 11. Testing & validation

The simulator is checked against math first, then against ngspice. Every phase in section 12 ends with its tests green in CI.

- **Expression engine:** property tests (`proptest`) comparing autodiff derivatives with finite differences over random expressions and inputs; constant folding and CSE must not change results.
- **Analytic circuits:** results within 0.1% of closed-form answers: voltage divider, RC charge and discharge, RL, series RLC ringing frequency and damping, RC low-pass corner frequency in AC, Wheatstone bridge.
- **ngspice golden corpus:** about 50 netlists run through `ecs-cli` and ngspice in CI. Waveforms are resampled to a common time base. Analysis mode must match within 1% of the signal's range; Interactive mode within 5%.
- **Convergence torture set:** Schmitt trigger, latch, ring oscillator, bridge rectifier with a large reservoir capacitor, flyback converter, op-amp at its rails. DC OP must converge and transient must finish without aborts.
- **Digital:** truth tables for every gate, flip-flop timing, a 4-bit counter driven by a 555 clock, mixed-signal threshold crossing.
- **Performance (`criterion`):** steps per second on reference circuits, tracked per commit. Initial target to revise after phase 2: a 50-component mixed circuit runs real time at a 5 µs step on a mid-range desktop CPU.
- **Scripting:** sandbox escape attempts (file or network access, `eval`, the `Function` constructor) fail; runaway hooks are stopped by the time budget; hot reload keeps the app running after a broken script; `e` template parse errors point to the right file, line and column.
- **Editor:** `ecs-schematic` commands tested headless (place, wire, junction creation, undo and redo, netlist extraction). Renderer snapshot tests compare off-screen PNGs of every symbol.
- **Files:** save, load and save again gives identical bytes; older format versions load through migrations.

## 12. Implementation plan

The work is split into nine phases. The engine (P1, P2) and the editor (P3) proceed in parallel and meet in P4, the first version that feels like iCircuit. v1 is P0 to P7; the datasheet tool follows.

&#91;embedded content: phase plan · 9 phases, 1 release gate\]

P5, P6 and P7 depend only on P4 and can run in any order or in parallel.

**Rules for the coding agent**

- Follow the decision log (section 2). Never add a dependency on WGPU, directly or through a crate's default features.
- Keep crate boundaries from section 3: `ecs-sim` and below must build and test with no UI crates.
- Every phase ends with its acceptance tests passing in CI: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, plus the phase's own checks.
- Deliver each phase as a series of small, reviewable commits. Public APIs get rustdoc comments.
- When the spec is unclear or wrong, stop and record the question rather than guessing silently.

### P0 Foundations

- Cargo workspace with all crates from section 3 as stubs; CI pipeline.
- `ecs-units`: SI values, engineering notation parse and format.
- `ecs-expr`: expression graph, constant folding, CSE, forward-mode autodiff, instruction tape and interpreter.
- `ecs-model`: `ComponentDef` and related types from section 5.3.
- **Done when:** autodiff property tests pass; a resistor and a diode can be defined in Rust and their tapes evaluated.

### P1 Linear engine (headless)

- Netlist compiler: node numbering, branch unknowns, sparsity pattern, topology checks (5.4).
- MNA assembly from tapes; `LinearSolver` trait with sparse (`faer`) and dense implementations.
- Components: resistor, capacitor, inductor, DC and AC voltage and current sources, pulse, PWL, controlled sources.
- Fixed-step transient with backward Euler and trapezoidal; linear fast path; breakpoints.
- `ecs-cli`: reads a small SPICE-like netlist, writes CSV.
- **Done when:** all linear analytic circuits in section 11 pass.

### P2 Nonlinear engine and analyses

- Newton-Raphson with limiting, gmin, convergence tests; DC operating point with gmin and source stepping.
- Variable-step transient with truncation-error control; Gear-2; AC small-signal; DC sweep; parameter step.
- Components: diodes, BJTs, MOSFETs, JFETs, op-amp models, switches, relay, fuse, lamp.
- ngspice golden corpus and convergence torture set in CI.
- **Done when:** the corpus matches ngspice within 1% and the torture set passes.

### P3 Editor and canvas (parallel with P1 and P2)

- eframe app on the glow backend; window layout from 9.1.
- Raylib off-screen canvas following Meshcraft; measure read-back cost at 1080p and 1440p and record it.
- Symbol format and ANSI symbols for every v1 component.
- `ecs-schematic`: placing, wiring, junctions, labels, selection, undo and redo, netlist extraction; `.ecs` save and load.
- **Done when:** a user can draw any section 11 test circuit, save it, reload it, and the extracted netlist matches the hand-written one.

### P4 Live simulation

- Simulation thread, command channel, triple-buffered snapshots, sample ring buffers (section 3).
- Interactive mode from 4.5: pacing, step halving, non-fatal failure with markers, live controls, reset.
- Visualization from 9.4: wire colors, current dots, part effects, hover tooltips; scopes from 9.5.
- **Done when:** an astable multivibrator, an RC filter driven by a function generator, and a bridge rectifier run live at real time with smooth animation, and switching a switch takes effect within one frame.

### P5 Digital and mixed-signal

- Event queue, logic families, digital pins coupled to analog (4.7).
- Digital components and I/O from section 7; 555 timer.
- **Done when:** a 555-clocked 4-bit counter driving a 7-segment display runs live and matches expected timing.

### P6 JavaScript scripting

- `ecs-script` with the API from 6.1, sandbox and limits from 6.3, hot reload, problems panel.
- Reference JavaScript components, the .d.ts API declarations, and a short user guide.
- **Done when:** the two examples in 6.2 load, simulate correctly, and sandbox tests pass.

### P7 Analysis UI and SPICE import

- Analysis commands in the UI with settings dialogs; plot windows with Bode layout, cursors, FFT, CSV and PNG export; operating point annotation.
- `ecs-spice` import with mapping, symbol selection and report (8.1); SPICE netlist export.
- **Done when:** a vendor op-amp `.subckt` and a transistor `.model` import and reproduce ngspice results; a Bode plot of an active filter matches ngspice.

### P8 Datasheet tool (after v1)

- Separate app implementing 8.2: forms per family, curve digitizer, fitter, overlay review, JavaScript export.
- **Done when:** a diode and an NPN transistor fitted from their datasheets reproduce the datasheet points within 5%.

## 13. Open questions

- [ ] **Q1 Name:** what is the product called? It sets the crate prefix (`ecs` for now) and the file extensions.
- [ ] **Q2 Canvas copy:** if reading the Raylib texture back to the CPU is too slow at high resolutions, can Raylib and egui share one OpenGL context, so egui draws Raylib's texture directly (`egui_glow` can register a native texture)? This depends on how Meshcraft creates its Raylib context.
- [ ] **Q3 Platforms:** Windows and Linux for sure? macOS too? (OpenGL is deprecated on macOS but still works.)
- [ ] **Q4 License:** open source or closed? It affects which dependencies are acceptable and whether a community component repository makes sense.
- [ ] **Q5 Datasheet extraction:** should the datasheet tool also read tables straight from PDFs, possibly with AI assistance, or stay with manual entry and curve digitizing?
