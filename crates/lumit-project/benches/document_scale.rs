//! Document-scale benchmarks (docs/13-PERFORMANCE-RULES.md §2.1 S2–S5),
//! measured against the deterministic stress document
//! ([`lumit_project::fixtures`]).
//!
//! These are the numbers behind the S-budgets: opening and saving the stress
//! `.lum`, and committing / undoing a single op on it. They run on demand with
//! `cargo bench -p lumit-project`; they are not a CI gate yet (the perf job that
//! turns them into pass/fail budgets is a separate step), but they keep the
//! measurement reproducible so a regression is visible.
//!
//! Bench harness code, not runtime: it may `unwrap`/`expect` on setup that is
//! known-good, so it opts out of the workspace no-unwrap lint like the test and
//! fixture modules do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use lumit_core::model::ProjectItem;
use lumit_core::ops::Op;
use lumit_core::DocumentStore;
use lumit_project::fixtures::{stress_document, StressParams};

fn document_scale(c: &mut Criterion) {
    let doc = stress_document(&StressParams::REFERENCE);
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("stress.lum");
    lumit_project::save(&doc, &path).expect("initial save");

    // S5: save the stress document.
    c.bench_function("save_stress_document", |b| {
        b.iter(|| lumit_project::save(&doc, &path).expect("save"));
    });

    // S4: open the stress document (.lum -> Document).
    c.bench_function("open_stress_document", |b| {
        b.iter(|| lumit_project::open(&path).expect("open"));
    });

    // A valid, cheap op to commit: rename the first composition.
    let comp_id = doc
        .items
        .iter()
        .find_map(|i| match i {
            ProjectItem::Composition(comp) => Some(comp.id),
            _ => None,
        })
        .expect("the stress document has a composition");
    let make_op = || Op::RenameItem {
        id: comp_id,
        name: "renamed".into(),
    };

    // S2: commit one op with the stress document open. The clone (the current
    // O(document) commit cost, a known debt) is the setup and is not timed.
    c.bench_function("commit_one_op", |b| {
        b.iter_batched(
            || DocumentStore::new(doc.clone()),
            |store| {
                let _ = store.commit(make_op());
            },
            BatchSize::SmallInput,
        );
    });

    // S3: undo one op.
    c.bench_function("undo_one_op", |b| {
        b.iter_batched(
            || {
                let store = DocumentStore::new(doc.clone());
                let _ = store.commit(make_op());
                store
            },
            |store| {
                let _ = store.undo();
            },
            BatchSize::SmallInput,
        );
    });
}

criterion_group!(benches, document_scale);
criterion_main!(benches);
