// Depth points (fx/depth_points.rs): one quad a pixel, placed in the vertex
// shader from the picture and its depth, so there is no per-point memory. They
// are opaque, so the depth test sorts them and nothing needs blending.

struct View {
    // Where a point lands is `origin + k * (along + across * x + down * y)`, as
    // (X, Y, W), comp pixels being (X / W, Y / W) and W its clip w. k is how far
    // along its ray from the source camera it is, 1 on the layer's plane.
    origin: vec4<f32>,
    along: vec4<f32>,
    across: vec4<f32>,
    down: vec4<f32>,
    // The frame drawn into, px, the render scale, and half a point on the
    // layer's plane, comp pixels.
    frame: vec4<f32>,
    // x: Depth, y: Density, z: 1 when there is a depth to read, w: 1 to invert it.
    look: vec4<f32>,
    // x and y: the scale and offset that steady the depth.
    steady: vec4<f32>,
    // The camera's depth of field: x the blur per unit of `|distance - focus| /
    // clip w`, nought for none, y the zoom, z the focus distance.
    lens: vec4<f32>,
    // x: columns, y: the step between them, raster pixels.
    grid: vec4<u32>,
};

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var picture: texture_2d<f32>;
@group(0) @binding(2) var depth: texture_2d<f32>;

struct Vs {
    @builtin(position) clip: vec4<f32>,
    @location(0) @interpolate(flat) colour: vec4<f32>,
    // xy: the point's middle, px, z: half its width, px, w: one over clip w.
    @location(1) @interpolate(flat) seen: vec4<f32>,
};

// When this pixel joins as Density rises, 0 to 1. A quarter of the pixels, the
// ones on every other row and column, come in the first quarter of the range,
// and so on down, with a hash spreading each level across its own part. Each
// pixel keeps its number, so points never move as more arrive.
fn dp_joins(x: u32, y: u32) -> f32 {
    let level = firstTrailingBit(x | y | 0x8000u);
    return (0.25 + 0.75 * dp_hash(dp_mix(x, y))) / f32(1u << (2u * level));
}

fn dp_mix(x: u32, y: u32) -> u32 {
    return ((x * 73856093u) ^ (y * 19349663u)) * 2654435769u;
}

// A number from 0 to 1 out of the top of a mixed word.
fn dp_hash(mixed: u32) -> f32 {
    return f32(mixed >> 8u) / 16777216.0;
}

@vertex
fn dp_vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) i: u32) -> Vs {
    var out: Vs;
    // Off the frame, which is how a point is left out.
    out.clip = vec4<f32>(2.0, 2.0, 0.0, 1.0);
    let x = (i % view.grid.x) * view.grid.y;
    let y = (i / view.grid.x) * view.grid.y;
    if (view.look.y < 1.0 && dp_joins(x, y) >= view.look.y) {
        return out;
    }
    let at = vec2<i32>(i32(x), i32(y));
    let colour = textureLoad(picture, at, 0);
    if (colour.a <= 0.0) {
        return out;
    }
    // Nearer is brighter, and what a depth model reads goes as one over the
    // distance, so the spread is even in that and not in the distance.
    var d = 0.5;
    if (view.look.z > 0.5) {
        d = dot(textureLoad(depth, at, 0).rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
        d = clamp(view.steady.x * d + view.steady.y, 0.0, 1.0);
        if (view.look.w > 0.5) {
            d = 1.0 - d;
        }
    }
    let k = 1.0 / (1.0 + view.look.x * (d - 0.5));
    let p = view.origin.xyz + k * (view.along.xyz + view.across.xyz * (f32(x) + 0.5)
        + view.down.xyz * (f32(y) + 0.5));
    if (p.z <= 0.0) {
        return out;
    }
    let scale = view.frame.z;
    var middle = p.xy / p.z * scale;
    // Never less than a pixel, or far points would fall between them, and never a
    // disc that fills the view from one near point.
    var half = clamp(view.frame.w * k * scale / p.z, 0.5, 32.0 * scale);
    if (view.lens.x > 0.0) {
        // Out of focus a point lands somewhere in its circle of confusion, the
        // same place every frame, so it costs nothing and is opaque like the rest.
        let blur = view.lens.x * abs(view.lens.y * p.z - view.lens.z) / p.z;
        let mixed = dp_mix(x, y);
        let turn = dp_hash(mixed) * 6.2831853;
        let out = sqrt(dp_hash(mixed * 2654435769u)) * blur;
        middle += vec2<f32>(cos(turn), sin(turn)) * out;
        // Wide enough that scattering them leaves no gaps to see through.
        if (blur > 1.0) {
            half = max(half, 1.5);
        }
    }
    let corner = vec2<f32>(f32(vi & 1u) * 2.0 - 1.0, f32((vi >> 1u) & 1u) * 2.0 - 1.0);
    let px = middle + corner * half;
    let iw = 1.0 / p.z;
    out.clip = vec4<f32>(
        px.x / view.frame.x * 2.0 - 1.0,
        1.0 - px.y / view.frame.y * 2.0,
        // Nearer is larger, as the compositor's depth goes.
        iw / (1.0 + iw),
        1.0,
    );
    out.colour = colour;
    out.seen = vec4<f32>(middle, half, iw);
    return out;
}

struct Fs {
    @location(0) colour: vec4<f32>,
    @location(1) inv_w: vec4<f32>,
};

@fragment
fn dp_fs(in: Vs) -> Fs {
    // Square at a pixel or two, so they tile with no gaps, and round beyond.
    if (in.seen.z > 1.0) {
        let off = (in.clip.xy - in.seen.xy) / in.seen.z;
        if (dot(off, off) > 1.0) {
            discard;
        }
    }
    return Fs(in.colour, vec4<f32>(in.seen.w, 0.0, 0.0, 1.0));
}
