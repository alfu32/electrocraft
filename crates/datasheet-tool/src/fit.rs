//! Fitting model parameters to datasheet points (spec 8.2). Every point is
//! evaluated on a test bench simulated with `ecs-sim` and the built-in model.

use ecs_sim::{Circuit, Netlist};
use std::sync::Arc;

/// Model families the tool can fit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Diode,
    Led,
    Zener,
    Npn,
    Nmos,
}

impl Family {
    pub const ALL: [Family; 5] = [
        Family::Diode,
        Family::Led,
        Family::Zener,
        Family::Npn,
        Family::Nmos,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Family::Diode => "Diode",
            Family::Led => "LED",
            Family::Zener => "Zener diode",
            Family::Npn => "NPN transistor",
            Family::Nmos => "N-MOSFET",
        }
    }
    fn builtin(self) -> &'static str {
        match self {
            Family::Diode => "core.diode",
            Family::Led => "core.led",
            Family::Zener => "core.zener",
            Family::Npn => "core.npn",
            Family::Nmos => "core.nmos",
        }
    }
    /// Point series: (title, x label, x unit, y label, y unit, log x).
    pub fn series(
        self,
    ) -> &'static [(
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        bool,
    )] {
        match self {
            Family::Diode | Family::Led => {
                &[("Forward voltage vs current", "If", "A", "Vf", "V", true)]
            }
            Family::Zener => &[
                ("Forward voltage vs current", "If", "A", "Vf", "V", true),
                ("Zener voltage vs current", "Iz", "A", "Vz", "V", true),
            ],
            Family::Npn => &[
                (
                    "DC current gain vs collector current",
                    "Ic",
                    "A",
                    "hFE",
                    "",
                    true,
                ),
                (
                    "Base-emitter voltage vs collector current",
                    "Ic",
                    "A",
                    "Vbe",
                    "V",
                    true,
                ),
            ],
            Family::Nmos => &[(
                "Drain current vs gate voltage",
                "Vgs",
                "V",
                "Id",
                "A",
                false,
            )],
        }
    }
    /// Parameters fitted, with whether they are fitted in log space.
    fn fitted(self) -> &'static [(&'static str, bool)] {
        match self {
            Family::Diode | Family::Led => &[("Is", true), ("N", false)],
            Family::Zener => &[("Is", true), ("N", false), ("BV", false)],
            Family::Npn => &[("Is", true), ("Nf", false), ("Bf", true)],
            Family::Nmos => &[("Kp", true), ("Vto", false)],
        }
    }
}

/// Datasheet points and bench conditions.
#[derive(Debug, Clone)]
pub struct Dataset {
    pub family: Family,
    /// One list of [x, y] per series of the family.
    pub series: Vec<Vec<[f64; 2]>>,
    /// Vce for transistor gain/Vbe points, Vds for MOSFET points.
    pub bias: f64,
}

impl Dataset {
    pub fn new(family: Family) -> Self {
        Self {
            family,
            series: vec![Vec::new(); family.series().len()],
            bias: if family == Family::Nmos { 10.0 } else { 5.0 },
        }
    }
}

/// A fit result.
#[derive(Debug, Clone)]
pub struct Fit {
    pub params: Vec<(String, f64)>,
    /// Relative error of each point, per series.
    pub errors: Vec<Vec<f64>>,
    pub max_error: f64,
    pub iterations: usize,
}

fn lib(id: &str) -> Arc<ecs_model::ComponentDef> {
    ecs_lib::library().get(id).cloned().expect("built-in")
}

fn op(n: &Netlist) -> Option<Circuit> {
    let mut c = Circuit::compile(n).ok()?;
    c.dc_op().ok()?;
    Some(c)
}

