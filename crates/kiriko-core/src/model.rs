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

/// A shared solid definition (docs/03-DATA-MODEL.md §2): solids are assets,
/// so many layers can reference one colour/size and they dedupe naturally.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SolidDef {
    pub id: Uuid,
    pub name: String,
    pub colour: LinearColour,
    pub width: u32,
    pub height: u32,
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
    /// Timeline markers (cues, chapters, detected beats — docs/03-DATA-MODEL.md
    /// §11), in no required order (snapping and drawing sort as needed).
    #[serde(default)]
    pub markers: Vec<crate::markers::Marker>,
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
    /// Degrees (z rotation — the 2D rotation).
    pub rotation: Property,
    /// 2.5D additions (K-023; serde-defaulted so pre-3D projects load).
    #[serde(default = "Property::zero")]
    pub position_z: Property,
    #[serde(default = "Property::zero")]
    pub rotation_x: Property,
    #[serde(default = "Property::zero")]
    pub rotation_y: Property,
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
            position_z: Property::fixed(0.0),
            rotation_x: Property::fixed(0.0),
            rotation_y: Property::fixed(0.0),
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
    PositionZ,
    ScaleX,
    ScaleY,
    Rotation,
    RotationX,
    RotationY,
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
            TransformProp::PositionZ => &self.position_z,
            TransformProp::RotationX => &self.rotation_x,
            TransformProp::RotationY => &self.rotation_y,
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
            TransformProp::PositionZ => &mut self.position_z,
            TransformProp::RotationX => &mut self.rotation_x,
            TransformProp::RotationY => &mut self.rotation_y,
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
    /// 2.5D: this layer positions in z and honours the active camera.
    #[serde(default)]
    pub three_d: bool,
}

impl Default for Switches {
    fn default() -> Self {
        Self {
            visible: true,
            audible: true,
            locked: false,
            three_d: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LayerKind {
    Footage {
        item: Uuid,
        /// Retime map (docs/04-RETIMING.md): local time → source time. None =
        /// no retiming (plays at source rate). Defaulted for projects saved
        /// before Retime existed.
        #[serde(default)]
        retime: Option<crate::retime::Retime>,
    },
    /// A SolidDef asset as this layer's source (docs/01-GLOSSARY.md: Solid
    /// layer; docs/03-DATA-MODEL.md §5.2 — solids are assets so they dedupe).
    Solid { def: Uuid },
    /// Another composition as this layer's source (docs/01-GLOSSARY.md:
    /// Precomp layer). Cycles are invalid states, guarded at insertion and
    /// defensively at render.
    Precomp { comp: Uuid },
    /// Editable styled text (v1: one run — docs/03-DATA-MODEL.md §9.1).
    Text { document: TextDocument },
    /// A 3D viewpoint (docs/01-GLOSSARY.md: Camera layer). Only affects
    /// layers with the 3D switch; the topmost visible camera is active.
    /// `zoom` is the AE model: focal distance in comp pixels — the z=0
    /// plane maps 1:1.
    Camera { zoom: Property },
    /// A Sequence layer (docs/01-GLOSSARY.md, §5.3): clips cut back-to-back on
    /// one row — Kiriko's Vegas-style editing surface. Resolution lives in
    /// [`crate::sequence`].
    Sequence {
        #[serde(default)]
        clips: Vec<crate::sequence::Clip>,
    },
}

/// The active camera's evaluated placement at one comp time — what both the
/// preview and the export pipeline hand to the GPU camera matrix, so the two
/// can never disagree (K-031).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraPose {
    /// Focal distance in comp pixels (the z=0 plane maps 1:1).
    pub zoom: f64,
    pub position: (f64, f64, f64),
    /// (x, y, z) rotation in degrees.
    pub rotation_deg: (f64, f64, f64),
}

impl Composition {
    /// The topmost visible Camera layer whose span contains `t`, evaluated at
    /// its layer time. None → the comp renders flat (3D switches ignored).
    pub fn camera_pose(&self, t: f64) -> Option<CameraPose> {
        self.layers.iter().find_map(|l| {
            let LayerKind::Camera { zoom } = &l.kind else {
                return None;
            };
            if !l.switches.visible || t < l.in_point.0.to_f64() || t >= l.out_point.0.to_f64() {
                return None;
            }
            let lt = t - l.start_offset.0.to_f64();
            let tr = &l.transform;
            Some(CameraPose {
                zoom: zoom.value_at(lt),
                position: (
                    tr.position_x.value_at(lt),
                    tr.position_y.value_at(lt),
                    tr.position_z.value_at(lt),
                ),
                rotation_deg: (
                    tr.rotation_x.value_at(lt),
                    tr.rotation_y.value_at(lt),
                    tr.rotation.value_at(lt),
                ),
            })
        })
    }
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
    Overlay,
    SoftLight,
    HardLight,
    Lighten,
    Darken,
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
    Solid(SolidDef),
}

impl ProjectItem {
    pub fn id(&self) -> Uuid {
        match self {
            ProjectItem::Footage(f) => f.id,
            ProjectItem::Folder(f) => f.id,
            ProjectItem::Composition(c) => c.id,
            ProjectItem::Solid(s) => s.id,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            ProjectItem::Footage(f) => &f.name,
            ProjectItem::Folder(f) => &f.name,
            ProjectItem::Composition(c) => &c.name,
            ProjectItem::Solid(s) => &s.name,
        }
    }

    pub fn set_name(&mut self, name: String) {
        match self {
            ProjectItem::Footage(f) => f.name = name,
            ProjectItem::Folder(f) => f.name = name,
            ProjectItem::Composition(c) => c.name = name,
            ProjectItem::Solid(s) => s.name = name,
        }
    }
}

/// The folders Kiriko files new assets into automatically: the first solid
/// creates a "Solids" folder, the first comp a "Compositions" folder, and
/// later ones follow the folder by id — so renaming or nesting the folder
/// doesn't break the habit. A deleted folder is simply recreated on next use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AutoFolders {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solids: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compositions: Option<Uuid>,
}

/// The whole editable document (docs/01-GLOSSARY.md: Project).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub id: Uuid,
    /// Flat item storage; Project panel order = Vec order, folders reference by id.
    pub items: Vec<ProjectItem>,
    /// Where new solids/comps are filed (see [`AutoFolders`]).
    #[serde(default)]
    pub auto_folders: AutoFolders,
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
            auto_folders: AutoFolders::default(),
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

