//! How a composition's layers mix with each other.

use serde::{Deserialize, Serialize};

/// The values a composition's layers, opacity, blend modes and gradients
/// mix in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompBlending {
    /// Linear light, where two lights add as they do in the world.
    #[default]
    Linear,
    /// Display sRGB values, as a browser, a design tool and After Effects'
    /// 8 and 16 bit modes mix them. A picture made in one of those looks the
    /// same here. The comp's own picture is still linear, so whatever it is
    /// placed in, exported to or graded with reads it as before.
    Display,
}

impl CompBlending {
    #[allow(clippy::trivially_copy_pass_by_ref)]
    pub(super) fn is_default(&self) -> bool {
        *self == Self::Linear
    }
}
