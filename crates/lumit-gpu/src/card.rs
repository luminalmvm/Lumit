use std::sync::OnceLock;

static NAME: OnceLock<String> = OnceLock::new();

/// Keep `name` as the card's. Only the first is kept: every context in a
/// process opens the same card.
pub(crate) fn keep(name: &str) {
    let _ = NAME.set(name.to_owned());
}

/// The card the first context opened, as the system names it, or `None`
/// before any context exists.
#[must_use]
pub fn name() -> Option<&'static str> {
    NAME.get().map(String::as_str)
}
