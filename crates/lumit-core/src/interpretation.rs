//! How a footage item's file is read (docs/01-GLOSSARY.md: Interpretation).
//!
//! In plain terms: Lumit never changes a file, so anything the user knows
//! better than the file does is kept here, on the item. What its alpha means,
//! the rate its frames play at, whether it is interlaced, the shape of its
//! pixels, how many times it plays and how its colour is taken.
//!
//! Every field's default is "as the file says", and an item left on the
//! defaults writes nothing to the project file, so a project from before this
//! existed round-trips byte for byte. The engine's media crate can't be seen
//! from here, so these are plain values, and the render crate turns them into
//! what the decoder is told.

use crate::time::FrameRate;
use serde::{Deserialize, Serialize};

/// What a file's alpha channel means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlphaMode {
    /// The colour is whole and the alpha cuts it out.
    #[default]
    Straight,
    /// The file has no transparency, whatever its alpha channel holds.
    Ignore,
    /// The colour was saved already mixed into `matte`, sRGB bytes.
    Premultiplied { matte: [u8; 3] },
}

/// Whether a frame is two fields, and which was shot first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldOrder {
    #[default]
    Off,
    Upper,
    Lower,
}

/// A film cadence to take back out of interlaced video. `phase` is where the
/// file's first frame sits in the cadence, 0 to 4.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pulldown {
    #[default]
    Off,
    /// 3:2 pulldown.
    ThreeTwo { phase: u8 },
    /// 24Pa, the advanced cadence.
    Advanced { phase: u8 },
}

/// Whether a file's numbers are already linear light.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinearLight {
    /// A float file is, anything else carries its colour space's curve.
    #[default]
    Auto,
    On,
    Off,
}

/// The timecode a file's first frame is given, in place of the file's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct StartTimecode {
    /// The first frame's number, counted at the footage's own rate.
    pub frame: u64,
    /// Written the drop-frame way, which skips numbers to keep 29.97 on the
    /// clock.
    #[serde(default, skip_serializing_if = "is_default")]
    pub drop_frame: bool,
}

impl StartTimecode {
    /// The whole frames a second of timecode counts, and how many numbers a
    /// drop-frame count skips each minute: 30 and 2 at 29.97, 60 and 4 at
    /// 59.94.
    ///
    /// Only the rates a thousandth short of a multiple of 30 drop anything.
    /// At any other, a count written with a `;` is still an ordinary count.
    fn base(rate: FrameRate) -> (u64, u64) {
        let (num, den) = (u64::from(rate.num()), u64::from(rate.den().max(1)));
        let per_second = ((num + den / 2) / den).max(1);
        let ntsc = per_second % 30 == 0 && num * 1001 == per_second * 1000 * den;
        (per_second, if ntsc { per_second / 15 } else { 0 })
    }

    /// Whether a count at `rate` can be written the drop-frame way at all.
    #[must_use]
    pub fn drops_at(rate: FrameRate) -> bool {
        Self::base(rate).1 > 0
    }

    /// Written out as `HH:MM:SS:FF`, with a `;` before the frames when it is
    /// drop-frame.
    #[must_use]
    pub fn text(&self, rate: FrameRate) -> String {
        let (fps, drop) = Self::base(rate);
        let mut frame = self.frame;
        let dropping = self.drop_frame && drop > 0;
        if dropping {
            // Ten minutes hold nine minutes that skip and one that doesn't.
            let ten_minutes = fps * 600 - drop * 9;
            let minute = fps * 60 - drop;
            let (tens, rest) = (frame / ten_minutes, frame % ten_minutes);
            frame = frame.saturating_add(drop.saturating_mul(9).saturating_mul(tens));
            if rest > drop {
                frame = frame.saturating_add(drop * ((rest - drop) / minute));
            }
        }
        let seconds = frame / fps;
        format!(
            "{:02}:{:02}:{:02}{}{:02}",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60,
            if dropping { ';' } else { ':' },
            frame % fps
        )
    }

    /// Read `HH:MM:SS:FF` back. A `;` anywhere makes it drop-frame. `None`
    /// for anything that isn't four numbers, or names a frame the rate
    /// doesn't have.
    #[must_use]
    pub fn parse(text: &str, rate: FrameRate) -> Option<Self> {
        let (fps, drop) = Self::base(rate);
        let drop_frame = text.contains(';') && drop > 0;
        let parts: Vec<u64> = text
            .split([':', ';', '.'])
            .map(|part| part.trim().parse().ok())
            .collect::<Option<_>>()?;
        let [h, m, s, f] = parts[..] else {
            return None;
        };
        if m >= 60 || s >= 60 || f >= fps {
            return None;
        }
        let mut frame = fps
            .checked_mul(h.checked_mul(3600)?.checked_add(m * 60 + s)?)?
            .checked_add(f)?;
        if drop_frame {
            let minutes = h * 60 + m;
            frame = frame.checked_sub(drop * (minutes - minutes / 10))?;
        }
        Some(Self { frame, drop_frame })
    }
}

