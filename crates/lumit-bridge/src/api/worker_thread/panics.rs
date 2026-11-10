//! Reading what a caught panic had to say, for the fault line the worker writes.

use flutter_rust_bridge::frb;

/// Whatever a panic payload has to say for itself. Panics carry a `&str` or a
/// `String` in practice; anything else is nameless, and saying so is better
/// than saying nothing.
#[frb(ignore)]
pub(super) fn panic_message(payload: &(dyn std::any::Any + Send)) -> &str {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        s
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s
    } else {
        "no message"
    }
}
