//! What this host offers a plugin beyond the core, and what happens to one that
//! cannot run without something else (docs/impl/lfx.md §2.1, §4.3).
//!
//! # In plain terms
//!
//! The frozen core is small on purpose. Anything beyond it - reading the frames
//! either side of this one, say - is an **extension**: a typed table of
//! function pointers fetched by name and version, and a missing one is a null
//! rather than a status. A plugin that can work without it asks and carries on;
//! a plugin that cannot says so in its descriptor and in its listing, and this
//! module is where that declaration is answered.
//!
//! **Before `create`, never after.** docs/12 §3.6 says a plugin requiring a
//! missing extension "fails to instantiate with a clear message and becomes a
//! placeholder", and the only way that is true rather than aspirational is for
//! the negotiation to run before the plugin is instantiated at all - from the
//! **listing**, so that the answer is known while the module is still shut, and
//! re-checked against the descriptor at the first describe once it is open
//! ([`crate::manifest::agrees`]). A plugin left to reach `create`, get a null
//! and fail somewhere later is the outcome the negotiation exists to prevent.
//!
//! # One list, two readers
//!
//! [`OFFERED_EXTENSIONS`] is read by the in-process host, which negotiates at
//! the moment it instantiates, and by discovery, which negotiates from the
//! listing before a plugin is offered to the catalogue at all. They are the
//! same question asked in two places and a host that answered it two ways would
//! catalogue an effect that cannot be made.

use crate::LfxRejection;

/// The extensions this host offers.
///
/// **Empty, and version 1 says so.** `lfx.temporal` is the one extension
/// docs/12 §3.5 admits in version 1 and the frozen header declares only its
/// *id*: the typed table a neighbour frame would arrive through has no
/// declaration yet, and offering a name with nothing behind it would be the
/// "left to fail somewhere later" outcome the negotiation exists to prevent. So
/// the header's rule is kept honestly - a missing extension is a null, never a
/// status - and a plugin that says it cannot run without one is refused before
/// it is instantiated, with the extension named.
///
/// The temporal *gate* is not this: it is `lfx_traits.temporal_lo/hi`, which a
/// plugin declares and [`crate::schema::traits_of`] lowers, and which needs no
/// extension at all.
pub const OFFERED_EXTENSIONS: &[&str] = &[];

/// Whether this host offers an extension by that name.
#[must_use]
pub fn offers(id: &str) -> bool {
    OFFERED_EXTENSIONS.contains(&id)
}

/// The first extension in a required list this host has not got, or `None` when
/// every one of them is offered.
///
/// The **first**, not all of them: a plugin is refused for the first thing it
/// asked for that is not here, and listing the rest would be telling a vendor
/// about four missing tables when fixing one is not the answer either.
#[must_use]
pub fn missing_from(required: &[String]) -> Option<&str> {
    required
        .iter()
        .map(String::as_str)
        .find(|wanted| !offers(wanted))
}

/// Answer the negotiation for one plugin's required list.
///
/// # Errors
///
/// [`LfxRejection::RequiresExtension`], naming the plugin and the extension it
/// asked for - the sentence §5.3's `REFUSED` table carries and the one the
/// Addons page prints under the row.
pub fn negotiate(id: &str, required: &[String]) -> Result<(), LfxRejection> {
    match missing_from(required) {
        None => Ok(()),
        Some(wanted) => Err(LfxRejection::RequiresExtension {
            id: id.to_owned(),
            extension: wanted.to_owned(),
        }),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// Version 1 offers nothing, and the two halves of this module have to say
    /// so together: a list that grew a name without a table behind it would
    /// admit a plugin the host then failed somewhere later.
    #[test]
    fn version_one_offers_no_extension_at_all() {
        assert!(OFFERED_EXTENSIONS.is_empty());
        assert!(!offers("lfx.temporal"));
        assert_eq!(missing_from(&[]), None);
    }

    /// The refusal names the plugin and the extension, because a row on the
    /// Addons page saying only "refused" is a row nobody can act on.
    #[test]
    fn a_required_extension_the_host_has_not_got_is_refused_by_name() {
        let required = vec!["lfx.temporal".to_owned(), "lfx.gpu-frames".to_owned()];
        let refusal = negotiate("com.example.needy", &required).expect_err("a refusal");
        assert_eq!(
            refusal,
            LfxRejection::RequiresExtension {
                id: "com.example.needy".to_owned(),
                extension: "lfx.temporal".to_owned(),
            },
            "the first thing it asked for that is not here"
        );
        assert!(
            refusal.refuses_the_effect(),
            "a plugin that cannot be made is not a report line"
        );
    }
}
