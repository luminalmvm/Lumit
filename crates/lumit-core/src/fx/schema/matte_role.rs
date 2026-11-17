//! What a matte role answers about its effect.

use super::*;

impl MatteRole {
    /// The parameter the matte layer reference is stored under, or `None` when
    /// this effect takes no matte. **The one lookup key**: the draw builder
    /// enumerates slots by it, so a role and a declared parameter that disagree
    /// is a matte bound to a layer nobody renders — which
    /// `every_matte_role_names_a_declared_layer_row` refuses to let happen.
    #[must_use]
    pub const fn param(self) -> Option<&'static str> {
        match self {
            MatteRole::None => None,
            MatteRole::Strength => Some(MATTE_PARAM),
            MatteRole::Own { param, .. } => Some(param),
        }
    }

    /// The one-sentence meaning an override declares, `None` for the generic
    /// strength semantic (whose meaning is the same sentence for every effect
    /// that uses it, and is written once in docs/08 §2.6).
    #[must_use]
    pub const fn meaning(self) -> Option<&'static str> {
        match self {
            MatteRole::Own { meaning, .. } => Some(meaning),
            _ => None,
        }
    }

    /// Whether the generic dissolve runs beside the dispatch. False for an
    /// override, which is the whole point of overriding: the matte is already
    /// spent inside the kernel.
    #[must_use]
    pub const fn generic(self) -> bool {
        matches!(self, MatteRole::Strength)
    }
}
