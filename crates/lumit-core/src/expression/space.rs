//! Where a layer's points are: in its own pixels, in the comp's 3D space, and
//! on the picture. What `toComp`, `fromComp`, `toWorld` and `fromWorld` read in
//! both languages.
//!
//! It is the compositor's placement worked in `f64`: each parent's transform
//! outside the layer's own, every one read at its own layer time
//! (`parent_world_placement` in the renderer), and a 3D layer seen through the
//! active camera ([`crate::camera::project_depth`]). A 2D layer is never seen
//! through the camera, so on the picture it is its placement with z dropped.

use std::cell::RefCell;
use std::sync::Arc;

use uuid::Uuid;

use crate::anim::{Animation, Property};
use crate::camera::{self, Place};
use crate::expression::{ExpressionContext, Slot};
use crate::model::{CameraPose, Composition, Document, Layer};

type P3 = (f64, f64, f64);

thread_local! {
    /// The layers whose placement is being worked out on this thread. A
    /// position that asks where its own layer is would otherwise ask again
    /// for every property it reads, which doubles at each level. Nothing may
    /// hand work to another thread while an entry is held, or that thread's
    /// frame would see it.
    static WALKING: RefCell<Vec<Uuid>> = const { RefCell::new(Vec::new()) };
}

struct Walking;

impl Drop for Walking {
    fn drop(&mut self) {
        WALKING.with(|walking| walking.borrow_mut().pop());
    }
}

/// One layer's spaces at one time.
pub(crate) struct Space {
    world: Place,
    /// The camera a 3D layer is seen through, and the comp's size.
    camera: Option<(CameraPose, f64, f64)>,
    three_d: bool,
    /// How many layers were read to place it, for the step budget.
    pub(crate) cost: u32,
}

/// `layer`'s spaces at comp time `t`, with each property read by `read`.
///
/// A layer already being placed further up this thread is placed again from
/// the numbers under its expressions, as After Effects reads a property that
/// refers to itself.
pub(crate) fn space(
    doc: &Document,
    comp: &Composition,
    layer: &Layer,
    t: f64,
    read: &dyn Fn(&Layer, &Property) -> f64,
) -> Space {
    let again = WALKING.with(|walking| {
        let mut walking = walking.borrow_mut();
        let again = walking.contains(&layer.id);
        if !again {
            walking.push(layer.id);
        }
        again
    });
    let _walking = (!again).then_some(Walking);
    let plain =
        |l: &Layer, p: &Property| underneath(p, crate::time::layer_time(t, l.start_offset.0));
    let read: &dyn Fn(&Layer, &Property) -> f64 = if again { &plain } else { read };
    let own = |l: &Layer| {
        let tr = &l.transform;
        let v = |p: &Property| read(l, p);
        // The renderer's own rule: a 2D layer's hidden z and tilt count for
        // nothing, in its own placement and as a parent.
        let hidden = |p: &Property| if l.is_3d() { v(p) } else { 0.0 };
        camera::place(
            (v(&tr.position_x), v(&tr.position_y), hidden(&tr.position_z)),
            (v(&tr.anchor_x), v(&tr.anchor_y)),
            (v(&tr.scale_x), v(&tr.scale_y)),
            (
                hidden(&tr.rotation_x),
                hidden(&tr.rotation_y),
                v(&tr.rotation),
            ),
        )
    };
    let mut world = own(layer);
    let mut cost = 1;
    // Nearest parent first, each one wrapping what is inside it, and a loop
    // in a broken file walked once.
    let mut seen = vec![layer.id];
    let mut child = layer;
    while let Some(id) = child.parent.filter(|id| !seen.contains(id)) {
        let Some(parent) = comp.layers.iter().find(|l| l.id == id) else {
            break;
        };
        world = world.within(&own(parent));
        seen.push(id);
        child = parent;
        cost += 1;
    }
    let three_d = layer.is_3d();
    let camera = three_d
        .then(|| crate::track::active_camera_pose(doc, comp, t))
        .flatten()
        .map(|pose| (pose, f64::from(comp.width), f64::from(comp.height)));
    Space {
        world,
        camera,
        three_d,
        cost,
    }
}

/// What a property holds under any expression on it: the number the
/// expression was put on, or its keys at layer time `lt`.
fn underneath(property: &Property, lt: f64) -> f64 {
    match &property.animation {
        Animation::Expression(_) => Slot::read(&property.extra).own().unwrap_or(0.0),
        _ => property.value_at(lt),
    }
}

/// Reads a property as the renderer does: at its layer's own time, with any
/// expression on it run for that layer, in its comp, a level deeper.
pub(crate) fn reader(
    context: &ExpressionContext,
    comp: Uuid,
    t: f64,
) -> impl Fn(&Layer, &Property) -> f64 + '_ {
    move |layer, property| {
        let lt = crate::time::layer_time(t, layer.start_offset.0);
        match &property.animation {
            Animation::Expression(_) => property.value_at_with_context(
                lt,
                Arc::new(ExpressionContext {
                    comp: Some(comp),
                    layer: Some(layer.id),
                    comp_time: t,
                    ..context.increase_depth()
                }),
            ),
            _ => property.value_at(lt),
        }
    }
}

