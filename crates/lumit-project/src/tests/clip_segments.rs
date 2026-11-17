//! Tests for a Sequence layer's clips moving to the Retime property.

use super::*;

/// A Sequence layer's **clips** convert too — the second half of the
/// migration, and the one that would otherwise have left the sequence view
/// editing a representation nothing else spoke.
#[test]
fn a_clips_segment_store_becomes_the_retime_property() {
    use lumit_core::retime::Retime;
    use lumit_core::time::Rational;

    let store = Retime::constant_speed(
        Rational::new(4, 1).unwrap(),
        Rational::ZERO,
        Rational::new(2, 1).unwrap(),
    );
    let doc = serde_json::json!({
        "comps": [{
            "layers": [{
                "kind": { "Sequence": { "clips": [{
                    "id": Uuid::now_v7(),
                    "source": { "Footage": Uuid::now_v7() },
                    "source_in": [0, 1],
                    "source_out": [8, 1],
                    "place_start": [0, 1],
                    "place_duration": [4, 1],
                    "retime": serde_json::to_value(&store).unwrap(),
                }]}}
            }]
        }]
    });

    let out = run_migrations(MIGRATIONS, doc, (0, 1, 0));
    // It typed as a real Clip, which is the whole test: the migration has
    // to produce a document *this* build can read.
    let clip: lumit_core::sequence::Clip = serde_json::from_value(
        out["comps"][0]["layers"][0]["kind"]["Sequence"]["clips"][0].clone(),
    )
    .expect("a clip");
    assert_eq!(
        clip.constant_speed(),
        Some(2.0),
        "double speed before, double speed after"
    );
    // …and it reads the same source moments it used to.
    assert!((clip.source_time(1.0) - store.evaluate(1.0)).abs() < 1e-6);
    assert!((clip.source_time(3.0) - store.evaluate(3.0)).abs() < 1e-6);
}
