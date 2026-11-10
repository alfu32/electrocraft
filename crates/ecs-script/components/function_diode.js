// The diode of simple_diode.js, written with plain functions instead of
// e`...` templates (spec 6.2). Shipped as a reference.
//
// Functions passed to c.current, c.voltage, c.let, c.probe and c.limit are
// not run: Electrocraft compiles their source into an equation, which it
// differentiates and evaluates natively. Their parameters name component
// variables (parameters, states, c.let names, vt, temp, t); V("A", "K") and
// I("name") read the circuit. Inside, use const/let, if/else, return, the
// conditional operator, Math.*, the expression functions (limexp, select,
// smoothstep...) and helper functions declared in this file, which are
// inlined. Loops, recursion and objects are not available.

/** Shockley junction current; limexp keeps large forward bias finite. */
function shockley(vd, Is, N) {
  return Is * (limexp(vd / (N * vt)) - 1);
}

/** Depletion charge, continued linearly above Fc·Vj so it never blows up. */
function depletionCharge(vd, Cj, Vj, M) {
  const fc = 0.5 * Vj;
  const v = Math.min(vd, fc);
  const q = (Cj * Vj) / (1 - M) * (1 - (1 - v / Vj) ** (1 - M));
  if (vd <= fc) {
    return q;
  }
  const slope = Cj * (1 - fc / Vj) ** -M;
  return q + slope * (vd - fc);
}

/**
 * Reverse breakdown: a mirrored exponential that reaches IBV at -BV. It is
 * smooth everywhere; a hard switch at -BV would leave Newton without a slope.
 */
function breakdown(vd, IBV, BV) {
  return -IBV * limexp(-(vd + BV) / vt);
}

export default component({
  id: "user.function_diode",
  name: "Diode (functions)",
  category: "Semiconductors",
  symbol: "diode",
  prefix: "D",
  description: "Junction diode written with plain JavaScript functions.",
  pins: ["A", "K"],
  params: {
    Is: param(1e-14, "A", "Saturation current"),
    N: param(1.0, "", "Emission coefficient"),
    Cj: param(2e-12, "F", "Zero-bias junction capacitance"),
    Vj: param(0.7, "V", "Junction potential"),
    M: param(0.5, "", "Grading coefficient"),
    BV: param(100, "V", "Reverse breakdown voltage"),
    IBV: param(1e-10, "A", "Current at breakdown"),
  },
  equations(c) {
    // c.let names a value; later functions can take it as a parameter.
    c.let("Id", ({ Is, N, IBV, BV }) => shockley(V("A", "K"), Is, N) + breakdown(V("A", "K"), IBV, BV));
    c.current("A", "K", ({ Id }) => Id, ({ Cj, Vj, M }) => depletionCharge(V("A", "K"), Cj, Vj, M));
    c.limit("pn", () => V("A", "K"));
    c.probe("P", ({ Id }) => V("A", "K") * Id, "W");
  },
});
