// Electrocraft component API (spec 6.1). Place beside your .js components
// for editor completion; Electrocraft itself loads JavaScript only.

/** An expression built with the e`...` tagged template. */
declare class Expr { private readonly __expr: true }
/** Build an equation: e`Is * (limexp(V(A, K) / (N * vt)) - 1)`. ${} takes an Expr or a number. */
declare function e(strings: TemplateStringsArray, ...values: (Expr | number)[]): Expr;

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
  let(name: string, x: Expr | number): Expr;
  /** Current i (+ dq/dt) flowing from a to b through the component. */
  current(a: string, b: string, i: Expr | number, q?: Expr | number): void;
  /** V(a) - V(b) = v + d(flux)/dt; adds the branch current I(name). */
  voltage(a: string, b: string, v: Expr | number, flux?: Expr | number | null, name?: string): void;
  /** Add an internal node. */
  node(name: string): string;
  /** Newton limiting on a junction voltage e`V(a, b)`. */
  limit(kind: "pn", x: Expr, opts?: { vte?: Expr; is?: Expr }): void;
  /** A named quantity shown in the inspector. */
  probe(name: string, x: Expr | number, unit?: string): void;
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
