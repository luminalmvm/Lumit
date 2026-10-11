//! Reading a Photoshop document one layer at a time.
//!
//! # In plain terms
//!
//! A `.psd` holds two things: the flattened picture, and the stack of layers
//! it was flattened from. ffmpeg reads the first and has no way to reach the
//! second. This module reads the second. [`open`] lists the layers with their
//! names, opacity, blend mode and groups, and [`read_layer`] hands back one
//! layer's pixels as an ordinary RGBA frame the size of the document, so a
//! layer sits where it sat in Photoshop with no transform at all.
//!
//! A layer mask and a clipping mask are baked into the alpha, since both are
//! part of what the layer looks like. A vector mask is too when the file holds
//! a drawn copy of it, as Photoshop's own do, and is handed over as an outline
//! when it does not. A solid colour fill layer has no pixels in the file, so
//! its picture is made here from its colour.
//!
//! Read: 8 and 16 bit, RGB and greyscale, every compression Photoshop writes.
//! An 8 bit layer comes back as sRGB bytes and a 16 bit one as linear floats.
//! Refused, with an error that says so: PSB, 32 bit, CMYK, Lab and indexed.
//!
//! Every number in the file is checked against the file's own length and
//! against a [`Budget`] before anything is allocated from it.

use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use lumit_ingress::{checked_raster_bytes, checked_usize, Budget, Limits};

use crate::decode::{DecodedFrame, PixelFormat};
use crate::MediaError;

/// What one read may spend. A picture's worth of bytes, and room for more
/// records than an image has, since a heavy document runs to thousands of
/// layers of four or five channels each.
const LIMITS: Limits = Limits {
    items: 1 << 17,
    ..Limits::IMAGE
};

/// Photoshop's own ceiling for a `.psd`, in pixels a side.
const MAX_SIDE: u32 = 30_000;

/// What a record in the layer list is: a layer, or one end of a group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Layer,
    /// A group's header. It carries the group's name, and it sits above the
    /// group's members.
    Group,
    /// The marker below a group's last member.
    GroupEnd,
}

#[derive(Debug, Clone, Copy, Default)]
struct Rect {
    top: i32,
    left: i32,
    bottom: i32,
    right: i32,
}

impl Rect {
    fn width(self) -> u64 {
        u64::try_from(i64::from(self.right) - i64::from(self.left)).unwrap_or(0)
    }

    fn height(self) -> u64 {
        u64::try_from(i64::from(self.bottom) - i64::from(self.top)).unwrap_or(0)
    }

    fn is_empty(self) -> bool {
        self.width() == 0 || self.height() == 0
    }
}

#[derive(Debug, Clone, Copy)]
struct Channel {
    id: i16,
    /// Where this channel's data starts in the file, compression word first.
    offset: u64,
    len: u64,
}

#[derive(Debug, Clone, Copy)]
struct Mask {
    rect: Rect,
    /// What the mask is outside its own rectangle: 0 or 255.
    default: u8,
    disabled: bool,
    /// Photoshop drew this from the layer's vector mask.
    rendered: bool,
}

/// A layer's vector mask: an outline that shows the layer inside it.
#[derive(Debug, Clone, PartialEq)]
pub struct VectorMask {
    /// The layer shows outside the outline instead.
    pub inverted: bool,
    pub paths: Vec<VectorPath>,
}

/// One subpath of a vector mask.
#[derive(Debug, Clone, PartialEq)]
pub struct VectorPath {
    /// How it joins the subpaths before it, by Photoshop's own number: 0 is
    /// exclude, 1 is add, 2 is subtract and 3 is intersect.
    pub operation: i16,
    /// Each knot as the handle before it, the point and the handle after it.
    /// A point is `(x, y)` as fractions of the document's width and height,
    /// and may lie outside the document.
    pub knots: Vec<[(f64, f64); 3]>,
}

/// One value out of a Photoshop descriptor. A switch is a number, 0 or 1.
#[derive(Debug, Clone, PartialEq)]
enum Note {
    Number(f64),
    Code(String),
}

/// One record of the layer list.
#[derive(Debug, Clone)]
pub struct PsdLayer {
    pub name: String,
    /// 0 to 255.
    pub opacity: u8,
    /// Photoshop's Fill, 0 to 255: an opacity for the layer's own picture
    /// that leaves its layer styles alone. 255 when the file does not say.
    pub fill_opacity: u8,
    pub visible: bool,
    /// Photoshop's four-letter blend key, such as `norm` or `mul `. A group
    /// with no blend of its own says `pass`.
    pub blend: [u8; 4],
    /// Clipped to the layer below it.
    pub clipped: bool,
    pub section: Section,
    rect: Rect,
    channels: Vec<Channel>,
    mask: Option<Mask>,
    vector_mask: bool,
    outline: Option<VectorMask>,
    adjustment: bool,
    /// What the record's fill, layer style and adjustment blocks hold, by
    /// path.
    notes: Vec<(String, Note)>,
}

impl PsdLayer {
    /// Whether there is a picture here to read. An adjustment layer, a
    /// gradient or pattern fill and an empty layer all answer no.
    #[must_use]
    pub fn has_pixels(&self) -> bool {
        self.section == Section::Layer && (!self.rect.is_empty() || self.fill().is_some())
    }

    /// The colour of a solid colour fill layer, as sRGB from 0 to 255.
    ///
    /// `None` for a shape, which is a fill with pixels of its own in the file.
    #[must_use]
    pub fn fill(&self) -> Option<[f64; 3]> {
        if !self.rect.is_empty() || self.vector_mask {
            return None;
        }
        Some([
            self.number("SoCo/Clr /Rd  ")?,
            self.number("SoCo/Clr /Grn ")?,
            self.number("SoCo/Clr /Bl  ")?,
        ])
    }

    /// Whether a layer mask may hide part of the layer.
    #[must_use]
    pub fn has_mask(&self) -> bool {
        self.mask
            .is_some_and(|m| !m.disabled && (m.default != 255 || !m.rect.is_empty()))
    }

    /// The layer's vector mask, when it is switched on and [`read_layer`] has
    /// not already baked it in. Photoshop writes a drawn copy of a vector mask
    /// as the layer mask, and a layer that has one is read with it applied.
    #[must_use]
    pub fn vector_mask(&self) -> Option<&VectorMask> {
        let outline = self.outline.as_ref();
        outline.filter(|_| !self.mask.is_some_and(|m| m.rendered))
    }

    /// A number or a switch from the record's fill or layer styles, by
    /// Photoshop's own keys: `lfx2/DrSh/Opct` is the drop shadow's opacity.
    ///
    /// An adjustment layer's block is kept as its first 16 bit numbers by
    /// place, so `post/0` is Posterize's levels, and the block's own key
    /// answers when the block is there at all. Exposure keeps its stops as
    /// `expA/0`, and Curves count from after their first byte.
    #[must_use]
    pub fn number(&self, path: &str) -> Option<f64> {
        match self.note(path)? {
            Note::Number(n) => Some(*n),
            Note::Code(_) => None,
        }
    }

    /// The same for a choice, which Photoshop stores as a code such as `Mltp`.
    #[must_use]
    pub fn code(&self, path: &str) -> Option<&str> {
        match self.note(path)? {
            Note::Code(code) => Some(code),
            Note::Number(_) => None,
        }
    }

    fn note(&self, path: &str) -> Option<&Note> {
        let (_, note) = self.notes.iter().find(|(at, _)| at == path)?;
        Some(note)
    }
}

/// A document's size and its layer list.
#[derive(Debug, Clone)]
pub struct PsdDocument {
    pub width: u32,
    pub height: u32,
    /// In the file's order, which is bottom to top. A layer's place in this
    /// list is the index [`read_layer`] takes.
    pub layers: Vec<PsdLayer>,
    /// The angle of the light that layer styles share, in degrees, when the
    /// file says what it is.
    pub global_angle: Option<f64>,
    depth: u16,
    grey: bool,
}

impl PsdDocument {
    /// How many bytes one sample takes: one at 8 bit, two at 16.
    fn sample(&self) -> usize {
        if self.depth == 16 {
            2
        } else {
            1
        }
    }
}

fn bad(what: &'static str) -> MediaError {
    MediaError::Psd(what)
}

/// Whether this path names a Photoshop document, by extension.
#[must_use]
pub fn is_psd(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("psd"))
}

/// The document's size and layer list. No pixels are read.
pub fn open(path: &Path) -> Result<PsdDocument, MediaError> {
    let mut r = Reader::new(BufReader::new(std::fs::File::open(path)?))?;
    parse(&mut r, &mut Budget::new(LIMITS))
}

/// One layer's pixels on a transparent frame the size of the document.
///
/// `index` counts through [`PsdDocument::layers`]. A record with no picture
/// reads as an empty frame, and an adjustment layer reads as where it acts.
pub fn read_layer(path: &Path, index: u32) -> Result<DecodedFrame, MediaError> {
    let mut r = Reader::new(BufReader::new(std::fs::File::open(path)?))?;
    read_layer_from(&mut r, index)
}

