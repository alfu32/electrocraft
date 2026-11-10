// A D flip-flop, purely behavioral (spec 6.2). Shipped as a reference.
export default component({
  id: "user.dff",
  name: "D flip-flop (JavaScript)",
  category: "Digital",
  symbol: "dff",
  prefix: "U",
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
