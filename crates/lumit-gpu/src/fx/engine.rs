//! Construction of the effect engine: `FxEngine::new` compiles every WGSL
//! kernel, builds the bind-group layouts and assembles the pipeline table.
//! The per-effect apply methods live in the sibling family modules.

use crate::{GpuContext, WORKING_FORMAT};

use super::FxEngine;

impl FxEngine {
    pub fn new(ctx: &GpuContext) -> Self {
        let layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("fx-layout"),
                entries: &[
                    texture_entry(0),
                    texture_entry(1),
                    // The generic Matte (K-395) at binding 4, below. It is on
                    // the SHARED layout rather than a layout of its own because
                    // a kernel need not use every binding its pipeline layout
                    // declares — so the two kernels that read a matte get one,
                    // the twenty that do not are unchanged, and there is no
                    // second bind-group shape to keep in step.
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::StorageTexture {
                            access: wgpu::StorageTextureAccess::WriteOnly,
                            format: WORKING_FORMAT,
                            view_dimension: wgpu::TextureViewDimension::D2,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    texture_entry(4),
                ],
            });
        let pipeline_layout = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("fx-pl"),
                bind_group_layouts: &[&layout],
                push_constant_ranges: &[],
            });
        let adjust_layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("fx-adjust-layout"),
                entries: &[
                    texture_entry(0),
                    texture_entry(1),
                    texture_entry(2),
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::StorageTexture {
                            access: wgpu::StorageTextureAccess::WriteOnly,
                            format: WORKING_FORMAT,
                            view_dimension: wgpu::TextureViewDimension::D2,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });
        let adjust_pl = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("fx-adjust-pl"),
                bind_group_layouts: &[&adjust_layout],
                push_constant_ranges: &[],
            });
        // The dominant-motion reduction Fast motion blur runs first (K-390,
        // docs/impl/optical-flow.md §4.5 item 3): the flow field in (0), one
        // texel per tile out (1), the uniform (2). Its own layout because the
        // output must be rgba32float — the tile vectors are compared against an
        // f32 CPU oracle, and the working fp16 format would round them.
        let mb_tile_layout =
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("fx-mb-tile-layout"),
                    entries: &[
                        texture_entry(0),
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::COMPUTE,
                            ty: wgpu::BindingType::StorageTexture {
                                access: wgpu::StorageTextureAccess::WriteOnly,
                                format: wgpu::TextureFormat::Rgba32Float,
                                view_dimension: wgpu::TextureViewDimension::D2,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 2,
                            visibility: wgpu::ShaderStages::COMPUTE,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Uniform,
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                    ],
                });
        let mb_tile_pl = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("fx-mb-tile-pl"),
                bind_group_layouts: &[&mb_tile_layout],
                push_constant_ranges: &[],
            });
        // Motion blur's layout: src (0) — also the orig-for-mix, since it is a
        // single pass — the dominant-motion tiles (1), the flow field (2), the
        // storage output (3) and the uniform (4) — the shared three-sampled-input
        // shape (modelled on adjust_layout).
        let mb_layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("fx-mb-layout"),
                entries: &[
                    texture_entry(0),
                    texture_entry(1),
                    texture_entry(2),
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::StorageTexture {
                            access: wgpu::StorageTextureAccess::WriteOnly,
                            format: WORKING_FORMAT,
                            view_dimension: wgpu::TextureViewDimension::D2,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // The Matte (K-429), on this layout rather than the shared
                    // one because Motion blur has three sampled inputs of its
                    // own before it gets to a matte.
                    texture_entry(5),
                ],
            });
        let mb_pl = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("fx-mb-pl"),
                bind_group_layouts: &[&mb_layout],
                push_constant_ranges: &[],
            });
        // The LUT lookup's layout: src (0), orig-for-mix (1), the storage
        // output (2), the uniform (3) and — the one thing no other kernel has —
        // the cube as a 3D texture at binding 4 (filterable:false; the shader
        // does its own trilinear via textureLoad, docs/impl/lut.md §3).
        let lut_layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("fx-lut-layout"),
                entries: &[
                    texture_entry(0),
                    texture_entry(1),
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::StorageTexture {
                            access: wgpu::StorageTextureAccess::WriteOnly,
                            format: WORKING_FORMAT,
                            view_dimension: wgpu::TextureViewDimension::D2,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D3,
                            multisampled: false,
                        },
                        count: None,
                    },
                ],
            });
        let lut_pl = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("fx-lut-pl"),
                bind_group_layouts: &[&lut_layout],
                push_constant_ranges: &[],
            });
        let module = |wgsl: &str, name: &str| {
            ctx.device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some(name),
                    source: wgpu::ShaderSource::Wgsl(wgsl.into()),
                })
        };
        let pipeline = |shader: &wgpu::ShaderModule, name: &str, entry: &str| {
            ctx.device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(name),
                    layout: Some(&pipeline_layout),
                    module: shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                })
        };
        let blur_mod = module(include_str!("../fx_blur.wgsl"), "fx-blur");
        let dir_blur_mod = module(include_str!("../fx_dirblur.wgsl"), "fx-dir-blur");
        let radial_blur_mod = module(include_str!("../fx_radialblur.wgsl"), "fx-radial-blur");
        let sharpen_mod = module(include_str!("../fx_sharpen.wgsl"), "fx-sharpen");
        let sharpen_simple_mod = module(
            include_str!("../fx_sharpen_simple.wgsl"),
            "fx-sharpen-simple",
        );
        let light_wrap_mod = module(include_str!("../fx_light_wrap.wgsl"), "fx-light-wrap");
        let sprite_flare_mod = module(include_str!("../fx_sprite_flare.wgsl"), "fx-sprite-flare");
        let rgb_split_mod = module(include_str!("../fx_rgbsplit.wgsl"), "fx-rgb-split");
        let spectral_mod = module(include_str!("../fx_spectral.wgsl"), "fx-spectral-split");
        let chromatic_mod = module(
            include_str!("../fx_chromatic.wgsl"),
            "fx-chromatic-aberration",
        );
        let flash_mod = module(include_str!("../fx_flash.wgsl"), "fx-flash");
        let balance_mod = module(
            include_str!("../fx_colourbalance.wgsl"),
            "fx-colour-balance",
        );
        let saturation_mod = module(include_str!("../fx_saturation.wgsl"), "fx-saturation");
        let vibrancy_mod = module(include_str!("../fx_vibrancy.wgsl"), "fx-vibrancy");
        let matte_key_mod = module(include_str!("../fx_matte_key.wgsl"), "fx-matte-key");
        let matte_tidy_mod = module(include_str!("../fx_matte_tidy.wgsl"), "fx-matte-tidy");
        let stroke_mod = module(include_str!("../fx_stroke.wgsl"), "fx-stroke");
        let matte_mask_mod = module(include_str!("../fx_matte_mask.wgsl"), "fx-matte-mask");
        let vignette_mod = module(include_str!("../fx_vignette.wgsl"), "fx-vignette");
        let exposure_mod = module(include_str!("../fx_exposure.wgsl"), "fx-exposure");
        let lighting_mod = module(include_str!("../fx_lighting.wgsl"), "fx-lighting");
        let temperature_mod = module(include_str!("../fx_temperature.wgsl"), "fx-temperature");
        let invert_mod = module(include_str!("../fx_invert.wgsl"), "fx-invert");
        let tint_mod = module(include_str!("../fx_tint.wgsl"), "fx-tint");
        let hue_mod = module(include_str!("../fx_hue.wgsl"), "fx-hue");
        let contrast_mod = module(include_str!("../fx_contrast.wgsl"), "fx-contrast");
        let gamma_mod = module(include_str!("../fx_gamma.wgsl"), "fx-gamma");
        let curves_mod = module(include_str!("../fx_curves.wgsl"), "fx-curves");
        let levels_mod = module(include_str!("../fx_levels.wgsl"), "fx-levels");
        let brightness_mod = module(include_str!("../fx_brightness.wgsl"), "fx-brightness");
        let huesat_mod = module(include_str!("../fx_huesat.wgsl"), "fx-hue-saturation");
        let posterize_mod = module(include_str!("../fx_posterize.wgsl"), "fx-posterize");
        let threshold_mod = module(include_str!("../fx_threshold.wgsl"), "fx-threshold");
        let tritone_mod = module(include_str!("../fx_tritone.wgsl"), "fx-tritone");
        let photo_filter_mod = module(include_str!("../fx_photofilter.wgsl"), "fx-photo-filter");
        let blackwhite_mod = module(include_str!("../fx_blackwhite.wgsl"), "fx-black-and-white");
        let shadow_highlight_mod = module(
            include_str!("../fx_shadowhighlight.wgsl"),
            "fx-shadow-highlight",
        );
        let fill_mod = module(include_str!("../fx_fill.wgsl"), "fx-fill");
        let gradient_mod = module(include_str!("../fx_gradient.wgsl"), "fx-gradient");
        let noise_mod = module(include_str!("../fx_noise.wgsl"), "fx-noise");
        // The noise core is not a kernel: it is prepended to every kernel that
        // reads the noise field, which is WGSL's only way of having a shared
        // module (docs/08 §3.37, §3.38). One twin of `lumit_core::fx::noise`,
        // not one per effect.
        let noise_core = include_str!("../fx_noise_core.wgsl");
        let fractal_noise_mod = module(
            &format!("{noise_core}{}", include_str!("../fx_fractal_noise.wgsl")),
            "fx-fractal-noise",
        );
        let beam_mod = module(include_str!("../fx_beam.wgsl"), "fx-beam");
        let lightning_mod = module(include_str!("../fx_lightning.wgsl"), "fx-lightning");
        let radio_waves_mod = module(include_str!("../fx_radiowaves.wgsl"), "fx-radio-waves");
        let vegas_mod = module(include_str!("../fx_vegas.wgsl"), "fx-vegas");
        // Scribble's waver is the shared lattice read as a displacement, so
        // the noise core is prepended exactly as Fractal noise's is.
        let path_draw_mod = module(
            &format!("{noise_core}{}", include_str!("../fx_pathdraw.wgsl")),
            "fx-path-draw",
        );
        // The grain is the shared lattice, so the noise core is prepended
        // exactly as Fractal noise's is.
        let add_grain_mod = module(
            &format!("{noise_core}{}", include_str!("../fx_addgrain.wgsl")),
            "fx-add-grain",
        );
        let turbdisplace_mod = module(
            &format!("{noise_core}{}", include_str!("../fx_turbdisplace.wgsl")),
            "fx-turbulent-displace",
        );
        let tile_mod = module(include_str!("../fx_tile.wgsl"), "fx-tile");
        let offset_mod = module(include_str!("../fx_offset.wgsl"), "fx-offset");
        let mirror_mod = module(include_str!("../fx_mirror.wgsl"), "fx-mirror");
        let lens_distort_mod = module(include_str!("../fx_lensdistort.wgsl"), "fx-lens-distort");
        let corner_pin_mod = module(include_str!("../fx_cornerpin.wgsl"), "fx-corner-pin");
        let dispmap_mod = module(include_str!("../fx_dispmap.wgsl"), "fx-displacement-map");
        let polar_mod = module(include_str!("../fx_polar.wgsl"), "fx-polar-coordinates");
        let twirl_mod = module(include_str!("../fx_twirl.wgsl"), "fx-twirl");
        let spherize_mod = module(include_str!("../fx_spherize.wgsl"), "fx-spherize");
        let ripple_mod = module(include_str!("../fx_ripple.wgsl"), "fx-ripple");
        let wave_warp_mod = module(include_str!("../fx_wavewarp.wgsl"), "fx-wave-warp");
        let bezier_warp_mod = module(include_str!("../fx_bezierwarp.wgsl"), "fx-bezier-warp");
        let warp_mod = module(include_str!("../fx_warp.wgsl"), "fx-warp");
        let roughen_edges_mod = module(
            &format!("{noise_core}{}", include_str!("../fx_roughenedges.wgsl")),
            "fx-roughen-edges",
        );
        let median_mod = module(include_str!("../fx_median.wgsl"), "fx-median");
        let mosaic_mod = module(include_str!("../fx_mosaic.wgsl"), "fx-mosaic");
        let find_edges_mod = module(include_str!("../fx_findedges.wgsl"), "fx-find-edges");
        let emboss_mod = module(include_str!("../fx_emboss.wgsl"), "fx-emboss");
        let texturize_mod = module(include_str!("../fx_texturize.wgsl"), "fx-texturize");
        let broadcast_safe_mod = module(
            include_str!("../fx_broadcastsafe.wgsl"),
            "fx-broadcast-safe",
        );
        let chan_blur_mod = module(include_str!("../fx_chanblur.wgsl"), "fx-channel-blur");
        let drop_shadow_mod = module(include_str!("../fx_dropshadow.wgsl"), "fx-drop-shadow");
        let set_matte_mod = module(include_str!("../fx_setmatte.wgsl"), "fx-set-matte");
        let set_channels_mod = module(include_str!("../fx_setchannels.wgsl"), "fx-set-channels");
        let linear_wipe_mod = module(include_str!("../fx_linearwipe.wgsl"), "fx-linear-wipe");
        let radial_wipe_mod = module(include_str!("../fx_radialwipe.wgsl"), "fx-radial-wipe");
        let venetian_blinds_mod = module(
            include_str!("../fx_venetianblinds.wgsl"),
            "fx-venetian-blinds",
        );
        let iris_wipe_mod = module(include_str!("../fx_iriswipe.wgsl"), "fx-iris-wipe");
        // The per-card shuffle is the shared hash, so the noise core is
        // prepended exactly as Fractal noise's and Roughen edges' are.
        let card_wipe_mod = module(
            &format!("{noise_core}{}", include_str!("../fx_cardwipe.wgsl")),
            "fx-card-wipe",
        );
        let transform_mod = module(include_str!("../fx_transform.wgsl"), "fx-transform");
        let shake_mb_mod = module(include_str!("../fx_shake_mb.wgsl"), "fx-shake-mb");
        let glow_mod = module(include_str!("../fx_glow.wgsl"), "fx-glow");
        let block_glitch_mod = module(include_str!("../fx_block_glitch.wgsl"), "fx-block-glitch");
        let scanlines_mod = module(include_str!("../fx_scanlines.wgsl"), "fx-scanlines");
        let echo_mod = module(include_str!("../fx_echo.wgsl"), "fx-echo");
        let motion_blur_mod = module(include_str!("../fx_motionblur.wgsl"), "fx-motion-blur");
        let mb_tilemax_mod = module(include_str!("../fx_mb_tilemax.wgsl"), "fx-mb-tilemax");
        let accum_shutter_mod =
            module(include_str!("../fx_accum_shutter.wgsl"), "fx-accum-shutter");
        let datamosh_mod = module(include_str!("../fx_datamosh.wgsl"), "fx-datamosh");
        let dof_mod = module(include_str!("../fx_dof.wgsl"), "fx-dof");
        let adjust_mod = module(include_str!("../fx_adjust.wgsl"), "fx-adjust");
        let matte_mix_mod = module(include_str!("../fx_matte_mix.wgsl"), "fx-matte-mix");
        let matte_prepare_mod =
            module(include_str!("../fx_matte_prepare.wgsl"), "fx-matte-prepare");
        let blend_mix_mod = module(include_str!("../fx_blend_mix.wgsl"), "fx-blend-mix");
        let lut_mod = module(include_str!("../fx_lut.wgsl"), "fx-lut");
        let blur = pipeline(&blur_mod, "fx-blur", "blur_pass");
        let dir_blur = pipeline(&dir_blur_mod, "fx-dir-blur", "dir_blur");
        let radial_blur = pipeline(&radial_blur_mod, "fx-radial-blur", "radial_blur");
        let sharpen_unpremultiply = pipeline(&sharpen_mod, "fx-sharpen-un", "unpremultiply");
        let sharpen_combine = pipeline(&sharpen_mod, "fx-sharpen", "sharpen_combine");
        let sharpen_simple = pipeline(&sharpen_simple_mod, "fx-sharpen-simple", "sharpen_simple");
        let sprite_flare = pipeline(&sprite_flare_mod, "fx-sprite-flare", "sprite_flare");
        let light_wrap_pack = pipeline(&light_wrap_mod, "fx-light-wrap-pack", "pack");
        let light_wrap_combine = pipeline(&light_wrap_mod, "fx-light-wrap", "combine");
        let rgb_split = pipeline(&rgb_split_mod, "fx-rgb-split", "rgb_split");
        let spectral_split = pipeline(&spectral_mod, "fx-spectral-split", "spectral_split");
        let chromatic_aberration = pipeline(
            &chromatic_mod,
            "fx-chromatic-aberration",
            "chromatic_aberration",
        );
        let flash = pipeline(&flash_mod, "fx-flash", "flash");
        let colour_balance = pipeline(&balance_mod, "fx-colour-balance", "colour_balance");
        let saturation = pipeline(&saturation_mod, "fx-saturation", "saturate_fx");
        let vibrancy = pipeline(&vibrancy_mod, "fx-vibrancy", "vibrance_fx");
        let matte_key = pipeline(&matte_key_mod, "fx-matte-key", "matte_key");
        let matte_key_screen = pipeline(&matte_key_mod, "fx-matte-key-screen", "matte_key_screen");
        let matte_key_combine =
            pipeline(&matte_key_mod, "fx-matte-key-combine", "matte_key_combine");
        let matte_morph = pipeline(&matte_tidy_mod, "fx-matte-morph", "matte_morph");
        let matte_despot = pipeline(&matte_tidy_mod, "fx-matte-despot", "matte_despot");
        let stroke_morph = pipeline(&stroke_mod, "fx-stroke-morph", "stroke_morph");
        let stroke_combine = pipeline(&stroke_mod, "fx-stroke-combine", "stroke_combine");
        let matte_mask = pipeline(&matte_mask_mod, "fx-matte-mask", "matte_mask");
        let vignette = pipeline(&vignette_mod, "fx-vignette", "vignette");
        let exposure = pipeline(&exposure_mod, "fx-exposure", "exposure");
        let lighting = pipeline(&lighting_mod, "fx-lighting", "lighting");
        let temperature = pipeline(&temperature_mod, "fx-temperature", "temperature");
        let invert = pipeline(&invert_mod, "fx-invert", "invert");
        let tint = pipeline(&tint_mod, "fx-tint", "tint");
        let hue_shift = pipeline(&hue_mod, "fx-hue", "hue_shift");
        let contrast = pipeline(&contrast_mod, "fx-contrast", "contrast");
        let gamma = pipeline(&gamma_mod, "fx-gamma", "gamma");
        let curves = pipeline(&curves_mod, "fx-curves", "curves");
        let levels = pipeline(&levels_mod, "fx-levels", "levels");
        let brightness = pipeline(&brightness_mod, "fx-brightness", "brightness");
        let hue_saturation = pipeline(&huesat_mod, "fx-hue-saturation", "hue_saturation");
        let posterize = pipeline(&posterize_mod, "fx-posterize", "posterize");
        let threshold = pipeline(&threshold_mod, "fx-threshold", "threshold");
        let tritone = pipeline(&tritone_mod, "fx-tritone", "tritone");
        let photo_filter = pipeline(&photo_filter_mod, "fx-photo-filter", "photo_filter");
        let black_and_white = pipeline(&blackwhite_mod, "fx-black-and-white", "black_and_white");
        let shadow_highlight = pipeline(
            &shadow_highlight_mod,
            "fx-shadow-highlight",
            "shadow_highlight",
        );
        let fill = pipeline(&fill_mod, "fx-fill", "fill");
        let gradient = pipeline(&gradient_mod, "fx-gradient", "gradient");
        let noise = pipeline(&noise_mod, "fx-noise", "noise");
        let fractal_noise = pipeline(&fractal_noise_mod, "fx-fractal-noise", "fractal_noise");
        let beam = pipeline(&beam_mod, "fx-beam", "beam");
        let lightning = pipeline(&lightning_mod, "fx-lightning", "lightning");
        let radio_waves = pipeline(&radio_waves_mod, "fx-radio-waves", "radio_waves");
        let vegas = pipeline(&vegas_mod, "fx-vegas", "vegas");
        let path_draw = pipeline(&path_draw_mod, "fx-path-draw", "path_draw");
        let add_grain = pipeline(&add_grain_mod, "fx-add-grain", "add_grain");
        let turbulent_displace = pipeline(
            &turbdisplace_mod,
            "fx-turbulent-displace",
            "turbulent_displace",
        );
        let tile = pipeline(&tile_mod, "fx-tile", "tile");
        let offset = pipeline(&offset_mod, "fx-offset", "offset");
        let mirror = pipeline(&mirror_mod, "fx-mirror", "mirror");
        let lens_distort = pipeline(&lens_distort_mod, "fx-lens-distort", "lens_distort");
        let corner_pin = pipeline(&corner_pin_mod, "fx-corner-pin", "corner_pin");
        let displacement_map = pipeline(&dispmap_mod, "fx-displacement-map", "displacement_map");
        let polar_coordinates = pipeline(&polar_mod, "fx-polar-coordinates", "polar_coordinates");
        let twirl = pipeline(&twirl_mod, "fx-twirl", "twirl");
        let spherize = pipeline(&spherize_mod, "fx-spherize", "spherize");
        let ripple = pipeline(&ripple_mod, "fx-ripple", "ripple");
        let wave_warp = pipeline(&wave_warp_mod, "fx-wave-warp", "wave_warp");
        let bezier_warp = pipeline(&bezier_warp_mod, "fx-bezier-warp", "bezier_warp");
        let warp = pipeline(&warp_mod, "fx-warp", "warp");
        let roughen_edges = pipeline(&roughen_edges_mod, "fx-roughen-edges", "roughen_edges");
        let median = pipeline(&median_mod, "fx-median", "median");
        let mosaic = pipeline(&mosaic_mod, "fx-mosaic", "mosaic");
        let find_edges = pipeline(&find_edges_mod, "fx-find-edges", "find_edges");
        let emboss = pipeline(&emboss_mod, "fx-emboss", "emboss");
        let texturize = pipeline(&texturize_mod, "fx-texturize", "texturize");
        let broadcast_safe = pipeline(&broadcast_safe_mod, "fx-broadcast-safe", "broadcast_safe");
        let channel_blur = pipeline(&chan_blur_mod, "fx-channel-blur", "channel_blur");
        let drop_shadow = pipeline(&drop_shadow_mod, "fx-drop-shadow", "drop_shadow");
        let set_matte = pipeline(&set_matte_mod, "fx-set-matte", "set_matte");
        let set_channels = pipeline(&set_channels_mod, "fx-set-channels", "set_channels");
        let linear_wipe = pipeline(&linear_wipe_mod, "fx-linear-wipe", "linear_wipe");
        let radial_wipe = pipeline(&radial_wipe_mod, "fx-radial-wipe", "radial_wipe");
        let venetian_blinds = pipeline(
            &venetian_blinds_mod,
            "fx-venetian-blinds",
            "venetian_blinds",
        );
        let iris_wipe = pipeline(&iris_wipe_mod, "fx-iris-wipe", "iris_wipe");
        let card_wipe = pipeline(&card_wipe_mod, "fx-card-wipe", "card_wipe");
        let transform = pipeline(&transform_mod, "fx-transform", "transform");
        let shake_mb = pipeline(&shake_mb_mod, "fx-shake-mb", "shake_mb");
        let glow_bright = pipeline(&glow_mod, "fx-glow-bright", "glow_bright");
        let glow_combine = pipeline(&glow_mod, "fx-glow", "glow_combine");
        let block_glitch = pipeline(&block_glitch_mod, "fx-block-glitch", "block_glitch");
        let scanlines = pipeline(&scanlines_mod, "fx-scanlines", "scanlines");
        let echo_accumulate = pipeline(&echo_mod, "fx-echo-accumulate", "echo_accumulate");
        let accum_shutter = pipeline(&accum_shutter_mod, "fx-accum-shutter", "accum_shutter");
        let echo_mix = pipeline(&echo_mod, "fx-echo-mix", "echo_mix");
        let motion_blur = ctx
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("fx-motion-blur"),
                layout: Some(&mb_pl),
                module: &motion_blur_mod,
                entry_point: Some("motion_blur"),
                compilation_options: Default::default(),
                cache: None,
            });
        let mb_tilemax = ctx
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("fx-mb-tilemax"),
                layout: Some(&mb_tile_pl),
                module: &mb_tilemax_mod,
                entry_point: Some("mb_tilemax"),
                compilation_options: Default::default(),
                cache: None,
            });
        // The supplied Motion vectors layer read as a flow field (K-429): the
        // same layout, since it is also "a picture in, an rgba32float field
        // out", and so no seam of its own.
        let mb_vectors = ctx
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("fx-mb-vectors"),
                layout: Some(&mb_tile_pl),
                module: &mb_tilemax_mod,
                entry_point: Some("mb_vectors"),
                compilation_options: Default::default(),
                cache: None,
            });
        let datamosh = ctx
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("fx-datamosh"),
                layout: Some(&mb_pl),
                module: &datamosh_mod,
                entry_point: Some("datamosh"),
                compilation_options: Default::default(),
                cache: None,
            });
        let dof = ctx
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("fx-dof"),
                layout: Some(&mb_pl),
                module: &dof_mod,
                entry_point: Some("dof"),
                compilation_options: Default::default(),
                cache: None,
            });
        let adjust = ctx
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("fx-adjust"),
                layout: Some(&adjust_pl),
                module: &adjust_mod,
                entry_point: Some("adjust_blend"),
                compilation_options: Default::default(),
                cache: None,
            });
        // The generic Matte dissolve (K-395). Same three-sampled-inputs shape
        // as the adjustment blend, so it borrows that layout rather than
        // declaring an identical one.
        let matte_mix = ctx
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("fx-matte-mix"),
                layout: Some(&adjust_pl),
                module: &matte_mix_mod,
                entry_point: Some("matte_mix"),
                compilation_options: Default::default(),
                cache: None,
            });
        // The seam's two K-425 passes, on the same layout for the same reason.
        let matte_prepare = ctx
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("fx-matte-prepare"),
                layout: Some(&adjust_pl),
                module: &matte_prepare_mod,
                entry_point: Some("matte_prepare"),
                compilation_options: Default::default(),
                cache: None,
            });
        let blend_mix = ctx
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("fx-blend-mix"),
                layout: Some(&adjust_pl),
                module: &blend_mix_mod,
                entry_point: Some("blend_mix"),
                compilation_options: Default::default(),
                cache: None,
            });
        let lut = ctx
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("fx-lut"),
                layout: Some(&lut_pl),
                module: &lut_mod,
                entry_point: Some("lut_apply"),
                compilation_options: Default::default(),
                cache: None,
            });
        let lens_flare = super::LazyFlare::spawn(ctx);
        // Particulate (docs/08 §3.86): four compute passes and one instanced
        // draw off one module, so the closed forms and the quad that shows them
        // are read from one file (docs/impl/particulate.md §6).
        let particulate = super::ParticulatePipelines::new(
            ctx,
            &module(
                &format!("{noise_core}{}", include_str!("../fx_particulate.wgsl")),
                "fx-particulate",
            ),
        );
        Self {
            particulate_layout: particulate.layout,
            particulate_draw_layout: particulate.draw_layout,
            particulate_empty_layout: particulate.empty_layout,
            particulate_alive: particulate.alive,
            particulate_scan: particulate.scan,
            particulate_blocks: particulate.blocks,
            particulate_scatter: particulate.scatter,
            particulate_draw: particulate.draw,
            lens_flare,
            blur,
            dir_blur,
            radial_blur,
            sharpen_unpremultiply,
            sharpen_combine,
            sharpen_simple,
            sprite_flare,
            light_wrap_pack,
            light_wrap_combine,
            rgb_split,
            spectral_split,
            chromatic_aberration,
            flash,
            colour_balance,
            saturation,
            vibrancy,
            matte_key,
            matte_key_screen,
            matte_key_combine,
            matte_morph,
            matte_despot,
            stroke_morph,
            stroke_combine,
            matte_mask,
            vignette,
            exposure,
            lighting,
            temperature,
            invert,
            tint,
            hue_shift,
            contrast,
            gamma,
            curves,
            levels,
            brightness,
            hue_saturation,
            posterize,
            threshold,
            tritone,
            photo_filter,
            black_and_white,
            shadow_highlight,
            fill,
            gradient,
            noise,
            fractal_noise,
            beam,
            lightning,
            radio_waves,
            vegas,
            path_draw,
            add_grain,
            turbulent_displace,
            tile,
            offset,
            mirror,
            lens_distort,
            corner_pin,
            displacement_map,
            polar_coordinates,
            twirl,
            spherize,
            ripple,
            wave_warp,
            bezier_warp,
            warp,
            roughen_edges,
            median,
            mosaic,
            find_edges,
            emboss,
            texturize,
            broadcast_safe,
            channel_blur,
            drop_shadow,
            set_matte,
            set_channels,
            linear_wipe,
            radial_wipe,
            venetian_blinds,
            iris_wipe,
            card_wipe,
            transform,
            shake_mb,
            glow_bright,
            glow_combine,
            block_glitch,
            scanlines,
            echo_accumulate,
            accum_shutter,
            echo_mix,
            motion_blur,
            mb_tilemax,
            mb_vectors,
            datamosh,
            dof,
            adjust,
            matte_mix,
            matte_prepare,
            blend_mix,
            lut,
            custom_shader: super::custom_shader::CustomShaderPipelines::new(ctx),
            layout,
            adjust_layout,
            mb_layout,
            mb_tile_layout,
            lut_layout,
        }
    }
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}
