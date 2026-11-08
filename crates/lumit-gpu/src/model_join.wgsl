// A model's picture where it joins a run of 3D layers (composite/join.rs):
// drawn full frame, each pixel at the depth the mesh pass found there, so the
// model and the run's layers cut through each other as solids do.

struct Join {
    // The full-frame quad to clip space.
    matrix: mat4x4<f32>,
    // x: opacity, y: the clip z its run is cut at, z: the clip w scene depth
    // is counted from, w: metres per clip w.
    params: vec4<f32>,
    // x: 1 to clear what a later run covers, for scene depth, y: 1 for a
    // holdout.
    flags: vec4<f32>,
};

@group(0) @binding(0) var colour: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
// One over clip w, nought where the model shows nothing.
@group(0) @binding(2) var inv_w: texture_2d<f32>;
@group(0) @binding(3) var<uniform> u: Join;
// What was under the run, which a holdout puts back.
@group(0) @binding(4) var plate: texture_2d<f32>;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_join(@builtin(vertex_index) i: u32) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
    );
    let c = corners[i];
    var out: VsOut;
    out.pos = u.matrix * vec4<f32>(c, 0.0, 1.0);
    out.uv = c;
    return out;
}

// Read exactly, never filtered: a depth halfway between two is neither.
fn inv_w_at(uv: vec2<f32>) -> f32 {
    let size = textureDimensions(inv_w);
    let xy = min(vec2<u32>(uv * vec2<f32>(size)), size - 1u);
    return textureLoad(inv_w, xy, 0).r;
}

struct Drawn {
    @location(0) colour: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

@fragment
fn fs_join(in: VsOut) -> Drawn {
    let c = textureSample(colour, samp, in.uv) * u.params.x;
    let iw = inv_w_at(in.uv);
    if (iw <= 0.0) {
        discard;
    }
    if (u.flags.y > 0.5) {
        // Its own picture, its shadows, over what was under the run.
        // ponytail: a holdout is the model's sharp outline, so a blurred
        // object in the footage isn't matched. Its motion would want a
        // coverage the depth can't carry.
        let under = textureLoad(plate, vec2<i32>(in.pos.xy), 0);
        return Drawn(c + under * (1.0 - c.a), min(u.params.y * iw, 1.0));
    }
    return Drawn(c, min(u.params.y * iw, 1.0));
}

// The model's depth laid down wherever any of it shows, before any colour,
// as composite.wgsl's fs_layer_cut does for a layer.
@fragment
fn fs_join_cut(in: VsOut) -> Drawn {
    let a = textureSample(colour, samp, in.uv).a * u.params.x;
    let iw = inv_w_at(in.uv);
    // A holdout is solid wherever it is, whatever it shows.
    if ((a <= 0.0 && u.flags.y < 0.5) || iw <= 0.0) {
        discard;
    }
    return Drawn(vec4<f32>(0.0), min(u.params.y * iw, 1.0));
}

// Scene depth, metres from the camera, as composite.wgsl's fs_layer_depth.
@fragment
fn fs_join_depth(in: VsOut) -> @location(0) vec4<f32> {
    let a = textureSample(colour, samp, in.uv).a * u.params.x;
    let iw = inv_w_at(in.uv);
    if ((a < 0.5 && u.flags.y < 0.5) || iw <= 0.0) {
        discard;
    }
    let m = select((1.0 / iw - u.params.z) * u.params.w, 65504.0, u.flags.x > 0.5);
    return vec4<f32>(m, m, m, 1.0);
}
