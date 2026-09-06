// Lens flare (docs/08-EFFECTS.md §3.27, docs/impl/lens-flare.md).
// See fx_lens_flare_trace.wgsl for the pass map; this file is split from it
// because each stage binds a different resource set.

// The combine stage.

// The layout fx_lens_flare_detect.wgsl fills and the trace reads. This pass
// reads the emitting extent too: the starburst of an extended source
// is the point sprite convolved with the source, and the stamp grid below is
// that convolution.
struct Light {
    pos_x: f32,
    pos_y: f32,
    r: f32,
    g: f32,
    b: f32,
    ext_x: f32,
    ext_y: f32,
    _pad2: f32,
};

struct CombineParams {
    w: f32,
    h: f32,
    fw: f32,
    fh: f32,
    intensity: f32,
    sb_intensity: f32,
    sb_half: f32,
    squeeze: f32,
    fscale: f32,
    mix_amt: f32,
    light_count: u32,
    // How the flare element combines with the layer under it: an index into
    // lumit_core::fx::lens_flare::BLEND_OPTIONS -- 0 Normal,
    // 1 Add, 2 Screen, 3 Multiply, 4 Overlay, 5 Soft light, 6 Hard light,
    // 7 Lighten, 8 Darken, 9 Difference, 10 Exclusion, 11 Subtract,
    // 12 Divide. Only reached while live -- the neutral early-out above the
    // flare maths returns first, so every mode keeps the Intensity-0 /
    // Mix-0 passthroughs bit-exact.
    blend: u32,
};

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var flare_tex: texture_2d<f32>;
@group(0) @binding(2) var sb_tex: texture_2d<f32>;
@group(0) @binding(3) var dst_tex: texture_storage_2d<rgba16float, write>;
@group(0) @binding(4) var<uniform> cp: CombineParams;
@group(0) @binding(5) var<storage, read> lights: array<Light>;

// Bilinear tap of an rgba texture's rgb — ZERO outside the texture: a
// squeeze or scale below 1 asks for coordinates past the flare buffer, and
// clamp-addressing repeated the edge row outward. Half a texel of grace
// keeps the true border texels filtered.
fn tap_rgb(tex: texture_2d<f32>, fx_in: f32, fy_in: f32, dims: vec2<i32>) -> vec3<f32> {
    let fx = fx_in;
    let fy = fy_in;
    if (fx < -0.5 || fy < -0.5 || fx > f32(dims.x) - 0.5 || fy > f32(dims.y) - 0.5) {
        return vec3<f32>(0.0);
    }
    let x0 = clamp(i32(floor(fx)), 0, dims.x - 1);
    let y0 = clamp(i32(floor(fy)), 0, dims.y - 1);
    let x1 = min(x0 + 1, dims.x - 1);
    let y1 = min(y0 + 1, dims.y - 1);
    let tx = clamp(fx - floor(fx), 0.0, 1.0);
    let ty = clamp(fy - floor(fy), 0.0, 1.0);
    let a = textureLoad(tex, vec2<i32>(x0, y0), 0).rgb * (1.0 - tx)
        + textureLoad(tex, vec2<i32>(x1, y0), 0).rgb * tx;
    let b = textureLoad(tex, vec2<i32>(x0, y1), 0).rgb * (1.0 - tx)
        + textureLoad(tex, vec2<i32>(x1, y1), 0).rgb * tx;
    return a * (1.0 - ty) + b * ty;
}

// Field-angle slices in the starburst atlas -- the WGSL spelling of
// lumit_core::fx::lens_flare::STARBURST_FIELDS, pinned against it by test.
// The atlas is ONE texture, STARBURST_RES wide by STARBURST_RES * F tall,
// slice 0 (on-axis) at the top.
const STARBURST_FIELDS: u32 = 8u;

