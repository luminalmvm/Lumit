//! The Custom shader's text side: the declaration grammar, the assembler and
//! the uniform layout (docs/impl/custom-shader.md §1.3, §1.4, §2.2, K-650).
//!
//! # In plain terms
//!
//! Every other effect in Lumit is a little program somebody here wrote. The
//! Custom shader is the one a *user* writes: twenty lines of shader code typed
//! into the effect, or loaded from a file somebody sent them.
//!
//! Two things have to happen to that text before a graphics card ever sees it.
//!
//! First it is **read**. The user declares a block of numbers their program
//! wants — a radius, an angle, a colour — with a short note above each one
//! saying what kind of control it should be. This module reads that block and
//! turns it into ordinary parameter rows, the same shape a built-in effect's
//! rows have, so they keyframe and animate and are read by expressions like
//! anything else. The reading is done by a plain line reader rather than by a
//! shader compiler, which is what keeps the controls working on a machine with
//! no graphics card and on a program that does not compile yet.
//!
//! Then it is **wrapped**. The user writes one function; everything around it —
//! the pictures coming in, the picture going out, the clock, the seed, the
//! helpers, and the entry point that checks the answer for nonsense before it
//! stores it — is host text written here, spliced round theirs. The user's own
//! lines are never rewritten, only moved past: exactly one thing is lifted, the
//! block of numbers, so that it can be declared before the helpers that read it.
//!
//! Nothing in here talks to a graphics card. `lumit-gpu` takes the assembled
//! text, validates it and builds the pipeline; this module is the half that
//! works everywhere, and is where the refusals a person can act on are decided.

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use super::params::{ParamId, Params, Unit};
use super::schema::{ParamKind, ParamSchema};

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;

/// The prologue, with `//__LUMIT_PARAMS__` marking where the lifted `Params`
/// struct goes (§1.3).
const PROLOGUE: &str = include_str!("prologue.wgsl");

/// Where the generated struct is spliced into [`PROLOGUE`].
const PARAMS_MARKER: &str = "//__LUMIT_PARAMS__";

/// The epilogue: the entry point, the sanitise and the Mix (§1.3, §2.3).
const EPILOGUE: &str = include_str!("epilogue.wgsl");

/// The one function the contract asks the user for (§1.3).
pub const ENTRY_FN: &str = "shade";

/// The generated module's compute entry point, which the pipeline names.
pub const ENTRY_POINT: &str = "lumit_shade";

/// The host's own header, `LumitHeader` in [`PROLOGUE`], byte for byte.
///
/// `roi_offset` and `roi_size` are the gpu-foundation common header's; the four
/// that follow are what the note calls "handed in" (§2.3) — the *entire* source
/// of variation a shader has, because WGSL itself has no clock and no random
/// number generator.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShaderHeader {
    pub roi_offset: [u32; 2],
    pub roi_size: [u32; 2],
    /// Raster pixels per px@comp: 1.0 at full, 0.5 at Half.
    pub comp_scale: f32,
    /// Layer time in seconds.
    pub time: f32,
    /// This instance's seed — constant for the life of the instance, and
    /// deliberately not a frame counter (§2.3).
    pub seed: u32,
    pub mix_amt: f32,
    pub matte_on: f32,
    pub input2_on: f32,
    pub _pad0: u32,
    pub _pad1: u32,
}

/// The size of [`ShaderHeader`] in bytes — 48, WGSL's uniform rounding of 40 up
/// to the struct alignment.
pub const HEADER_SIZE: usize = std::mem::size_of::<ShaderHeader>();

/// The names the host owns at module scope. A user declaration of any of them
/// is refused before assembly (§2.2): shadowing `p` would silently override the
/// parameters with nothing, and naga would not complain.
pub const RESERVED: &[&str] = &["src", "orig", "dst", "matte", "input2", "lumit", "p"];

/// The kinds of field the grammar accepts, and what each occupies in a uniform
/// block (§1.4, §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WgslTy {
    F32,
    I32,
    U32,
    /// `vec2<f32>` — a **point**, which derives two rows (`_x` and `_y`).
    Vec2,
    /// `vec4<f32>` — a colour.
    Vec4,
}

