//! The graphics card's colour samplers against the processor's oracle
//! (docs/impl/ocio.md §7.4), the double-encode trap pinned (§5.2), and the
//! K-031 parity row in every shipped colour configuration (§7.5, docs/06 §3.3).
//!
//! **In plain terms.** `lumit-colour` works a colour transform out on the
//! processor and bakes the answers into a small table. The Viewer and the
//! export both read that table on the graphics card. This file checks that the
//! card reads it the same way the processor does — including, and especially,
//! for colours off the end of the ordinary 0–1 range, which is where a
//! wide-gamut picture keeps its most saturated colour and where a colour
//! pipeline is judged.
//!
//! Why it lives here rather than in `lumit-gpu`: that crate deliberately
//! depends on no other Lumit crate, so the oracle and the shader can only be
//! put side by side one level up.
//!
//! Skips cleanly with no graphics card, like every other GPU test.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_colour::op::{CdlParams, Direction, Op};
use lumit_colour::{bake, Artefact, Chain, Shaper};
use lumit_core::anim::Property;
use lumit_core::model::{
    Composition, Document, Layer, LayerKind, LinearColour, MediaRef, ProjectItem, SolidDef,
    Switches, TransformGroup,
};
use lumit_core::time::{CompTime, Duration, FrameRate, Rational};
use lumit_render::colour::tables;
use lumit_render::headless::HeadlessRenderer;
use lumit_render::plan::Quality;
use uuid::Uuid;

/// Colours to compare, and the point of the list: the last eight rows are
/// outside 0–1, which is the seam this whole design exists to close.
fn probe_colours() -> Vec<[f32; 3]> {
    let mut v = vec![
        [0.0, 0.0, 0.0],
        [1.0, 1.0, 1.0],
        [0.18, 0.18, 0.18],
        [0.5, 0.25, 0.75],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        [0.004, 0.002, 0.001],
    ];
    // Out of domain: negatives a wider gamut leaves behind in Rec.709 working,
    // and high dynamic range up to the hundred §7.1 names.
    v.extend_from_slice(&[
        [-0.05, 0.4, 0.9],
        [-0.2, -0.1, 0.02],
        [2.0, 1.4, 0.3],
        [16.0, 8.0, 4.0],
        [100.0, 12.0, -0.3],
        [-1.0, 60.0, 0.5],
        [0.0, -0.004, 31.0],
        [64.0, 64.0, 64.0],
    ]);
    v
}

/// A factorised chain: a transfer curve then a primaries matrix, the shape
/// every camera input transform has.
fn factorised_chain() -> Chain {
    Chain::new(vec![
        Op::MonCurve {
            gamma: [2.4; 3],
            offset: [0.055; 3],
            dir: Direction::Forward,
        },
        Op::Matrix([
            1.1, -0.05, -0.05, 0.0, -0.02, 1.03, -0.01, 0.0, 0.0, -0.1, 1.1, 0.0,
        ]),
    ])
}

/// A chain that mixes the channels, so it cannot factorise and takes the cube.
fn cube_chain() -> Chain {
    Chain::new(vec![
        Op::Cdl {
            params: CdlParams {
                slope: [1.1, 0.95, 1.02],
                offset: [0.01, -0.02, 0.0],
                saturation: 1.3,
                clamp: false,
                ..CdlParams::default()
            },
            dir: Direction::Forward,
        },
        Op::MonCurve {
            gamma: [2.4; 3],
            offset: [0.055; 3],
            dir: Direction::Inverse,
        },
    ])
}

/// Run every probe colour through the shader, one texel each, and read the
/// answers back.
///
/// The pass is the real one: `linearise_through`, the same call footage takes,
/// into the fp16 working format so nothing is clamped on the way out and the
/// comparison is against the oracle rather than against a unorm write.
fn on_the_card(artefact: &Artefact, colours: &[[f32; 3]]) -> Option<Vec<[f32; 3]>> {
    let ctx = lumit_gpu::test_support::lease()?;
    let engine = ctx.colour();
    let uploaded = engine.upload_ocio(&ctx, &tables(artefact));

    // One row of fp16 texels carrying the probe colours, alpha 1 so the
    // shader's unpremultiply branch is the identity and what is measured is
    // the transform rather than the alpha handling.
    let width = colours.len() as u32;
    let mut halves: Vec<u16> = Vec::with_capacity(colours.len() * 4);
    for c in colours {
        for v in [c[0], c[1], c[2], 1.0] {
            halves.push(half::f16::from_f32(v).to_bits());
        }
    }
    let src = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("ocio-parity-src"),
        size: wgpu::Extent3d {
            width,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: lumit_gpu::WORKING_FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    ctx.queue.write_texture(
        src.as_image_copy(),
        bytemuck::cast_slice(&halves),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 8),
            rows_per_image: Some(1),
        },
        src.size(),
    );

    let out = engine.linearise_through(&ctx, &src, Some(&uploaded));
    let back = read_f16_row(&ctx, &out, width);
    Some(back)
}

