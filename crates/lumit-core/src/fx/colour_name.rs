/// Which of a config's lists a [`ParamKind::ColourName`] row offers. A view
/// row lists the views of the display its sibling `display` row names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColourNameRole {
    Space,
    Display,
    View,
    Look,
    /// The Information row: read-only, showing what the loaded config calls
    /// itself. Its value stays empty and nothing reads it.
    Config,
}