impl WgslTy {
    /// The WGSL spelling.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            WgslTy::F32 => "f32",
            WgslTy::I32 => "i32",
            WgslTy::U32 => "u32",
            WgslTy::Vec2 => "vec2<f32>",
            WgslTy::Vec4 => "vec4<f32>",
        }
    }

    /// Alignment in a uniform block, in bytes.
    #[must_use]
    pub const fn align(self) -> u32 {
        match self {
            WgslTy::F32 | WgslTy::I32 | WgslTy::U32 => 4,
            WgslTy::Vec2 => 8,
            WgslTy::Vec4 => 16,
        }
    }

    /// Size in a uniform block, in bytes.
    #[must_use]
    pub const fn size(self) -> u32 {
        match self {
            WgslTy::F32 | WgslTy::I32 | WgslTy::U32 => 4,
            WgslTy::Vec2 => 8,
            WgslTy::Vec4 => 16,
        }
    }

    fn parse(text: &str) -> Option<WgslTy> {
        let squashed: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        match squashed.as_str() {
            "f32" => Some(WgslTy::F32),
            "i32" => Some(WgslTy::I32),
            "u32" => Some(WgslTy::U32),
            "vec2<f32>" => Some(WgslTy::Vec2),
            "vec4<f32>" => Some(WgslTy::Vec4),
            _ => None,
        }
    }
}

/// One field of the user's `Params` struct: where it sits in the uniform buffer,
/// and the parameter row (or two, for a point) it derives.
#[derive(Debug, Clone, Copy)]
pub struct ShaderField {
    /// The WGSL field name, which is also the parameter id for every type but
    /// [`WgslTy::Vec2`] (whose halves are `<name>_x` and `<name>_y`).
    pub name: &'static str,
    pub ty: WgslTy,
    /// Byte offset in the `Params` buffer.
    pub offset: u32,
    /// The rows this field derives: one, two for a point, **none** for a field
    /// whose annotation would not parse (§2.2 — a typo in a doc comment costs
    /// that row and not the other eight). The bytes are still uploaded; a field
    /// with no rows uploads its declared default.
    pub params: &'static [ParamSchema],
    /// The default the bytes fall back to, per component.
    pub default: [f64; 4],
}

/// One user shader, read and wrapped: everything a render needs of it that is
/// not a graphics card.
///
/// Built once per distinct source and kept for the session
/// ([`program_for`]), so the render path costs a hash and a map lookup rather
/// than a parse.
#[derive(Debug)]
pub struct ShaderProgram {
    /// The 64-bit hash of the user's text **as the document holds it** — the
    /// pipeline cache's key, and the term the frame key folds (§2.4, §3.1).
    pub source_hash: u64,
    /// The whole module: prologue, generated struct, the user's text verbatim,
    /// epilogue.
    pub assembled: String,
    /// Every derived row, in declaration order (§1.5).
    pub params: &'static [ParamSchema],
    /// The fields behind them, with their uniform offsets.
    pub fields: &'static [ShaderField],
    /// The `Params` buffer's size in bytes, rounded up as a uniform block is.
    pub params_size: u32,
    /// How many lines of host text precede the user's, so a compiler's line
    /// numbers can be remapped to the text the user is looking at (§2.1).
    pub prologue_lines: u32,
    /// Annotations that would not parse, one calm sentence each (§2.2).
    pub notes: Vec<String>,
}

/// Why a shader source was refused **at the edit**, before anything was
/// assembled (§2.2).
///
/// These are the failures that are the user's own edit rather than a state some
/// other entity produced, so they refuse rather than degrade. A parse or
/// validation error is not among them: that is the state a person is in for most
/// of the time they are typing, and it degrades to a badge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShaderRefusal {
    /// The text declares its own `@group` or `@binding`.
    OwnBinding,
    /// The text declares a module-scope name the host owns.
    ReservedName(String),
    /// No `fn shade(uv: vec2<f32>) -> vec4<f32>`.
    NoShadeFunction,
    /// A `vec3<f32>` parameter — sixteen bytes wearing twelve bytes' name.
    Vec3Field(String),
    /// A parameter field of a type the grammar does not carry.
    UnknownType { field: String, ty: String },
    /// Two parameters would answer to one id.
    DuplicateId(String),
}

impl std::fmt::Display for ShaderRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShaderRefusal::OwnBinding => write!(
                f,
                "the host declares the bindings; a shader may not declare @group or @binding"
            ),
            ShaderRefusal::ReservedName(n) => write!(
                f,
                "`{n}` is one of the host's own names ({}, and anything starting lumit_)",
                RESERVED.join(", ")
            ),
            ShaderRefusal::NoShadeFunction => write!(
                f,
                "a shader needs `fn {ENTRY_FN}(uv: vec2<f32>) -> vec4<f32>`"
            ),
            ShaderRefusal::Vec3Field(n) => write!(
                f,
                "`{n}` is a vec3<f32>, which occupies sixteen bytes in a uniform block and \
                 would read back wrong; declare it vec4<f32>"
            ),
            ShaderRefusal::UnknownType { field, ty } => write!(
                f,
                "`{field}` is a {ty}; a parameter may be f32, i32, u32, vec2<f32> or vec4<f32>"
            ),
            ShaderRefusal::DuplicateId(id) => {
                write!(f, "two parameters would both answer to `{id}`")
            }
        }
    }
}

