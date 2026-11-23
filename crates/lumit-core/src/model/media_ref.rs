//! Where a piece of footage is, as the project holds it.

use super::*;

impl MediaRef {
    /// The path to *show* for this reference — the Project panel's Path column
    /// (docs/07 §3.1, docs/15 §12A.3a).
    ///
    /// The relative path, which is the one a saved project actually carries,
    /// falling back to the absolute path when there is no relative one — a
    /// project that has never been saved has nothing to be relative to, and
    /// showing the file's real location then is more use than showing nothing.
    ///
    /// Display data, and only that: it says where the reference *points*, not
    /// whether anything is there. Resolving it against the project folder and
    /// deciding whether the file exists is [`crate::model`]'s business no more
    /// than probing is — `lumit_project::resolve_media` answers that.
    #[must_use]
    pub fn display_path(&self) -> &str {
        if self.relative_path.is_empty() {
            &self.absolute_path
        } else {
            &self.relative_path
        }
    }

    /// The path to *open* for this reference — the other way round from
    /// [`Self::display_path`].
    ///
    /// The session's absolute path, falling back to the stored relative one
    /// when there is none: an unsaved project has nothing to be relative to,
    /// and a project just loaded has not had its media resolved yet.
    ///
    /// Says where to look, not whether anything is there — the same division
    /// [`Self::display_path`] keeps, and `lumit_project::resolve_media` is
    /// still what answers the second question.
    #[must_use]
    pub fn on_disk(&self) -> &str {
        if self.absolute_path.is_empty() {
            &self.relative_path
        } else {
            &self.absolute_path
        }
    }
}
