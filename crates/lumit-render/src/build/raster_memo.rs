//! The last pictures a text or shape layer was drawn as, kept for the next
//! frame that asks for the same one.
//!
//! # In plain terms
//!
//! A text or a shape is drawn by the processor into a picture of its own, and
//! the picture is then placed in the frame by the layer's transform. Moving,
//! turning or fading the layer changes where the picture goes, not the
//! picture, yet every frame drew it again: shaping the words, filling the
//! outlines, at the size the layer is shown at. On a slow processor that is
//! most of a drag tick spent drawing the same pixels. This keeps the pictures
//! by what they were drawn from, so a frame that asks for one already drawn
//! gets it back.
//!
//! The idea is Venturi's (github.com/VenturiVideo/Venturi, GPL-3.0-or-later),
//! whose `ARCHITECTURE.md` keeps transforms out of its cached pictures and
//! applies them on the card. No code is taken from it: Lumit already places a
//! layer's picture with its transform on the card, so only the drawing had to
//! be remembered.
//!
//! **What a picture is known by.** Everything the drawing reads, written out
//! with `Debug` and hashed: the art or the words and their style, the letters'
//! own animator moves, the path a text walks, the clock a trim reads and the
//! size the layer is shown at. Two frames that agree on all of it draw the same
//! bytes, so handing one the other's picture changes nothing on screen. A
//! scale change asks for a new picture, as it must, so a shape scaled up stays
//! sharp.
//!
//! **What it holds.** 64 MiB, least recently used first out: a few dozen
//! layers' pictures at 1080p, enough for a comp being dragged about and too
//! little to matter on any machine. Playback of animated art misses every
//! frame and costs a hash of what is drawn, which is small beside the drawing.

use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

/// A drawn picture: its bytes, its raster size and the layer's natural size.
#[derive(Clone)]
pub(crate) struct Drawn {
    pub rgba: Arc<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    pub natural: (f32, f32),
}

impl lumit_cache::ByteSized for Drawn {
    fn byte_size(&self) -> usize {
        self.rgba.len()
    }
}

const BUDGET: usize = 64 << 20;

fn memo() -> &'static Mutex<lumit_cache::ByteLru<u64, Drawn>> {
    static MEMO: OnceLock<Mutex<lumit_cache::ByteLru<u64, Drawn>>> = OnceLock::new();
    MEMO.get_or_init(|| Mutex::new(lumit_cache::ByteLru::new(BUDGET)))
}

/// Feeds what `Debug` writes straight into a hasher, so a key costs no
/// string.
struct Feed<'a>(&'a mut std::collections::hash_map::DefaultHasher);

impl std::fmt::Write for Feed<'_> {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        s.hash(self.0);
        Ok(())
    }
}

/// The key of a picture drawn from `parts`, each written with `Debug`.
pub(crate) fn key(kind: &str, parts: &[&dyn std::fmt::Debug]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    kind.hash(&mut hasher);
    for part in parts {
        let _ = std::fmt::write(&mut Feed(&mut hasher), format_args!("{part:?}|"));
    }
    hasher.finish()
}

/// The picture known by `key`, drawing it with `draw` if it is not held.
pub(crate) fn drawn(key: u64, draw: impl FnOnce() -> Drawn) -> Drawn {
    if let Some(held) = memo()
        .lock()
        .ok()
        .and_then(|mut memo| memo.get(&key).cloned())
    {
        return held;
    }
    let made = draw();
    if let Ok(mut memo) = memo().lock() {
        memo.insert(key, made.clone());
    }
    made
}

#[cfg(test)]
mod tests {
    use super::{drawn, key, Drawn};
    use std::sync::Arc;

    fn picture(byte: u8) -> Drawn {
        Drawn {
            rgba: Arc::new(vec![byte; 16]),
            width: 2,
            height: 2,
            natural: (2.0, 2.0),
        }
    }

    /// The same inputs are drawn once; anything different is drawn again.
    #[test]
    fn a_picture_is_drawn_once_for_the_same_inputs() {
        let a = key("test-shape", &[&"art", &1.5_f64]);
        let mut draws = 0;
        let first = drawn(a, || {
            draws += 1;
            picture(1)
        });
        let again = drawn(a, || {
            draws += 1;
            picture(2)
        });
        assert_eq!(draws, 1);
        assert!(Arc::ptr_eq(&first.rgba, &again.rgba));
        let b = key("test-shape", &[&"art", &1.6_f64]);
        assert_ne!(a, b, "a different size is a different picture");
        assert_ne!(a, key("test-text", &[&"art", &1.5_f64]));
    }
}
