//! Stabilise: taking the shake out of a shot, and putting it back.
//!
//! In plain terms: an analysis follows specks through the clip once, on its own
//! thread, and works out how the whole picture moved from each frame to the
//! next ([`analysis`], built by `lumit-track`). Everything after that is quick
//! and lives here: the camera's path is chained from those motions and smoothed
//! ([`path`]), each frame is moved onto the smooth path, and the edges that
//! drags into view are cropped, scaled away, left ([`frame`]) or filled from
//! the frames either side ([`fill`]). A sensor read row by row is undone per
//! row from the frame's own motion ([`shutter`]). The [`plan`] ties those
//! together for one effect's rows and keeps the result, so the render only
//! looks a frame up.
//!
//! Pure computation: no IO, no clock, deterministic. The tables in
//! [`analysis`] and [`plan`] hand out `Arc`s and are never locked across work.

pub mod analysis;
pub mod drop;
pub mod fill;
pub mod frame;
pub mod keys;
pub mod motion;
pub mod path;
pub mod place;
pub mod plan;
pub mod points;
pub mod resolve;
pub mod rows;
pub mod shown;
pub mod shutter;
pub mod warp;
