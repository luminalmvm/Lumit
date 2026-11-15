//! What a composition gets when nobody chose.

use super::*;

impl Default for CompSettings {
    /// What a comp gets when nobody chose: 1920×1080, 60 fps, 30 seconds,
    /// black, and the model's own shutter. One place decides what a comp is.
    fn default() -> Self {
        use lumit_core::time::{Duration, FrameRate};
        let blur = lumit_core::model::MotionBlur::default();
        CompSettings {
            name: String::new(),
            width: 1920,
            height: 1080,
            frame_rate: FrameRate::new(60, 1).unwrap_or(FrameRate::FPS_25),
            duration: Duration(Rational::new(30, 1).unwrap_or(Rational::ONE)),
            background: [0.0, 0.0, 0.0, 1.0],
            shutter_angle: blur.shutter_angle,
            motion_blur_samples: blur.samples,
            motion_blur_limit: blur.limit,
        }
    }
}
