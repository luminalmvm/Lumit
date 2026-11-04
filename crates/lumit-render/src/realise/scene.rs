//! Particulate's particles in the comp's 3D scene. The builder hands the
//! effect its layer's place in the comp, the realiser tells it how the frame
//! is seen and what 3D layers are in the way, and the picture it draws comes
//! back with one over clip w at each pixel, so the compositor can join it with
//! the 3D layers by depth as it joins a model.

use super::Realiser;
use crate::draw::CompLayerDraw;
use lumit_core::fx::effects::particulate::rows::is_scene;
use lumit_core::fx::effects::particulate::Particulate;
use lumit_core::fx::params::Params;
use lumit_core::fx::registry::EffectMetadata;
use lumit_core::model::{CameraPose, Composition, Document, EffectInstance, EffectValue, Layer};
use lumit_gpu::mesh::ModelPicture;
use std::cell::RefCell;
use std::sync::Arc;

/// How the frame a Particulate in the scene draws into is seen.
#[derive(Clone)]
pub(super) struct Frame {
    comp: (u32, u32),
    render_scale: f32,
    camera: CameraPose,
    /// Clip w of the nearest of the other 3D layers in its run, less `near`,
    /// which keeps the half floats it is laid in fine where the particles are.
    occluder: Option<wgpu::Texture>,
    near: f32,
    /// What the layer drawing now carries.
    carried: Carried,
    /// Its anti-aliasing, lights and Material options, which a model drawn as
    /// its particles takes.
    samples: u32,
    lights: Vec<lumit_gpu::fx::lighting::LightingLight>,
    material: lumit_gpu::fx::lighting::LightingMaterial,
}

/// What a Particulate in the scene drew beside its picture.
struct Joined {
    /// One over clip w at each pixel.
    depth: wgpu::Texture,
    /// The emitter in comp pixels, where the run is sorted by.
    middle: glam::Vec3,
}

/// The picture a layer's source and effects came to, how much they grew it,
/// and a model's depth ([`Realiser::layer_picture`]).
type Made = (wgpu::Texture, (f32, f32), Option<wgpu::Texture>);

thread_local! {
    // The frame being realised, innermost last.
    static FRAMES: RefCell<Vec<Frame>> = const { RefCell::new(Vec::new()) };
    // What the last Particulate in the scene drew, until its layer claims it.
    static PENDING: RefCell<Option<Joined>> = const { RefCell::new(None) };
}

/// What a layer whose Particulate is in the scene takes from the comp beyond
/// its own rows.
#[derive(Clone, Default)]
pub struct Carried {
    /// How long the shutter stays open, in seconds, when the layer blurs,
    /// and when it opens, from now.
    pub shutter: f32,
    pub opens: f32,
    /// The triangles a Layer emitter on a Model layer gives birth on, in that
    /// layer's own pixels: three corners each, the first carrying in w the
    /// area of every triangle up to and including it.
    pub surface: Option<Arc<[[f32; 4]]>>,
    /// The model its Sprite layer names, drawn at each particle, and that
    /// model's space to a box a pixel across about nought.
    pub shape: Option<(Arc<lumit_scene::ModelScene>, glam::Mat4)>,
}

/// Whether `layer` is a 3D layer with a Particulate in the scene.
#[must_use]
pub fn in_scene(layer: &Layer) -> bool {
    layer.switches.three_d && layer.effects.iter().any(is_scene)
}

/// What `layer` carries into the scene, `None` when it has nothing there.
pub(crate) fn carried(doc: &Document, comp: &Composition, layer: &Layer) -> Option<Carried> {
    if !in_scene(layer) {
        return None;
    }
    let blurs = layer.switches.motion_blur && !comp.motion_blur.sample_offsets().is_empty();
    let frame = 1.0 / comp.frame_rate.fps().max(1.0);
    let open = comp.motion_blur.shutter_angle / 360.0 * frame;
    Some(Carried {
        shutter: if blurs { open as f32 } else { 0.0 },
        opens: (comp.motion_blur.shutter_phase / 360.0 * frame) as f32,
        surface: surface(doc, comp, layer),
        shape: shape(doc, comp, layer),
    })
}

