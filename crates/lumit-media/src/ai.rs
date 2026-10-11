//! Reading an Illustrator document one layer at a time.
//!
//! # In plain terms
//!
//! An `.ai` file saved the usual way holds the artwork twice: once in
//! Illustrator's own form, and once as a PDF, which is the copy every other
//! program reads. ffmpeg opens neither. This module reads the PDF. [`open`]
//! lists the top-level layers of the first artboard with their names and
//! whether they are switched on, and [`read_layer`] draws one of them as an
//! ordinary RGBA frame the size of the artboard, so a layer sits where it sat
//! in Illustrator with no transform at all.
//!
//! Illustrator writes each top-level layer as a part of the page that a PDF
//! reader can switch off. A layer is drawn on its own by switching the others
//! off and drawing the page. That is done to a copy of the file's bytes in
//! memory, with a new list of switches added on the end the way a PDF editor
//! saves a change.
//!
//! The artwork is vectors, and the frame is pixels: a point in Illustrator is
//! a pixel here, which is the size After Effects gives the same file.
//!
//! Not read, with an error that says so: a file with no PDF copy, which is one
//! from before Illustrator 9. A file saved with Create PDF Compatible File
//! switched off opens as the one page of text Illustrator leaves in its place.
//!
//! The PDF itself is parsed and drawn by `hayro`. It is a stranger's bytes
//! going through somebody else's code, and a file can ask for more memory
//! than its stated size lets on. So the reading is done in a helper program,
//! `lumit-media-broker`, with a cap on its memory and a limit on its time:
//! see `helper`. A file that breaks the reader ends the helper, and the
//! read comes back as an error.

use std::path::Path;

use hayro::hayro_interpret::util::TransformExt;
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::content::ops::TypedInstruction;
use hayro::hayro_syntax::object::{Array, Dict, Name, Object, ObjectIdentifier};
use hayro::hayro_syntax::Pdf;
use hayro::kurbo::Affine;
use hayro::vello_cpu::color::palette::css::TRANSPARENT;
use hayro::vello_cpu::peniko::ImageAlphaType;
use hayro::vello_cpu::{Pixmap, RasterizerSettings, RenderContext, Resources, TargetInit};
use hayro::{RenderCache, RenderSettings};
use lumit_ingress::{checked_raster_bytes, Budget, Limits};

use crate::decode::{DecodedFrame, PixelFormat};
use crate::probe::{MediaProbe, VideoInfo};
use crate::MediaError;

mod helper;
pub use helper::{serve, BROKER_EXE_ENV};

/// What one read may spend: a picture's worth of bytes, and more layers than
/// anybody draws on.
const LIMITS: Limits = Limits::IMAGE;

/// The longest file that is read. All of it is held in memory while a layer
/// is drawn, and a document heavy with placed pictures runs to hundreds of
/// megabytes.
const MAX_FILE: u64 = 1 << 30;

/// The longest side that is drawn, in pixels. It is the largest composition
/// there is, and a little over Illustrator's own canvas.
const MAX_SIDE: u32 = 16_384;

/// One top-level layer.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AiLayer {
    pub name: String,
    pub visible: bool,
    /// The object in the PDF that switches this layer on and off.
    switch: (i32, i32),
}

/// An artboard's size and its layer list.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AiDocument {
    pub width: u32,
    pub height: u32,
    /// Bottom first. A layer's place in this list is the index [`read_layer`]
    /// takes. Empty for a file whose layers can't be told apart, which is
    /// still a picture: [`read_layer`] draws all of it.
    pub layers: Vec<AiLayer>,
}

fn bad(what: &'static str) -> MediaError {
    MediaError::Ai(what)
}

/// The reason for a file nothing more can be said about.
const UNREADABLE: &str = "the file could not be read";