/// The model's value at a point of `series`, with parameters `p`.
pub fn model_value(
    family: Family,
    p: &[(String, f64)],
    series: usize,
    x: f64,
    bias: f64,
) -> Option<f64> {
    let dev = lib(family.builtin());
    let params: Vec<(&str, f64)> = p.iter().map(|(k, v)| (k.as_str(), *v)).collect();
    let mut n = Netlist::new();
    match (family, series) {
        (Family::Diode | Family::Led | Family::Zener, 0) => {
            n.add("I1", lib("core.idc"), &["a", "0"], &[("I", x)]);
            n.add("D1", dev, &["a", "0"], &params);
            op(&n)?.voltage("a")
        }
        (Family::Zener, _) => {
            // Reverse current into the cathode: the node settles near BV.
            n.add("I1", lib("core.idc"), &["k", "0"], &[("I", x)]);
            n.add("D1", dev, &["0", "k"], &params);
            op(&n)?.voltage("k")
        }
        (Family::Npn, 0) => {
            // hFE at Ic: drive Ib = Ic / hFE guess and report Ic / Ib at the solution.
            let bf = p.iter().find(|(k, _)| k == "Bf").map_or(100.0, |x| x.1);
            let ib = x / bf;
            n.add("VC", lib("core.vdc"), &["c", "0"], &[("V", bias)]);
            n.add("IB", lib("core.idc"), &["b", "0"], &[("I", ib)]);
            n.add("Q1", dev, &["c", "b", "0"], &params);
            let c = op(&n)?;
            let k = c.instance_index("VC")?;
            let ic = -c.branch_currents(k)[0];
            Some(ic / ib)
        }
        (Family::Npn, _) => {
            // Vbe at Ic: bisect the base voltage so the collector carries Ic.
            let (mut lo, mut hi) = (0.0, 1.5);
            for _ in 0..40 {
                let vb = 0.5 * (lo + hi);
                let mut n = Netlist::new();
                n.add("VC", lib("core.vdc"), &["c", "0"], &[("V", bias)]);
                n.add("VB", lib("core.vdc"), &["b", "0"], &[("V", vb)]);
                n.add("Q1", dev.clone(), &["c", "b", "0"], &params);
                let c = op(&n)?;
                let k = c.instance_index("VC")?;
                if -c.branch_currents(k)[0] > x {
                    hi = vb
                } else {
                    lo = vb
                }
            }
            Some(0.5 * (lo + hi))
        }
        (Family::Nmos, _) => {
            n.add("VD", lib("core.vdc"), &["d", "0"], &[("V", bias)]);
            n.add("VG", lib("core.vdc"), &["g", "0"], &[("V", x)]);
            n.add("M1", dev, &["d", "g", "0"], &params);
            let c = op(&n)?;
            let k = c.instance_index("VD")?;
            Some(-c.branch_currents(k)[0])
        }
        _ => None,
    }
}

const VT: f64 = 0.025_865;

/// Starting values from closed-form estimates.
fn initial(ds: &Dataset) -> Vec<f64> {
    let slope = |pts: &[[f64; 2]], default_n: f64| -> (f64, f64) {
        // Two extreme points: N = ΔV / (vt ln(I2/I1)), Is = I1 / exp(V1 / (N vt)).
        if pts.len() >= 2 {
            let (a, b) = (pts[0], pts[pts.len() - 1]);
            if b[0] > a[0] && b[1] > a[1] {
                let n = ((b[1] - a[1]) / (VT * (b[0] / a[0]).ln())).clamp(0.6, 3.5);
                return (a[0] / (a[1] / (n * VT)).exp(), n);
            }
        }
        (1e-14, default_n)
    };
    let sorted = |s: &Vec<[f64; 2]>| {
        let mut v = s.clone();
        v.sort_by(|a, b| a[0].total_cmp(&b[0]));
        v
    };
    match ds.family {
        Family::Diode | Family::Led => {
            let (is, n) = slope(
                &sorted(&ds.series[0]),
                if ds.family == Family::Led { 2.0 } else { 1.0 },
            );
            vec![is.ln(), n]
        }
        Family::Zener => {
            let (is, n) = slope(&sorted(&ds.series[0]), 1.0);
            let bv = ds.series[1].first().map_or(5.1, |p| p[1]);
            vec![is.ln(), n, bv]
        }
        Family::Npn => {
            let (is, n) = slope(&sorted(&ds.series[1]), 1.0);
            let bf = if ds.series[0].is_empty() {
                100.0
            } else {
                ds.series[0].iter().map(|p| p[1]).sum::<f64>() / ds.series[0].len() as f64
            };
            vec![is.ln(), n, bf.ln()]
        }
        Family::Nmos => {
            // Saturation: sqrt(2 Id / Kp) = Vgs - Vto through two points.
            let s = sorted(&ds.series[0]);
            if s.len() >= 2 {
                let (a, b) = (s[0], s[s.len() - 1]);
                let (ra, rb) = (a[1].sqrt(), b[1].sqrt());
                let k = (rb - ra) / (b[0] - a[0]);
                let vto = a[0] - ra / k;
                return vec![(2.0 * k * k).max(1e-9).ln(), vto];
            }
            vec![(20e-3f64).ln(), 1.5]
        }
    }
}

fn params_of(ds: &Dataset, v: &[f64]) -> Vec<(String, f64)> {
    ds.family
        .fitted()
        .iter()
        .zip(v)
        .map(|((name, log), x)| (name.to_string(), if *log { x.exp() } else { *x }))
        .collect()
}

