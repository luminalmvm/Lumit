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
//! part of what the layer looks like.
//!
//! Read: 8 and 16 bit, RGB and greyscale, every compression Photoshop writes.
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
}

/// One record of the layer list.
#[derive(Debug, Clone)]
pub struct PsdLayer {
    pub name: String,
    /// 0 to 255.
    pub opacity: u8,
    pub visible: bool,
    /// Photoshop's four-letter blend key, such as `norm` or `mul `.
    pub blend: [u8; 4],
    /// Clipped to the layer below it.
    pub clipped: bool,
    pub section: Section,
    rect: Rect,
    channels: Vec<Channel>,
    mask: Option<Mask>,
}

impl PsdLayer {
    /// Whether there is a picture here to read. An adjustment layer, a fill
    /// layer and an empty layer all answer no.
    #[must_use]
    pub fn has_pixels(&self) -> bool {
        self.section == Section::Layer && self.rect.width() > 0 && self.rect.height() > 0
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
    depth: u16,
    grey: bool,
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
/// reads as an empty frame.
// ponytail: 16 bit is read down to 8. A float frame per layer is the upgrade.
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
    if doc.layers.get(index).is_some_and(|l| l.clipped) {
        if let Some(base) = clip_base(&doc, index) {
            let base = layer_rgba(&doc, r, &mut budget, base)?;
            for (px, base_px) in rgba.chunks_exact_mut(4).zip(base.chunks_exact(4)) {
                px[3] = mul255(px[3], base_px[3]);
            }
        }
    }
    Ok(DecodedFrame {
        width: doc.width,
        height: doc.height,
        rgba,
        format: PixelFormat::Srgb8,
    })
}

/// Box-average an 8-bit frame down to `target_width`, keeping its aspect.
///
/// Colour is weighted by alpha, since a layer is mostly empty and an empty
/// pixel's colour would otherwise darken every edge.
#[must_use]
pub fn downsample(frame: DecodedFrame, target_width: Option<u32>) -> DecodedFrame {
    let Some(dst_w) = target_width.filter(|w| *w < frame.width && *w >= 1) else {
        return frame;
    };
    if frame.format != PixelFormat::Srgb8 {
        return frame;
    }
    let (sw, sh) = (frame.width as usize, frame.height as usize);
    let dw = dst_w as usize;
    let dh = ((sh * dw) / sw.max(1)).max(1);
    let mut out = Vec::with_capacity(dw * dh * 4);
    for y in 0..dh {
        let (y0, y1) = ((y * sh) / dh, (((y + 1) * sh) / dh).max((y * sh) / dh + 1));
        for x in 0..dw {
            let (x0, x1) = ((x * sw) / dw, (((x + 1) * sw) / dw).max((x * sw) / dw + 1));
            let mut colour = [0u64; 3];
            let (mut alpha, mut count) = (0u64, 0u64);
            for sy in y0..y1.min(sh) {
                for sx in x0..x1.min(sw) {
                    let Some(px) = frame.rgba.get((sy * sw + sx) * 4..(sy * sw + sx) * 4 + 4)
                    else {
                        continue;
                    };
                    let a = u64::from(px[3]);
                    for (sum, c) in colour.iter_mut().zip(px) {
                        *sum += u64::from(*c) * a;
                    }
                    alpha += a;
                    count += 1;
                }
            }
            for sum in colour {
                out.push(sum.checked_div(alpha).unwrap_or(0) as u8);
            }
            out.push(alpha.checked_div(count).unwrap_or(0) as u8);
        }
    }
    DecodedFrame {
        width: dst_w,
        height: dh as u32,
        rgba: out,
        format: PixelFormat::Srgb8,
    }
}

fn mul255(a: u8, b: u8) -> u8 {
    ((u16::from(a) * u16::from(b) + 127) / 255) as u8
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

    // Colour mode data and image resources: nothing in either is needed.
    for _ in 0..2 {
        let len = r.u32()?;
        let end = r.block(u64::from(len))?;
        r.seek(end)?;
    }

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
        depth,
        grey,
    })
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
    let blend = r.array::<4>()?;
    let opacity = r.u8()?;
    let clipped = r.u8()? != 0;
    let flags = r.u8()?;
    r.u8()?;
    let extra_len = r.u32()?;
    let extra_end = r.block(u64::from(extra_len))?;

    let mask_len = r.u32()?;
    let mask_end = r.block(u64::from(mask_len))?;
    let mask = if mask_len >= 18 {
        let rect = r.rect()?;
        let default = r.u8()?;
        let disabled = r.u8()? & 0x02 != 0;
        Some(Mask {
            rect,
            default,
            disabled,
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
            }
            _ => {}
        }
        // Photoshop writes even lengths here. Other writers pad an odd one.
        at = end + (len & 1);
    }
    r.seek(extra_end)?;

    Ok(PsdLayer {
        name,
        opacity,
        visible: flags & 0x02 == 0,
        blend,
        clipped,
        section,
        rect,
        channels,
        mask,
    })
}

