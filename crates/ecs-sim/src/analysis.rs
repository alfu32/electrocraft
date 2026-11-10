//! Analysis-mode commands (spec 4.6): operating point, DC sweep, transient
//! with step control, AC small-signal, parameter step, and FFT.

use super::*;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// One named result vector. AC results are complex (`im` present).
#[derive(Debug, Clone, PartialEq)]
pub struct Trace {
    pub name: String,
    pub unit: String,
    pub re: Vec<f64>,
    pub im: Option<Vec<f64>>,
}

impl Trace {
    /// Magnitude at each point.
    pub fn magnitude(&self) -> Vec<f64> {
        match &self.im {
            Some(im) => self.re.iter().zip(im).map(|(r, i)| r.hypot(*i)).collect(),
            None => self.re.iter().map(|v| v.abs()).collect(),
        }
    }
    /// Phase in degrees (0 for real traces).
    pub fn phase_deg(&self) -> Vec<f64> {
        match &self.im {
            Some(im) => self
                .re
                .iter()
                .zip(im)
                .map(|(r, i)| i.atan2(*r).to_degrees())
                .collect(),
            None => vec![0.0; self.re.len()],
        }
    }
}

/// Which analysis produced a result.
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Op,
    DcSweep { inst: String, param: String },
    Transient,
    Ac,
}

/// A result set: an axis and traces over it (spec 10 `.ecsr` content).
#[derive(Debug, Clone, PartialEq)]
pub struct ResultSet {
    pub kind: Kind,
    /// Axis name: `time`, `frequency` or the swept parameter.
    pub axis_name: String,
    pub axis_unit: String,
    pub axis: Vec<f64>,
    pub traces: Vec<Trace>,
    /// Free-form label (a parameter-step value).
    pub label: String,
}

impl ResultSet {
    /// CSV with one column per trace (AC: magnitude and phase columns).
    pub fn to_csv(&self) -> String {
        let mut head = vec![self.axis_name.clone()];
        for t in &self.traces {
            if t.im.is_some() {
                head.push(format!("|{}|", t.name));
                head.push(format!("phase({})", t.name));
            } else {
                head.push(t.name.clone());
            }
        }
        let mut out = head.join(",") + "\n";
        let cols: Vec<(Vec<f64>, Option<Vec<f64>>)> = self
            .traces
            .iter()
            .map(|t| {
                if t.im.is_some() {
                    (t.magnitude(), Some(t.phase_deg()))
                } else {
                    (t.re.clone(), None)
                }
            })
            .collect();
        for (k, x) in self.axis.iter().enumerate() {
            let mut row = vec![format!("{x:e}")];
            for (a, b) in &cols {
                row.push(format!("{:e}", a[k]));
                if let Some(b) = b {
                    row.push(format!("{:e}", b[k]));
                }
            }
            out += &row.join(",");
            out.push('\n');
        }
        out
    }
}

/// Progress and cancellation shared with the UI.
#[derive(Debug, Default)]
pub struct Progress {
    /// Per mille.
    pub done: AtomicU32,
    pub cancel: AtomicBool,
}

impl Progress {
    fn set(&self, f: f64) {
        self.done
            .store((f.clamp(0.0, 1.0) * 1000.0) as u32, Ordering::Relaxed);
    }
    fn cancelled(&self) -> Result<(), SimError> {
        if self.cancel.load(Ordering::Relaxed) {
            Err(SimError::Component {
                instance: "analysis".into(),
                message: "cancelled".into(),
            })
        } else {
            Ok(())
        }
    }
}

fn unit_of(name: &str) -> &'static str {
    if name.starts_with("I(") { "A" } else { "V" }
}

fn traces_from(names: &[String], rows: &[Vec<f64>]) -> Vec<Trace> {
    names
        .iter()
        .enumerate()
        .map(|(k, n)| Trace {
            name: n.clone(),
            unit: unit_of(n).into(),
            re: rows.iter().map(|r| r[k]).collect(),
            im: None,
        })
        .collect()
}

/// DC operating point as a one-point result.
pub fn op(netlist: &Netlist, options: Options) -> Result<(ResultSet, OpMethod), SimError> {
    let mut c = Circuit::compile_with(netlist, options)?;
    let m = c.reset()?;
    let rs = ResultSet {
        kind: Kind::Op,
        axis_name: "point".into(),
        axis_unit: String::new(),
        axis: vec![0.0],
        traces: traces_from(c.unknown_names(), &[c.solution().to_vec()]),
        label: format!("{m:?}"),
    };
    Ok((rs, m))
}