// The starburst stamp grid -- lumit_core's SB_MIN_EXTENT / SB_STAMPS,
// pinned against them by test. The ghosts integrate their source per ray, but
// the starburst is a BAKED sprite and cannot; it is shift-invariant, though,
// so the starburst of an extended source is exactly the point sprite
// convolved with the source. These stamps are that convolution in quadrature
// form: a fixed 3x3 grid spanning +/-extent, each carrying 1/(nx*ny) of the
// light. A source narrower than SB_MIN_EXTENT of the raster is one stamp at
// full strength on its own position -- bit-identical to what a point always
// drew, which a test pins.
const SB_MIN_EXTENT: f32 = 0.004;
const SB_STAMPS: u32 = 3u;

// Stamps on one axis, and where stamp i of n sits across it in units of the
// half-extent (-1 .. +1; 0 when there is only one). lumit_core's
// `starburst_stamp_grid` / `starburst_stamp_offset`.
fn sb_stamp_count(ext: f32) -> u32 {
    if (ext >= SB_MIN_EXTENT) {
        return SB_STAMPS;
    }
    return 1u;
}

fn sb_stamp_offset(i: u32, n: u32) -> f32 {
    if (n <= 1u) {
        return 0.0;
    }
    return f32(i) / f32(n - 1u) * 2.0 - 1.0;
}

// Where a light sits in the field: (field fraction, azimuth) -- the twin of
// lumit_core's `starburst_field`. Offsets in sensor mm follow `dir_of`'s
// convention (half the 36 mm sensor width is 18, the y fraction scaled by
// the raster aspect), over the sensor's half-diagonal. The azimuth is taken
// on the RASTER's offsets, y down, because it turns the sprite in raster
// space; sensor y is up, so this mirrors the true meridional angle, which
// the cat's-eye's own symmetry makes invisible.
fn starburst_field(px: f32, py: f32, aspect: f32) -> vec2<f32> {
    let half_w = 18.0;
    let dx = px - 0.5;
    let dy = py - 0.5;
    let x_mm = 2.0 * dx * half_w;
    let y_mm = 2.0 * dy * aspect * half_w;
    let half_diag = 0.5 * sqrt(36.0 * 36.0 + 24.0 * 24.0);
    let frac = clamp(sqrt(x_mm * x_mm + y_mm * y_mm) / half_diag, 0.0, 1.0);
    return vec2<f32>(frac, atan2(dy, dx));
}

// Bilinear tap of one slice of the starburst atlas at unit (u, v) -- the
// same arithmetic lumit_core's combine uses. The slice's own rows are the
// only ones read (`base` offsets, the clamp keeps a malformed atlas in
// bounds), so slice s can never bleed into s +/- 1.
fn tap_sb(u: f32, v: f32, sw: i32, rows: i32, slice: i32) -> vec3<f32> {
    let fx = u * f32(sw - 1);
    let fy = v * f32(rows - 1);
    let x0 = clamp(i32(floor(fx)), 0, sw - 1);
    let y0 = clamp(i32(floor(fy)), 0, rows - 1);
    let x1 = min(x0 + 1, sw - 1);
    let y1 = min(y0 + 1, rows - 1);
    let tx = clamp(fx - floor(fx), 0.0, 1.0);
    let ty = clamp(fy - floor(fy), 0.0, 1.0);
    let sdims = vec2<i32>(textureDimensions(sb_tex));
    let base = slice * rows;
    let ry0 = clamp(base + y0, 0, sdims.y - 1);
    let ry1 = clamp(base + y1, 0, sdims.y - 1);
    let a = textureLoad(sb_tex, vec2<i32>(x0, ry0), 0).rgb * (1.0 - tx)
        + textureLoad(sb_tex, vec2<i32>(x1, ry0), 0).rgb * tx;
    let b = textureLoad(sb_tex, vec2<i32>(x0, ry1), 0).rgb * (1.0 - tx)
        + textureLoad(sb_tex, vec2<i32>(x1, ry1), 0).rgb * tx;
    return a * (1.0 - ty) + b * ty;
}

