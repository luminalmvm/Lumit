//! How far each cell of a box round a model is from its surface, so a million
//! particles can each ask whether they have gone in without walking a triangle.
//! Negative inside. A surface with no inside still has a thin skin to hit.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use glam::{Mat4, Vec3};
use lumit_core::model::Document;
use lumit_core::model_layer::ModelSettings;
use lumit_scene::{Clip, ModelScene};

/// Cells along a model's longest side.
const CELLS: f32 = 96.0;

/// Empty cells round the model.
const ROOM: f32 = 2.0;

/// How thick the skin on a surface is, in cells.
const SKIN: f32 = 0.75;

/// The most models whose distances stay in memory.
const HELD: usize = 8;

pub(super) struct Distances {
    pub values: Vec<f32>,
    pub cells: [u32; 3],
    pub cell_to_model: Mat4,
}

/// Held with the scene they came from, so a relinked file works them out again.
type Held = (Arc<ModelScene>, Arc<Distances>);

/// The model and the take it starts in, which two layers of one file can differ on.
type Key = (uuid::Uuid, Option<String>);

static HOLD: Mutex<Option<HashMap<Key, Held>>> = Mutex::new(None);

/// `item`'s distances in the pose the take `settings` names starts in, worked
/// out the first time they are asked for.
pub(super) fn of(
    doc: &Document,
    item: uuid::Uuid,
    settings: &ModelSettings,
) -> Option<Arc<Distances>> {
    let scene = crate::model::scene(doc, item)?;
    let take = crate::model_motion::clip_of(settings, &scene);
    let key = (item, take.map(|clip| clip.name.clone()));
    if let Some((from, found)) = HOLD.lock().ok()?.as_ref().and_then(|h| h.get(&key)) {
        if Arc::ptr_eq(from, &scene) {
            return Some(Arc::clone(found));
        }
    }
    let made = Arc::new(worked_out(&scene, take)?);
    let mut hold = HOLD.lock().ok()?;
    let held = hold.get_or_insert_with(HashMap::new);
    if held.len() >= HELD {
        held.clear();
    }
    held.insert(key, (Arc::clone(&scene), Arc::clone(&made)));
    Some(made)
}

/// The nearest distance from `p` to the triangle `a b c`.
fn to_triangle(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> f32 {
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return ap.length();
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return bp.length();
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return (ap - ab * (d1 / (d1 - d3))).length();
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return cp.length();
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return (ap - ac * (d2 / (d2 - d6))).length();
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 >= d3 && d5 >= d6 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return (bp - (c - b) * w).length();
    }
    let n = ab.cross(ac);
    let len = n.length();
    if len > 0.0 {
        n.dot(ap).abs() / len
    } else {
        ap.length()
    }
}

