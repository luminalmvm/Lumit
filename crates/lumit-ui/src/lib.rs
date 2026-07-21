//! Lumit's UI shell (egui). Engine crates must never depend on this crate —
//! the dependency arrow points the other way (docs/05-ARCHITECTURE.md).

pub mod app_state;
pub mod export;
pub mod fxops;
pub mod headless;
pub mod icons;
pub mod native_menu;
pub mod pixels;
pub mod preset;
pub mod shell;
pub mod splash;
pub mod theme;

pub use shell::Shell;
pub use theme::Theme;
