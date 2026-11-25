#![allow(clippy::unwrap_used, clippy::expect_used)]

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};
use lumit_core::model::ProjectItem;
use lumit_core::ops::Op;
use lumit_core::store::DocumentStore;
use lumit_project::fixtures::{stress_document, StressParams};

fn document_scale(c: &mut Criterion) {
    let doc = stress_document(&StressParams::REFERENCE);
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("stress.lum");
    lumit_project::save(&doc, &path).expect("initial save");

    c.bench_function("save_stress_document", |b| {
        b.iter(|| lumit_project::save(&doc, &path).expect("save"));
    });

    c.bench_function("open_stress_document", |b| {
        b.iter(|| lumit_project::open(&path).expect("open"));
    });

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

    c.bench_function("commit_one_op", |b| {
        b.iter_batched(
            || DocumentStore::new(doc.clone()),
            |store| {
                let _ = store.commit(make_op());
            },
            BatchSize::SmallInput,
        );
    });

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
