//! A model at one moment: its nodes where a take puts them, with the user's
//! part rows on top, and what skins and morph targets need from that.
//!
//! Worked out from the moment alone, with nothing kept from the frame before,
//! so a frame looks the same scrubbed, played or exported.

use glam::{Mat4, Quat, Vec3, Vec4};

use crate::{Bounds, Channel, Clip, Interpolation, ModelScene, Path};

/// What a part's rows do to it, in model space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PartEdit {
    pub node: usize,
    pub moved: Vec3,
    /// About the part's own origin.
    pub turned: Quat,
    pub scale: f32,
    pub hidden: bool,
}

/// Every node of a scene at one moment.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Pose {
    /// Each node's place in model space.
    pub worlds: Vec<Mat4>,
    /// Each node, and whether it or a parent is hidden.
    pub hidden: Vec<bool>,
    /// Every skin's joint matrices, skin after skin.
    pub palette: Vec<Mat4>,
    /// Morph weights, a run for each node whose mesh has targets, in node
    /// order.
    pub weights: Vec<f32>,
    /// The box round what shows.
    pub bounds: Bounds,
}

impl ModelScene {
    /// How many morph targets node `k`'s mesh has.
    #[must_use]
    pub fn targets_of(&self, k: usize) -> usize {
        let mesh = self.nodes.get(k).and_then(|n| self.meshes.get(n.mesh?));
        let counts = mesh.into_iter().flat_map(|m| &m.primitives);
        counts.map(|p| p.morph_targets.len()).max().unwrap_or(0)
    }
}

impl Pose {
    /// What moves node `k`'s vertices in this pose, on the processor, for a
    /// click to meet the shape that shows.
    #[must_use]
    pub fn mover<'a>(&'a self, scene: &'a ModelScene, k: usize) -> Mover<'a> {
        let first: usize = (0..k).map(|n| scene.targets_of(n)).sum();
        let node = scene.nodes.get(k);
        let skin = node.and_then(|n| n.skin).map(|s| {
            let joints = scene.skins.iter().take(s);
            joints
                .map(|j| j.joints.len().min(j.inverse_bind.len()))
                .sum::<usize>()
        });
        Mover {
            weights: self
                .weights
                .get(first..first + scene.targets_of(k))
                .unwrap_or(&[]),
            skin: skin.and_then(|first| self.palette.get(first..)),
            world: self.worlds.get(k).copied().unwrap_or(Mat4::IDENTITY),
        }
    }
}

/// A node's part of a pose: where its vertices go.
pub struct Mover<'a> {
    weights: &'a [f32],
    skin: Option<&'a [Mat4]>,
    world: Mat4,
}

impl Mover<'_> {
    /// Vertex `i` of `p` in model space, `None` past its vertices.
    #[must_use]
    pub fn vertex(&self, p: &crate::Primitive, i: usize) -> Option<Vec3> {
        let mut at = Vec3::from(*p.positions.get(i)?);
        for (w, target) in self.weights.iter().zip(&p.morph_targets) {
            if let Some(&d) = target.positions.get(i) {
                at += Vec3::from(d) * *w;
            }
        }
        let influences = p.joints.get(i).zip(p.weights.get(i));
        let (Some(skin), Some((joints, shares))) = (self.skin, influences) else {
            return Some(self.world.transform_point3(at));
        };
        let total: f32 = shares.iter().sum();
        let mut out = Vec3::ZERO;
        for (&j, &w) in joints.iter().zip(shares) {
            if w > 0.0 {
                let m = skin.get(usize::from(j)).unwrap_or(&Mat4::IDENTITY);
                out += m.transform_point3(at) * (w / total);
            }
        }
        Some(out)
    }
}