impl ShaderProgram {
    /// A compiler's message about the assembled module, rewritten to name the
    /// lines of **the user's own text** (§2.1).
    ///
    /// naga's `emit_to_string` writes `wgsl:LINE:COLUMN`, counted from the top of
    /// the module it was given — which begins with forty-odd lines of the host's
    /// prologue. Subtracting them is what turns "an error on line 47" into "an
    /// error on line 3", which is the only line number the person typing has.
    ///
    /// An error that lands inside the prologue or the epilogue is **not**
    /// renumbered into nonsense: it is labelled as what it is, a bug in Lumit's
    /// own wrapper, and reads like one.
    #[must_use]
    pub fn remap_error(&self, message: &str) -> String {
        let user_lines = self
            .assembled
            .lines()
            .count()
            .saturating_sub(self.prologue_lines as usize)
            .saturating_sub(EPILOGUE.lines().count());
        let mut out = String::with_capacity(message.len());
        let mut host = false;
        for chunk in message.split_inclusive('\n') {
            match chunk.find("wgsl:") {
                None => out.push_str(chunk),
                Some(at) => {
                    let rest = chunk.get(at + 5..).unwrap_or("");
                    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
                    match digits.parse::<usize>() {
                        Ok(line) if line > self.prologue_lines as usize => {
                            let moved = line - self.prologue_lines as usize;
                            if moved > user_lines {
                                host = true;
                                out.push_str(chunk);
                            } else {
                                out.push_str(chunk.get(..at + 5).unwrap_or(""));
                                out.push_str(&moved.to_string());
                                out.push_str(rest.get(digits.len()..).unwrap_or(""));
                            }
                        }
                        _ => {
                            host = true;
                            out.push_str(chunk);
                        }
                    }
                }
            }
        }
        if host {
            format!("in the host's own wrapper — please report this\n{out}")
        } else {
            out
        }
    }
}

/// FNV-1a 64 over bytes — the same hash [`ParamId`] is, used here for the source
/// (§3.1) rather than for an id.
#[must_use]
pub fn hash64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// The read and wrapped form of `source`, built once per distinct text and kept
/// for the session.
///
/// Memoised because the render path asks for it once per op per frame — the
/// derived rows are part of resolving the stack — and a parse per frame per
/// effect would be a parse nobody needs. The entries are `&'static`, which for
/// a text somebody typed means leaked: the honest spelling of "lives as long as
/// the session", and the same one a plugin's schema uses.
///
/// # Errors
/// The §2.2 refusals — a shader that binds its own group, shadows a host name,
/// declares no `shade`, or declares a parameter the grammar cannot carry.
///
/// ponytail: never evicts, so a session that types N distinct shaders holds N
/// small records. Bound it (and stop leaking) if that ever shows up in a heap
/// profile; the fix is an owned mirror of `ParamSchema`, which is why it was not
/// done first.
pub fn program_for(source: &str) -> Result<&'static ShaderProgram, ShaderRefusal> {
    static CACHE: OnceLock<RwLock<HashMap<u64, Result<&'static ShaderProgram, ShaderRefusal>>>> =
        OnceLock::new();
    let cache = CACHE.get_or_init(|| RwLock::new(HashMap::new()));
    let key = hash64(source.as_bytes());
    if let Ok(map) = cache.read() {
        if let Some(hit) = map.get(&key) {
            return hit.clone();
        }
    }
    let built = build(source).map(|p| &*Box::leak(Box::new(p)));
    if let Ok(mut map) = cache.write() {
        map.insert(key, built.clone());
    }
    built
}