/// Every reason the reader gives. The helper's comes back as text, and this
/// is how the caller tells which one it was. One left out of here comes back
/// as [`UNREADABLE`].
const REASONS: [&str; 7] = [
    UNREADABLE,
    "the file has no PDF copy of the artwork to read",
    "the file is damaged, or locked with a password",
    "no artboard",
    "the artboard size is out of range",
    "the layers can't be told apart",
    "no such layer",
];

/// Whether this path names an Illustrator document, by extension.
#[must_use]
pub fn is_ai(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("ai"))
}

/// The first artboard's size and layer list. Nothing is drawn.
pub fn open(path: &Path) -> Result<AiDocument, MediaError> {
    helper::open(path)
}

/// [`open`], in this process. Only the helper does this to a file it is given.
fn open_here(path: &Path) -> Result<AiDocument, MediaError> {
    let bytes = lumit_ingress::read_capped(path, MAX_FILE)?;
    guarded(move || Ok(parse(bytes)?.1))
}

/// What a probe of the file answers: a still the size of the artboard.
pub fn probe(path: &Path) -> Result<MediaProbe, MediaError> {
    let doc = open(path)?;
    Ok(MediaProbe {
        duration_seconds: 0.0,
        container: "ai".into(),
        video: Some(VideoInfo {
            width: doc.width,
            height: doc.height,
            // What ffmpeg says of any other still.
            fps_num: 25,
            fps_den: 1,
            codec: "ai".into(),
        }),
        audio: None,
    })
}

/// One layer drawn on a transparent frame the size of the artboard, or every
/// layer that is switched on when `index` is `None`.
///
/// `index` counts through [`AiDocument::layers`]. The layer is drawn whether
/// or not it is switched on in the file. `target_width` draws the frame that
/// much narrower, keeping its shape. It never draws it wider.
pub fn read_layer(
    path: &Path,
    index: Option<u32>,
    target_width: Option<u32>,
) -> Result<DecodedFrame, MediaError> {
    helper::read_layer(path, index, target_width, helper::DEADLINE)
}

/// [`read_layer`] with the helper given `deadline` to answer in, so a test
/// can watch one run out of time.
#[cfg(any(test, feature = "test-fixtures"))]
pub fn read_layer_within(
    path: &Path,
    index: Option<u32>,
    target_width: Option<u32>,
    deadline: std::time::Duration,
) -> Result<DecodedFrame, MediaError> {
    helper::read_layer(path, index, target_width, deadline)
}

/// [`read_layer`], in this process. Only the helper does this to a file it is
/// given.
fn read_layer_here(
    path: &Path,
    index: Option<u32>,
    target_width: Option<u32>,
) -> Result<DecodedFrame, MediaError> {
    let bytes = lumit_ingress::read_capped(path, MAX_FILE)?;
    guarded(move || draw(bytes, index, target_width))
}

/// Run `read`, turning a panic inside the PDF reader into an error. The
/// workspace's own code can't panic, and this is the one place that calls into
/// a parser that may.
fn guarded<T>(read: impl FnOnce() -> Result<T, MediaError>) -> Result<T, MediaError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(read))
        .unwrap_or_else(|_| Err(bad(UNREADABLE)))
}

/// The PDF in `bytes` and what it holds.
fn parse(bytes: Vec<u8>) -> Result<(Pdf, AiDocument), MediaError> {
    if !bytes.starts_with(b"%PDF-") {
        return Err(bad("the file has no PDF copy of the artwork to read"));
    }
    let pdf = Pdf::new(bytes).map_err(|_| bad("the file is damaged, or locked with a password"))?;
    let doc = describe(&pdf)?;
    Ok((pdf, doc))
}

/// The size of a page as the frame it is drawn on.
fn frame_size(pdf: &Pdf) -> Result<(u32, u32), MediaError> {
    // ponytail: the first artboard only, which is what After Effects reads.
    // The upgrade is a composition for each artboard.
    let page = pdf
        .pages()
        .iter()
        .next()
        .ok_or_else(|| bad("no artboard"))?;
    let (width, height) = page.render_dimensions();
    let side = |points: f32| {
        let pixels = points.round();
        (1.0..=MAX_SIDE as f32)
            .contains(&pixels)
            .then_some(pixels as u32)
    };
    side(width)
        .zip(side(height))
        .ok_or_else(|| bad("the artboard size is out of range"))
}

