//! Fresh links for the later pieces of a divided link.

use super::*;

/// Give the later pieces of a divided link an id of their own.
///
/// A cut or an overwrite leaves both pieces of a clip carrying its link, so a
/// razor through a picture clip and its sound leaves four clips sharing one
/// id. On each row the first clip of a link keeps it and the second, third
/// and so on take a fresh one each, the same fresh one on every row, so the
/// later halves pair up with each other and with nothing else.
#[frb(ignore)]
pub(super) fn relink(rows: &mut [Row]) {
    // A link, and the ids its later pieces have been given so far.
    let mut fresh: Vec<(Uuid, Vec<Uuid>)> = Vec::new();
    for row in rows {
        let mut order: Vec<usize> = (0..row.clips.len()).collect();
        order.sort_by_key(|i| row.clips[*i].place_start);
        let mut seen: Vec<(Uuid, usize)> = Vec::new();
        for i in order {
            let Some(link) = row.clips[i].link else {
                continue;
            };
            let nth = match seen.iter_mut().find(|(l, _)| *l == link) {
                Some((_, count)) => {
                    *count += 1;
                    *count
                }
                None => {
                    seen.push((link, 0));
                    continue;
                }
            };
            let at = match fresh.iter().position(|(l, _)| *l == link) {
                Some(at) => at,
                None => {
                    fresh.push((link, Vec::new()));
                    fresh.len() - 1
                }
            };
            let ids = &mut fresh[at].1;
            while ids.len() < nth {
                ids.push(Uuid::now_v7());
            }
            row.clips[i].link = Some(ids[nth - 1]);
        }
    }
}
