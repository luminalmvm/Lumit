//! The project every test starts from: a lower third with a text layer, a
//! shape layer with a mask, a solid carrying four effects and a Precomp layer
//! showing a logo, made a template the way the Controls panel makes one.

use std::sync::Arc;

use lumit_core::anim::Property;
use lumit_core::edits::base_layer;
use lumit_core::expression::ExpressionContext;
use lumit_core::fx::builtins;
use lumit_core::mask::Mask;
use lumit_core::model::{
    Composition, Document, EffectInstance, Layer, LayerKind, LinearColour, ProjectItem, SolidDef,
    TextDocument, TransformGroup, TransformProp,
};
use lumit_core::ops::{self, Op};
use lumit_core::sequence::{Clip, ClipSource};
use lumit_core::shape::modifier::ShapeEntry;
use lumit_core::shape::ShapeItem;
use lumit_core::template::{author, Control, Target, Template};
use lumit_core::time::{Duration, FrameRate, Rational};
use uuid::Uuid;

pub fn secs(n: i64) -> Rational {
    Rational::new(n, 1).unwrap()
}

pub fn comp(name: &str, width: u32, height: u32, seconds: i64, layers: Vec<Layer>) -> Composition {
    Composition {
        graph: None,
        master_volume_db: 0.0,
        master_keys: None,
        master_automation: Default::default(),
        loudness_target: None,
        lighting: Default::default(),
        sound_mix: false,
        groups: Vec::new(),
        beat_grid: None,
        id: Uuid::now_v7(),
        name: name.into(),
        width,
        height,
        frame_rate: FrameRate::new(30, 1).unwrap(),
        duration: Duration(secs(seconds)),
        background: LinearColour::BLACK,
        work_area: None,
        layers,
        markers: Vec::new(),
        motion_blur: Default::default(),
        extra: serde_json::Map::new(),
    }
}

fn layer(name: &str, kind: LayerKind) -> Layer {
    base_layer(name.into(), kind, secs(5), TransformGroup::default())
}

/// The project, and the ids the tests reach for.
pub struct Kit {
    pub doc: Document,
    /// The lower third.
    pub tpl: Uuid,
    /// Its text layer, saying "Hello" in Inter Regular at 72 px.
    pub title: Uuid,
    /// Its shape layer, one blue rectangle 400 by 80, with a mask.
    pub bar: Uuid,
    pub item: Uuid,
    pub mask: Uuid,
    /// Its solid at 960, 540, carrying Fill, Gaussian blur, Broadcast safe
    /// and a Point control.
    pub plate: Uuid,
    pub fill: Uuid,
    pub blur: Uuid,
    pub safe: Uuid,
    pub point: Uuid,
    /// Its Precomp layer showing the logo, 100 by 50.
    pub logo: Uuid,
    pub logo_comp: Uuid,
    /// A second picture, 200 by 200, for a copy to put in the logo's place.
    pub badge: Uuid,
}

impl Kit {
    pub fn comp(&self) -> &Composition {
        self.doc.comp(self.tpl).unwrap()
    }

    pub fn template(&self) -> Template {
        (*self.comp().template().unwrap()).clone()
    }
}

/// The control `id` of `template`.
pub fn control<'a>(template: &'a Template, id: &str) -> &'a Control {
    template.control(id).unwrap()
}

pub fn kit() -> Kit {
    let mut doc = Document::new();
    let solid = Uuid::now_v7();
    doc.items.push(ProjectItem::Solid(SolidDef {
        id: solid,
        name: "plate".into(),
        colour: LinearColour([1.0; 4]),
        width: 1920,
        height: 1080,
        extra: serde_json::Map::new(),
    }));
    let logo_comp = comp("logo", 100, 50, 5, Vec::new());
    let badge = comp("badge", 200, 200, 5, Vec::new());

    let mut document = TextDocument {
        text: "Hello".into(),
        expression: None,
        size: 72.0,
        fill: LinearColour([1.0; 4]),
        path: None,
        path_offset: Property::zero(),
        animators: Vec::new(),
        style: Box::default(),
        paragraph: Default::default(),
        captions: None,
        extra: serde_json::Map::new(),
    };
    document.style.family = "Inter".into();
    document.style.face = "Regular".into();
    let title = layer("Title", LayerKind::Text { document });

    let mut shape = ShapeItem::filled(
        "Bar",
        Mask::rectangle(0.0, 0.0, 400.0, 80.0).path,
        LinearColour([0.0, 0.0, 1.0, 1.0]),
    );
    shape.primitive = Some(
        serde_json::from_value(serde_json::json!({"kind": 0, "width": 400.0, "height": 80.0}))
            .unwrap(),
    );
    let item = shape.id;
    let mut bar = layer(
        "Bar",
        LayerKind::Shape {
            contents: vec![ShapeEntry::Item(shape)],
        },
    );
    let cut = Mask::rectangle(0.0, 0.0, 100.0, 100.0);
    let mask = cut.id;
    bar.masks.push(cut);

    let mut plate = base_layer(
        "Plate".into(),
        LayerKind::Solid { def: solid },
        secs(5),
        TransformGroup {
            position_x: Property::fixed(960.0),
            position_y: Property::fixed(540.0),
            ..TransformGroup::default()
        },
    );
    plate.effects = ["fill", "blur", "broadcast_safe", "point_control"]
        .into_iter()
        .map(|name| builtins::instantiate(name).unwrap())
        .collect();
    let [fill, blur, safe, point] = [0, 1, 2, 3].map(|n| plate.effects[n].id);

    let logo = layer("Logo", LayerKind::Precomp { comp: logo_comp.id });
    let ids = (title.id, bar.id, plate.id, logo.id);
    let inner = comp("Lower third", 1920, 1080, 5, vec![title, bar, plate, logo]);
    let comps = (inner.id, logo_comp.id, badge.id);
    for c in [logo_comp, badge, inner] {
        doc.items.push(ProjectItem::Composition(c));
    }
    Kit {
        doc,
        tpl: comps.0,
        title: ids.0,
        bar: ids.1,
        item,
        mask,
        plate: ids.2,
        fill,
        blur,
        safe,
        point,
        logo: ids.3,
        logo_comp: comps.1,
        badge: comps.2,
    }
}

