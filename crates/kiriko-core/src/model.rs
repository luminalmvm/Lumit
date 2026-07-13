//! The document model, Phase 0 scope (docs/03-DATA-MODEL.md).
//!
//! Phase 0 carries projects, folders, footage items, compositions, and Footage
//! layers with spans — no properties/keyframes yet (slice arrives in Phase 1).
//! All mutation goes through operations (ops.rs); this module is data + queries.

use crate::anim::Property;
use crate::time::{CompTime, Duration, FrameRate};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Linear-light RGBA (docs/10-FILE-FORMAT.md §1.1).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LinearColour(pub [f32; 4]);

impl LinearColour {
    pub const BLACK: Self = Self([0.0, 0.0, 0.0, 1.0]);
}

/// Media reference (docs/03-DATA-MODEL.md §3). Fingerprint lands in slice 4.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaRef {
    pub relative_path: String,
    pub absolute_path: String,
    /// Unknown fields from newer Kiriko versions, preserved on load/save
    /// (docs/10-FILE-FORMAT.md §1.1 — mandatory forward compatibility).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FootageItem {
    pub id: Uuid,
    pub name: String,
    pub media: MediaRef,
    /// Unknown fields from newer Kiriko versions, preserved on load/save
    /// (docs/10-FILE-FORMAT.md §1.1 — mandatory forward compatibility).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Folder {
    pub id: Uuid,
    pub name: String,
    /// Ordered children ids (docs/03-DATA-MODEL.md §2 table).
    pub children: Vec<Uuid>,
    /// Unknown fields from newer Kiriko versions, preserved on load/save
    /// (docs/10-FILE-FORMAT.md §1.1 — mandatory forward compatibility).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Composition {
    pub id: Uuid,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub frame_rate: FrameRate,
    pub duration: Duration,
    pub background: LinearColour,
    /// Preview/export span (docs/01-GLOSSARY.md: work area); None = full comp.
    #[serde(default)]
    pub work_area: Option<(CompTime, CompTime)>,
    /// Index 0 = top of the stack.
    pub layers: Vec<Layer>,
    /// Unknown fields from newer Kiriko versions, preserved on load/save
    /// (docs/10-FILE-FORMAT.md §1.1 — mandatory forward compatibility).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Layer transform (docs/03-DATA-MODEL.md §6; 2.5D fields join with the
/// camera work — all maths is 4x4 from day one at the evaluator level).
/// Dimensions are separated scalars in Phase 1 (AE's separated-dimensions
/// mode); coupled spatial paths arrive with the motion-path unit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransformGroup {
    pub anchor_x: Property,
    pub anchor_y: Property,
    pub position_x: Property,
    pub position_y: Property,
    /// Percent, 100 = natural size.
    pub scale_x: Property,
    pub scale_y: Property,
    /// Degrees.
    pub rotation: Property,
    /// Percent, 0..100.
    pub opacity: Property,
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for TransformGroup {
    fn default() -> Self {
        Self {
            anchor_x: Property::fixed(0.0),
            anchor_y: Property::fixed(0.0),
            position_x: Property::fixed(0.0),
            position_y: Property::fixed(0.0),
            scale_x: Property::fixed(100.0),
            scale_y: Property::fixed(100.0),
            rotation: Property::fixed(0.0),
            opacity: Property::fixed(100.0),
            extra: serde_json::Map::new(),
        }
    }
}

/// Which transform property an op addresses (stable, serialisable path).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransformProp {
    AnchorX,
    AnchorY,
    PositionX,
    PositionY,
    ScaleX,
    ScaleY,
    Rotation,
    Opacity,
}

impl TransformGroup {
    pub fn get(&self, prop: TransformProp) -> &Property {
        match prop {
            TransformProp::AnchorX => &self.anchor_x,
            TransformProp::AnchorY => &self.anchor_y,
            TransformProp::PositionX => &self.position_x,
            TransformProp::PositionY => &self.position_y,
            TransformProp::ScaleX => &self.scale_x,
            TransformProp::ScaleY => &self.scale_y,
            TransformProp::Rotation => &self.rotation,
            TransformProp::Opacity => &self.opacity,
        }
    }

    pub fn get_mut(&mut self, prop: TransformProp) -> &mut Property {
        match prop {
            TransformProp::AnchorX => &mut self.anchor_x,
            TransformProp::AnchorY => &mut self.anchor_y,
            TransformProp::PositionX => &mut self.position_x,
            TransformProp::PositionY => &mut self.position_y,
            TransformProp::ScaleX => &mut self.scale_x,
            TransformProp::ScaleY => &mut self.scale_y,
            TransformProp::Rotation => &mut self.rotation,
            TransformProp::Opacity => &mut self.opacity,
        }
    }
}

