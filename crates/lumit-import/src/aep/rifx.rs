//! The RIFX container walk — the bottom of the direct `.aep` route
//! (docs/impl/ae-import.md §7, K-418).
//!
//! In plain terms: an After Effects project file is a box of boxes. Each box
//! starts with a four-letter name, then says in four more bytes how long it is,
//! then holds its contents; a box named `LIST` holds more boxes instead of
//! data, and says what *kind* of list it is in the first four bytes of its
//! contents. Walking the file is just opening boxes in order. The whole job of
//! this module is to do that walk without ever trusting the file: every length
//! is checked against what is actually left, so a corrupt or truncated project
//! produces an error or a skipped box, never a crash and never a huge
//! allocation. Nothing here understands what any box *means* — that is
//! [`super`]'s job.
//!
//! It is RIFF with two changes: the sizes are big-endian (hence RIF**X**), and
//! the form type is `Egg!`. Odd-sized chunks are followed by one pad byte, as
//! in RIFF.
//!
//! Reimplemented in Rust from the public description in
//! `forticheprod/aep_parser` (MIT), read as documentation; no code is vendored.

/// A chunk's four-letter name, kept as bytes so comparison is a plain array
/// compare and a name with non-ASCII bytes still round-trips into an error
/// message.
pub type FourCc = [u8; 4];

/// How deep the walk may nest before it refuses to go further.
///
/// A real project nests perhaps a dozen levels (item ▸ comp ▸ layer ▸ property
/// group ▸ …). Anything far past that is a malformed or hostile file trying to
/// make the walk recurse without bound, so the depth is capped rather than
/// trusted.
pub const MAX_DEPTH: u32 = 64;

/// What can go wrong reading the container itself, as opposed to what any
/// chunk means.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RifxError {
    /// The file does not begin with `RIFX`.
    #[error("not an After Effects project: the file does not start with RIFX")]
    NotRifx,
    /// The form type is not `Egg!` — a RIFX that is not an `.aep`.
    #[error("not an After Effects project: form type is {found}, not Egg!")]
    WrongForm { found: String },
    /// A header or body ran off the end of the bytes it was read from.
    #[error("truncated: {what} wanted {wanted} bytes but only {available} remain")]
    Truncated {
        what: &'static str,
        wanted: u64,
        available: u64,
    },
    /// A chunk declared a size that reaches past its parent's end. This is the
    /// single most important check in the file: `size` is attacker-controlled
    /// and is what every read is bounded by.
    #[error("chunk {id} declares {size} bytes but only {available} remain in its parent")]
    Overrun {
        id: String,
        size: u64,
        available: u64,
    },
    /// The nesting cap was hit.
    #[error("chunks nested deeper than {MAX_DEPTH} levels")]
    TooDeep,
}

/// One chunk: its name, its list type when it is a container, and its body.
///
/// The body is borrowed from the caller's bytes, so walking a project allocates
/// nothing at all — which is also why the parser takes a `&[u8]` and leaves the
/// reading of the file to its caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chunk<'a> {
    /// The four-letter name: `idta`, `cdta`, `ldta`, `LIST`, …
    pub id: FourCc,
    /// For a `LIST` (or the root `RIFX`), the list's own four-letter type:
    /// `Fold`, `Item`, `Layr`, `tdgp`, …
    pub list_type: Option<FourCc>,
    /// The chunk's contents, with the list type already stripped off a `LIST`.
    pub body: &'a [u8],
    /// How deep this chunk sits, so [`Chunk::children`] can enforce the cap.
    depth: u32,
}

impl<'a> Chunk<'a> {
    /// Whether this is a `LIST` of the given type.
    pub fn is_list(&self, list_type: &FourCc) -> bool {
        self.list_type.as_ref() == Some(list_type)
    }

    /// The chunks inside this one. Not meaningful on a leaf chunk — it will
    /// simply produce nonsense or an error, which is why callers ask only of
    /// chunks they have already identified as containers.
    pub fn children(&self) -> Chunks<'a> {
        Chunks::at_depth(self.body, self.depth.saturating_add(1))
    }

    /// The body read as a NUL-terminated UTF-8 string — how AE stores names and
    /// match names. Invalid bytes become replacement characters rather than an
    /// error: a name is never worth failing an import over.
    pub fn text(&self) -> String {
        text_of(self.body)
    }
}

/// A NUL-terminated UTF-8 string out of a fixed-size field.
pub fn text_of(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(bytes.get(..end).unwrap_or_default()).into_owned()
}

/// The chunks of one container, in order.
///
/// Yields `Err` at most once: a malformed header or an overrunning size ends
/// the walk of *that* container, which is what lets a damaged corner of a
/// project cost only the items in it.
#[derive(Debug, Clone)]
pub struct Chunks<'a> {
    rest: &'a [u8],
    depth: u32,
    done: bool,
}

