
// The Custom shader's host epilogue (docs/impl/custom-shader.md §1.3, §2.3).
// Constant text the user cannot remove: the entry point that bounds-checks,
// builds `uv`, calls the one function the contract asks for, sanitises what
// comes back, applies the host-uniform Mix and stores.
//
// The sanitise is a trust boundary rather than a nicety. A NaN written into the
// working texture is read by the compositor, by every effect above this one, by
// the scopes and by the exporter, and one poisoned pixel becomes a black
// composition three effects later.
@compute @workgroup_size(8, 8)
fn lumit_shade(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let uv = (vec2<f32>(xy) + vec2<f32>(0.5, 0.5)) / vec2<f32>(size);
    var c = shade(uv);
    // NaN: the only value that is not equal to itself.
    c = select(c, vec4<f32>(0.0, 0.0, 0.0, 0.0), c != c);
    // +/-Inf, without clamping any picture a person could have meant.
    c = clamp(c, vec4<f32>(-3.4e38), vec4<f32>(3.4e38));
    let o = textureLoad(src, xy, 0);
    textureStore(dst, xy, mix(o, c, lumit.mix_amt));
}
