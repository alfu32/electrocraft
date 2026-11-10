// Electrocraft component API (spec 6.1). Place beside your .js components
// for editor completion; Electrocraft itself loads JavaScript only.

/** An expression built with the e`...` tagged template. */
declare class Expr { private readonly __expr: true }
/** Build an equation: e`Is * (limexp(V(A, K) / (N * vt)) - 1)`. ${} takes an Expr or a number. */
declare function e(strings: TemplateStringsArray, ...values: (Expr | number)[]): Expr;

/**
 * An equation written as a plain function. It is compiled, not run: its
 * parameters name component variables (parameters, states, c.let names, vt,
 * temp, t), usually destructured: ({ Is, N }) => Is * (limexp(V("A", "K") / (N * vt)) - 1).
 * Allowed inside: const/let, if/else, return, ?:, arithmetic, comparisons,
 * &&, ||, !, **, Math.*, the functions below and helper functions declared
 * at the top of the file (inlined). No loops, recursion, objects or strings
 * other than pin and branch names.
 */
type EquationFn = (vars: any, ...more: any[]) => number;
/** What c.current, c.voltage, c.let, c.probe and c.limit accept. */
type Equation = Expr | number | EquationFn;

/** Voltage of a pin or node, or between two (inside equation functions). */
declare function V(a: string, b?: string): number;
/** Current of a named voltage branch (inside equation functions). */
declare function I(branch: string): number;
/** Thermal voltage kT/q at 27 °C (in equations: at the circuit temperature). */
declare const vt: number;
/** Exponential that continues linearly above 80, so junctions cannot overflow. */
declare function limexp(x: number): number;
/** Natural logarithm. */
declare function ln(x: number): number;
/** sqrt(max(x, 0)). */
declare function safe_sqrt(x: number): number;
/** a / b with |b| kept above 1e-30. */
declare function safe_div(a: number, b: number): number;
/** a when cond is non-zero, else b. */
declare function select(cond: number | boolean, a: number, b: number): number;
/** Smooth 0..1 step of x between edge0 and edge1. */
declare function smoothstep(edge0: number, edge1: number, x: number): number;
/** sqrt(x² + eps²), a differentiable |x| (eps defaults to 1e-6). */
declare function smooth_abs(x: number, eps?: number): number;

interface ParamOptions { live?: boolean; min?: number; max?: number; values?: string[] }
interface Param { readonly __param: true }
declare function param(def: number, unit?: string, description?: string, opts?: ParamOptions): Param;

type Family = "cmos5" | "cmos33" | "ttl" | string;
interface DigitalPin { readonly __pin: "in" | "out" }
declare function digital(family?: Family): DigitalPin;
declare function digitalOut(family?: Family): DigitalPin;

type Level = 0 | 1 | "X";

interface Equations {
  /** Name a subexpression for later templates. */
  let(name: string, x: Equation): Expr;
  /** Current i (+ dq/dt) flowing from a to b through the component. */
  current(a: string, b: string, i: Equation, q?: Equation): void;
  /** V(a) - V(b) = v + d(flux)/dt; adds the branch current I(name). */
  voltage(a: string, b: string, v: Equation, flux?: Equation | null, name?: string): void;
  /** Add an internal node. */
  node(name: string): string;
  /** Newton limiting on a junction voltage e`V(a, b)`. */
  limit(kind: "pn", x: Expr | (() => number), opts?: { vte?: Equation; is?: Equation }): void;
  /** A named quantity shown in the inspector. */
  probe(name: string, x: Equation, unit?: string): void;
}

interface Ctx {
  readonly time: number;
  state: Record<string, number | Level>;
  readonly param: Record<string, number>;
  level(pin: string): Level;
  rising(pin: string): boolean;
  falling(pin: string): boolean;
  voltage(pin: string): number;
  drive(pin: string, level: Level, delay?: number): void;
  timer(delay: number, tag?: number): void;
}

interface ControlEvent { kind: "state" | "param"; name: string; value: number }

interface ComponentSpec {
  id: string;
  name?: string;
  category?: string;
  symbol?: string;
  prefix?: string;
  description?: string;
  version?: number;
  author?: string;
  pins: string[] | Record<string, DigitalPin | "analog">;
  params?: Record<string, Param | number>;
  states?: Record<string, number>;
  controls?: { kind: "toggle" | "momentary" | "slider"; state?: string; param?: string; label: string }[];
  equations?(c: Equations): void;
  onStart?(ctx: Ctx): void;
  onInputs?(ctx: Ctx): void;
  onTimer?(ctx: Ctx, tag: number): void;
  onControl?(ctx: Ctx, event: ControlEvent): void;
}
declare function component(spec: ComponentSpec): ComponentSpec;