/// Sweep one instance parameter and solve the operating point at each value.
pub fn dc_sweep(
    netlist: &Netlist,
    inst: &str,
    param: &str,
    values: &[f64],
    options: Options,
    progress: &Progress,
) -> Result<ResultSet, SimError> {
    let mut rows = Vec::new();
    let mut names = Vec::new();
    let mut circuit: Option<Circuit> = None;
    for (k, &v) in values.iter().enumerate() {
        progress.cancelled()?;
        // Live parameters change in place (warm start); others recompile.
        let reuse = circuit
            .as_mut()
            .and_then(|c| c.instance_index(inst).map(|i| (c, i)))
            .is_some_and(|(c, i)| c.set_param(i, param, v));
        if !reuse {
            let mut n = netlist.clone();
            let found = n
                .instances
                .iter_mut()
                .find(|i| i.name == inst)
                .ok_or_else(|| SimError::Component {
                    instance: inst.into(),
                    message: "not in the circuit".into(),
                })?;
            found.params.insert(param.into(), v);
            let mut c = Circuit::compile_with(&n, options)?;
            if let Some(prev) = &circuit
                && prev.n == c.n
            {
                c.x.clone_from(&prev.x);
            }
            circuit = Some(c);
        }
        let c = circuit.as_mut().unwrap();
        c.dc_op()?;
        names = c.unknown_names().to_vec();
        rows.push(c.solution().to_vec());
        progress.set((k + 1) as f64 / values.len() as f64);
    }
    Ok(ResultSet {
        kind: Kind::DcSweep {
            inst: inst.into(),
            param: param.into(),
        },
        axis_name: format!("{inst}.{param}"),
        axis_unit: String::new(),
        axis: values.to_vec(),
        traces: traces_from(&names, &rows),
        label: String::new(),
    })
}

/// Transient with error-controlled step size (spec 4.6): the step adapts so
/// the difference between the solution and a linear predictor stays near
/// `reltol`. Steps land on digital events; `hmax` caps the step.
pub fn transient(
    netlist: &Netlist,
    stop: f64,
    hmax: f64,
    options: Options,
    method: Method,
    progress: &Progress,
) -> Result<ResultSet, SimError> {
    let mut c = Circuit::compile_with(netlist, options)?;
    c.reset()?;
    let mut axis = vec![0.0];
    let mut rows = vec![c.x.clone()];
    let mut h = (hmax / 50.0).max(stop * 1e-9);
    let hmin = stop * 1e-12;
    let mut prev: Option<(f64, Vec<f64>)> = None;
    let scale_v = options.vntol.max(1e-6) * 100.0;
    while c.time < stop * (1.0 - 1e-12) {
        progress.cancelled()?;
        let take = h.min(stop - c.time).min(hmax);
        let t0 = c.time;
        let x0 = c.x.clone();
        match c.step(take, method) {
            Ok(_) => {
                let taken = c.time - t0;
                // Error estimate against linear extrapolation from the last two points.
                let ratio = match &prev {
                    Some((tp, xp)) if t0 > *tp => {
                        let f = taken / (t0 - tp);
                        let mut worst: f64 = 0.0;
                        for k in 0..c.n {
                            let pred = x0[k] + (x0[k] - xp[k]) * f;
                            let tol = options.reltol * c.x[k].abs().max(x0[k].abs())
                                + if k < c.n_nodes - 1 { scale_v } else { 1e-9 };
                            worst = worst.max((c.x[k] - pred).abs() / tol);
                        }
                        worst
                    }
                    _ => 1.0,
                };
                prev = Some((t0, x0));
                axis.push(c.time);
                rows.push(c.x.clone());
                let grow = if ratio <= 1e-9 {
                    2.0
                } else {
                    (0.9 / ratio.sqrt()).clamp(0.25, 2.0)
                };
                h = (taken * grow).clamp(hmin, hmax);
                progress.set(c.time / stop);
            }
            Err(e) => {
                h /= 4.0;
                c.after_breakpoint = true;
                if h < hmin {
                    return Err(e);
                }
            }
        }
    }
    Ok(ResultSet {
        kind: Kind::Transient,
        axis_name: "time".into(),
        axis_unit: "s".into(),
        axis,
        traces: traces_from(c.unknown_names(), &rows),
        label: String::new(),
    })
}

/// Logarithmic frequency points, `per_decade` per decade.
pub fn log_points(start: f64, stop: f64, per_decade: usize) -> Vec<f64> {
    let decades = (stop / start).log10();
    let n = (decades * per_decade as f64).ceil().max(1.0) as usize;
    (0..=n)
        .map(|k| start * 10f64.powf(decades * k as f64 / n as f64))
        .collect()
}

