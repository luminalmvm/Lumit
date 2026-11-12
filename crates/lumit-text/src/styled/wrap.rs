//! Where a block's lines start and where a paragraph wraps.

/// The text cut into lines. Each is its first character's index and its
/// characters, without the break that ended it.
pub(super) fn lines_of(text: &str) -> Vec<(usize, Vec<char>)> {
    let mut out = Vec::new();
    let mut current: Vec<char> = Vec::new();
    let mut start = 0usize;
    let mut previous = '\0';
    for (i, ch) in text.chars().enumerate() {
        match ch {
            // The second half of a Windows line ending. The line is already
            // broken, so it only moves where the next one starts.
            '\n' if previous == '\r' => start = i + 1,
            '\n' | '\r' => {
                out.push((start, std::mem::take(&mut current)));
                start = i + 1;
            }
            _ => current.push(ch),
        }
        previous = ch;
    }
    out.push((start, current));
    out
}

/// Where a paragraph wraps to fit `room`: the index of each space a line ends
/// at. A word wider than the box is left whole and runs past it.
///
/// ponytail: breaks at spaces only. Chinese and Japanese need the Unicode line
/// breaking rules.
pub(super) fn breaks(
    chars: &[char],
    carets: &[f32],
    room: f32,
    first_line: f32,
    tracking: f32,
) -> Vec<usize> {
    let mut cuts = Vec::new();
    let mut from = 0usize;
    let mut space = None;
    for (k, ch) in chars.iter().enumerate() {
        if *ch == ' ' {
            space = Some(k);
            continue;
        }
        let room = if cuts.is_empty() {
            room - first_line
        } else {
            room
        };
        let (Some(end), Some(line)) = (carets.get(k + 1), carets.get(from)) else {
            break;
        };
        if end - tracking - line > room {
            if let Some(at) = space.take() {
                cuts.push(at);
                from = at + 1;
            }
        }
    }
    cuts
}