/// `scene` at `at` seconds into `take`, or at rest with none, with `edits`
/// over that. Never panics on a broken file: a channel that doesn't add up
/// leaves its node where it was.
#[must_use]
pub fn pose(scene: &ModelScene, take: Option<(&Clip, f64)>, edits: &[PartEdit]) -> Pose {
    let count = scene.nodes.len();
    let mut parts: Vec<(Vec3, Quat, Vec3)> = scene
        .nodes
        .iter()
        .map(|n| {
            let turn = unit(Quat::from_array(n.rotation)).unwrap_or(Quat::IDENTITY);
            (n.translation.into(), turn, n.scale.into())
        })
        .collect();
    let mut weights = Vec::new();
    let mut starts = vec![0; count];
    for (k, node) in scene.nodes.iter().enumerate() {
        starts[k] = weights.len();
        let n = scene.targets_of(k);
        weights.extend((0..n).map(|i| node.weights.get(i).copied().unwrap_or(0.0)));
    }
    if let Some((clip, at)) = take {
        for channel in &clip.channels {
            let Some(part) = parts.get_mut(channel.node) else {
                continue;
            };
            let at = at as f32;
            match channel.path {
                Path::YFov => {}
                Path::Translation => part.0 = sample_vec3(channel, at).unwrap_or(part.0),
                Path::Scale => part.2 = sample_vec3(channel, at).unwrap_or(part.2),
                Path::Rotation => part.1 = sample_turn(channel, at).unwrap_or(part.1),
                Path::Weights => {
                    let first = starts[channel.node];
                    let run = first..first + scene.targets_of(channel.node);
                    if let Some(run) = weights.get_mut(run) {
                        sample_into(channel, at, run);
                    }
                }
                Path::Weight(target) => {
                    let target = usize::from(target);
                    if target < scene.targets_of(channel.node) {
                        let one = &mut weights[starts[channel.node] + target];
                        sample_into(channel, at, std::slice::from_mut(one));
                    }
                }
            }
        }
    }
    let mut edit_of: Vec<Option<&PartEdit>> = vec![None; count];
    for e in edits {
        if let Some(slot) = edit_of.get_mut(e.node) {
            *slot = Some(e);
        }
    }
    let mut worlds = vec![Mat4::IDENTITY; count];
    let mut hidden = vec![false; count];
    let mut stack: Vec<(usize, Mat4, bool)> = scene
        .roots
        .iter()
        .rev()
        .map(|&n| (n, Mat4::IDENTITY, false))
        .collect();
    while let Some((k, parent, gone)) = stack.pop() {
        let Some(node) = scene.nodes.get(k) else {
            continue;
        };
        let (t, r, s) = parts[k];
        let mut world = parent * Mat4::from_scale_rotation_translation(s, r, t);
        let mut gone = gone;
        if let Some(e) = edit_of[k] {
            let at = world.w_axis.truncate();
            world = Mat4::from_translation(at + e.moved)
                * Mat4::from_quat(e.turned)
                * Mat4::from_translation(-at)
                * world
                * Mat4::from_scale(Vec3::splat(e.scale));
            gone |= e.hidden;
        }
        worlds[k] = world;
        hidden[k] = gone;
        // Followed only from the parent the tree was linked with, so a child
        // two nodes both claim is walked once.
        let children = node.children.iter().rev();
        stack.extend(
            children
                .filter(|&&c| scene.nodes.get(c).is_some_and(|n| n.parent == Some(k)))
                .map(|&c| (c, world, gone)),
        );
    }
    let mut palette = Vec::new();
    for skin in &scene.skins {
        palette.extend(skin.joints.iter().zip(&skin.inverse_bind).map(|(&j, ib)| {
            // A hidden bone takes what hangs from it down to nothing, as its
            // mesh isn't its child to hide.
            let gone = hidden.get(j).copied().unwrap_or(false);
            let scale = if gone { Vec3::ZERO } else { Vec3::ONE };
            worlds.get(j).copied().unwrap_or(Mat4::IDENTITY)
                * Mat4::from_scale(scale)
                * Mat4::from_cols_array_2d(ib)
        }));
    }
    let mut pose = Pose {
        worlds,
        hidden,
        palette,
        weights,
        bounds: Bounds::default(),
    };
    pose.bounds = posed_bounds(scene, &pose, &starts);
    pose
}

