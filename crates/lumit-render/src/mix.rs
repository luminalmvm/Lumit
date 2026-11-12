//! The comp's sound: the jobs a document walk hands the mixer, the gain and
//! rack bakes, the bus stage, and the two mixers that place them, the export's
//! mixdown and the live plan playback reads. Plan building and mixdown run on
//! the thread that asks for them, never the realtime callback.

pub mod bake;
pub mod jobs;
pub mod live;
pub mod mixdown;
pub mod stage;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;
