//! Small dense linear algebra: the square matrix and the Cholesky
//! factor/solve pair shared by the puppet deformer here and the planar
//! tracker's bundle adjustment in `lumit-track`.
//!
//! In plain terms: a table of numbers with rows and columns, and the standard
//! way of solving a system of equations built from one. Every accessor is
//! bounds-checked, so an index slip reads a zero rather than panicking
//! (docs/14-ENGINEERING-RULES.md §4), and every loop runs a fixed number of
//! times in a fixed order, so two runs give the same bits.

/// A square dense matrix, row-major, with bounds-checked accessors so that an
/// index slip is a zero rather than a panic (docs/14-ENGINEERING-RULES.md §4).
pub struct Dense {
    n: usize,
    a: Vec<f64>,
}

impl Dense {
    pub fn zero(n: usize) -> Dense {
        Dense {
            n,
            a: vec![0.0; n.saturating_mul(n)],
        }
    }

    pub fn size(&self) -> usize {
        self.n
    }

    pub fn at(&self, r: usize, c: usize) -> f64 {
        if r >= self.n || c >= self.n {
            return 0.0;
        }
        self.a.get(r * self.n + c).copied().unwrap_or(0.0)
    }

    pub fn add(&mut self, r: usize, c: usize, v: f64) {
        if r >= self.n || c >= self.n {
            return;
        }
        let i = r * self.n + c;
        if let Some(x) = self.a.get_mut(i) {
            *x += v;
        }
    }

    pub fn set(&mut self, r: usize, c: usize, v: f64) {
        if r >= self.n || c >= self.n {
            return;
        }
        let i = r * self.n + c;
        if let Some(x) = self.a.get_mut(i) {
            *x = v;
        }
    }
}

/// Cholesky factor `L` with `A = L·Lᵀ`, or `None` when `A` is not positive
/// definite — the caller's signal to fall back or damp harder, not an error.
pub fn cholesky(a: &Dense) -> Option<Dense> {
    let n = a.size();
    let mut l = Dense::zero(n);
    for i in 0..n {
        for j in 0..=i {
            let mut s = a.at(i, j);
            for k in 0..j {
                s -= l.at(i, k) * l.at(j, k);
            }
            if i == j {
                if !s.is_finite() || s <= 0.0 {
                    return None;
                }
                l.set(i, j, s.sqrt());
            } else {
                let d = l.at(j, j);
                if d.abs() < 1e-300 {
                    return None;
                }
                l.set(i, j, s / d);
            }
        }
    }
    Some(l)
}

/// Solve `L·Lᵀ·x = b` for the factor `L` from [`cholesky`].
pub fn cholesky_solve(l: &Dense, b: &[f64]) -> Vec<f64> {
    let n = l.size();
    let mut y = vec![0.0f64; n];
    for i in 0..n {
        let mut s = b.get(i).copied().unwrap_or(0.0);
        for k in 0..i {
            s -= l.at(i, k) * y.get(k).copied().unwrap_or(0.0);
        }
        let d = l.at(i, i);
        if let Some(slot) = y.get_mut(i) {
            *slot = if d.abs() > 1e-300 { s / d } else { 0.0 };
        }
    }
    let mut x = vec![0.0f64; n];
    for i in (0..n).rev() {
        let mut s = y.get(i).copied().unwrap_or(0.0);
        for k in (i + 1)..n {
            s -= l.at(k, i) * x.get(k).copied().unwrap_or(0.0);
        }
        let d = l.at(i, i);
        if let Some(slot) = x.get_mut(i) {
            *slot = if d.abs() > 1e-300 { s / d } else { 0.0 };
        }
    }
    x
}
