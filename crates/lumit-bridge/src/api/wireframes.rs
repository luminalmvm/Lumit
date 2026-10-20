//! The wireframes a Viewer draws over a 3D view (docs/impl/camera.md §6):
//! every 3D layer's rectangle, every camera's frustum and every light, and the
//! ground, the comp's frame and the axes to judge them by. All of it is placed,
//! projected and cut at the eye here, so the frontend only joins the dots.
//!
//! Lines cross as flat lists of comp-pixel numbers, four to a line
//! (`x1 y1 x2 y2`), or five where each line carries one more number. Only the
//! part of a line in front of the eye, and near enough the comp to draw, is
//! ever in them.

use flutter_rust_bridge::frb;
use uuid::Uuid;

use super::composition::CompositionReference;
use super::layer::BridgeCameraPose;
use super::BridgeError;

/// A comp-space point taken through the view: where it lands in comp pixels,
/// and whether it sits in front of the view's eye. A point behind it still
/// carries a number, but not one worth drawing.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BridgeWirePoint {
    pub x: f64,
    pub y: f64,
    pub in_front: bool,
}

/// A 3D layer's rectangle, four lines to a list. A Null's square comes with a
/// small cross through its anchor along its own three axes.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeWireLayer {
    pub layer: Uuid,
    pub lines: Vec<f64>,
}

/// A camera: its eye, and the lines of its frustum (the first four go round
/// the comp-sized rectangle its zoom in front of it), its body behind the eye,
/// and the line to its point of interest when it has one.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeWireCamera {
    pub layer: Uuid,
    pub eye: BridgeWirePoint,
    pub lines: Vec<f64>,
}

#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeWireLightKind {
    Point,
    Spot,
    Area,
}

/// A light: where it is, for its mark, and its lines. A spot has its cone
/// (the edge and where the feather starts, as two rings) and the line it
/// shines along, out to its point of interest when it has one; a parallel
/// light has the line its rays run along, and an area light its rectangle.
/// An ambient light has no place, so none is drawn.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeWireLight {
    pub layer: Uuid,
    pub kind: BridgeWireLightKind,
    pub at: BridgeWirePoint,
    pub lines: Vec<f64>,
}

#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeWireframes {
    pub layers: Vec<BridgeWireLayer>,
    pub cameras: Vec<BridgeWireCamera>,
    pub lights: Vec<BridgeWireLight>,
    /// The ground grid, and a line from each camera, light and Null drawn
    /// down to it: five numbers a line, the fifth how strongly it is drawn,
    /// 0 to 1, fading out with distance from the middle.
    pub ground: Vec<f64>,
    /// The comp's own frame at its 1:1 depth, four numbers a line.
    pub frame: Vec<f64>,
    /// x, y and z from the world origin, five numbers a line, the fifth the
    /// axis (0, 1 or 2).
    pub axes: Vec<f64>,
}

/// A footage item's measured size, from the frontend's own probe cache: the
/// engine's probe lives on the worker, and this call runs on the caller's
/// thread, so the sizes it already holds come along rather than being asked
/// for again.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeMediaSize {
    pub item: Uuid,
    pub width: u32,
    pub height: u32,
}

type P3 = (f64, f64, f64);
type P2 = (f64, f64);
type Rows = [[f64; 4]; 3];
/// A placement, column by column as the compositor keeps it.
type Place = [[f64; 4]; 4];
/// A line in comp space, end to end.
type Seg = (P3, P3);

/// How far in front of the eye a line is cut, in zooms: the same hair the
/// Viewer's own outlines are cut at.
const NEAR: f64 = 1e-3;

fn row(r: &[f64; 4], p: P3) -> f64 {
    r[0] * p.0 + r[1] * p.1 + r[2] * p.2 + r[3]
}

fn lerp(a: P3, b: P3, t: f64) -> P3 {
    (
        a.0 + (b.0 - a.0) * t,
        a.1 + (b.1 - a.1) * t,
        a.2 + (b.2 - a.2) * t,
    )
}

fn sub(a: P3, b: P3) -> P3 {
    (a.0 - b.0, a.1 - b.1, a.2 - b.2)
}

