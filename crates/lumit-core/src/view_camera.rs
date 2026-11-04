//! The Camera layer a Viewer's 3D view is drawn through. The view puts one on
//! top of a patched copy of the comp, so it is the active camera for the render
//! and the frame key alike, and anything wanting the comp's own camera skips it.

/// Its id: fixed, so every render through the same view builds the same
/// document and names the same frames.
pub const ID: uuid::Uuid = uuid::Uuid::from_u128(0x6c75_6d69_7420_7669_6577_2063_616d_6572);
