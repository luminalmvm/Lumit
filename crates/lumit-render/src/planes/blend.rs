//! A plane read for a Blend or Flow in-between.

use std::sync::Arc;

use uuid::Uuid;

use super::{plane, PlaneKind, PlaneRead};

/// [`plane`] for `frame`, mixed with the plane of `next` by `w` where the
/// picture is an in-between of the two. The partner missing, or at another
/// raster, leaves `frame`'s plane as it was.
#[must_use]
pub fn plane_blended(instance: Uuid, frame: i64, blend: Option<(i64, f32)>) -> Option<PlaneRead> {
    let (width, height, kind, data, content) = plane(instance, frame)?;
    let Some((next, w)) = blend else {
        return Some((width, height, kind, data, content));
    };
    let Some((nw, nh, nkind, other, _)) = plane(instance, next) else {
        return Some((width, height, kind, data, content));
    };
    if (nw, nh, nkind) != (width, height, kind) || other.len() != data.len() {
        return Some((width, height, kind, data, content));
    }
    let mixed: Vec<u8> = match kind {
        PlaneKind::Depth => data
            .chunks_exact(2)
            .zip(other.chunks_exact(2))
            .flat_map(|(a, b)| {
                let a = f32::from(u16::from_le_bytes([a[0], a[1]]));
                let b = f32::from(u16::from_le_bytes([b[0], b[1]]));
                ((a + (b - a) * w).round() as u16).to_le_bytes()
            })
            .collect(),
        PlaneKind::Matte => data
            .iter()
            .zip(other.iter())
            .map(|(&a, &b)| (f32::from(a) + (f32::from(b) - f32::from(a)) * w).round() as u8)
            .collect(),
    };
    // A different picture, so a different name: the texture memo must not
    // hand this mix the first frame's plane.
    let content = content
        ^ (next as u64)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .wrapping_add((u64::from(w.to_bits()) << 32) | 1);
    Some((width, height, kind, Arc::new(mixed), content))
}
