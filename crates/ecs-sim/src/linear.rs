//! Linear solvers behind one trait (spec 4.4).
//!
//! Only a dense LU with partial pivoting exists so far. It is the faster choice
//! for small circuits; the sparse `faer` back end and the measured switch-over
//! size are still to do (see the spec's implementation status).

/// Solves `A x = b` for a square system assembled each Newton iteration.
pub trait LinearSolver: Send {
    /// Resize for `n` unknowns and clear the matrix.
    fn reset(&mut self, n: usize);
    /// Add `v` at row `r`, column `c`.
    fn add(&mut self, r: usize, c: usize, v: f64);
    /// Factor the assembled matrix. Returns false when it is singular.
    /// Implementations may reuse the previous factors when the matrix is unchanged.
    fn factor(&mut self) -> bool;
    /// Solve in place with the factors from [`LinearSolver::factor`].
    fn solve(&self, b: &mut [f64]);
    /// Number of numeric factorizations performed (for tests and statistics).
    fn factorizations(&self) -> u64;
    /// `out = A x` with the assembled (not factored) matrix.
    fn mul(&self, x: &[f64], out: &mut [f64]);
}

/// Dense LU with partial pivoting and factor reuse when the matrix repeats.
#[derive(Debug, Clone, Default)]
pub struct DenseLu {
    n: usize,
    a: Vec<f64>,
    lu: Vec<f64>,
    last: Vec<f64>,
    perm: Vec<usize>,
    valid: bool,
    count: u64,
}

impl DenseLu {
    /// An empty solver.
    pub fn new() -> Self {
        Self::default()
    }
}

impl LinearSolver for DenseLu {
    fn reset(&mut self, n: usize) {
        if n != self.n {
            self.n = n;
            self.valid = false;
            self.last.clear();
        }
        self.a.clear();
        self.a.resize(n * n, 0.0);
    }

    fn add(&mut self, r: usize, c: usize, v: f64) {
        self.a[r * self.n + c] += v;
    }

    fn factor(&mut self) -> bool {
        let n = self.n;
        if self.valid && self.last == self.a {
            return true;
        }
        self.count += 1;
        self.lu.clone_from(&self.a);
        self.perm = (0..n).collect();
        let lu = &mut self.lu;
        for k in 0..n {
            let mut p = k;
            let mut best = lu[k * n + k].abs();
            for r in k + 1..n {
                let v = lu[r * n + k].abs();
                if v > best {
                    best = v;
                    p = r;
                }
            }
            if best < 1e-300 {
                self.valid = false;
                return false;
            }
            if p != k {
                for c in 0..n {
                    lu.swap(k * n + c, p * n + c);
                }
                self.perm.swap(k, p);
            }
            let pivot = lu[k * n + k];
            for r in k + 1..n {
                let f = lu[r * n + k] / pivot;
                if f == 0.0 {
                    continue;
                }
                lu[r * n + k] = f;
                for c in k + 1..n {
                    lu[r * n + c] -= f * lu[k * n + c];
                }
            }
        }
        self.last.clone_from(&self.a);
        self.valid = true;
        true
    }

    fn solve(&self, b: &mut [f64]) {
        let n = self.n;
        let lu = &self.lu;
        let mut y: Vec<f64> = self.perm.iter().map(|&p| b[p]).collect();
        for r in 0..n {
            let mut s = y[r];
            for c in 0..r {
                s -= lu[r * n + c] * y[c];
            }
            y[r] = s;
        }
        for r in (0..n).rev() {
            let mut s = y[r];
            for c in r + 1..n {
                s -= lu[r * n + c] * y[c];
            }
            y[r] = s / lu[r * n + r];
        }
        b.copy_from_slice(&y);
    }

    fn factorizations(&self) -> u64 {
        self.count
    }

    fn mul(&self, x: &[f64], out: &mut [f64]) {
        let n = self.n;
        for (r, o) in out.iter_mut().enumerate().take(n) {
            *o = self.a[r * n..(r + 1) * n]
                .iter()
                .zip(x)
                .map(|(a, b)| a * b)
                .sum();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solves_with_pivoting_and_reuses_factors() {
        let mut s = DenseLu::new();
        for _ in 0..2 {
            s.reset(3);
            for (r, c, v) in [
                (0, 1, 2.0),
                (0, 2, 1.0),
                (1, 0, 1.0),
                (1, 1, 1.0),
                (2, 0, 3.0),
                (2, 2, 2.0),
            ] {
                s.add(r, c, v);
            }
            assert!(s.factor());
            let mut b = [3.0, 3.0, 8.0];
            s.solve(&mut b);
            // x = (2, 1, 1)
            for (got, want) in b.iter().zip([2.0, 1.0, 1.0]) {
                assert!((got - want).abs() < 1e-12);
            }
        }
        assert_eq!(s.factorizations(), 1);
    }

    #[test]
    fn reports_singular() {
        let mut s = DenseLu::new();
        s.reset(2);
        s.add(0, 0, 1.0);
        s.add(1, 0, 1.0);
        assert!(!s.factor());
    }
}
