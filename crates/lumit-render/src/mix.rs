//! The comp's sound: the jobs a document walk hands the mixer, the gain and
//! rack bakes, the bus stage, the desk built from them and the runs that play
//! it offline. Plan building and mixdown run on the thread that asks for
//! them, never the realtime callback.

pub mod bake;
pub mod graph;
pub mod jobs;
pub mod live;
pub mod mixdown;
mod order;
pub mod peaks;
pub mod rack_stream;
pub mod render;
pub mod stage;
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;
mod whole;