// ---------------------------------------------------------------------------
// The pixels
// ---------------------------------------------------------------------------

/// One layer on a document-sized frame, with its mask applied.
fn layer_rgba<R: Read + Seek>(
    doc: &PsdDocument,
    r: &mut Reader<R>,
    budget: &mut Budget,
    index: usize,
) -> Result<Vec<u8>, MediaError> {
    let layer = doc.layers.get(index).ok_or_else(|| bad("no such layer"))?;
    let (width, height) = (u64::from(doc.width), u64::from(doc.height));
    let bytes = checked_usize(checked_raster_bytes(width, height, 4, 1)?)?;
    let mut out = budget.vec_with_capacity::<u8>(bytes)?;
    out.resize(bytes, 0);
    if !layer.has_pixels() {
        return Ok(out);
    }

    // A layer with no transparency channel is solid across its rectangle.
    if !layer.channels.iter().any(|c| c.id == -1) {
        blit(&mut out, doc.width, layer.rect, &[3], None);
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
        blit(&mut out, doc.width, layer.rect, slots, Some(&plane));
    }

    if let Some(mask) = layer.mask.filter(|m| !m.disabled) {
        if let Some(channel) = layer.channels.iter().find(|c| c.id == -2) {
            let plane = if mask.rect.width() > 0 && mask.rect.height() > 0 {
                read_plane(r, budget, channel, mask.rect, doc.depth)?
            } else {
                Vec::new()
            };
            apply_mask(&mut out, doc.width, &plane, mask);
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
/// With no plane, write 255.
fn blit(out: &mut [u8], width: u32, rect: Rect, slots: &[usize], plane: Option<&[u8]>) {
    let canvas_w = (width as usize).max(1);
    let Some(o) = overlap(rect, canvas_w, out.len() / 4 / canvas_w) else {
        return;
    };
    let plane_w = usize::try_from(rect.width()).unwrap_or(0);
    for y in o.y0..o.y1 {
        for x in o.x0..o.x1 {
            let value = match plane {
                Some(plane) => {
                    let at = (y - o.y0 + o.skip_y) * plane_w + (x - o.x0 + o.skip_x);
                    plane.get(at).copied().unwrap_or(0)
                }
                None => 255,
            };
            let px = (y * canvas_w + x) * 4;
            for slot in slots {
                if let Some(byte) = out.get_mut(px + slot) {
                    *byte = value;
                }
            }
        }
    }
}

/// Multiply the frame's alpha by a layer mask. Outside its own rectangle a
/// mask is its default colour, which is what hides or shows the rest.
fn apply_mask(out: &mut [u8], width: u32, plane: &[u8], mask: Mask) {
    let canvas_w = (width as usize).max(1);
    let inside = overlap(mask.rect, canvas_w, out.len() / 4 / canvas_w);
    let plane_w = usize::try_from(mask.rect.width()).unwrap_or(0);
    for (i, px) in out.chunks_exact_mut(4).enumerate() {
        let (x, y) = (i % canvas_w, i / canvas_w);
        let value = match &inside {
            Some(o) if (o.x0..o.x1).contains(&x) && (o.y0..o.y1).contains(&y) => {
                let at = (y - o.y0 + o.skip_y) * plane_w + (x - o.x0 + o.skip_x);
                plane.get(at).copied().unwrap_or(mask.default)
            }
            _ => mask.default,
        };
        px[3] = mul255(px[3], value);
    }
}

/// One channel of a layer as 8-bit samples, `rect` across and down.
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
    let mut raw = match r.u16()? {
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
            out.resize(raw_len, 0);
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
    };

    // Sixteen bit keeps its high byte.
    if sample == 2 {
        for i in 0..raw.len() / 2 {
            raw[i] = raw[i * 2];
        }
        raw.truncate(raw_len / 2);
    }
    Ok(raw)
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
        pub packing: Packing,
    }

    impl Layer {
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
                packing: Packing::Raw,
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
            records.extend_from_slice(&layer.blend);
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
                    extra.extend_from_slice(&[*default, 0, 0, 0]);
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
            let mut blocks = vec![(b"luni", luni)];
            if let Some(kind) = layer.section {
                blocks.push((b"lsct", kind.to_be_bytes().to_vec()));
            }
            for (key, body) in blocks {
                extra.extend_from_slice(b"8BIM");
                extra.extend_from_slice(key);
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

    use super::fixture::{document, Layer, Packing};
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
                assert_eq!(frame.rgba, want, "{packing:?} at {depth} bit");
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
        let good = document(4, 3, 8, &[masked, zipped]);

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
    }
}