/// The surface `layer`'s Particulate in the scene gives birth on, in px@comp.
pub(crate) fn surface_of(
    doc: &Document,
    comp: &Composition,
    layer: &Layer,
) -> Option<Arc<[[f32; 4]]>> {
    in_scene(layer).then(|| surface(doc, comp, layer)).flatten()
}

/// The most triangles a surface gives birth on.
// ponytail: the triangles are gathered again each frame. Keep them with the
// model's scene if a large model shows in a profile.
const MAX_SURFACE: usize = 1 << 20;

/// The surface `layer`'s Particulate in the scene gives birth on, when its
/// Layer emitter is a Model layer, where that model is in its own layer.
fn surface(doc: &Document, comp: &Composition, layer: &Layer) -> Option<Arc<[[f32; 4]]>> {
    let e = layer.effects.iter().find(|e| is_scene(e))?;
    let src = emitter_layer(comp, e)?;
    let lumit_core::model::LayerKind::Model { item, settings } = &src.kind else {
        return None;
    };
    let scene = crate::model::scene(doc, *item)?;
    let comp_size = (comp.width as f32, comp.height as f32);
    let to_layer = crate::model::model_to_comp(settings, &scene.bounds, comp_size);
    let to_layer = glam::Mat4::from_cols_array_2d(&to_layer);
    let mut out = Vec::new();
    let mut total = 0.0;
    for (k, node) in scene.nodes.iter().enumerate() {
        let Some(mesh) = node.mesh.and_then(|m| scene.meshes.get(m)) else {
            continue;
        };
        let at = to_layer * scene.node_world(k);
        for p in &mesh.primitives {
            let corner = |i: &u32| {
                let v = p.positions.get(*i as usize)?;
                Some(at.transform_point3(glam::Vec3::from(*v)))
            };
            for tri in p.indices.chunks_exact(3).take(MAX_SURFACE - out.len() / 3) {
                let (Some(a), Some(b), Some(c)) =
                    (corner(&tri[0]), corner(&tri[1]), corner(&tri[2]))
                else {
                    continue;
                };
                total += (b - a).cross(c - a).length() * 0.5;
                out.extend([a.extend(total), b.extend(0.0), c.extend(0.0)].map(|v| v.to_array()));
            }
        }
    }
    (total > 0.0).then(|| out.into())
}

/// The model `layer`'s Particulate in the scene draws as each particle, when
/// it is in Sprite mode and its Sprite layer is a Model layer, and that model's
/// space to a box a pixel across about nought, its up axis as its layer has it.
fn shape(
    doc: &Document,
    comp: &Composition,
    layer: &Layer,
) -> Option<(Arc<lumit_scene::ModelScene>, glam::Mat4)> {
    let e = layer.effects.iter().find(|e| is_scene(e))?;
    let src = matches!(e.param("mode"), Some(EffectValue::Choice(1)))
        .then(|| e.layer_ref("sprite_layer"))
        .flatten()
        .and_then(|id| comp.layers.iter().find(|l| l.id == id))?;
    let lumit_core::model::LayerKind::Model { item, settings } = &src.kind else {
        return None;
    };
    let scene = crate::model::scene(doc, *item)?;
    let comp_size = (comp.width as f32, comp.height as f32);
    let m = crate::model::model_to_comp(settings, &scene.bounds, comp_size);
    let m = glam::Mat4::from_cols_array_2d(&m);
    let lo = m.transform_point3(scene.bounds.min.into());
    let hi = m.transform_point3(scene.bounds.max.into());
    let across = (hi - lo).abs().max_element();
    let unit = glam::Mat4::from_scale(glam::Vec3::splat(1.0 / across))
        * glam::Mat4::from_translation(-(lo + hi) / 2.0)
        * m;
    (across > 0.0).then_some((scene, unit))
}

/// The layer a Layer emitter names, if `e` is one.
fn emitter_layer<'a>(comp: &'a Composition, e: &EffectInstance) -> Option<&'a Layer> {
    matches!(e.param("shape"), Some(EffectValue::Choice(9)))
        .then(|| e.layer_ref("emitter_layer"))
        .flatten()
        .and_then(|id| comp.layers.iter().find(|l| l.id == id))
}

