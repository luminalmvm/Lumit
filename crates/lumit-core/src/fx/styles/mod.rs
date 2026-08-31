//! Layer styles (docs/impl/layer-styles.md, K-706): the nine named slots a
//! layer wears, in one pinned order.
//!
//! **In plain terms.** Photoshop lets you hang a wardrobe on a layer — a shadow
//! behind it, a glow around it, a colour across its face, a stroke around its
//! edge — without adding a single effect, and After Effects carries that
//! wardrobe over as *layer styles*. They are not a stack you reorder: they are
//! nine named slots in an order Photoshop fixed twenty years ago, and everyone's
//! muscle memory expects that order.
//!
//! The trick that keeps this small is that a style *is* an effect instance
//! wearing a uniform. Each of the nine is an ordinary `#[derive(Effect)]`
//! declaration, so every style property is a keyframeable, expression-drivable,
//! undo-covered [`Property`](crate::anim::Property) for free, and the resolve
//! walk that turns effects into GPU ops turns styles into GPU ops without
//! learning anything new. What is new is only this second, order-locked list on
//! the layer ([`crate::model::Layer::styles`]) and a handful of kernels.
//!
//! The nine live in [`STYLE_DEFS`], which is deliberately **not** the effect
//! catalogue: the Add-effect menu must never offer "Drop shadow (style)" beside
//! the Drop shadow effect. [`crate::fx::def`] is the lookup that answers for
//! both, and it is the only place the two lists meet.

use crate::model::EffectInstance;

use super::registry::{Catalogue, EffectDef};

/// The nine declarations (docs/impl/layer-styles.md §1).
pub mod defs;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;

use defs::{
    BevelEmbossDef, ColourOverlayDef, DropShadowStyleDef, GradientOverlayDef, InnerGlowDef,
    InnerShadowDef, OuterGlowDef, SatinDef, StrokeStyleDef,
};

/// Every layer style's definition, in **§2's painting order** — first is
/// furthest behind.
///
/// This one list carries the order: [`style_index`] reads a match name's
/// position out of it, and [`normalise_styles`] sorts by that. A second written
/// list would be a second thing to keep in step.
///
/// A [`Catalogue`] rather than a plain slice so the lookup, the iteration and
/// the "one name, one definition" rule are the ones the effect registry already
/// implements. Nothing ever registers into it at run time — a style is a fixed
/// family, not an extension point — but sharing the type is cheaper than a
/// second one.
pub static STYLE_DEFS: Catalogue = Catalogue::new(&[
    // 1. Behind everything.
    &DropShadowStyleDef,
    // 2. Behind the layer, in front of its shadow.
    &OuterGlowDef,
    //    (the layer's own pixels, post-effect-stack, sit here)
    // 4. Interior, clipped to alpha.
    &GradientOverlayDef,
    // 5. Interior, over the gradient overlay.
    &ColourOverlayDef,
    // 6. Interior.
    &SatinDef,
    // 7. Interior.
    &InnerGlowDef,
    // 8. Interior.
    &InnerShadowDef,
    // 9. Straddles the edge, over the interiors.
    &StrokeStyleDef,
    // 10. Topmost — its highlights sit on everything, stroke included.
    &BevelEmbossDef,
]);

/// Where `match_name` sits in §2's order, or `None` for a name this build does
/// not know.
///
/// An unknown name is not an error: a file written by a newer Lumit may carry a
/// tenth style, and K-258's degrade rule says it loads, reports and renders as
/// identity rather than being thrown away.
#[must_use]
pub fn style_index(match_name: &str) -> Option<usize> {
    STYLE_DEFS
        .builtins()
        .position(|d| d.schema().match_name == match_name)
}

/// Whether this style paints **outside** the layer's own alpha — Drop shadow
/// and Outer glow, §2's entries 1 and 2.
///
/// The distinction is the render seam's, not the panel's: an outer style adds
/// premultiplied pixels *underneath* the picture, so it has to run after the
/// interiors have finished changing that picture (see `build.rs`'s
/// `resolve_layer_fx`).
#[must_use]
pub fn style_is_outer(match_name: &str) -> bool {
    matches!(style_index(match_name), Some(0 | 1))
}

/// How many of a **sorted** style list are outer styles — the length of the
/// prefix the render seam runs last.
///
/// Sorted means §2's order, which puts the outer pair first, so the outers are
/// exactly a prefix and the split costs no allocation.
#[must_use]
pub fn outer_prefix(styles: &[EffectInstance]) -> usize {
    styles
        .iter()
        .take_while(|s| style_is_outer(&s.effect.match_name))
        .count()
}

/// Restore §1's invariants on a style list: **at most one instance of each
/// style**, and the list **sorted by §2's order**.
///
/// Run by every mutation path and on load, so a hand-edited or hand-shuffled
/// file comes back in order rather than rendering in whatever order it was
/// written in. Nothing is thrown away except a genuine duplicate — an unknown
/// `style_*` name from a newer Lumit keeps its instance and sorts to the end,
/// where it renders as identity (K-258).
///
/// The sort is stable, so two unknown names keep the order the file had.
pub fn normalise_styles(styles: &mut Vec<EffectInstance>) {
    let mut seen: Vec<String> = Vec::with_capacity(styles.len());
    styles.retain(|s| {
        if seen.contains(&s.effect.match_name) {
            return false;
        }
        seen.push(s.effect.match_name.clone());
        true
    });
    styles.sort_by_key(|s| style_index(&s.effect.match_name).unwrap_or(usize::MAX));
}

/// Every style definition, in §2's order — for the tests and for package 3's
/// "add style" menu, neither of which should have to know [`STYLE_DEFS`] is a
/// catalogue.
pub fn all() -> impl Iterator<Item = &'static dyn EffectDef> {
    STYLE_DEFS.builtins()
}
