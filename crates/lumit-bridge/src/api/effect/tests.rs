//! Tests for the effect commands.

use super::*;

/// **The coating rows resolve against a parameter that exists**.
///
/// A group's visibility is looked up in the panel by sibling id, and a
/// sibling that is not there fails silently — the rows naming it simply
/// never draw. Shipping `"lens"` instead of `"lens_model"` did exactly
/// that, leaving only the rows with an unreachable threshold, whose empty
/// value set means "always visible". So: the id names a real Choice, and
/// every emitted rule is one the panel can act on.
#[test]
fn every_element_row_names_a_real_sibling_and_a_reachable_set() {
    let flare = lumit_core::fx::catalogue::BUILTINS
        .iter()
        .find(|s| s.match_name == "lens_flare")
        .expect("the Lens flare is a builtin");
    assert!(
        flare.params.iter().any(|p| p.id == LENS_PICK_PARAM),
        "`{LENS_PICK_PARAM}` must be a parameter of the Lens flare"
    );

    let groups = list_parameter_groups("lens_flare".to_owned());
    let mut per_element = 0;
    for g in &groups {
        let Some(param) = &g.visible_when_param else {
            assert!(g.visible_when_values.is_empty());
            continue;
        };
        assert!(
            flare.params.iter().any(|p| p.id == param),
            "group `{}` is resolved against `{param}`, which the schema \
                 does not declare",
            g.label
        );
        assert!(
            !g.visible_when_values.is_empty(),
            "a conditional group with an empty value set reads as \
                 unconditional, which is the opposite of what it means"
        );
        if param == LENS_PICK_PARAM {
            per_element += 1;
        }
    }
    assert_eq!(
        per_element,
        lumit_core::fx::lens_flare::MAX_COATING_ELEMENTS,
        "every element row must carry a rule"
    );

    // A threshold no bundled lens reaches must resolve to "never", and it
    // says so with an index no Lens choice can hold.
    let deep = groups
        .iter()
        .filter(|g| g.visible_when_param.as_deref() == Some(LENS_PICK_PARAM))
        .filter(|g| g.visible_when_values == vec![u32::MAX])
        .count();
    let reachable = lumit_core::fx::lens_flare::library_element_counts()
        .into_iter()
        .max()
        .expect("a library");
    assert_eq!(
        deep,
        lumit_core::fx::lens_flare::MAX_COATING_ELEMENTS - reachable as usize,
        "the rows past the deepest bundled lens are the ones that never draw"
    );
}