/// Whether a resolved op is one.
pub(crate) fn is_scene_op(op: &lumit_core::fx::params::ResolvedFx<'_>) -> bool {
    op.def.schema().match_name == <Particulate as EffectMetadata>::SCHEMA.match_name
        && Particulate::read(op.params).in_scene
}

pub(super) fn draws_in_scene(l: &CompLayerDraw) -> bool {
    l.three_d && l.fx.iter().any(|op| is_scene_op(&op))
}

/// What a Particulate in the scene is handed in place of the camera's
/// projection onto its plane: its layer's own pixels to comp pixels, the
/// placement's top three rows. `None` for any other effect, or on a 2D layer,
/// where the particles draw on the layer as before. A Layer emitter's particles
/// sit on the emitter layer's plane, stretched from this layer's frame to that
/// one's, so `size` gives each layer's own size.
pub(crate) fn placement(
    doc: &Arc<Document>,
    comp: &Composition,
    owner: Option<&Layer>,
    e: &EffectInstance,
    t_comp: f64,
    size: &dyn Fn(&Layer) -> Option<(f32, f32)>,
) -> Option<lumit_core::fx::points::Projection> {
    let owner = owner.filter(|o| o.switches.three_d && is_scene(e))?;
    let world = |l: &Layer| {
        let context = Arc::new(lumit_core::expression::ExpressionContext {
            document: doc.clone(),
            comp: Some(comp.id),
            layer: Some(l.id),
            comp_time: t_comp,
            current_depth: 0,
            inputs: None,
        });
        glam::Mat4::from_cols_array_2d(&crate::placement::world_placement(comp, l, t_comp, context))
    };
    let emitter = emitter_layer(comp, e);
    // A model's surface is already in its own layer's pixels.
    let model = emitter.filter(|l| matches!(l.kind, lumit_core::model::LayerKind::Model { .. }));
    let place = match emitter.zip(size(owner)).zip(emitter.and_then(size)) {
        Some(((src, (ow, oh)), (sw, sh))) if ow > 0.0 && oh > 0.0 => {
            world(src) * glam::Mat4::from_scale(glam::vec3(sw / ow, sh / oh, 1.0))
        }
        _ => world(model.unwrap_or(owner)),
    };
    let r = place.transpose().to_cols_array_2d();
    Some(lumit_core::fx::points::Projection {
        m: [r[0], r[1], r[2]],
    })
}

