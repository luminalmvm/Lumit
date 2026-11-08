//! Reading a driven row: what the wire into a parameter carries at a frame.
//!
//! The interface never asks this in a rebuild; the panels name the driver
//! and leave the number to the render. It is here so a drive wired from a
//! row's menu can be checked from the interface side, with no sound.

use std::sync::Arc;

use flutter_rust_bridge::frb;
use uuid::Uuid;

use crate::api::layer::LayerReference;
use crate::api::BridgeError;

/// What the wire into `effect`'s row `param` on `layer` carries at `frame`,
/// or `None` while the row is unwired.
#[frb(sync)]
pub fn driven_value_at(
    layer: LayerReference,
    effect: Uuid,
    param: String,
    frame: i64,
) -> Result<Option<f64>, BridgeError> {
    let item = layer.item()?;
    let comp = layer.composition()?;
    let Some(doc) = crate::api::effect::document_for(&layer) else {
        return Ok(None);
    };
    let time = comp
        .frame_rate
        .time_of_frame(frame)
        .map_err(|_| BridgeError::InvalidComp)?;
    let comp_time = time.0.to_f64();
    let context = Arc::new(lumit_core::expression::ExpressionContext {
        document: doc,
        comp: Some(layer.comp_id),
        layer: Some(layer.layer_id),
        comp_time,
        current_depth: 0,
        inputs: None,
    });
    Ok(lumit_core::fx::drivers::driven_param(
        &item.graph,
        effect,
        &param,
        lumit_core::time::layer_time(comp_time, item.start_offset.0),
        context,
        None,
    ))
}