/// Residuals (relative errors, log for currents) of every point.
fn residuals(ds: &Dataset, v: &[f64]) -> Option<Vec<f64>> {
    let p = params_of(ds, v);
    let mut r = Vec::new();
    for (s, pts) in ds.series.iter().enumerate() {
        for pt in pts {
            let m = model_value(ds.family, &p, s, pt[0], ds.bias)?;
            r.push(if ds.family == Family::Nmos {
                (m.max(1e-30) / pt[1]).ln()
            } else {
                (m - pt[1]) / pt[1]
            });
        }
    }
    Some(r)
}

/// Levenberg-Marquardt fit.
pub fn fit(ds: &Dataset) -> Result<Fit, String> {
    let npts: usize = ds.series.iter().map(Vec::len).sum();
    let np = ds.family.fitted().len();
    if npts < np {
        return Err(format!("{} needs at least {np} points", ds.family.name()));
    }
    let mut v = initial(ds);
    let cost = |r: &[f64]| r.iter().map(|x| x * x).sum::<f64>();
    let mut r =
        residuals(ds, &v).ok_or("the model could not be simulated at the starting values")?;
    let mut c0 = cost(&r);
    let mut lambda = 1e-3;
    let mut iterations = 0;
    for it in 0..200 {
        iterations = it + 1;
        // Numerical Jacobian.
        let mut j = vec![vec![0.0; np]; r.len()];
        for k in 0..np {
            let mut vk = v.clone();
            let h = 1e-6 * v[k].abs().max(1.0);
            vk[k] += h;
            let rk = residuals(ds, &vk).ok_or("simulation failed while fitting")?;
            for (row, (a, b)) in j.iter_mut().zip(rk.iter().zip(&r)) {
                row[k] = (a - b) / h;
            }
        }
        let mut jtj = vec![vec![0.0; np]; np];
        let mut jtr = vec![0.0; np];
        for (row, ri) in j.iter().zip(&r) {
            for a in 0..np {
                jtr[a] += row[a] * ri;
                for b in 0..np {
                    jtj[a][b] += row[a] * row[b];
                }
            }
        }
        let mut improved = false;
        for _ in 0..12 {
            let mut m = jtj.clone();
            for (a, row) in m.iter_mut().enumerate() {
                row[a] += lambda * jtj[a][a].max(1e-12);
            }
            let Some(step) = solve(m, jtr.iter().map(|x| -x).collect()) else {
                break;
            };
            let vn: Vec<f64> = v.iter().zip(&step).map(|(a, d)| a + d).collect();
            if let Some(rn) = residuals(ds, &vn) {
                let cn = cost(&rn);
                if cn < c0 {
                    let gain = (c0 - cn) / c0.max(1e-300);
                    v = vn;
                    r = rn;
                    c0 = cn;
                    lambda = (lambda / 3.0).max(1e-12);
                    improved = gain > 1e-10;
                    break;
                }
            }
            lambda *= 4.0;
        }
        if !improved {
            break;
        }
    }
    let params = params_of(ds, &v);
    // Report relative errors of the datasheet quantity itself.
    let mut errors = Vec::new();
    let mut max_error: f64 = 0.0;
    for (s, pts) in ds.series.iter().enumerate() {
        let e: Vec<f64> = pts
            .iter()
            .map(|pt| {
                model_value(ds.family, &params, s, pt[0], ds.bias)
                    .map_or(f64::INFINITY, |m| (m - pt[1]).abs() / pt[1].abs())
            })
            .collect();
        max_error = e.iter().fold(max_error, |a, b| a.max(*b));
        errors.push(e);
    }
    Ok(Fit {
        params,
        errors,
        max_error,
        iterations,
    })
}

fn solve(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let n = b.len();
    for k in 0..n {
        let p = (k..n).max_by(|&i, &j| a[i][k].abs().total_cmp(&a[j][k].abs()))?;
        if a[p][k].abs() < 1e-300 {
            return None;
        }
        a.swap(k, p);
        b.swap(k, p);
        let pivot = a[k].clone();
        for i in k + 1..n {
            let f = a[i][k] / pivot[k];
            for (x, y) in a[i][k..].iter_mut().zip(&pivot[k..]) {
                *x -= f * y;
            }
            b[i] -= f * b[k];
        }
    }
    let mut x = vec![0.0; n];
    for i in (0..n).rev() {
        x[i] = (b[i] - (i + 1..n).map(|j| a[i][j] * x[j]).sum::<f64>()) / a[i][i];
    }
    Some(x)
}

