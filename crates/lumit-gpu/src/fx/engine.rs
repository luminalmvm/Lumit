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
        // Motion blur's layout: src (0), orig-for-mix (1), the flow field (2),
        // the storage output (3) and the uniform (4) — the shared two-input
        // shape plus the one extra sampled texture (modelled on adjust_layout).
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
        let vignette_mod = module(include_str!("../fx_vignette.wgsl"), "fx-vignette");
        let exposure_mod = module(include_str!("../fx_exposure.wgsl"), "fx-exposure");
        let temperature_mod = module(include_str!("../fx_temperature.wgsl"), "fx-temperature");
        let invert_mod = module(include_str!("../fx_invert.wgsl"), "fx-invert");
        let tint_mod = module(include_str!("../fx_tint.wgsl"), "fx-tint");
        let hue_mod = module(include_str!("../fx_hue.wgsl"), "fx-hue");
        let contrast_mod = module(include_str!("../fx_contrast.wgsl"), "fx-contrast");
        let gamma_mod = module(include_str!("../fx_gamma.wgsl"), "fx-gamma");
        let transform_mod = module(include_str!("../fx_transform.wgsl"), "fx-transform");
        let glow_mod = module(include_str!("../fx_glow.wgsl"), "fx-glow");
        let block_glitch_mod = module(include_str!("../fx_block_glitch.wgsl"), "fx-block-glitch");
        let scanlines_mod = module(include_str!("../fx_scanlines.wgsl"), "fx-scanlines");
        let echo_mod = module(include_str!("../fx_echo.wgsl"), "fx-echo");
        let motion_blur_mod = module(include_str!("../fx_motionblur.wgsl"), "fx-motion-blur");
        let datamosh_mod = module(include_str!("../fx_datamosh.wgsl"), "fx-datamosh");
        let dof_mod = module(include_str!("../fx_dof.wgsl"), "fx-dof");
        let adjust_mod = module(include_str!("../fx_adjust.wgsl"), "fx-adjust");
        let lut_mod = module(include_str!("../fx_lut.wgsl"), "fx-lut");
        let blur = pipeline(&blur_mod, "fx-blur", "blur_pass");
        let dir_blur = pipeline(&dir_blur_mod, "fx-dir-blur", "dir_blur");
        let radial_blur = pipeline(&radial_blur_mod, "fx-radial-blur", "radial_blur");
        let sharpen_unpremultiply = pipeline(&sharpen_mod, "fx-sharpen-un", "unpremultiply");
        let sharpen_combine = pipeline(&sharpen_mod, "fx-sharpen", "sharpen_combine");
        let sharpen_simple = pipeline(&sharpen_simple_mod, "fx-sharpen-simple", "sharpen_simple");
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
        let vignette = pipeline(&vignette_mod, "fx-vignette", "vignette");
        let exposure = pipeline(&exposure_mod, "fx-exposure", "exposure");
        let temperature = pipeline(&temperature_mod, "fx-temperature", "temperature");
        let invert = pipeline(&invert_mod, "fx-invert", "invert");
        let tint = pipeline(&tint_mod, "fx-tint", "tint");
        let hue_shift = pipeline(&hue_mod, "fx-hue", "hue_shift");
        let contrast = pipeline(&contrast_mod, "fx-contrast", "contrast");
        let gamma = pipeline(&gamma_mod, "fx-gamma", "gamma");
        let transform = pipeline(&transform_mod, "fx-transform", "transform");
        let glow_bright = pipeline(&glow_mod, "fx-glow-bright", "glow_bright");
        let glow_combine = pipeline(&glow_mod, "fx-glow", "glow_combine");
        let block_glitch = pipeline(&block_glitch_mod, "fx-block-glitch", "block_glitch");
        let scanlines = pipeline(&scanlines_mod, "fx-scanlines", "scanlines");
        let echo_accumulate = pipeline(&echo_mod, "fx-echo-accumulate", "echo_accumulate");
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
        Self {
            blur,
            dir_blur,
            radial_blur,
            sharpen_unpremultiply,
            sharpen_combine,
            sharpen_simple,
            rgb_split,
            spectral_split,
            chromatic_aberration,
            flash,
            colour_balance,
            saturation,
            vibrancy,
            matte_key,
            vignette,
            exposure,
            temperature,
            invert,
            tint,
            hue_shift,
            contrast,
            gamma,
            transform,
            glow_bright,
            glow_combine,
            block_glitch,
            scanlines,
            echo_accumulate,
            echo_mix,
            motion_blur,
            datamosh,
            dof,
            adjust,
            lut,
            layout,
            adjust_layout,
            mb_layout,
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
