//! The name an effect goes by, and a free one for a copy.
//!
//! An expression or a link finds an effect by name and takes the first that
//! answers, so two effects on one layer must never share one.

use crate::model::EffectInstance;

/// The name an effect goes by: the user's own for it, or the effect's label.
#[must_use]
pub fn name_of(inst: &EffectInstance) -> &str {
    inst.custom_name.as_deref().unwrap_or_else(|| {
        super::builtins::schema(&inst.effect.match_name)
            .map_or(&inst.effect.match_name, |s| s.label)
    })
}

/// Give each of `copies` a name no effect in `stack` or an earlier copy has,
/// "Glow 2" for a second Glow. A copy whose name is still free keeps it.
pub fn name_copies(stack: &[EffectInstance], copies: &mut [EffectInstance]) {
    let mut taken: Vec<String> = stack.iter().map(|e| name_of(e).to_lowercase()).collect();
    for copy in copies {
        if let Some(free) = free_name(name_of(copy), &taken) {
            copy.custom_name = Some(free);
        }
        taken.push(name_of(copy).to_lowercase());
    }
}

/// A name none of `taken` (kept in lower case) has, or `None` while `name`
/// itself is free. A name ending in a number counts on from it, so a copy of
/// "Mask 1" is "Mask 2"; any other gets " 2", or the next number free.
#[must_use]
pub fn free_name(name: &str, taken: &[String]) -> Option<String> {
    if !taken.contains(&name.to_lowercase()) {
        return None;
    }
    let (stem, from) = match name.rsplit_once(' ') {
        Some((stem, n)) if !stem.is_empty() => match n.parse::<u32>() {
            Ok(n) => (stem, n.saturating_add(1)),
            Err(_) => (name, 2),
        },
        _ => (name, 2),
    };
    (from..)
        .map(|n| format!("{stem} {n}"))
        .find(|n| !taken.contains(&n.to_lowercase()))
}
