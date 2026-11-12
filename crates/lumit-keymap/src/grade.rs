//! What the Colour workspace's actions are called in Settings → Keymap: the
//! grade nodes, the shots and the grade clipboard.

/// The words for a grading action, or `None` for an action that isn't one.
pub(crate) fn description(id: &str) -> Option<&'static str> {
    Some(match id {
        "grade.copy" => "Copy the shot's grade",
        "grade.paste" => "Paste the grade onto the picked shots",
        "grade.node.serial" => "Add a serial node",
        "grade.node.parallel" => "Add a parallel node",
        "grade.node.layer" => "Add a layer node",
        "grade.node.outside" => "Add an outside node",
        "grade.matte" => "Change the matte view",
        "grade.still.grab" => "Grab a still",
        "grade.shot.prev" => "Previous shot",
        "grade.shot.next" => "Next shot",
        "grade.shot.first" => "First shot",
        "grade.shot.last" => "Last shot",
        "grade.apply.previous" => "Apply the previous shot's grade",
        "grade.apply.second" => "Apply the grade from two shots back",
        "grade.lightbox" => "Show or hide the Lightbox",
        "grade.compare" => "Show or hide the reference",
        _ => return None,
    })
}