fn read_f16_row(ctx: &lumit_gpu::GpuContext, tex: &wgpu::Texture, width: u32) -> Vec<[f32; 3]> {
    let row = width * 8;
    let padded =
        row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ocio-parity-read"),
        size: u64::from(padded),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    ctx.flush();
    let mut encoder = ctx.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        tex.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(1),
            },
        },
        tex.size(),
    );
    ctx.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    ctx.device.poll(wgpu::Maintain::Wait);
    rx.recv().unwrap().unwrap();
    let data = slice.get_mapped_range();
    let mut out = Vec::with_capacity(width as usize);
    for texel in data[..row as usize].chunks_exact(8) {
        let v = |i: usize| {
            half::f16::from_bits(u16::from_le_bytes([texel[i * 2], texel[i * 2 + 1]])).to_f32()
        };
        out.push([v(0), v(1), v(2)]);
    }
    drop(data);
    buffer.unmap();
    out
}

/// The tolerance §7.4 states: two fp16 units in the last place. Relative,
/// because fp16's step scales with the value — two ULP at 0.5 is 0.001 and two
/// ULP at 60 is 0.12, and quoting one absolute number for both would either
/// pass everything or fail everything.
fn agrees(got: f32, want: f32) -> bool {
    let ulp = (want.abs().max(6.1e-5) / 1024.0).max(6.0e-8);
    (got - want).abs() <= 2.0 * ulp
}

fn compare(artefact: &Artefact, what: &str) {
    let colours = probe_colours();
    let Some(gpu) = on_the_card(artefact, &colours) else {
        eprintln!("no adapter here, skipping the {what} parity check");
        return;
    };
    for (c, got) in colours.iter().zip(gpu) {
        // The oracle is fed what the CARD was fed. The probe row is an fp16
        // texture, so 0.18 reaches the shader as 0.180053711; handing the
        // oracle 0.18 instead would measure that rounding rather than the
        // sampler, and on a steep table it measures it loudly.
        let c16 = [
            half::f16::from_f32(c[0]).to_f32(),
            half::f16::from_f32(c[1]).to_f32(),
            half::f16::from_f32(c[2]).to_f32(),
        ];
        let want = artefact.eval(c16);
        for k in 0..3 {
            // fp16 is the working format, so the oracle's answer is compared
            // as fp16 too: the difference being measured is the sampler's, not
            // the texture's.
            let want_k = half::f16::from_f32(want[k]).to_f32();
            assert!(
                agrees(got[k], want_k),
                "{what}: at {c:?} channel {k}, the card said {} and the oracle said {want_k}",
                got[k]
            );
        }
    }
}

#[test]
fn the_factorised_sampler_agrees_with_the_oracle_including_out_of_domain() {
    let artefact = bake(&factorised_chain(), Shaper::DEFAULT).expect("bakes");
    assert!(
        lumit_render::colour::is_factorised(&artefact),
        "this chain is supposed to take the factorised form"
    );
    compare(&artefact, "factorised");
}

#[test]
fn the_tetrahedral_sampler_agrees_with_the_oracle_including_out_of_domain() {
    let artefact = bake(&cube_chain(), Shaper::DEFAULT).expect("bakes");
    assert!(
        !lumit_render::colour::is_factorised(&artefact),
        "this chain is supposed to take the cube form"
    );
    compare(&artefact, "tetrahedral");
}