/// A Particulate in the scene, drawn through the frame's camera onto the whole
/// frame, with its distances left for its layer to claim. `None` when the
/// particles are not in the scene, or nothing is realising a frame to see them
/// in, and the effect draws on its layer as before. `op` is the effect's own,
/// its projection the layer's placement as [`placement`] made it, taken into
/// `raster` by `p`'s raster factor.
pub(crate) fn draw(
    fx: &lumit_gpu::fx::FxEngine,
    ctx: &lumit_gpu::GpuContext,
    src: &wgpu::Texture,
    raster: (u32, u32),
    sprite: Option<&wgpu::Texture>,
    op: &lumit_gpu::fx::particulate::ParticulateOp<'_>,
    p: Params<'_>,
) -> Option<wgpu::Texture> {
    let [x, y, z] = op.projection.filter(|_| Particulate::read(p).in_scene)?;
    let frame = FRAMES.with(|f| f.borrow().last().cloned())?;
    let (cw, ch) = frame.comp;
    let r = frame.render_scale.max(1e-6);
    // The placement as the builder made it, before the effect took it into
    // its raster as it would a projection onto its plane.
    let s = Particulate::px_scale_of(p).max(1e-6);
    let x = [x[0], x[1], x[2], x[3] / s];
    let y = [y[0], y[1], y[2], y[3] / s];
    let z = [z[0] * s, z[1] * s, z[2] * s, z[3]];
    let cam = crate::export::camera_mat(cw, ch, frame.camera);
    let place = glam::Mat4::from_cols_array_2d(&[x, y, z, [0.0, 0.0, 0.0, 1.0]]).transpose();
    let raster_to_clip = cam * place * glam::Mat4::from_scale(glam::Vec3::splat(1.0 / s));
    let m = raster_to_clip.transpose().to_cols_array_2d();
    // Into the frame at the render scale, foreshortened so a particle's size
    // in the raster comes out at its size on screen.
    let k = s / r;
    let projection = [
        m[0].map(|v| v * s),
        m[1].map(|v| v * s),
        m[3].map(|v| v * k),
    ];
    let at = op.emitter_pos;
    let w_middle = m[3][0] * at[0] + m[3][1] * at[1] + m[3][2] * at[2] + m[3][3];
    // A particle is as big as the layer's scale makes it.
    let scale = place
        .x_axis
        .truncate()
        .cross(place.y_axis.truncate())
        .length()
        .sqrt();
    // A blurring layer draws each disc from where it was when the shutter
    // opened to where it was when it closed, as a streak is drawn.
    let blur = frame.carried.shutter > 0.0 && op.tail_seconds == 0.0 && op.mode != 1;
    // A model's surface to give birth on, taken into the raster.
    let surface = frame.carried.surface.as_ref().map(|tris| {
        tris.iter()
            .map(|c| [c[0] * s, c[1] * s, c[2] * s, c[3]])
            .collect::<Vec<_>>()
    });
    let target = lumit_gpu::composite::scaled_size(cw, ch, r);
    let zoom = frame.camera.zoom.max(1.0);
    // The camera's blur, as the layers take it (dof.rs).
    let lens = frame.camera.dof.map_or([0.0; 3], |dof| {
        let k = lumit_core::camera::coc_per_zoom(&dof, zoom) / 2.0 * super::dof::DOF_SOFTNESS;
        [k as f32 * r, zoom as f32, dof.focus_distance as f32]
    });
    let op = lumit_gpu::fx::particulate::ParticulateOp {
        projection: Some(projection),
        shape: if surface.is_some() { 10 } else { op.shape },
        path: surface.as_deref().unwrap_or(op.path),
        tail_seconds: if blur {
            frame.carried.shutter
        } else {
            op.tail_seconds
        },
        tail_lead: if blur {
            frame.carried.opens + frame.carried.shutter
        } else {
            0.0
        },
        ..*op
    };
    let middle = place.transform_point3(glam::Vec3::from(at) / s);
    let joined = |depth| PENDING.with(|p| *p.borrow_mut() = Some(Joined { depth, middle }));
    if let Some(picture) = shapes(
        fx,
        ctx,
        src,
        raster,
        &op,
        &frame,
        place * glam::Mat4::from_scale(glam::Vec3::splat(1.0 / s)),
    ) {
        joined(picture.depth);
        return Some(picture.colour);
    }
    let (colour, depth) = fx.particulate_scene(
        ctx,
        src,
        raster,
        sprite,
        &op,
        &lumit_gpu::fx::particulate_scene::SceneView {
            target,
            k,
            soft: 1.0 / (zoom as f32 * r),
            occluder: frame.occluder.as_ref(),
            near: frame.near,
            fallback: if w_middle > 0.0 { 1.0 / w_middle } else { 0.0 },
            lens,
            scale,
            blur,
        },
    );
    joined(depth);
    Some(colour)
}

/// The most models a Particulate draws as its particles.
// ponytail: the particles are read back to place the copies, which waits on
// the card once a frame. Placing them on the card is the way up.
const MAX_SHAPES: usize = 20_000;