/// Read and wrap one source, without the cache. `program_for` is what callers
/// want; this is the working half, and what the tests drive directly.
///
/// # Errors
/// As [`program_for`].
pub fn build(source: &str) -> Result<ShaderProgram, ShaderRefusal> {
    // Comments blanked to spaces rather than removed, so every byte offset in
    // `code` is the same byte offset in `source` and the struct can be cut out
    // of the user's own text with its doc comments intact.
    let code = blank_comments(source);
    if code.contains("@group") || code.contains("@binding") {
        return Err(ShaderRefusal::OwnBinding);
    }
    let decls = module_scope_decls(&code);
    for d in &decls {
        if d.name.starts_with("lumit_") || RESERVED.contains(&d.name.as_str()) {
            return Err(ShaderRefusal::ReservedName(d.name.clone()));
        }
        if d.name == "Params" && d.kind != "struct" {
            return Err(ShaderRefusal::ReservedName(d.name.clone()));
        }
    }
    if !decls.iter().any(|d| d.kind == "fn" && d.name == ENTRY_FN) {
        return Err(ShaderRefusal::NoShadeFunction);
    }

    let block = decls
        .iter()
        .find(|d| d.kind == "struct" && d.name == "Params");
    let (fields, notes) = match block {
        Some(d) => read_fields(source.get(d.body.clone()).unwrap_or(""))?,
        None => (Vec::new(), Vec::new()),
    };

    // Declaration order is never changed (§7): the panel, the bridge and the
    // key all rely on schema order, and a sort here would silently reorder the
    // interface as well as the bytes.
    let (laid, params_size) = lay_out(&fields);
    let struct_text = struct_wgsl(&laid);

    let mut rows: Vec<ParamSchema> = Vec::new();
    for f in &laid {
        for row in f.params {
            if rows.iter().any(|r| r.id == row.id) || DECLARED_IDS.contains(&row.id) {
                return Err(ShaderRefusal::DuplicateId(row.id.to_owned()));
            }
            rows.push(*row);
        }
    }

    // The user's text with exactly one thing taken out of it: the `Params`
    // struct, which the prologue re-declares above the bindings that read it.
    let user = match block {
        Some(d) => {
            let mut s = String::with_capacity(source.len());
            s.push_str(source.get(..d.start).unwrap_or(""));
            s.push_str(source.get(d.end..).unwrap_or(""));
            s
        }
        None => source.to_owned(),
    };

    // The user's first line is the line after the prologue's last, exactly —
    // `prologue_lines` is what a compiler's line numbers are shifted by, so an
    // off-by-one here is an off-by-one in every message a person reads.
    let mut assembled = PROLOGUE.replace(PARAMS_MARKER, &struct_text);
    if !assembled.ends_with('\n') {
        assembled.push('\n');
    }
    let prologue_lines = assembled.lines().count() as u32;
    assembled.push_str(&user);
    if !assembled.ends_with('\n') {
        assembled.push('\n');
    }
    assembled.push_str(EPILOGUE);

    Ok(ShaderProgram {
        source_hash: hash64(source.as_bytes()),
        assembled,
        params: Box::leak(rows.into_boxed_slice()),
        fields: Box::leak(laid.into_boxed_slice()),
        params_size,
        prologue_lines,
        notes,
    })
}

/// The ids the Custom shader's own declaration already uses — a derived row may
/// not collide with one (§1.4).
const DECLARED_IDS: &[&str] = &[
    "input2",
    "edit",
    "load_from_file",
    "mix",
    "blend",
    "matte",
    "matte_invert",
    "matte_channel",
];

impl ShaderProgram {
    /// The bytes of the `Params` uniform buffer for one resolved bag.
    ///
    /// A field the bag has nothing for uploads its declared default, which is
    /// what makes "a missing parameter is a default, not a fault" true here as
    /// well: an instance that has not adopted the derived rows yet still renders.
    #[must_use]
    pub fn pack(&self, p: Params<'_>) -> Vec<u8> {
        let mut out = vec![0u8; self.params_size as usize];
        for f in self.fields {
            let at = f.offset as usize;
            let mut put = |i: usize, bytes: [u8; 4]| {
                if let Some(slot) = out.get_mut(at + i * 4..at + i * 4 + 4) {
                    slot.copy_from_slice(&bytes);
                }
            };
            let id = |n: usize| ParamId::new(self.field_id(f, n));
            match f.ty {
                WgslTy::F32 => {
                    put(0, p.float(id(0), f.default[0] as f32).to_le_bytes());
                }
                WgslTy::I32 => {
                    put(0, p.int(id(0), f.default[0] as i32).to_le_bytes());
                }
                WgslTy::U32 => {
                    // A toggle, a choice or a seed — all three ride the bag as
                    // an unsigned code, and a bare u32 as a whole number.
                    let v = match f.params.first().map(|r| r.kind) {
                        Some(ParamKind::Bool { .. }) => {
                            u32::from(p.bool(id(0), f.default[0] != 0.0))
                        }
                        Some(ParamKind::Choice { .. }) => p.choice(id(0), f.default[0] as u32),
                        Some(ParamKind::Seed) => p.int(id(0), 0) as u32,
                        _ => p.int(id(0), f.default[0] as i32).max(0) as u32,
                    };
                    put(0, v.to_le_bytes());
                }
                WgslTy::Vec2 => {
                    put(0, p.float(id(0), f.default[0] as f32).to_le_bytes());
                    put(1, p.float(id(1), f.default[1] as f32).to_le_bytes());
                }
                WgslTy::Vec4 => {
                    let d = [
                        f.default[0] as f32,
                        f.default[1] as f32,
                        f.default[2] as f32,
                        f.default[3] as f32,
                    ];
                    for (i, c) in p.colour(id(0), d).into_iter().enumerate() {
                        put(i, c.to_le_bytes());
                    }
                }
            }
        }
        out
    }

