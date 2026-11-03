// An Environment light as the passes that are lit by it read it
// (environment.rs): its colour and turn, and its two cubes passed in, since
// each pass binds them where it has room. Prepended to mesh.wgsl and
// fx_lighting.wgsl, as WGSL has no modules.

struct Environment {
    // rgb = the light's colour with its intensity in it, w = 1 when there
    // is one.
    colour: vec4<f32>,
    // Comp directions into the picture's, a column each. turn0.w = the
    // specular cube's last level.
    turn0: vec4<f32>,
    turn1: vec4<f32>,
    turn2: vec4<f32>,
};

// Where a direction in the picture's own space falls on the panorama:
// across is all the way round, a quarter in straight ahead, as Blender has
// it; down is from straight up to straight down, up being -y as in the comp.
fn panorama_uv(d: vec3<f32>) -> vec2<f32> {
    let around = 0.25 + atan2(d.x, d.z) / 6.28318531;
    return vec2<f32>(around, acos(clamp(-d.y, -1.0, 1.0)) / 3.14159265);
}

fn env_turn(e: Environment, d: vec3<f32>) -> vec3<f32> {
    return mat3x3<f32>(e.turn0.xyz, e.turn1.xyz, e.turn2.xyz) * d;
}

// What a matte surface facing `n` takes from every way at once.
fn env_irradiance(cube: texture_cube<f32>, s: sampler, e: Environment, n: vec3<f32>) -> vec3<f32> {
    return textureSampleLevel(cube, s, env_turn(e, n), 0.0).rgb * e.colour.rgb;
}

// What a surface reflects along `r`, nought rough a mirror and one matte.
fn env_radiance(
    cube: texture_cube<f32>,
    s: sampler,
    e: Environment,
    r: vec3<f32>,
    roughness: f32,
) -> vec3<f32> {
    let level = clamp(roughness, 0.0, 1.0) * e.turn0.w;
    return textureSampleLevel(cube, s, env_turn(e, r), level).rgb * e.colour.rgb;
}