/// Random cubes, because the six-wedge branch is where a transcription slip
/// hides: an identity cube agrees whichever wedge is picked, and only a cube
/// with no symmetry at all makes a wrong branch show up.
///
/// A **uniform** shaper here rather than the logarithmic one, and the reason is
/// worth writing down. `log2` on the card and `log2` in Rust agree to a unit in
/// the last place, not to the bit; on a smooth cube — every cube a real config
/// bakes — that shifts the sample point by a hair and the answer by less than a
/// hair, which is what the test above measures. On a cube of *random* numbers
/// neighbouring samples differ by half of everything, so the same hair of a
/// shift becomes a visible difference and the test would be measuring the two
/// logarithms rather than the two samplers. A uniform shaper is a subtract and
/// a divide, identical on both sides, which leaves only the wedge arithmetic in
/// the frame.
#[test]
fn a_cube_with_no_symmetry_agrees_wedge_for_wedge() {
    let mut state = 0x2545_F491_4F6C_DD1D_u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 40) as f32 / 16_777_216.0
    };
    let size = lumit_colour::bake::CUBE_SIZE;
    let data: Vec<[f32; 3]> = (0..size * size * size)
        .map(|_| [next(), next(), next()])
        .collect();
    let cube = lumit_colour::Cube::new("random", size, [0.0; 3], [1.0; 3], data).expect("built");
    let artefact = Artefact::ShaperCube {
        shaper: Shaper::Uniform { min: 0.0, max: 1.0 },
        cube,
    };
    compare(&artefact, "a random cube");
}

/// **The double-encode trap** (§5.2), pinned rather than described.
///
/// A baked view's output is already display-encoded. If the OCIO display pass
/// wrote it into the `Rgba8UnormSrgb` target the built-in pass uses, the
/// hardware would encode it a second time and every mid-tone would come out
/// pale. So the pass writes through a plain `Unorm` view of the same texture,
/// and this is what proves it: a baked artefact that encodes sRGB by hand must
/// produce the same bytes as the built-in pass, which encodes in hardware.
#[test]
fn a_baked_srgb_view_matches_the_built_in_pass_rather_than_encoding_twice() {
    let Some(ctx) = lumit_gpu::test_support::lease() else {
        eprintln!("no adapter here");
        return;
    };
    let engine = ctx.colour();

    // Linear → sRGB, the transform the built-in display pass performs. As a
    // chain, so it bakes exactly as a config's view would.
    let view = Chain::new(vec![Op::MonCurve {
        gamma: [2.4; 3],
        offset: [0.055; 3],
        dir: Direction::Inverse,
    }]);
    let artefact = bake(&view, Shaper::DEFAULT).expect("bakes");
    let uploaded = engine.upload_ocio(&ctx, &tables(&artefact));

    let colours: Vec<[f32; 3]> = (0..=16)
        .map(|i| {
            let v = i as f32 / 16.0;
            [v, v * 0.6, v * 0.25]
        })
        .collect();
    let width = colours.len() as u32;
    let mut halves: Vec<u16> = Vec::with_capacity(colours.len() * 4);
    for c in &colours {
        for v in [c[0], c[1], c[2], 1.0] {
            halves.push(half::f16::from_f32(v).to_bits());
        }
    }
    let src = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("double-encode-src"),
        size: wgpu::Extent3d {
            width,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: lumit_gpu::WORKING_FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    ctx.queue.write_texture(
        src.as_image_copy(),
        bytemuck::cast_slice(&halves),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 8),
            rows_per_image: Some(1),
        },
        src.size(),
    );

    let neutral = lumit_gpu::DisplayParams::NEUTRAL;
    let built_in = engine.display(&ctx, &src, neutral);
    let through = engine.display_through(&ctx, &src, neutral, Some(&uploaded));
    let a = engine.readback8(&ctx, &built_in).expect("read back");
    let b = engine.readback8(&ctx, &through).expect("read back");

    assert_eq!(a.len(), b.len());
    for (i, (x, y)) in a.iter().zip(&b).enumerate() {
        let d = i32::from(*x) - i32::from(*y);
        assert!(
            d.abs() <= 1,
            "byte {i}: the built-in pass wrote {x} and the baked one wrote {y}. \
             A large difference here — everything pale — means the target was \
             encoded twice (docs/impl/ocio.md §5.2)."
        );
    }
}