/// The template the tests share, made as the Controls panel makes one: each
/// control from the row it names, its id free among those before it. The
/// title lets a copy change its font, size and colour too.
pub fn authored(kit: &Kit) -> Template {
    let (doc, c) = (&kit.doc, kit.comp());
    let mut template = Template {
        intro: Some(secs(1)),
        outro: Some(secs(4)),
        ..Template::default()
    };
    let transform = |props: Vec<TransformProp>| Target::Transform {
        layer: kit.plate,
        props,
    };
    let param = |effect: Uuid, param: &str| Target::Param {
        layer: kit.plate,
        effect,
        param: param.into(),
    };
    let rows = vec![
        (Target::Text { layer: kit.title }, "Title"),
        (transform(vec![TransformProp::Opacity]), "Opacity"),
        (
            transform(vec![TransformProp::PositionX, TransformProp::PositionY]),
            "Position",
        ),
        (param(kit.fill, "colour"), "Tint"),
        (
            Target::EffectOn {
                layer: kit.plate,
                effect: kit.blur,
            },
            "Glow on",
        ),
        (param(kit.safe, "standard"), "Style"),
        (
            Target::Source {
                layer: kit.logo,
                frame: (100, 50),
            },
            "Logo",
        ),
        (transform(vec![TransformProp::Rotation]), "Spin"),
        (Target::Font { layer: kit.title }, "Font"),
        (param(kit.blur, "radius"), "Radius"),
    ];
    for (target, label) in rows {
        let mut control = author::control_for(doc, c, target, label).unwrap();
        control.id = template.free_id(label);
        template.controls.push(control);
    }
    let title = &mut template.controls[0];
    title.font = true;
    title.size = true;
    title.colour = true;
    title.group = Some("Words".into());
    template.controls[1].group = Some("Look".into());
    template.controls[3].group = Some("Look".into());
    template
}

/// Make the lower third `template` through the op the Controls panel
/// commits, answering its undo.
pub fn install(kit: &mut Kit, template: &Template) -> Op {
    let op = Op::SetCompTemplate {
        comp: kit.tpl,
        template: Some(Box::new(template.clone())),
    };
    ops::apply(&mut kit.doc, &op).unwrap()
}

pub fn kit_with_template() -> (Kit, Template) {
    let mut kit = kit();
    let template = authored(&kit);
    install(&mut kit, &template);
    (kit, template)
}

/// A composition "Show" placing the template as a Precomp layer "Copy"
/// holding `layer_values`, and as one clip on a Sequence layer "Cut" from two
/// seconds holding `clip_values`, beside a Null "Other". Answers its id.
pub fn show(
    kit: &mut Kit,
    layer_values: Option<EffectInstance>,
    clip_values: Option<EffectInstance>,
) -> Uuid {
    let mut copy = layer("Copy", LayerKind::Precomp { comp: kit.tpl });
    copy.graph_inputs = layer_values;
    let mut clip = Clip::new(
        ClipSource::Comp(kit.tpl),
        Rational::ZERO,
        secs(5),
        secs(2),
        secs(5),
    );
    clip.inputs = clip_values;
    let cut = layer("Cut", LayerKind::Sequence { clips: vec![clip] });
    let other = layer("Other", LayerKind::Null);
    let show = comp("Show", 1920, 1080, 10, vec![copy, cut, other]);
    let id = show.id;
    kit.doc.items.push(ProjectItem::Composition(show));
    id
}

pub fn layer_named<'a>(c: &'a Composition, name: &str) -> &'a Layer {
    c.layers.iter().find(|l| l.name == name).unwrap()
}

pub fn context(doc: &Arc<Document>, comp: Uuid, layer: Uuid, time: f64) -> Arc<ExpressionContext> {
    Arc::new(ExpressionContext {
        document: doc.clone(),
        comp: Some(comp),
        layer: Some(layer),
        comp_time: time,
        current_depth: 0,
        inputs: None,
    })
}