/// AC small-signal analysis (spec 4.6): linearize at the operating point,
/// G + jωC, excited by every source's `AC` magnitude.
pub fn ac(
    netlist: &Netlist,
    freqs: &[f64],
    options: Options,
    progress: &Progress,
) -> Result<ResultSet, SimError> {
    let mut c = Circuit::compile_with(netlist, options)?;
    c.reset()?;
    let n = c.n;
    let (g, cm, b) = c.linearize(netlist);
    let mut re = vec![Vec::with_capacity(freqs.len()); n];
    let mut im = vec![Vec::with_capacity(freqs.len()); n];
    for (k, &f) in freqs.iter().enumerate() {
        progress.cancelled()?;
        let w = 2.0 * std::f64::consts::PI * f;
        let a: Vec<(f64, f64)> = g.iter().zip(&cm).map(|(gv, cv)| (*gv, w * cv)).collect();
        let x =
            complex_solve(n, a, b.iter().map(|v| (*v, 0.0)).collect()).ok_or(SimError::Singular)?;
        for (j, (r, i)) in x.into_iter().enumerate() {
            re[j].push(r);
            im[j].push(i);
        }
        progress.set((k + 1) as f64 / freqs.len() as f64);
    }
    let traces = c
        .unknown_names()
        .iter()
        .enumerate()
        .map(|(j, name)| Trace {
            name: name.clone(),
            unit: unit_of(name).into(),
            re: re[j].clone(),
            im: Some(im[j].clone()),
        })
        .collect();
    Ok(ResultSet {
        kind: Kind::Ac,
        axis_name: "frequency".into(),
        axis_unit: "Hz".into(),
        axis: freqs.to_vec(),
        traces,
        label: String::new(),
    })
}

/// Dense complex LU with partial pivoting; `None` if singular.
fn complex_solve(
    n: usize,
    mut a: Vec<(f64, f64)>,
    mut b: Vec<(f64, f64)>,
) -> Option<Vec<(f64, f64)>> {
    let mul = |x: (f64, f64), y: (f64, f64)| (x.0 * y.0 - x.1 * y.1, x.0 * y.1 + x.1 * y.0);
    let div = |x: (f64, f64), y: (f64, f64)| {
        let d = y.0 * y.0 + y.1 * y.1;
        ((x.0 * y.0 + x.1 * y.1) / d, (x.1 * y.0 - x.0 * y.1) / d)
    };
    for k in 0..n {
        let p = (k..n).max_by(|&i, &j| {
            let (u, v) = (a[i * n + k], a[j * n + k]);
            u.0.hypot(u.1).total_cmp(&v.0.hypot(v.1))
        })?;
        let pv = a[p * n + k];
        if pv.0.hypot(pv.1) < 1e-300 {
            return None;
        }
        if p != k {
            for col in 0..n {
                a.swap(k * n + col, p * n + col);
            }
            b.swap(k, p);
        }
        for r in k + 1..n {
            let f = div(a[r * n + k], a[k * n + k]);
            if f == (0.0, 0.0) {
                continue;
            }
            for col in k..n {
                let t = mul(f, a[k * n + col]);
                a[r * n + col].0 -= t.0;
                a[r * n + col].1 -= t.1;
            }
            let t = mul(f, b[k]);
            b[r].0 -= t.0;
            b[r].1 -= t.1;
        }
    }
    let mut x = vec![(0.0, 0.0); n];
    for r in (0..n).rev() {
        let mut s = b[r];
        for col in r + 1..n {
            let t = mul(a[r * n + col], x[col]);
            s.0 -= t.0;
            s.1 -= t.1;
        }
        x[r] = div(s, a[r * n + r]);
    }
    Some(x)
}