/// The box round what `pose` shows: each mesh's box at rest, grown by its
/// morph weights, and carried by its node or by each joint of its skin.
fn posed_bounds(scene: &ModelScene, pose: &Pose, starts: &[usize]) -> Bounds {
    let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    let mut grow = |m: &Mat4, a: Vec3, b: Vec3| {
        for i in 0..8 {
            let pick = |bit: usize, k: usize| if i & bit == 0 { a[k] } else { b[k] };
            let corner = Vec3::new(pick(1, 0), pick(2, 1), pick(4, 2));
            let p = m.transform_point3(corner);
            if p.is_finite() {
                lo = lo.min(p);
                hi = hi.max(p);
            }
        }
    };
    let mut first_joint = Vec::with_capacity(scene.skins.len());
    let mut joints = 0;
    for skin in &scene.skins {
        first_joint.push(joints);
        joints += skin.joints.len().min(skin.inverse_bind.len());
    }
    for (k, node) in scene.nodes.iter().enumerate() {
        let Some(mesh) = node.mesh.and_then(|m| scene.meshes.get(m)) else {
            continue;
        };
        if pose.hidden.get(k).copied().unwrap_or(false) {
            continue;
        }
        // How far the targets can carry a corner, weighted now.
        let (mut below, mut above) = (Vec3::ZERO, Vec3::ZERO);
        for p in &mesh.primitives {
            for (i, target) in p.morph_targets.iter().enumerate() {
                let w = pose.weights.get(starts[k] + i).copied().unwrap_or(0.0);
                let (a, b) = (
                    Vec3::from(target.reach.min) * w,
                    Vec3::from(target.reach.max) * w,
                );
                below += a.min(b);
                above += a.max(b);
            }
        }
        let skin = node.skin.and_then(|s| Some((s, scene.skins.get(s)?)));
        if let Some((s, skin)) = skin {
            for (j, b) in skin.boxes.iter().enumerate() {
                if b.min[0] > b.max[0] {
                    continue;
                }
                let m = pose.palette.get(first_joint[s] + j);
                grow(
                    m.unwrap_or(&Mat4::IDENTITY),
                    Vec3::from(b.min) + below,
                    Vec3::from(b.max) + above,
                );
            }
        } else if let Some(m) = pose.worlds.get(k) {
            grow(
                m,
                Vec3::from(mesh.bounds.min) + below,
                Vec3::from(mesh.bounds.max) + above,
            );
        }
    }
    if lo.x > hi.x {
        return Bounds::default();
    }
    Bounds {
        min: lo.to_array(),
        max: hi.to_array(),
    }
}

/// The two keys around `at`, and how far between them it is, with how far
/// apart they are in seconds. A moment outside the keys holds the nearer.
fn around(times: &[f32], at: f32) -> Option<(usize, usize, f32, f32)> {
    let last = times.len().checked_sub(1)?;
    let after = times.partition_point(|&t| t <= at);
    if after == 0 {
        return Some((0, 0, 0.0, 0.0));
    }
    if after > last {
        return Some((last, last, 0.0, 0.0));
    }
    let (a, b) = (after - 1, after);
    let span = times[b] - times[a];
    let u = if span > 0.0 {
        (at - times[a]) / span
    } else {
        0.0
    };
    Some((a, b, u, span))
}

/// Calls `out` with each of the `n` numbers of `channel` at `at`, `None` when
/// the channel's lists don't add up.
fn sample(channel: &Channel, at: f32, n: usize, mut out: impl FnMut((usize, f32))) -> Option<()> {
    let (a, b, u, span) = around(&channel.times, at)?;
    let stride = match channel.interpolation {
        Interpolation::CubicSpline => n * 3,
        _ => n,
    };
    if n == 0 || channel.values.len() < channel.times.len() * stride {
        return None;
    }
    for i in 0..n {
        let v = channel.values.as_slice();
        let value = match channel.interpolation {
            Interpolation::Step => *v.get(a * stride + i)?,
            Interpolation::Linear => {
                let (x, y) = (*v.get(a * stride + i)?, *v.get(b * stride + i)?);
                x + (y - x) * u
            }
            // Each key holds an in-tangent, the value and an out-tangent.
            Interpolation::CubicSpline => {
                let x = *v.get(a * stride + n + i)?;
                let y = *v.get(b * stride + n + i)?;
                let out_tangent = *v.get(a * stride + 2 * n + i)? * span;
                let in_tangent = *v.get(b * stride + i)? * span;
                let (u2, u3) = (u * u, u * u * u);
                (2.0 * u3 - 3.0 * u2 + 1.0) * x
                    + (u3 - 2.0 * u2 + u) * out_tangent
                    + (-2.0 * u3 + 3.0 * u2) * y
                    + (u3 - u2) * in_tangent
            }
        };
        out((i, value));
    }
    Some(())
}