fn describe(pdf: &Pdf) -> Result<AiDocument, MediaError> {
    let (width, height) = frame_size(pdf)?;
    let mut doc = AiDocument {
        width,
        height,
        layers: Vec::new(),
    };
    // With nowhere to add the new switches, the file is one picture.
    if startxref(pdf.data().as_ref()).is_none() {
        return Ok(doc);
    }
    let Some(page) = pdf.pages().iter().next() else {
        return Ok(doc);
    };
    let catalog: Dict<'_> = pdf.xref().get(pdf.xref().root_id()).unwrap_or_default();
    let config = catalog
        .get::<Dict<'_>>("OCProperties")
        .and_then(|all| all.get::<Dict<'_>>("D"))
        .unwrap_or_default();
    let listed = |key: &str, switch: (i32, i32)| {
        config.get::<Array<'_>>(key).is_some_and(|list| {
            let mut refs = list.raw_iter().filter_map(|item| item.as_obj_ref());
            refs.any(|r| (r.obj_number, r.gen_number) == switch)
        })
    };
    let off_unless_listed = config
        .get::<Name<'_>>("BaseState")
        .is_some_and(|state| state.as_str() == "OFF");

    // A layer is a stretch of the page marked with a switch, and the page
    // draws them bottom first. A stretch inside another is a sublayer, which
    // is drawn as part of the layer it is in.
    // ponytail: top-level layers only, as Illustrator writes them. The upgrade
    // is reading the sublayers out of Illustrator's own half of the file.
    let mut budget = Budget::new(LIMITS);
    let properties = &page.resources().properties;
    let mut depth = 0usize;
    let mut operations = page.typed_operations();
    while let Some(operation) = operations.next() {
        match operation {
            TypedInstruction::BeginMarkedContentWithProperties(marked) => {
                depth += 1;
                let Object::Name(key) = marked.1 else {
                    continue;
                };
                let (Some(switch), Some(dict)) = (
                    properties.get_ref(key.as_ref()),
                    properties.get::<Dict<'_>>(key.as_ref()),
                ) else {
                    continue;
                };
                let switch = (switch.obj_number, switch.gen_number);
                let is_group = dict
                    .get::<Name<'_>>("Type")
                    .is_some_and(|kind| kind.as_str() == "OCG");
                if depth != 1 || !is_group || doc.layers.iter().any(|l| l.switch == switch) {
                    continue;
                }
                budget.take_items(1)?;
                let name = dict
                    .get::<hayro::hayro_syntax::object::String<'_>>("Name")
                    .map(|name| text(name.as_bytes()))
                    .unwrap_or_default();
                let visible = if off_unless_listed {
                    listed("ON", switch)
                } else {
                    !listed("OFF", switch)
                };
                doc.layers.push(AiLayer {
                    name,
                    visible,
                    switch,
                });
            }
            TypedInstruction::BeginMarkedContent(_) => depth += 1,
            TypedInstruction::EndMarkedContent(_) => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    Ok(doc)
}

/// A PDF text string: UTF-16 or UTF-8 behind its mark, or else one byte a
/// character.
fn text(bytes: &[u8]) -> String {
    match bytes {
        [0xfe, 0xff, rest @ ..] => {
            let units: Vec<u16> = rest
                .chunks_exact(2)
                .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                .collect();
            String::from_utf16_lossy(&units)
        }
        [0xef, 0xbb, 0xbf, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        _ => bytes.iter().map(|b| char::from(*b)).collect(),
    }
}

/// How far back from the end of a file its last `startxref` is looked for.
const TAIL: usize = 1 << 16;

/// Where the file's newest table of objects starts, which a change added on
/// the end has to point back to.
fn startxref(data: &[u8]) -> Option<usize> {
    let needle = b"startxref";
    let tail = data.get(data.len().saturating_sub(TAIL)..)?;
    let at = tail.windows(needle.len()).rposition(|w| w == needle)?;
    let digits: String = tail
        .get(at + needle.len()..)?
        .iter()
        .skip_while(|b| b.is_ascii_whitespace())
        .take_while(|b| b.is_ascii_digit())
        .map(|b| char::from(*b))
        .collect();
    digits.parse().ok()
}

/// The file's bytes with every layer but `keep` switched off: the same file,
/// and after it a new catalog and the table that names it, the way a PDF
/// editor saves a change without rewriting what was there.
fn only(pdf: &Pdf, doc: &AiDocument, keep: usize) -> Result<Vec<u8>, MediaError> {
    let data = pdf.data().as_ref();
    let root = pdf.xref().root_id();
    let catalog: Dict<'_> = pdf.xref().get(root).unwrap_or_default();
    let (Some(pages), Some(previous)) = (catalog.get_ref("Pages"), startxref(data)) else {
        return Err(bad("the layers can't be told apart"));
    };
    let list = |layers: &mut dyn Iterator<Item = &AiLayer>| {
        let refs: Vec<String> = layers
            .map(|l| format!("{} {} R", l.switch.0, l.switch.1))
            .collect();
        refs.join(" ")
    };
    let all = list(&mut doc.layers.iter());
    let on = list(&mut doc.layers.iter().skip(keep).take(1));
    let off = list(
        &mut doc
            .layers
            .iter()
            .enumerate()
            .filter_map(|(i, l)| (i != keep).then_some(l)),
    );

    let mut budget = Budget::new(LIMITS);
    let mut out = budget.vec_with_capacity::<u8>(data.len() + all.len() * 2 + 512)?;
    out.extend_from_slice(data);
    out.push(b'\n');
    let object = out.len();
    let ObjectIdentifier {
        obj_number: number,
        gen_number: generation,
    } = root;
    out.extend_from_slice(
        format!(
            "{number} {generation} obj\n<</Type/Catalog/Pages {} {} R\
             /OCProperties<</OCGs[{all}]/D<</ON[{on}]/OFF[{off}]>>>>>>\nendobj\n",
            pages.obj_number, pages.gen_number,
        )
        .as_bytes(),
    );
    let table = out.len();
    out.extend_from_slice(
        format!(
            "xref\n{number} 1\n{object:010} {generation:05} n \n\
             trailer\n<</Size {}/Root {number} {generation} R/Prev {previous}>>\n\
             startxref\n{table}\n%%EOF\n",
            i64::from(number) + 1,
        )
        .as_bytes(),
    );
    Ok(out)
}

/// Whether `pdf` is a file [`only`] made, read the way it was meant: its
/// catalog is the new one, with every layer but one switched off.
fn took(pdf: &Pdf, layers: usize) -> bool {
    let catalog: Dict<'_> = pdf.xref().get(pdf.xref().root_id()).unwrap_or_default();
    let off = catalog
        .get::<Dict<'_>>("OCProperties")
        .and_then(|all| all.get::<Dict<'_>>("D"))
        .and_then(|config| config.get::<Array<'_>>("OFF"));
    off.is_some_and(|off| off.raw_iter().count() + 1 == layers)
}

fn draw(
    bytes: Vec<u8>,
    index: Option<u32>,
    target_width: Option<u32>,
) -> Result<DecodedFrame, MediaError> {
    let (mut pdf, doc) = parse(bytes)?;
    // A file with no layers to tell apart is all one layer.
    if let Some(index) = index.filter(|_| !doc.layers.is_empty()) {
        let keep = usize::try_from(index)
            .ok()
            .filter(|keep| *keep < doc.layers.len())
            .ok_or_else(|| bad("no such layer"))?;
        pdf =
            Pdf::new(only(&pdf, &doc, keep)?).map_err(|_| bad("the layers can't be told apart"))?;
        if !took(&pdf, doc.layers.len()) {
            return Err(bad("the layers can't be told apart"));
        }
    }
    let page = pdf
        .pages()
        .iter()
        .next()
        .ok_or_else(|| bad("no artboard"))?;

    let width = target_width
        .filter(|w| *w < doc.width && *w >= 1)
        .unwrap_or(doc.width);
    let narrower = f64::from(width) / f64::from(doc.width);
    let height = ((f64::from(doc.height) * narrower).round() as u32).max(1);
    // The renderer works on a frame of its own, so the picture is held twice
    // for a moment.
    let mut budget = Budget::new(LIMITS);
    budget.take_bytes(checked_raster_bytes(
        u64::from(width),
        u64::from(height),
        4,
        2,
    )?)?;
    let (Ok(wide), Ok(high)) = (u16::try_from(width), u16::try_from(height)) else {
        return Err(bad("the artboard size is out of range"));
    };

    let mut context = RenderContext::new(wide, high);
    // An artboard is rarely a whole number of points across, so the page is
    // fitted to the frame and not scaled by one number.
    let (points_wide, points_high) = page.render_dimensions();
    let fit = Affine::scale_non_uniform(
        f64::from(width) / f64::from(points_wide).max(1.0),
        f64::from(height) / f64::from(points_high).max(1.0),
    );
    let transform = fit * page.initial_transform(true).to_kurbo();
    hayro::render_into(
        page,
        &RenderCache::new(),
        &InterpreterSettings::default(),
        &RenderSettings::default(),
        &mut context,
        transform,
    );
    context.flush();
    let mut pixmap = Pixmap::new(wide, high);
    context.render_with(
        &mut pixmap,
        &mut Resources::default(),
        RasterizerSettings {
            target_init: TargetInit::Clear(TRANSPARENT),
            ..Default::default()
        },
    );
    // A layer is mostly empty, whatever the renderer worked out about it.
    pixmap.set_may_have_transparency(true);
    Ok(DecodedFrame {
        width,
        height,
        rgba: pixmap.take_rgba8(ImageAlphaType::Alpha),
        format: PixelFormat::Srgb8,
    })
}

// ---------------------------------------------------------------------------
// A writer, for tests
// ---------------------------------------------------------------------------

/// A small Illustrator document written from scratch, so tests here and in the
/// crates above need no file on disk.
#[cfg(any(test, feature = "test-fixtures"))]
pub mod fixture {
    /// One layer to write: a rectangle of one colour.
    #[derive(Debug, Clone)]
    pub struct Layer {
        pub name: String,
        /// Top, left, bottom, right, in pixels from the top left.
        pub rect: [u32; 4],
        /// Red, green and blue from 0 to 255.
        pub colour: [u8; 3],
        pub visible: bool,
    }

    impl Layer {
        #[must_use]
        pub fn solid(name: &str, rect: [u32; 4], colour: [u8; 3]) -> Self {
            Self {
                name: name.to_owned(),
                rect,
                colour,
                visible: true,
            }
        }
    }

    /// The bytes of a document `width` by `height` holding `layers`, bottom
    /// first, each one a top-level layer the way Illustrator writes it.
    #[must_use]
    pub fn document(width: u32, height: u32, layers: &[Layer]) -> Vec<u8> {
        // Objects 1 to 4 are the catalog, the page list, the page and what
        // the page draws. A layer's switch is object 5 and up.
        let switch = |i: usize| format!("{} 0 R", 5 + i);
        let refs = |want: bool| {
            let listed = layers.iter().enumerate();
            let refs: Vec<String> = listed
                .filter(|(_, l)| l.visible == want)
                .map(|(i, _)| switch(i))
                .collect();
            refs.join(" ")
        };
        let every: Vec<String> = (0..layers.len()).map(switch).collect();
        let mut objects = vec![
            format!(
                "<</Type/Catalog/Pages 2 0 R/OCProperties<</OCGs[{}]/D<</ON[{}]/OFF[{}]>>>>>>",
                every.join(" "),
                refs(true),
                refs(false),
            ),
            "<</Type/Pages/Kids[3 0 R]/Count 1>>".to_owned(),
        ];
        let properties: Vec<String> = (0..layers.len())
            .map(|i| format!("/MC{i} {}", switch(i)))
            .collect();
        objects.push(format!(
            "<</Type/Page/Parent 2 0 R/MediaBox[0 0 {width} {height}]/Contents 4 0 R\
             /Resources<</Properties<<{}>>>>>>",
            properties.join(""),
        ));
        let mut content = String::new();
        for (i, layer) in layers.iter().enumerate() {
            let [top, left, bottom, right] = layer.rect;
            let [red, green, blue] = layer.colour.map(|c| f64::from(c) / 255.0);
            // A PDF measures up from the bottom left.
            content.push_str(&format!(
                "/OC /MC{i} BDC\n{red} {green} {blue} rg\n{left} {} {} {} re\nf\nEMC\n",
                height.saturating_sub(bottom),
                right.saturating_sub(left),
                bottom.saturating_sub(top),
            ));
        }
        objects.push(format!(
            "<</Length {}>>\nstream\n{content}endstream",
            content.len()
        ));
        for layer in layers {
            // The name as Illustrator writes one: UTF-16 behind its mark.
            let name: String = std::iter::once(0xfeff)
                .chain(layer.name.encode_utf16())
                .map(|unit| format!("{unit:04X}"))
                .collect();
            objects.push(format!("<</Type/OCG/Name<{name}>>>"));
        }

        let mut out = b"%PDF-1.5\n".to_vec();
        let mut offsets = Vec::new();
        for (i, object) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", i + 1).as_bytes());
        }
        let table = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
        out.extend_from_slice(b"0000000000 65535 f \n");
        for offset in offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<</Size {}/Root 1 0 R>>\nstartxref\n{table}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        out
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::fixture::{document, Layer};
    use super::*;

    const RED: [u8; 3] = [255, 0, 0];
    const BLUE: [u8; 3] = [0, 0, 255];

    fn px(frame: &DecodedFrame, x: usize, y: usize) -> [u8; 4] {
        let at = (y * frame.width as usize + x) * 4;
        [
            frame.rgba[at],
            frame.rgba[at + 1],
            frame.rgba[at + 2],
            frame.rgba[at + 3],
        ]
    }

    /// An 8 by 6 artboard: a full background, a square, and a hidden square
    /// above both.
    fn sample() -> Vec<u8> {
        let mut hidden = Layer::solid("Guide", [0, 0, 2, 2], [0, 255, 0]);
        hidden.visible = false;
        let layers = [
            Layer::solid("Background", [0, 0, 6, 8], RED),
            Layer::solid("Hat \u{00e9}", [1, 2, 3, 6], BLUE),
            hidden,
        ];
        document(8, 6, &layers)
    }

    #[test]
    fn the_layer_list_carries_names_and_switches_bottom_first() {
        let (_, doc) = parse(sample()).unwrap();
        assert_eq!((doc.width, doc.height), (8, 6));
        let layers: Vec<(&str, bool)> = doc
            .layers
            .iter()
            .map(|l| (l.name.as_str(), l.visible))
            .collect();
        assert_eq!(
            layers,
            [
                ("Background", true),
                ("Hat \u{00e9}", true),
                ("Guide", false)
            ]
        );
    }

    #[test]
    fn a_layer_is_drawn_alone_where_it_sat() {
        let frame = draw(sample(), Some(1), None).unwrap();
        assert_eq!((frame.width, frame.height), (8, 6));
        assert_eq!(px(&frame, 2, 1), [0, 0, 255, 255]);
        assert_eq!(px(&frame, 5, 2), [0, 0, 255, 255]);
        assert_eq!(px(&frame, 0, 0), [0; 4], "the background is another layer");
        assert_eq!(px(&frame, 2, 3), [0; 4], "below the square is empty");

        // A hidden layer is still a layer, and is drawn when asked for.
        let hidden = draw(sample(), Some(2), None).unwrap();
        assert_eq!(px(&hidden, 1, 1), [0, 255, 0, 255]);
        assert_eq!(px(&hidden, 4, 4), [0; 4]);

        assert!(matches!(
            draw(sample(), Some(3), None),
            Err(MediaError::Ai(_))
        ));
    }

    #[test]
    fn the_whole_artboard_is_drawn_as_the_file_has_it() {
        let frame = draw(sample(), None, None).unwrap();
        assert_eq!(px(&frame, 2, 1), [0, 0, 255, 255]);
        assert_eq!(
            px(&frame, 0, 0),
            [255, 0, 0, 255],
            "the hidden layer stays hidden"
        );
    }

    #[test]
    fn a_narrower_frame_is_drawn_small_and_keeps_its_shape() {
        let frame = draw(sample(), Some(0), Some(4)).unwrap();
        assert_eq!((frame.width, frame.height), (4, 3));
        assert_eq!(px(&frame, 3, 2), [255, 0, 0, 255]);
        // Never wider than the artboard.
        assert_eq!(draw(sample(), Some(0), Some(64)).unwrap().width, 8);
    }

    #[test]
    fn a_file_with_no_layers_to_tell_apart_is_one_picture() {
        // The same page with the switches taken out of its resources.
        let bytes = String::from_utf8(sample())
            .unwrap()
            .replace("/Properties", "/Propertiez");
        let (_, doc) = parse(bytes.clone().into_bytes()).unwrap();
        assert!(doc.layers.is_empty());
        let frame = draw(bytes.into_bytes(), Some(0), None).unwrap();
        assert_eq!(px(&frame, 0, 5), [255, 0, 0, 255]);
    }

    #[test]
    fn what_is_not_read_is_refused_by_name() {
        assert!(matches!(
            parse(b"%!PS-Adobe-3.0\n%%Creator: Adobe Illustrator(R) 8.0\n".to_vec()),
            Err(MediaError::Ai(_))
        ));
        let huge = document(40_000, 6, &[Layer::solid("Wide", [0, 0, 6, 8], RED)]);
        assert!(matches!(parse(huge), Err(MediaError::Ai(_))));
    }

    /// A file is a stranger's bytes: cut short anywhere, or with any one byte
    /// changed, it reads or it errors. It never panics and never hangs.
    #[test]
    fn a_damaged_file_errors_and_never_panics() {
        let good = sample();
        let read_all = |bytes: &[u8]| {
            for index in [None, Some(0), Some(2)] {
                let _ = guarded(|| draw(bytes.to_vec(), index, None));
            }
        };
        for cut in (0..good.len()).step_by(7) {
            read_all(&good[..cut]);
        }
        for at in (0..good.len()).step_by(3) {
            for value in [0x00, b'9', b'/', 0xff] {
                let mut bytes = good.clone();
                bytes[at] = value;
                read_all(&bytes);
            }
        }
    }

    /// Nothing but this module opens the file, so the probe and the frame
    /// index have to answer for it themselves.
    #[test]
    fn the_file_probes_and_indexes_as_a_still_of_its_own_size() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("art.AI");
        std::fs::write(&path, sample()).unwrap();
        assert!(is_ai(&path));

        let probe = crate::probe::probe(path.as_path()).unwrap();
        let video = probe.video.as_ref().unwrap();
        assert_eq!((video.width, video.height), (8, 6));
        assert!(!probe.runs_as_video());
        let index = crate::index::build_frame_index(path.as_path()).unwrap();
        assert_eq!(index.frame_count(), 1);

        assert_eq!(open(&path).unwrap().layers.len(), 3);
        let frame = crate::read_own(&crate::MediaSource::file(&path), None).unwrap();
        assert_eq!(frame.unwrap().rgba.len(), 8 * 6 * 4);
    }
}