/// A JavaScript component with the fitted parameters (spec 6 API).
pub fn export_js(family: Family, name: &str, fit: &Fit) -> String {
    let id: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    let p = |k: &str| fit.params.iter().find(|x| x.0 == k).map(|x| x.1);
    let def = |k: &str, d: f64| p(k).unwrap_or(d);
    let head = format!(
        "// {name}: {} fitted by electrocraft-datasheet (max error {:.2} %).\n",
        family.name(),
        fit.max_error * 100.0
    );
    match family {
        Family::Diode | Family::Led | Family::Zener => {
            let (bv, ibv) = if family == Family::Zener {
                (def("BV", 5.1), 1e-3)
            } else {
                (100.0, 1e-10)
            };
            format!(
                "{head}export default component({{\n  id: \"ds.{id}\",\n  name: \"{name}\",\n  category: \"Datasheet\",\n  symbol: \"{sym}\",\n  prefix: \"D\",\n  pins: [\"A\", \"K\"],\n  params: {{\n    Is: param({is:e}, \"A\", \"Saturation current\"),\n    N: param({n}, \"\", \"Emission coefficient\"),\n    BV: param({bv}, \"V\", \"Breakdown voltage\"),\n    IBV: param({ibv:e}, \"A\", \"Current at breakdown\"),\n  }},\n  equations(c) {{\n    c.let(\"Id\", e`Is * (limexp(V(A, K) / (N * vt)) - 1) - IBV * limexp(-(V(A, K) + BV) / vt) + 1e-12 * V(A, K)`);\n    c.current(\"A\", \"K\", e`Id`);\n    c.limit(\"pn\", e`V(A, K)`);\n    c.probe(\"I\", e`Id`, \"A\");\n  }},\n}});\n",
                sym = match family {
                    Family::Led => "led",
                    Family::Zener => "zener",
                    _ => "diode",
                },
                is = def("Is", 1e-14),
                n = def("N", 1.0),
            )
        }
        Family::Npn => format!(
            "{head}export default component({{\n  id: \"ds.{id}\",\n  name: \"{name}\",\n  category: \"Datasheet\",\n  symbol: \"npn\",\n  prefix: \"Q\",\n  pins: [\"C\", \"B\", \"E\"],\n  params: {{\n    Is: param({is:e}, \"A\", \"Transport saturation current\"),\n    Nf: param({nf}, \"\", \"Forward emission coefficient\"),\n    Bf: param({bf}, \"\", \"Forward beta\"),\n    Br: param(1, \"\", \"Reverse beta\"),\n  }},\n  equations(c) {{\n    c.let(\"If\", e`Is * (limexp(V(B, E) / (Nf * vt)) - 1)`);\n    c.let(\"Ir\", e`Is * (limexp(V(B, C) / vt) - 1)`);\n    c.current(\"C\", \"E\", e`If - Ir`);\n    c.current(\"B\", \"E\", e`If / Bf + 1e-12 * V(B, E)`);\n    c.current(\"B\", \"C\", e`Ir / Br + 1e-12 * V(B, C)`);\n    c.limit(\"pn\", e`V(B, E)`, {{ vte: e`Nf * vt` }});\n    c.limit(\"pn\", e`V(B, C)`, {{ vte: e`vt` }});\n    c.probe(\"Ic\", e`If - Ir - Ir / Br`, \"A\");\n  }},\n}});\n",
            is = def("Is", 1e-14),
            nf = def("Nf", 1.0),
            bf = def("Bf", 100.0),
        ),
        Family::Nmos => format!(
            "{head}export default component({{\n  id: \"ds.{id}\",\n  name: \"{name}\",\n  category: \"Datasheet\",\n  symbol: \"nmos\",\n  prefix: \"M\",\n  pins: [\"D\", \"G\", \"S\"],\n  params: {{\n    Kp: param({kp:e}, \"A/V²\", \"Transconductance parameter\"),\n    Vto: param({vto}, \"V\", \"Threshold voltage\"),\n  }},\n  equations(c) {{\n    c.let(\"vov\", e`V(G, S) - Vto`);\n    c.current(\"D\", \"S\", e`select(vov <= 0, 0, select(V(D, S) < vov, Kp * (vov * V(D, S) - V(D, S) * V(D, S) / 2), Kp / 2 * vov * vov)) + 1e-12 * V(D, S)`);\n  }},\n}});\n",
            kp = def("Kp", 20e-3),
            vto = def("Vto", 1.5),
        ),
    }
}