/// The particles of `op` each drawn as the model the frame carries, through
/// the mesh pass, lit by the layer's lights. `place` takes a particle in the
/// raster to comp pixels. `None` when there is no model, or no particle.
fn shapes(
    fx: &lumit_gpu::fx::FxEngine,
    ctx: &lumit_gpu::GpuContext,
    src: &wgpu::Texture,
    (w, h): (u32, u32),
    op: &lumit_gpu::fx::particulate::ParticulateOp<'_>,
    frame: &Frame,
    place: glam::Mat4,
) -> Option<ModelPicture> {
    use glam::{Mat4, Vec3};
    let (model, unit) = frame.carried.shape.as_ref()?;
    let (count, words) = fx.particulate_copies(ctx, src, (w, h), op, MAX_SHAPES as u32)?;
    // Positions first, then each one's size, turn, colour and id.
    let n = (op.cap as usize).min(MAX_SHAPES);
    let word = |i: usize| words.get(i).copied().unwrap_or(0);
    let float = |i: usize| f32::from_bits(word(i));
    let halves = |i: usize| {
        let v = word(i);
        [v & 0xffff, v >> 16].map(|b| lumit_gpu::fx::common::f16_to_f32(b as u16))
    };
    let copies: Vec<lumit_gpu::mesh::ModelCopy> = (0..(count as usize).min(n))
        .map(|i| {
            let at = Vec3::new(float(i * 3), float(i * 3 + 1), float(i * 3 + 2));
            let [r, g] = halves(5 * n + 2 * i);
            let [b, a] = halves(5 * n + 2 * i + 1);
            // Each one turns about its own axis, picked by its id.
            let id = word(7 * n + 2 * i).wrapping_mul(0x9e37_79b9);
            let spin = (id & 0xffff) as f32 / 65_536.0 * std::f32::consts::TAU;
            let up = (id >> 16) as f32 / 65_536.0 * 2.0 - 1.0;
            let round = (1.0 - up * up).max(0.0).sqrt();
            let axis = Vec3::new(round * spin.cos(), round * spin.sin(), up);
            // Shrunk as it fades, so it goes rather than pops.
            let size = float(3 * n + i) * a.clamp(0.0, 1.0);
            lumit_gpu::mesh::ModelCopy {
                place: (place
                    * Mat4::from_translation(at)
                    * Mat4::from_axis_angle(axis, float(4 * n + i))
                    * Mat4::from_scale(Vec3::splat(size)))
                .to_cols_array_2d(),
                tint: [r, g, b, a],
            }
        })
        .collect();
    if copies.is_empty() {
        return None;
    }
    let eye = frame.camera.position;
    let view = lumit_gpu::mesh::ModelView {
        comp: frame.comp,
        render_scale: frame.render_scale,
        samples: frame.samples,
        camera: crate::export::camera_mat(frame.comp.0, frame.comp.1, frame.camera),
        model_to_comp: *unit,
        shutter: &[],
        lights: &frame.lights,
        material: lumit_gpu::fx::lighting::LightingMaterial {
            eye: [eye.0 as f32, eye.1 as f32, eye.2 as f32],
            ..frame.material
        },
        instances: &copies,
        casters: None,
        maps: None,
        display: crate::model::display(),
        environment: None,
        looks: &[],
        behind: None,
    };
    fx.model(ctx, model, &view).ok()
}

/// What `l`'s Particulate in the scene drew beside its picture: its depth,
/// and its emitter in comp pixels. Anything left over from a stack that is
/// not `l`'s goes.
pub(super) fn claim(l: &CompLayerDraw) -> Option<(wgpu::Texture, glam::Vec3)> {
    let joined = PENDING.with(|p| p.borrow_mut().take())?;
    draws_in_scene(l).then_some((joined.depth, joined.middle))
}

/// While it is held, a Particulate in the scene draws into its frame.
pub(super) struct Seen;

impl Seen {
    pub(super) fn enter(frame: &Frame, l: &CompLayerDraw) -> Self {
        let frame = Frame {
            carried: l.in_scene.clone().unwrap_or_default(),
            lights: l.lights.clone(),
            material: l.material,
            ..frame.clone()
        };
        FRAMES.with(|f| f.borrow_mut().push(frame));
        Seen
    }
}

impl Drop for Seen {
    fn drop(&mut self) {
        FRAMES.with(|f| f.borrow_mut().pop());
    }
}