/// Using another layer's alpha or luma to gate this layer
/// (docs/01-GLOSSARY.md §6: matte — any layer, one matte may serve many).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatteRef {
    pub layer: Uuid,
    pub channel: MatteChannel,
    pub inverted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MatteChannel {
    Alpha,
    Luma,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Switches {
    pub visible: bool,
    pub audible: bool,
    pub locked: bool,
}

impl Default for Switches {
    fn default() -> Self {
        Self {
            visible: true,
            audible: true,
            locked: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LayerKind {
    Footage {
        item: Uuid,
    },
    /// Flat colour at the comp's size (docs/01-GLOSSARY.md: Solid layer).
    Solid {
        colour: LinearColour,
    },
    /// Another composition as this layer's source (docs/01-GLOSSARY.md:
    /// Precomp layer). Cycles are invalid states, guarded at insertion and
    /// defensively at render.
    Precomp {
        comp: Uuid,
    },
    /// Editable styled text (v1: one run — docs/03-DATA-MODEL.md §9.1).
    Text {
        document: TextDocument,
    },
}

/// v1 text: single run. Styled runs, fonts and animators follow the doc.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextDocument {
    pub text: String,
    /// Pixel size at natural scale.
    pub size: f64,
    pub fill: LinearColour,
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Per-layer composite operator — the linear subset first
/// (docs/06-RENDER-PIPELINE.md §blend domains; the perceptual set joins with
/// the ping-pong compositing pass).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BlendMode {
    #[default]
    Normal,
    Add,
    Multiply,
    Screen,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub id: Uuid,
    pub name: String,
    pub kind: LayerKind,
    pub in_point: CompTime,
    /// Exclusive; must be > in_point.
    pub out_point: CompTime,
    /// Where layer time 0 sits on the comp timeline.
    pub start_offset: CompTime,
    /// Defaulted for projects saved before transforms existed (forward compat).
    #[serde(default)]
    pub transform: TransformGroup,
    /// Matte reference; a missing/deleted target degrades to "no matte"
    /// (docs/03-DATA-MODEL.md §5.1 invariants), never an error.
    #[serde(default)]
    pub matte: Option<MatteRef>,
    #[serde(default)]
    pub blend: BlendMode,
    /// Masks gate the layer's alpha before effects/transform
    /// (docs/06-RENDER-PIPELINE.md render order).
    #[serde(default)]
    pub masks: Vec<crate::mask::Mask>,
    pub switches: Switches,
    /// Unknown fields from newer Kiriko versions, preserved on load/save
    /// (docs/10-FILE-FORMAT.md §1.1 — mandatory forward compatibility).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ProjectItem {
    Footage(FootageItem),
    Folder(Folder),
    Composition(Composition),
}

impl ProjectItem {
    pub fn id(&self) -> Uuid {
        match self {
            ProjectItem::Footage(f) => f.id,
            ProjectItem::Folder(f) => f.id,
            ProjectItem::Composition(c) => c.id,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            ProjectItem::Footage(f) => &f.name,
            ProjectItem::Folder(f) => &f.name,
            ProjectItem::Composition(c) => &c.name,
        }
    }

    pub fn set_name(&mut self, name: String) {
        match self {
            ProjectItem::Footage(f) => f.name = name,
            ProjectItem::Folder(f) => f.name = name,
            ProjectItem::Composition(c) => c.name = name,
        }
    }
}

/// The whole editable document (docs/01-GLOSSARY.md: Project).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub id: Uuid,
    /// Flat item storage; Project panel order = Vec order, folders reference by id.
    pub items: Vec<ProjectItem>,
    /// Unknown fields from newer Kiriko versions, preserved on load/save
    /// (docs/10-FILE-FORMAT.md §1.1 — mandatory forward compatibility).
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Document {
    pub fn new() -> Self {
        Self {
            id: Uuid::now_v7(),
            items: Vec::new(),
            extra: serde_json::Map::new(),
        }
    }

    pub fn item(&self, id: Uuid) -> Option<&ProjectItem> {
        self.items.iter().find(|i| i.id() == id)
    }

    pub fn item_mut(&mut self, id: Uuid) -> Option<&mut ProjectItem> {
        self.items.iter_mut().find(|i| i.id() == id)
    }

    pub fn comp(&self, id: Uuid) -> Option<&Composition> {
        match self.item(id) {
            Some(ProjectItem::Composition(c)) => Some(c),
            _ => None,
        }
    }

    pub fn comp_mut(&mut self, id: Uuid) -> Option<&mut Composition> {
        match self.item_mut(id) {
            Some(ProjectItem::Composition(c)) => Some(c),
            _ => None,
        }
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}