fn dot(a: P3, b: P3) -> f64 {
    a.0 * b.0 + a.1 * b.1 + a.2 * b.2
}

fn apply(m: &Place, p: P3) -> P3 {
    (
        m[0][0] * p.0 + m[1][0] * p.1 + m[2][0] * p.2 + m[3][0],
        m[0][1] * p.0 + m[1][1] * p.1 + m[2][1] * p.2 + m[3][1],
        m[0][2] * p.0 + m[1][2] * p.1 + m[2][2] * p.2 + m[3][2],
    )
}

/// `a · b`: `b` applied first, then `a`.
fn compose(a: &Place, b: &Place) -> Place {
    std::array::from_fn(|c| std::array::from_fn(|r| (0..4).map(|k| a[k][r] * b[c][k]).sum()))
}

/// A closed loop through `corners`, as lines.
fn ring(corners: &[P3], out: &mut Vec<Seg>) {
    for (i, a) in corners.iter().enumerate() {
        out.push((*a, corners[(i + 1) % corners.len()]));
    }
}

/// A round number of comp pixels near `v`: 1, 2 or 5 times a power of ten.
fn nice(v: f64) -> f64 {
    if v <= 0.0 || !v.is_finite() {
        return 100.0;
    }
    let ten = 10f64.powf(v.log10().floor());
    let m = v / ten;
    ten * if m < 1.5 {
        1.0
    } else if m < 3.5 {
        2.0
    } else if m < 7.5 {
        5.0
    } else {
        10.0
    }
}

/// A 3D layer's picture as the view sees it, as something that hides the
/// lines behind it: its outline on screen, wound so the inside is to the left
/// of each edge, and its plane.
struct Card {
    quad: [P2; 4],
    low: P2,
    high: P2,
    origin: P3,
    normal: P3,
    /// Which side of the plane the eye is on.
    eye_side: f64,
}

impl Card {
    /// The card a picture of `size` placed by `place` makes, or none when it
    /// reaches behind the eye or is seen edge on.
    fn new(view: &View, place: &Place, size: P2) -> Option<Self> {
        let corners = [
            apply(place, (0.0, 0.0, 0.0)),
            apply(place, (size.0, 0.0, 0.0)),
            apply(place, (size.0, size.1, 0.0)),
            apply(place, (0.0, size.1, 0.0)),
        ];
        if corners.iter().any(|&c| row(&view.rows[2], c) < NEAR) {
            return None;
        }
        let mut quad = corners.map(|c| view.flat(c));
        let area: f64 = (0..4)
            .map(|i| {
                let (a, b) = (quad[i], quad[(i + 1) % 4]);
                a.0 * b.1 - b.0 * a.1
            })
            .sum();
        if area.abs() < 1e-6 {
            return None;
        }
        if area < 0.0 {
            quad.reverse();
        }
        let (u, v) = (sub(corners[1], corners[0]), sub(corners[3], corners[0]));
        let normal = (
            u.1 * v.2 - u.2 * v.1,
            u.2 * v.0 - u.0 * v.2,
            u.0 * v.1 - u.1 * v.0,
        );
        let eye_side = dot(normal, sub(view.eye, corners[0])).signum();
        let low = quad
            .iter()
            .fold((f64::MAX, f64::MAX), |m, p| (m.0.min(p.0), m.1.min(p.1)));
        let high = quad
            .iter()
            .fold((f64::MIN, f64::MIN), |m, p| (m.0.max(p.0), m.1.max(p.1)));
        Some(Self {
            quad,
            low,
            high,
            origin: corners[0],
            normal,
            eye_side,
        })
    }