    pub fn solid(&self, id: Uuid) -> Option<&SolidDef> {
        match self.item(id) {
            Some(ProjectItem::Solid(s)) => Some(s),
            _ => None,
        }
    }

    pub fn folder(&self, id: Uuid) -> Option<&Folder> {
        match self.item(id) {
            Some(ProjectItem::Folder(f)) => Some(f),
            _ => None,
        }
    }

    /// Ids that sit at the Project panel root: every item not referenced as
    /// any folder's child (missing children are ignored, never an error).
    pub fn root_items(&self) -> Vec<Uuid> {
        let mut in_folder = std::collections::HashSet::new();
        for item in &self.items {
            if let ProjectItem::Folder(f) = item {
                in_folder.extend(f.children.iter().copied());
            }
        }
        self.items
            .iter()
            .map(|i| i.id())
            .filter(|id| !in_folder.contains(id))
            .collect()
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::time::{CompTime, Rational};

    fn secs(s: i64) -> CompTime {
        CompTime(Rational::new(s, 1).unwrap())
    }

    fn comp_with_cameras() -> Composition {
        let mut comp = Composition {
            id: Uuid::now_v7(),
            name: "cam test".into(),
            width: 1920,
            height: 1080,
            frame_rate: FrameRate::new(60, 1).unwrap(),
            duration: Duration(Rational::new(10, 1).unwrap()),
            background: LinearColour([0.0, 0.0, 0.0, 1.0]),
            work_area: None,
            layers: Vec::new(),
            markers: Vec::new(),
            extra: serde_json::Map::new(),
        };
        let cam = |name: &str, zoom: f64, z_pos: f64, visible: bool, in_s: i64, out_s: i64| Layer {
            id: Uuid::now_v7(),
            name: name.into(),
            kind: LayerKind::Camera {
                zoom: Property::fixed(zoom),
            },
            in_point: secs(in_s),
            out_point: secs(out_s),
            start_offset: secs(0),
            transform: TransformGroup {
                position_z: Property::fixed(z_pos),
                ..TransformGroup::default()
            },
            matte: None,
            blend: BlendMode::Normal,
            masks: Vec::new(),
            switches: Switches {
                visible,
                ..Switches::default()
            },
            extra: serde_json::Map::new(),
        };
        comp.layers.push(cam("hidden", 500.0, -10.0, false, 0, 10));
        comp.layers.push(cam("short", 800.0, -20.0, true, 2, 4));
        comp.layers.push(cam("main", 1200.0, -30.0, true, 0, 10));
        comp
    }

    /// The topmost visible in-span camera wins; hidden and out-of-span ones
    /// never do; no camera at all → None (flat comp).
    #[test]
    fn camera_pose_picks_topmost_visible_in_span() {
        let comp = comp_with_cameras();
        // t=1: "hidden" is invisible, "short" not yet in span → "main".
        let pose = comp.camera_pose(1.0).unwrap();
        assert_eq!(pose.zoom, 1200.0);
        assert_eq!(pose.position.2, -30.0);
        // t=3: "short" is topmost visible in-span.
        let pose = comp.camera_pose(3.0).unwrap();
        assert_eq!(pose.zoom, 800.0);
        assert_eq!(pose.position.2, -20.0);
        // Out point is exclusive.
        assert_eq!(comp.camera_pose(4.0).unwrap().zoom, 1200.0);
        // No cameras → flat.
        let mut flat = comp_with_cameras();
        flat.layers.clear();
        assert!(flat.camera_pose(1.0).is_none());
    }
}