/// **K-031, by construction.** The eight-bit display pass and the deep one are
/// two targets of one transform, so a baked view must reach both as the same
/// values — the deep one being wider, not different. This is the row the
/// standing preview-equals-export matrix gains for OCIO: the export reads back
/// from `display16_through`, the Viewer presents `display_through`, and both
/// bind the artefact this test binds.
#[test]
fn the_deep_display_carries_the_same_baked_view_as_the_eight_bit_one() {
    let Some(ctx) = lumit_gpu::test_support::lease() else {
        eprintln!("no adapter here");
        return;
    };
    let engine = ctx.colour();
    let artefact = bake(&cube_chain(), Shaper::DEFAULT).expect("bakes");
    let uploaded = engine.upload_ocio(&ctx, &tables(&artefact));

    let width = 17u32;
    let mut halves: Vec<u16> = Vec::with_capacity(width as usize * 4);
    for i in 0..width {
        let v = i as f32 / (width - 1) as f32;
        for c in [v, v * 0.6, v * 0.25, 1.0] {
            halves.push(half::f16::from_f32(c).to_bits());
        }
    }
    let src = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("k031-src"),
        size: wgpu::Extent3d {
            width,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: lumit_gpu::WORKING_FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    ctx.queue.write_texture(
        src.as_image_copy(),
        bytemuck::cast_slice(&halves),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 8),
            rows_per_image: Some(1),
        },
        src.size(),
    );

    let neutral = lumit_gpu::DisplayParams::NEUTRAL;
    let eight = engine.display_through(&ctx, &src, neutral, Some(&uploaded));
    let deep = engine.display16_through(&ctx, &src, None, neutral, Some(&uploaded));
    let a = engine.readback8(&ctx, &eight).expect("read back");
    let b = engine.readback16(&ctx, &deep).expect("read back");

    assert_eq!(a.len(), b.len());
    for (i, (x, y)) in a.iter().zip(&b).enumerate() {
        // The deep target holds the same value at sixteen bits; comparing at
        // eight is comparing the picture rather than the precision.
        let deep_as_8 = (f32::from(*y) / 65_535.0 * 255.0).round() as i32;
        assert!(
            (i32::from(*x) - deep_as_8).abs() <= 1,
            "byte {i}: eight-bit {x}, deep {deep_as_8}"
        );
    }
}

// ---------------------------------------------------------------------------
// §7.5 — the K-031 parity row, in every shipped colour configuration.
//
// The matrix in `headless.rs` walks constructions: a precomp, a matte, motion
// blur, a driven parameter. This one walks the *colour* axis instead, which is
// the axis docs/06 §3.3 names ("a reference comp in every shipped colour
// configuration") and the one the OCIO work added rows to. Four of them:
//
//   1. no config at all — the built-in display transform, what every project
//      before K-490 rendered through;
//   2. a built-in colour family named at export — which must reach the shared
//      display pass as *nothing*, because the family is a pack-stage transform
//      strictly downstream of the parity point;
//   3. a loaded config's display/view, the Viewer's case;
//   4. a loaded config's space named at export, the delivery case.
//
// In each, the Viewer's eight-bit present and the export's deep one are one
// dispatch with one table bound, so they carry one picture. That is the claim,
// and here it is measured rather than described.
// ---------------------------------------------------------------------------

/// The config the bridge's own seam tests use: one space that is not the
/// reference, one display with one view, and the roles resolution reads. Its
/// view is an sRGB encode, which is also what the built-in pass does — so
/// configuration 3 doubles as the double-encode trap at renderer level.
const FIXTURE_CONFIG: &str = r"
ocio_profile_version: 1
roles:
  scene_linear: lin
  reference: ref
displays:
  sRGB:
    - !<View> {name: Standard, colorspace: out_srgb}
    - !<View> {name: Flat gamma, colorspace: out_g22}
colorspaces:
  - !<ColorSpace>
    name: ref
  - !<ColorSpace>
    name: lin
  - !<ColorSpace>
    name: srgb_texture
    to_reference: !<ExponentWithLinearTransform> {gamma: [2.4, 2.4, 2.4, 1], offset: [0.055, 0.055, 0.055, 0]}
  - !<ColorSpace>
    name: out_srgb
    from_reference: !<ExponentWithLinearTransform> {gamma: [2.4, 2.4, 2.4, 1], offset: [0.055, 0.055, 0.055, 0], direction: inverse}
  - !<ColorSpace>
    name: out_g22
    from_reference: !<ExponentTransform> {value: [2.2, 2.2, 2.2, 1], direction: inverse}
";