    /// The stretch of the screen line `p` to `q` this card hides, as a span
    /// of 0 to 1 along it: where the line is inside the card's outline and
    /// behind its plane. `a` and `b` are the line's own ends in comp space.
    fn hides(&self, view: &View, (p, q): (P2, P2), (a, b): Seg) -> Option<P2> {
        if p.0.max(q.0) < self.low.0
            || p.0.min(q.0) > self.high.0
            || p.1.max(q.1) < self.low.1
            || p.1.min(q.1) > self.high.1
        {
            return None;
        }
        // Cyrus-Beck against the outline: inside is to the left of each edge.
        let d = (q.0 - p.0, q.1 - p.1);
        let (mut t0, mut t1) = (0.0_f64, 1.0_f64);
        for i in 0..4 {
            let (s, e) = (self.quad[i], self.quad[(i + 1) % 4]);
            let edge = (e.0 - s.0, e.1 - s.1);
            let at = edge.0 * (p.1 - s.1) - edge.1 * (p.0 - s.0);
            let along = edge.0 * d.1 - edge.1 * d.0;
            if along.abs() < 1e-12 {
                if at < 0.0 {
                    return None;
                }
            } else if along > 0.0 {
                t0 = t0.max(-at / along);
            } else {
                t1 = t1.min(-at / along);
            }
        }
        if t0 >= t1 {
            return None;
        }
        // Behind the plane is the side away from the eye. A straight line
        // crosses it at most once, so one split is enough.
        let (da, db) = (
            dot(self.normal, sub(a, self.origin)) * self.eye_side,
            dot(self.normal, sub(b, self.origin)) * self.eye_side,
        );
        if da >= 0.0 && db >= 0.0 {
            return None;
        }
        let (from, to) = if da < 0.0 && db < 0.0 {
            (0.0, 1.0)
        } else {
            let x = view.flat(lerp(a, b, da / (da - db)));
            let len = d.0 * d.0 + d.1 * d.1;
            let u = if len < 1e-12 {
                0.0
            } else {
                ((x.0 - p.0) * d.0 + (x.1 - p.1) * d.1) / len
            };
            if da < 0.0 {
                (0.0, u)
            } else {
                (u, 1.0)
            }
        };
        let (from, to) = (from.max(t0), to.min(t1));
        (from < to).then_some((from, to))
    }
}

/// How the view sees the comp: its rows, its eye, and the box on screen past
/// which a line is cut, so one passing close to the eye never hands the
/// canvas numbers in the millions.
struct View {
    rows: Rows,
    eye: P3,
    low: P2,
    high: P2,
}

impl View {
    fn flat(&self, p: P3) -> P2 {
        let w = row(&self.rows[2], p);
        (row(&self.rows[0], p) / w, row(&self.rows[1], p) / w)
    }

    fn point(&self, p: P3) -> BridgeWirePoint {
        let w = row(&self.rows[2], p);
        let d = if w.abs() < 1e-9 { 1e-9 } else { w };
        BridgeWirePoint {
            x: row(&self.rows[0], p) / d,
            y: row(&self.rows[1], p) / d,
            in_front: w > NEAR,
        }
    }

    /// The part of `a` to `b` in front of the eye, on screen.
    fn cut(&self, a: P3, b: P3) -> Option<(Seg, (P2, P2))> {
        let (wa, wb) = (row(&self.rows[2], a), row(&self.rows[2], b));
        if wa < NEAR && wb < NEAR {
            return None;
        }
        let a = if wa < NEAR {
            lerp(a, b, (NEAR - wa) / (wb - wa))
        } else {
            a
        };
        let b = if wb < NEAR {
            lerp(b, a, (NEAR - wb) / (wa - wb))
        } else {
            b
        };
        Some(((a, b), (self.flat(a), self.flat(b))))
    }

    /// `p` to `q` between `from` and `to` along it, inside the box, onto
    /// `out` as four numbers, and `tail` after them when there is one.
    fn emit(&self, (p, q): (P2, P2), from: f64, to: f64, tail: Option<f64>, out: &mut Vec<f64>) {
        let d = (q.0 - p.0, q.1 - p.1);
        let (mut t0, mut t1) = (from, to);
        // Liang-Barsky against the box.
        for (along, room) in [
            (-d.0, p.0 - self.low.0),
            (d.0, self.high.0 - p.0),
            (-d.1, p.1 - self.low.1),
            (d.1, self.high.1 - p.1),
        ] {
            if along.abs() < 1e-12 {
                if room < 0.0 {
                    return;
                }
            } else if along < 0.0 {
                t0 = t0.max(room / along);
            } else {
                t1 = t1.min(room / along);
            }
        }
        if t0 >= t1 {
            return;
        }
        out.extend_from_slice(&[
            p.0 + d.0 * t0,
            p.1 + d.1 * t0,
            p.0 + d.0 * t1,
            p.1 + d.1 * t1,
        ]);
        if let Some(tail) = tail {
            out.push(tail);
        }
    }