    /// The id of a field's n-th row, falling back to the field's own name for a
    /// field whose annotation was skipped.
    fn field_id(&self, f: &ShaderField, n: usize) -> &str {
        f.params.get(n).map_or(f.name, |r| r.id)
    }
}

// ---------------------------------------------------------------- the reader

/// One module-scope declaration: what kind it is, what it is called, and — for a
/// struct — where its braces are.
struct Decl {
    kind: String,
    name: String,
    /// Byte range of the whole declaration in the source.
    start: usize,
    end: usize,
    /// Byte range of a struct's body, between its braces.
    body: std::ops::Range<usize>,
}

/// Comments replaced by spaces, newlines kept — so scanning the result gives
/// byte offsets into the original.
fn blank_comments(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = String::with_capacity(source.len());
    let mut i = 0usize;
    while i < bytes.len() {
        let two = bytes.get(i..i + 2);
        if two == Some(b"//") {
            while i < bytes.len() && bytes.get(i) != Some(&b'\n') {
                out.push(' ');
                i += 1;
            }
        } else if two == Some(b"/*") {
            let mut depth = 1usize;
            out.push_str("  ");
            i += 2;
            while i < bytes.len() && depth > 0 {
                match bytes.get(i..i + 2) {
                    Some(b"/*") => {
                        depth += 1;
                        out.push_str("  ");
                        i += 2;
                    }
                    Some(b"*/") => {
                        depth -= 1;
                        out.push_str("  ");
                        i += 2;
                    }
                    _ => {
                        let c = bytes.get(i).copied().unwrap_or(b' ');
                        out.push(if c == b'\n' { '\n' } else { ' ' });
                        i += 1;
                    }
                }
            }
        } else {
            // Multi-byte characters are copied whole; a non-ASCII byte inside an
            // identifier is naga's business, not the reader's.
            let ch = source
                .get(i..)
                .and_then(|s| s.chars().next())
                .unwrap_or(' ');
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Every declaration at module scope — brace depth nought — in `code`.
///
/// A hundred lines of nothing clever, and it must never panic: a malformed
/// source is a source to refuse with a sentence, not a fault (docs/14 §4).
fn module_scope_decls(code: &str) -> Vec<Decl> {
    const KEYWORDS: &[&str] = &["fn", "var", "let", "const", "struct", "alias", "override"];
    let chars: Vec<(usize, char)> = code.char_indices().collect();
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut i = 0usize;
    while i < chars.len() {
        let (at, c) = chars[i];
        match c {
            '{' => {
                depth += 1;
                i += 1;
                continue;
            }
            '}' => {
                depth -= 1;
                i += 1;
                continue;
            }
            _ => {}
        }
        if depth != 0 || !is_ident(c) {
            i += 1;
            continue;
        }
        // A word at module scope.
        let start = i;
        while i < chars.len() && is_ident(chars[i].1) {
            i += 1;
        }
        let word: String = chars[start..i].iter().map(|(_, c)| *c).collect();
        if !KEYWORDS.contains(&word.as_str()) {
            continue;
        }
        // The declared name is the next identifier, past any `<…>` the
        // declaration wears (`var<uniform> x`).
        let mut j = i;
        let mut angle = 0i32;
        let mut name = String::new();
        while j < chars.len() {
            let ch = chars[j].1;
            if ch == '<' {
                angle += 1;
            } else if ch == '>' {
                angle -= 1;
            } else if angle == 0 && is_ident(ch) {
                let from = j;
                while j < chars.len() && is_ident(chars[j].1) {
                    j += 1;
                }
                name = chars[from..j].iter().map(|(_, c)| *c).collect();
                break;
            } else if angle == 0 && !ch.is_whitespace() {
                break;
            }
            j += 1;
        }
        if name.is_empty() {
            continue;
        }
        // A struct's body, and the end of the whole declaration.
        let mut body = 0usize..0usize;
        let mut end = j;
        if word == "struct" {
            let mut k = j;
            while k < chars.len() && chars[k].1 != '{' {
                k += 1;
            }
            if k < chars.len() {
                let open = chars[k].0 + 1;
                let mut d = 1i32;
                k += 1;
                while k < chars.len() && d > 0 {
                    match chars[k].1 {
                        '{' => d += 1,
                        '}' => d -= 1,
                        _ => {}
                    }
                    k += 1;
                }
                let close = chars.get(k - 1).map_or(code.len(), |(o, _)| *o);
                body = open..close.max(open);
                // Swallow a trailing `;`, which WGSL allows after a struct.
                let mut e = k;
                while e < chars.len() && chars[e].1.is_whitespace() {
                    e += 1;
                }
                if chars.get(e).map(|(_, c)| *c) == Some(';') {
                    e += 1;
                }
                end = chars.get(e).map_or(code.len(), |(o, _)| *o);
                i = k;
            }
        }
        out.push(Decl {
            kind: word,
            name,
            start: at,
            end,
            body,
        });
    }
    out
}

/// One annotation from a doc comment: `@slider(0, 200)`.
struct Ann {
    name: String,
    args: Vec<String>,
}

/// The annotations on a doc block, and the label left when they are taken off.
fn annotations(doc: &str) -> (Vec<Ann>, String) {
    let chars: Vec<char> = doc.chars().collect();
    let mut anns = Vec::new();
    let mut label = String::new();
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] != '@' {
            label.push(chars[i]);
            i += 1;
            continue;
        }
        let from = i + 1;
        let mut j = from;
        while j < chars.len() && is_ident(chars[j]) {
            j += 1;
        }
        if j == from {
            label.push('@');
            i += 1;
            continue;
        }
        let name: String = chars[from..j].iter().collect();
        let mut args = Vec::new();
        let mut k = j;
        while k < chars.len() && chars[k] == ' ' {
            k += 1;
        }
        if chars.get(k) == Some(&'(') {
            let mut depth = 1i32;
            let mut arg = String::new();
            k += 1;
            while k < chars.len() && depth > 0 {
                let c = chars[k];
                match c {
                    '(' => {
                        depth += 1;
                        arg.push(c);
                    }
                    ')' => {
                        depth -= 1;
                        if depth > 0 {
                            arg.push(c);
                        }
                    }
                    ',' if depth == 1 => {
                        args.push(arg.trim().trim_matches('"').to_owned());
                        arg = String::new();
                    }
                    _ => arg.push(c),
                }
                k += 1;
            }
            if !arg.trim().is_empty() {
                args.push(arg.trim().trim_matches('"').to_owned());
            }
            j = k;
        }
        anns.push(Ann { name, args });
        i = j;
    }
    (anns, label.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// `blend_point` to "Blend point" — what a line with nothing left on it gets.
fn humanise(name: &str) -> String {
    let spaced = name.replace('_', " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn num(args: &[String], i: usize) -> Option<f64> {
    args.get(i)?.trim().parse::<f64>().ok()
}

/// Read the fields of the `Params` body: the type chooses the family, the
/// annotation refines it (§1.4).
#[allow(clippy::type_complexity)]
fn read_fields(body: &str) -> Result<(Vec<ShaderField>, Vec<String>), ShaderRefusal> {
    let mut fields = Vec::new();
    let mut notes = Vec::new();
    let mut doc = String::new();
    for line in body.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("///") {
            doc.push(' ');
            doc.push_str(rest);
            continue;
        }
        if t.is_empty() || t.starts_with("//") {
            continue;
        }
        let Some((name, ty_text)) = t.split_once(':') else {
            doc.clear();
            continue;
        };
        let name = name.trim();
        if name.is_empty() || !name.chars().all(is_ident) {
            doc.clear();
            continue;
        }
        let ty_text = ty_text.trim().trim_end_matches(',').trim();
        if ty_text.replace(char::is_whitespace, "") == "vec3<f32>" {
            return Err(ShaderRefusal::Vec3Field(name.to_owned()));
        }
        let Some(ty) = WgslTy::parse(ty_text) else {
            return Err(ShaderRefusal::UnknownType {
                field: name.to_owned(),
                ty: ty_text.to_owned(),
            });
        };
        let (anns, label) = annotations(&doc);
        doc.clear();
        let label = if label.is_empty() {
            humanise(name)
        } else {
            label
        };
        match derive_rows(name, ty, &anns, &label) {
            Ok((params, default)) => fields.push(ShaderField {
                name: leak(name),
                ty,
                offset: 0,
                params: Box::leak(params.into_boxed_slice()),
                default,
            }),
            // A typo in a doc comment costs that row and nothing else (§2.2).
            Err(why) => {
                notes.push(format!("`{name}` was skipped: {why}"));
                fields.push(ShaderField {
                    name: leak(name),
                    ty,
                    offset: 0,
                    params: &[],
                    default: [0.0; 4],
                });
            }
        }
    }
    Ok((fields, notes))
}

fn leak(text: &str) -> &'static str {
    Box::leak(text.to_owned().into_boxed_str())
}

/// The rows one field derives, and the defaults its bytes fall back to.
fn derive_rows(
    name: &str,
    ty: WgslTy,
    anns: &[Ann],
    label: &str,
) -> Result<(Vec<ParamSchema>, [f64; 4]), String> {
    let find = |k: &str| anns.iter().find(|a| a.name == k);
    let default = find("default");
    let unit = match find("unit").and_then(|a| a.args.first().map(String::as_str)) {
        None => None,
        Some("px") => Some(Unit::Px),
        Some("deg") => Some(Unit::Degrees),
        Some("s") => Some(Unit::Seconds),
        Some(other) => return Err(format!("`@unit({other})` — the units are px, deg and s")),
    };
    let row = |id: &str, label: &str, kind: ParamKind, unit: Unit| ParamSchema {
        id: leak(id),
        label: leak(label),
        kind,
        unit,
    };
    match ty {
        WgslTy::F32 => {
            let d = default.and_then(|a| num(&a.args, 0)).unwrap_or(0.0);
            if find("dial").is_some() {
                return Ok((
                    vec![row(
                        name,
                        label,
                        ParamKind::Angle {
                            default: d,
                            dial_step: 15.0,
                        },
                        Unit::Degrees,
                    )],
                    [d, 0.0, 0.0, 0.0],
                ));
            }
            if let Some(a) = find("bounded") {
                let (lo, hi) = (
                    num(&a.args, 0).ok_or("`@bounded` needs two numbers")?,
                    num(&a.args, 1).ok_or("`@bounded` needs two numbers")?,
                );
                return Ok((
                    vec![row(
                        name,
                        label,
                        ParamKind::Slider {
                            default: d,
                            range: (lo, hi),
                        },
                        unit.unwrap_or(Unit::Raw),
                    )],
                    [d, 0.0, 0.0, 0.0],
                ));
            }
            let (lo, hi) = match find("slider") {
                Some(a) => (
                    num(&a.args, 0).ok_or("`@slider` needs two numbers")?,
                    num(&a.args, 1).ok_or("`@slider` needs two numbers")?,
                ),
                None => (0.0, 1.0),
            };
            Ok((
                vec![row(
                    name,
                    label,
                    ParamKind::Float {
                        default: d,
                        slider: (lo, hi),
                        hard: (None, None),
                    },
                    unit.unwrap_or(Unit::Raw),
                )],
                [d, 0.0, 0.0, 0.0],
            ))
        }
        WgslTy::I32 => {
            let d = default.and_then(|a| num(&a.args, 0)).unwrap_or(0.0);
            let (lo, hi) = match find("counter") {
                Some(a) => (
                    num(&a.args, 0).ok_or("`@counter` needs two numbers")?,
                    num(&a.args, 1).ok_or("`@counter` needs two numbers")?,
                ),
                None => (0.0, 100.0),
            };
            Ok((
                vec![row(
                    name,
                    label,
                    ParamKind::Int {
                        default: d as i64,
                        slider: (lo as i64, hi as i64),
                        hard: (None, None),
                    },
                    unit.unwrap_or(Unit::Raw),
                )],
                [d, 0.0, 0.0, 0.0],
            ))
        }
        WgslTy::U32 => {
            if find("toggle").is_some() {
                let on = matches!(
                    default.and_then(|a| a.args.first().map(String::as_str)),
                    Some("true" | "1")
                );
                return Ok((
                    vec![row(name, label, ParamKind::Bool { default: on }, Unit::Raw)],
                    [f64::from(u8::from(on)), 0.0, 0.0, 0.0],
                ));
            }
            if let Some(a) = find("choice") {
                if a.args.is_empty() {
                    return Err("`@choice` needs at least one option".to_owned());
                }
                let options: Vec<&'static str> = a.args.iter().map(|s| leak(s)).collect();
                let d = match default.and_then(|a| a.args.first().cloned()) {
                    Some(text) => match text.parse::<u32>() {
                        Ok(i) => i,
                        Err(_) => a
                            .args
                            .iter()
                            .position(|o| *o == text)
                            .ok_or(format!("`@default(\"{text}\")` names no option"))?
                            as u32,
                    },
                    None => 0,
                };
                return Ok((
                    vec![row(
                        name,
                        label,
                        ParamKind::Choice {
                            options: Box::leak(options.into_boxed_slice()),
                            default: d,
                            dividers_after: &[],
                        },
                        Unit::Raw,
                    )],
                    [f64::from(d), 0.0, 0.0, 0.0],
                ));
            }
            if find("seed").is_some() {
                return Ok((vec![row(name, label, ParamKind::Seed, Unit::Raw)], [0.0; 4]));
            }
            let d = default.and_then(|a| num(&a.args, 0)).unwrap_or(0.0);
            Ok((
                vec![row(
                    name,
                    label,
                    ParamKind::Int {
                        default: d as i64,
                        slider: (0, 100),
                        hard: (Some(0), None),
                    },
                    unit.unwrap_or(Unit::Raw),
                )],
                [d, 0.0, 0.0, 0.0],
            ))
        }
        WgslTy::Vec4 => {
            let d = match default {
                Some(a) if a.args.len() == 4 => [
                    num(&a.args, 0).ok_or("`@default` on a colour needs four numbers")?,
                    num(&a.args, 1).ok_or("`@default` on a colour needs four numbers")?,
                    num(&a.args, 2).ok_or("`@default` on a colour needs four numbers")?,
                    num(&a.args, 3).ok_or("`@default` on a colour needs four numbers")?,
                ],
                Some(_) => return Err("`@default` on a colour needs four numbers".to_owned()),
                None => [1.0, 1.0, 1.0, 1.0],
            };
            Ok((
                vec![row(
                    name,
                    label,
                    ParamKind::Colour {
                        default: d,
                        range: (0.0, 1.0),
                    },
                    Unit::Raw,
                )],
                d,
            ))
        }
        // A point is two adjacent `_x` / `_y` numbers, which is what a point has
        // always been in Lumit (there is no Point kind, and there is no need of
        // one): the panel folds the pair into one row with a crosshair pick.
        WgslTy::Vec2 => {
            let d = match default {
                Some(a) if a.args.len() == 2 => [
                    num(&a.args, 0).ok_or("`@default` on a point needs two numbers")?,
                    num(&a.args, 1).ok_or("`@default` on a point needs two numbers")?,
                ],
                Some(_) => return Err("`@default` on a point needs two numbers".to_owned()),
                None => [0.0, 0.0],
            };
            let u = unit.unwrap_or(Unit::Px);
            Ok((
                vec![
                    row(
                        &format!("{name}_x"),
                        &format!("{label} X"),
                        ParamKind::Float {
                            default: d[0],
                            slider: (0.0, 3840.0),
                            hard: (None, None),
                        },
                        u,
                    ),
                    row(
                        &format!("{name}_y"),
                        &format!("{label} Y"),
                        ParamKind::Float {
                            default: d[1],
                            slider: (0.0, 2160.0),
                            hard: (None, None),
                        },
                        u,
                    ),
                ],
                [d[0], d[1], 0.0, 0.0],
            ))
        }
    }
}

// ------------------------------------------------------------- the arithmetic

/// The fields with their uniform offsets, and the buffer's size.
///
/// WGSL aligns `vec2<f32>` to 8 and `vec4<f32>` to 16, and a struct in the
/// uniform address space is aligned — and so sized — to a multiple of 16. One
/// wrong offset and every field after it reads a neighbour's value, with no
/// error anywhere, which is why this is arithmetic in one place and a test.
fn lay_out(fields: &[ShaderField]) -> (Vec<ShaderField>, u32) {
    let mut at = 0u32;
    let mut out = Vec::with_capacity(fields.len());
    for f in fields {
        let align = f.ty.align();
        at = at.div_ceil(align) * align;
        out.push(ShaderField { offset: at, ..*f });
        at += f.ty.size();
    }
    // A uniform block is a multiple of sixteen bytes, and never nought: WGSL
    // has no empty struct, so an empty block is the one placeholder member the
    // generated text declares.
    (out, at.div_ceil(16).max(1) * 16)
}

/// The generated `struct Params`, with its padding as **named fields** so a
/// person reading the compiled shader sees the layout the host uploaded rather
/// than one they have to infer (§7).
fn struct_wgsl(fields: &[ShaderField]) -> String {
    let mut s = String::from("struct Params {\n");
    let mut at = 0u32;
    let mut pad = 0u32;
    for f in fields {
        let align = f.ty.align();
        let want = at.div_ceil(align) * align;
        while at < want {
            s.push_str(&format!("    _pad{pad}: u32,\n"));
            pad += 1;
            at += 4;
        }
        s.push_str(&format!("    {}: {},\n", f.name, f.ty.name()));
        at += f.ty.size();
    }
    let end = at.div_ceil(16).max(1) * 16;
    while at < end {
        s.push_str(&format!("    _pad{pad}: u32,\n"));
        pad += 1;
        at += 4;
    }
    s.push_str("};");
    s
}
