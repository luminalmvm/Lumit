//! Whisper's word pieces: from ids back to text, and from a short prompt to
//! ids, read from the pack's own `tokenizer.json`.
//!
//! Every piece is a run of bytes written in GPT-2's printable stand-ins for
//! bytes, so turning ids back into text is a table lookup and a UTF-8 read.
//! Turning text into ids is GPT-2's byte-level BPE: split the text the way
//! GPT-2 splits it, then merge neighbouring pieces in the order the merges
//! list ranks them. Only the short fixed prompt ever goes this way.
//!
//! # Thread role and contract
//!
//! Pure: built once from text when a pack opens, then read-only. No IO here;
//! the caller reads the file.

use std::collections::HashMap;

use serde::Deserialize;

use crate::error::MlError;

/// The word pieces of one pack.
#[derive(Debug)]
pub struct Tokens {
    /// Each id's bytes. Specials carry their own text.
    bytes: Vec<Vec<u8>>,
    /// Text piece, in GPT-2's stand-ins, to id.
    ids: HashMap<String, u32>,
    /// A pair of pieces to how early it merges.
    ranks: HashMap<(String, String), usize>,
    /// Special token text to id.
    specials: HashMap<String, u32>,
}

#[derive(Deserialize)]
struct File {
    model: Model,
    #[serde(default)]
    added_tokens: Vec<Added>,
}

#[derive(Deserialize)]
struct Model {
    vocab: HashMap<String, u32>,
    merges: Vec<Merge>,
}

/// One merge, written either as `"a b"` or as `["a", "b"]`.
#[derive(Deserialize)]
#[serde(untagged)]
enum Merge {
    Joined(String),
    Pair(String, String),
}

#[derive(Deserialize)]
struct Added {
    id: u32,
    content: String,
}

impl Tokens {
    /// Read a `tokenizer.json`.
    ///
    /// # Errors
    ///
    /// [`MlError::PackUnreadable`] when the text is not a byte-level BPE
    /// tokenizer.
    pub fn parse(text: &str) -> Result<Self, MlError> {
        let file: File = serde_json::from_str(text).map_err(|_| MlError::PackUnreadable)?;
        let to_byte = unicode_to_byte();
        let size = file
            .model
            .vocab
            .values()
            .chain(file.added_tokens.iter().map(|added| &added.id))
            .max()
            .map_or(0, |top| *top as usize + 1);
        let mut bytes = vec![Vec::new(); size];
        for (piece, id) in &file.model.vocab {
            bytes[*id as usize] = piece
                .chars()
                .map(|c| to_byte.get(&c).copied().unwrap_or(b'?'))
                .collect();
        }
        let mut specials = HashMap::new();
        for added in &file.added_tokens {
            bytes[added.id as usize] = added.content.as_bytes().to_vec();
            specials.insert(added.content.clone(), added.id);
        }
        let ranks = file
            .model
            .merges
            .into_iter()
            .enumerate()
            .filter_map(|(rank, merge)| {
                let pair = match merge {
                    Merge::Joined(text) => {
                        let (a, b) = text.split_once(' ')?;
                        (a.to_owned(), b.to_owned())
                    }
                    Merge::Pair(a, b) => (a, b),
                };
                Some((pair, rank))
            })
            .collect();
        Ok(Tokens {
            bytes,
            ids: file.model.vocab,
            ranks,
            specials,
        })
    }

    /// A special token's id, by its text, such as `<|en|>`.
    #[must_use]
    pub fn special(&self, text: &str) -> Option<u32> {
        self.specials.get(text).copied()
    }

    /// The bytes some ids stand for. Specials are left out.
    #[must_use]
    pub fn bytes(&self, ids: &[u32], first_special: u32) -> Vec<u8> {
        ids.iter()
            .filter(|id| **id < first_special)
            .flat_map(|id| self.bytes.get(*id as usize).into_iter().flatten().copied())
            .collect()
    }

