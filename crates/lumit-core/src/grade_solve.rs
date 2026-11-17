//! Solvers that work out a Correction's rows from the picture: auto balance,
//! shot match, the colour chart, and the qualifier and hue curve picks.
//!
//! **In plain terms.** Each one looks at the picture coming into a node and
//! says which rows to set so the node does what a colourist would do by hand.
//! They are pure functions on pixel slices: the caller renders the picture
//! (scene-linear, premultiplied, four floats a pixel) and writes the rows it
//! gets back. Every formula they solve against is the Correction's own, from
//! [`maths`](crate::fx::effects::correction::maths), so a solved row lands
//! where the render will put it.

pub mod balance;
pub mod cdl_fit;
pub mod chart;
pub mod matching;
pub mod pick;
pub mod pick3d;

use crate::fx::effects::correction::maths;
use crate::fx::effects::correction::Correction;

/// One row's new value, by the Correction's own param kinds.
// A Shape is a few hundred bytes, and a solver hands back a handful of rows,
// so boxing it would cost more than it saves.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RowValue {
    Float(f32),
    /// Red, green and blue; alpha stays as it is.
    Colour([f32; 3]),
    Choice(u32),
    Bool(bool),
    /// A Shape row's points, such as the 3D qualifier's samples.
    Shape(crate::fx::shape_points::ShapePoints),
}

/// The rows a solver sets, by param id. Rows not listed are left alone.
pub type Rows = Vec<(&'static str, RowValue)>;

/// Why a solver gave no answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SolveError {
    /// Nothing in the picture (or under the pick) to read.
    #[error("there's nothing in the picture to read")]
    Empty,
}

/// The colours of the pixels worth reading, unpremultiplied: mostly opaque
/// and finite. A layer drawn alone is clear around its edges, and those
/// pixels are not part of the shot.
pub fn colours(rgba: &[f32]) -> impl Iterator<Item = [f32; 3]> + '_ {
    rgba.chunks_exact(4).filter_map(|p| {
        let a = p[3];
        (a >= 0.5 && p.iter().all(|c| c.is_finite())).then(|| [p[0] / a, p[1] / a, p[2] / a])
    })
}

/// The value at fraction `p` (0..1) of `values`, read between the two
/// nearest ranks. Sorts `values`. Nought when empty.
pub fn percentile(values: &mut [f32], p: f32) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_unstable_by(f32::total_cmp);
    let at = p.clamp(0.0, 1.0) * (values.len() - 1) as f32;
    let i = at.floor() as usize;
    let j = (i + 1).min(values.len() - 1);
    let f = at - i as f32;
    values[i] + (values[j] - values[i]) * f
}

/// Each channel's mean over `picked`, in order. `None` when nothing was.
fn mean(picked: impl Iterator<Item = [f32; 3]>) -> Option<[f32; 3]> {
    let mut sum = [0.0f64; 3];
    let mut n = 0u32;
    for v in picked {
        for c in 0..3 {
            sum[c] += f64::from(v[c]);
        }
        n += 1;
    }
    (n > 0).then(|| sum.map(|s| (s / f64::from(n)) as f32))
}

/// The node's gains before the grade (`2^exposure` and Temperature's), and
/// the space it grades in, as the render reads them.
fn pre_gain(node: &Correction) -> ([f32; 3], u32) {
    let pk = node.packed(1.0);
    (pk.pre_gain, pk.space)
}

/// What the node's wheels see of a scene-linear colour: the gains, the
/// encode into the grading space, and Offset.
fn wheel_input(u: [f32; 3], gain: [f32; 3], space: u32, offset: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|c| maths::encode(u[c] * gain[c], space) + offset[c])
}

/// One channel through lift, gamma (as its exponent) and gain.
fn wheels(x: f32, lift: f32, exponent: f32, gain: f32) -> f32 {
    maths::sgnpow(x + lift * (1.0 - x), exponent) * gain
}

/// The Gamma row whose exponent is `e`: `e = 2^(−2·gamma)`.
fn gamma_row(e: f32) -> f32 {
    (-e.max(1e-6).log2() / 2.0).clamp(-1.0, 1.0)
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::{RowValue, Rows};
    use crate::fx::effects::correction::{cpu, Correction};
    use crate::fx::params::{ParamId, Params, Value};
    use crate::fx::registry::EffectMetadata;

    /// A Correction with `rows` set over `base`'s, the way the panel writes
    /// them: by id.
    pub fn with_rows(base: &[(&'static str, RowValue)], rows: &Rows) -> Correction {
        let entries: Vec<(ParamId, Value)> = base
            .iter()
            .chain(rows.iter())
            .map(|(id, v)| {
                let value = match *v {
                    RowValue::Float(f) => Value::Float(f),
                    RowValue::Colour([r, g, b]) => Value::Colour([r, g, b, 1.0]),
                    RowValue::Choice(c) => Value::Choice(c),
                    RowValue::Bool(b) => Value::Bool(b),
                    RowValue::Shape(s) => Value::Shape(s),
                };
                (ParamId::new(id), value)
            })
            .rev()
            .collect();
        // A later row wins: the list is reversed, and a lookup takes the
        // first it meets.
        Correction::read(Params::new(&entries))
    }

    /// `rgba` through `node` on the CPU twin.
    pub fn graded(rgba: &[f32], w: u32, h: u32, node: &Correction) -> Vec<f32> {
        let mut out = rgba.to_vec();
        cpu::correction(&mut out, w, h, &node.packed(1.0), &[]);
        out
    }

    /// Every solver's row id is one the Correction declares.
    pub fn declared(rows: &Rows) -> bool {
        let schema = &<Correction as EffectMetadata>::SCHEMA;
        rows.iter()
            .all(|(id, _)| schema.params.iter().any(|p| p.id == *id))
    }
}