impl Space {
    fn to_world(&self, p: P3) -> P3 {
        self.world.point(p)
    }

    fn world_to_layer(&self, p: P3) -> Option<P3> {
        Some(self.world.invert()?.point(p))
    }

    /// On the picture. A 3D layer's third number is its depth in front of the
    /// plane the camera sees 1:1, which is plain z with no camera. A 2D
    /// layer's is nought.
    fn to_comp(&self, p: P3) -> P3 {
        let w = self.world.point(p);
        match &self.camera {
            Some((pose, width, height)) => camera::project_depth(pose, *width, *height, w),
            None if self.three_d => w,
            None => (w.0, w.1, 0.0),
        }
    }

    /// Back off the picture. A 2D layer ignores the third number, since all
    /// of it is on the comp's plane.
    fn comp_to_layer(&self, p: P3) -> Option<P3> {
        Some(match &self.camera {
            Some((pose, width, height)) => self
                .world
                .invert()?
                .point(camera::unproject(pose, *width, *height, p)),
            None if self.three_d => self.world.invert()?.point(p),
            None => self.flat_to_layer(p)?,
        })
    }

    /// The point on the layer's own plane that the picture shows at `p`: where
    /// the line from the eye through it meets the layer. `None` when the layer
    /// is seen edge on.
    fn comp_to_surface(&self, p: P3) -> Option<P3> {
        let Some((pose, width, height)) = &self.camera else {
            return self.flat_to_layer(p);
        };
        let back = self.world.invert()?;
        let eye = back.point(pose.position);
        let far = back.point(camera::unproject(pose, *width, *height, (p.0, p.1, 0.0)));
        let along = sub(far, eye);
        if along.2.abs() < 1e-12 {
            return None;
        }
        let k = -eye.2 / along.2;
        Some((eye.0 + along.0 * k, eye.1 + along.1 * k, 0.0))
    }

    /// The flat part of the placement, the one the 2D draw uses, undone.
    fn flat_to_layer(&self, p: P3) -> Option<P3> {
        let (m, t) = (&self.world.m, self.world.t);
        let det = m[0][0] * m[1][1] - m[0][1] * m[1][0];
        if !det.is_finite() || det.abs() < 1e-12 {
            return None;
        }
        let (x, y) = (p.0 - t.0, p.1 - t.1);
        Some((
            (m[1][1] * x - m[0][1] * y) / det,
            (m[0][0] * y - m[1][0] * x) / det,
            0.0,
        ))
    }

    fn to_comp_vec(&self, v: P3) -> P3 {
        sub(self.to_comp(v), self.to_comp((0.0, 0.0, 0.0)))
    }

    fn comp_vec_to_layer(&self, v: P3) -> Option<P3> {
        let origin = self.to_comp((0.0, 0.0, 0.0));
        let at = (origin.0 + v.0, origin.1 + v.1, origin.2 + v.2);
        Some(sub(self.comp_to_layer(at)?, self.comp_to_layer(origin)?))
    }

    fn to_world_vec(&self, v: P3) -> P3 {
        self.world.vector(v)
    }

    fn world_vec_to_layer(&self, v: P3) -> Option<P3> {
        Some(self.world.invert()?.vector(v))
    }
}

/// Which way a point is carried, and so how many numbers come back.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Way {
    ToComp,
    FromComp,
    ToWorld,
    FromWorld,
    ToCompVec,
    FromCompVec,
    ToWorldVec,
    FromWorldVec,
    FromCompToSurface,
}

impl Space {
    /// `p`, of which `given` numbers were given, carried `way`. Three numbers
    /// come out of world space and off a 3D layer, two off its surface, and as
    /// many as went in off a 2D layer. `None` for a layer scaled to nothing,
    /// or seen edge on from its surface.
    pub(crate) fn carry(&self, way: Way, p: P3, given: usize) -> Option<Vec<f64>> {
        let out = match way {
            Way::ToComp => self.to_comp(p),
            Way::FromComp => self.comp_to_layer(p)?,
            Way::ToWorld => self.to_world(p),
            Way::FromWorld => self.world_to_layer(p)?,
            Way::ToCompVec => self.to_comp_vec(p),
            Way::FromCompVec => self.comp_vec_to_layer(p)?,
            Way::ToWorldVec => self.to_world_vec(p),
            Way::FromWorldVec => self.world_vec_to_layer(p)?,
            Way::FromCompToSurface => self.comp_to_surface(p)?,
        };
        let world = matches!(
            way,
            Way::ToWorld | Way::FromWorld | Way::ToWorldVec | Way::FromWorldVec
        );
        let len = if way == Way::FromCompToSurface {
            2
        } else if world || self.three_d {
            3
        } else {
            given.clamp(2, 3)
        };
        Some([out.0, out.1, out.2].into_iter().take(len).collect())
    }
}

fn sub(a: P3, b: P3) -> P3 {
    (a.0 - b.0, a.1 - b.1, a.2 - b.2)
}