    /// A line onto `out`, cut at the eye and the box.
    fn line(&self, (a, b): Seg, out: &mut Vec<f64>) {
        if let Some((_, flat)) = self.cut(a, b) {
            self.emit(flat, 0.0, 1.0, None, out);
        }
    }

    /// A line onto `out`, less the stretches the cards hide, each part with
    /// `tail` after it.
    fn hidden_line(&self, cards: &[Card], (a, b): Seg, tail: Option<f64>, out: &mut Vec<f64>) {
        let Some((ends, flat)) = self.cut(a, b) else {
            return;
        };
        let mut hidden: Vec<P2> = cards
            .iter()
            .filter_map(|c| c.hides(self, flat, ends))
            .collect();
        hidden.sort_by(|x, y| x.0.total_cmp(&y.0));
        let mut from = 0.0;
        for (start, end) in hidden {
            if start > from {
                self.emit(flat, from, start, tail, out);
            }
            from = from.max(end);
        }
        if from < 1.0 {
            self.emit(flat, from, 1.0, tail, out);
        }
    }
}

/// What an outline is waiting for before it can be projected: the ground's
/// size sets the box its lines are cut to.
enum Outline {
    Layer(Uuid, Vec<Seg>),
    Camera(Uuid, P3, Vec<Seg>),
    Light(Uuid, BridgeWireLightKind, P3, Vec<Seg>),
}

impl CompositionReference {
    /// How the picture sees the comp's 3D space at `frame`: twelve numbers,
    /// three rows of four, taking a comp-space `(x, y, z, 1)` to `(X, Y, W)`,
    /// with the comp pixel at `(X / W, Y / W)` and `W` above 0 in front of the
    /// eye. Through `view` when one is given, otherwise through the comp's
    /// active camera as the render poses it, aim and solve link included, or
    /// the default camera when the comp has none.
    /// Asked once per frame, view or document change and held.
    #[frb(sync)]
    pub fn view_projection(
        &self,
        frame: u64,
        view: Option<BridgeCameraPose>,
    ) -> Result<Vec<f64>, BridgeError> {
        let doc = {
            let proj = self.project()?;
            let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
            proj.store.snapshot()
        };
        let comp = doc.comp(self.id).ok_or(BridgeError::InvalidComp)?;
        let t = comp
            .frame_rate
            .time_of_frame(i64::try_from(frame).unwrap_or(i64::MAX))
            .map_err(|_| BridgeError::InvalidComp)?
            .0
            .to_f64();
        let pose = match view {
            Some(view) => view.core(),
            None => lumit_render::track::camera_pose(&doc, comp, t),
        };
        let rows =
            lumit_core::camera::projection(&pose, f64::from(comp.width), f64::from(comp.height));
        Ok(rows.concat())
    }

