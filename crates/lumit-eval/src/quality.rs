//! The preview quality a frame is drawn at when nobody chose.

use super::Quality;

impl Default for Quality {
    fn default() -> Self {
        Self { divisor: 1 }
    }
}