impl<'a> Chunks<'a> {
    /// Walk `body` as a sequence of chunks at the top of the tree.
    pub fn new(body: &'a [u8]) -> Self {
        Self::at_depth(body, 0)
    }

    fn at_depth(body: &'a [u8], depth: u32) -> Self {
        Self {
            rest: body,
            depth,
            done: false,
        }
    }

    /// Every chunk, stopping at the first malformed one. The error is dropped:
    /// callers that want it use the iterator directly.
    pub fn ok(self) -> impl Iterator<Item = Chunk<'a>> {
        self.map_while(Result::ok)
    }
}

impl<'a> Iterator for Chunks<'a> {
    type Item = Result<Chunk<'a>, RifxError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        // Fewer than eight bytes left is the ordinary end of a container: RIFF
        // pads to even, so a trailing byte or two is normal, not damage.
        if self.rest.len() < 8 {
            self.done = true;
            return None;
        }
        if self.depth > MAX_DEPTH {
            self.done = true;
            return Some(Err(RifxError::TooDeep));
        }

        let (header, after_header) = self.rest.split_at(8);
        let mut id: FourCc = [0; 4];
        let mut size_bytes: FourCc = [0; 4];
        id.copy_from_slice(header.get(..4)?);
        size_bytes.copy_from_slice(header.get(4..8)?);
        let size = u32::from_be_bytes(size_bytes) as usize;

        // The one check everything else rests on: a declared size may never
        // reach past the bytes we actually hold. Because the body is a slice of
        // the caller's buffer, this also caps the size against the file length
        // — there is no length to allocate from, only one to bound.
        if size > after_header.len() {
            self.done = true;
            return Some(Err(RifxError::Overrun {
                id: text_of(&id),
                size: size as u64,
                available: after_header.len() as u64,
            }));
        }
        let (body, tail) = after_header.split_at(size);

        // Odd bodies are followed by one pad byte, which belongs to neither
        // chunk. A missing pad byte at the very end of a file is not damage.
        self.rest = if size % 2 == 1 {
            tail.get(1..).unwrap_or_default()
        } else {
            tail
        };

        let (list_type, body) = if id == *b"LIST" || id == *b"RIFX" {
            match body.split_at_checked(4) {
                Some((head, rest)) => {
                    let mut lt: FourCc = [0; 4];
                    lt.copy_from_slice(head);
                    (Some(lt), rest)
                }
                // A LIST too short to hold its own type is damaged; treat it as
                // a leaf rather than refusing the whole container.
                None => (None, body),
            }
        } else {
            (None, body)
        };

        Some(Ok(Chunk {
            id,
            list_type,
            body,
            depth: self.depth,
        }))
    }
}

/// The root of an `.aep`: check the `RIFX` header and the `Egg!` form type, and
/// hand back the chunks inside.
///
/// The declared root size is clamped to what the file actually holds, because
/// After Effects writes an XMP metadata packet *after* the root chunk and a
/// re-saved file can legitimately carry trailing bytes.
pub fn open_egg(bytes: &[u8]) -> Result<Chunks<'_>, RifxError> {
    let Some(header) = bytes.get(..8) else {
        return Err(RifxError::Truncated {
            what: "the RIFX header",
            wanted: 8,
            available: bytes.len() as u64,
        });
    };
    if header.get(..4) != Some(b"RIFX") {
        return Err(RifxError::NotRifx);
    }
    let mut size_bytes: FourCc = [0; 4];
    size_bytes.copy_from_slice(header.get(4..8).unwrap_or(&[0; 4]));
    let declared = u32::from_be_bytes(size_bytes) as usize;

    let after_header = bytes.get(8..).unwrap_or_default();
    let size = declared.min(after_header.len());
    let root = after_header.get(..size).unwrap_or_default();

    let Some((form, inner)) = root.split_at_checked(4) else {
        return Err(RifxError::Truncated {
            what: "the RIFX form type",
            wanted: 4,
            available: root.len() as u64,
        });
    };
    if form != b"Egg!" {
        return Err(RifxError::WrongForm {
            found: text_of(form),
        });
    }
    Ok(Chunks::new(inner))
}

/// Read a big-endian `u8` from a fixed-layout record, or `None` past its end.
pub fn u8_at(body: &[u8], offset: usize) -> Option<u8> {
    body.get(offset).copied()
}

/// Read a big-endian `u16` from a fixed-layout record.
pub fn u16_at(body: &[u8], offset: usize) -> Option<u16> {
    let end = offset.checked_add(2)?;
    let slice = body.get(offset..end)?;
    Some(u16::from_be_bytes([*slice.first()?, *slice.get(1)?]))
}

