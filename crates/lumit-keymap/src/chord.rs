//! Reading a chord from text and writing it back.

use super::{Chord, ChordError, Modifiers};
use std::fmt;
use std::str::FromStr;

/// Normalise a bare key token: single ASCII letters upper-case (so `d` == `D`),
/// everything else (named keys, punctuation) kept verbatim after trimming.
fn normalise_key(raw: &str) -> String {
    let k = raw.trim();
    if k.len() == 1 && k.chars().all(|c| c.is_ascii_alphabetic()) {
        k.to_ascii_uppercase()
    } else {
        k.to_string()
    }
}

impl FromStr for Chord {
    type Err = ChordError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut mods = Modifiers::default();
        // The final token is always the key, even if it spells a modifier
        // word (so `Shift` alone is the Shift *key*). The plus key is written
        // with the sign that joins the others, so `+` and `Shift++` are read
        // for what they are before anything is split on it: otherwise the
        // chord would not read back, and a stored keymap holding one would
        // not load at all.
        let (held, key) = match s.strip_suffix('+') {
            Some(rest) if rest.is_empty() || rest.ends_with('+') => {
                (rest.strip_suffix('+').unwrap_or(""), "+")
            }
            _ => s.rsplit_once('+').unwrap_or(("", s)),
        };
        for tok in held.split('+').filter(|_| !held.is_empty()) {
            match tok.trim().to_ascii_lowercase().as_str() {
                "mod" | "cmd" | "command" | "ctrl" | "control" | "primary" => mods.primary = true,
                "shift" => mods.shift = true,
                "alt" | "option" | "opt" => mods.alt = true,
                other => return Err(ChordError::UnknownModifier(other.to_string())),
            }
        }
        match normalise_key(key) {
            k if !k.is_empty() => Ok(Chord { mods, key: k }),
            _ => Err(ChordError::Empty),
        }
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.mods.primary {
            f.write_str("Mod+")?;
        }
        if self.mods.alt {
            f.write_str("Alt+")?;
        }
        if self.mods.shift {
            f.write_str("Shift+")?;
        }
        f.write_str(&self.key)
    }
}

impl From<Chord> for String {
    fn from(c: Chord) -> Self {
        c.to_string()
    }
}

impl TryFrom<String> for Chord {
    type Error = ChordError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}
