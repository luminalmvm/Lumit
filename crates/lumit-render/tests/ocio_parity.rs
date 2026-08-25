//! The graphics card's colour samplers against the processor's oracle
//! (docs/impl/ocio.md §7.4), and the double-encode trap pinned (§5.2).
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
use lumit_render::colour::tables;

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
    let ctx = lumit_gpu::GpuContext::headless().ok()?;
    let engine = lumit_gpu::ColourEngine::new(&ctx);
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
    let Ok(ctx) = lumit_gpu::GpuContext::headless() else {
        eprintln!("no adapter here");
        return;
    };
    let engine = lumit_gpu::ColourEngine::new(&ctx);

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
    let Ok(ctx) = lumit_gpu::GpuContext::headless() else {
        eprintln!("no adapter here");
        return;
    };
    let engine = lumit_gpu::ColourEngine::new(&ctx);
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