    /// Text to ids, by GPT-2's byte-level BPE. A piece the vocabulary does
    /// not have is dropped, which only a broken tokenizer can cause.
    #[must_use]
    pub fn encode(&self, text: &str) -> Vec<u32> {
        let to_char = byte_to_unicode();
        let mut out = Vec::new();
        for word in pre_split(text) {
            let mut parts: Vec<String> = word
                .bytes()
                .map(|b| to_char[b as usize].to_string())
                .collect();
            loop {
                let best = parts
                    .windows(2)
                    .enumerate()
                    .filter_map(|(at, pair)| {
                        self.ranks
                            .get(&(pair[0].clone(), pair[1].clone()))
                            .map(|rank| (*rank, at))
                    })
                    .min();
                let Some((_, at)) = best else { break };
                let joined = format!("{}{}", parts[at], parts[at + 1]);
                parts.splice(at..at + 2, [joined]);
            }
            out.extend(parts.iter().filter_map(|part| self.ids.get(part).copied()));
        }
        out
    }
}

/// GPT-2's split: contractions, runs of letters, runs of digits and runs of
/// anything else, each taking one leading space, and whitespace on its own.
fn pre_split(text: &str) -> Vec<&str> {
    #[derive(PartialEq, Clone, Copy)]
    enum Class {
        Letter,
        Digit,
        Other,
        Space,
    }
    let class = |c: char| {
        if c.is_alphabetic() {
            Class::Letter
        } else if c.is_numeric() {
            Class::Digit
        } else if c.is_whitespace() {
            Class::Space
        } else {
            Class::Other
        }
    };
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let end_of = |i: usize| chars.get(i).map_or(text.len(), |(at, _)| *at);
    let mut pieces = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let start = i;
        let (at, c) = chars[i];
        if c == '\'' {
            let rest = &text[at..];
            if let Some(len) = ["'s", "'t", "'re", "'ve", "'m", "'ll", "'d"]
                .iter()
                .find(|ending| rest.starts_with(**ending))
                .map(|ending| ending.len())
            {
                pieces.push(&text[at..at + len]);
                i = chars
                    .iter()
                    .position(|(p, _)| *p >= at + len)
                    .unwrap_or(chars.len());
                continue;
            }
        }
        let mut kind = class(c);
        if c == ' ' {
            match chars.get(i + 1).map(|(_, next)| class(*next)) {
                Some(next) if next != Class::Space => {
                    i += 1;
                    kind = next;
                }
                _ => {}
            }
        }
        if kind == Class::Space {
            // A run of whitespace, leaving the last space to the word after
            // it when one follows.
            let mut j = i;
            while j < chars.len() && class(chars[j].1) == Class::Space {
                j += 1;
            }
            if j < chars.len() && j - i > 1 && chars[j - 1].1 == ' ' {
                j -= 1;
            }
            pieces.push(&text[at..end_of(j)]);
            i = j;
            continue;
        }
        let mut j = i + 1;
        while j < chars.len() && class(chars[j].1) == kind {
            j += 1;
        }
        pieces.push(&text[chars[start].0..end_of(j)]);
        i = j;
    }
    pieces
}

/// GPT-2's printable stand-in for every byte.
fn byte_to_unicode() -> Vec<char> {
    let mut out = vec!['\0'; 256];
    let mut extra = 0u32;
    for byte in 0..=255u8 {
        let printable = matches!(byte, b'!'..=b'~' | 0xA1..=0xAC | 0xAE..=0xFF);
        out[byte as usize] = if printable {
            char::from(byte)
        } else {
            extra += 1;
            char::from_u32(255 + extra).unwrap_or('?')
        };
    }
    out
}

/// The inverse of [`byte_to_unicode`].
fn unicode_to_byte() -> HashMap<char, u8> {
    byte_to_unicode()
        .into_iter()
        .enumerate()
        .map(|(byte, c)| (c, byte as u8))
        .collect()
}