/// Read a big-endian `u32` from a fixed-layout record.
pub fn u32_at(body: &[u8], offset: usize) -> Option<u32> {
    let end = offset.checked_add(4)?;
    let slice = body.get(offset..end)?;
    let mut raw: FourCc = [0; 4];
    raw.copy_from_slice(slice);
    Some(u32::from_be_bytes(raw))
}

/// Read a big-endian `i32` from a fixed-layout record.
pub fn i32_at(body: &[u8], offset: usize) -> Option<i32> {
    u32_at(body, offset).map(|raw| raw as i32)
}

/// Read a big-endian `f32` from a fixed-layout record.
pub fn f32_at(body: &[u8], offset: usize) -> Option<f32> {
    u32_at(body, offset).map(f32::from_bits)
}

/// One bit of a flag byte, counted from the least significant.
pub fn bit(byte: u8, index: u32) -> bool {
    (byte >> index) & 1 == 1
}

/// An AE rational time: a signed dividend over an unsigned divisor, in seconds.
/// A zero divisor is AE's "no value" and reads as zero rather than dividing.
pub fn rational_at(body: &[u8], dividend: usize, divisor: usize) -> Option<f64> {
    let top = i32_at(body, dividend)?;
    let bottom = u32_at(body, divisor)?;
    if bottom == 0 {
        return Some(0.0);
    }
    Some(f64::from(top) / f64::from(bottom))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A chunk: four-letter name, big-endian size, body, and a pad byte when
    /// the body is odd.
    fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = id.to_vec();
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        out.extend_from_slice(body);
        if body.len() % 2 == 1 {
            out.push(0);
        }
        out
    }

    /// A whole file: `RIFX`, a size, the form type, then the given body.
    fn file(form: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = b"RIFX".to_vec();
        out.extend_from_slice(&((body.len() + 4) as u32).to_be_bytes());
        out.extend_from_slice(form);
        out.extend_from_slice(body);
        out
    }

    /// **A well-formed walk finds every chunk, pad bytes and all.**
    ///
    /// The baseline the damage tests are damage *from*. The odd-sized chunk in
    /// the middle is the point: RIFF pads to an even boundary and the pad byte
    /// belongs to nobody, so a walker that forgets it reads the next chunk's
    /// name one byte late and every chunk after it is nonsense.
    #[test]
    fn an_odd_sized_chunk_is_followed_by_a_pad_byte_that_belongs_to_nobody() {
        let mut body = chunk(b"head", &[1, 2, 3, 4]);
        body.extend(chunk(b"Utf8", b"odd"));
        body.extend(chunk(b"tail", &[9, 9]));
        let bytes = file(b"Egg!", &body);

        let found: Vec<Chunk<'_>> = open_egg(&bytes).unwrap().ok().collect();
        assert_eq!(found.len(), 3);
        assert_eq!(found[0].id, *b"head");
        assert_eq!(found[1].text(), "odd");
        assert_eq!(found[2].body, &[9, 9]);
    }

    /// **A LIST hands back its type and its children.**
    ///
    /// The list type is the first four bytes of the body, not a separate field,
    /// so it has to be stripped before the children are walked — leave it in
    /// and every list's first child starts four bytes early.
    #[test]
    fn a_list_strips_its_type_before_walking_its_children() {
        let inner = chunk(b"idta", &[0, 4]);
        let mut list = b"Item".to_vec();
        list.extend(inner);
        let bytes = file(b"Egg!", &chunk(b"LIST", &list));

        let found: Vec<Chunk<'_>> = open_egg(&bytes).unwrap().ok().collect();
        assert_eq!(found.len(), 1);
        assert!(found[0].is_list(b"Item"));
        let children: Vec<Chunk<'_>> = found[0].children().ok().collect();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].id, *b"idta");
    }

    /// **Something that is not an `.aep` is turned away by name.**
    ///
    /// Three ordinary user mistakes, three plain refusals: a file that is not
    /// RIFX at all, a RIFX of some other kind, and a file cut short before its
    /// header finishes. None of them may reach the item walk, because a walk
    /// over arbitrary bytes finds arbitrary chunks.
    #[test]
    fn a_file_that_is_not_an_aep_is_refused_rather_than_walked() {
        assert_eq!(
            open_egg(b"not a project at all").unwrap_err(),
            RifxError::NotRifx
        );

        let wrong = file(b"WAVE", &chunk(b"fmt ", &[0; 4]));
        assert_eq!(
            open_egg(&wrong).unwrap_err(),
            RifxError::WrongForm {
                found: "WAVE".to_string()
            }
        );

        let short = b"RIFX\x00\x00";
        assert!(matches!(
            open_egg(short).unwrap_err(),
            RifxError::Truncated { .. }
        ));
    }

    /// **A chunk that claims more bytes than exist is an error, not a read.**
    ///
    /// The single most important check in the parser. `size` comes straight out
    /// of an untrusted file, and it is what every subsequent read is bounded
    /// by — so a chunk declaring four gigabytes inside a fifty-byte file must
    /// end the walk of its container with a named error, never allocate, and
    /// never index past the slice. The walk of everything *before* it survives,
    /// which is what makes a damaged corner cost only that corner.
    #[test]
    fn a_chunk_claiming_more_bytes_than_exist_stops_its_container() {
        let mut body = chunk(b"good", &[7, 7]);
        body.extend_from_slice(b"evil");
        body.extend_from_slice(&u32::MAX.to_be_bytes());
        body.extend_from_slice(&[0; 8]);
        let bytes = file(b"Egg!", &body);

        let mut walk = open_egg(&bytes).unwrap();
        assert_eq!(walk.next().unwrap().unwrap().id, *b"good");
        match walk.next().unwrap().unwrap_err() {
            RifxError::Overrun { id, size, .. } => {
                assert_eq!(id, "evil");
                assert_eq!(size, u64::from(u32::MAX));
            }
            other => panic!("expected an overrun, got {other:?}"),
        }
        assert!(walk.next().is_none(), "the container's walk is over");
    }

    /// **A truncated file ends its walk where the bytes end.**
    ///
    /// After Effects writes an XMP packet after the root chunk, so the declared
    /// root size is clamped to what is really there rather than trusted — and a
    /// file cut off mid-chunk must therefore end quietly at the last whole
    /// chunk instead of reading into whatever follows.
    #[test]
    fn a_truncated_file_stops_at_the_last_whole_chunk() {
        let mut body = chunk(b"head", &[1, 2, 3, 4]);
        body.extend(chunk(b"cdta", &[5; 16]));
        let whole = file(b"Egg!", &body);
        let cut = whole.get(..whole.len() - 10).unwrap();

        let found: Vec<Chunk<'_>> = open_egg(cut).unwrap().ok().collect();
        assert_eq!(found.len(), 1, "only the chunk that fits is read");
        assert_eq!(found[0].id, *b"head");
    }

    /// **Nesting stops at the cap rather than recursing without bound.**
    ///
    /// A hostile file can nest lists as deep as it likes, and this parser eats
    /// untrusted files. Past the cap the walk refuses with a named error, so a
    /// deeply-nested project costs an error and not the stack.
    #[test]
    fn nesting_deeper_than_the_cap_is_refused() {
        // Wrap one leaf in enough lists to pass the cap.
        let mut nested = chunk(b"leaf", &[1, 2]);
        for _ in 0..=MAX_DEPTH {
            let mut list = b"deep".to_vec();
            list.extend(nested);
            nested = chunk(b"LIST", &list);
        }
        let bytes = file(b"Egg!", &nested);

        let mut chunk = open_egg(&bytes).unwrap().next().unwrap().unwrap();
        let mut depth = 0_u32;
        loop {
            let mut children = chunk.children();
            match children.next() {
                Some(Ok(child)) => {
                    chunk = child;
                    depth = depth.saturating_add(1);
                }
                Some(Err(RifxError::TooDeep)) => break,
                other => panic!("expected the cap to bite at depth {depth}, got {other:?}"),
            }
        }
        assert!(depth >= MAX_DEPTH, "the cap bit at depth {depth}");
    }

    /// **Fixed-layout reads past the end of a record return nothing.**
    ///
    /// The layer and comp records are read by offset, and a record from an
    /// older After Effects is simply shorter. Every accessor answers "not
    /// there" rather than indexing, which is what lets the structure decode
    /// treat a missing field as absent instead of as a crash.
    #[test]
    fn reading_past_the_end_of_a_record_is_absence_not_a_panic() {
        let short = [0x00, 0x10, 0x00];
        assert_eq!(u8_at(&short, 2), Some(0));
        assert_eq!(u8_at(&short, 9), None);
        assert_eq!(u16_at(&short, 0), Some(0x0010));
        assert_eq!(u16_at(&short, 2), None);
        assert_eq!(u32_at(&short, 0), None);
        assert_eq!(i32_at(&short, 0), None);
        assert_eq!(f32_at(&short, 0), None);
        assert_eq!(rational_at(&short, 0, 4), None);
        // A zero divisor is After Effects' own "no value": zero, not a divide.
        assert_eq!(rational_at(&[0, 0, 0, 5, 0, 0, 0, 0], 0, 4), Some(0.0));
        assert_eq!(u16_at(&short, usize::MAX), None);
    }
}