/// The equivalent SPICE `.model` line.
pub fn export_spice(family: Family, name: &str, fit: &Fit) -> String {
    let p = |k: &str| fit.params.iter().find(|x| x.0 == k).map_or(0.0, |x| x.1);
    match family {
        Family::Diode | Family::Led => format!(".model {name} D(IS={:e} N={})", p("Is"), p("N")),
        Family::Zener => format!(
            ".model {name} D(IS={:e} N={} BV={} IBV=1m)",
            p("Is"),
            p("N"),
            p("BV")
        ),
        Family::Npn => format!(
            ".model {name} NPN(IS={:e} NF={} BF={})",
            p("Is"),
            p("Nf"),
            p("Bf")
        ),
        Family::Nmos => format!(
            ".model {name} NMOS(LEVEL=1 KP={:e} VTO={})",
            p("Kp"),
            p("Vto")
        ),
    }
}

/// A Markdown fit report.
pub fn report(ds: &Dataset, name: &str, fit: &Fit) -> String {
    let mut out = format!(
        "# {name}: {} fit\n\nIterations: {}. Largest error: {:.2} %.\n\n## Parameters\n\n| Parameter | Value |\n| --- | --- |\n",
        ds.family.name(),
        fit.iterations,
        fit.max_error * 100.0
    );
    for (k, v) in &fit.params {
        out += &format!("| {k} | {} |\n", ecs_units::format_digits(*v, "", 4));
    }
    for ((title, xl, xu, yl, yu, _), (pts, errs)) in ds
        .family
        .series()
        .iter()
        .zip(ds.series.iter().zip(&fit.errors))
    {
        if pts.is_empty() {
            continue;
        }
        out += &format!(
            "\n## {title}\n\n| {xl} | {yl} datasheet | {yl} model | error |\n| --- | --- | --- | --- |\n"
        );
        let s = ds
            .family
            .series()
            .iter()
            .position(|x| x.0 == *title)
            .unwrap_or(0);
        for (pt, e) in pts.iter().zip(errs) {
            let m = model_value(ds.family, &fit.params, s, pt[0], ds.bias).unwrap_or(f64::NAN);
            out += &format!(
                "| {} | {} | {} | {:.2} % |\n",
                ecs_units::format_digits(pt[0], xu, 3),
                ecs_units::format_digits(pt[1], yu, 4),
                ecs_units::format_digits(m, yu, 4),
                e * 100.0
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diode_fitted_from_1n4148_typicals_within_5_percent() {
        // Typical forward characteristic read from the 1N4148 datasheet curve (25 °C).
        let mut ds = Dataset::new(Family::Diode);
        ds.series[0] = vec![
            [0.1e-3, 0.52],
            [0.3e-3, 0.565],
            [1e-3, 0.62],
            [3e-3, 0.665],
            [10e-3, 0.72],
        ];
        let fit = fit(&ds).unwrap();
        assert!(
            fit.max_error < 0.05,
            "{:.3} {:?}",
            fit.max_error,
            fit.params
        );
        let js = export_js(Family::Diode, "1N4148", &fit);
        let def = ecs_script::load_component(&js, "1n4148.js").unwrap();
        assert_eq!(def.id, "ds.1n4148");
    }

    #[test]
    fn npn_fitted_from_2n3904_typicals_within_5_percent() {
        // Typical hFE (Vce = 1 V) and VBE(on) read from the 2N3904 datasheet curves.
        let mut ds = Dataset::new(Family::Npn);
        ds.bias = 1.0;
        ds.series[0] = vec![[1e-3, 110.0], [3e-3, 115.0], [10e-3, 118.0]];
        ds.series[1] = vec![[0.1e-3, 0.56], [1e-3, 0.65], [10e-3, 0.74]];
        let fit = fit(&ds).unwrap();
        assert!(
            fit.max_error < 0.05,
            "{:.3} {:?} {:?}",
            fit.max_error,
            fit.params,
            fit.errors
        );
        let def =
            ecs_script::load_component(&export_js(Family::Npn, "2N3904", &fit), "q.js").unwrap();
        assert_eq!(def.pins.len(), 3);
        assert!(report(&ds, "2N3904", &fit).contains("| Bf |"));
    }

    #[test]
    fn mosfet_threshold_and_kp() {
        let mut ds = Dataset::new(Family::Nmos);
        // Generated from Kp = 50 mA/V², Vto = 2 V in saturation.
        ds.series[0] = [2.5, 3.0, 4.0, 5.0]
            .iter()
            .map(|&v: &f64| [v, 0.05 / 2.0 * (v - 2.0) * (v - 2.0)])
            .collect();
        let fit = fit(&ds).unwrap();
        assert!(fit.max_error < 0.02, "{} {:?}", fit.max_error, fit.params);
    }
}