// W3C soft-light D(d) helper (== the compositor's and Echo's).
fn flare_soft_light_d(d: vec4<f32>) -> vec4<f32> {
    let poly = ((16.0 * d - 12.0) * d + 4.0) * d;
    return select(sqrt(d), poly, d <= vec4<f32>(0.25));
}

// Combine the flare element `e` with the layer under it `d`, both
// premultiplied linear RGBA, per channel on all four -- the exact
// arithmetic order lumit_core::fx::lens_flare::flare_blend uses, so the two
// agree bit-for-bit (docs/08 1.6). Normal ignores `d` and returns the
// element on its opaque black background.
fn flare_blend(mode: u32, d: vec4<f32>, e: vec4<f32>) -> vec4<f32> {
    let one = vec4<f32>(1.0);
    if (mode == 0u) {
        return vec4<f32>(e.rgb, 1.0); // Normal: the element replaces the layer
    } else if (mode == 1u) {
        return d + e; // Add
    } else if (mode == 2u) {
        return d + e - d * e; // Screen
    } else if (mode == 3u) {
        return d * e; // Multiply
    } else if (mode == 4u) {
        // Overlay = hard light with the LAYER as the switch.
        let lo = 2.0 * d * e;
        let hi = one - 2.0 * (one - d) * (one - e);
        return select(hi, lo, d <= vec4<f32>(0.5));
    } else if (mode == 5u) {
        // Soft light (W3C), source = the element, backdrop = the layer.
        let darkened = d - (one - 2.0 * e) * d * (one - d);
        let lightened = d + (2.0 * e - one) * (flare_soft_light_d(d) - d);
        return select(lightened, darkened, e <= vec4<f32>(0.5));
    } else if (mode == 6u) {
        // Hard light: the element is the switch.
        let lo = 2.0 * d * e;
        let hi = one - 2.0 * (one - d) * (one - e);
        return select(hi, lo, e <= vec4<f32>(0.5));
    } else if (mode == 7u) {
        return max(d, e); // Lighten
    } else if (mode == 8u) {
        return min(d, e); // Darken
    } else if (mode == 9u) {
        return abs(d - e); // Difference
    } else if (mode == 10u) {
        return d + e - 2.0 * d * e; // Exclusion
    } else if (mode == 11u) {
        return max(d - e, vec4<f32>(0.0)); // Subtract
    }
    return max(d / max(e, vec4<f32>(1e-6)), vec4<f32>(0.0)); // Divide
}