fn read_layer_from<R: Read + Seek>(
    r: &mut Reader<R>,
    index: u32,
) -> Result<DecodedFrame, MediaError> {
    let mut budget = Budget::new(LIMITS);
    let doc = parse(r, &mut budget)?;
    let index = usize::try_from(index).map_err(|_| bad("no such layer"))?;
    let mut rgba = layer_rgba(&doc, r, &mut budget, index)?;

    // A clipped layer shows only where the layer it is clipped to does.
    // ponytail: baked in, so the clip stays put if the base layer is moved in
    // Lumit. A matte on the Lumit layer is the upgrade.
    let sample = doc.sample();
    if doc.layers.get(index).is_some_and(|l| l.clipped) {
        if let Some(base) = clip_base(&doc, index) {
            let base = layer_rgba(&doc, r, &mut budget, base)?;
            let pairs = rgba
                .chunks_exact_mut(4 * sample)
                .zip(base.chunks_exact(4 * sample));
            for (px, base_px) in pairs {
                fade(&mut px[3 * sample..], &base_px[3 * sample..]);
            }
        }
    }
    let (rgba, format) = if sample == 2 {
        (linear_f32(&rgba, &mut budget)?, PixelFormat::LinearF32)
    } else {
        (rgba, PixelFormat::Srgb8)
    };
    Ok(DecodedFrame {
        width: doc.width,
        height: doc.height,
        rgba,
        format,
    })
}

/// A 16 bit frame as the linear floats the compositor works in. A document's
/// colour is sRGB encoded and its alpha is not.
fn linear_f32(deep: &[u8], budget: &mut Budget) -> Result<Vec<u8>, MediaError> {
    // The decode is a power for each sample, so it is worked out once for
    // each value a sample can take.
    let decode: Vec<f32> = (0..=u16::MAX)
        .map(|v| {
            let encoded = f32::from(v) / 65535.0;
            if encoded <= 0.040_45 {
                encoded / 12.92
            } else {
                ((encoded + 0.055) / 1.055).powf(2.4)
            }
        })
        .collect();
    let mut out = budget.vec_with_capacity::<u8>(deep.len() * 2)?;
    for (i, pair) in deep.chunks_exact(2).enumerate() {
        let v = u16::from_be_bytes([pair[0], pair[1]]);
        let value = match decode.get(usize::from(v)) {
            Some(linear) if i % 4 != 3 => *linear,
            _ => f32::from(v) / 65535.0,
        };
        out.extend_from_slice(&value.to_le_bytes());
    }
    Ok(out)
}

/// Box-average a frame down to `target_width`, keeping its aspect.
///
/// Colour is weighted by alpha, since a layer is mostly empty and an empty
/// pixel's colour would otherwise darken every edge.
#[must_use]
pub fn downsample(frame: DecodedFrame, target_width: Option<u32>) -> DecodedFrame {
    let Some(dst_w) = target_width.filter(|w| *w < frame.width && *w >= 1) else {
        return frame;
    };
    let float = frame.format == PixelFormat::LinearF32;
    let size = frame.format.bytes_per_px();
    // One channel of a pixel as a number, at either width.
    let read = |px: &[u8], c: usize| {
        if float {
            let bytes = px.get(c * 4..c * 4 + 4).and_then(|b| b.try_into().ok());
            bytes.map_or(0.0, |b| f64::from(f32::from_le_bytes(b)))
        } else {
            px.get(c).map_or(0.0, |v| f64::from(*v))
        }
    };
    let (sw, sh) = (frame.width as usize, frame.height as usize);
    let dw = dst_w as usize;
    let dh = ((sh * dw) / sw.max(1)).max(1);
    let mut out = Vec::with_capacity(dw * dh * size);
    for y in 0..dh {
        let (y0, y1) = ((y * sh) / dh, (((y + 1) * sh) / dh).max((y * sh) / dh + 1));
        for x in 0..dw {
            let (x0, x1) = ((x * sw) / dw, (((x + 1) * sw) / dw).max((x * sw) / dw + 1));
            let mut colour = [0f64; 3];
            let (mut alpha, mut count) = (0f64, 0f64);
            for sy in y0..y1.min(sh) {
                for sx in x0..x1.min(sw) {
                    let at = (sy * sw + sx) * size;
                    let Some(px) = frame.rgba.get(at..at + size) else {
                        continue;
                    };
                    let a = read(px, 3);
                    for (c, sum) in colour.iter_mut().enumerate() {
                        *sum += read(px, c) * a;
                    }
                    alpha += a;
                    count += 1.0;
                }
            }
            let [r, g, b] = colour.map(|sum| if alpha > 0.0 { sum / alpha } else { 0.0 });
            for v in [r, g, b, alpha / count.max(1.0)] {
                if float {
                    out.extend_from_slice(&(v as f32).to_le_bytes());
                } else {
                    out.push(v as u8);
                }
            }
        }
    }
    DecodedFrame {
        width: dst_w,
        height: dh as u32,
        rgba: out,
        format: frame.format,
    }
}

fn mul255(a: u8, b: u8) -> u8 {
    ((u16::from(a) * u16::from(b) + 127) / 255) as u8
}

/// Multiply one alpha sample by another of the same width.
fn fade(alpha: &mut [u8], by: &[u8]) {
    match (alpha, by) {
        ([a], [b]) => *a = mul255(*a, *b),
        ([a0, a1], [b0, b1]) => {
            let product = u32::from(u16::from_be_bytes([*a0, *a1]))
                * u32::from(u16::from_be_bytes([*b0, *b1]));
            [*a0, *a1] = (((product + 32767) / 65535) as u16).to_be_bytes();
        }
        _ => {}
    }
}