/// The reference comp: two overlapping solids at values that are neither black
/// nor clipped, over a mid-grey backdrop, so a transform wrong by a curve shows
/// and one wrong by a matrix shows too.
fn reference_comp(config: Option<&std::path::Path>) -> (std::sync::Arc<Document>, Uuid) {
    const SIZE: u32 = 32;
    let mut doc = Document::new();
    let mut layers = Vec::new();
    for (name, colour, offset) in [
        ("warm", LinearColour([0.42, 0.11, 0.06, 1.0]), -6.0),
        ("cool", LinearColour([0.05, 0.18, 0.55, 1.0]), 6.0),
    ] {
        let def = Uuid::now_v7();
        doc.items.push(ProjectItem::Solid(SolidDef {
            id: def,
            name: name.into(),
            colour,
            width: SIZE / 2,
            height: SIZE,
            extra: serde_json::Map::new(),
        }));
        let mut layer = solid_layer(name, def);
        layer.transform.position_x = Property::fixed(f64::from(SIZE) / 2.0 + offset);
        layer.transform.position_y = Property::fixed(f64::from(SIZE) / 2.0);
        layers.push(layer);
    }
    let comp = Composition {
        master_volume_db: 0.0,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
        name: "Reference".into(),
        width: SIZE,
        height: SIZE,
        frame_rate: FrameRate::new(24, 1).expect("a rate"),
        duration: Duration(Rational::new(1, 1).expect("a second")),
        // Mid grey rather than black: the background is the largest area of the
        // frame, and black is the one value every wrong transform agrees on.
        background: LinearColour([0.18, 0.18, 0.18, 1.0]),
        work_area: None,
        layers,
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    };
    let comp_id = comp.id;
    doc.items.push(ProjectItem::Composition(comp));
    if let Some(path) = config {
        doc.colour.config = Some(MediaRef {
            relative_path: path.to_string_lossy().into_owned(),
            absolute_path: path.to_string_lossy().into_owned(),
            fingerprint: None,
            extra: serde_json::Map::new(),
        });
    }
    (std::sync::Arc::new(doc), comp_id)
}

fn solid_layer(name: &str, def: Uuid) -> Layer {
    Layer {
        graph: Default::default(),
        markers: Vec::new(),
        id: Uuid::now_v7(),
        name: name.into(),
        kind: LayerKind::Solid { def },
        in_point: CompTime(Rational::ZERO),
        out_point: CompTime(Rational::new(1, 1).expect("a second")),
        start_offset: CompTime(Rational::ZERO),
        transform: TransformGroup::default(),
        matte: None,
        parent: None,
        label: 0,
        volume_db: Property::zero(),
        pan: Property::zero(),
        audio_only: false,
        adjustment: false,
        retime: None,
        interpolation: Default::default(),
        parked_flow: None,
        blend: Default::default(),
        masks: Vec::new(),
        paint: Vec::new(),
        puppet: None,
        effects: Vec::new(),
        styles: Vec::new(),
        switches: Switches::default(),
        extra: serde_json::Map::new(),
    }
}

/// One configuration's row: render the Viewer's present and the export's, and
/// require one picture. Returns the eight-bit bytes so configurations can be
/// compared with each other as well as with themselves.
fn parity_row(
    r: &mut HeadlessRenderer,
    doc: &std::sync::Arc<Document>,
    comp_id: Uuid,
    what: &str,
) -> Vec<u8> {
    let (preview, pw, ph) = r
        .render_preview(doc, comp_id, 0, Quality::default(), 1.0)
        .unwrap_or_else(|e| panic!("{what}: the Viewer's present failed: {e}"));
    let (deep, ew, eh) = r
        .render_preview16(doc, comp_id, 0, Quality::default())
        .unwrap_or_else(|e| panic!("{what}: the export's present failed: {e}"));
    assert_eq!((pw, ph), (ew, eh), "{what}: two sizes for one comp");
    assert_eq!(
        preview.len(),
        deep.len(),
        "{what}: two channel counts for one comp"
    );
    for (i, (eight, sixteen)) in preview.iter().zip(&deep).enumerate() {
        // The deep target holds the same value at sixteen bits, so the
        // comparison happens at eight: what is measured is the picture, not the
        // precision the deeper target buys.
        let deep_as_8 = (f32::from(*sixteen) / 65_535.0 * 255.0).round() as i32;
        assert!(
            (i32::from(*eight) - deep_as_8).abs() <= 1,
            "{what}: at channel {i} the Viewer wrote {eight} and the export wrote \
             {deep_as_8}. Preview and export are one colour path (K-031, docs/06 §3.3)."
        );
    }
    preview
}

