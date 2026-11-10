// A diode with junction capacitance (spec 6.2). Shipped as a reference.
export default component({
  id: "user.simple_diode",
  name: "Simple diode",
  category: "Semiconductors",
  symbol: "diode",
  prefix: "D",
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