/// The layer a clipped layer is clipped to: the nearest one below it that is
/// not clipped itself. `None` when a group boundary comes first.
fn clip_base(doc: &PsdDocument, index: usize) -> Option<usize> {
    for i in (0..index).rev() {
        let below = doc.layers.get(i)?;
        if below.section != Section::Layer {
            return None;
        }
        if !below.clipped {
            return Some(i);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// The structure
// ---------------------------------------------------------------------------

/// A file, its length, and big-endian reads that refuse to run off the end.
struct Reader<R> {
    inner: R,
    len: u64,
}

impl<R: Read + Seek> Reader<R> {
    fn new(mut inner: R) -> Result<Self, MediaError> {
        let len = inner.seek(SeekFrom::End(0))?;
        inner.seek(SeekFrom::Start(0))?;
        Ok(Self { inner, len })
    }

    fn pos(&mut self) -> Result<u64, MediaError> {
        Ok(self.inner.stream_position()?)
    }

    fn seek(&mut self, to: u64) -> Result<(), MediaError> {
        if to > self.len {
            return Err(bad("the file ends early"));
        }
        self.inner.seek(SeekFrom::Start(to))?;
        Ok(())
    }

    /// Where a block of `len` bytes starting here ends, refused when the file
    /// is not that long.
    fn block(&mut self, len: u64) -> Result<u64, MediaError> {
        let end = self.pos()?.checked_add(len).filter(|end| *end <= self.len);
        end.ok_or_else(|| bad("the file ends early"))
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], MediaError> {
        let mut bytes = [0u8; N];
        self.inner.read_exact(&mut bytes)?;
        Ok(bytes)
    }

    fn u8(&mut self) -> Result<u8, MediaError> {
        Ok(self.array::<1>()?[0])
    }

    fn u16(&mut self) -> Result<u16, MediaError> {
        Ok(u16::from_be_bytes(self.array()?))
    }

    fn i16(&mut self) -> Result<i16, MediaError> {
        Ok(i16::from_be_bytes(self.array()?))
    }

    fn u32(&mut self) -> Result<u32, MediaError> {
        Ok(u32::from_be_bytes(self.array()?))
    }

    fn i32(&mut self) -> Result<i32, MediaError> {
        Ok(i32::from_be_bytes(self.array()?))
    }

    fn rect(&mut self) -> Result<Rect, MediaError> {
        Ok(Rect {
            top: self.i32()?,
            left: self.i32()?,
            bottom: self.i32()?,
            right: self.i32()?,
        })
    }

    fn skip(&mut self, len: u64) -> Result<(), MediaError> {
        let end = self.block(len)?;
        self.seek(end)
    }

    fn bytes(&mut self, n: usize, budget: &mut Budget) -> Result<Vec<u8>, MediaError> {
        self.block(n as u64)?;
        let mut out = budget.vec_with_capacity::<u8>(n)?;
        out.resize(n, 0);
        self.inner.read_exact(&mut out)?;
        Ok(out)
    }
}

fn parse<R: Read + Seek>(
    r: &mut Reader<R>,
    budget: &mut Budget,
) -> Result<PsdDocument, MediaError> {
    r.seek(0)?;
    if &r.array::<4>()? != b"8BPS" {
        return Err(bad("not a Photoshop document"));
    }
    if r.u16()? != 1 {
        return Err(bad("the large document format (PSB) is not read"));
    }
    r.array::<6>()?;
    let _channels = r.u16()?;
    let height = r.u32()?;
    let width = r.u32()?;
    let depth = r.u16()?;
    let mode = r.u16()?;
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return Err(bad("the document size is out of range"));
    }
    if depth != 8 && depth != 16 {
        return Err(bad("only 8 and 16 bit documents are read"));
    }
    let grey = match mode {
        1 => true,
        3 => false,
        _ => return Err(bad("only RGB and greyscale documents are read")),
    };

    // Colour mode data: nothing in it is needed.
    let len = r.u32()?;
    r.skip(u64::from(len))?;
    // The image resources hold the light that layer styles can share. A list
    // that will not read costs that angle and nothing else.
    let len = r.u32()?;
    let resources_end = r.block(u64::from(len))?;
    let global_angle = global_angle(r, resources_end).ok().flatten();
    r.seek(resources_end)?;

    let mut layers = Vec::new();
    let section_len = r.u32()?;
    let section_end = r.block(u64::from(section_len))?;
    if section_len >= 4 {
        let info_len = r.u32()?;
        let info_end = r.block(u64::from(info_len))?.min(section_end);
        if info_len > 0 {
            layers = layer_info(r, budget, info_end)?;
        }
        r.seek(info_end)?;

        // A 16 bit document leaves the list above empty and keeps its layers
        // in a tagged block after the global mask.
        if layers.is_empty() && r.pos()? + 4 <= section_end {
            let mask_len = r.u32()?;
            let mut at = r.block(u64::from(mask_len))?;
            while at + 12 <= section_end {
                r.seek(at)?;
                let signature = r.array::<4>()?;
                if &signature != b"8BIM" && &signature != b"8B64" {
                    break;
                }
                let key = r.array::<4>()?;
                let len = u64::from(r.u32()?);
                let end = r.block(len)?.min(section_end);
                if &key == b"Lr16" {
                    layers = layer_info(r, budget, end)?;
                    break;
                }
                // These blocks are padded to four bytes.
                at = end + ((4 - len % 4) % 4);
            }
        }
    }

    Ok(PsdDocument {
        width,
        height,
        layers,
        global_angle,
        depth,
        grey,
    })
}

/// Image resource 1037, the global light angle, from a list ending at `end`.
fn global_angle<R: Read + Seek>(r: &mut Reader<R>, end: u64) -> Result<Option<f64>, MediaError> {
    while r.pos()? + 12 <= end {
        r.array::<4>()?;
        let id = r.u16()?;
        // A name, padded with its length byte to an even count.
        let name = r.u8()?;
        r.skip(u64::from(name | 1))?;
        let len = u64::from(r.u32()?);
        if id == 1037 && len >= 4 {
            return Ok(Some(f64::from(r.i32()?)));
        }
        r.skip(len + (len & 1))?;
    }
    Ok(None)
}

/// The layer records, then where each one's channel data sits. `end` is the
/// end of the block the list lives in.
fn layer_info<R: Read + Seek>(
    r: &mut Reader<R>,
    budget: &mut Budget,
    end: u64,
) -> Result<Vec<PsdLayer>, MediaError> {
    // Negative means the merged picture's first alpha is its transparency,
    // which changes nothing about the layers.
    let count = usize::from(r.i16()?.unsigned_abs());
    budget.take_items(count as u64)?;
    let mut layers = Vec::with_capacity(count);
    for _ in 0..count {
        layers.push(layer_record(r, budget)?);
    }

    // The channel data follows the last record, layer by layer, in the order
    // each record listed its channels.
    let mut at = r.pos()?;
    for layer in &mut layers {
        for channel in &mut layer.channels {
            channel.offset = at;
            at = at
                .checked_add(channel.len)
                .filter(|next| *next <= end)
                .ok_or_else(|| bad("a channel runs past the layer data"))?;
        }
    }
    Ok(layers)
}

fn layer_record<R: Read + Seek>(
    r: &mut Reader<R>,
    budget: &mut Budget,
) -> Result<PsdLayer, MediaError> {
    let rect = r.rect()?;
    let channel_count = usize::from(r.u16()?);
    budget.take_items(channel_count as u64)?;
    let mut channels = Vec::with_capacity(channel_count);
    for _ in 0..channel_count {
        channels.push(Channel {
            id: r.i16()?,
            offset: 0,
            len: u64::from(r.u32()?),
        });
    }
    if &r.array::<4>()? != b"8BIM" {
        return Err(bad("a layer record is malformed"));
    }
    let mut blend = r.array::<4>()?;
    let opacity = r.u8()?;
    let clipped = r.u8()? != 0;
    let flags = r.u8()?;
    r.u8()?;
    let extra_len = r.u32()?;
    let extra_end = r.block(u64::from(extra_len))?;

    let mask_len = r.u32()?;
    let mask_end = r.block(u64::from(mask_len))?;
    let mut mask = if mask_len >= 18 {
        let rect = r.rect()?;
        let default = r.u8()?;
        let flags = r.u8()?;
        Some(Mask {
            rect,
            default,
            disabled: flags & 0x02 != 0,
            rendered: flags & 0x08 != 0,
        })
    } else {
        None
    };
    r.seek(mask_end)?;

    let ranges_len = r.u32()?;
    let ranges_end = r.block(u64::from(ranges_len))?;
    r.seek(ranges_end)?;

    // The short name: a length byte and that many bytes, padded to four. It is
    // in the document's own code page, so the Unicode name below replaces it
    // whenever there is one.
    let name_start = r.pos()?;
    let name_len = r.u8()?;
    let short = r.bytes(usize::from(name_len), budget)?;
    let mut name: String = short.iter().map(|b| char::from(*b)).collect();
    let mut at = name_start + ((u64::from(name_len) + 4) & !3);

    let mut section = Section::Layer;
    let mut vector_mask = false;
    let mut outline = None;
    let mut fill_opacity = 255;
    let mut adjustment = false;
    let mut notes = Vec::new();
    while at + 12 <= extra_end {
        r.seek(at)?;
        let signature = r.array::<4>()?;
        if &signature != b"8BIM" && &signature != b"8B64" {
            break;
        }
        let key = r.array::<4>()?;
        let len = u64::from(r.u32()?);
        let end = r.block(len)?;
        if end > extra_end {
            break;
        }
        let from = notes.len();
        match &key {
            b"luni" if len >= 4 => {
                let units = u64::from(r.u32()?).min((len - 4) / 2);
                let raw = r.bytes(checked_usize(units * 2)?, budget)?;
                let utf16: Vec<u16> = raw
                    .chunks_exact(2)
                    .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                    .collect();
                name = String::from_utf16_lossy(&utf16)
                    .trim_end_matches('\0')
                    .to_owned();
            }
            b"lsct" | b"lsdk" if len >= 4 => {
                section = match r.u32()? {
                    1 | 2 => Section::Group,
                    3 => Section::GroupEnd,
                    _ => Section::Layer,
                };
                // A group's own blend key follows, after a signature.
                if len >= 12 {
                    r.array::<4>()?;
                    blend = r.array()?;
                }
            }
            b"iOpa" if len >= 1 => fill_opacity = r.u8()?,
            b"vmsk" | b"vsms" => {
                vector_mask = true;
                // A mask that will not read is left off, never the document.
                if let Ok((shape, disabled)) = vector_outline(r, end) {
                    let knots: usize = shape.paths.iter().map(|path| path.knots.len()).sum();
                    budget.take_bytes((knots * std::mem::size_of::<[(f64, f64); 3]>()) as u64)?;
                    // A drawn copy of a mask that is switched off is off too.
                    if let Some(mask) = mask.as_mut().filter(|m| disabled && m.rendered) {
                        mask.disabled = true;
                    }
                    outline = (!disabled).then_some(shape);
                }
            }
            b"SoCo" | b"lfx2" | b"CgEd" | b"blwh" | b"vibA" => {
                // A block that will not read costs the layer its fill, its
                // styles or its adjustment, never the document.
                adjustment |= !matches!(&key, b"SoCo" | b"lfx2");
                if describe(r, key, end, &mut notes).is_err() {
                    notes.truncate(from);
                }
            }
            b"nvrt" | b"post" | b"thrs" | b"brit" | b"hue2" | b"levl" | b"expA" | b"curv"
            | b"blnc" | b"phfl" => {
                // An adjustment layer's settings. Each kind lays its own out
                // differently, so what is kept is the block's first numbers.
                adjustment = true;
                let name: String = key.iter().map(|b| char::from(*b)).collect();
                if &key == b"expA" {
                    // A version, then the exposure in stops.
                    if len >= 6 {
                        r.u16()?;
                        let stops = f64::from(f32::from_be_bytes(r.array()?));
                        notes.push((format!("{name}/0"), Note::Number(stops)));
                    }
                } else {
                    // Curves open with a byte that is set when they are drawn
                    // tables and not points. Those are not kept.
                    let words = match &key {
                        b"curv" if len == 0 || r.u8()? != 0 => 0,
                        b"curv" => (len - 1) / 2,
                        _ => len / 2,
                    };
                    for i in 0..words.min(WORDS) {
                        let word = f64::from(r.i16()?);
                        notes.push((format!("{name}/{i}"), Note::Number(word)));
                    }
                }
                notes.push((name, Note::Number(1.0)));
            }
            _ => {}
        }
        // Each is charged for the path it is kept under as well, which is
        // most of what a deep one costs.
        let kept: usize = notes
            .get(from..)
            .unwrap_or_default()
            .iter()
            .map(|(path, _)| path.len() + NOTE_BYTES)
            .sum();
        budget.take_bytes(kept as u64)?;
        // Photoshop writes even lengths here. Other writers pad an odd one.
        at = end + (len & 1);
    }
    r.seek(extra_end)?;

    Ok(PsdLayer {
        name,
        opacity,
        fill_opacity,
        visible: flags & 0x02 == 0,
        blend,
        clipped,
        section,
        rect,
        channels,
        mask,
        vector_mask,
        outline,
        adjustment,
        notes,
    })
}

/// How many knots and subpaths one vector mask may hold. A traced drawing
/// runs to a few thousand.
const MAX_KNOTS: u64 = 1 << 16;

/// The outline in a `vmsk` block ending at `end`, and whether the mask is
/// switched off.
///
/// The block is a list of 26 byte records: what each one is, then what it
/// holds. How many there are is how many fit in the block, and a subpath is
/// the knots that follow its own record, however many it says to expect.
fn vector_outline<R: Read + Seek>(
    r: &mut Reader<R>,
    end: u64,
) -> Result<(VectorMask, bool), MediaError> {
    // A version, then the switches.
    r.u32()?;
    let flags = r.u32()?;
    let mut budget = Budget::new(Limits {
        items: MAX_KNOTS,
        ..LIMITS
    });
    let mut paths: Vec<VectorPath> = Vec::new();
    while r.pos()? + 26 <= end {
        let selector = r.u16()?;
        let body = r.array::<24>()?;
        match selector {
            // A subpath starts, closed or open. Photoshop fills an open one
            // as if it were closed.
            0 | 3 => {
                budget.take_items(1)?;
                paths.push(VectorPath {
                    operation: i16::from_be_bytes([body[2], body[3]]),
                    knots: Vec::new(),
                });
            }
            // A knot: three points, each down then across, as 8.24 fixed
            // point fractions of the document.
            1 | 2 | 4 | 5 => {
                let Some(path) = paths.last_mut() else {
                    continue;
                };
                budget.take_items(1)?;
                let mut words = body.chunks_exact(4).map(|word| {
                    let fixed = i32::from_be_bytes([word[0], word[1], word[2], word[3]]);
                    f64::from(fixed) / f64::from(1 << 24)
                });
                let mut point = || {
                    let (y, x) = (words.next().unwrap_or(0.0), words.next().unwrap_or(0.0));
                    (x, y)
                };
                path.knots.push([point(), point(), point()]);
            }
            // The fill rule, the clipboard's place and the first fill.
            _ => {}
        }
    }
    paths.retain(|path| !path.knots.is_empty());
    let mask = VectorMask {
        inverted: flags & 0x01 != 0,
        paths,
    };
    Ok((mask, flags & 0x04 != 0))
}

/// Roughly what one kept descriptor value costs in memory.
const NOTE_BYTES: usize = 128;

/// How many numbers of an adjustment block are kept: room for four curves of
/// nineteen points, which is the longest kind.
const WORDS: u64 = 160;

/// Read the descriptor in a `SoCo` or `lfx2` block ending at `end` into
/// `notes`, each value under its path from the block's key down.
fn describe<R: Read + Seek>(
    r: &mut Reader<R>,
    key: [u8; 4],
    end: u64,
    notes: &mut Vec<(String, Note)>,
) -> Result<(), MediaError> {
    // A version first, and layer styles carry two.
    r.skip(if &key == b"lfx2" { 8 } else { 4 })?;
    // A budget of its own, so a document full of styles cannot use up the
    // records the layer list needs.
    let mut budget = Budget::new(Limits {
        items: 1 << 12,
        depth: 16,
        ..LIMITS
    });
    let path: String = key.iter().map(|b| char::from(*b)).collect();
    descriptor(r, &mut budget, &path, notes)?;
    if r.pos()? > end {
        return Err(bad("a descriptor runs past its block"));
    }
    Ok(())
}

/// A descriptor key or class: a length and that many bytes, or four bytes
/// when the length is zero.
fn ident<R: Read + Seek>(r: &mut Reader<R>, budget: &mut Budget) -> Result<String, MediaError> {
    let len = match r.u32()? {
        0 => 4,
        len @ 1..=64 => len as usize,
        _ => return Err(bad("a descriptor is malformed")),
    };
    Ok(r.bytes(len, budget)?
        .iter()
        .map(|b| char::from(*b))
        .collect())
}

fn descriptor<R: Read + Seek>(
    r: &mut Reader<R>,
    budget: &mut Budget,
    path: &str,
    notes: &mut Vec<(String, Note)>,
) -> Result<(), MediaError> {
    // A name in UTF-16 and a class, neither of which is needed.
    let name = r.u32()?;
    r.skip(u64::from(name) * 2)?;
    ident(r, budget)?;
    for _ in 0..r.u32()? {
        let key = ident(r, budget)?;
        budget.nested(|budget| value(r, budget, &format!("{path}/{key}"), notes))?;
    }
    Ok(())
}

/// One descriptor value. Numbers, switches and codes are kept, objects and
/// lists are walked, text and raw data are stepped over.
fn value<R: Read + Seek>(
    r: &mut Reader<R>,
    budget: &mut Budget,
    path: &str,
    notes: &mut Vec<(String, Note)>,
) -> Result<(), MediaError> {
    let note = match &r.array::<4>()? {
        b"Objc" | b"GlbO" => return descriptor(r, budget, path, notes),
        b"VlLs" => {
            for i in 0..r.u32()? {
                budget.nested(|budget| value(r, budget, &format!("{path}/{i}"), notes))?;
            }
            return Ok(());
        }
        b"doub" => Note::Number(f64::from_be_bytes(r.array()?)),
        b"UntF" => {
            r.array::<4>()?;
            Note::Number(f64::from_be_bytes(r.array()?))
        }
        b"long" => Note::Number(f64::from(r.i32()?)),
        b"bool" => Note::Number(f64::from(r.u8()?)),
        b"enum" => {
            ident(r, budget)?;
            Note::Code(ident(r, budget)?)
        }
        b"TEXT" => {
            let units = r.u32()?;
            return r.skip(u64::from(units) * 2);
        }
        b"tdta" | b"alis" => {
            let len = r.u32()?;
            return r.skip(u64::from(len));
        }
        _ => return Err(bad("a descriptor holds a kind that is not read")),
    };
    notes.push((path.to_owned(), note));
    Ok(())
}

// ---------------------------------------------------------------------------
// The pixels
// ---------------------------------------------------------------------------

/// One layer on a document-sized frame, with its mask applied. A pixel is
/// four samples of the document's own depth, the high byte first.
fn layer_rgba<R: Read + Seek>(
    doc: &PsdDocument,
    r: &mut Reader<R>,
    budget: &mut Budget,
    index: usize,
) -> Result<Vec<u8>, MediaError> {
    let layer = doc.layers.get(index).ok_or_else(|| bad("no such layer"))?;
    let (width, height) = (u64::from(doc.width), u64::from(doc.height));
    let sample = doc.sample();
    let bytes = checked_usize(checked_raster_bytes(width, height, 4, sample as u64)?)?;
    let mut out = budget.vec_with_capacity::<u8>(bytes)?;
    out.resize(bytes, 0);
    // An adjustment layer has no picture, but where it acts is one: white
    // across the document, cut by its mask and by what it is clipped to.
    let flood = match layer.fill() {
        None if layer.adjustment && layer.rect.is_empty() => Some([255.0; 3]),
        fill => fill,
    };
    if !layer.has_pixels() && flood.is_none() {
        return Ok(out);
    }

    if let Some([red, green, blue]) = flood {
        let wide = [red, green, blue, 255.0].map(|c| ((c * 257.0).round() as u16).to_be_bytes());
        for px in out.chunks_exact_mut(4 * sample) {
            for (slot, value) in px.chunks_exact_mut(sample).zip(&wide) {
                slot.copy_from_slice(&value[..sample]);
            }
        }
    } else {
        // A layer with no transparency channel is solid across its rectangle.
        if !layer.channels.iter().any(|c| c.id == -1) {
            blit(&mut out, doc.width, layer.rect, &[3], None, sample);
        }
        for channel in &layer.channels {
            let slots: &[usize] = match (channel.id, doc.grey) {
                (0, true) => &[0, 1, 2],
                (0, false) => &[0],
                (1, false) => &[1],
                (2, false) => &[2],
                (-1, _) => &[3],
                _ => continue,
            };
            let plane = read_plane(r, budget, channel, layer.rect, doc.depth)?;
            blit(&mut out, doc.width, layer.rect, slots, Some(&plane), sample);
        }
    }

    if let Some(mask) = layer.mask.filter(|m| !m.disabled) {
        if let Some(channel) = layer.channels.iter().find(|c| c.id == -2) {
            let plane = if mask.rect.is_empty() {
                Vec::new()
            } else {
                read_plane(r, budget, channel, mask.rect, doc.depth)?
            };
            apply_mask(&mut out, doc.width, &plane, mask, sample);
        }
    }
    Ok(out)
}

/// The part of a rectangle that lands on the canvas.
struct Overlap {
    x0: usize,
    x1: usize,
    y0: usize,
    y1: usize,
    /// How far into the rectangle the part starts.
    skip_x: usize,
    skip_y: usize,
}

fn overlap(rect: Rect, canvas_w: usize, canvas_h: usize) -> Option<Overlap> {
    let clamp = |v: i32, hi: usize| {
        usize::try_from(i64::from(v).clamp(0, i64::try_from(hi).unwrap_or(0))).unwrap_or(0)
    };
    let (x0, x1) = (clamp(rect.left, canvas_w), clamp(rect.right, canvas_w));
    let (y0, y1) = (clamp(rect.top, canvas_h), clamp(rect.bottom, canvas_h));
    if x0 >= x1 || y0 >= y1 {
        return None;
    }
    let into = |edge: i32| usize::try_from(-i64::from(edge)).unwrap_or(0);
    Some(Overlap {
        x0,
        x1,
        y0,
        y1,
        skip_x: into(rect.left),
        skip_y: into(rect.top),
    })
}

/// Write a plane into `slots` of every canvas pixel its rectangle covers.
/// With no plane, write full. A sample is `sample` bytes wide.
fn blit(
    out: &mut [u8],
    width: u32,
    rect: Rect,
    slots: &[usize],
    plane: Option<&[u8]>,
    sample: usize,
) {
    let canvas_w = (width as usize).max(1);
    let Some(o) = overlap(rect, canvas_w, out.len() / (4 * sample) / canvas_w) else {
        return;
    };
    let plane_w = usize::try_from(rect.width()).unwrap_or(0);
    for y in o.y0..o.y1 {
        for x in o.x0..o.x1 {
            let value = match plane {
                Some(plane) => {
                    let at = ((y - o.y0 + o.skip_y) * plane_w + (x - o.x0 + o.skip_x)) * sample;
                    plane.get(at..at + sample).unwrap_or(&[0; 2][..sample])
                }
                None => &[255; 2][..sample],
            };
            let px = (y * canvas_w + x) * 4;
            for slot in slots {
                let at = (px + slot) * sample;
                if let Some(target) = out.get_mut(at..at + sample) {
                    target.copy_from_slice(value);
                }
            }
        }
    }
}

/// Multiply the frame's alpha by a layer mask. Outside its own rectangle a
/// mask is its default colour, which is what hides or shows the rest.
fn apply_mask(out: &mut [u8], width: u32, plane: &[u8], mask: Mask, sample: usize) {
    let canvas_w = (width as usize).max(1);
    let inside = overlap(mask.rect, canvas_w, out.len() / (4 * sample) / canvas_w);
    let plane_w = usize::try_from(mask.rect.width()).unwrap_or(0);
    let default = [mask.default; 2];
    let default = &default[..sample];
    for (i, px) in out.chunks_exact_mut(4 * sample).enumerate() {
        let (x, y) = (i % canvas_w, i / canvas_w);
        let value = match &inside {
            Some(o) if (o.x0..o.x1).contains(&x) && (o.y0..o.y1).contains(&y) => {
                let at = ((y - o.y0 + o.skip_y) * plane_w + (x - o.x0 + o.skip_x)) * sample;
                plane.get(at..at + sample).unwrap_or(default)
            }
            _ => default,
        };
        fade(&mut px[3 * sample..], value);
    }
}

/// One channel of a layer at the document's depth, `rect` across and down.
fn read_plane<R: Read + Seek>(
    r: &mut Reader<R>,
    budget: &mut Budget,
    channel: &Channel,
    rect: Rect,
    depth: u16,
) -> Result<Vec<u8>, MediaError> {
    let (width, height) = (rect.width(), rect.height());
    let sample = u64::from(depth / 8);
    let raw_len = checked_usize(checked_raster_bytes(width, height, 1, sample)?)?;
    let row = checked_usize(width * sample)?.max(1);
    let body = channel
        .len
        .checked_sub(2)
        .ok_or_else(|| bad("a channel is too short"))?;

    r.seek(channel.offset)?;
    Ok(match r.u16()? {
        0 => {
            if body < raw_len as u64 {
                return Err(bad("a channel is too short"));
            }
            r.bytes(raw_len, budget)?
        }
        1 => {
            // A byte count per row, then the rows, each packed on its own.
            let counts = r.bytes(checked_usize(height.saturating_mul(2))?, budget)?;
            let packed_len = body
                .checked_sub(counts.len() as u64)
                .ok_or_else(|| bad("a channel is too short"))?;
            let packed = r.bytes(checked_usize(packed_len)?, budget)?;
            // The plane is as big as the layer says it is, and a layer can
            // say anything. Packed bytes grow at most sixty-four times over,
            // so a plane bigger than that was never in the file, and is not
            // given the room.
            if raw_len as u64 > packed_len.saturating_mul(64) {
                return Err(bad("a channel is too short"));
            }
            let mut out = budget.vec_with_capacity::<u8>(raw_len)?;
            out.resize(raw_len, 0);
            let mut at = 0usize;
            for (count, target) in counts.chunks_exact(2).zip(out.chunks_exact_mut(row)) {
                let count = usize::from(u16::from_be_bytes([count[0], count[1]]));
                budget.take_work(row as u64)?;
                unpack_bits(
                    packed.get(at..at.saturating_add(count)).unwrap_or(&[]),
                    target,
                );
                at = at.saturating_add(count);
            }
            out
        }
        kind @ (2 | 3) => {
            let packed = r.bytes(checked_usize(body)?, budget)?;
            budget.take_bytes(raw_len as u64)?;
            let mut out = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(&packed, raw_len)
                .map_err(|_| bad("a channel would not decompress"))?;
            // Short of the plane the layer claims, it is not that plane. It
            // is not padded out to a size only the layer's word stands for.
            if out.len() != raw_len {
                return Err(bad("a channel would not decompress"));
            }
            if kind == 3 {
                // Each row stores the difference from the sample to its left.
                for line in out.chunks_exact_mut(row) {
                    if sample == 2 {
                        for x in (2..line.len().saturating_sub(1)).step_by(2) {
                            let sum = u16::from_be_bytes([line[x], line[x + 1]])
                                .wrapping_add(u16::from_be_bytes([line[x - 2], line[x - 1]]));
                            line[x..x + 2].copy_from_slice(&sum.to_be_bytes());
                        }
                    } else {
                        for x in 1..line.len() {
                            line[x] = line[x].wrapping_add(line[x - 1]);
                        }
                    }
                }
            }
            out
        }
        _ => return Err(bad("a channel uses a compression that is not read")),
    })
}

/// Unpack one PackBits row. A row that runs short is left at zero, and one
/// that runs long is cut off.
fn unpack_bits(source: &[u8], target: &mut [u8]) {
    let (mut from, mut to) = (0usize, 0usize);
    while to < target.len() {
        let Some(&head) = source.get(from) else {
            return;
        };
        from += 1;
        let room = target.len() - to;
        if head < 128 {
            let run = usize::from(head) + 1;
            let Some(bytes) = source.get(from..from + run) else {
                return;
            };
            let run = run.min(room);
            target[to..to + run].copy_from_slice(&bytes[..run]);
            from += usize::from(head) + 1;
            to += run;
        } else if head != 128 {
            let Some(&byte) = source.get(from) else {
                return;
            };
            from += 1;
            let run = (257 - usize::from(head)).min(room);
            target[to..to + run].fill(byte);
            to += run;
        }
    }
}

// ---------------------------------------------------------------------------
// A writer, for tests
// ---------------------------------------------------------------------------

/// A small Photoshop document written from scratch, so tests here and in the
/// crates above need no file on disk.
#[cfg(any(test, feature = "test-fixtures"))]
pub mod fixture {
    /// How a fixture layer's channels are stored.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Packing {
        Raw,
        Rle,
        Zip,
        ZipPredicted,
    }

    /// One record to write. A group's two ends are records too.
    #[derive(Debug, Clone)]
    pub struct Layer {
        pub name: String,
        /// Top, left, bottom, right.
        pub rect: [i32; 4],
        /// Straight RGBA, `rect` across and down. Empty for a record with no
        /// picture.
        pub rgba: Vec<u8>,
        pub opacity: u8,
        pub visible: bool,
        pub blend: [u8; 4],
        pub clipped: bool,
        /// 1 or 2 for a group header, 3 for the marker under a group.
        pub section: Option<u32>,
        /// A layer mask: its rectangle, its default colour and its samples.
        pub mask: Option<([i32; 4], u8, Vec<u8>)>,
        /// The layer mask's flags. 8 says Photoshop drew it from the layer's
        /// vector mask.
        pub mask_flags: u8,
        pub packing: Packing,
        /// Further tagged blocks, by key, such as the ones [`described`] makes.
        pub blocks: Vec<([u8; 4], Vec<u8>)>,
    }

    /// A value in a descriptor to write.
    #[derive(Debug, Clone)]
    pub enum Value {
        Number(f64),
        Switch(bool),
        Code(&'static str),
        Object(Vec<(&'static str, Value)>),
    }

    fn ident(out: &mut Vec<u8>, id: &str) {
        let len = if id.len() == 4 { 0 } else { id.len() as u32 };
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(id.as_bytes());
    }

    fn object(out: &mut Vec<u8>, items: &[(&'static str, Value)]) {
        // An empty name, a class, then the items.
        out.extend_from_slice(&0u32.to_be_bytes());
        ident(out, "null");
        out.extend_from_slice(&(items.len() as u32).to_be_bytes());
        for (key, value) in items {
            ident(out, key);
            match value {
                Value::Number(n) => {
                    out.extend_from_slice(b"doub");
                    out.extend_from_slice(&n.to_be_bytes());
                }
                Value::Switch(on) => {
                    out.extend_from_slice(b"bool");
                    out.push(u8::from(*on));
                }
                Value::Code(code) => {
                    out.extend_from_slice(b"enum");
                    ident(out, "null");
                    ident(out, code);
                }
                Value::Object(inner) => {
                    out.extend_from_slice(b"Objc");
                    object(out, inner);
                }
            }
        }
    }

    /// A vector mask (`vmsk`) block: one subpath through `corners`, each
    /// `(x, y)` as fractions of the document.
    #[must_use]
    pub fn vector_mask(corners: &[(f64, f64)]) -> ([u8; 4], Vec<u8>) {
        // A version, no switches, then a closed subpath that is added.
        let mut body = [3u32.to_be_bytes(), 0u32.to_be_bytes()].concat();
        body.extend_from_slice(&[0, 0, 0, corners.len() as u8, 0, 1]);
        body.resize(body.len() + 20, 0);
        for (x, y) in corners {
            // A knot with both handles on the point, each down then across.
            body.extend_from_slice(&1u16.to_be_bytes());
            for _ in 0..3 {
                for part in [y, x] {
                    let fixed = (part * f64::from(1 << 24)).round() as i32;
                    body.extend_from_slice(&fixed.to_be_bytes());
                }
            }
        }
        (*b"vmsk", body)
    }

    /// A fill (`SoCo`) or layer styles (`lfx2`) block holding `items`.
    #[must_use]
    pub fn described(key: [u8; 4], items: &[(&'static str, Value)]) -> ([u8; 4], Vec<u8>) {
        // A version first, and layer styles carry two.
        let mut body = vec![0; if &key == b"lfx2" { 8 } else { 4 }];
        object(&mut body, items);
        body.resize(body.len().next_multiple_of(2), 0);
        (key, body)
    }

    impl Layer {
        /// A solid colour fill layer, which has no pixels of its own. `colour`
        /// is sRGB from 0 to 255.
        #[must_use]
        pub fn fill(name: &str, colour: [f64; 3]) -> Self {
            let [red, green, blue] = colour.map(Value::Number);
            let colour = Value::Object(vec![("Rd  ", red), ("Grn ", green), ("Bl  ", blue)]);
            Self {
                blocks: vec![described(*b"SoCo", &[("Clr ", colour)])],
                ..Self::solid(name, [0, 0, 0, 0], [0; 4])
            }
        }

        /// A solid rectangle of one colour.
        #[must_use]
        pub fn solid(name: &str, rect: [i32; 4], colour: [u8; 4]) -> Self {
            let area = ((rect[2] - rect[0]) * (rect[3] - rect[1])).max(0) as usize;
            Self {
                name: name.to_owned(),
                rect,
                rgba: colour.repeat(area),
                opacity: 255,
                visible: true,
                blend: *b"norm",
                clipped: false,
                section: None,
                mask: None,
                mask_flags: 0,
                packing: Packing::Raw,
                blocks: Vec::new(),
            }
        }

        /// A group header (`header` true) or the marker under a group.
        #[must_use]
        pub fn group(name: &str, header: bool) -> Self {
            Self {
                section: Some(if header { 1 } else { 3 }),
                blend: *b"pass",
                ..Self::solid(name, [0, 0, 0, 0], [0; 4])
            }
        }
    }

    fn pack(samples: &[u8], width: usize, depth: u16, packing: Packing) -> Vec<u8> {
        // Widen to the document's depth first.
        let wide: Vec<u8> = if depth == 16 {
            samples.iter().flat_map(|s| [*s, *s]).collect()
        } else {
            samples.to_vec()
        };
        let row = (width * usize::from(depth / 8)).max(1);
        let mut out = Vec::new();
        match packing {
            Packing::Raw => {
                out.extend_from_slice(&0u16.to_be_bytes());
                out.extend_from_slice(&wide);
            }
            Packing::Rle => {
                out.extend_from_slice(&1u16.to_be_bytes());
                // Each row as one literal run and one repeat run of its last
                // byte, so both halves of the unpacker are used.
                let rows: Vec<Vec<u8>> = wide
                    .chunks(row)
                    .map(|line| {
                        let (last, head) = line.split_last().unwrap_or((&0, &[]));
                        let mut packed = Vec::new();
                        for run in head.chunks(128) {
                            packed.push((run.len() - 1) as u8);
                            packed.extend_from_slice(run);
                        }
                        // A repeat of one is a literal, so repeat twice and
                        // let the row's end cut the spare off.
                        packed.extend_from_slice(&[255, *last]);
                        packed
                    })
                    .collect();
                for line in &rows {
                    out.extend_from_slice(&(line.len() as u16).to_be_bytes());
                }
                for line in &rows {
                    out.extend_from_slice(line);
                }
            }
            Packing::Zip | Packing::ZipPredicted => {
                let mut body = wide.clone();
                if packing == Packing::ZipPredicted {
                    for line in body.chunks_mut(row) {
                        if depth == 16 {
                            for x in (2..line.len()).step_by(2).rev() {
                                let delta = u16::from_be_bytes([line[x], line[x + 1]])
                                    .wrapping_sub(u16::from_be_bytes([line[x - 2], line[x - 1]]));
                                line[x..x + 2].copy_from_slice(&delta.to_be_bytes());
                            }
                        } else {
                            for x in (1..line.len()).rev() {
                                line[x] = line[x].wrapping_sub(line[x - 1]);
                            }
                        }
                    }
                }
                let kind: u16 = if packing == Packing::Zip { 2 } else { 3 };
                out.extend_from_slice(&kind.to_be_bytes());
                out.extend_from_slice(&miniz_oxide::deflate::compress_to_vec_zlib(&body, 6));
            }
        }
        out
    }

    fn layer_info(layers: &[Layer], depth: u16) -> Vec<u8> {
        let mut records = Vec::new();
        let mut data = Vec::new();
        for layer in layers {
            let width = (layer.rect[3] - layer.rect[1]).max(0) as usize;
            let mut channels: Vec<(i16, Vec<u8>)> = Vec::new();
            if !layer.rgba.is_empty() {
                for (id, slot) in [(-1i16, 3usize), (0, 0), (1, 1), (2, 2)] {
                    let plane: Vec<u8> = layer.rgba.chunks_exact(4).map(|px| px[slot]).collect();
                    channels.push((id, pack(&plane, width, depth, layer.packing)));
                }
            }
            if let Some((rect, _, samples)) = &layer.mask {
                let width = (rect[3] - rect[1]).max(0) as usize;
                channels.push((-2, pack(samples, width, depth, layer.packing)));
            }

            for edge in layer.rect {
                records.extend_from_slice(&edge.to_be_bytes());
            }
            records.extend_from_slice(&(channels.len() as u16).to_be_bytes());
            for (id, body) in &channels {
                records.extend_from_slice(&id.to_be_bytes());
                records.extend_from_slice(&(body.len() as u32).to_be_bytes());
            }
            records.extend_from_slice(b"8BIM");
            // Photoshop writes a group's real blend key in its section block.
            let grouped = layer.section.is_some();
            records.extend_from_slice(if grouped { b"norm" } else { &layer.blend });
            records.push(layer.opacity);
            records.push(u8::from(layer.clipped));
            records.push(if layer.visible { 0 } else { 0x02 });
            records.push(0);

            let mut extra = Vec::new();
            match &layer.mask {
                Some((rect, default, _)) => {
                    extra.extend_from_slice(&20u32.to_be_bytes());
                    for edge in rect {
                        extra.extend_from_slice(&edge.to_be_bytes());
                    }
                    extra.extend_from_slice(&[*default, layer.mask_flags, 0, 0]);
                }
                None => extra.extend_from_slice(&0u32.to_be_bytes()),
            }
            extra.extend_from_slice(&0u32.to_be_bytes());
            // The short name is deliberately not the real one, so a reader
            // that skips the Unicode name is caught.
            extra.extend_from_slice(&[1, b'?', 0, 0]);
            let units: Vec<u16> = layer.name.encode_utf16().collect();
            let mut luni = (units.len() as u32).to_be_bytes().to_vec();
            luni.extend(units.iter().flat_map(|u| u.to_be_bytes()));
            if !luni.len().is_multiple_of(4) {
                luni.extend_from_slice(&[0, 0]);
            }
            let mut blocks = vec![(*b"luni", luni)];
            if let Some(kind) = layer.section {
                let mut body = kind.to_be_bytes().to_vec();
                if kind != 3 {
                    body.extend_from_slice(b"8BIM");
                    body.extend_from_slice(&layer.blend);
                }
                blocks.push((*b"lsct", body));
            }
            blocks.extend(layer.blocks.iter().cloned());
            for (key, body) in blocks {
                extra.extend_from_slice(b"8BIM");
                extra.extend_from_slice(&key);
                extra.extend_from_slice(&(body.len() as u32).to_be_bytes());
                extra.extend_from_slice(&body);
            }
            records.extend_from_slice(&(extra.len() as u32).to_be_bytes());
            records.extend_from_slice(&extra);

            for (_, body) in channels {
                data.extend_from_slice(&body);
            }
        }
        let mut info = (layers.len() as i16).to_be_bytes().to_vec();
        info.extend_from_slice(&records);
        info.extend_from_slice(&data);
        if info.len() % 2 == 1 {
            info.push(0);
        }
        info
    }

    /// The bytes of a document `width` by `height` holding `layers`, bottom
    /// first. The flattened picture is written as opaque mid grey, which is
    /// all ffmpeg needs to open the file and say how big it is.
    #[must_use]
    pub fn document(width: u32, height: u32, depth: u16, layers: &[Layer]) -> Vec<u8> {
        let mut out = b"8BPS".to_vec();
        out.extend_from_slice(&1u16.to_be_bytes());
        out.extend_from_slice(&[0; 6]);
        out.extend_from_slice(&3u16.to_be_bytes());
        out.extend_from_slice(&height.to_be_bytes());
        out.extend_from_slice(&width.to_be_bytes());
        out.extend_from_slice(&depth.to_be_bytes());
        out.extend_from_slice(&3u16.to_be_bytes());
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(&0u32.to_be_bytes());

        let info = layer_info(layers, depth);
        let mut section = Vec::new();
        if depth == 16 {
            // Sixteen bit: an empty list, an empty global mask, then the
            // layers in a tagged block padded to four bytes.
            section.extend_from_slice(&0u32.to_be_bytes());
            section.extend_from_slice(&0u32.to_be_bytes());
            section.extend_from_slice(b"8BIMLr16");
            section.extend_from_slice(&(info.len() as u32).to_be_bytes());
            section.extend_from_slice(&info);
            section.resize(section.len() + (4 - info.len() % 4) % 4, 0);
        } else {
            section.extend_from_slice(&(info.len() as u32).to_be_bytes());
            section.extend_from_slice(&info);
            section.extend_from_slice(&0u32.to_be_bytes());
        }
        out.extend_from_slice(&(section.len() as u32).to_be_bytes());
        out.extend_from_slice(&section);

        out.extend_from_slice(&0u16.to_be_bytes());
        let plane = width as usize * height as usize * usize::from(depth / 8);
        out.resize(out.len() + plane * 3, 128);
        out
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::io::Cursor;

    use super::fixture::{described, document, vector_mask, Layer, Packing, Value};
    use super::*;

    const RED: [u8; 4] = [255, 0, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];

    fn structure(bytes: &[u8]) -> Result<PsdDocument, MediaError> {
        let mut r = Reader::new(Cursor::new(bytes))?;
        parse(&mut r, &mut Budget::new(LIMITS))
    }

    fn pixels(bytes: &[u8], index: u32) -> DecodedFrame {
        let mut r = Reader::new(Cursor::new(bytes)).unwrap();
        read_layer_from(&mut r, index).unwrap()
    }

    fn px(frame: &DecodedFrame, x: usize, y: usize) -> [u8; 4] {
        let at = (y * frame.width as usize + x) * 4;
        [
            frame.rgba[at],
            frame.rgba[at + 1],
            frame.rgba[at + 2],
            frame.rgba[at + 3],
        ]
    }

    /// A 4 by 3 document: a full background, and a group holding a hidden,
    /// half opacity Multiply layer.
    fn sample() -> Vec<Layer> {
        let mut hat = Layer::solid("Hat \u{00e9}", [1, 1, 3, 3], BLUE);
        hat.opacity = 128;
        hat.visible = false;
        hat.blend = *b"mul ";
        vec![
            Layer::solid("Background", [0, 0, 3, 4], RED),
            Layer::group("</Layer group>", false),
            hat,
            Layer::group("Props", true),
        ]
    }

    #[test]
    fn the_layer_list_carries_names_switches_and_groups() {
        let doc = structure(&document(4, 3, 8, &sample())).unwrap();
        assert_eq!((doc.width, doc.height), (4, 3));
        let names: Vec<&str> = doc.layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(
            names,
            ["Background", "</Layer group>", "Hat \u{00e9}", "Props"]
        );
        let sections: Vec<Section> = doc.layers.iter().map(|l| l.section).collect();
        assert_eq!(
            sections,
            [
                Section::Layer,
                Section::GroupEnd,
                Section::Layer,
                Section::Group
            ]
        );
        let hat = &doc.layers[2];
        assert_eq!(
            (hat.opacity, hat.visible, &hat.blend),
            (128, false, b"mul ")
        );
        assert!(hat.has_pixels());
        assert!(!doc.layers[3].has_pixels(), "a group header has no picture");
        assert_eq!(&doc.layers[3].blend, b"pass", "a group's own blend key");
    }

    #[test]
    fn a_layer_lands_where_it_sat_on_a_document_sized_frame() {
        let frame = pixels(&document(4, 3, 8, &sample()), 2);
        assert_eq!((frame.width, frame.height), (4, 3));
        assert_eq!(px(&frame, 1, 1), BLUE);
        assert_eq!(px(&frame, 2, 2), BLUE);
        assert_eq!(px(&frame, 0, 0), [0; 4], "outside the layer is empty");
        assert_eq!(px(&frame, 3, 1), [0; 4]);
    }

    #[test]
    fn a_layer_hanging_off_the_document_is_cropped_not_wrapped() {
        // Rows -1 to 1 and columns -2 to 1, so only the corner lands.
        let layers = [Layer::solid("Wide", [-1, -2, 2, 2], BLUE)];
        let frame = pixels(&document(4, 3, 8, &layers), 0);
        assert_eq!(px(&frame, 0, 0), BLUE);
        assert_eq!(px(&frame, 1, 1), BLUE);
        assert_eq!(px(&frame, 2, 0), [0; 4]);
        assert_eq!(px(&frame, 0, 2), [0; 4]);
    }

    #[test]
    fn every_packing_reads_the_same_pixels_at_both_depths() {
        // A gradient, so a predictor or a row that is off by one shows.
        let mut layer = Layer::solid("Ramp", [0, 0, 3, 4], RED);
        for (i, px) in layer.rgba.chunks_exact_mut(4).enumerate() {
            px.copy_from_slice(&[i as u8 * 20, 255 - i as u8 * 20, i as u8, 200]);
        }
        let want = layer.rgba.clone();
        for depth in [8, 16] {
            for packing in [
                Packing::Raw,
                Packing::Rle,
                Packing::Zip,
                Packing::ZipPredicted,
            ] {
                layer.packing = packing;
                let frame = pixels(&document(4, 3, depth, std::slice::from_ref(&layer)), 0);
                if depth == 8 {
                    assert_eq!(frame.rgba, want, "{packing:?} at {depth} bit");
                    continue;
                }
                // Sixteen bit keeps its depth: the same colour as linear
                // floats, and the alpha as it was.
                assert_eq!(frame.format, PixelFormat::LinearF32);
                assert_eq!(frame.rgba.len(), want.len() * 4);
                for (i, (got, byte)) in frame.rgba.chunks_exact(4).zip(&want).enumerate() {
                    let got = f32::from_le_bytes(got.try_into().unwrap());
                    let encoded = f32::from(*byte) / 255.0;
                    let linear = if i % 4 == 3 {
                        encoded
                    } else if encoded <= 0.040_45 {
                        encoded / 12.92
                    } else {
                        ((encoded + 0.055) / 1.055).powf(2.4)
                    };
                    assert!((got - linear).abs() < 1e-5, "{packing:?}, sample {i}");
                }
            }
        }
    }

    #[test]
    fn a_layer_mask_is_baked_into_the_alpha() {
        // The mask covers the left column only, half grey, and hides the rest.
        let mut layer = Layer::solid("Masked", [0, 0, 3, 4], RED);
        layer.mask = Some(([0, 0, 3, 1], 0, vec![128; 3]));
        let frame = pixels(&document(4, 3, 8, &[layer.clone()]), 0);
        assert_eq!(px(&frame, 0, 1), [255, 0, 0, 128]);
        assert_eq!(px(&frame, 1, 1)[3], 0, "the default colour hides the rest");

        // The same mask with a white default shows the rest.
        layer.mask = Some(([0, 0, 3, 1], 255, vec![128; 3]));
        let frame = pixels(&document(4, 3, 8, &[layer]), 0);
        assert_eq!(px(&frame, 0, 1)[3], 128);
        assert_eq!(px(&frame, 1, 1)[3], 255);

        // A fill layer has no pixels in the file. It is its colour wherever
        // its mask shows.
        let mut fill = Layer::fill("Fill", [0.0, 128.0, 255.0]);
        let bare = structure(&document(4, 3, 8, &[fill.clone()])).unwrap();
        assert!(!bare.layers[0].has_mask());
        fill.mask = Some(([0, 0, 3, 1], 0, vec![255; 3]));
        let bytes = document(4, 3, 8, &[fill]);
        assert!(structure(&bytes).unwrap().layers[0].has_mask());
        let frame = pixels(&bytes, 0);
        assert_eq!(px(&frame, 0, 1), [0, 128, 255, 255]);
        assert_eq!(px(&frame, 1, 1)[3], 0);

        // An adjustment layer has no picture either. It reads as where it
        // acts, which is white wherever its mask shows.
        let mut invert = Layer::solid("Invert", [0, 0, 0, 0], [0; 4]);
        invert.blocks = vec![(*b"nvrt", Vec::new())];
        invert.mask = Some(([0, 0, 3, 1], 0, vec![255; 3]));
        let bytes = document(4, 3, 8, &[invert]);
        assert!(!structure(&bytes).unwrap().layers[0].has_pixels());
        let frame = pixels(&bytes, 0);
        assert_eq!(px(&frame, 0, 1), [255; 4]);
        assert_eq!(px(&frame, 1, 1)[3], 0);

        // A vector mask is handed over as its outline. One Photoshop has
        // drawn into the layer mask is baked in above with it, and handing it
        // over as well would mask the layer twice.
        let mut shaped = Layer::solid("Shaped", [0, 0, 3, 4], RED);
        shaped.blocks = vec![vector_mask(&[(0.0, 0.0), (0.5, 0.0), (0.5, 1.0)])];
        shaped.mask = Some(([0, 0, 3, 1], 0, vec![255; 3]));
        let outline = |layer: &Layer| {
            let doc = structure(&document(4, 3, 8, std::slice::from_ref(layer))).unwrap();
            doc.layers[0].vector_mask().cloned()
        };
        let knots = outline(&shaped).unwrap().paths[0].knots.clone();
        assert_eq!(knots[1], [(0.5, 0.0); 3]);
        shaped.mask_flags = 0x08;
        assert_eq!(outline(&shaped), None);
    }

    #[test]
    fn a_clipped_layer_shows_only_over_its_base() {
        let mut clipped = Layer::solid("Shade", [0, 0, 3, 4], BLUE);
        clipped.clipped = true;
        let layers = [Layer::solid("Base", [1, 1, 3, 3], RED), clipped];
        let frame = pixels(&document(4, 3, 8, &layers), 1);
        assert_eq!(px(&frame, 1, 1), BLUE);
        assert_eq!(px(&frame, 0, 0)[3], 0, "off the base, the clip hides it");
    }

    #[test]
    fn what_is_not_read_is_refused_by_name() {
        let good = document(4, 3, 8, &sample());
        let refused = |edit: &dyn Fn(&mut Vec<u8>)| {
            let mut bytes = good.clone();
            edit(&mut bytes);
            matches!(structure(&bytes), Err(MediaError::Psd(_)))
        };
        assert!(refused(&|b| b[0] = b'X'), "not a Photoshop document");
        assert!(refused(&|b| b[5] = 2), "PSB");
        assert!(refused(&|b| b[23] = 32), "32 bit");
        assert!(refused(&|b| b[25] = 4), "CMYK");
        assert!(
            refused(&|b| b[14..18].copy_from_slice(&[0xff; 4])),
            "a huge height"
        );
    }

    /// A file is a stranger's bytes: cut short anywhere, or with any one byte
    /// changed, it reads or it errors. It never panics and never hangs.
    #[test]
    fn a_damaged_file_errors_and_never_panics() {
        let mut masked = Layer::solid("Masked", [0, 0, 3, 4], RED);
        masked.mask = Some(([0, 0, 3, 2], 0, vec![128; 6]));
        masked.packing = Packing::Rle;
        let mut zipped = Layer::solid("Zipped", [1, 1, 3, 3], BLUE);
        zipped.packing = Packing::ZipPredicted;
        zipped.clipped = true;
        let shadow = Value::Object(vec![
            ("enab", Value::Switch(true)),
            ("Md  ", Value::Code("Mltp")),
            ("Opct", Value::Number(75.0)),
        ]);
        // Posterize to four levels, as an adjustment layer stores it.
        let posterize = (*b"post", vec![0, 4, 0, 0]);
        // Curves: a byte, a version, which curves are there, then two points
        // on the first. A Vibrance block is a descriptor.
        let curves = (
            *b"curv",
            vec![0, 0, 1, 0, 0, 0, 1, 0, 2, 0, 0, 0, 0, 0, 255, 0, 255, 0],
        );
        zipped.blocks = vec![
            described(*b"lfx2", &[("DrSh", shadow)]),
            posterize,
            curves,
            described(*b"vibA", &[("vibrance", Value::Number(30.0))]),
        ];
        let good = document(4, 3, 8, &[masked, zipped]);
        let styled = &structure(&good).unwrap().layers[1];
        assert_eq!(styled.number("lfx2/DrSh/Opct"), Some(75.0));
        assert_eq!(styled.code("lfx2/DrSh/Md  "), Some("Mltp"));
        assert_eq!(styled.number("post/0"), Some(4.0));
        assert_eq!(styled.number("curv/3"), Some(2.0));
        assert_eq!(styled.number("curv/6"), Some(255.0));
        assert_eq!(styled.number("vibA/vibrance"), Some(30.0));

        let read_all = |bytes: &[u8]| {
            for index in 0..3 {
                if let Ok(mut r) = Reader::new(Cursor::new(bytes)) {
                    let _ = read_layer_from(&mut r, index);
                }
            }
        };
        for cut in 0..good.len() {
            read_all(&good[..cut]);
        }
        // The document's own size is left alone: a bigger one is believed up
        // to Photoshop's ceiling, which only makes this test slow.
        for at in (0..good.len()).filter(|at| !(14..22).contains(at)) {
            for value in [0x00, 0x7f, 0x80, 0xff] {
                let mut bytes = good.clone();
                bytes[at] = value;
                read_all(&bytes);
            }
        }
    }

    #[test]
    fn a_layer_larger_than_the_budget_is_refused_before_it_is_read() {
        // A few zipped bytes whose record claims a rectangle two billion
        // pixels a side.
        let mut layer = Layer::solid("Big", [0, 0, 3, 4], RED);
        layer.packing = Packing::Zip;
        let mut bytes = document(4, 3, 8, &[layer]);
        let rect = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 0, 0, 0, 4];
        let record = bytes.windows(16).position(|w| w == rect).unwrap() + 8;
        bytes[record..record + 4].copy_from_slice(&i32::MAX.to_be_bytes());
        bytes[record + 4..record + 8].copy_from_slice(&i32::MAX.to_be_bytes());
        let mut r = Reader::new(Cursor::new(&bytes)).unwrap();
        assert!(matches!(
            read_layer_from(&mut r, 0),
            Err(MediaError::TooLarge(_))
        ));
    }

    /// A layer item is probed as the file it lives in, so ffmpeg has to open
    /// the document and report the size every layer is read at.
    #[test]
    fn ffmpeg_probes_the_document_as_a_still_of_its_own_size() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("art.psd");
        std::fs::write(&path, document(4, 3, 8, &sample())).unwrap();
        assert!(is_psd(&path));

        let probe = crate::probe::probe(path.as_path()).unwrap();
        let video = probe.video.as_ref().unwrap();
        assert_eq!((video.width, video.height), (4, 3));
        assert!(!probe.runs_as_video());

        let doc = open(&path).unwrap();
        assert_eq!(doc.layers.len(), 4);
        assert_eq!(read_layer(&path, 2).unwrap().rgba.len(), 4 * 3 * 4);
    }

    #[test]
    fn downsampling_weights_colour_by_alpha() {
        // One red pixel beside three empty ones: still red, a quarter as solid.
        let frame = DecodedFrame {
            width: 2,
            height: 2,
            rgba: [RED, [0; 4], [0; 4], [0; 4]].concat(),
            format: PixelFormat::Srgb8,
        };
        let small = downsample(frame, Some(1));
        assert_eq!((small.width, small.height), (1, 1));
        assert_eq!(small.rgba, [255, 0, 0, 63]);

        // The same at 16 bit, where a frame is floats.
        let floats = [1.0f32, 0.0, 0.0, 1.0].into_iter().chain([0.0; 12]);
        let frame = DecodedFrame {
            width: 2,
            height: 2,
            rgba: floats.flat_map(f32::to_le_bytes).collect(),
            format: PixelFormat::LinearF32,
        };
        let small = downsample(frame, Some(1));
        let want = [1.0f32, 0.0, 0.0, 0.25].map(f32::to_le_bytes).concat();
        assert_eq!(small.rgba, want);
    }
}
