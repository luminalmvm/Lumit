//! The wireframes a Viewer draws over a 3D view (docs/impl/camera.md §6):
//! every 3D layer's rectangle, every camera's frustum and every light,
//! placed and projected here so the frontend only joins the dots.

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

/// A 3D layer's rectangle: four corners, clockwise from the top left.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeWireLayer {
    pub layer: Uuid,
    pub corners: Vec<BridgeWirePoint>,
}

/// A camera: its eye, the four corners of the comp-sized rectangle its zoom
/// in front of it, and the point of interest when it has one.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeWireCamera {
    pub layer: Uuid,
    pub eye: BridgeWirePoint,
    pub corners: Vec<BridgeWirePoint>,
    pub point_of_interest: Option<BridgeWirePoint>,
}

#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeWireLight {
    pub layer: Uuid,
    pub at: BridgeWirePoint,
}

#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeWireframes {
    pub layers: Vec<BridgeWireLayer>,
    pub cameras: Vec<BridgeWireCamera>,
    pub lights: Vec<BridgeWireLight>,
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

impl CompositionReference {
    /// The wireframes at `frame`, projected through `view`. Asked once per
    /// frame change or view change, never per rebuild.
    #[frb(sync)]
    pub fn wireframes(
        &self,
        frame: u64,
        view: BridgeCameraPose,
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
        let pose = view.core();
        let project = |p: (f64, f64, f64)| {
            let ((x, y), in_front) = lumit_core::camera::project(&pose, w, h, p);
            BridgeWirePoint { x, y, in_front }
        };
        let world = |layer: &lumit_core::model::Layer, p: [f64; 3]| -> (f64, f64, f64) {
            let context = std::sync::Arc::new(lumit_core::expression::ExpressionContext {
                document: doc.clone(),
                comp: Some(self.id),
                layer: Some(layer.id),
                comp_time: t,
                current_depth: 0,
            });
            match lumit_render::build::parent_world_placement(comp, layer, t, context) {
                Some(m) => {
                    let m = m.map(|c| c.map(f64::from));
                    (
                        m[0][0] * p[0] + m[1][0] * p[1] + m[2][0] * p[2] + m[3][0],
                        m[0][1] * p[0] + m[1][1] * p[1] + m[2][1] * p[2] + m[3][1],
                        m[0][2] * p[0] + m[1][2] * p[1] + m[2][2] * p[2] + m[3][2],
                    )
                }
                None => (p[0], p[1], p[2]),
            }
        };

        let mut out = BridgeWireframes {
            layers: Vec::new(),
            cameras: Vec::new(),
            lights: Vec::new(),
        };
        for layer in &comp.layers {
            let live = layer.switches.visible
                && t >= layer.in_point.0.to_f64()
                && t < layer.out_point.0.to_f64();
            if !live {
                continue;
            }
            let lt = lumit_core::time::layer_time(t, layer.start_offset.0);
            let tr = &layer.transform;
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
                    let [right, up, forward] = lumit_core::camera::axes(cam.rotation_deg);
                    let eye = cam.position;
                    let at = |sx: f64, sy: f64| {
                        (
                            eye.0 + right.0 * sx + up.0 * sy + forward.0 * cam.zoom,
                            eye.1 + right.1 * sx + up.1 * sy + forward.1 * cam.zoom,
                            eye.2 + right.2 * sx + up.2 * sy + forward.2 * cam.zoom,
                        )
                    };
                    let (hw, hh) = (w * 0.5, h * 0.5);
                    out.cameras.push(BridgeWireCamera {
                        layer: layer.id,
                        eye: project(eye),
                        corners: vec![
                            project(at(-hw, -hh)),
                            project(at(hw, -hh)),
                            project(at(hw, hh)),
                            project(at(-hw, hh)),
                        ],
                        point_of_interest: options.two_node.then(|| {
                            project((
                                options.point_of_interest[0].value_at(lt),
                                options.point_of_interest[1].value_at(lt),
                                options.point_of_interest[2].value_at(lt),
                            ))
                        }),
                    });
                }
                K::Light { .. } => {
                    let at = world(
                        layer,
                        [
                            tr.position_x.value_at(lt),
                            tr.position_y.value_at(lt),
                            tr.position_z.value_at(lt),
                        ],
                    );
                    out.lights.push(BridgeWireLight {
                        layer: layer.id,
                        at: project(at),
                    });
                }
                K::Null => {}
                _ => {
                    if !layer.switches.three_d || layer.audio_only {
                        continue;
                    }
                    let (nw, nh) = match &layer.kind {
                        K::Precomp { comp: inner } => doc
                            .comp(*inner)
                            .map_or((w, h), |c| (f64::from(c.width), f64::from(c.height))),
                        K::Solid { def } => match doc.item(*def) {
                            Some(lumit_core::model::ProjectItem::Solid(s)) => {
                                (f64::from(s.width), f64::from(s.height))
                            }
                            _ => (w, h),
                        },
                        K::Footage { item, .. } => media_sizes
                            .iter()
                            .find(|m| m.item == *item)
                            .map_or((w, h), |m| (f64::from(m.width), f64::from(m.height))),
                        _ => (w, h),
                    };
                    let own = lumit_render::place_matrix(
                        (
                            tr.position_x.value_at(lt) as f32,
                            tr.position_y.value_at(lt) as f32,
                        ),
                        (
                            tr.anchor_x.value_at(lt) as f32,
                            tr.anchor_y.value_at(lt) as f32,
                        ),
                        (
                            tr.scale_x.value_at(lt) as f32,
                            tr.scale_y.value_at(lt) as f32,
                        ),
                        tr.rotation.value_at(lt) as f32,
                        tr.position_z.value_at(lt) as f32,
                        tr.rotation_x.value_at(lt) as f32,
                        tr.rotation_y.value_at(lt) as f32,
                    )
                    .map(|c| c.map(f64::from));
                    let placed = |x: f64, y: f64| {
                        let p = [
                            own[0][0] * x + own[1][0] * y + own[3][0],
                            own[0][1] * x + own[1][1] * y + own[3][1],
                            own[0][2] * x + own[1][2] * y + own[3][2],
                        ];
                        project(world(layer, p))
                    };
                    out.layers.push(BridgeWireLayer {
                        layer: layer.id,
                        corners: vec![
                            placed(0.0, 0.0),
                            placed(nw, 0.0),
                            placed(nw, nh),
                            placed(0.0, nh),
                        ],
                    });
                }
            }
        }
        Ok(out)
    }
}