/// How one footage item's file is read.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Interpretation {
    #[serde(skip_serializing_if = "is_default")]
    pub alpha: AlphaMode,
    /// Swap what the alpha calls opaque and clear.
    #[serde(skip_serializing_if = "is_default")]
    pub invert_alpha: bool,
    /// Play a video's frames at this rate instead of the file's own. A run of
    /// stills keeps its rate on [`crate::model::SequenceRef`], since it has no
    /// other.
    #[serde(skip_serializing_if = "is_default")]
    pub frame_rate: Option<FrameRate>,
    #[serde(skip_serializing_if = "is_default")]
    pub start_timecode: Option<StartTimecode>,
    #[serde(skip_serializing_if = "is_default")]
    pub fields: FieldOrder,
    /// Fill a separated field's missing rows along the picture's edges.
    #[serde(skip_serializing_if = "is_default")]
    pub preserve_edges: bool,
    /// Only read while [`Self::fields`] is on.
    #[serde(skip_serializing_if = "is_default")]
    pub pulldown: Pulldown,
    /// How much wider than tall one pixel is, as a fraction. `None` is square.
    #[serde(skip_serializing_if = "is_default")]
    pub pixel_aspect: Option<(u32, u32)>,
    /// How many times the file plays through. Nought and one both mean once.
    #[serde(skip_serializing_if = "is_default")]
    pub loops: u32,
    #[serde(skip_serializing_if = "is_default")]
    pub linear_light: LinearLight,
    /// Take the file's numbers as they are, with no colour conversion. For
    /// pictures that aren't colour, such as a normal map.
    #[serde(skip_serializing_if = "is_default")]
    pub preserve_rgb: bool,
    /// Which render pass of an EXR to read, by name. `None` is Auto, the
    /// beauty pass.
    #[serde(skip_serializing_if = "is_default")]
    pub render_pass: Option<String>,
    /// Unknown fields from newer Lumit versions, preserved on load/save.
    #[serde(flatten, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Interpretation {
    /// Whether this reads the file exactly as it is.
    #[must_use]
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// How many times the file plays, never fewer than once.
    #[must_use]
    pub fn plays(&self) -> u32 {
        self.loops.max(1)
    }
}

fn is_default<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A default interpretation must write nothing, or every project saved
    /// before this existed would come back changed, and a set one must come
    /// back as it went.
    #[test]
    fn the_default_writes_nothing_and_a_set_one_round_trips() {
        assert_eq!(
            serde_json::to_string(&Interpretation::default()).unwrap(),
            "{}"
        );
        let set = Interpretation {
            alpha: AlphaMode::Premultiplied {
                matte: [255, 0, 16],
            },
            invert_alpha: true,
            frame_rate: FrameRate::new(24000, 1001).ok(),
            start_timecode: Some(StartTimecode {
                frame: 108_000,
                drop_frame: true,
            }),
            fields: FieldOrder::Lower,
            preserve_edges: true,
            pulldown: Pulldown::ThreeTwo { phase: 3 },
            pixel_aspect: Some((10, 11)),
            loops: 4,
            linear_light: LinearLight::On,
            preserve_rgb: true,
            render_pass: Some("Depth".into()),
            extra: serde_json::Map::new(),
        };
        let text = serde_json::to_string(&set).unwrap();
        assert_eq!(serde_json::from_str::<Interpretation>(&text).unwrap(), set);
    }

    /// Drop-frame timecode skips numbers, and gets them wrong by a frame if
    /// the sums are off: one hour of 29.97 is 107 892 frames, not 108 000.
    #[test]
    fn timecode_text_reads_back_including_drop_frame() {
        let ntsc = FrameRate::new(30000, 1001).unwrap();
        let hour = StartTimecode {
            frame: 107_892,
            drop_frame: true,
        };
        assert_eq!(hour.text(ntsc), "01:00:00;00");
        assert_eq!(StartTimecode::parse("01:00:00;00", ntsc), Some(hour));
        // The first frame of a minute that skips is numbered 02.
        let minute = StartTimecode {
            frame: 1800,
            drop_frame: true,
        };
        assert_eq!(minute.text(ntsc), "00:01:00;02");
        assert_eq!(StartTimecode::parse("00:01:00;02", ntsc), Some(minute));

        let film = FrameRate::new(24, 1).unwrap();
        let plain = StartTimecode::parse("01:00:00:12", film).unwrap();
        assert_eq!((plain.frame, plain.drop_frame), (86_412, false));
        assert_eq!(plain.text(film), "01:00:00:12");
        assert_eq!(StartTimecode::parse("00:00:00:24", film), None);
        // A `;` at a rate that drops nothing is an ordinary count.
        let pal = FrameRate::new(25, 1).unwrap();
        let hour = StartTimecode::parse("01:00:00;00", pal).unwrap();
        assert_eq!((hour.frame, hour.drop_frame), (90_000, false));
        assert_eq!(StartTimecode::parse("soon", film), None);
    }
}
