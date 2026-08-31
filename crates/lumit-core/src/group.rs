//! **Layer groups** (docs/03-DATA-MODEL.md §5.4, K-702): a named fold in the
//! Timeline's outline that gathers a run of layers under one row.
//!
//! In plain terms: a comp with forty layers in it is a comp nobody can read.
//! A group is a labelled band you drop over a handful of them — "the lower
//! third", "the background plates" — with a triangle on it. Twirl it shut and
//! those layers fold away behind a single row; twirl it open and they are back
//! exactly where they were. That is the whole of it. **Nothing about the
//! picture changes**: the layers render in the same order, with the same
//! blends and the same mattes, whether the group is open, shut, or never made.
//! Lumit already has a tool that *does* change the picture by collapsing
//! layers — Precompose, which packs them into a comp of their own — and the
//! two sit side by side deliberately: the group is the cheap organisational
//! fold, Precompose is the render-level one, and the group's own menu offers
//! Precompose so the second is one click from the first.
//!
//! **Membership is a list on the composition, not a mark on each layer.** A
//! group names the layers it holds; a layer knows nothing about it. That is
//! the shape the project already uses for a project item's label colour
//! (`Project::item_labels`) and for its proxies, and it is what keeps grouping
//! out of every path that reads a layer for the picture: no render walk, no
//! frame key and no import mapping had to learn a new field.
//!
//! **Nothing is policed.** A named layer that has since been deleted, or
//! dragged out of the middle of its group's run, is not an error to repair —
//! it is read as no longer in the group, the same degrade-not-fault rule
//! [`crate::model::Layer::matte`] and `parent` already follow. See
//! [`drawn_members`] for exactly what "still in the group" means.
//!
//! Everything here works on the **stack's ids** rather than on `Layer`s: the
//! only thing a group cares about is which layer sits where, and taking ids
//! keeps this module out of the layer's business entirely.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A named fold over a run of a composition's layers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayerGroup {
    pub id: Uuid,
    pub name: String,
    /// Label colour: an index into the theme's label palette (K-567), the same
    /// palette a layer's own [`crate::model::Layer::label`] indexes. Drawn as
    /// the tick on the group's header row, so a group and the layers in it can
    /// be given one colour between them. 0 by default; purely organisational.
    #[serde(default)]
    pub label: u8,
    /// The layers this group holds, in no required order — [`drawn_members`]
    /// puts them in stack order, because the stack is the only order that
    /// means anything here.
    pub members: Vec<Uuid>,
}

/// The members a group **actually draws over**: the longest unbroken run of
/// its layers in the stack, starting at the topmost one it names.
///
/// **Why a run, rather than "every member still present".** The group is a
/// band in the outline with rows folded under it. A band can only cover layers
/// that sit together: if a member is dragged out to the bottom of a stack of
/// forty, there is no honest way to draw one header over both halves, and
/// folding would hide rows from the middle of the comp while leaving their
/// neighbours showing. So the run is what the header spans, and a member that
/// has drifted out of it simply draws as the ungrouped layer it now looks
/// like. Drag it back and it is in the group again, with nothing to repair and
/// no edit recorded — which is why no reorder anywhere has to know that groups
/// exist.
///
/// `stack` is the composition's layer ids, topmost first; the answer comes
/// back in that same order. Empty when the group names no layer that is still
/// here, which is what a group whose layers were all deleted comes to: it
/// draws nothing, and an Ungroup or a save carries it harmlessly along.
#[must_use]
pub fn drawn_members(stack: &[Uuid], group: &LayerGroup) -> Vec<Uuid> {
    let Some(start) = stack.iter().position(|id| group.members.contains(id)) else {
        return Vec::new();
    };
    stack[start..]
        .iter()
        .copied()
        .take_while(|id| group.members.contains(id))
        .collect()
}

/// Which group a layer is drawn inside, if any — the reverse of
/// [`drawn_members`].
///
/// A layer can be named by two groups at once only if an edit put it there;
/// the first group in list order wins, and the loser reads as one member
/// short. `Op::GroupLayers` declining a layer that is already in a group is
/// what keeps that from arising, so this is the belt to that op's braces
/// rather than a rule of its own.
#[must_use]
pub fn group_of(stack: &[Uuid], groups: &[LayerGroup], layer: Uuid) -> Option<Uuid> {
    groups
        .iter()
        .find(|g| drawn_members(stack, g).contains(&layer))
        .map(|g| g.id)
}

/// Whether `members` sit together in the stack — what `Op::GroupLayers`
/// requires before it will make a group at all.
///
/// Grouping a scattered selection would make a group that is half-drawn the
/// moment it exists ([`drawn_members`] keeps the first run and drops the
/// rest), and a fold that silently loses layers is worse than a refusal. The
/// Timeline's answer to a scattered selection is to say so, not to rearrange
/// the stack behind the user's back: moving layers to make them adjacent
/// changes what the comp *looks like*, and grouping must never be one of the
/// things that does.
#[must_use]
pub fn is_contiguous(stack: &[Uuid], members: &[Uuid]) -> bool {
    if members.is_empty() {
        return false;
    }
    let mut indices: Vec<usize> = stack
        .iter()
        .enumerate()
        .filter(|(_, id)| members.contains(id))
        .map(|(i, _)| i)
        .collect();
    // Every named layer must be in this comp, and no id may be named twice —
    // both come out as a length that does not match.
    if indices.len() != members.len() {
        return false;
    }
    indices.sort_unstable();
    indices.windows(2).all(|w| w[1] == w[0] + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[u128]) -> Vec<Uuid> {
        v.iter().map(|i| Uuid::from_u128(*i)).collect()
    }

    fn group(members: &[u128]) -> LayerGroup {
        LayerGroup {
            id: Uuid::from_u128(999),
            name: "G".into(),
            label: 0,
            members: ids(members),
        }
    }

    #[test]
    fn a_run_is_drawn_in_stack_order() {
        // Named bottom-first; read back the way the stack has them.
        assert_eq!(
            drawn_members(&ids(&[1, 2, 3, 4]), &group(&[3, 2])),
            ids(&[2, 3])
        );
    }

    #[test]
    fn a_member_dragged_out_of_the_run_leaves_the_group() {
        // 1 and 3 are named, but an ungrouped 2 sits between them: the run
        // stops at 1, and 3 draws as the ungrouped layer it now looks like.
        let stack = ids(&[1, 2, 3]);
        assert_eq!(drawn_members(&stack, &group(&[1, 3])), ids(&[1]));
        assert_eq!(
            group_of(&stack, &[group(&[1, 3])], Uuid::from_u128(3)),
            None
        );
        assert_eq!(
            group_of(&stack, &[group(&[1, 3])], Uuid::from_u128(1)),
            Some(Uuid::from_u128(999))
        );
    }

    #[test]
    fn a_group_whose_layers_are_gone_draws_nothing() {
        assert!(drawn_members(&ids(&[1]), &group(&[7, 8])).is_empty());
    }

    #[test]
    fn contiguity_is_what_grouping_asks_for() {
        let stack = ids(&[1, 2, 3]);
        assert!(is_contiguous(&stack, &ids(&[2, 3])));
        // Selection order is not stack order, and must not have to be.
        assert!(is_contiguous(&stack, &ids(&[3, 1, 2])));
        assert!(!is_contiguous(&stack, &ids(&[1, 3])));
        // An id that is not in this comp at all is not a run either.
        assert!(!is_contiguous(&stack, &ids(&[1, 9])));
        assert!(!is_contiguous(&stack, &[]));
    }
}