impl Circuit {
    /// Small-signal matrices at the present solution: conductances G, charge
    /// Jacobian C, and the AC excitation vector from sources' `AC` parameter.
    fn linearize(&mut self, netlist: &Netlist) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
        let n = self.n;
        let mut g = vec![0.0; n * n];
        let mut cm = vec![0.0; n * n];
        let mut b = vec![0.0; n];
        for i in 0..self.n_nodes - 1 {
            g[i * n + i] += self.options.gmin;
        }
        let flat = flatten(netlist).unwrap_or_else(|_| netlist.clone());
        let ws = &mut self.ws;
        for c in &mut self.insts {
            for (l, &gi) in c.map.iter().enumerate() {
                c.xl[l] = self.x[gi];
            }
            c.live[LIVE_T] = self.time;
            c.tape.eval(&c.xl, &c.live, ws);
            let ac_mag = flat
                .instances
                .iter()
                .find(|i| i.name == c.name)
                .and_then(|i| {
                    i.params.get("AC").copied().or_else(|| {
                        i.def
                            .params
                            .iter()
                            .find(|p| p.name == "AC")
                            .map(|p| p.default)
                    })
                })
                .unwrap_or(0.0);
            let mut excited = false;
            for br in &c.branches {
                match br {
                    CBranch::Current {
                        a,
                        b: bb,
                        out_i,
                        out_q,
                        source,
                        ..
                    } => {
                        let gi = c.tape.grad(ws, *out_i);
                        for (row, sign) in [(a, 1.0), (bb, -1.0)] {
                            let Some(r) = row else { continue };
                            let r = c.map[*r];
                            for (k, d) in gi.iter().enumerate() {
                                g[r * n + c.map[k]] += sign * d;
                            }
                            if let Some(oq) = out_q {
                                for (k, d) in c.tape.grad(ws, *oq).iter().enumerate() {
                                    cm[r * n + c.map[k]] += sign * d;
                                }
                            }
                            if *source && !excited && ac_mag != 0.0 {
                                b[r] -= sign * ac_mag;
                            }
                        }
                        if *source && ac_mag != 0.0 {
                            excited = true;
                        }
                    }
                    CBranch::Voltage {
                        a,
                        b: bb,
                        j,
                        out_v,
                        out_f,
                        source,
                        ..
                    } => {
                        let jg = c.map[*j];
                        if let Some(a) = a {
                            g[c.map[*a] * n + jg] += 1.0;
                            g[jg * n + c.map[*a]] += 1.0;
                        }
                        if let Some(bb) = bb {
                            g[c.map[*bb] * n + jg] -= 1.0;
                            g[jg * n + c.map[*bb]] -= 1.0;
                        }
                        for (k, d) in c.tape.grad(ws, *out_v).iter().enumerate() {
                            g[jg * n + c.map[k]] -= d;
                        }
                        if let Some(of) = out_f {
                            for (k, d) in c.tape.grad(ws, *of).iter().enumerate() {
                                cm[jg * n + c.map[k]] -= d;
                            }
                        }
                        if *source && !excited && ac_mag != 0.0 {
                            b[jg] += ac_mag;
                            excited = true;
                        }
                    }
                }
            }
        }
        (g, cm, b)
    }
}

/// Run `analysis` once per parameter value, in parallel threads.
pub fn param_step<F>(
    netlist: &Netlist,
    inst: &str,
    param: &str,
    values: &[f64],
    analysis: F,
) -> Vec<Result<ResultSet, SimError>>
where
    F: Fn(&Netlist) -> Result<ResultSet, SimError> + Sync,
{
    std::thread::scope(|scope| {
        let handles: Vec<_> = values
            .iter()
            .map(|&v| {
                let mut n = netlist.clone();
                if let Some(i) = n.instances.iter_mut().find(|i| i.name == inst) {
                    i.params.insert(param.into(), v);
                }
                let analysis = &analysis;
                scope.spawn(move || {
                    analysis(&n).map(|mut r| {
                        r.label = format!("{inst}.{param} = {v:e}");
                        r
                    })
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or(Err(SimError::Singular)))
            .collect()
    })
}

/// Magnitude spectrum of a non-uniformly sampled signal: resampled to a
/// power of two, Hann-windowed, radix-2 FFT. Returns (frequencies, magnitudes).
pub fn fft(times: &[f64], values: &[f64]) -> (Vec<f64>, Vec<f64>) {
    if times.len() < 4 {
        return (Vec::new(), Vec::new());
    }
    let n = times.len().next_power_of_two().clamp(64, 1 << 16);
    let (t0, t1) = (times[0], times[times.len() - 1]);
    let dt = (t1 - t0) / n as f64;
    let mut re = Vec::with_capacity(n);
    let mut j = 0;
    for k in 0..n {
        let t = t0 + k as f64 * dt;
        while j + 1 < times.len() - 1 && times[j + 1] < t {
            j += 1;
        }
        let (ta, tb) = (times[j], times[j + 1]);
        let f = if tb > ta {
            ((t - ta) / (tb - ta)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let v = values[j] + (values[j + 1] - values[j]) * f;
        let w = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * k as f64 / n as f64).cos();
        re.push(v * w);
    }
    let mut im = vec![0.0; n];
    // Bit reversal.
    let bits = n.trailing_zeros();
    for i in 0..n {
        let r = i.reverse_bits() >> (usize::BITS - bits);
        if r > i {
            re.swap(i, r);
            im.swap(i, r);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * std::f64::consts::PI / len as f64;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (s, co) = (ang * k as f64).sin_cos();
                let (a, b) = (start + k, start + k + len / 2);
                let tr = re[b] * co - im[b] * s;
                let ti = re[b] * s + im[b] * co;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len *= 2;
    }
    let half = n / 2;
    let freqs = (0..half).map(|k| k as f64 / (n as f64 * dt)).collect();
    // Hann coherent gain 0.5: scale so a sine of amplitude A shows A.
    let mags = (0..half)
        .map(|k| re[k].hypot(im[k]) * 4.0 / n as f64)
        .collect();
    (freqs, mags)
}
