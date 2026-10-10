//! Turning expression text into tokens.

/// One piece of a template literal: the text between the `${…}` holes, and the
/// source inside each hole, which the parser reads as an expression of its own.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum TemplatePiece {
    Text(String),
    Code(String),
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Tok {
    Num(f64),
    Str(String),
    Template(Vec<TemplatePiece>),
    /// A name or a keyword. The parser tells them apart, since a keyword is
    /// only one where the grammar expects it (`thisComp.layer`).
    Ident(String),
    Punct(&'static str),
    Eof,
}

#[derive(Debug, Clone)]
pub(super) struct Token {
    pub tok: Tok,
    pub line: u32,
    /// Whether a line break sits between this token and the one before. It is
    /// what lets a statement end without a semicolon.
    pub newline_before: bool,
}

/// Longest first, so `>>>=` is not read as `>` four times.
const PUNCTUATORS: [&str; 51] = [
    ">>>=", "===", "!==", "**=", "<<=", ">>=", ">>>", "...", "&&=", "||=", "??=", "=>", "==", "!=",
    "<=", ">=", "&&", "||", "??", "?.", "++", "--", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=",
    "**", "<<", ">>", "{", "}", "(", ")", "[", "]", ";", ",", "<", ">", "+", "-", "*", "/", "%",
    "&", "|", "^",
];
const SINGLE: [&str; 6] = ["!", "~", "?", ":", "=", "."];

/// An expression is a few lines to a few hundred. A megabyte of it is a file
/// somebody made to see what happens.
const MAX_SOURCE_BYTES: usize = 256 * 1024;

pub(super) fn lex(source: &str) -> Result<Vec<Token>, String> {
    if source.len() > MAX_SOURCE_BYTES {
        return Err("the expression is too long".into());
    }
    let chars: Vec<char> = source.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let mut line = 1u32;
    let mut newline = false;

    while let Some(&c) = chars.get(i) {
        // After Effects writes its presets with bare carriage returns, so all
        // three line endings have to count.
        if c == '\n' || c == '\r' || c == '\u{2028}' || c == '\u{2029}' {
            if !(c == '\n' && i > 0 && chars.get(i - 1) == Some(&'\r')) {
                line += 1;
            }
            newline = true;
            i += 1;
            continue;
        }
        if c.is_whitespace() || c == '\u{feff}' {
            i += 1;
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while chars.get(i).is_some_and(|c| *c != '\n' && *c != '\r') {
                i += 1;
            }
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            loop {
                match chars.get(i) {
                    None => return Err(format!("line {line}: a comment that never closes")),
                    Some('*') if chars.get(i + 1) == Some(&'/') => {
                        i += 2;
                        break;
                    }
                    Some(c) => {
                        if *c == '\n' || *c == '\r' {
                            newline = true;
                            line += 1;
                        }
                        i += 1;
                    }
                }
            }
            continue;
        }

        let tok = if c.is_ascii_digit() || (c == '.' && next_is_digit(&chars, i + 1)) {
            number(&chars, &mut i, line)?
        } else if c == '"' || c == '\'' {
            i += 1;
            Tok::Str(string(&chars, &mut i, c, line)?)
        } else if c == '`' {
            i += 1;
            template(&chars, &mut i, line)?
        } else if c.is_alphabetic() || c == '_' || c == '$' {
            let start = i;
            while chars
                .get(i)
                .is_some_and(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
            {
                i += 1;
            }
            Tok::Ident(chars.get(start..i).unwrap_or_default().iter().collect())
        } else {
            let found = PUNCTUATORS.iter().chain(SINGLE.iter()).find(|p| {
                p.chars()
                    .enumerate()
                    .all(|(n, pc)| chars.get(i + n) == Some(&pc))
            });
            match found {
                Some(p) => {
                    i += p.len();
                    Tok::Punct(p)
                }
                None => return Err(format!("line {line}: unexpected '{c}'")),
            }
        };
        out.push(Token {
            tok,
            line,
            newline_before: newline,
        });
        newline = false;
    }
    out.push(Token {
        tok: Tok::Eof,
        line,
        newline_before: true,
    });
    Ok(out)
}

fn next_is_digit(chars: &[char], i: usize) -> bool {
    chars.get(i).is_some_and(char::is_ascii_digit)
}

fn number(chars: &[char], i: &mut usize, line: u32) -> Result<Tok, String> {
    let start = *i;
    if chars.get(*i) == Some(&'0') && matches!(chars.get(*i + 1), Some('x' | 'X')) {
        *i += 2;
        let digits = *i;
        while chars.get(*i).is_some_and(char::is_ascii_hexdigit) {
            *i += 1;
        }
        let text: String = chars.get(digits..*i).unwrap_or_default().iter().collect();
        return u64::from_str_radix(&text, 16)
            .map(|n| Tok::Num(n as f64))
            .map_err(|_| format!("line {line}: a number that does not read"));
    }
    while chars.get(*i).is_some_and(char::is_ascii_digit) {
        *i += 1;
    }
    if chars.get(*i) == Some(&'.') {
        *i += 1;
        while chars.get(*i).is_some_and(char::is_ascii_digit) {
            *i += 1;
        }
    }
    if matches!(chars.get(*i), Some('e' | 'E')) {
        let sign = usize::from(matches!(chars.get(*i + 1), Some('+' | '-')));
        if next_is_digit(chars, *i + 1 + sign) {
            *i += 1 + sign;
            while chars.get(*i).is_some_and(char::is_ascii_digit) {
                *i += 1;
            }
        }
    }
    let text: String = chars.get(start..*i).unwrap_or_default().iter().collect();
    text.parse::<f64>()
        .map(Tok::Num)
        .map_err(|_| format!("line {line}: a number that does not read"))
}

/// The character an escape stands for, with `i` on the character after the
/// backslash.
fn escape(chars: &[char], i: &mut usize, line: u32) -> Result<Option<char>, String> {
    let Some(&c) = chars.get(*i) else {
        return Err(format!("line {line}: text that never closes"));
    };
    *i += 1;
    let hex = |i: &mut usize, digits: usize| -> Result<Option<char>, String> {
        let text: String = chars
            .get(*i..*i + digits)
            .unwrap_or_default()
            .iter()
            .collect();
        *i += digits;
        u32::from_str_radix(&text, 16)
            .ok()
            .and_then(char::from_u32)
            .map(Some)
            .ok_or_else(|| format!("line {line}: an escape that does not read"))
    };
    Ok(match c {
        'n' => Some('\n'),
        't' => Some('\t'),
        'r' => Some('\r'),
        '0' => Some('\0'),
        'b' => Some('\u{8}'),
        'f' => Some('\u{c}'),
        'v' => Some('\u{b}'),
        'x' => return hex(i, 2),
        'u' => return hex(i, 4),
        // A backslash before a line break carries the text on.
        '\n' | '\r' => None,
        other => Some(other),
    })
}

fn string(chars: &[char], i: &mut usize, quote: char, line: u32) -> Result<String, String> {
    let mut out = String::new();
    loop {
        match chars.get(*i) {
            None | Some('\n' | '\r') => return Err(format!("line {line}: text that never closes")),
            Some(c) if *c == quote => {
                *i += 1;
                return Ok(out);
            }
            Some('\\') => {
                *i += 1;
                if let Some(c) = escape(chars, i, line)? {
                    out.push(c);
                }
            }
            Some(c) => {
                out.push(*c);
                *i += 1;
            }
        }
    }
}

fn template(chars: &[char], i: &mut usize, line: u32) -> Result<Tok, String> {
    let mut pieces = Vec::new();
    let mut text = String::new();
    loop {
        match chars.get(*i) {
            None => return Err(format!("line {line}: text that never closes")),
            Some('`') => {
                *i += 1;
                pieces.push(TemplatePiece::Text(text));
                return Ok(Tok::Template(pieces));
            }
            Some('\\') => {
                *i += 1;
                if let Some(c) = escape(chars, i, line)? {
                    text.push(c);
                }
            }
            Some('$') if chars.get(*i + 1) == Some(&'{') => {
                *i += 2;
                let start = *i;
                let mut depth = 1u32;
                while depth > 0 {
                    match chars.get(*i) {
                        None => return Err(format!("line {line}: text that never closes")),
                        Some('{') => depth += 1,
                        Some('}') => depth -= 1,
                        Some(_) => {}
                    }
                    *i += 1;
                }
                pieces.push(TemplatePiece::Text(std::mem::take(&mut text)));
                pieces.push(TemplatePiece::Code(
                    chars
                        .get(start..*i - 1)
                        .unwrap_or_default()
                        .iter()
                        .collect(),
                ));
            }
            Some(c) => {
                text.push(*c);
                *i += 1;
            }
        }
    }
}