/// `channel`'s one number at `at`.
#[must_use]
pub fn sampled(channel: &Channel, at: f32) -> Option<f32> {
    let mut one = None;
    sample(channel, at, 1, |(_, v)| one = Some(v))?;
    one
}

/// The eight corners of joint `k`'s box in `pose`, in model space, when `k` is
/// a joint of a skin that moves anything.
#[must_use]
pub fn joint_box(scene: &ModelScene, pose: &Pose, k: usize) -> Option<[Vec3; 8]> {
    let mut first = 0;
    for skin in &scene.skins {
        if let Some(j) = skin.joints.iter().position(|&n| n == k) {
            let b = skin.boxes.get(j).filter(|b| b.min[0] <= b.max[0])?;
            return Some(corners(pose.palette.get(first + j)?, b));
        }
        first += skin.joints.len().min(skin.inverse_bind.len());
    }
    None
}

/// The corners of `b` through `m`, by the bits of x, y and z.
#[must_use]
pub fn corners(m: &Mat4, b: &Bounds) -> [Vec3; 8] {
    std::array::from_fn(|i| {
        let pick = |bit: usize, k: usize| if i & bit == 0 { b.min[k] } else { b.max[k] };
        m.transform_point3(Vec3::new(pick(1, 0), pick(2, 1), pick(4, 2)))
    })
}

/// `channel`'s numbers at `at` into `run`, left as it was if the lists don't
/// add up.
fn sample_into(channel: &Channel, at: f32, run: &mut [f32]) {
    let mut held = vec![0.0; run.len()];
    let done = sample(channel, at, run.len(), |(i, v)| held[i] = v);
    if done.is_some() {
        run.copy_from_slice(&held);
    }
}

fn sample_vec3(channel: &Channel, at: f32) -> Option<Vec3> {
    let mut v = [0.0; 3];
    sample(channel, at, 3, |(i, x)| v[i] = x)?;
    Some(Vec3::from(v)).filter(|v| v.is_finite())
}

fn sample_turn(channel: &Channel, at: f32) -> Option<Quat> {
    let (a, b, u, _) = around(&channel.times, at)?;
    if channel.interpolation == Interpolation::Linear {
        // Round the short way, at an even speed, as glTF says.
        let key = |k: usize| {
            let v = channel.values.get(k * 4..k * 4 + 4)?;
            unit(Quat::from_xyzw(v[0], v[1], v[2], v[3]))
        };
        return Some(key(a)?.slerp(key(b)?, u));
    }
    let mut v = [0.0; 4];
    sample(channel, at, 4, |(i, x)| v[i] = x)?;
    unit(Quat::from_array(v))
}

/// `q` at length one, `None` when it has no length to scale.
fn unit(q: Quat) -> Option<Quat> {
    Vec4::from(q).try_normalize().map(Quat::from_vec4)
}

/// How `at` seconds of a take `length` long come out under a Loop row: Off
/// plays once and rests either side of it, Loop goes round, Mirror goes there
/// and back, and Freeze holds the first and last poses. `None` at rest.
#[must_use]
pub fn take_time(at: f64, length: f64, looping: Looping) -> Option<f64> {
    if length <= 0.0 {
        return Some(0.0);
    }
    match looping {
        Looping::Off => (0.0..=length).contains(&at).then_some(at),
        Looping::Loop => Some(at.rem_euclid(length)),
        Looping::Mirror => {
            let t = at.rem_euclid(length * 2.0);
            Some(if t > length { length * 2.0 - t } else { t })
        }
        Looping::Freeze => Some(at.clamp(0.0, length)),
    }
}

