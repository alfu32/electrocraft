"use strict";
// Electrocraft component API (spec 6.1). Runs before every component script.

// Sandbox: no eval and no Function constructors (plain, async, generator).
(function () {
  for (const f of [function () {}, async function () {}, function* () {}, async function* () {}]) {
    Object.defineProperty(Object.getPrototypeOf(f), "constructor", { value: undefined, writable: false, configurable: false });
  }
})();
globalThis.eval = undefined;
globalThis.Function = undefined;

class Expr {
  constructor(src, loc) {
    this.src = src;
    this.loc = loc;
  }
  toString() {
    return "(" + this.src + ")";
  }
}

// "line:column" of the caller in the component file.
function __loc() {
  const lines = (new Error().stack || "").split("\n");
  for (const l of lines) {
    if (l.includes("electrocraft-prelude.js")) continue;
    const m = l.match(/:(\d+)(?::(\d+))?\)?\s*$/);
    if (m) return m[1] + ":" + (m[2] || "0");
  }
  return "0:0";
}

/** Tagged template: e`Is * (limexp(V(A, K) / (N * vt)) - 1)`. */
function e(strings, ...values) {
  let s = strings[0];
  values.forEach((v, i) => {
    if (v instanceof Expr) s += "(" + v.src + ")";
    else if (typeof v === "number" && Number.isFinite(v)) s += "(" + String(v) + ")";
    else throw new TypeError("e`...`: ${} must be an expression or a finite number");
    s += strings[i + 1];
  });
  return new Expr(s, __loc());
}

/** param(default, unit, description, { live, min, max, values }) */
function param(def, unit, description, opts) {
  return { __param: true, def: def, unit: unit || "", description: description || "", opts: opts || {} };
}
function digital(family) {
  return { __pin: "in", family: family || "" };
}
function digitalOut(family) {
  return { __pin: "out", family: family || "" };
}
function component(spec) {
  return spec;
}

function __expr(x) {
  if (x instanceof Expr) return { src: x.src, loc: x.loc };
  if (typeof x === "number") return { src: String(x), loc: "0:0" };
  if (typeof x === "string") throw new TypeError("write expressions with e`...`, not plain strings: " + x);
  throw new TypeError("expected an e`...` expression");
}

function __describeComponent() {
  const spec = globalThis.__component;
  if (!spec || typeof spec !== "object") throw new Error("the file must `export default component({...})`");
  const out = {
    id: spec.id || "", name: spec.name || spec.id || "", category: spec.category || "User",
    symbol: spec.symbol || "", prefix: spec.prefix || "U", description: spec.description || "",
    version: spec.version || 1, author: spec.author || "",
    pins: [], params: [], states: [], nodes: [], lets: [], branches: [], limits: [], probes: [],
    controls: spec.controls || [], hooks: {},
  };
  if (Array.isArray(spec.pins)) {
    for (const p of spec.pins) out.pins.push({ name: p, kind: "analog", family: "" });
  } else {
    for (const [k, v] of Object.entries(spec.pins || {})) {
      out.pins.push({ name: k, kind: v && v.__pin ? v.__pin : "analog", family: (v && v.family) || "" });
    }
  }
  for (const [k, v] of Object.entries(spec.params || {})) {
    const p = v && v.__param ? v : param(v);
    out.params.push({
      name: k, default: p.def, unit: p.unit, description: p.description, live: !!p.opts.live,
      min: p.opts.min === undefined ? null : p.opts.min, max: p.opts.max === undefined ? null : p.opts.max,
      values: p.opts.values || null,
    });
  }
  for (const [k, v] of Object.entries(spec.states || {})) out.states.push({ name: k, initial: Number(v) });
  if (typeof spec.equations === "function") {
    let n = 0;
    const c = {
      let(name, x) { out.lets.push({ name: name, expr: __expr(x) }); return new Expr(name, __loc()); },
      current(a, b, i, q) { out.branches.push({ kind: "current", a: a, b: b, i: __expr(i), q: q === undefined || q === null ? null : __expr(q) }); },
      voltage(a, b, v, flux, name) {
        n += 1;
        out.branches.push({ kind: "voltage", a: a, b: b, v: __expr(v), flux: flux === undefined || flux === null ? null : __expr(flux), name: name || "B" + n });
      },
      node(name) { out.nodes.push(name); return name; },
      limit(kind, x, opts) {
        if (kind !== "pn") throw new Error("only pn limiting is available");
        opts = opts || {};
        out.limits.push({ expr: __expr(x), vte: opts.vte ? __expr(opts.vte) : null, is: opts.is ? __expr(opts.is) : null });
      },
      probe(name, x, unit) { out.probes.push({ name: name, expr: __expr(x), unit: unit || "" }); },
    };
    spec.equations(c);
  }
  out.hooks = {
    start: typeof spec.onStart === "function", inputs: typeof spec.onInputs === "function",
    timer: typeof spec.onTimer === "function", control: typeof spec.onControl === "function",
  };
  return JSON.stringify(out);
}

function __runHook(which, json, tag) {
  const d = JSON.parse(json);
  const actions = [];
  const ctx = {
    time: d.time,
    state: d.state,
    param: d.params,
    event: d.event,
    level(p) { const l = d.levels[p]; return l === undefined ? "X" : l; },
    rising(p) { return d.levels[p] === 1 && d.previous[p] !== 1; },
    falling(p) { return d.levels[p] === 0 && d.previous[p] !== 0; },
    voltage(p) { return d.voltages[p] || 0; },
    drive(pin, level, delay) { actions.push({ pin: pin, level: level, delay: delay || 0 }); },
    timer(delay, t) { actions.push({ timer: t || 0, delay: delay || 0 }); },
  };
  const s = globalThis.__component;
  if (which === "start") (s.onStart || s.onInputs || function () {}).call(s, ctx);
  else if (which === "inputs" && s.onInputs) s.onInputs(ctx);
  else if (which === "timer" && s.onTimer) s.onTimer(ctx, tag);
  else if (which === "control") {
    if (s.onControl) s.onControl(ctx, d.event);
    else if (d.event && d.event.kind === "state") ctx.state[d.event.name] = d.event.value;
  }
  return JSON.stringify({ state: ctx.state, actions: actions });
}