#[test]
fn preview_equals_export_in_every_colour_configuration() {
    let mut r = match HeadlessRenderer::shared() {
        Ok(r) => r,
        Err(_) => {
            eprintln!("no adapter here, skipping the colour parity matrix");
            return;
        }
    };

    // 1 — no config. The built-in display transform, and the state every
    // project is in until somebody names a config.
    let (plain, plain_comp) = reference_comp(None);
    r.sync_colour(&plain);
    r.set_colour_view(None);
    r.set_colour_output(None);
    assert!(
        r.colour().loaded().is_none(),
        "no config named, so nothing should be loaded"
    );
    let built_in = parity_row(&mut r, &plain, plain_comp, "no config");

    // 2 — a built-in colour family named at export. Every built-in space
    // answers `None` to `ocio_name`, which is what the export asks, so the
    // family reaches the display pass as nothing at all: it is a pack-stage
    // transform, strictly downstream of the point K-031 is measured at. The row
    // exists to hold that structure, because a built-in space that started
    // binding a table here would silently move the parity point.
    for space in lumit_render::export::BUILT_IN_COLOUR_SPACES {
        assert!(
            space.ocio_name().is_none(),
            "{space:?} is a built-in and must not name a config space"
        );
        r.set_colour_output(space.ocio_name().map(str::to_owned));
        let family = parity_row(&mut r, &plain, plain_comp, "the built-in colour family");
        assert_eq!(
            family, built_in,
            "{space:?}: the built-in family must not touch the shared display pass"
        );
    }

    // 3 and 4 need a config on disk. It is written rather than vendored because
    // it is the same text the bridge's seam tests use, and one copy of a fixture
    // that two crates both edit is a fixture that drifts.
    let dir = tempfile::tempdir().expect("a directory");
    let path = dir.path().join("config.ocio");
    std::fs::write(&path, FIXTURE_CONFIG).expect("a config on disk");
    let (managed, managed_comp) = reference_comp(Some(&path));
    r.sync_colour(&managed);
    assert!(
        r.colour().loaded().is_some_and(|l| l.usable()),
        "the fixture config should load: {:?}",
        r.colour().loaded().and_then(|l| l.problem.clone())
    );

    // 3 — the Viewer looking through a display/view.
    r.set_colour_output(None);
    r.set_colour_view(Some(("sRGB".into(), "Standard".into())));
    let through_view = parity_row(&mut r, &managed, managed_comp, "a config's display/view");

    // 4 — a config space named at export. Same bake, same blit, bound at the
    // other end of the same `display_tables` choice.
    r.set_colour_view(None);
    r.set_colour_output(Some("out_srgb".into()));
    assert!(
        r.can_deliver_colour_space("out_srgb"),
        "the config names out_srgb, so the export must be able to deliver it"
    );
    let through_output = parity_row(&mut r, &managed, managed_comp, "a config space at export");

    // The two ends of that choice are one transform: this config's view and its
    // output space are both the sRGB encode, so the pictures are the same one.
    assert_eq!(
        through_view, through_output,
        "the view the Viewer shows and the space the export delivers are one table"
    );

    // Non-vacuity, and it is worth being blunt about why this row exists. The
    // two checks above both pass if no table is bound at all — this config's
    // sRGB view IS the built-in transform, which is exactly what makes it a
    // double-encode test and exactly what makes it a poor proof that anything
    // was bound. So the config carries a second view that is plainly not the
    // built-in, and switching to it must move the picture.
    r.set_colour_output(None);
    r.set_colour_view(Some(("sRGB".into(), "Flat gamma".into())));
    let through_gamma = parity_row(
        &mut r,
        &managed,
        managed_comp,
        "a config's plain-gamma view",
    );
    assert_ne!(
        through_gamma, built_in,
        "a view that is not the built-in transform must render a different picture,          or nothing was bound and the rows above prove nothing"
    );

    // And **the double-encode trap** (§5.2) at renderer level: that baked sRGB
    // encode must land where the built-in pass lands, not a second time on top
    // of it. A pale picture here is the whole failure this design exists to
    // prevent, and it would be a large difference rather than a rounding one.
    assert_eq!(built_in.len(), through_view.len());
    for (i, (a, b)) in built_in.iter().zip(&through_view).enumerate() {
        let d = i32::from(*a) - i32::from(*b);
        assert!(
            d.abs() <= 1,
            "byte {i}: the built-in pass wrote {a} and the config's identity view wrote {b}. \
             A large difference means the target was encoded twice (docs/impl/ocio.md §5.2)."
        );
    }
}