/// What a take does past its ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Looping {
    Off,
    Loop,
    Mirror,
    Freeze,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::{Node, Skin};

    fn node(parent: Option<usize>, children: Vec<usize>, at: [f32; 3]) -> Node {
        Node {
            parent,
            children,
            translation: at,
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0; 3],
            ..Node::default()
        }
    }

    fn scene() -> ModelScene {
        ModelScene {
            nodes: vec![
                node(None, vec![1], [1.0, 0.0, 0.0]),
                node(Some(0), vec![], [0.0, 2.0, 0.0]),
            ],
            roots: vec![0],
            ..ModelScene::default()
        }
    }

    fn slide(values: Vec<f32>, times: Vec<f32>) -> Clip {
        Clip {
            name: "slide".into(),
            duration: 2.0,
            channels: vec![Channel {
                node: 1,
                path: Path::Translation,
                interpolation: Interpolation::Linear,
                times,
                values,
            }],
        }
    }

    /// The same moment gives the same pose however it is reached, children
    /// follow their parents, and a part's rows carry its children with it.
    #[test]
    fn a_pose_comes_from_the_moment_alone() {
        let s = scene();
        let clip = slide(vec![0.0, 0.0, 0.0, 4.0, 0.0, 0.0], vec![0.0, 2.0]);
        let once = pose(&s, Some((&clip, 1.0)), &[]);
        let _ = pose(&s, Some((&clip, 1.7)), &[]);
        assert_eq!(once, pose(&s, Some((&clip, 1.0)), &[]));
        assert_eq!(
            once.worlds[1].w_axis.truncate(),
            Vec3::new(3.0, 0.0, 0.0),
            "half way to 4, on the parent's 1"
        );
        let moved = PartEdit {
            node: 0,
            moved: Vec3::Y,
            turned: Quat::IDENTITY,
            scale: 1.0,
            hidden: true,
        };
        let edited = pose(&s, None, &[moved]);
        assert_eq!(edited.worlds[1].w_axis.y, 3.0);
        assert!(edited.hidden[1], "hiding a part hides what hangs off it");
        // A bone hidden collapses what it moves.
        let mut rig = scene();
        rig.skins.push(Skin {
            joints: vec![1],
            inverse_bind: vec![Mat4::IDENTITY.to_cols_array_2d()],
            ..Skin::default()
        });
        let palette = pose(&rig, None, &[moved]).palette;
        assert_eq!(palette[0].transform_vector3(Vec3::ONE), Vec3::ZERO);
    }

    /// A file with broken lists draws at rest rather than panicking.
    #[test]
    fn broken_channels_leave_the_node_at_rest() {
        let s = scene();
        for clip in [
            slide(vec![1.0], vec![0.0, 2.0]),
            slide(Vec::new(), Vec::new()),
            Clip {
                channels: vec![Channel {
                    node: 99,
                    path: Path::Rotation,
                    interpolation: Interpolation::CubicSpline,
                    times: vec![0.0],
                    values: vec![],
                }],
                ..Clip::default()
            },
        ] {
            assert_eq!(
                pose(&s, Some((&clip, 1.0)), &[]).worlds,
                pose(&s, None, &[]).worlds
            );
        }
        // A skin naming nodes that aren't there holds still too.
        let mut s = s;
        s.skins.push(Skin {
            joints: vec![42],
            inverse_bind: vec![Mat4::IDENTITY.to_cols_array_2d()],
            ..Skin::default()
        });
        assert_eq!(pose(&s, None, &[]).palette, vec![Mat4::IDENTITY]);
    }

    #[test]
    fn the_loop_row_folds_time() {
        use Looping::*;
        assert_eq!(take_time(2.5, 2.0, Off), None);
        assert_eq!(take_time(-0.5, 2.0, Off), None);
        assert_eq!(take_time(2.5, 2.0, Loop), Some(0.5));
        assert_eq!(take_time(2.5, 2.0, Mirror), Some(1.5));
        assert_eq!(take_time(4.5, 2.0, Mirror), Some(0.5));
        assert_eq!(take_time(9.0, 2.0, Freeze), Some(2.0));
    }
}
