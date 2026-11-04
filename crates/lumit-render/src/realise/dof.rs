//! The camera's depth of field on a 3D layer.

/// How far one pass of the camera's blur reaches at most, in texels. A layer
/// seen nearly edge on would otherwise reach across all of it.
const DOF_REACH: f32 = 512.0;

/// The layer seen from the eye, laid out as `fx_blur.wgsl` says.
type DofField = [[f32; 4]; 3];

/// How much wider the gaussian's reach is than half the circle of confusion.
/// After Effects blurs with a box that wide either side, and this gaussian is
/// the one that comes closest to its pictures.
pub(crate) const DOF_SOFTNESS: f64 = 1.37;

/// Comp pixels as the camera's eye sees them: the eye at nought, looking
/// down +z.
fn from_eye(cam: lumit_core::model::CameraPose, comp: (u32, u32), zoom: f32) -> glam::Mat4 {
    let (cw, ch) = (comp.0 as f32, comp.1 as f32);
    let frame = lumit_gpu::composite::camera_frame(
        cw,
        ch,
        zoom,
        (
            cam.position.0 as f32,
            cam.position.1 as f32,
            cam.position.2 as f32,
        ),
        (
            cam.rotation_deg.0 as f32,
            cam.rotation_deg.1 as f32,
            cam.rotation_deg.2 as f32,
        ),
    );
    // In the camera's frame the eye is at the comp centre, zoom back.
    glam::Mat4::from_translation(glam::Vec3::new(-cw * 0.5, -ch * 0.5, zoom)) * frame
}

/// Whether the camera's depth of field leaves `l` sharp at its own size,
/// read without its picture: wholly in front of the eye, and blurred under a
/// quarter of a pixel at every corner. [`dof_blur`] leaves such a layer
/// alone too.
pub(crate) fn dof_leaves_sharp(
    cam: lumit_core::model::CameraPose,
    l: &crate::draw::CompLayerDraw,
    comp: (u32, u32),
) -> bool {
    let Some(dof) = cam.dof else {
        return true;
    };
    if !l.three_d {
        return true;
    }
    let zoom = (cam.zoom as f32).max(1.0);
    let k = (lumit_core::camera::coc_per_zoom(&dof, f64::from(zoom)) / 2.0 * DOF_SOFTNESS) as f32;
    let focus = dof.focus_distance as f32;
    let mut place = glam::Mat4::from_cols_array_2d(&lumit_gpu::composite::place_matrix(
        l.position,
        l.anchor,
        l.scale,
        l.rotation_deg,
        l.z,
        l.rotation_x_deg,
        l.rotation_y_deg,
    ));
    if let Some(pre) = &l.pre {
        place = glam::Mat4::from_cols_array_2d(pre) * place;
    }
    let size = glam::vec3(l.natural_size.0, l.natural_size.1, 1.0);
    let m = from_eye(cam, comp, zoom) * place * glam::Mat4::from_scale(size);
    [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)]
        .map(|(x, y)| m.transform_point3(glam::vec3(x, y, 0.0)))
        .iter()
        .all(|c| c.z > 0.0 && k * zoom * (c.z - focus).abs() / c.z < 0.25)
}

