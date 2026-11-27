use super::geometry::{Gradient, Guide, Noise, Ramp, Sine};
use super::noise::Perlin;
use super::Turbiniu;

pub(super) struct Look {
    pub(super) distribution: Ramp,
    pub(super) distortion: Noise,
    pub(super) rotation: Noise,
    pub(super) alpha: Noise,
    pub(super) density: [Ramp; 2],
    pub(super) guides: [Guide; 2],
    pub(super) cross: Gradient,
    pub(super) dry_ramp: Ramp,
    pub(super) dry_field: Perlin,
    pub(super) rough_field: Perlin,
}

impl Turbiniu {
    pub(super) fn look(&self) -> Look {
        Look {
            distribution: Ramp::new(self.distribution_ramp, 0.0, 100.0, false),
            distortion: Noise {
                coordinates: self.distortion_coordinates,
                amplitude: self.distortion_amplitude,
                frequency: self.distortion_frequency,
                axes: [self.distortion_scale_x, self.distortion_scale_y],
                offset: [self.distortion_offset_x, self.distortion_offset_y],
                evolution: self.distortion_evolution,
                cycle: self.distortion_loop.clamp(0.0, 1_000_000.0),
                line_offset: self.distortion_line_offset,
                line_steps_phase: false,
                field: Perlin::new(self.distortion_seed as u32),
                ramp: Ramp::new(
                    self.distortion_ramp,
                    self.distortion_ramp_offset,
                    self.distortion_ramp_scale,
                    self.distortion_ramp_loop,
                ),
            },
            rotation: Noise {
                coordinates: self.rotation_noise_coordinates,
                amplitude: self.rotation_noise_amplitude,
                frequency: self.rotation_noise_frequency,
                axes: [self.rotation_noise_scale_x, self.rotation_noise_scale_y],
                offset: [self.rotation_noise_offset_x, self.rotation_noise_offset_y],
                evolution: self.rotation_noise_evolution,
                cycle: self.rotation_noise_loop.clamp(0.0, 1_000_000.0),
                line_offset: self.rotation_noise_line_offset,
                line_steps_phase: true,
                field: Perlin::new(self.rotation_noise_seed as u32),
                ramp: Ramp::new(
                    self.rotation_noise_ramp,
                    self.rotation_noise_ramp_offset,
                    self.rotation_noise_ramp_scale,
                    self.rotation_noise_ramp_loop,
                ),
            },
            alpha: Noise {
                coordinates: self.alpha_noise_coordinates,
                amplitude: self.alpha_noise_amplitude,
                frequency: self.alpha_noise_frequency,
                axes: [self.alpha_noise_scale_x, self.alpha_noise_scale_y],
                offset: [self.alpha_noise_offset_x, self.alpha_noise_offset_y],
                evolution: self.alpha_noise_evolution,
                cycle: self.alpha_noise_loop.clamp(0.0, 1_000_000.0),
                line_offset: self.alpha_noise_line_offset,
                line_steps_phase: true,
                field: Perlin::new(self.alpha_noise_seed as u32),
                ramp: Ramp::new(
                    self.alpha_noise_ramp,
                    self.alpha_noise_ramp_offset,
                    self.alpha_noise_ramp_scale,
                    self.alpha_noise_ramp_loop,
                ),
            },
            density: [
                Ramp::new(self.mask_1_density, 0.0, 100.0, false),
                Ramp::new(self.mask_2_density, 0.0, 100.0, false),
            ],
            guides: [
                Guide {
                    thickness: self.mask_1_thickness,
                    thickness_ramp: Ramp::new(
                        self.mask_1_thickness_ramp,
                        self.mask_1_thickness_ramp_offset,
                        self.mask_1_thickness_ramp_scale,
                        self.mask_1_thickness_ramp_loop,
                    ),
                    feather: self.mask_1_feather,
                    opacity: self.mask_1_opacity,
                    opacity_ramp: Ramp::new(
                        self.mask_1_opacity_ramp,
                        self.mask_1_opacity_ramp_offset,
                        self.mask_1_opacity_ramp_scale,
                        self.mask_1_opacity_ramp_loop,
                    ),
                    gradient: Gradient {
                        ramp: Ramp::new(
                            self.mask_1_gradient,
                            self.mask_1_gradient_offset,
                            100.0,
                            self.mask_1_gradient_loop,
                        ),
                        colors: [
                            self.mask_1_colour_1,
                            self.mask_1_colour_2,
                            self.mask_1_colour_3,
                            self.mask_1_colour_4,
                            self.mask_1_colour_5,
                        ],
                        count: self.mask_1_colours_count.clamp(1, 5) as usize,
                    },
                    gradient_random: self.mask_1_gradient_random,
                    sine_operation: self.mask_1_sine_operation,
                    sines: [
                        Sine {
                            amplitude: self.mask_1_sine_1_amplitude,
                            frequency: self.mask_1_sine_1_frequency,
                            offset: self.mask_1_sine_1_offset,
                        },
                        Sine {
                            amplitude: self.mask_1_sine_2_amplitude,
                            frequency: self.mask_1_sine_2_frequency,
                            offset: self.mask_1_sine_2_offset,
                        },
                    ],
                },
                Guide {
                    thickness: self.mask_2_thickness,
                    thickness_ramp: Ramp::new(
                        self.mask_2_thickness_ramp,
                        self.mask_2_thickness_ramp_offset,
                        self.mask_2_thickness_ramp_scale,
                        self.mask_2_thickness_ramp_loop,
                    ),
                    feather: self.mask_2_feather,
                    opacity: self.mask_2_opacity,
                    opacity_ramp: Ramp::new(
                        self.mask_2_opacity_ramp,
                        self.mask_2_opacity_ramp_offset,
                        self.mask_2_opacity_ramp_scale,
                        self.mask_2_opacity_ramp_loop,
                    ),
                    gradient: Gradient {
                        ramp: Ramp::new(
                            self.mask_2_gradient,
                            self.mask_2_gradient_offset,
                            100.0,
                            self.mask_2_gradient_loop,
                        ),
                        colors: [
                            self.mask_2_colour_1,
                            self.mask_2_colour_2,
                            self.mask_2_colour_3,
                            self.mask_2_colour_4,
                            self.mask_2_colour_5,
                        ],
                        count: self.mask_2_colours_count.clamp(1, 5) as usize,
                    },
                    gradient_random: self.mask_2_gradient_random,
                    sine_operation: self.mask_2_sine_operation,
                    sines: [
                        Sine {
                            amplitude: self.mask_2_sine_1_amplitude,
                            frequency: self.mask_2_sine_1_frequency,
                            offset: self.mask_2_sine_1_offset,
                        },
                        Sine {
                            amplitude: self.mask_2_sine_2_amplitude,
                            frequency: self.mask_2_sine_2_frequency,
                            offset: self.mask_2_sine_2_offset,
                        },
                    ],
                },
            ],
            cross: Gradient {
                ramp: Ramp::new(
                    self.cross_gradient,
                    self.cross_gradient_offset,
                    self.cross_gradient_scale,
                    self.cross_gradient_loop,
                ),
                colors: [
                    self.cross_colour_1,
                    self.cross_colour_2,
                    self.cross_colour_3,
                    self.cross_colour_4,
                    self.cross_colour_5,
                ],
                count: self.cross_colours_count.clamp(1, 5) as usize,
            },
            dry_ramp: Ramp::new(self.dry_brush_ramp, 0.0, 100.0, false),
            dry_field: Perlin::new(self.random_seed.clamp(0, 32768) as u32 ^ 0x0d12),
            rough_field: Perlin::new(self.random_seed.clamp(0, 32768) as u32 ^ 0x7a3e),
        }
    }
}
