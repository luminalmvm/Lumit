//! A gradient's colours (`GCst`): the property's metadata in a `tdbs`, beside
//! a `GCky` holding one small XML document per value.
//!
//! The document lists the colour stops, each as position, midpoint, red, green,
//! blue and alpha, and the opacity stops separately, each as position, midpoint
//! and opacity. They are handed on as two lists, `colours` of `[position, r, g,
//! b]` and `alphas` of `[position, alpha]`, all 0 to 1. The midpoints are left
//! at the middle.

use serde_json::{json, Value};

use super::*;

pub(super) fn read_gradient(
    match_name: &str,
    gcst: &Chunk<'_>,
    run: &[Chunk<'_>],
    ctx: Ctx<'_>,
) -> Property {
    let inside: Vec<Chunk<'_>> = gcst.children().ok().collect();
    let values: Vec<Value> = inside
        .iter()
        .filter(|chunk| chunk.is_list(b"GCky"))
        .flat_map(|gcky| gcky.children().ok().collect::<Vec<_>>())
        .filter(|chunk| chunk.id == *b"Utf8")
        .filter_map(|chunk| stops(&chunk.text()))
        .collect();
    let Some(first) = values.first().cloned() else {
        return unreadable_node(match_name, "the gradient's colours could not be read");
    };
    // The keys' times, where it is keyed. A key this reader cannot place is
    // not worth a row of its own: the first value stands either way.
    let mut ignored = Vec::new();
    let mut leaf = match inside.iter().find(|chunk| chunk.is_list(b"tdbs")) {
        Some(tdbs) if values.len() > 1 => read_leaf(match_name, tdbs, run, ctx, &mut ignored),
        _ => Property {
            match_name: Some(match_name.to_string()),
            ..Property::default()
        },
    };
    leaf.value_type = Some("gradient".to_string());
    leaf.unreadable = None;
    leaf.value = Some(first);
    if let Some(keys) = leaf.keyframes.as_mut() {
        for (key, value) in keys.iter_mut().zip(values) {
            key.v = Some(value);
        }
    }
    leaf
}

/// The two lists of stops out of one document, or `None` when it has none.
fn stops(xml: &str) -> Option<Value> {
    let colours_at = xml.find("<key>Color Stops</key>")?;
    let alphas_at = xml.find("<key>Alpha Stops</key>")?;
    let (colour_part, alpha_part) = if colours_at < alphas_at {
        (xml.get(colours_at..alphas_at)?, xml.get(alphas_at..)?)
    } else {
        (xml.get(colours_at..)?, xml.get(alphas_at..colours_at)?)
    };
    let colours: Vec<Value> = arrays(colour_part)
        .into_iter()
        .filter(|a| a.len() >= 5)
        .map(|a| json!([a[0], a[2], a[3], a[4]]))
        .collect();
    let alphas: Vec<Value> = arrays(alpha_part)
        .into_iter()
        .filter(|a| a.len() >= 3)
        .map(|a| json!([a[0], a[2]]))
        .collect();
    (!colours.is_empty()).then(|| json!({ "colours": colours, "alphas": alphas }))
}

/// Every `<array>` of `<float>`s in `xml`, in order.
fn arrays(xml: &str) -> Vec<Vec<f64>> {
    xml.split("<array>")
        .skip(1)
        .filter_map(|rest| rest.split("</array>").next())
        .map(|body| {
            body.split("<float>")
                .skip(1)
                .filter_map(|v| v.split("</float>").next()?.trim().parse().ok())
                .collect()
        })
        .collect()
}