    /// The wireframes at `frame`, projected through `view`, or through the
    /// comp's active camera when there is none (which is then left out, being
    /// the eye). `ground` adds the ground, the comp's frame and the axes from
    /// the origin. `only` names the cameras and lights to outline, and then
    /// no other layer is; with none given everything is. Asked once per
    /// frame, view or document change, never per rebuild.
    #[frb(sync)]
    pub fn wireframes(
        &self,
        frame: u64,
        view: Option<BridgeCameraPose>,
        ground: bool,
        only: Option<Vec<Uuid>>,
        media_sizes: Vec<BridgeMediaSize>,
    ) -> Result<BridgeWireframes, BridgeError> {
        use lumit_core::model::LayerKind as K;

        let doc = {
            let proj = self.project()?;
            let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
            proj.store.snapshot()
        };
        let comp = doc.comp(self.id).ok_or(BridgeError::InvalidComp)?;
        let t = comp
            .frame_rate
            .time_of_frame(i64::try_from(frame).unwrap_or(i64::MAX))
            .map_err(|_| BridgeError::InvalidComp)?
            .0
            .to_f64();
        let (w, h) = (f64::from(comp.width), f64::from(comp.height));
        let mut out = BridgeWireframes {
            layers: Vec::new(),
            cameras: Vec::new(),
            lights: Vec::new(),
            ground: Vec::new(),
            frame: Vec::new(),
            axes: Vec::new(),
        };
        let wanted = |id: Uuid| only.as_ref().is_none_or(|o| o.contains(&id));
        // Nothing to work out: no outlines and no ground.
        if !ground && only.as_ref().is_some_and(Vec::is_empty) {
            return Ok(out);
        }
        // Looking through the comp's own camera, that camera is the eye.
        let eye_layer = if view.is_none() {
            comp.active_camera(t).map(|l| l.id)
        } else {
            None
        };
        let pose = match view {
            Some(view) => view.core(),
            None => lumit_render::track::camera_pose(&doc, comp, t),
        };
        let rows = lumit_core::camera::projection(&pose, w, h);
        // Until the ground's size is known the cards are worked out against
        // a box with no edge; only their outlines on screen are kept.
        let mut seen = View {
            rows,
            eye: pose.position,
            low: (f64::MIN, f64::MIN),
            high: (f64::MAX, f64::MAX),
        };

        // What the ground has to reach, what can hide it, what drops a line
        // to it, and the outlines to draw once the box is known.
        let mut reach_to: Vec<P3> = Vec::new();
        let mut cards: Vec<Card> = Vec::new();
        let mut drops: Vec<P3> = Vec::new();
        let mut outlines: Vec<Outline> = Vec::new();

        for layer in &comp.layers {
            let live = layer.switches.visible
                && t >= layer.in_point.0.to_f64()
                && t < layer.out_point.0.to_f64();
            if !live || Some(layer.id) == eye_layer {
                continue;
            }
            // With no ground to reach or hide, only what is outlined matters.
            if !ground && !wanted(layer.id) {
                continue;
            }
            let lt = lumit_core::time::layer_time(t, layer.start_offset.0);
            let tr = &layer.transform;
            // The layer's own placement, the compositor's, then its parents,
            // walked once for the whole layer.
            let parent = {
                let context = std::sync::Arc::new(lumit_core::expression::ExpressionContext {
                    document: doc.clone(),
                    comp: Some(self.id),
                    layer: Some(layer.id),
                    comp_time: t,
                    current_depth: 0,
                    inputs: None,
                });
                lumit_render::build::parent_world_placement(comp, layer, t, context)
                    .map(|m| m.map(|c| c.map(f64::from)))
            };
            let place = |anchor: P2, scale: P2| -> Place {
                let own = lumit_render::place_matrix(
                    (
                        tr.position_x.value_at(lt) as f32,
                        tr.position_y.value_at(lt) as f32,
                    ),
                    (anchor.0 as f32, anchor.1 as f32),
                    (scale.0 as f32, scale.1 as f32),
                    tr.rotation.value_at(lt) as f32,
                    tr.position_z.value_at(lt) as f32,
                    tr.rotation_x.value_at(lt) as f32,
                    tr.rotation_y.value_at(lt) as f32,
                )
                .map(|c| c.map(f64::from));
                match &parent {
                    Some(p) => compose(p, &own),
                    None => own,
                }
            };
            match &layer.kind {
                K::Camera { options, .. } => {
                    let Some(cam) = lumit_core::track::camera_pose_of(
                        &doc,
                        comp,
                        layer,
                        t,
                        &lumit_render::track::Store,
                    ) else {
                        continue;
                    };
                    let cam = cam.pose;
                    let eye = cam.position;
                    reach_to.push(eye);
                    // In the parent's space, like the eye.
                    let poi = options
                        .two_node
                        .then(|| {
                            let context = lumit_core::model::comp_context(&doc, comp, t);
                            let solves = Some(
                                &lumit_render::track::Store
                                    as &dyn lumit_core::track::CameraSolveStore,
                            );
                            lumit_core::model::point_of_interest(comp, layer, t, &context, solves)
                        })
                        .flatten();
                    reach_to.extend(poi);
                    if !wanted(layer.id) {
                        continue;
                    }
                    let [right, up, forward] = lumit_core::camera::axes(cam.rotation_deg);
                    let at = |sx: f64, sy: f64, sz: f64| {
                        (
                            eye.0 + right.0 * sx + up.0 * sy + forward.0 * sz,
                            eye.1 + right.1 * sx + up.1 * sy + forward.1 * sz,
                            eye.2 + right.2 * sx + up.2 * sy + forward.2 * sz,
                        )
                    };
                    let (hw, hh) = (w * 0.5, h * 0.5);
                    let corners = [
                        at(-hw, -hh, cam.zoom),
                        at(hw, -hh, cam.zoom),
                        at(hw, hh, cam.zoom),
                        at(-hw, hh, cam.zoom),
                    ];
                    let mut segs = Vec::new();
                    ring(&corners, &mut segs);
                    segs.extend(corners.map(|c| (eye, c)));
                    // The body: a box behind the eye, sized to the comp so it
                    // reads at the distances the comp is seen from.
                    let (bw, bh, bd) = (w * 0.04, w * 0.03, w * 0.08);
                    let front = [
                        at(-bw, -bh, 0.0),
                        at(bw, -bh, 0.0),
                        at(bw, bh, 0.0),
                        at(-bw, bh, 0.0),
                    ];
                    let back = front.map(|p| {
                        (
                            p.0 - forward.0 * bd,
                            p.1 - forward.1 * bd,
                            p.2 - forward.2 * bd,
                        )
                    });
                    ring(&front, &mut segs);
                    ring(&back, &mut segs);
                    segs.extend(front.iter().zip(&back).map(|(a, b)| (*a, *b)));
                    if let Some(poi) = poi {
                        segs.push((eye, poi));
                    }
                    drops.push(eye);
                    outlines.push(Outline::Camera(layer.id, eye, segs));
                }
                K::Light { light } => {
                    use lumit_core::model::LightKind as L;
                    // Light from everywhere has no place to mark.
                    if light.kind == L::Ambient {
                        continue;
                    }
                    // Placed as the lighting pass places it: an aimed light
                    // faces its point of interest, its rows on top, all of it
                    // read in its parent's space.
                    let context = lumit_core::model::comp_context(&doc, comp, t);
                    let placed = lumit_core::model::placement(layer, t, &context);
                    let mut eye = lumit_core::camera::Eye {
                        position: placed.position,
                        rotation_deg: placed.rotation_deg,
                        point_of_interest: None,
                    };
                    if let Some(space) =
                        lumit_core::model::parent_space(comp, layer, t, &context, None)
                    {
                        eye = space.carry(eye);
                    }
                    let own = eye.position;
                    let poi = light
                        .two_node
                        .then(|| {
                            lumit_core::model::point_of_interest(comp, layer, t, &context, None)
                        })
                        .flatten();
                    let (rx, ry, rz) = eye.rotation_deg;
                    let m = lumit_render::place_matrix(
                        (own.0 as f32, own.1 as f32),
                        (0.0, 0.0),
                        (100.0, 100.0),
                        rz as f32,
                        own.2 as f32,
                        rx as f32,
                        ry as f32,
                    )
                    .map(|c| c.map(f64::from));
                    let at = apply(&m, (0.0, 0.0, 0.0));
                    reach_to.push(at);
                    reach_to.extend(poi);
                    if !wanted(layer.id) {
                        continue;
                    }
                    // How far the cone or the rays are drawn along the light's
                    // own z: to the point of interest, or a fifth of the comp.
                    let reach = poi
                        .map(|poi| dot(sub(poi, own), sub(poi, own)).sqrt())
                        .filter(|d| *d > 1.0)
                        .unwrap_or(w * 0.2);
                    let mut segs = Vec::new();
                    let kind = match light.kind {
                        L::Point | L::Ambient => BridgeWireLightKind::Point,
                        L::Parallel => {
                            // Every ray runs the same way, so one line shows it.
                            segs.push((at, apply(&m, (0.0, 0.0, reach))));
                            BridgeWireLightKind::Point
                        }
                        L::Spot => {
                            let half = (light.cone_angle.value_at(lt) * 0.5).clamp(0.0, 85.0);
                            // Where the feather starts, as the lighting pass
                            // has it: a share of the cone's angle, or of its
                            // cosine for a light from before the row.
                            let feather = light.cone_feather.value_at(lt).clamp(0.0, 100.0) / 100.0;
                            let inner = if light.legacy_shape {
                                let c = half.to_radians().cos();
                                (c + (1.0 - c) * feather).clamp(-1.0, 1.0).acos()
                            } else {
                                (half * (1.0 - feather)).to_radians()
                            };
                            let circle = |angle: f64| -> Vec<P3> {
                                let r = reach * angle.tan();
                                (0..48)
                                    .map(|i| {
                                        let a = f64::from(i) * std::f64::consts::TAU / 48.0;
                                        apply(&m, (r * a.cos(), r * a.sin(), reach))
                                    })
                                    .collect()
                            };
                            let edge = circle(half.to_radians());
                            ring(&edge, &mut segs);
                            ring(&circle(inner), &mut segs);
                            segs.extend(edge.iter().step_by(12).map(|p| (at, *p)));
                            segs.push((at, apply(&m, (0.0, 0.0, reach))));
                            BridgeWireLightKind::Spot
                        }
                        L::Area => {
                            let (hw, hh) = (
                                (light.size[0].value_at(lt) * 0.5).max(0.0),
                                (light.size[1].value_at(lt) * 0.5).max(0.0),
                            );
                            ring(
                                &[
                                    apply(&m, (-hw, -hh, 0.0)),
                                    apply(&m, (hw, -hh, 0.0)),
                                    apply(&m, (hw, hh, 0.0)),
                                    apply(&m, (-hw, hh, 0.0)),
                                ],
                                &mut segs,
                            );
                            BridgeWireLightKind::Area
                        }
                    };
                    drops.push(at);
                    outlines.push(Outline::Light(layer.id, kind, at, segs));
                }
                _ => {
                    if !layer.switches.three_d || layer.audio_only {
                        continue;
                    }
                    // The picture's size, and whether it is known: a text or
                    // shape box, or footage not yet probed, is the comp's
                    // size rather than its own.
                    let (nw, nh, known) = match &layer.kind {
                        // After Effects' square, which the Viewer boxes too.
                        K::Null => (100.0, 100.0, false),
                        K::Precomp { comp: inner } => doc.comp(*inner).map_or((w, h, false), |c| {
                            (f64::from(c.width), f64::from(c.height), true)
                        }),
                        K::Solid { def } => match doc.item(*def) {
                            Some(lumit_core::model::ProjectItem::Solid(s)) => {
                                (f64::from(s.width), f64::from(s.height), true)
                            }
                            _ => (w, h, false),
                        },
                        K::Footage { item, .. } => media_sizes
                            .iter()
                            .find(|m| m.item == *item)
                            .map_or((w, h, false), |m| {
                                (f64::from(m.width), f64::from(m.height), true)
                            }),
                        _ => (w, h, false),
                    };
                    let anchor = (tr.anchor_x.value_at(lt), tr.anchor_y.value_at(lt));
                    let m = place(anchor, (tr.scale_x.value_at(lt), tr.scale_y.value_at(lt)));
                    let corners = [
                        apply(&m, (0.0, 0.0, 0.0)),
                        apply(&m, (nw, 0.0, 0.0)),
                        apply(&m, (nw, nh, 0.0)),
                        apply(&m, (0.0, nh, 0.0)),
                    ];
                    reach_to.extend(corners);
                    if ground && known && !layer.is_adjustment() {
                        cards.extend(Card::new(&seen, &m, (nw, nh)));
                    }
                    if only.is_some() {
                        continue;
                    }
                    let mut segs = Vec::new();
                    ring(&corners, &mut segs);
                    if matches!(layer.kind, K::Null) {
                        let (ax, ay) = anchor;
                        let arm = nw * 0.25;
                        segs.extend(
                            [
                                ((ax - arm, ay, 0.0), (ax + arm, ay, 0.0)),
                                ((ax, ay - arm, 0.0), (ax, ay + arm, 0.0)),
                                ((ax, ay, -arm), (ax, ay, arm)),
                            ]
                            .map(|(a, b)| (apply(&m, a), apply(&m, b))),
                        );
                        drops.push(apply(&m, (ax, ay, 0.0)));
                    }
                    outlines.push(Outline::Layer(layer.id, segs));
                }
            }
        }

        // The floor runs through the comp's middle, at half its height:
        // After Effects' scene centre, and where a game camera's zero height
        // lands when its units map 1:1 onto the comp. It reaches out past
        // everything in the scene, so each drop line lands on it, and is
        // never smaller than ten squares of a tenth of the comp.
        let (cx, gy, cz) = (w * 0.5, h * 0.5, 0.0);
        let least = nice(w.max(h) / 8.0) * 10.0;
        let far = reach_to
            .iter()
            .map(|p| (p.0 - cx).abs().max((p.2 - cz).abs()) * 1.1)
            .fold(least, f64::max);
        // Lines are cut a comp's size past the ground's reach, which no
        // zoom the Viewer offers shows and float numbers hold easily.
        let edge = far + w.max(h);
        seen.low = (-edge, -edge);
        seen.high = (w + edge, h + edge);

        for outline in outlines {
            let mut lines = Vec::new();
            let segs = match &outline {
                Outline::Layer(_, segs)
                | Outline::Camera(_, _, segs)
                | Outline::Light(.., segs) => segs,
            };
            for seg in segs {
                seen.line(*seg, &mut lines);
            }
            match outline {
                Outline::Layer(layer, _) => out.layers.push(BridgeWireLayer { layer, lines }),
                Outline::Camera(layer, eye, _) => out.cameras.push(BridgeWireCamera {
                    layer,
                    eye: seen.point(eye),
                    lines,
                }),
                Outline::Light(layer, kind, at, _) => out.lights.push(BridgeWireLight {
                    layer,
                    kind,
                    at: seen.point(at),
                    lines,
                }),
            }
        }

        if ground {
            let step = nice(far / 10.0);
            let n = ((far / step).ceil() as i32).clamp(1, 20);
            let reach = f64::from(n) * step;
            for k in -n..=n {
                let across = f64::from(k) * step;
                // The two lines through the middle a little stronger.
                let weight = if k == 0 { 1.0 } else { 0.6 };
                for s in -n..n {
                    let (s0, s1) = (f64::from(s) * step, f64::from(s + 1) * step);
                    let mid = (s0 + s1) * 0.5;
                    let fade = 1.0 - (mid * mid + across * across).sqrt() / reach;
                    if fade <= 0.0 {
                        continue;
                    }
                    for seg in [
                        ((cx + s0, gy, cz + across), (cx + s1, gy, cz + across)),
                        ((cx + across, gy, cz + s0), (cx + across, gy, cz + s1)),
                    ] {
                        seen.hidden_line(&cards, seg, Some(weight * fade), &mut out.ground);
                    }
                }
            }
            // A faint line from each thing straight down (or up) to the
            // ground, so its height reads in a perspective view.
            for p in drops {
                if (p.1 - gy).abs() > 1e-6 {
                    seen.hidden_line(&cards, (p, (p.0, gy, p.2)), Some(0.5), &mut out.ground);
                }
            }
            let mut frame = Vec::new();
            ring(
                &[(0.0, 0.0, 0.0), (w, 0.0, 0.0), (w, h, 0.0), (0.0, h, 0.0)],
                &mut frame,
            );
            for seg in frame {
                seen.hidden_line(&cards, seg, None, &mut out.frame);
            }
            let arm = nice(w.max(h) / 8.0) * 2.0;
            for (k, end) in [
                (0.0, (arm, 0.0, 0.0)),
                (1.0, (0.0, arm, 0.0)),
                (2.0, (0.0, 0.0, arm)),
            ] {
                seen.hidden_line(&cards, ((0.0, 0.0, 0.0), end), Some(k), &mut out.axes);
            }
        }
        Ok(out)
    }
}