impl Realiser<'_> {
    /// How a Particulate in the scene among `layers` sees the frame, and the
    /// other 3D layers in its run made first, so the depth they lay is what
    /// the particles meet. The segment takes those pictures rather than
    /// making them again. Nothing for a segment without one.
    pub(super) fn scene_frame(
        &self,
        camera: Option<CameraPose>,
        (width, height): (u32, u32),
        layers: &[CompLayerDraw],
    ) -> (Option<Frame>, Vec<Option<Made>>) {
        let Some(first) = layers.iter().position(draws_in_scene) else {
            return (None, Vec::new());
        };
        // The 3D layers either side of it, up to a 2D layer, are its run.
        let in_run = |l: &CompLayerDraw| l.three_d && !l.ends_run;
        let start = layers[..first]
            .iter()
            .rposition(|l| !in_run(l))
            .map_or(0, |i| i + 1);
        let end = layers[first..]
            .iter()
            .position(|l| !in_run(l))
            .map_or(layers.len(), |i| first + i);
        let pose = camera.unwrap_or_else(|| {
            lumit_core::camera::default_pose(f64::from(width), f64::from(height))
        });
        let cam = crate::export::camera_mat(width, height, pose);
        // Distances are counted from the particles' own layer.
        let own = &layers[first];
        let near = (cam * glam::vec4(own.position.0, own.position.1, own.z, 1.0)).w;
        let mut made: Vec<Option<Made>> = layers.iter().map(|_| None).collect();
        for (slot, l) in made.iter_mut().zip(layers).take(end).skip(start) {
            if !draws_in_scene(l) && !super::shadow_only(l) {
                *slot = Some(self.layer_picture(
                    l,
                    camera,
                    (width, height),
                    (None, None, None),
                    &mut None,
                ));
            }
        }
        let occluder = camera.map(|_| {
            let placed: Vec<_> = made
                .iter()
                .zip(layers)
                .filter_map(|(m, l)| m.as_ref().map(|(t, g, _)| super::placed_in_3d(t, l, *g)))
                .collect();
            let models: Vec<ModelPicture> = made
                .iter()
                .flatten()
                .filter_map(|(colour, _, depth)| {
                    Some(ModelPicture {
                        colour: colour.clone(),
                        depth: depth.clone()?,
                        holdout: false,
                    })
                })
                .collect();
            self.compositor.composite_depth_joined(
                &self.ctx,
                width,
                height,
                &[&placed],
                cam,
                1.0,
                near,
                self.render_scale,
                None,
                &models,
            )
        });
        let frame = Frame {
            comp: (width, height),
            render_scale: self.render_scale,
            camera: pose,
            occluder,
            near,
            carried: Carried::default(),
            samples: self.samples,
            lights: Vec::new(),
            material: Default::default(),
        };
        (Some(frame), made)
    }

    /// Particles in the scene lit by the comp's lights where the layer takes
    /// them, each pixel where `depth` puts it, and shaded by `casters`. They
    /// were drawn through the camera, so nothing lights them as a plane or
    /// blurs them as one.
    // ponytail: each pixel is lit as its nearest particle, so in a dense cloud
    // the ones behind take the front one's light and shadow. Lighting each
    // particle in its own draw is the way up (item 11).
    pub(super) fn scene_lit(
        &self,
        l: &CompLayerDraw,
        tex: wgpu::Texture,
        depth: &wgpu::Texture,
        camera: Option<CameraPose>,
        (cw, ch): (u32, u32),
        casters: Option<&lumit_gpu::fx::lighting::ShadowCasters>,
    ) -> wgpu::Texture {
        // A model drawn as the particles was lit as it was drawn.
        let shaped = l.in_scene.as_ref().is_some_and(|c| c.shape.is_some());
        if l.lights.is_empty() || shaped {
            return tex;
        }
        let pose = camera
            .unwrap_or_else(|| lumit_core::camera::default_pose(f64::from(cw), f64::from(ch)));
        // Comp pixels back from a pixel and its clip w, the camera undone.
        let cam = crate::export::camera_mat(cw, ch, pose);
        let rows = cam.transpose().to_cols_array_2d();
        let a = glam::Mat3::from_cols_array_2d(&[
            [rows[0][0], rows[1][0], rows[3][0]],
            [rows[0][1], rows[1][1], rows[3][1]],
            [rows[0][2], rows[1][2], rows[3][2]],
        ]);
        let back = a.inverse();
        if !back.is_finite() {
            return tex;
        }
        let turn = -(back * glam::vec3(rows[0][3], rows[1][3], rows[3][3]));
        let per = cw as f32 / tex.width().max(1) as f32;
        let eye = pose.position;
        let op = lumit_gpu::fx::lighting::LightingOp {
            origin: turn.to_array(),
            du: (back.col(0) * per).to_array(),
            dv: (back.col(1) * per).to_array(),
            normal: back.col(2).to_array(),
            material: lumit_gpu::fx::lighting::LightingMaterial {
                eye: [eye.0 as f32, eye.1 as f32, eye.2 as f32],
                specular: 0.0,
                ..l.material
            },
            lights: l.lights.clone(),
            own_caster: None,
        };
        self.fx.lighting_scene(&self.ctx, &tex, depth, &op, casters)
    }
}