/// Depth of field on a 3D layer (docs/impl/camera.md §5), for
/// `FxEngine::blur_field`: the layer seen from the eye, how far a pass may
/// reach, and how many texels of room the layer needs on each side for its
/// edges to blur into, with the field already moved out by that room. Each
/// pixel is blurred by its own distance from the focus plane, so a floor going
/// into the distance softens towards its far end. A parent's scale comes in
/// through the placement.
///
/// `None` for a sharp layer: no camera, no depth of field, a 2D layer, one
/// wholly behind the eye or off the frame, or one blurred under half a pixel.
pub(super) fn dof_blur(
    camera: Option<lumit_core::model::CameraPose>,
    l: &crate::draw::CompLayerDraw,
    texture: &wgpu::Texture,
    grow: (f32, f32),
    comp: (u32, u32),
    render_scale: f32,
) -> Option<(f32, DofField, f32, (u32, u32))> {
    let cam = camera?;
    let dof = cam.dof?;
    if !l.three_d {
        return None;
    }
    let zoom = (cam.zoom as f32).max(1.0);
    // The blur's reach on screen is this times zoom times how far off focus a
    // point is over its distance.
    let k = (lumit_core::camera::coc_per_zoom(&dof, f64::from(zoom)) / 2.0 * DOF_SOFTNESS) as f32;
    let focus = dof.focus_distance as f32;
    if k <= 0.0 {
        return None;
    }
    let (cw, ch) = (comp.0 as f32, comp.1 as f32);
    let m = from_eye(cam, comp, zoom) * super::placed_in_3d(texture, l, grow).world();
    let corner = m.col(3).truncate();
    let (across, down) = (m.col(0).truncate(), m.col(1).truncate());
    let corners = [
        corner,
        corner + across,
        corner + down,
        corner + across + down,
    ];
    if corners.iter().all(|c| c.z <= 0.0) {
        return None;
    }
    if corners.iter().all(|c| c.z > 0.0) {
        let seen = corners.map(|c| (c.x * zoom / c.z, c.y * zoom / c.z));
        let off = |f: &dyn Fn((f32, f32)) -> bool| seen.iter().all(|&p| f(p));
        if off(&|p| p.0 < -cw * 0.5)
            || off(&|p| p.0 > cw * 0.5)
            || off(&|p| p.1 < -ch * 0.5)
            || off(&|p| p.1 > ch * 0.5)
        {
            return None;
        }
        // The widest blur on screen is at the nearest or farthest corner. A
        // layer reaching past the eye is left to the shader, as its nearest
        // part is no corner.
        let widest = corners
            .iter()
            .map(|c| k * zoom * (c.z - focus).abs() / c.z)
            .fold(0.0, f32::max);
        if widest * render_scale < 0.5 {
            return None;
        }
    }
    let (tw, th) = (texture.width() as f32, texture.height() as f32);
    let (x, y) = (across / tw, down / th);
    // A layer blurred past the reach all over, far away or shrunk, is blurred
    // on a grid a few pixels apart instead, where it reaches far enough.
    let along = |q: glam::Vec3, s: glam::Vec3| (s.x * q.z - q.x * s.z).hypot(s.y * q.z - q.y * s.z);
    let in_front = corners.iter().all(|c| c.z > 0.0);
    let texels =
        |c: glam::Vec3, s: glam::Vec3| k * (c.z - focus).abs() * c.z / along(c, s).max(1e-9);
    let least = if in_front {
        corners
            .iter()
            .flat_map(|&c| [x, y].map(|s| texels(c, s)))
            .fold(f32::MAX, f32::min)
    } else {
        0.0
    };
    let step = if least > DOF_REACH {
        (least / 32.0).min(tw.max(th))
    } else {
        1.0
    };
    let reach = DOF_REACH.min(tw.max(th) / step);
    // The widest blur along each side is at one of its corners in front of
    // the eye, as what is behind it never shows. Never past the widest picture
    // the card takes.
    let room = |s: glam::Vec3, n: f32| {
        let widest = corners
            .iter()
            .filter(|c| c.z > 0.0)
            .map(|&c| texels(c, s))
            .fold(0.0, f32::max);
        lumit_core::largest_picture::room(widest.min(reach * step).ceil() as u32, n as u32)
    };
    let room = (room(x, tw), room(y, th));
    let corner = corner - x * room.0 as f32 - y * room.1 as f32;
    Some((
        reach,
        [
            [x.x, x.y, x.z, k],
            [y.x, y.y, y.z, focus],
            [corner.x, corner.y, corner.z, 0.0],
        ],
        step,
        room,
    ))
}