fn worked_out(scene: &ModelScene, take: Option<&Clip>) -> Option<Distances> {
    // The take's first moment, as a skin's file lists its vertices in a bind
    // pose nothing draws. Moved parts are left out, so a part's rows draw again
    // without another shape to work out.
    let posed = lumit_scene::pose::pose(scene, take.map(|clip| (clip, 0.0)), &[]);
    let mut tris: Vec<[Vec3; 3]> = Vec::new();
    for (k, node) in scene.nodes.iter().enumerate() {
        let Some(mesh) = node.mesh.and_then(|m| scene.meshes.get(m)) else {
            continue;
        };
        let mover = posed.mover(scene, k);
        for p in &mesh.primitives {
            let corner = |i: u32| mover.vertex(p, i as usize);
            for tri in p.indices.chunks_exact(3) {
                if let (Some(a), Some(b), Some(c)) =
                    (corner(tri[0]), corner(tri[1]), corner(tri[2]))
                {
                    tris.push([a, b, c]);
                }
            }
        }
    }
    let mut lo = Vec3::splat(f32::INFINITY);
    let mut hi = Vec3::splat(f32::NEG_INFINITY);
    for v in tris.iter().flatten() {
        lo = lo.min(*v);
        hi = hi.max(*v);
    }
    let span = (hi - lo).max_element();
    if !(span.is_finite() && span > 0.0) {
        return None;
    }
    let cell = span / CELLS;
    let start = lo - Vec3::splat(cell * ROOM);
    let size = ((hi - lo) / cell).ceil() + Vec3::splat(ROOM * 2.0 + 1.0);
    let (nx, ny, nz) = (size.x as usize, size.y as usize, size.z as usize);
    let at = |x: usize, y: usize, z: usize| x + y * nx + z * nx * ny;
    let mut far = vec![f32::INFINITY; nx * ny * nz];
    for [a, b, c] in tris.iter().map(|t| t.map(|v| (v - start) / cell)) {
        let n = (b - a).cross(c - a).normalize_or_zero();
        let least = (a.min(b).min(c) - Vec3::ONE).max(Vec3::ZERO);
        let most = (a.max(b).max(c) + Vec3::ONE).min(size - Vec3::ONE);
        for z in least.z as usize..=most.z as usize {
            for y in least.y as usize..=most.y as usize {
                for x in least.x as usize..=most.x as usize {
                    let p = Vec3::new(x as f32, y as f32, z as f32);
                    if n != Vec3::ZERO && n.dot(p - a).abs() > 1.5 {
                        continue;
                    }
                    let d = to_triangle(p, a, b, c);
                    let slot = &mut far[at(x, y, z)];
                    *slot = slot.min(d);
                }
            }
        }
    }
    // Out from the surface, a sweep each way across every cell.
    let reach: Vec<(isize, isize, isize, f32)> = (-1..=1)
        .flat_map(|z: isize| {
            (-1..=1).flat_map(move |y: isize| (-1..=1).map(move |x: isize| (x, y, z)))
        })
        .filter(|&(x, y, z)| (z, y, x) < (0, 0, 0))
        .map(|(x, y, z)| (x, y, z, ((x * x + y * y + z * z) as f32).sqrt()))
        .collect();
    let (dy, dz) = (nx as isize, (nx * ny) as isize);
    let sweep = |far: &mut Vec<f32>, x: usize, y: usize, z: usize, sign: isize| {
        let i = at(x, y, z) as isize;
        let mut best = far[i as usize];
        for &(ox, oy, oz, w) in &reach {
            let (ox, oy, oz) = (ox * sign, oy * sign, oz * sign);
            let (px, py, pz) = (x as isize + ox, y as isize + oy, z as isize + oz);
            if px < 0
                || py < 0
                || pz < 0
                || px >= nx as isize
                || py >= ny as isize
                || pz >= nz as isize
            {
                continue;
            }
            best = best.min(far[(i + ox + oy * dy + oz * dz) as usize] + w);
        }
        far[i as usize] = best;
    };
    for z in 0..nz {
        for y in 0..ny {
            for x in 0..nx {
                sweep(&mut far, x, y, z, 1);
            }
        }
    }
    for z in (0..nz).rev() {
        for y in (0..ny).rev() {
            for x in (0..nx).rev() {
                sweep(&mut far, x, y, z, -1);
            }
        }
    }
    // Whatever can reach the box's edge without crossing the skin is outside.
    let mut outside = vec![false; far.len()];
    let mut stack = Vec::new();
    let mut seed = |x: usize, y: usize, z: usize, outside: &mut Vec<bool>| {
        let i = at(x, y, z);
        if !outside[i] && far[i] >= SKIN {
            outside[i] = true;
            stack.push((x, y, z));
        }
    };
    for z in 0..nz {
        for y in 0..ny {
            for x in 0..nx {
                if x == 0 || y == 0 || z == 0 || x == nx - 1 || y == ny - 1 || z == nz - 1 {
                    seed(x, y, z, &mut outside);
                }
            }
        }
    }
    while let Some((x, y, z)) = stack.pop() {
        for (ox, oy, oz) in [
            (1isize, 0isize, 0isize),
            (-1, 0, 0),
            (0, 1, 0),
            (0, -1, 0),
            (0, 0, 1),
            (0, 0, -1),
        ] {
            let (px, py, pz) = (x as isize + ox, y as isize + oy, z as isize + oz);
            if px < 0
                || py < 0
                || pz < 0
                || px >= nx as isize
                || py >= ny as isize
                || pz >= nz as isize
            {
                continue;
            }
            let (px, py, pz) = (px as usize, py as usize, pz as usize);
            let i = at(px, py, pz);
            if !outside[i] && far[i] >= SKIN {
                outside[i] = true;
                stack.push((px, py, pz));
            }
        }
    }
    let values = far
        .iter()
        .zip(&outside)
        .map(|(&d, &out)| if out || d < SKIN { d - SKIN } else { -d })
        .collect();
    Some(Distances {
        values,
        cells: [nx as u32, ny as u32, nz as u32],
        cell_to_model: Mat4::from_translation(start) * Mat4::from_scale(Vec3::splat(cell)),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// The distance at model point `p`.
    fn at(d: &Distances, p: Vec3) -> f32 {
        let c = d.cell_to_model.inverse().transform_point3(p).round();
        let [nx, ny, _] = d.cells.map(|n| n as usize);
        d.values[c.x as usize + c.y as usize * nx + c.z as usize * nx * ny]
    }

    /// A skinned model is hit where its take starts, not in its bind pose.
    #[test]
    fn a_skin_is_hit_in_the_pose_its_take_starts_in() {
        let mut scene = lumit_scene::fixtures::bending(24, 16, 4);
        // Starts bent, by moving each bone's middle key to the front.
        for channel in &mut scene.clips[0].channels {
            if channel.path == lumit_scene::Path::Rotation {
                let bent: Vec<f32> = channel.values[4..8].to_vec();
                channel.values[..4].copy_from_slice(&bent);
            }
        }
        let top = Vec3::new(0.0, 0.9, 0.0);
        let bind = worked_out(&scene, None).unwrap();
        assert!(at(&bind, top) < 0.0, "at rest the top is inside");
        let posed = worked_out(&scene, scene.clips.first()).unwrap();
        assert!(at(&posed, top) > 0.0, "bent, it has moved away from there");
    }
}
