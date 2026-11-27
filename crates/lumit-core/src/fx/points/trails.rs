/// A Particulate with Persistence above nought: the card draws every frame
/// from the in point again, each from the document at its own time.
#[derive(Debug, Clone, PartialEq)]
pub struct Trails {
    pub key: [u8; 32],
    pub doc: std::sync::Arc<crate::model::Document>,
    pub comp: uuid::Uuid,
    pub layer: uuid::Uuid,
    pub clone: (u32, u32),
    /// How far the room round the art moves the points in, px@comp.
    pub inset: [f32; 2],
    /// The picture a Sprite layer stamps, named by what is in it. `None` for
    /// one nothing names, and then the walk keeps no pictures.
    pub sprite: Option<u128>,
    pub sim: bool,
}