@compute @workgroup_size(8, 8)
fn combine(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src_tex));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src_tex, xy, 0);
    if (cp.intensity <= 0.0 || cp.mix_amt <= 0.0) {
        textureStore(dst_tex, xy, o);
        return;
    }
    // Whole-flare Scale plus the anamorphic squeeze (x only), both about
    // the frame centre (== lens_flare::cpu_combine).
    let cx = cp.w / 2.0;
    let cyc = cp.h / 2.0;
    let sx = cx + (f32(xy.x) + 0.5 - cx) / (cp.squeeze * cp.fscale);
    let sy = cyc + (f32(xy.y) + 0.5 - cyc) / cp.fscale;
    // Flare buffer tap (resolution-relative: Draft renders it half-size).
    // The buffer may be PADDED past the base cp.fw × cp.fh for Squeeze or
    // Scale under 1, geometry centred — the padding only adds the
    // constant border offset, zero when unpadded.
    let fdims = vec2<i32>(textureDimensions(flare_tex));
    let f = tap_rgb(
        flare_tex,
        sx / cp.w * cp.fw - 0.5 + (f32(fdims.x) - cp.fw) / 2.0,
        sy / cp.h * cp.fh - 0.5 + (f32(fdims.y) - cp.fh) / 2.0,
        fdims,
    );
    // One starburst sprite per STAMP: anchored on the stamp, sized by Scale,
    // stretched by the squeeze, tinted by its share of the light -- and
    // turned and blended across the field-angle slices at the stamp's OWN
    // position, so a smeared starburst near the frame edge leans a little
    // differently at each end of itself, which is the physical picture. A
    // point light is one stamp on its own position at full strength.
    var sb = vec3<f32>(0.0);
    if (cp.sb_intensity > 0.0 && cp.sb_half > 0.0) {
        let sdims = vec2<i32>(textureDimensions(sb_tex));
        let rows = max(sdims.y / i32(STARBURST_FIELDS), 1);
        // A stamp can only contribute where |rot| is inside the sprite, and
        // the rotation preserves length -- so this rejects, before any trig,
        // exactly the stamps the u/v test below would have rejected anyway.
        // The CPU twin has no need of it: it works its stamps out once per
        // frame where this shader redoes them per pixel.
        let reach2 = cp.sb_half * cp.sb_half * (cp.squeeze * cp.squeeze + 1.0);
        for (var li = 0u; li < cp.light_count; li = li + 1u) {
            let light = lights[li];
            if (light.r <= 0.0 && light.g <= 0.0 && light.b <= 0.0) {
                continue;
            }
            let nx = sb_stamp_count(light.ext_x);
            let ny = sb_stamp_count(light.ext_y);
            // The share folded into the colour, not applied after the sprite:
            // the CPU twin does it here too, so the two agree op for op.
            let rgb = vec3<f32>(light.r, light.g, light.b) * (1.0 / f32(nx * ny));
            for (var iy = 0u; iy < ny; iy = iy + 1u) {
                for (var ix = 0u; ix < nx; ix = ix + 1u) {
                    let sp_x = light.pos_x + sb_stamp_offset(ix, nx) * light.ext_x;
                    let sp_y = light.pos_y + sb_stamp_offset(iy, ny) * light.ext_y;
                    let rel_x = f32(xy.x) + 0.5 - sp_x * cp.w;
                    let rel_y = f32(xy.y) + 0.5 - sp_y * cp.h;
                    if (rel_x * rel_x + rel_y * rel_y > reach2) {
                        continue;
                    }
                    // Turn the sprite so the baked +x cat's-eye lean points
                    // along the stamp's own radial direction (rotation by
                    // -azimuth).
                    let fa = starburst_field(sp_x, sp_y, cp.h / cp.w);
                    let ca = cos(fa.y);
                    let sa = sin(fa.y);
                    let rot_x = rel_x * ca + rel_y * sa;
                    let rot_y = -rel_x * sa + rel_y * ca;
                    let u = rot_x / (cp.sb_half * cp.squeeze) * 0.5 + 0.5;
                    let v = rot_y / cp.sb_half * 0.5 + 0.5;
                    if (u < 0.0 || u > 1.0 || v < 0.0 || v > 1.0) {
                        continue;
                    }
                    // The two slices bracketing this stamp's field fraction.
                    let s = fa.x * f32(STARBURST_FIELDS - 1u);
                    let s0 = min(i32(floor(s)), i32(STARBURST_FIELDS) - 1);
                    let s1 = min(s0 + 1, i32(STARBURST_FIELDS) - 1);
                    let ts = s - f32(s0);
                    let sprite = tap_sb(u, v, sdims.x, rows, s0) * (1.0 - ts)
                        + tap_sb(u, v, sdims.x, rows, s1) * ts;
                    sb = sb + sprite * cp.sb_intensity * rgb;
                }
            }
        }
    }
    let add = (f + sb) * cp.intensity;
    let luma = 0.2126 * add.r + 0.7152 * add.g + 0.0722 * add.b;
    // The flare element: the light this frame drew, with the
    // coverage that light implies as its alpha. Blend it with the layer,
    // then saturate alpha at 1 -- Add reduces to o + add with alpha
    // min(o.a + luma, 1), the pre-menu behaviour exactly.
    var flared = flare_blend(cp.blend, o, vec4<f32>(add, luma));
    flared.a = min(flared.a, 1.0);
    let outv = o * (1.0 - cp.mix_amt) + flared * cp.mix_amt;
    textureStore(dst_tex, xy, outv);
}
